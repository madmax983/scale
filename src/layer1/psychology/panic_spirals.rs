//! # Panic Spirals (Spec 1371)
//!
//! Fear is contagious. Pops witnessing a "Terrifying Event" (Monster, Fire,
//! Corpse) flee in a random direction; pops sharing a tile with (or adjacent
//! to) a panicking pop catch the Panic state themselves. Panicked pops drop
//! whatever they are carrying and cannot work.
//!
//! Design tension: crowd control (wide halls break the chains) vs. efficiency
//! (tight spaces propagate the spiral).
//!
//! Interaction with Spec 1370 (Civilizational Compost): an unburied corpse is
//! a terrifying sight while it rots (decay < 1.0). Burying it (despawn via
//! `bury_corpse`) removes the terror; full decomposition (despawn) ends it.
//!
//! Counterplay built in:
//! - [`Panic`] carries a timer: pops calm down on their own.
//! - The [`Trait::Courage`] trait (see [`crate::layer1::psychology::traits`])
//!   resists cascades: courageous pops only panic at very close range and only
//!   catch spreading panic when surrounded by two or more panickers.
//! - [`Trait::Psychopath`] ("unaffected by horror") pops never panic at all.
//! - Spatial lookup is a one-pass `HashMap` tile grid, not O(N^2) pairwise.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::access_control::{AccessControl, AccessMode};
use crate::layer1::building::Building;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::culture::funeral::Corpse;
use crate::layer1::defense::is_walkable;
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::economy::items::CarryingItem;
use crate::layer1::eva_suit::{EvaSuit, EVA_AIR_LOW_FRACTION};
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::fauna::{Fauna, FaunaState};
use crate::layer1::health::{Dead, Health};
use crate::layer1::items::Equipment;
use crate::layer1::map::GridPosition;
use crate::layer1::nature::fire::Fire;
use crate::layer1::nature::terrain::TerrainType;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::pressure::{PressureGrid, SUFFOCATION_PRESSURE_THRESHOLD};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::social::ship_stowaway::Stowaway;
use crate::layer1::terrain::TerrainGrid;
use crate::layer1::utility_types::{ActionType, PopAction};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Witness radius (Chebyshev) for terrifying events: a pop this close to a
/// terrifying sighting panics.
pub const TERRIFY_RADIUS: u32 = 6;
/// Courageous pops only panic when the horror is very close.
pub const COURAGEOUS_TERRIFY_RADIUS: u32 = 2;
/// Ticks a panic lasts before the pop calms down.
pub const PANIC_DURATION_TICKS: u32 = 60;
/// A courageous pop catches spreading panic only when this many panicking
/// pops are adjacent (strength in numbers overwhelms courage).
pub const COURAGEOUS_SPREAD_OVERWHELM: usize = 2;
/// Maximum panic chronicle entries emitted per system run (spam guard — a
/// crowded colony discovering one corpse should not fill the chronicle).
pub const PANIC_CHRONICLE_CAP_PER_RUN: usize = 2;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Kind of a terrifying sighting, used for chronicle flavor text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrifyingKind {
    /// A monster: a hostile fauna (chasing/attacking) or a [`Terrifying`]
    /// marker spawned by an event.
    Monster,
    /// A burning fire.
    Fire,
    /// A rotting, unburied corpse.
    Corpse,
    /// A fellow colonist screaming and fleeing (panic spreading).
    PanickingPop,
}

impl TerrifyingKind {
    /// Short noun phrase for chronicle text.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Monster => "a monster",
            Self::Fire => "a fire",
            Self::Corpse => "a rotting corpse",
            Self::PanickingPop => "a screaming colonist",
        }
    }
}

/// Marker for an entity that is intrinsically terrifying to witness
/// (a monster sighting, a haunting apparition, a debug `scare` spawn).
#[derive(Component, Debug, Clone, Copy)]
pub struct Terrifying {
    /// What kind of horror this is (chronicle flavor).
    pub kind: TerrifyingKind,
}

/// A pop gripped by panic: flees, drops items, cannot work.
///
/// The timer ([`PANIC_DURATION_TICKS`]) counts down in
/// [`panic_timer_system`]; at zero the pop calms down and the component is
/// removed. `fled_from` remembers the sighting that caused the panic so the
/// flee system runs *away* from it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Panic {
    /// Ticks remaining before the pop calms down.
    pub timer: u32,
    /// Tile of the terrifying sighting that caused this panic.
    pub fled_from: GridPosition,
}

impl Panic {
    /// Create a fresh panic with the full duration.
    #[must_use]
    pub const fn new(fled_from: GridPosition) -> Self {
        Self {
            timer: PANIC_DURATION_TICKS,
            fled_from,
        }
    }
}

// ---------------------------------------------------------------------------
// Terrifying sightings
// ---------------------------------------------------------------------------

/// One terrifying thing a pop can witness: its tile and what it is.
#[derive(Debug, Clone, Copy)]
pub struct TerrifyingSighting {
    /// Tile the horror occupies.
    pub pos: GridPosition,
    /// What kind of horror it is.
    pub kind: TerrifyingKind,
}

