//! Spec 373: The Void Nomad Fleet.
//!
//! "A city that never stops moving." A wandering fleet passes
//! through the Layer 2 system, strip-mines asteroid nodes in its
//! path, and opens a temporary trade market while it is present.
//! Peaceful — but if you shoot at it to protect your mines, its
//! ancient weaponry is overwhelming (see [`NomadCombatStats`] and
//! [`nomad_fleet_composition`]).
//!
//! ## Adaptations from the mock-style spec (established pattern)
//!
//! * **The spec's `layer2::map` / `layer2::resources` modules do
//!   not exist.** There was no `SystemNode`, `NodeType`,
//!   `SystemPosition`, or `NodeResources` anywhere in Layer 2 —
//!   the real Layer 2 bodies are [`OrbitalBody`][crate::layer2::
//!   system::OrbitalBody] + [`Orbit`][crate::layer2::system::
//!   Orbit] entities, and the real mineable-resource seam is
//!   [`MiningTarget`][crate::layer2::mining::MiningTarget] (a
//!   fleet in orbit mines it into its `FleetCargo`). Per the
//!   brief (spec wins on mechanics, real codebase wins on type
//!   names/paths), this module defines the spec's four map types
//!   here, at the real new path `layer2::entities::nomad_fleet`,
//!   so the spec's mechanics run verbatim against them — AND
//!   [`process_nomad_orbit_mining_system`] bridges the same
//!   strip-mining onto real `MiningTarget` asteroids for fleets
//!   holding [`InOrbit`], so nomads deplete the resources the
//!   rest of the game actually mines. No parallel mining economy
//!   is invented.
//! * **Random arrival, deterministic.** The spec sketch rolls
//!   `rand::thread_rng()` at 1%/tick. This module keeps the 1%
//!   per-tick chance but rolls it from a seeded LCG carried in
//!   [`NomadArrivalState`] (the fossil-fleet deterministic
//!   precedent): the same world replays identically, and tests
//!   can pin `chance_per_tick` to 1.0 / 0.0 for exact RED/GREEN.
//! * **Trade Hub (REFACTOR).** A trade seam exists —
//!   `layer1::trade::TradeMarket` — but it is a colony-wide
//!   price/legal-status registry with no temporary-listing
//!   concept, so there is no honest way to hang a *temporary*
//!   market off it. The nomads therefore carry their temporary
//!   market on the fleet entity itself ([`NomadTradeMarket`],
//!   priced in real [`ResourceType`]s, high-tier goods); it
//!   opens on arrival and dies with the fleet. The global
//!   registry is deliberately left alone (documented, not
//!   silently skipped).
//! * **Aggression Reaction (REFACTOR).** The combat seam is
//!   real: `layer2::combat::fleet_combat_system` groups fleets
//!   by [`InOrbit`] parent and resolves hostile pairs through
//!   `resolve_combat` over [`FleetComposition`]. Nomad fleets
//!   spawn with that seam fully wired — `Fleet` +
//!   `FleetFaction::Merchant` (neutral traders; a player fleet
//!   sharing their orbit is a hostile pair) + an overwhelming
//!   [`nomad_fleet_composition`] (20 ancient frigates: 1,000
//!   attack power against a frigate's 50) — plus
//!   [`NomadCombatStats`] recording the same overwhelming
//!   numbers for any future direct-duel seam. Attacking them
//!   through the existing combat system is suicide, as specced.
//!   No new `FleetFaction` variant was added: exhaustive matches
//!   on that enum elsewhere were not worth the blast radius.
//! * **Pathfinding (REFACTOR).** [`nomad_movement_system`] is
//!   goal-directed, not a random walk: each tick a fleet steps
//!   one tile (Chebyshev) toward the nearest asteroid node that
//!   still holds resources, and holds position while it is
//!   sitting on one (mining). The entity-based `Fleet` /
//!   `InTransit` transit seam is left alone — the nomads'
//!   locomotion lives on the spec's own `SystemPosition` map.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::resources::ResourceType;
use crate::layer2::fleet::{Fleet, FleetComposition, FleetFaction, InOrbit};
use crate::layer2::mining::MiningTarget;
use crate::layer2::ship::{Ship, ShipType};

/// Default strip-mining rate (metal per tick; volatiles at half).
pub const DEFAULT_MINING_RATE: f32 = 10.0;
/// Default per-tick arrival chance (the spec's 1%).
pub const DEFAULT_ARRIVAL_CHANCE: f32 = 0.01;
/// The nomad fleet's recorded attack power — ancient weaponry,
/// two orders of magnitude past a single frigate (50).
pub const NOMAD_ATTACK_POWER: f32 = 5_000.0;
/// The nomad fleet's recorded hull — see [`NOMAD_ATTACK_POWER`].
pub const NOMAD_HULL: f32 = 5_000.0;
/// Ancient frigates in the spawned [`FleetComposition`]:
/// 20 x 50 attack = 1,000 power through the real combat seam.
pub const NOMAD_ESCORT_FRIGATES: usize = 20;

