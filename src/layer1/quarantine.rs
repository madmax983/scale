//! Spec 406: Quarantine Protocols — player-designated sealed zones.
//!
//! # Grounding audit (spec §8 adaptation pattern, specs 360–405)
//!
//! The spec sketch types were audited against the real codebase first:
//!
//! * `layer1::zone::{ZoneGrid, ZoneType}` — **REAL** (`administration/zone.rs`,
//!   re-exported as `layer1::zone`). `ZoneType` already carries `Penal`,
//!   `Jail`, `Sanctuary`, etc. — but had **no `Quarantine` variant**. This
//!   spec adds exactly that one variant (plus its persistence parse arm).
//! * `layer1::map::PathfindingGrid` — **MOCK, does not exist.** The real
//!   pathfinding seam is the A* in [`crate::layer1::pathfinding`]
//!   (`find_path` / `find_path_for_pop`, neighbor expansion gated by a
//!   private `is_walkable`), plus the greedy step seam in
//!   `execution::movement`. No parallel grid was built: quarantine
//!   severance is an **edge check** ([`is_quarantine_edge_blocked`])
//!   consulted by both real seams, exactly as spec §5/§7 ask — tiles
//!   inside a zone stay walkable to pops inside it; only crossings of
//!   the boundary are severed.
//! * `building::Door { is_locked }` — **MOCK, does not exist.** The real
//!   door/lock seams (spec 119 airlock-pressure lane) are
//!   [`AccessControl`] with `AccessMode::Lockdown`,
//!   [`DoorControl`] with `DoorState::Locked`, and the legacy
//!   [`Gate`] `is_locked` flag. Pathfinding and movement already honor
//!   all three. Quarantine lockdown drives all three real seams and
//!   remembers the prior state in [`QuarantineSealed`] so lifting a
//!   zone restores a door exactly as it was.
//! * `Hostile` tag for the defense grid — **no such pop component
//!   exists.** Real turrets (`architecture::turret`) target only
//!   `Fauna`; there is no pop-targeting defense grid to hook. The real
//!   law-enforcement seam is [`Wanted`] (`law::justice`): wardens
//!   arrest `Wanted` pops, and `check_crime_system` already tags
//!   Vandalize-break pops `Wanted`. Quarantine breach therefore tags
//!   the breaker `Wanted { severity: 1.0 }` + [`QuarantineBreacher`]
//!   — the strongest real "hostile to the colony's defenses" signal
//!   the codebase carries. Documented honestly: turrets will not fire
//!   on the breaker; wardens will arrest them.
//! * Morale stress spike — **REAL** via [`Morale`] / [`MoodModifier`]
//!   (`social::morale`), the pattern every recent spec uses.
//!
//! # New constants (no existing balance constants were retuned)
//!
//! * [`QUARANTINE_STRESS_VALUE`] `-0.30`, [`QUARANTINE_STRESS_DURATION`]
//!   `600` ticks, label [`QUARANTINE_STRESS_LABEL`].

use crate::layer1::access_control::{AccessControl, AccessMode};
use crate::layer1::control::{DoorControl, DoorState};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::defense::Gate;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::law::justice::Wanted;
use crate::layer1::map::GridPosition;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::utility_types::ActionType;
use crate::layer1::zone::{ZoneGrid, ZoneType};
use bevy_ecs::prelude::*;

/// Morale hit applied colony-wide when a quarantine is designated.
pub const QUARANTINE_STRESS_VALUE: f32 = -0.30;
/// Duration (ticks) of the quarantine stress modifier.
pub const QUARANTINE_STRESS_DURATION: u32 = 600;
/// Player-facing label of the quarantine stress modifier.
pub const QUARANTINE_STRESS_LABEL: &str = "Quarantine Dread";
/// Chronicle text written when a quarantine zone is first sealed.
pub const QUARANTINE_SEALED_TEXT: &str =
    "A quarantine zone has been sealed. Those inside are on their own.";
/// Chronicle text written when a pop breaks quarantine.
pub const QUARANTINE_BREACH_TEXT: &str =
    "Quarantine broken: a sealed door was forced. The breaker is wanted.";

