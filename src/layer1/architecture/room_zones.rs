//! Room Zones (Spec 381): persistent designated rooms with cached value.
//!
//! # Relationship to existing systems (no parallel duplicates)
//!
//! Scale already has two room-adjacent legs, and this module is the third,
//! missing one — the persistent *entity* layer the spec asks for:
//!
//! * [`crate::layer1::zone::ZoneGrid`] is the per-tile designation grid
//!   (Bedroom / Dining / Hospital / …). Designating a `RoomZone` also paints
//!   its tiles into the `ZoneGrid`, so the two views never disagree.
//! * [`crate::layer1::room_quality::calculate_room_quality`] is an ephemeral
//!   flood-fill quality probe (Space + Beauty) used for waking thoughts.
//!   It stays the authority for *un-designated* space.
//! * This module owns designated [`RoomZone`] entities: a cached
//!   `Room Value = Size + Furniture Quality + Wall Material`, pop assignment
//!   via the pre-existing [`AssignedRoom`] component (shared with resonant
//!   architecture — not redefined here), mood effects on [`Needs`]/[`Morale`],
//!   barracks overcrowding penalties, and suite jealousy.
//!
//! # Design notes (spec §5 REFACTOR items, implemented)
//!
//! * Occupant counts are **derived** from `AssignedRoom` queries. The
//!   [`OccupantCount`] component exists only as a legacy/cache override for
//!   spec-test compatibility; the effective count is
//!   `max(derived, stored)`.
//! * Evaluation is **throttled**: [`evaluate_rooms_system`] re-evaluates at
//!   most every [`EVAL_INTERVAL_TICKS`] ticks, or immediately when
//!   [`RoomZoneState::dirty`] is set (designation / furniture changes set
//!   it). When the state/time resources are absent (unit tests), it always
//!   evaluates.
//! * Wall material uses the real seam: perimeter `BuildingType::Wall`
//!   buildings carry a [`Material`] component, scored through
//!   [`MaterialType::beauty_modifier`] — the same value the rest of the
//!   game already assigns to wall materials.
//! * Mood application is cadence-gated the same way (every
//!   [`MOOD_INTERVAL_TICKS`] ticks in the live schedule), so needs never
//!   jitter per-tick.