// ---------------------------------------------------------------------------
// Spec map types (see module docs: no layer2::map existed)
// ---------------------------------------------------------------------------

/// Integer position on the Layer 2 system map (spec type).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SystemPosition {
    /// Map column.
    pub x: i32,
    /// Map row.
    pub y: i32,
}

impl SystemPosition {
    /// Chebyshev distance to another position.
    #[must_use]
    pub fn chebyshev_distance(self, other: Self) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }

    /// One step toward `other` (each axis moves by its sign).
    #[must_use]
    pub fn step_toward(self, other: Self) -> Self {
        Self {
            x: self.x + (other.x - self.x).signum(),
            y: self.y + (other.y - self.y).signum(),
        }
    }
}

/// What a system node is (spec type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// Mineable rock — the nomads' target.
    Asteroid,
    /// A planet.
    Planet,
    /// A moon (also strip-mined in the fiction; mechanics target
    /// asteroids, per the spec's test).
    Moon,
    /// A station.
    Station,
}

/// A node on the system map (spec type).
#[derive(Component, Debug, Clone, Copy)]
pub struct SystemNode {
    /// What this node is.
    pub node_type: NodeType,
}

/// Resources held by a node (spec type).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NodeResources {
    /// Metal remaining.
    pub metal: f32,
    /// Volatiles remaining.
    pub volatiles: f32,
}

impl NodeResources {
    /// Total remaining yield.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.metal + self.volatiles
    }
}

// ---------------------------------------------------------------------------
// The fleet
// ---------------------------------------------------------------------------

/// The Void Nomad Fleet: a city that never stops moving.
/// Kept to the spec's exact shape (`mining_rate` only) so the
/// spec's literal construction compiles unchanged; combat and
/// trade state ride in sibling components.
#[derive(Component, Debug, Clone, Copy)]
pub struct NomadFleet {
    /// Metal strip-mined per tick while overlapping an asteroid
    /// (volatiles are taken at half this rate).
    pub mining_rate: f32,
}

impl Default for NomadFleet {
    fn default() -> Self {
        Self {
            mining_rate: DEFAULT_MINING_RATE,
        }
    }
}

/// Overwhelming ancient combat stats, recorded on the fleet for
/// direct-duel seams; the live combat seam expresses the same
/// fact through [`nomad_fleet_composition`].
#[derive(Component, Debug, Clone, Copy)]
pub struct NomadCombatStats {
    /// Attack power (a frigate is 50).
    pub attack_power: f32,
    /// Hull integrity.
    pub hull: f32,
}

impl NomadCombatStats {
    /// The specced overwhelming stat block.
    #[must_use]
    pub fn overwhelming() -> Self {
        Self {
            attack_power: NOMAD_ATTACK_POWER,
            hull: NOMAD_HULL,
        }
    }
}

/// One line in the nomads' temporary market.
#[derive(Debug, Clone, PartialEq)]
pub struct NomadTradeGood {
    /// What is for sale (a real colony resource type).
    pub resource: ResourceType,
    /// Asking price per unit (credits).
    pub price: f32,
    /// Units aboard.
    pub stock: f32,
}

/// The temporary trade market a nomad fleet opens on arrival.
/// Lives on the fleet entity: it closes when the fleet is gone.
#[derive(Component, Debug, Clone)]
pub struct NomadTradeMarket {
    /// Whether the market is currently open.
    pub open: bool,
    /// High-tier goods on offer.
    pub goods: Vec<NomadTradeGood>,
}

impl NomadTradeMarket {
    /// The nomads' standard high-tier listing.
    #[must_use]
    pub fn nomad_goods() -> Self {
        Self {
            open: true,
            goods: vec![
                NomadTradeGood {
                    resource: ResourceType::AncientTech,
                    price: 500.0,
                    stock: 5.0,
                },
                NomadTradeGood {
                    resource: ResourceType::HyperValuable,
                    price: 250.0,
                    stock: 10.0,
                },
                NomadTradeGood {
                    resource: ResourceType::MemoryCore,
                    price: 300.0,
                    stock: 3.0,
                },
                NomadTradeGood {
                    resource: ResourceType::Fuel,
                    price: 30.0,
                    stock: 100.0,
                },
            ],
        }
    }
}

