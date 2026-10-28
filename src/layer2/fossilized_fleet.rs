//! Spec 370: The Fossilized Fleet.
//!
//! An asteroid belt turns out to be a massive, ancient battlefield:
//! fossilized fleet nodes can be strip-mined for pristine ancient
//! technology — but disturbing the wreckage periodically reactivates
//! dormant Ghost Drones, which drop to Layer 1 and attack the colony.
//! High-tier resource acquisition with unpredictable automated
//! reprisals.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No `MiningOperation`, no `layer2::system` event channel, no
//!   `layer1::combat::EnemyDropEvent` / `EnemyType` exist.** The real
//!   Layer 2 mining seam is [`MiningTarget`][crate::layer2::mining::
//!   MiningTarget] + [`FleetMining`][crate::layer2::mining::
//!   FleetMining] + `FleetCargo` in `layer2::mining` (a fleet in
//!   orbit mines a target into its cargo hold). The spec's
//!   `MiningOperation { resource, ticks }` is therefore adapted to
//!   that real pair: a fossil node is a `MiningTarget` entity
//!   carrying the new [`FossilizedFleetNode`] marker, and an
//!   "operation" is a fleet holding `FleetMining` against it. No
//!   parallel mining system is built.
//! * **`ResourceType::AncientTech` did not exist.** Following the
//!   Spec 270 `Organs` / Spec 368 `DarkMote` precedent exactly, this
//!   spec adds the real variant plus `ancient_tech` /
//!   `max_ancient_tech` ledger fields on
//!   [`ColonyResources`][crate::layer1::resources::ColonyResources],
//!   wired through `consume`, `get_amount`, `get_max_amount`,
//!   `has_room_for` and `add_resource` (and the exhaustive matches in
//!   the GPU buffer extraction, the map renderer, and the ephemeral
//!   market — where ancient tech is untradeable, like `VoidAle`).
//!   Like `Organs`/`DarkMote`, it is deliberately absent from
//!   `can_afford`/`deduct`: building costs cannot be priced in it.
//!   The sketch's `ColonyResources::get_amount(..) -> u32` is the
//!   real `get_amount(ResourceType) -> f32`.
//! * **`MiningCompleteEvent` did not exist.** The real mining loop
//!   delivers into `FleetCargo`, never into the colony ledger, and
//!   has no completion event. This module defines the completion
//!   seam the spec's RED test 3 names — [`MiningCompleteEvent`] —
//!   and [`handle_mining_complete_system`] grants its payload
//!   through the real ledger (`add_resource`). The amount is `f32`
//!   (the ledger's unit); the sketch's integer literal coerces.
//! * **The Layer 1 drop seam is `Fauna`, not an invented enemy
//!   type.** There is no `EnemyType` enum; every Layer 1 hostile
//!   that chases and damages pops does so through
//!   [`Fauna`][crate::layer1::fauna::Fauna] + `fauna_behavior_system` +
//!   [`Health`][crate::layer1::health::Health]. The spec's
//!   `EnemyDropEvent { enemy_type: EnemyType::GhostDrone }` is
//!   adapted to [`EnemyDropEvent`] { kind: [`EnemyDropKind::
//!   GhostDrone`] }, and the drop handler spawns each drone as a
//!   real `(GhostDrone, Fauna, Health, GridPosition)` hostile, so
//!   the existing fauna AI hunts pops with it — no parallel combat
//!   system. (`Fauna::fauna_type` keeps its default: it is the
//!   generic hostile body-plan label; the drone identity lives on
//!   the [`GhostDrone`] marker.) The module lives in `layer2`
//!   alongside `mining`, rather than the sketch's nonexistent
//!   `layer2/mining/` directory or `layer1/combat` — the
//!   cross-layer precedent is `layer2::void_between` / the
//!   Star-Eater's event/state pair.
//! * **§5 RNG: a deterministic threshold, not `rand::random`.**
//!   Each actively-mined fossil node accumulates one disturbance
//!   point per tick; at [`WAKE_THRESHOLD`] the node wakes exactly
//!   one drone and the counter resets. The same mining replays
//!   identically, tests are exact, and players can learn the
//!   rhythm ("twelve ticks of digging, then something wakes").
//!   Drop positions use a small seeded LCG over the standard
//!   80x50 surface grid (the Star-Eater scatter precedent).
//! * **Spec 1113's `FossilizedShip`** (`layer1::architecture::
//!   fossilized_fleet`) is a Layer 1 combat-defense building seam
//!   (decay + defense bonus) — a different thing from a Layer 2
//!   mining node. It is left entirely alone; this module's node
//!   component is `FossilizedFleetNode`, not a second ship type.
//! * **§5 visuals (drop pods / meteor effects) are deferred** —
//!   the drop writes a Major chronicle entry instead; no particle
//!   seam is wired for Layer 2 events elsewhere either.
//! * **Spec §6's `cargo clippy -- -D warnings` / 85% coverage**
//!   are aspirational template text; the verified bar is the
//!   established one: `cargo check --all-targets` green, clippy
//!   clean on touched files, and this module's tests green.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::fauna::{Fauna, FaunaState};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::layer2::mining::{FleetMining, MiningTarget};