use crate::layer1::building::{Building, BuildingType, Material, MaterialType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::resonant_architecture::AssignedRoom;
use crate::layer1::zone::{ZoneGrid, ZoneType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

/// Room value above which assigned pops gain a mood buff.
pub const HIGH_VALUE_THRESHOLD: f32 = 20.0;
/// Room value at which a room is "legendary" — chronicle-worthy, and the
/// envy of everyone sleeping somewhere worse.
pub const LEGENDARY_VALUE_THRESHOLD: f32 = 50.0;
/// A room with more occupants than this is an overcrowded barracks.
pub const OVERCROWDED_LIMIT: usize = 2;
/// Minimum value gap before a pop envies the colony's best room.
pub const JEALOUSY_GAP: f32 = 20.0;
/// Best-room value at/above which jealousy can trigger at all.
pub const JEALOUSY_FLOOR: f32 = 40.0;
/// Ticks between room re-evaluations (unless marked dirty).
pub const EVAL_INTERVAL_TICKS: u64 = 50;
/// Ticks between mood applications in the live schedule.
pub const MOOD_INTERVAL_TICKS: u64 = 50;
/// Needs delta for the high-value room buff (Needs are 0.0..=1.0).
pub const ROOM_BUFF_DELTA: f32 = 0.05;
/// Needs delta for the overcrowding leisure penalty.
pub const BARRACKS_PENALTY_DELTA: f32 = 0.05;
/// Needs delta for suite jealousy.
pub const JEALOUSY_PENALTY_DELTA: f32 = 0.02;

/// A designated room: a set of tiles the player has claimed as one room.
///
/// `base_value` is the cached non-size part of the room value (furniture
/// quality + wall-material bonus), refreshed by
/// [`evaluate_rooms_system`]. The spec's sketch stored furniture quality
/// there directly; the meaning is unchanged, the inputs grew.
#[derive(Component, Clone, Debug)]
pub struct RoomZone {
    /// What kind of room this is (shared with the per-tile `ZoneGrid`).
    pub zone_type: ZoneType,
    /// Tiles belonging to the room.
    pub tiles: Vec<(i32, i32)>,
    /// Cached furniture quality + wall-material bonus.
    pub base_value: f32,
    /// The wall-material portion of `base_value` (for display/debug).
    pub wall_bonus: f32,
}

impl RoomZone {
    /// Creates an unevaluated room (call [`evaluate_rooms_system`] or set
    /// `base_value` directly, as the spec's tests do).
    #[must_use]
    pub fn new(zone_type: ZoneType, tiles: Vec<(i32, i32)>) -> Self {
        Self {
            zone_type,
            tiles,
            base_value: 0.0,
            wall_bonus: 0.0,
        }
    }

    /// Room Value = Size + Furniture Quality + Wall Material.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn value(&self) -> f32 {
        self.base_value + self.tiles.len() as f32
    }

    /// Number of tiles in the room.
    #[must_use]
    pub fn size(&self) -> usize {
        self.tiles.len()
    }

    /// True when this room's value earns the high-value mood buff.
    #[must_use]
    pub fn is_high_value(&self) -> bool {
        self.value() > HIGH_VALUE_THRESHOLD
    }
}

/// A piece of furniture inside a room. Quality feeds the room value.
///
/// This is the furnishing leg the codebase lacked: decorative *buildings*
/// already emit into the `BeautyGrid` (room_quality's input), but no
/// per-item quality component existed for designated rooms.
#[derive(Component, Clone, Copy, Debug)]
pub struct Furniture {
    /// Furniture quality contribution to the room value.
    pub quality: f32,
}

/// Links an entity (furniture, props) to the [`RoomZone`] containing it.
#[derive(Component, Clone, Copy, Debug)]
pub struct InRoom(pub Entity);

/// Legacy/cache occupant count (spec sketch type).
///
/// Per spec §5 this is *not* the source of truth: occupant counts are
/// derived from `AssignedRoom` queries. When present, the effective count
/// is `max(derived, stored)` so a manually-declared barracks still reads
/// as crowded before every bunk is formally assigned.
#[derive(Component, Clone, Copy, Debug)]
pub struct OccupantCount(pub usize);

/// Fired after a room's cached value is recomputed.
#[derive(Event, Debug, Clone, Copy)]
pub struct RoomEvaluatedEvent(pub Entity);

/// Throttle/dirty state for room evaluation (first-tick resource: it is
/// initialised in `setup` and `simulation` before any system reads it).
#[derive(Resource, Default, Debug)]
pub struct RoomZoneState {
    /// Set when designations or furniture change; forces re-evaluation.
    pub dirty: bool,
    /// Tick of the last evaluation pass.
    pub last_eval_tick: u64,
}

/// Chronicle bookkeeping so celebrations/warnings fire once per room.
#[derive(Resource, Default)]
pub struct RoomChronicleState {
    /// Rooms already celebrated for reaching legendary value.
    pub celebrated: HashSet<Entity>,
    /// Rooms already warned about as overcrowded barracks.
    pub warned_barracks: HashSet<Entity>,
}

/// Human label for a zone type (display + chronicle text).
#[must_use]
pub const fn zone_type_label(zone_type: ZoneType) -> &'static str {
    match zone_type {
        ZoneType::Bedroom => "bedroom",
        ZoneType::Dining => "dining hall",
        ZoneType::Hospital => "hospital ward",
        ZoneType::Office => "office",
        ZoneType::Storage => "store room",
        ZoneType::Jail => "cell block",
        ZoneType::Pasture => "pasture",
        ZoneType::Penal => "penal block",
        ZoneType::Sanctuary => "sanctuary",
        ZoneType::Extraterritorial => "embassy suite",
        ZoneType::Customs => "customs room",
        ZoneType::Dezoned => "dezoned room",
        ZoneType::None => "room",
    }
}

/// Wall-material contribution of one wall tile, via the game's existing
/// material beauty seam (Wood 0, Stone 1, Metal 0, Gold 10).
#[must_use]
pub fn wall_material_value(material: MaterialType) -> f32 {
    material.beauty_modifier()
}

/// Derived occupant count: how many `AssignedRoom` links point at `room`.
#[must_use]
pub fn derived_occupant_count<'a>(
    room: Entity,
    assigned: impl Iterator<Item = &'a AssignedRoom>,
) -> usize {
    assigned.filter(|a| a.0 == room).count()
}