/// Marker + prior-state record on a building locked down by quarantine.
///
/// Stores the lock state the building had *before* the seal so lifting
/// the zone restores it exactly (a door that was already `Restricted`
/// or `Locked` is not silently opened by an unrelated lift).
#[derive(Component, Debug, Clone, Copy)]
pub struct QuarantineSealed {
    /// Prior `AccessControl` mode, if the building had one.
    pub prior_access: Option<AccessMode>,
    /// Prior `DoorControl` state, if the building had one.
    pub prior_door: Option<DoorState>,
    /// Prior legacy `Gate` lock flag, if the building had one.
    pub prior_gate_locked: Option<bool>,
}

/// Marker on a pop that forced a sealed quarantine door.
///
/// The pop also carries [`Wanted`], which is what the real warden /
/// arrest seam acts on.
#[derive(Component, Debug, Default)]
pub struct QuarantineBreacher;

/// Bookkeeping for quarantine designations and breaches.
///
/// Initialized in `simulation.rs` (first-tick pattern).
#[derive(Resource, Debug, Default)]
pub struct QuarantineState {
    /// Quarantine tile count seen on the previous designation check.
    /// Growth past this value is a new designation and fires the
    /// stress spike exactly once per growth.
    pub prev_quarantine_tiles: usize,
    /// Total quarantine designations (growth events) this run.
    pub designations: u32,
    /// Total quarantine breaches this run.
    pub breaches: u32,
}

/// Is this tile inside a quarantine zone?
#[must_use]
pub fn in_quarantine(zone_grid: &ZoneGrid, x: i32, y: i32) -> bool {
    zone_grid.get(x, y) == ZoneType::Quarantine
}

/// Spec §5/§7 edge severance: an edge is blocked iff exactly one of
/// its two tiles is quarantined. Inside→inside and outside→outside
/// edges stay open, so pops sealed inside can still path to a bed,
/// a meal, or a door *within* the zone — they just cannot leave,
/// and nobody outside can path in.
#[must_use]
pub fn is_quarantine_edge_blocked(
    zone_grid: &ZoneGrid,
    from: (i32, i32),
    to: (i32, i32),
) -> bool {
    in_quarantine(zone_grid, from.0, from.1) != in_quarantine(zone_grid, to.0, to.1)
}

/// Inverse of [`is_quarantine_edge_blocked`], named for the spec's
/// §7 `is_edge_walkable` guidance.
#[must_use]
pub fn is_edge_walkable(zone_grid: &ZoneGrid, from: (i32, i32), to: (i32, i32)) -> bool {
    !is_quarantine_edge_blocked(zone_grid, from, to)
}

/// Count of tiles currently designated quarantine.
#[must_use]
pub fn count_quarantine_tiles(zone_grid: &ZoneGrid) -> usize {
    zone_grid
        .grid
        .iter()
        .filter(|&&z| z == ZoneType::Quarantine)
        .count()
}