/// Disturbance points an actively-mined fossil node accumulates
/// before it wakes one Ghost Drone (§5 deterministic threshold:
/// one point per tick of active strip-mining, per node).
pub const WAKE_THRESHOLD: u32 = 12;
/// Health of a dropped Ghost Drone — tougher than a pop's default
/// prey, weaker than a pop: a nuisance a drafted pop can kill.
pub const DRONE_HEALTH: f32 = 40.0;
/// How far a dropped drone can see pops (the fauna detection
/// range): wide, because it falls with its sensors already hot.
pub const DRONE_DETECTION_RANGE: f32 = 14.0;
/// Standard surface grid the drop scatter targets (the same
/// 80x50 fallback the Star-Eater scatter uses when no grid
/// resource is at hand).
pub const DROP_GRID_WIDTH: i32 = 80;
/// See [`DROP_GRID_WIDTH`].
pub const DROP_GRID_HEIGHT: i32 = 50;

/// Marker + disturbance ledger for a fossilized fleet node: a
/// [`MiningTarget`] entity in the ancient battlefield belt.
/// Strip-mining it (a fleet holding
/// [`FleetMining`][crate::layer2::mining::FleetMining] against it)
/// accumulates `disturbance`; at [`WAKE_THRESHOLD`] one Ghost
/// Drone wakes and the counter resets.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FossilizedFleetNode {
    /// Ticks of active mining since the last wake (0..WAKE_THRESHOLD).
    pub disturbance: u32,
}

/// Marker for a Ghost Drone that has dropped to Layer 1. Always
/// paired with a real [`Fauna`] hostile body + [`Health`] +
/// [`GridPosition`], so the existing fauna AI hunts pops with it.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct GhostDrone;

/// A Ghost Drone woke in the wreckage (Layer 2 event). Handled
/// immediately by [`handle_ghost_drone_drops`], which converts
/// each drone into a Layer 1 [`EnemyDropEvent`].
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GhostDroneSpawnEvent {
    /// How many drones woke.
    pub count: u32,
}

/// The kind of enemy an [`EnemyDropEvent`] delivers to Layer 1
/// (adaptation: no `EnemyType` enum exists in the codebase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyDropKind {
    /// A reactivated Ghost Drone from the fossilized fleet.
    GhostDrone,
}

/// An enemy drops onto Layer 1 (the cross-layer reprisal seam).
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnemyDropEvent {
    /// What is dropping.
    pub kind: EnemyDropKind,
}

/// A mining operation completed and its yield is certified into
/// the colony ledger (adaptation: the real mining loop delivers
/// into `FleetCargo` and has no completion event — see module
/// docs). [`handle_mining_complete_system`] grants the payload
/// through `ColonyResources::add_resource`.
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub struct MiningCompleteEvent {
    /// What was recovered.
    pub resource: ResourceType,
    /// How much (ledger units).
    pub amount: f32,
}