/// Effective occupant count: derived count, floored by any stored cache.
#[must_use]
pub fn effective_occupant_count(derived: usize, stored: Option<&OccupantCount>) -> usize {
    stored.map_or(derived, |c| derived.max(c.0))
}

/// True when a room houses more pops than privacy allows.
#[must_use]
pub const fn is_overcrowded(occupants: usize) -> bool {
    occupants > OVERCROWDED_LIMIT
}

/// True when a pop in a room of `room_value` would envy the colony's best
/// room (`best_value`): a genuinely fine suite exists, and theirs is far
/// behind it.
#[must_use]
pub fn is_jealous(room_value: f32, best_value: f32) -> bool {
    best_value >= JEALOUSY_FLOOR && best_value - room_value >= JEALOUSY_GAP && room_value < best_value
}

fn upsert_modifier(morale: &mut Morale, label: &str, value: f32, duration: u32) {
    if let Some(existing) = morale.modifiers.iter_mut().find(|m| m.label == label) {
        existing.value = value;
        existing.duration = duration;
    } else {
        morale.add_modifier(MoodModifier {
            label: label.to_string(),
            value,
            duration,
        });
    }
}

/// Recomputes every room's cached value from its furniture and perimeter
/// wall materials, and emits [`RoomEvaluatedEvent`] per room.
///
/// Throttled per the module docs: with [`RoomZoneState`] +
/// [`SimulationTime`] present, a clean state inside the interval is a
/// no-op; without them (unit tests, tools) it always runs.
#[allow(clippy::type_complexity)]
pub fn evaluate_rooms_system(
    mut rooms: Query<(Entity, &mut RoomZone)>,
    furniture: Query<(&Furniture, &InRoom)>,
    walls: Query<(&GridPosition, &Building, Option<&Material>)>,
    mut events: EventWriter<RoomEvaluatedEvent>,
    state: Option<ResMut<RoomZoneState>>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.as_deref().map_or(0, |t| t.tick);
    if let (Some(st), Some(_)) = (state.as_deref(), time.as_deref()) {
        if !st.dirty && tick > 0 && tick.saturating_sub(st.last_eval_tick) < EVAL_INTERVAL_TICKS {
            return;
        }
    }

    // Perimeter wall materials, keyed by tile (walls only).
    let mut wall_at: HashMap<(i32, i32), f32> = HashMap::new();
    for (pos, building, material) in &walls {
        if building.building_type == BuildingType::Wall {
            let value = material.map_or(0.0, |m| wall_material_value(m.0));
            wall_at.insert((pos.x, pos.y), value);
        }
    }

    for (room_entity, mut room) in &mut rooms {
        let mut furniture_quality = 0.0;
        for (furn, in_room) in &furniture {
            if in_room.0 == room_entity {
                furniture_quality += furn.quality;
            }
        }

        // Wall material: mean material value of the distinct wall tiles
        // enclosing the room (neighbours of room tiles that are not room
        // tiles themselves). No enclosing walls -> no bonus.
        let tile_set: HashSet<(i32, i32)> = room.tiles.iter().copied().collect();
        let mut perimeter: HashSet<(i32, i32)> = HashSet::new();
        for &(x, y) in &room.tiles {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let n = (x + dx, y + dy);
                if !tile_set.contains(&n) {
                    perimeter.insert(n);
                }
            }
        }
        let wall_values: Vec<f32> = perimeter
            .iter()
            .filter_map(|p| wall_at.get(p).copied())
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let wall_bonus = if wall_values.is_empty() {
            0.0
        } else {
            wall_values.iter().sum::<f32>() / wall_values.len() as f32
        };

        room.wall_bonus = wall_bonus;
        room.base_value = furniture_quality + wall_bonus;
        events.send(RoomEvaluatedEvent(room_entity));
    }

    if let Some(mut st) = state {
        st.last_eval_tick = tick;
        st.dirty = false;
    }
}