/// Collect every terrifying sighting in the world right now.
///
/// Sources (Spec 1371's three kinds, adapted to the real codebase):
/// - entities carrying [`Terrifying`] (monster sightings / event spawns),
/// - [`Fire`] entities (burning fires),
/// - [`Corpse`] entities with `decay < 1.0` (rotting, unburied corpses —
///   burial despawns the corpse via `bury_corpse`, full decomposition
///   despawns it via compost; either way the terror ends),
/// - [`Fauna`] in `Chase`/`Attack` state (hostile monsters in the flesh).
///
/// Uses one-pass queries plus a `HashSet` dedupe — no O(N^2) pairwise work.
#[must_use]
pub fn collect_terrifying_sightings(world: &mut World) -> Vec<TerrifyingSighting> {
    let mut out = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let push = |out: &mut Vec<TerrifyingSighting>,
                    seen: &mut HashSet<(i32, i32)>,
                    pos: GridPosition,
                    kind: TerrifyingKind| {
        if seen.insert((pos.x, pos.y)) {
            out.push(TerrifyingSighting { pos, kind });
        }
    };

    {
        let mut q = world.query::<(&GridPosition, &Terrifying)>();
        for (pos, terrifying) in q.iter(world) {
            push(&mut out, &mut seen, *pos, terrifying.kind);
        }
    }
    {
        let mut q = world.query_filtered::<&GridPosition, With<Fire>>();
        for pos in q.iter(world) {
            push(&mut out, &mut seen, *pos, TerrifyingKind::Fire);
        }
    }
    {
        let mut q = world.query::<(&GridPosition, &Corpse)>();
        for (pos, corpse) in q.iter(world) {
            // A fully-rotted corpse is compost (despawned by the compost
            // system); only the rotting one is a horror.
            if corpse.decay < 1.0 {
                push(&mut out, &mut seen, *pos, TerrifyingKind::Corpse);
            }
        }
    }
    {
        let mut q = world.query::<(&GridPosition, &Fauna)>();
        for (pos, fauna) in q.iter(world) {
            if matches!(fauna.state, FaunaState::Chase | FaunaState::Attack) {
                push(&mut out, &mut seen, *pos, TerrifyingKind::Monster);
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Item 97: vacuum survival (balance Bug #4 — the panic trap)
// ---------------------------------------------------------------------------
//
// Balance evidence (2026-10-17, 1,819 vacuum episodes / 60 colonies):
// 83% of fatal episodes include panic; suited deaths end at air=0 after
// episodes equal to full bottle + full HP — pops never turned back; and
// fatal episodes run 150-295 ticks against a 60-tick panic timer.
// Forensics on trunk:
// - `panic_flee_system` chose flee tiles by distance from the horror
//   only (`is_walkable` was the sole filter) — no `PressureGrid` read
//   anywhere in this module, so flight could run *deeper* into vacuum.
// - `try_panic_pop` rejected only already-panicked pops, so a pop whose
//   timer expired in vacuum next to the same corpse re-panicked the
//   same tick — chained 60t panics, not a timer refresh (the existing
//   `test_try_panic_pop_rejects_already_panicked` pins no-refresh).
// - No return behaviour existed anywhere: `suit_air_system`
//   (eva_suit.rs) only refills in pressure, `pressure_damage_system`
//   only drains/damages, and utility AI excludes `Panic` pops entirely
//   while Idle pops hold no `MovementTarget` — a pop in vacuum had no
//   system at all moving it toward air.
// Fix (survival first, indoors unchanged): panic cannot start in
// vacuum, panic flight in vacuum heads for pressure, flee steps never
// enter vacuum when a pressurized step exists, and a non-panicked pop
// that is suffocating (unsuited / dry suit) or on a low bottle gains a
// return-to-pressure drive in [`vacuum_return_system`].

/// Pressure at a tile, if a [`PressureGrid`] exists in the world.
fn pressure_at(world: &World, pos: GridPosition) -> Option<f32> {
    world
        .get_resource::<PressureGrid>()
        .map(|grid| grid.get(pos.x, pos.y))
}

/// Is this tile vacuum (below the suffocation threshold)?
fn in_vacuum(world: &World, pos: GridPosition) -> bool {
    pressure_at(world, pos).is_some_and(|p| p < SUFFOCATION_PRESSURE_THRESHOLD)
}

/// Multi-source BFS distance field: for every tile, how many steps
/// (8-directional) to the nearest pressurized tile. `-1` = unreachable
/// (or out of bounds). Computed once per system run and shared by every
/// fleeing/returning pop — O(W*H) terrain lookups plus one building
/// snapshot, instead of a per-pop search.
struct PressureField {
    width: usize,
    height: usize,
    dist: Vec<i32>,
}

impl PressureField {
    fn dist_at(&self, x: i32, y: i32) -> i32 {
        if x < 0 || y < 0 {
            return -1;
        }
        let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) else {
            return -1;
        };
        if ux >= self.width || uy >= self.height {
            return -1;
        }
        self.dist.get(uy * self.width + ux).copied().unwrap_or(-1)
    }
}

fn pressure_field(world: &mut World) -> Option<PressureField> {
    // Snapshot everything the BFS needs, then drop the resource borrows.
    let (width, height, pressurized) = {
        let grid = world.get_resource::<PressureGrid>()?;
        let mut sources = Vec::new();
        for y in 0..grid.height {
            for x in 0..grid.width {
                if grid.get(x as i32, y as i32) >= SUFFOCATION_PRESSURE_THRESHOLD {
                    sources.push((x as i32, y as i32));
                }
            }
        }
        (grid.width, grid.height, sources)
    };
    // Tiles a pop cannot step onto because a building blocks them
    // (public access points — airlocks, gates — stay passable, matching
    // `is_tile_walkable_data`).
    let mut walkable_tiles: Vec<bool> = {
        let terrain = world.get_resource::<TerrainGrid>();
        let mut v = vec![true; width * height];
        if let Some(t) = terrain {
            for y in 0..height {
                for x in 0..width {
                    v[y * width + x] = t.get(x, y).is_some_and(TerrainType::is_walkable);
                }
            }
        }
        v
    };
    {
        let mut q = world.query::<(&GridPosition, &Building, Option<&AccessControl>)>();
        for (pos, building, access) in q.iter(world) {
            let public = matches!(access, Some(a) if a.mode == AccessMode::Public);
            if building.building_type.is_obstacle() && !public {
                if let (Ok(ux), Ok(uy)) = (usize::try_from(pos.x), usize::try_from(pos.y)) {
                    if ux < width && uy < height {
                        walkable_tiles[uy * width + ux] = false;
                    }
                }
            }
        }
    }
    let walkable = |x: i32, y: i32, tw: &[bool]| -> bool {
        let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) else {
            return false;
        };
        ux < width && uy < height && tw[uy * width + ux]
    };

    const DIRS: [(i32, i32); 8] = [
        (-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1),
    ];
    let mut dist = vec![-1_i32; width * height];
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for (x, y) in pressurized {
        let idx = (y as usize) * width + (x as usize);
        if dist[idx] < 0 {
            dist[idx] = 0;
            queue.push_back((x, y));
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let d = dist[(y as usize) * width + (x as usize)];
        for (dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if !walkable(nx, ny, &walkable_tiles) {
                continue;
            }
            let nidx = (ny as usize) * width + (nx as usize);
            if dist[nidx] < 0 {
                dist[nidx] = d + 1;
                queue.push_back((nx, ny));
            }
        }
    }
    Some(PressureField {
        width,
        height,
        dist,
    })
}

/// One walkable step from `pos` that strictly reduces the BFS distance
/// to pressure (any reachable neighbour counts when the pop stands on
/// an unreachable tile, e.g. force-placed inside a blocked cell).
fn step_toward_pressure(
    world: &mut World,
    field: &PressureField,
    pos: GridPosition,
) -> Option<(i32, i32)> {
    const DIRS: [(i32, i32); 8] = [
        (-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1),
    ];
    let current = field.dist_at(pos.x, pos.y);
    let mut best: Option<(i32, i32, i32)> = None;
    for (dx, dy) in DIRS {
        let (nx, ny) = (pos.x + dx, pos.y + dy);
        let d = field.dist_at(nx, ny);
        if d < 0 || (current >= 0 && d >= current) {
            continue;
        }
        if !is_walkable(world, nx, ny) {
            continue;
        }
        if best.is_none_or(|(_, _, bd)| d < bd) {
            best = Some((nx, ny, d));
        }
    }
    best.map(|(x, y, _)| (x, y))
}

// ---------------------------------------------------------------------------
// Panic application
// ---------------------------------------------------------------------------

/// Chronicle helper: init the resource if missing (tests), then send.
fn send_panic_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world.resource_mut::<Events<AddChronicleEvent>>().send(AddChronicleEvent {
        text: text.to_string(),
        importance,
    });
}

/// Drop whatever the pop is carrying at their tile (the hauling payload goes
/// everywhere — this is the emergent "dropped the explosive payload" hook).
///
/// Mirrors the drop half of `hauling::drop_item_on_ground`: the item entity is
/// re-grounded at the pop's tile, the pop's [`CarryingItem`] is removed, and
/// any in-flight movement targets are abandoned.
fn drop_carried_items(world: &mut World, pop: Entity, pos: GridPosition) {
    let carrying = world.get::<CarryingItem>(pop).copied();
    let Some(CarryingItem(item)) = carrying else {
        return;
    };
    if world.get_entity(item).is_ok() {
        world.entity_mut(item).insert(pos);
        // Photophobic items track a container parent; dropping to the ground
        // clears that.
        world
            .entity_mut(item)
            .remove::<crate::layer1::environment::photophobic::Parent>();
    }
    world.entity_mut(pop).remove::<CarryingItem>();
    world.entity_mut(pop).remove::<MovementTarget>();
    world.entity_mut(pop).remove::<AtTarget>();
}

/// Try to push a pop into the [`Panic`] state.
///
/// Returns `true` when the panic took hold. Resistance rules:
/// - [`Trait::Psychopath`] pops are immune ("unaffected by horror");
/// - [`Trait::Courage`] pops only panic from direct sightings within
///   [`COURAGEOUS_TERRIFY_RADIUS`], and only catch *spread* panic when
///   surrounded by at least [`COURAGEOUS_SPREAD_OVERWHELM`] panickers
///   (checked by the caller via `adjacent_panickers`).
///
/// On success the pop abandons work ([`ActionType::Idle`]), drops carried
/// items, and a chronicle entry fires (spam-capped by the caller).
pub fn try_panic_pop(
    world: &mut World,
    pop: Entity,
    sighting: TerrifyingSighting,
    distance: u32,
    is_spread: bool,
    adjacent_panickers: usize,
) -> bool {
    if world.get::<Panic>(pop).is_some() {
        return false;
    }
    // Item 97: survival first — a pop already in vacuum never (re)panics.
    // Without this, a pop whose 60-tick timer expired in vacuum next to
    // the same corpse re-panicked the same tick, chaining panics into
    // the 150-295 tick fatal episodes the balance probe measured.
    // Pressurized pops are unaffected: the colony stays brutal indoors.
    if let Some(pos) = world.get::<GridPosition>(pop).copied() {
        if in_vacuum(world, pos) {
            return false;
        }
    }
    let traits = world.get::<Traits>(pop).cloned().unwrap_or_default();
    if traits.has(Trait::Psychopath) {
        return false;
    }
    if traits.has(Trait::Courage) {
        if is_spread {
            if adjacent_panickers < COURAGEOUS_SPREAD_OVERWHELM {
                return false;
            }
        } else if distance > COURAGEOUS_TERRIFY_RADIUS {
            return false;
        }
    }

    world.entity_mut(pop).insert(Panic::new(sighting.pos));
    drop_carried_items(
        world,
        pop,
        world.get::<GridPosition>(pop).copied().unwrap_or_default(),
    );
    // Panicked pops abandon work: the utility AI never picks them up again
    // (see the `Without<Panic>` filter in `collect_pop_data`), and their
    // current action is reset so no in-flight work system keeps driving them.
    if let Some(mut action) = world.get_mut::<PopAction>(pop) {
        action.current = ActionType::Idle;
        action.current_utility = 0.0;
        action.ticks_committed = 0;
    }
    true
}

/// Chronicle the panic of a pop, honoring the per-run spam cap.
fn chronicle_panic(
    world: &mut World,
    pop: Entity,
    sighting: TerrifyingSighting,
    emitted: &mut usize,
) {
    if *emitted >= PANIC_CHRONICLE_CAP_PER_RUN {
        return;
    }
    *emitted += 1;
    let name = world
        .get::<PopName>(pop)
        .map_or_else(|| "Someone".to_string(), |n| n.0.clone());
    send_panic_chronicle(
        world,
        &format!("{name} panics at the sight of {}!", sighting.kind.label()),
        EventImportance::Minor,
    );
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Trigger: pops witnessing a terrifying event panic.
///
/// Each calm pop within [`TERRIFY_RADIUS`] (Chebyshev) of a sighting is
/// offered panic via [`try_panic_pop`] (courage/psychopath resistance
/// applies). Chronicle output is spam-capped.
pub fn trigger_panic_system(world: &mut World) {
    let sightings = collect_terrifying_sightings(world);
    if sightings.is_empty() {
        return;
    }
    let pops: Vec<(Entity, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition), (With<Pop>, Without<Panic>)>();
        q.iter(world).map(|(e, p)| (e, *p)).collect()
    };
    let mut emitted = 0;
    for (pop, pos) in pops {
        for sighting in &sightings {
            let distance = pos.distance_chebyshev(sighting.pos);
            if distance <= TERRIFY_RADIUS
                && try_panic_pop(world, pop, *sighting, distance, false, 0)
            {
                chronicle_panic(world, pop, *sighting, &mut emitted);
                break;
            }
        }
    }
}

/// Spread: pops sharing a tile with (or adjacent to) a panicking pop catch
/// the panic themselves.
///
/// Spatial lookup is a one-pass tile `HashMap`, not O(N^2) pairwise checks:
/// build calm-pop buckets per tile once, then probe the 3x3 neighborhood of
/// each panicker.
pub fn spread_panic_system(world: &mut World) {
    let panickers: Vec<(Entity, GridPosition, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition, &Panic), With<Pop>>();
        q.iter(world)
            .map(|(e, pos, panic)| (e, *pos, panic.fled_from))
            .collect()
    };
    if panickers.is_empty() {
        return;
    }
    // Calm pops, bucketed by tile.
    let mut calm_by_tile: HashMap<(i32, i32), Vec<Entity>> = HashMap::new();
    {
        let mut q = world.query_filtered::<(Entity, &GridPosition), (With<Pop>, Without<Panic>)>();
        for (e, pos) in q.iter(world) {
            calm_by_tile.entry((pos.x, pos.y)).or_default().push(e);
        }
    }
    let mut emitted = 0;
    // Accumulate adjacent-panicker counts across ALL panickers first: a
    // courageous pop is overwhelmed only when the crowd of panickers around
    // it (not the neighborhood of a single panicker) is big enough.
    let mut candidates: HashMap<Entity, usize> = HashMap::new();
    let mut origins: HashMap<Entity, GridPosition> = HashMap::new();
    for (_panicker, pos, fled_from) in panickers {
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ents) = calm_by_tile.get(&(pos.x + dx, pos.y + dy)) {
                    for e in ents {
                        *candidates.entry(*e).or_default() += 1;
                        origins.entry(*e).or_insert(fled_from);
                    }
                }
            }
        }
    }
    for (candidate, adjacent) in candidates {
        let fled_from = origins.get(&candidate).copied().unwrap_or_default();
        let spread_sighting = TerrifyingSighting {
            // The new panicker flees *away from the horror that started the
            // chain*, not from the panicker that scared them.
            pos: fled_from,
            kind: TerrifyingKind::PanickingPop,
        };
        if try_panic_pop(world, candidate, spread_sighting, 1, true, adjacent) {
            chronicle_panic(
                world,
                candidate,
                TerrifyingSighting {
                    pos: fled_from,
                    kind: TerrifyingKind::PanickingPop,
                },
                &mut emitted,
            );
        }
    }
}