/// Designate a `w`×`h` rectangle (anchored at `x`,`y`) as quarantine.
/// Returns how many tiles were newly designated.
pub fn designate_quarantine(
    zone_grid: &mut ZoneGrid,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> usize {
    let mut newly = 0;
    for dy in 0..h.max(0) {
        for dx in 0..w.max(0) {
            if zone_grid.get(x + dx, y + dy) != ZoneType::Quarantine {
                zone_grid.set(x + dx, y + dy, ZoneType::Quarantine);
                newly += 1;
            }
        }
    }
    newly
}

/// Lift quarantine over a `w`×`h` rectangle. Returns tiles cleared.
/// Doors in the rectangle are restored by
/// [`enforce_quarantine_locks_system`] on its next run.
pub fn lift_quarantine(zone_grid: &mut ZoneGrid, x: i32, y: i32, w: i32, h: i32) -> usize {
    let mut cleared = 0;
    for dy in 0..h.max(0) {
        for dx in 0..w.max(0) {
            if zone_grid.get(x + dx, y + dy) == ZoneType::Quarantine {
                zone_grid.set(x + dx, y + dy, ZoneType::None);
                cleared += 1;
            }
        }
    }
    cleared
}

/// Lock down every door/bulkhead/gate inside a quarantine zone, and
/// restore any quarantine-sealed building whose tile is no longer
/// quarantined. Drives the three real lock seams (spec 119 lane):
/// `AccessControl → Lockdown`, `DoorControl → Locked`, `Gate →
/// is_locked`. Adapted from the spec's mock `enforce_quarantine_locks_system`.
#[allow(clippy::type_complexity)]
pub fn enforce_quarantine_locks_system(
    zone_grid: Option<Res<ZoneGrid>>,
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &GridPosition,
        Option<&mut AccessControl>,
        Option<&mut DoorControl>,
        Option<&mut Gate>,
        Option<&QuarantineSealed>,
    )>,
) {
    let Some(zone_grid) = zone_grid else {
        return;
    };
    for (entity, pos, access, door, gate, sealed) in &mut query {
        if in_quarantine(&zone_grid, pos.x, pos.y) {
            if sealed.is_none() {
                let record = QuarantineSealed {
                    prior_access: access.as_deref().map(|a| a.mode),
                    prior_door: door.as_deref().map(|d| d.state),
                    prior_gate_locked: gate.as_deref().map(|g| g.is_locked),
                };
                commands.entity(entity).insert(record);
            }
            if let Some(mut a) = access {
                a.mode = AccessMode::Lockdown;
            }
            if let Some(mut d) = door {
                d.state = DoorState::Locked;
            }
            if let Some(mut g) = gate {
                g.is_locked = true;
            }
        } else if let Some(record) = sealed {
            if let (Some(mut a), Some(mode)) = (access, record.prior_access) {
                a.mode = mode;
            }
            if let (Some(mut d), Some(state)) = (door, record.prior_door) {
                d.state = state;
            }
            if let (Some(mut g), Some(locked)) = (gate, record.prior_gate_locked) {
                g.is_locked = locked;
            }
            commands.entity(entity).remove::<QuarantineSealed>();
        }
    }
}

/// Colony-wide stress spike on designation (spec §5 Morale Impact).
///
/// Edge-triggered on tile-count growth: the first designation — and
/// any later expansion — applies one [`QUARANTINE_STRESS_LABEL`]
/// modifier to every pop with a [`Morale`] component and writes one
/// Major chronicle entry. Shrinking or steady state never re-fires.
pub fn quarantine_designation_system(
    zone_grid: Option<Res<ZoneGrid>>,
    state: Option<ResMut<QuarantineState>>,
    mut morale_query: Query<&mut Morale>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let (Some(zone_grid), Some(mut state)) = (zone_grid, state) else {
        return;
    };
    let current = count_quarantine_tiles(&zone_grid);
    if current > state.prev_quarantine_tiles {
        state.designations += 1;
        for mut morale in &mut morale_query {
            morale.add_modifier(MoodModifier {
                label: QUARANTINE_STRESS_LABEL.to_string(),
                value: QUARANTINE_STRESS_VALUE,
                duration: QUARANTINE_STRESS_DURATION,
            });
        }
        if let Some(events) = chronicle.as_deref_mut() {
            events.send(AddChronicleEvent {
                text: QUARANTINE_SEALED_TEXT.to_string(),
                importance: EventImportance::Major,
            });
        }
    }
    state.prev_quarantine_tiles = current;
}

/// Tag a pop as a quarantine breaker: [`Wanted`] (the real arrest
/// seam) + [`QuarantineBreacher`], ledger + chronicle updated.
/// Returns `false` if the pop was already tagged (no double-count).
pub fn tag_quarantine_breacher(world: &mut World, pop: Entity) -> bool {
    if world.get::<QuarantineBreacher>(pop).is_some() {
        return false;
    }
    world
        .entity_mut(pop)
        .insert((Wanted { severity: 1.0 }, QuarantineBreacher));
    if let Some(mut state) = world.get_resource_mut::<QuarantineState>() {
        state.breaches += 1;
    }
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent {
            text: QUARANTINE_BREACH_TEXT.to_string(),
            importance: EventImportance::Major,
        });
    }
    true
}