/// Applies room mood effects to assigned pops:
///
/// * High-value room (value > [`HIGH_VALUE_THRESHOLD`]): rest buff
///   (bedrooms/wards) or leisure buff (dining), plus a "Fine Room"
///   [`Morale`] modifier when the pop has one.
/// * Overcrowded barracks (effective occupants >
///   [`OVERCROWDED_LIMIT`]): leisure penalty, "Cramped Barracks" morale
///   modifier, and a [`MemoryType::CrampedBarracks`] memory.
/// * Jealousy: pops in poor rooms envy a legendary suite elsewhere —
///   leisure penalty, "Suite Envy" modifier, [`MemoryType::JealousOfSuite`].
///
/// In the live schedule this is cadence-gated to every
/// [`MOOD_INTERVAL_TICKS`] ticks; without a [`SimulationTime`] resource
/// (unit tests) it applies on every run.
#[allow(clippy::type_complexity)]
pub fn apply_room_mood_system(
    mut pops: Query<(
        &mut Needs,
        &AssignedRoom,
        Option<&mut Morale>,
        Option<&mut Memories>,
    )>,
    rooms: Query<(Entity, &RoomZone, Option<&OccupantCount>)>,
    assigned: Query<&AssignedRoom>,
    time: Option<Res<SimulationTime>>,
) {
    if let Some(t) = time.as_deref() {
        if !t.tick.is_multiple_of(MOOD_INTERVAL_TICKS) {
            return;
        }
    }
    let tick = time.as_deref().map_or(0, |t| t.tick);

    // Derived occupant counts + the colony's best room (for jealousy).
    let mut derived: HashMap<Entity, usize> = HashMap::new();
    for a in &assigned {
        *derived.entry(a.0).or_insert(0) += 1;
    }
    let best_value = rooms
        .iter()
        .map(|(_, r, _)| r.value())
        .fold(0.0_f32, f32::max);

    for (mut needs, assigned_room, morale, memories) in &mut pops {
        let Ok((_, room, stored)) = rooms.get(assigned_room.0) else {
            continue;
        };
        let value = room.value();
        let occupants = effective_occupant_count(
            derived.get(&assigned_room.0).copied().unwrap_or(0),
            stored,
        );
        let mut morale = morale;
        let mut memories = memories;

        if value > HIGH_VALUE_THRESHOLD {
            match room.zone_type {
                ZoneType::Dining => {
                    needs.leisure = (needs.leisure + ROOM_BUFF_DELTA).clamp(0.0, 1.0);
                }
                _ => {
                    needs.rest = (needs.rest + ROOM_BUFF_DELTA).clamp(0.0, 1.0);
                }
            }
            if let Some(m) = morale.as_deref_mut() {
                upsert_modifier(m, "Fine Room", 0.05, 60);
            }
        }

        if is_overcrowded(occupants) {
            needs.leisure = (needs.leisure - BARRACKS_PENALTY_DELTA).clamp(0.0, 1.0);
            if let Some(m) = morale.as_deref_mut() {
                upsert_modifier(m, "Cramped Barracks", -0.08, 60);
            }
            if let Some(mem) = memories.as_deref_mut() {
                if !mem.has_memory(MemoryType::CrampedBarracks) {
                    mem.add(MemoryType::CrampedBarracks, tick);
                }
            }
        } else if is_jealous(value, best_value) {
            // Jealousy only bites when you are not already miserable from
            // crowding — one complaint at a time.
            needs.leisure = (needs.leisure - JEALOUSY_PENALTY_DELTA).clamp(0.0, 1.0);
            if let Some(m) = morale.as_deref_mut() {
                upsert_modifier(m, "Suite Envy", -0.04, 60);
            }
            if let Some(mem) = memories.as_deref_mut() {
                if !mem.has_memory(MemoryType::JealousOfSuite) {
                    mem.add(MemoryType::JealousOfSuite, tick);
                }
            }
        }
    }
}

/// Bridges room evaluations into the Chronicle: a Major entry the first
/// time a room reaches legendary value, and a Minor warning the first
/// time a room reads as an overcrowded barracks.
pub fn room_zone_chronicle_bridge_system(
    mut events: EventReader<RoomEvaluatedEvent>,
    rooms: Query<&RoomZone>,
    assigned: Query<&AssignedRoom>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut state: ResMut<RoomChronicleState>,
) {
    let mut derived: HashMap<Entity, usize> = HashMap::new();
    for a in &assigned {
        *derived.entry(a.0).or_insert(0) += 1;
    }
    for event in events.read() {
        let room_entity = event.0;
        let Ok(room) = rooms.get(room_entity) else {
            continue;
        };
        let value = room.value();
        if value >= LEGENDARY_VALUE_THRESHOLD && state.celebrated.insert(room_entity) {
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "The {} is finished, and it is magnificent (room value {:.0}). The colony talks of little else — and those sleeping elsewhere talk most of all.",
                    zone_type_label(room.zone_type),
                    value
                ),
                importance: EventImportance::Major,
            });
        }
        let occupants = derived.get(&room_entity).copied().unwrap_or(0);
        if is_overcrowded(occupants) && state.warned_barracks.insert(room_entity) {
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "{} pops now share the {} ({} tiles). It is, officially, a barracks — privacy is a memory there.",
                    occupants,
                    zone_type_label(room.zone_type),
                    room.size()
                ),
                importance: EventImportance::Minor,
            });
        }
    }
}