/// The fleet composition the combat seam fights with:
/// [`NOMAD_ESCORT_FRIGATES`] ancient frigates.
#[must_use]
pub fn nomad_fleet_composition() -> FleetComposition {
    let mut comp = FleetComposition::default();
    for _ in 0..NOMAD_ESCORT_FRIGATES {
        comp.add_ship(Ship::new(ShipType::Frigate));
    }
    comp
}

/// A nomad fleet has entered the system (spec event).
#[derive(Event, Debug, Clone, Copy)]
pub struct NomadArrivalEvent {
    /// Where the fleet enters the map.
    pub entry_point: SystemPosition,
}

/// Arrival + strip-mining ledger (STATS / panel surface) and
/// the deterministic arrival-roll state.
#[derive(Resource, Debug, Clone)]
pub struct NomadArrivalState {
    /// Per-tick arrival probability (spec: 0.01).
    pub chance_per_tick: f32,
    /// Seeded LCG state for the arrival roll.
    pub rng_state: u64,
    /// Arrival events fired.
    pub arrivals: u64,
    /// Fleets actually spawned by the arrival handler.
    pub fleets_spawned: u64,
    /// Total metal strip-mined from nodes / mining targets.
    pub total_metal_mined: f32,
    /// Total volatiles strip-mined from nodes.
    pub total_volatiles_mined: f32,
    /// Temporary markets opened.
    pub markets_opened: u64,
}

impl Default for NomadArrivalState {
    fn default() -> Self {
        Self {
            chance_per_tick: DEFAULT_ARRIVAL_CHANCE,
            rng_state: 9_814_723_657,
            arrivals: 0,
            fleets_spawned: 0,
            total_metal_mined: 0.0,
            total_volatiles_mined: 0.0,
            markets_opened: 0,
        }
    }
}