/// Panic timer: pops calm down after [`PANIC_DURATION_TICKS`].
pub fn panic_timer_system(world: &mut World) {
    let mut calmed: Vec<(Entity, String)> = Vec::new();
    {
        let mut q = world.query_filtered::<(Entity, &mut Panic, Option<&PopName>), With<Pop>>();
        for (e, mut panic, name) in q.iter_mut(world) {
            panic.timer = panic.timer.saturating_sub(1);
            if panic.timer == 0 {
                calmed.push((
                    e,
                    name.map_or_else(|| "Someone".to_string(), |n| n.0.clone()),
                ));
            }
        }
    }
    for (e, name) in calmed {
        world.entity_mut(e).remove::<Panic>();
        send_panic_chronicle(
            world,
            &format!("{name} catches their breath and calms down."),
            EventImportance::Minor,
        );
    }
}

/// Flee: panicked pops move one tile per tick away from the horror.
///
/// Picks a random walkable neighbor maximizing Chebyshev distance from the
/// `fled_from` tile; falls back to any random walkable neighbor; stands
/// still if boxed in.
pub fn panic_flee_system(world: &mut World) {
    const DIRS: [(i32, i32); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];
    let panickers: Vec<(Entity, GridPosition, GridPosition)> = {
        // Item 110 Gap 2: a pop answering a Douse designation
        // ([`FireBrigade`]) does not flee — brigade duty overrides panic
        // flight for that pop only. Everyone else flees exactly as before.
        let mut q = world.query_filtered::<
            (Entity, &GridPosition, &Panic),
            (With<Pop>, Without<FireBrigade>),
        >();
        q.iter(world)
            .map(|(e, pos, panic)| (e, *pos, panic.fled_from))
            .collect()
    };
    // Item 97: one shared distance-to-pressure field per run.
    let field = pressure_field(world);
    for (e, pos, fled_from) in panickers {
        // Item 97: a panicked pop IN vacuum flees toward air, not away
        // from the horror — away-from-horror flight is the trap that
        // ran pops deeper out until the bottle and their HP were gone.
        if in_vacuum(world, pos) {
            if let Some(f) = &field {
                if let Some((nx, ny)) = step_toward_pressure(world, f, pos) {
                    if let Some(mut grid_pos) = world.get_mut::<GridPosition>(e) {
                        grid_pos.x = nx;
                        grid_pos.y = ny;
                    }
                    continue;
                }
            }
        }
        let old_distance = pos.distance_chebyshev(fled_from);
        let mut walkable: Vec<(i32, i32, u32)> = Vec::new();
        for (dx, dy) in DIRS {
            let (nx, ny) = (pos.x + dx, pos.y + dy);
            if is_walkable(world, nx, ny) {
                let d = GridPosition { x: nx, y: ny }.distance_chebyshev(fled_from);
                walkable.push((nx, ny, d));
            }
        }
        if walkable.is_empty() {
            continue;
        }
        // Item 97: never flee INTO vacuum when a pressurized step
        // exists (e.g. panicking at the habitat edge, horror inside).
        if world.get_resource::<PressureGrid>().is_some() {
            let pressurized: Vec<(i32, i32, u32)> = walkable
                .iter()
                .copied()
                .filter(|(x, y, _)| {
                    !in_vacuum(world, GridPosition { x: *x, y: *y })
                })
                .collect();
            if !pressurized.is_empty() {
                walkable = pressurized;
            }
        }
        // Prefer tiles strictly farther from the horror; fall back to any
        // walkable tile so the pop at least keeps moving.
        let best_distance = walkable
            .iter()
            .map(|(_, _, d)| *d)
            .max()
            .unwrap_or(old_distance);
        let away: Vec<(i32, i32)> = walkable
            .iter()
            .filter(|(_, _, d)| *d == best_distance && *d > old_distance)
            .map(|(x, y, _)| (*x, *y))
            .collect();
        let pool: Vec<(i32, i32)> = if away.is_empty() {
            walkable.iter().map(|(x, y, _)| (*x, *y)).collect()
        } else {
            away
        };
        let idx = rand::thread_rng().gen_range(0..pool.len());
        let (nx, ny) = pool[idx];
        if let Some(mut grid_pos) = world.get_mut::<GridPosition>(e) {
            grid_pos.x = nx;
            grid_pos.y = ny;
        }
    }
}