/// Designates a room: spawns the [`RoomZone`] entity, paints its tiles
/// into the [`ZoneGrid`] (when present) so the per-tile view agrees, and
/// marks evaluation dirty. Used by the headless console and tests.
pub fn designate_room_zone(
    world: &mut World,
    zone_type: ZoneType,
    tiles: Vec<(i32, i32)>,
) -> Entity {
    if let Some(mut grid) = world.get_resource_mut::<ZoneGrid>() {
        for &(x, y) in &tiles {
            grid.set(x, y, zone_type);
        }
    }
    let entity = world.spawn(RoomZone::new(zone_type, tiles)).id();
    if let Some(mut state) = world.get_resource_mut::<RoomZoneState>() {
        state.dirty = true;
    }
    entity
}

/// Census of designated rooms for STATS / console panels.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoomZoneCensus {
    /// Number of designated rooms.
    pub rooms: usize,
    /// Mean room value across designated rooms (0.0 when none).
    pub avg_value: f32,
    /// Rooms currently overcrowded (derived occupants, incl. cache floor).
    pub barracks: usize,
    /// Pops assigned to any designated room.
    pub assigned: usize,
}

/// Computes the [`RoomZoneCensus`] from plain `World` access (headless /
/// tools friendly, mirroring `room_quality`'s world-level helpers).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn room_zone_census(world: &mut World) -> RoomZoneCensus {
    let mut derived: HashMap<Entity, usize> = HashMap::new();
    let mut assigned_total = 0usize;
    {
        let mut q = world.query::<&AssignedRoom>();
        for a in q.iter(world) {
            *derived.entry(a.0).or_insert(0) += 1;
            assigned_total += 1;
        }
    }
    let mut census = RoomZoneCensus {
        assigned: assigned_total,
        ..Default::default()
    };
    let mut total_value = 0.0f32;
    let mut q = world.query::<(Entity, &RoomZone, Option<&OccupantCount>)>();
    for (entity, room, stored) in q.iter(world) {
        census.rooms += 1;
        total_value += room.value();
        let occupants =
            effective_occupant_count(derived.get(&entity).copied().unwrap_or(0), stored);
        if is_overcrowded(occupants) {
            census.barracks += 1;
        }
    }
    if census.rooms > 0 {
        census.avg_value = total_value / census.rooms as f32;
    }
    census
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType, Material, MaterialType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::system::RunSystemOnce;

    fn needs_half() -> Needs {
        Needs {
            hunger: 0.5,
            rest: 0.5,
            leisure: 0.5,
            hygiene: 0.5,
        }
    }

    // Spec 381 RED test 1, adapted: spec's `Position` is the real
    // `GridPosition`; spec's Needs scale (0..50) is the real 0.0..=1.0
    // (see test_pop_room_mood_buff). App -> World + Schedule, the idiom
    // used across this codebase's layer-1 tests.
    #[test]
    fn test_room_zone_evaluation() {
        let mut world = World::new();
        world.init_resource::<Events<RoomEvaluatedEvent>>();
        let room_id = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(0, 0), (0, 1), (1, 0), (1, 1)],
                base_value: 0.0,
                wall_bonus: 0.0,
            })
            .id();
        world.spawn((
            Furniture { quality: 10.0 },
            GridPosition { x: 0, y: 0 },
            InRoom(room_id),
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_rooms_system);
        schedule.run(&mut world);

        let room = world.get::<RoomZone>(room_id).unwrap();
        // Size (4) + Furniture (10) = 14
        assert_eq!(room.value(), 14.0);
    }

    // Spec 381 RED test 2, adapted: base_value 50 + 1 tile = 51 > 20, so
    // the pop's rest rises above its 0.5 start (spec used a 0..50 scale).
    #[test]
    fn test_pop_room_mood_buff() {
        let mut world = World::new();
        let room_id = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(0, 0)],
                base_value: 50.0,
                wall_bonus: 0.0,
            })
            .id();
        let pop_id = world.spawn((needs_half(), AssignedRoom(room_id))).id();
        world.run_system_once(apply_room_mood_system).unwrap();
        let needs = world.get::<Needs>(pop_id).unwrap();
        assert!(needs.rest > 0.5, "rest {} should be buffed", needs.rest);
    }

    // Spec 381 RED test 3, adapted: OccupantCount(3) is the stored cache;
    // effective occupants = max(derived 1, stored 3) = 3 > 2, so leisure
    // takes the barracks privacy penalty.
    #[test]
    fn test_barracks_privacy_penalty() {
        let mut world = World::new();
        let room_id = world
            .spawn((
                RoomZone {
                    zone_type: ZoneType::Bedroom,
                    tiles: vec![(0, 0), (0, 1)],
                    base_value: 10.0,
                    wall_bonus: 0.0,
                },
                OccupantCount(3),
            ))
            .id();
        let pop_id = world.spawn((needs_half(), AssignedRoom(room_id))).id();
        world.run_system_once(apply_room_mood_system).unwrap();
        let needs = world.get::<Needs>(pop_id).unwrap();
        assert!(
            needs.leisure < 0.5,
            "leisure {} should be penalised",
            needs.leisure
        );
    }

    // REFACTOR requirement: overcrowding derived from AssignedRoom links
    // alone — no OccupantCount component anywhere.
    #[test]
    fn test_derived_occupant_count_overcrowding() {
        let mut world = World::new();
        let room_id = world
            .spawn(RoomZone::new(ZoneType::Bedroom, vec![(0, 0), (0, 1)]))
            .id();
        let mut pops = Vec::new();
        for _ in 0..3 {
            pops.push(world.spawn((needs_half(), AssignedRoom(room_id))).id());
        }
        world.run_system_once(apply_room_mood_system).unwrap();
        for pop in pops {
            let needs = world.get::<Needs>(pop).unwrap();
            assert!(needs.leisure < 0.5, "derived barracks penalty missing");
        }
        assert_eq!(
            derived_occupant_count(
                room_id,
                world
                    .query::<&AssignedRoom>()
                    .iter(&world)
                    .collect::<Vec<_>>()
                    .into_iter()
            ),
            3
        );
    }

    // Wall material seam: gold perimeter walls lift the room value via
    // MaterialType::beauty_modifier (Gold = 10), averaged over the
    // distinct enclosing wall tiles.
    #[test]
    fn test_wall_material_bonus() {
        let mut world = World::new();
        world.init_resource::<Events<RoomEvaluatedEvent>>();
        let room_id = world
            .spawn(RoomZone::new(ZoneType::Bedroom, vec![(1, 1)]))
            .id();
        for (x, y) in [(1, 0), (0, 1), (2, 1), (1, 2)] {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y },
                Material(MaterialType::Gold),
            ));
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_rooms_system);
        schedule.run(&mut world);
        let room = world.get::<RoomZone>(room_id).unwrap();
        assert_eq!(room.wall_bonus, 10.0);
        assert_eq!(room.value(), 11.0); // size 1 + gold walls 10
    }

    // Emergent flavour from the spec overview: pops get jealous of the
    // fine suite. The suite's resident is content; the hovel's is not.
    #[test]
    fn test_jealousy_of_high_value_suite() {
        let mut world = World::new();
        let suite = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(0, 0)],
                base_value: 59.0,
                wall_bonus: 0.0,
            })
            .id();
        let hovel = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(5, 5)],
                base_value: 4.0,
                wall_bonus: 0.0,
            })
            .id();
        let lucky = world.spawn((needs_half(), AssignedRoom(suite))).id();
        let envious = world.spawn((needs_half(), AssignedRoom(hovel))).id();
        world.run_system_once(apply_room_mood_system).unwrap();
        let lucky_needs = world.get::<Needs>(lucky).unwrap();
        assert!(lucky_needs.rest > 0.5);
        assert_eq!(lucky_needs.leisure, 0.5);
        let envious_needs = world.get::<Needs>(envious).unwrap();
        assert!(
            envious_needs.leisure < 0.5,
            "hovel pop should envy the suite"
        );
        assert_eq!(envious_needs.rest, 0.5);
    }

    // REFACTOR requirement: evaluation is throttled — a clean state
    // inside the interval skips work; the dirty flag forces it.
    #[test]
    fn test_evaluation_throttled_unless_dirty() {
        let mut world = World::new();
        world.init_resource::<Events<RoomEvaluatedEvent>>();
        world.insert_resource(RoomZoneState {
            dirty: false,
            last_eval_tick: 0,
        });
        world.insert_resource(SimulationTime {
            tick: 10,
            ..Default::default()
        });
        let room_id = world
            .spawn(RoomZone::new(ZoneType::Bedroom, vec![(0, 0)]))
            .id();
        world.spawn((Furniture { quality: 7.0 }, InRoom(room_id)));
        world.run_system_once(evaluate_rooms_system).unwrap();
        assert_eq!(world.get::<RoomZone>(room_id).unwrap().base_value, 0.0);
        world.resource_mut::<RoomZoneState>().dirty = true;
        world.run_system_once(evaluate_rooms_system).unwrap();
        assert_eq!(world.get::<RoomZone>(room_id).unwrap().base_value, 7.0);
    }

    // Mood application cadence: in the live schedule (SimulationTime
    // present) off-cadence ticks are a no-op; cadence ticks apply.
    #[test]
    fn test_mood_cadence_gate() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 7,
            ..Default::default()
        });
        let room_id = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(0, 0)],
                base_value: 50.0,
                wall_bonus: 0.0,
            })
            .id();
        let pop_id = world.spawn((needs_half(), AssignedRoom(room_id))).id();
        world.run_system_once(apply_room_mood_system).unwrap();
        assert_eq!(world.get::<Needs>(pop_id).unwrap().rest, 0.5);
        world.resource_mut::<SimulationTime>().tick = 50;
        world.run_system_once(apply_room_mood_system).unwrap();
        assert!(world.get::<Needs>(pop_id).unwrap().rest > 0.5);
    }

    // Chronicle bridge: a legendary room is celebrated exactly once.
    #[test]
    fn test_chronicle_bridge_celebrates_once() {
        let mut world = World::new();
        world.init_resource::<Events<RoomEvaluatedEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<RoomChronicleState>();
        let room_id = world
            .spawn(RoomZone {
                zone_type: ZoneType::Bedroom,
                tiles: vec![(0, 0)],
                base_value: 60.0,
                wall_bonus: 0.0,
            })
            .id();
        for _ in 0..2 {
            world
                .resource_mut::<Events<RoomEvaluatedEvent>>()
                .send(RoomEvaluatedEvent(room_id));
            world
                .run_system_once(room_zone_chronicle_bridge_system)
                .unwrap();
            world
                .resource_mut::<Events<RoomEvaluatedEvent>>()
                .update();
        }
        let state = world.resource::<RoomChronicleState>();
        assert_eq!(state.celebrated.len(), 1);
        assert!(state.celebrated.contains(&room_id));
    }

    // Designation paints the ZoneGrid and the census sees the room.
    #[test]
    fn test_designate_and_census() {
        let mut world = World::new();
        world.insert_resource(ZoneGrid::new(10, 10));
        world.init_resource::<RoomZoneState>();
        let room_id = designate_room_zone(
            &mut world,
            ZoneType::Dining,
            vec![(2, 2), (3, 2), (2, 3), (3, 3)],
        );
        assert_eq!(
            world.resource::<ZoneGrid>().get(2, 2),
            ZoneType::Dining
        );
        assert!(world.resource::<RoomZoneState>().dirty);
        world.spawn((needs_half(), AssignedRoom(room_id)));
        let census = room_zone_census(&mut world);
        assert_eq!(census.rooms, 1);
        assert_eq!(census.assigned, 1);
        assert_eq!(census.barracks, 0);
        assert_eq!(census.avg_value, 4.0);
    }

    // IP guard: public-facing room vocabulary stays generic/original —
    // no borrowed setting names in labels or chronicle phrasing.
    #[test]
    fn test_ip_guard_room_zone_strings() {
        const BANNED: &[&str] = &[
            "rimworld",
            "dwarf fortress",
            "windsor",
            "hogwarts",
            "downton",
        ];
        for zone in [
            ZoneType::Bedroom,
            ZoneType::Dining,
            ZoneType::Hospital,
            ZoneType::Office,
            ZoneType::Storage,
        ] {
            let label = zone_type_label(zone).to_lowercase();
            for banned in BANNED {
                assert!(!label.contains(banned), "label {label} tainted");
            }
        }
    }
}