/// Next deterministic roll in `[0, 1)` from the state's LCG.
fn next_roll(state: &mut NomadArrivalState) -> f32 {
    state.rng_state = state
        .rng_state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    #[allow(clippy::cast_precision_loss)]
    let bits = ((state.rng_state >> 40) & 0x00FF_FFFF) as f32;
    bits / 16_777_216.0
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Random arrival generation (spec system, name kept): each
/// tick, roll `chance_per_tick`; on success the fleet enters at
/// a deterministic edge point derived from the same roll stream.
pub fn trigger_nomad_arrival_system(
    state: Option<ResMut<NomadArrivalState>>,
    mut events: EventWriter<NomadArrivalEvent>,
) {
    let Some(mut state) = state else {
        return;
    };
    let roll = next_roll(&mut state);
    if roll < state.chance_per_tick {
        state.arrivals += 1;
        #[allow(clippy::cast_possible_truncation)]
        let edge = (next_roll(&mut state) * 24.0) as i32 - 12;
        events.send(NomadArrivalEvent {
            entry_point: SystemPosition { x: -12, y: edge },
        });
    }
}

/// Arrival handling: spawn the fleet at its entry point with
/// the full seam bundle (combat composition + stats, temporary
/// trade market), count it, and chronicle the arrival.
pub fn handle_nomad_arrival_system(
    mut events: EventReader<NomadArrivalEvent>,
    mut commands: Commands,
    state: Option<ResMut<NomadArrivalState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut spawned = 0_u64;
    for event in events.read() {
        commands.spawn((
            NomadFleet::default(),
            event.entry_point,
            Fleet,
            FleetFaction::Merchant,
            nomad_fleet_composition(),
            NomadCombatStats::overwhelming(),
            NomadTradeMarket::nomad_goods(),
        ));
        spawned += 1;
    }
    if spawned > 0 {
        if let Some(mut state) = state {
            state.fleets_spawned += spawned;
            state.markets_opened += spawned;
        }
        chronicle.send(AddChronicleEvent {
            text: "A city that never stops moving has entered the system. The Void Nomad Fleet is strip-mining as it comes — and its market is open while it passes.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Goal-directed movement (REFACTOR): step one tile toward the
/// nearest asteroid node that still holds resources; hold
/// position while sitting on one (that is the mining posture).
pub fn nomad_movement_system(
    mut fleets: Query<(&NomadFleet, &mut SystemPosition)>,
    // Without<NomadFleet> makes the two SystemPosition accesses
    // provably disjoint (B0001): a fleet is never a node.
    nodes: Query<(&SystemNode, &SystemPosition, &NodeResources), Without<NomadFleet>>,
) {
    // Snapshot targets once per tick (nearest-first is computed
    // per fleet from the same list).
    let targets: Vec<SystemPosition> = nodes
        .iter()
        .filter(|(node, _, resources)| {
            node.node_type == NodeType::Asteroid && resources.total() > 0.0
        })
        .map(|(_, pos, _)| *pos)
        .collect();
    if targets.is_empty() {
        return;
    }
    for (_fleet, mut pos) in &mut fleets {
        if targets.contains(&pos) {
            continue; // On a live asteroid: stay and mine.
        }
        if let Some(nearest) = targets
            .iter()
            .min_by_key(|target| pos.chebyshev_distance(**target))
        {
            *pos = pos.step_toward(*nearest);
        }
    }
}

/// Strip-mining (spec system, name kept verbatim): a fleet
/// overlapping an `Asteroid` node depletes its `NodeResources`
/// — metal at `mining_rate`, volatiles at half — clamped at 0.
pub fn process_nomad_mining_system(
    fleets: Query<(&NomadFleet, &SystemPosition)>,
    mut nodes: Query<(&SystemPosition, &SystemNode, &mut NodeResources)>,
    state: Option<ResMut<NomadArrivalState>>,
) {
    let mut metal_taken = 0.0_f32;
    let mut volatiles_taken = 0.0_f32;
    for (fleet, fleet_pos) in fleets.iter() {
        for (node_pos, node, mut resources) in nodes.iter_mut() {
            if *fleet_pos == *node_pos && node.node_type == NodeType::Asteroid {
                let metal = fleet.mining_rate.min(resources.metal);
                let volatiles = (fleet.mining_rate / 2.0).min(resources.volatiles);
                resources.metal -= metal;
                resources.volatiles -= volatiles;
                metal_taken += metal;
                volatiles_taken += volatiles;
            }
        }
    }
    if let Some(mut state) = state {
        state.total_metal_mined += metal_taken;
        state.total_volatiles_mined += volatiles_taken;
    }
}

/// Real-seam mining bridge: a nomad fleet holding [`InOrbit`]
/// of a [`MiningTarget`] depletes the target's real `amount`
/// at its mining rate (the stock the game's own fleets mine).
pub fn process_nomad_orbit_mining_system(
    fleets: Query<(&NomadFleet, &InOrbit)>,
    mut targets: Query<&mut MiningTarget>,
    state: Option<ResMut<NomadArrivalState>>,
) {
    let mut taken = 0.0_f32;
    for (fleet, orbit) in fleets.iter() {
        if let Ok(mut target) = targets.get_mut(orbit.parent) {
            let amount = fleet.mining_rate.min(target.amount);
            target.amount -= amount;
            taken += amount;
        }
    }
    if let Some(mut state) = state {
        state.total_metal_mined += taken;
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census: (live nomad fleets, arrivals, metal mined,
/// volatiles mined, markets opened).
#[must_use]
pub fn nomad_counts(world: &mut World) -> (usize, u64, f32, f32, u64) {
    let fleets = {
        let mut query = world.query::<&NomadFleet>();
        query.iter(world).count()
    };
    let (arrivals, metal, volatiles, markets) = world
        .get_resource::<NomadArrivalState>()
        .map_or((0, 0.0, 0.0, 0), |s| {
            (
                s.arrivals.max(s.fleets_spawned),
                s.total_metal_mined,
                s.total_volatiles_mined,
                s.markets_opened,
            )
        });
    (fleets, arrivals, metal, volatiles, markets)
}

/// One-token STATS label, e.g. `fleets1/mined140`, or `none`
/// when no fleet is present and none ever mined.
#[must_use]
pub fn nomad_stats_label(world: &mut World) -> String {
    let (fleets, arrivals, metal, _, _) = nomad_counts(world);
    if fleets == 0 && arrivals == 0 && metal <= 0.0 {
        "none".to_string()
    } else {
        format!("fleets{fleets}/mined{metal:.0}")
    }
}

/// Spawn a nomad fleet from a `&mut World` context (headless
/// debug) at `pos`, with the full arrival seam bundle.
pub fn spawn_nomad_fleet(world: &mut World, pos: SystemPosition) -> Entity {
    world
        .spawn((
            NomadFleet::default(),
            pos,
            Fleet,
            FleetFaction::Merchant,
            nomad_fleet_composition(),
            NomadCombatStats::overwhelming(),
            NomadTradeMarket::nomad_goods(),
        ))
        .id()
}

/// Fire an arrival event from a `&mut World` context (headless
/// debug); the arrival handler spawns the fleet next tick.
pub fn trigger_nomad_arrival(world: &mut World) {
    world.init_resource::<Events<NomadArrivalEvent>>();
    world.resource_mut::<Events<NomadArrivalEvent>>().send(
        NomadArrivalEvent {
            entry_point: SystemPosition { x: -12, y: 0 },
        },
    );
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn nomad_public_strings() -> Vec<String> {
    vec![
        "The Void Nomad Fleet".to_string(),
        "A city that never stops moving has entered the system. The Void Nomad Fleet is strip-mining as it comes — and its market is open while it passes.".to_string(),
    ]
}