/// Colony ledger for the fossilized fleet (STATS / panel surface).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct FossilizedFleetState {
    /// Total ticks of active fossil-node mining observed.
    pub disturbance_ticks: u64,
    /// Total Ghost Drones woken in the wreckage.
    pub drones_woken: u64,
    /// Total Ghost Drones dropped onto Layer 1.
    pub drones_dropped: u64,
    /// Total AncientTech certified into the ledger by completions.
    pub tech_granted: f32,
}

/// Advance one node's disturbance by `ticks`; returns how many
/// drones wake (one per full [`WAKE_THRESHOLD`] crossed). Pure
/// and deterministic — the §5 threshold logic, shared by the
/// mining system and directly unit-testable.
pub fn advance_disturbance(node: &mut FossilizedFleetNode, ticks: u32) -> u32 {
    let total = node.disturbance.saturating_add(ticks);
    let wakes = total / WAKE_THRESHOLD;
    node.disturbance = total % WAKE_THRESHOLD;
    wakes
}

/// Deterministic drop position for the `index`-th drone dropped
/// overall (a small LCG — replayable, test-exact; the Star-Eater
/// scatter precedent).
#[must_use]
pub fn drop_position(index: u64) -> GridPosition {
    let mut seed = 0x3700_F055_11ED_u64
        ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as i32
    };
    GridPosition {
        x: next().rem_euclid(DROP_GRID_WIDTH),
        y: next().rem_euclid(DROP_GRID_HEIGHT),
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Strip-mining disturbance: every fossil node under active
/// [`FleetMining`] with yield remaining accumulates disturbance;
/// crossing [`WAKE_THRESHOLD`] wakes one Ghost Drone per node.
pub fn process_fossilized_mining(
    mining: Query<&FleetMining>,
    mut nodes: Query<(&mut FossilizedFleetNode, &MiningTarget)>,
    mut state: ResMut<FossilizedFleetState>,
    mut events: EventWriter<GhostDroneSpawnEvent>,
) {
    // Which targets are under active mining this tick (a node
    // mined by two fleets is still disturbed once per tick).
    let mut targets: Vec<Entity> = mining.iter().map(|m| m.target).collect();
    targets.sort_unstable();
    targets.dedup();

    for target in targets {
        let Ok((mut node, mining_target)) = nodes.get_mut(target) else {
            continue; // Not a fossil node: ordinary mining is quiet.
        };
        if mining_target.amount <= 0.0 {
            continue; // Depleted bones disturb nothing.
        }
        state.disturbance_ticks += 1;
        let wakes = advance_disturbance(&mut node, 1);
        for _ in 0..wakes {
            state.drones_woken += 1;
            events.send(GhostDroneSpawnEvent { count: 1 });
        }
    }
}

/// Bridge: each woken Ghost Drone becomes one Layer 1 drop,
/// in the same tick it woke (spec §7: handled immediately).
pub fn handle_ghost_drone_drops(
    mut events: EventReader<GhostDroneSpawnEvent>,
    mut drops: EventWriter<EnemyDropEvent>,
) {
    for event in events.read() {
        for _ in 0..event.count {
            drops.send(EnemyDropEvent {
                kind: EnemyDropKind::GhostDrone,
            });
        }
    }
}

/// The drop itself: spawn each drone on Layer 1 as a real
/// hostile ([`Fauna`] body the existing AI hunts pops with),
/// at its deterministic drop position, and chronicle the fall.
pub fn handle_enemy_drops_system(
    mut events: EventReader<EnemyDropEvent>,
    mut commands: Commands,
    mut state: ResMut<FossilizedFleetState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut dropped_this_tick = 0_u64;
    for event in events.read() {
        match event.kind {
            EnemyDropKind::GhostDrone => {
                let pos = drop_position(state.drones_dropped);
                commands.spawn((
                    GhostDrone,
                    Fauna {
                        state: FaunaState::Wander,
                        target: None,
                        detection_range: DRONE_DETECTION_RANGE,
                        attack_cooldown: 0,
                        ..Default::default()
                    },
                    Health {
                        current: DRONE_HEALTH,
                        max: DRONE_HEALTH,
                        has_rust_lung: false,
                    },
                    pos,
                ));
                state.drones_dropped += 1;
                dropped_this_tick += 1;
            }
        }
    }
    if dropped_this_tick > 0 {
        chronicle.send(AddChronicleEvent {
            text: "Something woke in the fossil fleet. A ghost drone shook off a hundred years of dust and fell, burning, toward the colony.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Completion payout: certify a finished operation's yield
/// into the colony ledger through the real `add_resource`.
pub fn handle_mining_complete_system(
    mut events: EventReader<MiningCompleteEvent>,
    resources: Option<ResMut<ColonyResources>>,
    mut state: ResMut<FossilizedFleetState>,
) {
    let Some(mut resources) = resources else {
        // Still drain the channel so stale completions never replay.
        for _ in events.read() {}
        return;
    };
    for ev in events.read() {
        if !ev.amount.is_finite() || ev.amount <= 0.0 {
            continue;
        }
        let before = resources.get_amount(ev.resource);
        resources.add_resource(&ev.resource, ev.amount);
        if ev.resource == ResourceType::AncientTech {
            state.tech_granted += resources.get_amount(ev.resource) - before;
        }
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census: (fossil nodes, live drones on Layer 1, drones woken,
/// drones dropped, AncientTech stockpiled, AncientTech granted).
#[must_use]
pub fn fossil_counts(world: &mut World) -> (usize, usize, u64, u64, f32, f32) {
    let nodes = {
        let mut query = world.query::<&FossilizedFleetNode>();
        query.iter(world).count()
    };
    let drones = {
        let mut query = world.query::<&GhostDrone>();
        query.iter(world).count()
    };
    let (woken, dropped, granted) = world
        .get_resource::<FossilizedFleetState>()
        .map_or((0, 0, 0.0), |s| {
            (s.drones_woken, s.drones_dropped, s.tech_granted)
        });
    let stockpiled = world
        .get_resource::<ColonyResources>()
        .map_or(0.0, |r| r.get_amount(ResourceType::AncientTech));
    (nodes, drones, woken, dropped, stockpiled, granted)
}

/// One-token STATS label, e.g. `nodes1/drones2/tech7`, or
/// `none` when no fossil node exists and nothing ever woke.
#[must_use]
pub fn fossil_stats_label(world: &mut World) -> String {
    let (nodes, drones, woken, _dropped, stockpiled, _granted) = fossil_counts(world);
    if nodes == 0 && drones == 0 && woken == 0 && stockpiled <= 0.0 {
        "none".to_string()
    } else {
        format!("nodes{nodes}/drones{drones}/tech{stockpiled:.0}")
    }
}

/// Send a wake from a `&mut World` context (headless debug).
pub fn trigger_ghost_drones(world: &mut World, count: u32) {
    world.init_resource::<Events<GhostDroneSpawnEvent>>();
    world
        .resource_mut::<Events<GhostDroneSpawnEvent>>()
        .send(GhostDroneSpawnEvent { count });
}

/// Send a completion from a `&mut World` context (headless debug).
pub fn trigger_mining_complete(world: &mut World, amount: f32) {
    world.init_resource::<Events<MiningCompleteEvent>>();
    world
        .resource_mut::<Events<MiningCompleteEvent>>()
        .send(MiningCompleteEvent {
            resource: ResourceType::AncientTech,
            amount,
        });
}

/// Spawn a fossil node from a `&mut World` context (headless
/// debug): a real mining target of pristine ancient tech,
/// waiting for a fleet to strip it.
pub fn spawn_fossil_node(world: &mut World, amount: f32) -> Entity {
    world
        .spawn((
            MiningTarget {
                resource_type: ResourceType::AncientTech,
                amount,
                mining_difficulty: 2.0,
            },
            FossilizedFleetNode::default(),
        ))
        .id()
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn fossil_public_strings() -> Vec<String> {
    vec![
        "The Fossilized Fleet".to_string(),
        "Ghost Drone".to_string(),
        "AncientTech".to_string(),
        "Something woke in the fossil fleet. A ghost drone shook off a hundred years of dust and fell, burning, toward the colony.".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real codebase — see the
// module docs for every adaptation and why)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer2::fleet::Fleet;
    use crate::layer2::mining::FleetCargo;
    use bevy_ecs::system::RunSystemOnce;

    fn base_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<GhostDroneSpawnEvent>>();
        world.init_resource::<Events<EnemyDropEvent>>();
        world.init_resource::<Events<MiningCompleteEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<FossilizedFleetState>();
        world
    }

    fn spawn_mining_fleet(world: &mut World, target: Entity) -> Entity {
        world
            .spawn((
                Fleet,
                FleetMining {
                    target,
                    rate: 1.0,
                },
                FleetCargo {
                    contents: vec![],
                    capacity: 100.0,
                },
            ))
            .id()
    }

    fn drain_ghost_events(world: &mut World) -> Vec<GhostDroneSpawnEvent> {
        let events = world
            .resource::<Events<GhostDroneSpawnEvent>>()
            .iter_current_update_events()
            .copied()
            .collect();
        world
            .resource_mut::<Events<GhostDroneSpawnEvent>>()
            .clear();
        events
    }

    /// Spec RED test 1, adapted: no `MiningOperation` component
    /// exists — an operation is the real `FleetMining` +
    /// `MiningTarget` pair, and the wake is the §5 deterministic
    /// threshold (exact tick), not a 5% coin flip. Below the
    /// threshold nothing wakes; at it, exactly one drone does.
    #[test]
    fn test_mining_fossilized_fleet_spawns_drones() {
        let mut world = base_world();
        let node = spawn_fossil_node(&mut world, 100.0);
        spawn_mining_fleet(&mut world, node);

        for _ in 0..WAKE_THRESHOLD - 1 {
            world.run_system_once(process_fossilized_mining).unwrap();
        }
        assert!(
            drain_ghost_events(&mut world).is_empty(),
            "no drone may wake below the disturbance threshold"
        );

        world.run_system_once(process_fossilized_mining).unwrap();
        let fired = drain_ghost_events(&mut world);
        assert_eq!(fired.len(), 1, "one wake event at the threshold");
        assert_eq!(fired[0].count, 1);
        assert_eq!(world.resource::<FossilizedFleetState>().drones_woken, 1);
        // The counter reset: another full threshold is required.
        assert_eq!(
            world.get::<FossilizedFleetNode>(node).unwrap().disturbance,
            0
        );
    }

    /// Mining an ordinary (non-fossil) target never wakes drones,
    /// however long it runs.
    #[test]
    fn test_plain_mining_never_wakes_drones() {
        let mut world = base_world();
        let rock = world
            .spawn(MiningTarget {
                resource_type: ResourceType::Ore,
                amount: 100.0,
                mining_difficulty: 1.0,
            })
            .id();
        spawn_mining_fleet(&mut world, rock);

        for _ in 0..WAKE_THRESHOLD * 3 {
            world.run_system_once(process_fossilized_mining).unwrap();
        }
        assert!(drain_ghost_events(&mut world).is_empty());
        assert_eq!(world.resource::<FossilizedFleetState>().drones_woken, 0);
    }

    /// A depleted fossil node is just bones: no yield left, no
    /// disturbance, no wakes.
    #[test]
    fn test_depleted_node_never_wakes() {
        let mut world = base_world();
        let node = spawn_fossil_node(&mut world, 0.0);
        spawn_mining_fleet(&mut world, node);

        for _ in 0..WAKE_THRESHOLD * 2 {
            world.run_system_once(process_fossilized_mining).unwrap();
        }
        assert!(drain_ghost_events(&mut world).is_empty());
    }

    /// Spec RED test 2, adapted: the sketch's `app.update()`
    /// pipeline is the two real systems run in chain order —
    /// the bridge converts one spawn event of count 3 into
    /// exactly 3 Layer 1 drop events.
    #[test]
    fn test_ghost_drones_drop_on_layer1() {
        let mut world = base_world();
        world
            .resource_mut::<Events<GhostDroneSpawnEvent>>()
            .send(GhostDroneSpawnEvent { count: 3 });
        world.run_system_once(handle_ghost_drone_drops).unwrap();

        let drops: Vec<_> = world
            .resource::<Events<EnemyDropEvent>>()
            .iter_current_update_events()
            .copied()
            .collect();
        assert_eq!(drops.len(), 3);
        assert!(drops
            .iter()
            .all(|d| d.kind == EnemyDropKind::GhostDrone));
    }

    /// The drop itself lands real hostiles: each drop event
    /// spawns a Ghost Drone with the fauna body the existing
    /// AI hunts pops with, health, and an in-bounds position.
    #[test]
    fn test_drop_spawns_real_hostile_drones() {
        let mut world = base_world();
        for _ in 0..3 {
            world
                .resource_mut::<Events<EnemyDropEvent>>()
                .send(EnemyDropEvent {
                    kind: EnemyDropKind::GhostDrone,
                });
        }
        world.run_system_once(handle_enemy_drops_system).unwrap();

        let mut query = world.query::<(&GhostDrone, &Fauna, &Health, &GridPosition)>();
        let drones: Vec<_> = query.iter(&world).collect();
        assert_eq!(drones.len(), 3);
        for (_, fauna, health, pos) in &drones {
            assert_eq!(fauna.state, FaunaState::Wander);
            assert!((health.current - DRONE_HEALTH).abs() < f32::EPSILON);
            assert!((0..DROP_GRID_WIDTH).contains(&pos.x));
            assert!((0..DROP_GRID_HEIGHT).contains(&pos.y));
        }
        assert_eq!(world.resource::<FossilizedFleetState>().drones_dropped, 3);
        // The fall is chronicled.
        let chronicles: Vec<_> = world
            .resource::<Events<AddChronicleEvent>>()
            .iter_current_update_events()
            .collect();
        assert_eq!(chronicles.len(), 1);
    }

    /// Spec RED test 3, adapted: the completion payload lands
    /// in the real `f32` ledger (`get_amount` before/after,
    /// exactly the sketch's assertion shape).
    #[test]
    fn test_ancient_tech_reward() {
        let mut world = base_world();
        world.insert_resource(ColonyResources::default());
        let initial = world
            .resource::<ColonyResources>()
            .get_amount(ResourceType::AncientTech);

        world
            .resource_mut::<Events<MiningCompleteEvent>>()
            .send(MiningCompleteEvent {
                resource: ResourceType::AncientTech,
                amount: 5.0,
            });
        world.run_system_once(handle_mining_complete_system).unwrap();

        let current = world
            .resource::<ColonyResources>()
            .get_amount(ResourceType::AncientTech);
        assert!((current - (initial + 5.0)).abs() < f32::EPSILON);
        assert!((world.resource::<FossilizedFleetState>().tech_granted - 5.0).abs() < f32::EPSILON);
    }

    /// The threshold math is exact and shared: 11 quiet ticks,
    /// the 12th wakes, and a long strip wakes repeatedly.
    #[test]
    fn test_advance_disturbance_threshold_math() {
        let mut node = FossilizedFleetNode::default();
        assert_eq!(advance_disturbance(&mut node, WAKE_THRESHOLD - 1), 0);
        assert_eq!(node.disturbance, WAKE_THRESHOLD - 1);
        assert_eq!(advance_disturbance(&mut node, 1), 1);
        assert_eq!(node.disturbance, 0);
        assert_eq!(advance_disturbance(&mut node, WAKE_THRESHOLD * 3), 3);
        assert_eq!(node.disturbance, 0);
    }

    /// Drop positions are deterministic per drop index and
    /// always land inside the surface grid.
    #[test]
    fn test_drop_positions_deterministic() {
        assert_eq!(drop_position(7), drop_position(7));
        for index in 0..64 {
            let pos = drop_position(index);
            assert!((0..DROP_GRID_WIDTH).contains(&pos.x), "x={}", pos.x);
            assert!((0..DROP_GRID_HEIGHT).contains(&pos.y), "y={}", pos.y);
        }
        // Not every drone lands on the same tile.
        let first = drop_position(0);
        assert!((0..64).any(|i| drop_position(i) != first));
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "galactus",
            "marvel",
            "mass effect",
            "reaper",
            "rimworld",
            "dwarf fortress",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
            "halo",
            "forerunner",
            "promethean",
        ];
        for text in fossil_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