/// Return to pressure (Item 97): a non-panicked pop in vacuum that is
/// suffocating — no suit, or a suit run dry — or whose bottle is at/below
/// the low-air fraction ([`EVA_AIR_LOW_FRACTION`], the existing warning
/// threshold) abandons its plan and moves one tile per tick toward the
/// nearest pressurized tile, until it is back in air.
///
/// This is the missing half the balance probe measured: healthy suited
/// pops are left alone (legitimate EVA work continues), pressurized
/// pops are untouched, and panicked pops are handled by the
/// hazard-aware [`panic_flee_system`] instead. Player-possessed pops,
/// dormant origin pops, the phased Chronostalker, and pops in
/// cryo-stasis are never dragged by this drive. The dormant exemption
/// matters (Item 100): the land-origin sites (cradle, scar,
/// moment-wound) hold no pressure envelope — the vessel origins write
/// theirs into the PressureGrid, these do not — so a dormant pop DOES
/// read as vacuum here; it is exempt because it waits in stasis
/// (pressure and thermal damage both spare the dormant state) and its
/// origin tether would fight this drive tile-for-tile in vacuum.
pub fn vacuum_return_system(world: &mut World) {
    let Some(field) = pressure_field(world) else {
        return;
    };
    let pops: Vec<(Entity, GridPosition)> = {
        let mut q =
            world.query_filtered::<(Entity, &GridPosition), (With<Pop>, Without<Panic>)>();
        q.iter(world).map(|(e, p)| (e, *p)).collect()
    };
    for (e, pos) in pops {
        if !in_vacuum(world, pos) {
            continue;
        }
        if world.get::<Dead>(e).is_some() {
            continue;
        }
        if world.get::<Health>(e).is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        if world
            .get::<crate::layer1::direct_link::Possessed>(e)
            .is_some()
            || world
                .get::<crate::layer1::culture::lawbound::DormantAutomaton>(e)
                .is_some()
            || world
                .get::<crate::layer1::culture::bloomtouched::DormantTouched>(e)
                .is_some()
            || world
                .get::<crate::layer1::culture::chronostalker::Chronostalker>(e)
                .is_some()
            || world
                .get::<crate::layer1::culture::chronostalker::DormantStalker>(e)
                .is_some()
            || world
                .get::<crate::layer1::culture::chronostalker::Phased>(e)
                .is_some()
            || world.get::<crate::layer1::cryo::CryoStasis>(e).is_some()
        {
            continue;
        }
        // Must this pop turn back? Unsuited and dry-suited pops are
        // taking pressure damage; a low bottle is the turn-back line.
        // A healthy bottle means the pop may be out on purpose.
        let must_return = match world
            .get::<Equipment>(e)
            .and_then(|eq| eq.body)
            .and_then(|body| world.get::<EvaSuit>(body))
        {
            None => true,
            Some(suit) => suit.air <= 0.0 || suit.air_fraction() <= EVA_AIR_LOW_FRACTION,
        };
        if !must_return {
            continue;
        }
        if let Some((nx, ny)) = step_toward_pressure(world, &field, pos) {
            // Survival overrides the plan: drop any movement order that
            // would drag the pop back out, then take the step.
            world.entity_mut(e).remove::<MovementTarget>();
            world.entity_mut(e).remove::<AtTarget>();
            if let Some(mut grid_pos) = world.get_mut::<GridPosition>(e) {
                grid_pos.x = nx;
                grid_pos.y = ny;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Item 110 Gap 2: the fire brigade — Douse stays executable under panic
// ---------------------------------------------------------------------------
//
// Balance forensics 2026-10-20 (l2r2k_11): the one fire counterplay verb,
// the Douse designation, is executed through utility AI — and utility AI
// excludes panicked pops (`Without<Panic>` in `collect_pop_data`). A fire
// panics every pop within radius 6 for 60 ticks, longer than the whole
// observed cascade (~40 ticks): at the first sample after ignition 26/27
// pops were panicking and nobody could douse. The brigade is the minimal
// seam that fixes exactly that, in the style of [`vacuum_return_system`]
// (a stateless world-system drive that bypasses utility AI for one
// survival verb):
//
// - A Douse designation on a burning tile drafts the nearest eligible
//   living pop as its brigadier — panicked or not. Panic itself is
//   untouched: radius, duration, spread, and the utility-AI exclusion
//   all stay exactly as specced (1371); the brigadier still carries
//   their [`Panic`] and still calms on the normal timer.
// - The brigadier is marked [`FireBrigade`]: `panic_flee_system` skips
//   them (duty overrides flight for that one pop), and this system steps
//   them one walkable tile per tick toward the fire — never onto a
//   burning tile — and douses from adjacency (Chebyshev <= 1), the same
//   effect as `execution::general_work::handle_douse` (every Fire on the
//   tile despawned, designation completed).
// - Undesignated fires draft nobody: without a keeper's Douse order the
//   colony's panic behavior is unchanged. Non-panicked pops were never
//   blocked (utility AI already routes them to Douse designations);
//   this system simply guarantees the designation stays executable when
//   the only nearby pops are the panicking ones.
// - Markers are recomputed from scratch every tick (nearest-pop draft),
//   so a completed designation, an extinguished fire, or a dead
//   brigadier never leaves a stale exemption behind.

/// Marker: this pop is answering a Douse designation (Item 110 Gap 2).
///
/// Inserted and removed only by [`fire_brigade_system`], recomputed every
/// tick. While held, `panic_flee_system` skips the pop and the brigade
/// system walks them toward their designated fire.
#[derive(Component, Debug, Clone, Copy)]
pub struct FireBrigade;

/// Is this pop eligible for brigade draft? Mirrors the exemption list of
/// [`vacuum_return_system`] (possessed, dormant origin pops, the phased
/// stalker, cryo) plus the dead, the healthless-dead, and hidden
/// stowaways — a survival drive never drafts pops another system owns.
fn brigade_eligible(world: &World, pop: Entity) -> bool {
    if world.get::<Dead>(pop).is_some() {
        return false;
    }
    if world.get::<Health>(pop).is_some_and(|h| h.current <= 0.0) {
        return false;
    }
    world.get::<crate::layer1::direct_link::Possessed>(pop).is_none()
        && world
            .get::<crate::layer1::culture::lawbound::DormantAutomaton>(pop)
            .is_none()
        && world
            .get::<crate::layer1::culture::bloomtouched::DormantTouched>(pop)
            .is_none()
        && world
            .get::<crate::layer1::culture::chronostalker::Chronostalker>(pop)
            .is_none()
        && world
            .get::<crate::layer1::culture::chronostalker::DormantStalker>(pop)
            .is_none()
        && world
            .get::<crate::layer1::culture::chronostalker::Phased>(pop)
            .is_none()
        && world.get::<crate::layer1::cryo::CryoStasis>(pop).is_none()
        && world.get::<Stowaway>(pop).is_none()
}

/// Douse every Fire on `pos` for brigadier `pop`, completing the
/// designation — the same effect as the utility-AI work path
/// (`handle_douse`), reached here without utility AI.
fn brigade_douse(world: &mut World, pop: Entity, designation: Entity, pos: GridPosition) {
    let fires: Vec<Entity> = {
        let mut q = world.query::<(Entity, &GridPosition, &Fire)>();
        q.iter(world)
            .filter(|(_, fire_pos, _)| **fire_pos == pos)
            .map(|(entity, _, _)| entity)
            .collect()
    };
    for fire in fires {
        world.despawn(fire);
    }
    if world.get_entity(designation).is_ok() {
        world.despawn(designation);
    }
    if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
        log.add(format!("Doused the fire at ({}, {})", pos.x, pos.y));
    }
    world.entity_mut(pop).remove::<FireBrigade>();
}

/// Fire brigade: Douse designations draft their nearest eligible pop —
/// panicked or not — and walk them to the fire (Item 110 Gap 2).
pub fn fire_brigade_system(world: &mut World) {
    const DIRS: [(i32, i32); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];

    // Burning tiles right now.
    let fire_tiles: HashSet<(i32, i32)> = {
        let mut q = world.query_filtered::<&GridPosition, With<Fire>>();
        q.iter(world).map(|p| (p.x, p.y)).collect()
    };

    // Douse duties: designations sitting on a burning tile.
    let mut duties: Vec<(Entity, GridPosition)> = {
        let mut q = world.query::<(Entity, &GridPosition, &Designation)>();
        q.iter(world)
            .filter(|(_, _, des)| des.designation_type == DesignationType::Douse)
            .filter(|(_, pos, _)| fire_tiles.contains(&(pos.x, pos.y)))
            .map(|(e, pos, _)| (e, *pos))
            .collect()
    };
    duties.sort_by_key(|(e, _)| e.index());

    // Candidate workforce: every living pop, panicked included — that
    // inclusion is the entire point of the brigade.
    let mut pops: Vec<(Entity, GridPosition)> = {
        let mut q = world.query_filtered::<(Entity, &GridPosition), With<Pop>>();
        q.iter(world).map(|(e, p)| (e, *p)).collect()
    };
    pops.retain(|(e, _)| brigade_eligible(world, *e));
    pops.sort_by_key(|(e, _)| e.index());

    // Draft: nearest eligible pop per duty, one duty per pop.
    let mut assigned: HashMap<Entity, (Entity, GridPosition)> = HashMap::new();
    let mut taken: HashSet<Entity> = HashSet::new();
    for (designation, duty_pos) in &duties {
        let nearest = pops
            .iter()
            .filter(|(e, _)| !taken.contains(e))
            .min_by_key(|(_, pos)| (pos.distance_chebyshev(*duty_pos), pos.x, pos.y));
        if let Some((pop, _)) = nearest {
            assigned.insert(*pop, (*designation, *duty_pos));
            taken.insert(*pop);
        }
    }

    // Refresh markers from scratch: holders not drafted this tick lose
    // the marker (their panic flight resumes); draftees gain it.
    let holders: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<FireBrigade>>();
        q.iter(world).collect()
    };
    for holder in holders {
        if !assigned.contains_key(&holder) {
            world.entity_mut(holder).remove::<FireBrigade>();
        }
    }
    for pop in assigned.keys() {
        world.entity_mut(*pop).insert(FireBrigade);
    }

    // Act: douse from adjacency, otherwise step toward the fire.
    for (pop, (designation, duty_pos)) in &assigned {
        let Some(pos) = world.get::<GridPosition>(*pop).copied() else {
            continue;
        };
        if pos.distance_chebyshev(*duty_pos) <= 1 {
            brigade_douse(world, *pop, *designation, *duty_pos);
            continue;
        }
        // One walkable step that strictly closes the distance, never
        // onto a burning tile (the brigadier works from adjacency —
        // stepping into 5 HP/tick fire to reach it would be a death
        // sentence, not counterplay).
        let current = pos.distance_chebyshev(*duty_pos);
        let mut best: Option<(i32, i32, u32)> = None;
        for (dx, dy) in DIRS {
            let (nx, ny) = (pos.x + dx, pos.y + dy);
            if fire_tiles.contains(&(nx, ny)) {
                continue;
            }
            let d = GridPosition { x: nx, y: ny }.distance_chebyshev(*duty_pos);
            if d >= current || !is_walkable(world, nx, ny) {
                continue;
            }
            if best.is_none_or(|(_, _, bd)| d < bd) {
                best = Some((nx, ny, d));
            }
        }
        if let Some((nx, ny, _)) = best {
            if let Some(mut grid_pos) = world.get_mut::<GridPosition>(*pop) {
                grid_pos.x = nx;
                grid_pos.y = ny;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Headless helpers
// ---------------------------------------------------------------------------

/// Spawn a [`Terrifying`] sighting at a tile (headless `scare` command,
/// playtests).
pub fn spawn_terrifying(world: &mut World, x: i32, y: i32, kind: TerrifyingKind) -> Entity {
    world
        .spawn((Terrifying { kind }, GridPosition { x, y }))
        .id()
}

/// Count currently panicking pops (headless STATS `panicking=`).
///
/// This is a *census* count: it uses exactly the population the headless
/// STATS `pops=` field reports — entities with [`Pop`], [`Health`] and a
/// [`GridPosition`], excluding hidden [`Stowaway`] pops (Spec 327: the
/// census never counts stowaways) and [`Dead`] pops awaiting despawn —
/// plus the [`Panic`] component. `panicking=` is therefore always a true
/// subset count of `pops=` (`panicking <= pops`).
///
/// The previous filter (`With<Pop>, With<Panic>` only) counted panicking
/// stowaways that `pops=` deliberately hides, and any health-less /
/// position-less `Pop` entity, so a scare near hidden stowaways reported
/// e.g. `panicking=17` against `pops=15` (Item 85). Stowaways still panic
/// and flee like anyone else — the panic *systems* are unchanged; only
/// this instrument was lying about which population it counted.
#[must_use]
pub fn panicking_count(world: &mut World) -> usize {
    let mut q = world.query_filtered::<
        Entity,
        (
            With<Pop>,
            With<Panic>,
            With<Health>,
            With<GridPosition>,
            Without<Stowaway>,
            Without<Dead>,
        ),
    >();
    q.iter(world).count()
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD: RED first, adapted to the real codebase)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::{Item, ItemType};
    use crate::layer1::nature::terrain::{
        generate_terrain, TerrainGrid, TerrainType,
    };
    use crate::shared::time::SimulationTime;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        // Walkable everywhere, so the flee system has somewhere to run.
        world.insert_resource(generate_terrain(60, 60));
        world
    }

    /// A world with a fully open, deterministic field: every tile walkable
    /// Grass (no random rocks/water from `generate_terrain`).
    fn setup_open_field() -> World {
        let mut world = setup();
        world.insert_resource(TerrainGrid {
            width: 60,
            height: 60,
            tiles: vec![TerrainType::Grass; 60 * 60],
        });
        world
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName("Testy".to_string()),
                GridPosition { x, y },
                // Production pops always carry Health via PopBundle; the
                // census (STATS pops= / panicking=) requires it (Item 85).
                Health::default(),
                PopAction::default(),
                Traits::default(),
            ))
            .id()
    }

    fn spawn_courageous_pop(world: &mut World, x: i32, y: i32) -> Entity {
        let pop = spawn_pop(world, x, y);
        world
            .get_mut::<Traits>(pop)
            .expect("traits")
            .add(Trait::Courage);
        pop
    }

    fn spawn_psychopath_pop(world: &mut World, x: i32, y: i32) -> Entity {
        let pop = spawn_pop(world, x, y);
        world
            .get_mut::<Traits>(pop)
            .expect("traits")
            .add(Trait::Psychopath);
        pop
    }

    fn spawn_monster(world: &mut World, x: i32, y: i32) -> Entity {
        spawn_terrifying(world, x, y, TerrifyingKind::Monster)
    }

    fn spawn_fire(world: &mut World, x: i32, y: i32) -> Entity {
        world.spawn((Fire::default(), GridPosition { x, y })).id()
    }

    fn spawn_corpse(world: &mut World, x: i32, y: i32, decay: f32) -> Entity {
        world
            .spawn((
                Corpse {
                    name: "Poor Yorick".to_string(),
                    decay,
                },
                GridPosition { x, y },
            ))
            .id()
    }

    fn spawn_hostile_fauna(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Fauna {
                    state: FaunaState::Attack,
                    ..Default::default()
                },
                GridPosition { x, y },
            ))
            .id()
    }

    fn drain_chronicle(world: &mut World) -> Vec<String> {
        let texts: Vec<String> = world
            .resource::<Events<AddChronicleEvent>>()
            .iter_current_update_events()
            .map(|e| e.text.clone())
            .collect();
        world.resource_mut::<Events<AddChronicleEvent>>().clear();
        texts
    }

    // --- trigger ----------------------------------------------------------

    #[test]
    fn test_pop_panics_near_terrifying_event() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6); // adjacent

        trigger_panic_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("pop should be panicking");
        assert_eq!(panic.timer, PANIC_DURATION_TICKS);
        assert_eq!(panic.fled_from, GridPosition { x: 5, y: 5 });
    }

    #[test]
    fn test_pop_does_not_panic_far_from_terrifying_event() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 5 + TERRIFY_RADIUS as i32 + 1);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_pop_panics_near_fire() {
        let mut world = setup();
        spawn_fire(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, 11, 10);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
    }

    #[test]
    fn test_pop_panics_near_rotting_corpse() {
        let mut world = setup();
        spawn_corpse(&mut world, 20, 20, 0.5);
        let pop = spawn_pop(&mut world, 21, 20);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
    }

    #[test]
    fn test_pop_panics_near_hostile_fauna() {
        let mut world = setup();
        spawn_hostile_fauna(&mut world, 30, 30);
        let pop = spawn_pop(&mut world, 31, 30);

        trigger_panic_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("should panic");
        assert_eq!(panic.fled_from, GridPosition { x: 30, y: 30 });
    }

    #[test]
    fn test_pop_ignores_wandering_fauna() {
        let mut world = setup();
        let _ = world
            .spawn((
                Fauna {
                    state: FaunaState::Wander,
                    ..Default::default()
                },
                GridPosition { x: 40, y: 40 },
            ))
            .id();
        let pop = spawn_pop(&mut world, 41, 40);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_psychopath_immune_to_terror() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_psychopath_pop(&mut world, 5, 6);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_none(),
            "psychopaths are unaffected by horror"
        );
    }

    #[test]
    fn test_courage_resists_distant_terror() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        // Distance 3: inside normal radius, outside courageous radius.
        let pop = spawn_courageous_pop(&mut world, 8, 5);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_none(),
            "courageous pops shrug off distant horror"
        );
    }

    #[test]
    fn test_courage_panics_up_close() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_courageous_pop(&mut world, 6, 5);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_some(),
            "even courage breaks at arm's length"
        );
    }

    #[test]
    fn test_panic_chronicle_fires() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        spawn_pop(&mut world, 5, 6);

        trigger_panic_system(&mut world);

        let log = drain_chronicle(&mut world);
        assert!(
            log.iter().any(|t| t.contains("panics at the sight of a monster")),
            "panic chronicle should fire, got: {log:?}"
        );
    }

    #[test]
    fn test_panic_chronicle_spam_capped() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        for i in 0..10 {
            spawn_pop(&mut world, 5 + i, 6);
        }

        trigger_panic_system(&mut world);

        let log = drain_chronicle(&mut world);
        assert!(
            log.len() <= PANIC_CHRONICLE_CAP_PER_RUN,
            "chronicle spam-capped, got {} entries",
            log.len()
        );
    }

    // --- spread -----------------------------------------------------------

    #[test]
    fn test_panic_spreads_on_collision() {
        let mut world = setup();
        // Panicked pop.
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        // Normal pop on the same tile.
        let normal = spawn_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(normal).is_some(),
            "sharing a tile with a panicker spreads panic"
        );
    }

    #[test]
    fn test_panic_spreads_to_adjacent() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let normal = spawn_pop(&mut world, 11, 11); // diagonal neighbor

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(normal).is_some(),
            "adjacent pops catch the panic"
        );
    }

    #[test]
    fn test_panic_does_not_spread_far() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let far = spawn_pop(&mut world, 12, 10); // two tiles away

        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(far).is_none());
    }

    #[test]
    fn test_courage_resists_single_panicker_spread() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let brave = spawn_courageous_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(brave).is_none(),
            "one panicker does not rattle the courageous"
        );
    }

    #[test]
    fn test_courage_overwhelmed_by_crowd_spread() {
        let mut world = setup();
        for (x, y) in [(10, 10), (10, 11)] {
            let p = spawn_pop(&mut world, x, y);
            world
                .entity_mut(p)
                .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        }
        let brave = spawn_courageous_pop(&mut world, 11, 10);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(brave).is_some(),
            "two adjacent panickers overwhelm courage"
        );
    }

    #[test]
    fn test_psychopath_immune_to_spread() {
        let mut world = setup();
        let panicker = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(panicker)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));
        let psycho = spawn_psychopath_pop(&mut world, 10, 10);

        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(psycho).is_none());
    }

    // --- timer ------------------------------------------------------------

    #[test]
    fn test_panic_timer_decrements() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        world
            .entity_mut(pop)
            .insert(Panic::new(GridPosition { x: 1, y: 1 }));

        panic_timer_system(&mut world);

        let panic = world.get::<Panic>(pop).expect("still panicking");
        assert_eq!(panic.timer, PANIC_DURATION_TICKS - 1);
    }

    #[test]
    fn test_panic_ends_and_chronicles() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        world.entity_mut(pop).insert(Panic {
            timer: 1,
            fled_from: GridPosition { x: 1, y: 1 },
        });

        panic_timer_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none(), "panic should end");
        let log = drain_chronicle(&mut world);
        assert!(
            log.iter().any(|t| t.contains("calms down")),
            "calm-down chronicle should fire, got: {log:?}"
        );
    }

    // --- flee -------------------------------------------------------------

    #[test]
    fn test_panicked_pop_flees_away_from_horror() {
        let mut world = setup_open_field();
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });
        let before = world.get::<GridPosition>(pop).copied().unwrap();

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert_ne!(before, after, "a panicking pop must move");
        assert!(
            after.distance_chebyshev(GridPosition { x: 30, y: 30 })
                >= before.distance_chebyshev(GridPosition { x: 30, y: 30 }),
            "fleeing must not move toward the horror"
        );
    }

    #[test]
    fn test_flee_prefers_increasing_distance() {
        let mut world = setup_open_field();
        // Horror directly south; every neighbor north is walkable and
        // farther, so the pop must end up strictly farther away.
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 40 },
        });

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        let horror = GridPosition { x: 30, y: 40 };
        assert!(
            after.distance_chebyshev(horror) > GridPosition { x: 30, y: 31 }.distance_chebyshev(horror),
            "flee should increase distance, ended at {after:?}"
        );
    }

    // --- items & work -----------------------------------------------------

    #[test]
    fn test_panicked_pop_drops_carried_item() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        let item = world
            .spawn((Item {
                item_type: ItemType::Potato,
            },))
            .id();
        world.entity_mut(pop).insert(CarryingItem(item));
        let pop_pos = world.get::<GridPosition>(pop).copied().unwrap();

        trigger_panic_system(&mut world);

        assert!(
            world.get::<CarryingItem>(pop).is_none(),
            "panicked pop drops the payload"
        );
        assert_eq!(
            world.get::<GridPosition>(item).copied(),
            Some(pop_pos),
            "dropped item lands on the pop's tile"
        );
    }

    #[test]
    fn test_panic_resets_action_to_idle() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        world.entity_mut(pop).insert(PopAction {
            current: ActionType::Work,
            current_utility: 0.9,
            ticks_committed: 40,
        });

        trigger_panic_system(&mut world);

        let action = world.get::<PopAction>(pop).expect("action");
        assert_eq!(
            action.current,
            ActionType::Idle,
            "panicked pops abandon work for flight"
        );
    }

    // --- sightings --------------------------------------------------------

    #[test]
    fn test_collect_terrifying_sightings_kinds() {
        let mut world = setup();
        spawn_monster(&mut world, 1, 1);
        spawn_fire(&mut world, 2, 2);
        spawn_corpse(&mut world, 3, 3, 0.2);
        spawn_hostile_fauna(&mut world, 4, 4);

        let sightings = collect_terrifying_sightings(&mut world);
        let kinds: HashSet<TerrifyingKind> =
            sightings.iter().map(|s| s.kind).collect();

        assert!(kinds.contains(&TerrifyingKind::Monster));
        assert!(kinds.contains(&TerrifyingKind::Fire));
        assert!(kinds.contains(&TerrifyingKind::Corpse));
        assert_eq!(sightings.len(), 4, "got {sightings:?}");
    }

    #[test]
    fn test_fully_rotted_corpse_not_terrifying() {
        let mut world = setup();
        // decay == 1.0: compost-ready, despawned by the compost system.
        spawn_corpse(&mut world, 3, 3, 1.0);
        let pop = spawn_pop(&mut world, 4, 3);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_terrifying_kind_labels() {
        assert_eq!(TerrifyingKind::Monster.label(), "a monster");
        assert_eq!(TerrifyingKind::Fire.label(), "a fire");
        assert_eq!(TerrifyingKind::Corpse.label(), "a rotting corpse");
        assert_eq!(TerrifyingKind::PanickingPop.label(), "a screaming colonist");
    }

    #[test]
    fn test_panicking_count() {
        let mut world = setup();
        let a = spawn_pop(&mut world, 1, 1);
        spawn_pop(&mut world, 2, 2);
        world
            .entity_mut(a)
            .insert(Panic::new(GridPosition { x: 0, y: 0 }));

        assert_eq!(panicking_count(&mut world), 1);
    }

    /// The STATS census population, replicating the headless `pops=`
    /// filter exactly: Pop + Health + GridPosition, no stowaways, not dead.
    fn census_pop_count(world: &mut World) -> usize {
        let mut q = world.query_filtered::<
            Entity,
            (
                With<Pop>,
                With<Health>,
                With<GridPosition>,
                Without<Stowaway>,
                Without<Dead>,
            ),
        >();
        q.iter(world).count()
    }

    #[test]
    fn test_panicking_count_never_exceeds_census_pops() {
        // Regression (Item 85): the balance loop observed STATS
        // `panicking=17` against `pops=15`. Reproduce that exact shape:
        // 15 census pops, ALL panicking, plus 2 panicking stowaways —
        // full PopBundle pops the census deliberately hides (Spec 327).
        // The old filter (With<Pop> + With<Panic> only) counted 17.
        use crate::layer1::social::ship_stowaway::StowawayAgenda;

        let mut world = setup();
        for i in 0..15 {
            let pop = spawn_pop(&mut world, i, 1);
            world
                .entity_mut(pop)
                .insert(Panic::new(GridPosition { x: 0, y: 0 }));
        }
        for i in 0..2 {
            let stowaway = spawn_pop(&mut world, 20 + i, 1);
            world.entity_mut(stowaway).insert((
                Stowaway::new(StowawayAgenda::Spy, "Test Faction", 0.0, 100),
                Panic::new(GridPosition { x: 0, y: 0 }),
            ));
        }

        assert_eq!(census_pop_count(&mut world), 15);
        assert_eq!(panicking_count(&mut world), 15);
        assert!(panicking_count(&mut world) <= census_pop_count(&mut world));
    }

    #[test]
    fn test_panicking_count_excludes_healthless_and_dead_pops() {
        // The other two ways the old count escaped the census: a
        // Pop+Panic entity with no Health (never counted by `pops=`,
        // whose query requires Health) and a Dead pop awaiting despawn.
        let mut world = setup();
        let counted = spawn_pop(&mut world, 1, 1);
        world
            .entity_mut(counted)
            .insert(Panic::new(GridPosition { x: 0, y: 0 }));

        let healthless = world
            .spawn((
                Pop,
                PopName("Ghost".to_string()),
                GridPosition { x: 2, y: 2 },
                Panic::new(GridPosition { x: 0, y: 0 }),
            ))
            .id();
        let _ = healthless;

        let dead = spawn_pop(&mut world, 3, 3);
        world.entity_mut(dead).insert((
            Dead,
            Panic::new(GridPosition { x: 0, y: 0 }),
        ));

        assert_eq!(census_pop_count(&mut world), 1);
        assert_eq!(panicking_count(&mut world), 1);
    }

    #[test]
    fn test_no_panic_without_sightings() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);

        trigger_panic_system(&mut world);
        spread_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_none());
    }

    #[test]
    fn test_flee_boxed_in_stays_put() {
        let mut world = setup();
        // Wall the pop in with rock on all 8 neighbors.
        {
            let mut grid = world.resource_mut::<TerrainGrid>();
            for (dx, dy) in [
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (0, -1),
                (0, 1),
                (1, -1),
                (1, 0),
                (1, 1),
            ] {
                let idx = (31 + dy) as usize * 60 + (30 + dx) as usize;
                grid.tiles[idx] = TerrainType::Rock;
            }
        }
        let pop = spawn_pop(&mut world, 30, 31);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });

        panic_flee_system(&mut world);

        assert_eq!(
            world.get::<GridPosition>(pop).copied(),
            Some(GridPosition { x: 30, y: 31 }),
            "a boxed-in pop holds position instead of walking through rock"
        );
    }

    #[test]
    fn test_flee_falls_back_when_no_neighbor_farther() {
        let mut world = setup();
        // Horror five tiles south. Wall every neighbor except (30,34)
        // (d=4, closer) and (29,35)/(31,35) (d=5, same) — none strictly
        // farther than the current d=5, so the pop must still move (fallback
        // pool) rather than freeze.
        {
            let mut grid = world.resource_mut::<TerrainGrid>();
            // Keep only (30,34) d=4, (29,35)/(31,35) d=5 walkable — none
            // strictly farther than the pop's current d=5.
            let keep = [(0, -1), (-1, 0), (1, 0)];
            for (dx, dy) in [
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (0, -1),
                (0, 1),
                (1, -1),
                (1, 0),
                (1, 1),
            ] {
                if !keep.contains(&(dx, dy)) {
                    let idx = (35 + dy) as usize * 60 + (30 + dx) as usize;
                    grid.tiles[idx] = TerrainType::Rock;
                }
            }
        }
        let pop = spawn_pop(&mut world, 30, 35);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 30, y: 30 },
        });
        let before = world.get::<GridPosition>(pop).copied().unwrap();

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert_ne!(before, after, "a panicking pop keeps moving");
        assert!(
            after.distance_chebyshev(GridPosition { x: 30, y: 30 }) <= 5,
            "fallback tiles are no farther than the start, got {after:?}"
        );
    }

    #[test]
    fn test_try_panic_pop_rejects_already_panicked() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 5, 6);
        world
            .entity_mut(pop)
            .insert(Panic::new(GridPosition { x: 5, y: 5 }));
        let sighting = TerrifyingSighting {
            pos: GridPosition { x: 5, y: 5 },
            kind: TerrifyingKind::Monster,
        };

        assert!(!try_panic_pop(&mut world, pop, sighting, 1, false, 0));
        assert_eq!(
            world.get::<Panic>(pop).unwrap().timer,
            PANIC_DURATION_TICKS,
            "re-panicking must not refresh the timer"
        );
    }

    // --- Item 97: vacuum panic-trap -------------------------------------
    //
    // Balance evidence (2026-10-17): 83% of fatal vacuum episodes include
    // panic; suited deaths run the full bottle + full HP because pops
    // never turn back; fatal episodes (150-295t) far outlast the 60t
    // panic timer (suspected corpse-sight retriggering).

    /// World where x < 30 is pressurized (1.0) and x >= 30 is vacuum.
    fn setup_vacuum_world() -> World {
        let mut world = setup_open_field();
        let mut grid = crate::layer1::pressure::PressureGrid::new(60, 60);
        for y in 0..60 {
            for x in 0..30 {
                grid.set(x, y, 1.0);
            }
        }
        world.insert_resource(grid);
        world
    }

    #[test]
    fn test_panic_flee_in_vacuum_heads_toward_pressure_not_away_from_horror() {
        // Horror is INSIDE the habitat (west); blindly fleeing away from
        // it pushes the pop east, deeper into vacuum — the trap.
        let mut world = setup_vacuum_world();
        let pop = spawn_pop(&mut world, 35, 30);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 28, y: 30 },
        });

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(
            after.x < 35,
            "panicked pop in vacuum must move toward pressure (west), got {after:?}"
        );
    }

    #[test]
    fn test_panic_flee_in_vacuum_reaches_pressure_within_bottle_budget() {
        let mut world = setup_vacuum_world();
        let pop = spawn_pop(&mut world, 38, 30);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 28, y: 30 },
        });

        for _ in 0..12 {
            panic_flee_system(&mut world);
        }

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(
            after.x < 30,
            "panicked pop must be back in pressurized tiles within 12 ticks, got {after:?}"
        );
    }

    #[test]
    fn test_pop_in_vacuum_does_not_panic_at_corpse() {
        // Survival first: a pop already in vacuum must not be (re)panicked
        // by a corpse — that retrigger is what chains 60t panics into
        // 150-295t fatal episodes.
        let mut world = setup_vacuum_world();
        spawn_corpse(&mut world, 36, 30, 0.5);
        let pop = spawn_pop(&mut world, 35, 30);

        trigger_panic_system(&mut world);

        assert!(
            world.get::<Panic>(pop).is_none(),
            "a pop in vacuum must not panic — survival comes first"
        );
    }

    #[test]
    fn test_panic_does_not_spread_to_pop_in_vacuum() {
        let mut world = setup_vacuum_world();
        let panicker = spawn_pop(&mut world, 34, 30);
        world.entity_mut(panicker).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 28, y: 30 },
        });
        let bystander = spawn_pop(&mut world, 35, 30);

        spread_panic_system(&mut world);

        assert!(
            world.get::<Panic>(bystander).is_none(),
            "panic must not spread to a pop in vacuum"
        );
    }

    fn spawn_suited_pop(
        world: &mut World,
        x: i32,
        y: i32,
        air: f32,
    ) -> (Entity, Entity) {
        use crate::layer1::eva_suit::{EvaSuit, SuitClass};
        use crate::layer1::items::Equipment;
        let suit = world
            .spawn((
                Item::default(),
                EvaSuit {
                    class: SuitClass::Base,
                    air,
                    durability: 100.0,
                    max_durability: 100.0,
                    colony_issued: true,
                },
                GridPosition { x, y },
            ))
            .id();
        let pop = spawn_pop(world, x, y);
        world.entity_mut(pop).insert(Equipment {
            body: Some(suit),
            ..Default::default()
        });
        (pop, suit)
    }

    #[test]
    fn test_vacuum_return_unsuited_pop_heads_to_pressure_and_arrives() {
        let mut world = setup_vacuum_world();
        let pop = spawn_pop(&mut world, 35, 30);

        vacuum_return_system(&mut world);
        let after_one = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(after_one.x < 35, "unsuited pop in vacuum must turn back, got {after_one:?}");

        for _ in 0..8 {
            vacuum_return_system(&mut world);
        }
        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(after.x < 30, "unsuited pop must reach pressurized tiles, got {after:?}");
    }

    #[test]
    fn test_vacuum_return_low_air_suit_turns_back_but_healthy_suit_stays() {
        let mut world = setup_vacuum_world();
        // Base capacity is 50; 10 air = 0.20 <= EVA_AIR_LOW_FRACTION (0.25).
        let (low, _suit) = spawn_suited_pop(&mut world, 35, 30, 10.0);
        let (healthy, _suit2) = spawn_suited_pop(&mut world, 40, 40, 50.0);

        vacuum_return_system(&mut world);

        let low_after = world.get::<GridPosition>(low).copied().unwrap();
        assert!(low_after.x < 35, "low-air suited pop must turn back, got {low_after:?}");
        let healthy_after = world.get::<GridPosition>(healthy).copied().unwrap();
        assert_eq!(
            healthy_after,
            GridPosition { x: 40, y: 40 },
            "a healthy bottle means legitimate EVA work — do not yank the pop home"
        );
    }

    #[test]
    fn test_vacuum_return_dry_suit_turns_back() {
        let mut world = setup_vacuum_world();
        let (pop, _suit) = spawn_suited_pop(&mut world, 35, 30, 0.0);

        vacuum_return_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(after.x < 35, "dry-suited pop is suffocating and must turn back, got {after:?}");
    }

    #[test]
    fn test_vacuum_return_never_drags_dormant_origin_pops() {
        // Item 100: dormant origin pops wait in stasis at their outdoor
        // sites. The return drive must not drag them toward the habitat:
        // their origin tethers pull them back to the site every tick, so
        // claiming them is a 1:1 tug-of-war fought in vacuum (and Tock
        // was already excluded here on a mistaken envelope premise).
        let mut world = setup_vacuum_world();
        let vigil = spawn_pop(&mut world, 35, 30);
        world
            .entity_mut(vigil)
            .insert(crate::layer1::culture::lawbound::DormantAutomaton);
        let quill = spawn_pop(&mut world, 40, 40);
        world
            .entity_mut(quill)
            .insert(crate::layer1::culture::bloomtouched::DormantTouched);
        let tock = spawn_pop(&mut world, 45, 30);
        world
            .entity_mut(tock)
            .insert(crate::layer1::culture::chronostalker::DormantStalker);

        for _ in 0..5 {
            vacuum_return_system(&mut world);
        }

        for (pop, x, y) in [(vigil, 35, 30), (quill, 40, 40), (tock, 45, 30)] {
            assert_eq!(
                world.get::<GridPosition>(pop).copied().unwrap(),
                GridPosition { x, y },
                "dormant origin pops stay at their sites"
            );
        }
    }

    #[test]
    fn test_vacuum_return_ignores_pressurized_pops() {
        let mut world = setup_vacuum_world();
        let pop = spawn_pop(&mut world, 20, 30);

        vacuum_return_system(&mut world);

        assert_eq!(
            world.get::<GridPosition>(pop).copied().unwrap(),
            GridPosition { x: 20, y: 30 },
            "pressurized pops are untouched by the return drive"
        );
    }

    #[test]
    fn test_panic_flee_at_habitat_edge_does_not_enter_vacuum() {
        // Pop on the last pressurized column, horror deeper inside:
        // blind distance-maximizing flight would step east into vacuum.
        let mut world = setup_vacuum_world();
        let pop = spawn_pop(&mut world, 29, 30);
        world.entity_mut(pop).insert(Panic {
            timer: PANIC_DURATION_TICKS,
            fled_from: GridPosition { x: 25, y: 30 },
        });

        panic_flee_system(&mut world);

        let after = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(
            after.x < 30,
            "flee step must not enter vacuum when pressurized steps exist, got {after:?}"
        );
    }

    #[test]
    fn test_panic_drops_missing_item_gracefully() {
        let mut world = setup();
        spawn_monster(&mut world, 5, 5);
        let pop = spawn_pop(&mut world, 5, 6);
        let item = world
            .spawn((Item {
                item_type: ItemType::Potato,
            },))
            .id();
        world.entity_mut(pop).insert(CarryingItem(item));
        // The item entity is gone (consumed elsewhere) — panic must not crash.
        world.despawn(item);

        trigger_panic_system(&mut world);

        assert!(world.get::<Panic>(pop).is_some());
        assert!(world.get::<CarryingItem>(pop).is_none());
    }

    // ------------------------------------------------------------------
    // Item 110 Gap 2: fire brigade — Douse executable under panic (RED
    // first: before the brigade existed, a panicked pop could never
    // execute a Douse designation — utility AI excludes `Panic` pops and
    // no other system moved them toward a fire).
    // ------------------------------------------------------------------

    fn spawn_douse_designation(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Designation {
                    designation_type: DesignationType::Douse,
                },
                GridPosition { x, y },
            ))
            .id()
    }

    fn fire_count(world: &mut World) -> usize {
        world.query::<&Fire>().iter(world).count()
    }

    #[test]
    fn test_panicked_pop_douses_designated_fire_from_adjacency() {
        let mut world = setup_open_field();
        world.insert_resource(crate::shared::log::MessageLog::default());
        let fire_pos = GridPosition { x: 10, y: 10 };
        spawn_fire(&mut world, fire_pos.x, fire_pos.y);
        let designation = spawn_douse_designation(&mut world, fire_pos.x, fire_pos.y);
        // The only nearby pop is panicking — the l2r2k_11 shape.
        let pop = spawn_pop(&mut world, 10, 11);
        world.entity_mut(pop).insert(Panic::new(fire_pos));

        fire_brigade_system(&mut world);

        assert_eq!(fire_count(&mut world), 0, "brigadier must douse the fire");
        assert!(
            world.get_entity(designation).is_err(),
            "douse completes the designation"
        );
        assert!(
            world.get::<Panic>(pop).is_some(),
            "brigade duty does not cure panic — the pop stays panicked"
        );
    }

    #[test]
    fn test_panicked_pop_walks_to_designated_fire_through_panic_chain() {
        // The live-schedule shape: flee runs in the same chain as the
        // brigade (brigade last, as registered in simulation.rs). A
        // panicked pop four tiles out must still reach and douse the
        // fire instead of fleeing in circles until it burns down.
        let mut world = setup_open_field();
        world.insert_resource(crate::shared::log::MessageLog::default());
        let fire_pos = GridPosition { x: 10, y: 10 };
        spawn_fire(&mut world, fire_pos.x, fire_pos.y);
        spawn_douse_designation(&mut world, fire_pos.x, fire_pos.y);
        let pop = spawn_pop(&mut world, 10, 14);
        world.entity_mut(pop).insert(Panic::new(fire_pos));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                panic_flee_system,
                vacuum_return_system,
                fire_brigade_system,
            )
                .chain(),
        );
        for _ in 0..15 {
            schedule.run(&mut world);
            if fire_count(&mut world) == 0 {
                break;
            }
        }

        assert_eq!(
            fire_count(&mut world),
            0,
            "panicked brigadier must reach and douse the designated fire"
        );
        let pos = world.get::<GridPosition>(pop).copied().unwrap();
        assert!(
            pos.distance_chebyshev(fire_pos) <= 1,
            "brigadier ends adjacent to the fire tile, got {pos:?}"
        );
    }

    #[test]
    fn test_undesignated_fire_drafts_no_brigade() {
        // No keeper order, no brigade: panic behavior is unchanged and
        // the fire is left for the normal (non-panicked) work path.
        let mut world = setup_open_field();
        let fire_pos = GridPosition { x: 10, y: 10 };
        spawn_fire(&mut world, fire_pos.x, fire_pos.y);
        let pop = spawn_pop(&mut world, 10, 11);
        world.entity_mut(pop).insert(Panic::new(fire_pos));

        fire_brigade_system(&mut world);

        assert_eq!(fire_count(&mut world), 1, "undesignated fire must remain");
        assert!(
            world.get::<FireBrigade>(pop).is_none(),
            "no draft without a Douse designation"
        );
    }

    #[test]
    fn test_calm_pop_also_answers_brigade() {
        // The brigade is not panic-only: when the nearest pop happens to
        // be calm, the designation is still guaranteed executable.
        let mut world = setup_open_field();
        world.insert_resource(crate::shared::log::MessageLog::default());
        let fire_pos = GridPosition { x: 10, y: 10 };
        spawn_fire(&mut world, fire_pos.x, fire_pos.y);
        spawn_douse_designation(&mut world, fire_pos.x, fire_pos.y);
        spawn_pop(&mut world, 11, 10);

        fire_brigade_system(&mut world);

        assert_eq!(fire_count(&mut world), 0, "calm brigadier douses too");
    }
}