/// Breach detection (spec §5 Lethal Force, adapted): a pop that has
/// arrived at a quarantine-sealed building with a `Vandalize` action
/// — the real vandalism seam (`execution::vandalism` runs the same
/// `AtTarget` + `MovementTarget` pattern) — is tagged via
/// [`tag_quarantine_breacher`]. Vandalizing anything else, anywhere
/// else, is ordinary crime and left to `check_crime_system`.
#[allow(clippy::type_complexity)]
pub fn quarantine_breach_system(
    zone_grid: Option<Res<ZoneGrid>>,
    mut commands: Commands,
    vandals: Query<
        (Entity, &MovementTarget),
        (With<AtTarget>, Without<QuarantineBreacher>),
    >,
    targets: Query<(&GridPosition, Option<&QuarantineSealed>)>,
    state: Option<ResMut<QuarantineState>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let Some(zone_grid) = zone_grid else {
        return;
    };
    // Count first, mutate bookkeeping once after the loop.
    let mut new_breaches = 0_u32;
    for (pop, mt) in &vandals {
        if mt.for_action != ActionType::Vandalize {
            continue;
        }
        let Ok((pos, sealed)) = targets.get(mt.target_entity) else {
            continue;
        };
        if sealed.is_none() || !in_quarantine(&zone_grid, pos.x, pos.y) {
            continue;
        }
        commands
            .entity(pop)
            .insert((Wanted { severity: 1.0 }, QuarantineBreacher));
        new_breaches += 1;
    }
    if new_breaches > 0 {
        if let Some(mut state) = state {
            state.breaches += new_breaches;
        }
        if let Some(events) = chronicle.as_deref_mut() {
            for _ in 0..new_breaches {
                events.send(AddChronicleEvent {
                    text: QUARANTINE_BREACH_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// Census of the live quarantine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuarantineCensus {
    /// Tiles currently designated quarantine.
    pub tiles: usize,
    /// Buildings currently sealed by quarantine.
    pub locked: usize,
    /// Designations so far this run.
    pub designations: u32,
    /// Breaches so far this run.
    pub breaches: u32,
}

/// Read the live census from a world.
pub fn quarantine_census(world: &mut World) -> QuarantineCensus {
    let tiles = world
        .get_resource::<ZoneGrid>()
        .map_or(0, count_quarantine_tiles);
    let locked = {
        let mut q = world.query::<&QuarantineSealed>();
        q.iter(world).count()
    };
    let (designations, breaches) = world
        .get_resource::<QuarantineState>()
        .map_or((0, 0), |s| (s.designations, s.breaches));
    QuarantineCensus {
        tiles,
        locked,
        designations,
        breaches,
    }
}

/// Compact STATS label, following the `cyber_stats_label` pattern.
pub fn quarantine_stats_label(world: &mut World) -> String {
    let c = quarantine_census(world);
    format!("{}t/{}locked/{}breach", c.tiles, c.locked, c.breaches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingMap, BuildingType, OccupiedTiles};
    use crate::layer1::pathfinding::find_path;
    use crate::layer1::pop::Pop;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use bevy_ecs::system::RunSystemOnce;

    fn path_world() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());
        world
    }

    /// Spec RED test 1, adapted: the mock asserted a `PathfindingGrid`
    /// tile toggle; the real assertion is on A* — a full-height
    /// quarantine wall severs crossing paths, while paths wholly
    /// inside or wholly outside the zone still resolve (spec §5:
    /// internal tiles stay walkable; pops inside are NOT trapped
    /// in place).
    #[test]
    fn test_quarantine_designation_blocks_pathfinding() {
        let mut world = path_world();
        let mut zones = ZoneGrid::new(10, 10);
        // Wall at x=4..=6, full height.
        designate_quarantine(&mut zones, 4, 0, 3, 10);
        world.insert_resource(zones);

        // Crossing the boundary is severed, both directions.
        assert!(
            find_path(&world, (0, 5), (9, 5)).is_none(),
            "path into the quarantine zone must be blocked"
        );
        assert!(
            find_path(&world, (5, 5), (0, 5)).is_none(),
            "path out of the quarantine zone must be blocked"
        );
        // Internal pathing still works (the §5 edge-case fix).
        assert!(
            find_path(&world, (4, 5), (6, 5)).is_some(),
            "pops inside must still path within the zone"
        );
        // Outside pathing is untouched.
        assert!(find_path(&world, (0, 0), (3, 0)).is_some());
    }

    /// Edge predicate truth table, incl. both crossing directions.
    #[test]
    fn test_quarantine_edge_walkable() {
        let mut zones = ZoneGrid::new(10, 10);
        designate_quarantine(&mut zones, 5, 5, 1, 1);
        assert!(!is_edge_walkable(&zones, (4, 5), (5, 5)));
        assert!(!is_edge_walkable(&zones, (5, 5), (4, 5)));
        // inside->inside: use a 2-wide zone:
        let mut zones2 = ZoneGrid::new(10, 10);
        designate_quarantine(&mut zones2, 5, 5, 2, 1);
        assert!(is_edge_walkable(&zones2, (5, 5), (6, 5)));
        assert!(is_edge_walkable(&zones2, (0, 0), (1, 0)));
    }

    /// Spec RED test 2, adapted to the real lock seams: the mock
    /// `Door { is_locked }` becomes `AccessControl` + `DoorControl`
    /// (+ legacy `Gate`) on a real building entity.
    #[test]
    fn test_door_locks_on_quarantine() {
        let mut world = World::new();
        let mut zones = ZoneGrid::new(10, 10);
        zones.set(5, 5, ZoneType::Quarantine);
        world.insert_resource(zones);

        let door = world
            .spawn((
                Building {
                    building_type: BuildingType::Gate,
                },
                GridPosition { x: 5, y: 5 },
                AccessControl::default(),
                DoorControl::default(),
                Gate::default(),
            ))
            .id();
        let outside = world
            .spawn((
                Building {
                    building_type: BuildingType::Gate,
                },
                GridPosition { x: 1, y: 1 },
                AccessControl::default(),
                DoorControl::default(),
            ))
            .id();

        world.run_system_once(enforce_quarantine_locks_system).unwrap();

        let access = world.get::<AccessControl>(door).unwrap();
        assert_eq!(access.mode, AccessMode::Lockdown, "door in quarantine must lock down");
        assert_eq!(
            world.get::<DoorControl>(door).unwrap().state,
            DoorState::Locked
        );
        assert!(world.get::<Gate>(door).unwrap().is_locked);
        assert!(world.get::<QuarantineSealed>(door).is_some());
        // Outside door untouched.
        assert_eq!(
            world.get::<AccessControl>(outside).unwrap().mode,
            AccessMode::Public
        );
        assert!(world.get::<QuarantineSealed>(outside).is_none());
    }

    /// Lifting the zone restores the door's exact prior state.
    #[test]
    fn test_door_unlocks_on_lift_restoring_prior_state() {
        let mut world = World::new();
        let mut zones = ZoneGrid::new(10, 10);
        zones.set(5, 5, ZoneType::Quarantine);
        world.insert_resource(zones);

        let prior = AccessControl {
            mode: AccessMode::Restricted,
            ..Default::default()
        };
        let door = world
            .spawn((
                Building {
                    building_type: BuildingType::Gate,
                },
                GridPosition { x: 5, y: 5 },
                prior,
                DoorControl {
                    state: DoorState::Open,
                },
            ))
            .id();

        world.run_system_once(enforce_quarantine_locks_system).unwrap();
        assert_eq!(
            world.get::<AccessControl>(door).unwrap().mode,
            AccessMode::Lockdown
        );

        {
            let mut zg = world.resource_mut::<ZoneGrid>();
            lift_quarantine(&mut zg, 5, 5, 1, 1);
        }
        world.run_system_once(enforce_quarantine_locks_system).unwrap();

        assert_eq!(
            world.get::<AccessControl>(door).unwrap().mode,
            AccessMode::Restricted,
            "lift must restore the pre-quarantine mode, not force Public"
        );
        assert_eq!(
            world.get::<DoorControl>(door).unwrap().state,
            DoorState::Open
        );
        assert!(world.get::<QuarantineSealed>(door).is_none());
    }

    /// Designation fires the stress spike once per growth, never on
    /// steady state or shrinkage.
    #[test]
    fn test_designation_applies_stress_spike_once() {
        let mut world = World::new();
        world.insert_resource(ZoneGrid::new(10, 10));
        world.init_resource::<QuarantineState>();
        world.init_resource::<Events<AddChronicleEvent>>();
        let pop = world.spawn((Pop, Morale::default())).id();

        world.run_system_once(quarantine_designation_system).unwrap();
        assert!(world.get::<Morale>(pop).unwrap().modifiers.is_empty());

        {
            let mut zg = world.resource_mut::<ZoneGrid>();
            designate_quarantine(&mut zg, 2, 2, 2, 2);
        }
        world.run_system_once(quarantine_designation_system).unwrap();
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(morale.modifiers.len(), 1);
        assert_eq!(morale.modifiers[0].label, QUARANTINE_STRESS_LABEL);
        assert!((morale.modifiers[0].value - QUARANTINE_STRESS_VALUE).abs() < f32::EPSILON);

        // Steady state: no second spike.
        world.run_system_once(quarantine_designation_system).unwrap();
        assert_eq!(world.get::<Morale>(pop).unwrap().modifiers.len(), 1);
        assert_eq!(world.resource::<QuarantineState>().designations, 1);
    }

    /// The world-fn breach tag: Wanted + Breacher, counted once.
    #[test]
    fn test_tag_quarantine_breacher() {
        let mut world = World::new();
        world.init_resource::<QuarantineState>();
        world.init_resource::<Events<AddChronicleEvent>>();
        let pop = world.spawn(Pop).id();

        assert!(tag_quarantine_breacher(&mut world, pop));
        assert!(world.get::<Wanted>(pop).is_some());
        assert!(world.get::<QuarantineBreacher>(pop).is_some());
        assert_eq!(world.resource::<QuarantineState>().breaches, 1);

        assert!(!tag_quarantine_breacher(&mut world, pop));
        assert_eq!(world.resource::<QuarantineState>().breaches, 1);
    }

    /// The system seam: only a Vandalize action against a *sealed*
    /// building inside the zone tags the breaker.
    #[test]
    fn test_breach_system_tags_only_sealed_door_vandals() {
        let mut world = World::new();
        let mut zones = ZoneGrid::new(10, 10);
        zones.set(5, 5, ZoneType::Quarantine);
        world.insert_resource(zones);
        world.init_resource::<QuarantineState>();
        world.init_resource::<Events<AddChronicleEvent>>();

        let door = world
            .spawn((
                Building {
                    building_type: BuildingType::Gate,
                },
                GridPosition { x: 5, y: 5 },
                QuarantineSealed {
                    prior_access: None,
                    prior_door: None,
                    prior_gate_locked: None,
                },
            ))
            .id();
        let breaker = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                MovementTarget {
                    target_entity: door,
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Vandalize,
                },
                AtTarget,
            ))
            .id();
        // A bystander working at the same door is not a breaker.
        let worker = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                MovementTarget {
                    target_entity: door,
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        world.run_system_once(quarantine_breach_system).unwrap();

        assert!(world.get::<Wanted>(breaker).is_some());
        assert!(world.get::<QuarantineBreacher>(breaker).is_some());
        assert!(world.get::<Wanted>(worker).is_none());
        assert_eq!(world.resource::<QuarantineState>().breaches, 1);
    }

    /// Census + STATS label reflect tiles/locks/breaches.
    #[test]
    fn test_quarantine_census_and_label() {
        let mut world = World::new();
        let mut zones = ZoneGrid::new(10, 10);
        designate_quarantine(&mut zones, 0, 0, 2, 2);
        world.insert_resource(zones);
        world.init_resource::<QuarantineState>();
        world.spawn(QuarantineSealed {
            prior_access: None,
            prior_door: None,
            prior_gate_locked: None,
        });
        let c = quarantine_census(&mut world);
        assert_eq!(c.tiles, 4);
        assert_eq!(c.locked, 1);
        assert_eq!(quarantine_stats_label(&mut world), "4t/1locked/0breach");
    }

    /// IP guard: every new player-facing string is original/generic.
    #[test]
    fn test_ip_guard_quarantine_strings() {
        let banned = [
            "covid", "coronavirus", "pandemic", "wuhan", "umbrella",
            "raccoon city", "t-virus", "plague inc", "contagion",
            "outbreak", "world war z", "28 days",
        ];
        let strings = [
            QUARANTINE_STRESS_LABEL,
            QUARANTINE_SEALED_TEXT,
            QUARANTINE_BREACH_TEXT,
        ];
        for s in strings {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(!lower.contains(term), "string {s:?} contains banned term {term:?}");
            }
        }
    }
}
