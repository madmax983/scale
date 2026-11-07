//! Spec 400: Lagrange Point Hubs.
//!
//! In a star system (Layer 2), stations holding a planetary orbit pay
//! constant station-keeping fuel upkeep. Gravity-neutral Lagrange
//! points (L-points) hold position for free — so they naturally become
//! trade hubs when the player builds there, and pirate dens when the
//! player ignores them.
//!
//! ## Grounding audit (spec §8 pattern: adapt to the real codebase)
//!
//! * **The spec sketch's `layer2::system::SystemNode` does not exist.**
//!   The real Layer 2 bodies are [`OrbitalBody`][crate::layer2::system::
//!   OrbitalBody] + [`Orbit`][crate::layer2::system::Orbit] entities;
//!   the only `SystemNode` in Layer 2 is the spec-373 type at
//!   [`crate::layer2::entities::nomad_fleet::SystemNode`] (a map node
//!   with a `NodeType`). L-points here are entities carrying
//!   [`LagrangePoint`] (plus, when spawned through
//!   [`spawn_lagrange_point`], a real `SystemNode` / `OrbitalBody` /
//!   `Orbit` so they live on the same system map as everything else).
//! * **The spec sketch's `layer2::station::{OrbitalStation,
//!   StationFuelUpkeep}` do not exist.** The real station is
//!   [`crate::layer2::station::Station`] (spawned by
//!   `build_station_system` with an `Orbit` whose `parent` is the body
//!   it circles), and there is *no* station fuel-upkeep component or
//!   system anywhere — the only upkeep in Layer 2 is the Debt-Trap
//!   Megastructure's credit upkeep, a different seam that stays
//!   untouched. This module therefore builds the missing leg with the
//!   spec's exact type shapes ([`OrbitalStation`] /
//!   [`StationFuelUpkeep`]) and bridges it to real stations: the
//!   upkeep rule runs over BOTH the spec leg (`OrbitalStation.
//!   parent_node`) and the real leg (`Station` + `Orbit.parent`), so a
//!   station built by the real build system at an L-point also pays
//!   zero. No parallel station system is invented: a hub spawned by
//!   [`spawn_hub_station`] is one entity carrying both legs.
//! * **Pirates.** The real pirate seams are Layer 1 raids and
//!   `layer2::piracy`'s `PirateHaven` progression (wealth -> level ->
//!   republic), neither of which models a *place* in the system view.
//!   [`PirateBase`] is the missing place: it spawns attached to its
//!   node through the real [`InOrbit`] fleet seam and carries the
//!   spec's `strength`. Haven progression is left alone.
//! * **Merchant traffic.** The real merchant seam is
//!   `layer2::trade::merchant_landings` (ships landing on a Layer 1
//!   Trade Pad) — a colony-surface flow with no system-view routing,
//!   so there is no honest hook for "ships prefer this orbit" inside
//!   it. Following the spec-373 precedent (temporary markets ride the
//!   entity, the global registry is left alone), hub traffic rides the
//!   station itself: [`HubTraffic`] counts merchant visits rolled by
//!   [`lagrange_merchant_traffic_system`], and
//!   [`merchant_visit_chance`] prices the L-point advantage at exactly
//!   [`LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER`]x the ordinary-orbit
//!   chance. The landing ledger is deliberately untouched.
//! * **Spawn cadence (spec §7).** Not modulo-only and not
//!   `thread_rng`: [`deterministic_roll`] is a splitmix64 hash of
//!   (game seed, tick, point ordinal), so a given tick always rolls
//!   the same outcome for a given seed — reloading a save and
//!   re-running the same tick cannot re-roll a pirate spawn, while
//!   waiting for a different check tick legitimately changes the
//!   odds. The seed lives in [`LagrangeState`], the module's ledger
//!   resource (the merchant-landings deterministic-state pattern).
//!
//! All names and strings are original and generic — "Lagrange point"
//! is the ordinary orbital-mechanics term. See the IP-guard test.

use bevy_ecs::prelude::*;
use ratatui::style::Color;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer2::entities::nomad_fleet::{NodeType, SystemNode};
use crate::layer2::fleet::InOrbit;
use crate::layer2::station::Station;
use crate::layer2::system::{Orbit, OrbitalBody};
use crate::shared::time::SimulationTime;

/// Default game seed for the deterministic L-point rolls.
pub const DEFAULT_LAGRANGE_SEED: u64 = 0x4000_5EED_1A6B_0001;
/// Earliest tick at which pirates may take root in an empty L-point.
pub const PIRATE_MIN_TICK: u64 = 5_000;
/// Ticks between pirate spawn checks on empty L-points.
pub const PIRATE_CHECK_INTERVAL: u64 = 500;
/// Chance that one empty L-point gains a pirate base at one check.
pub const PIRATE_SPAWN_CHANCE: f32 = 0.5;
/// Strength of a freshly rooted pirate base.
pub const PIRATE_BASE_STRENGTH: u32 = 100;
/// Ticks between merchant-traffic rolls for hub stations.
pub const TRAFFIC_CHECK_INTERVAL: u64 = 250;
/// Merchant-visit chance per traffic check at an ordinary orbit.
pub const BASE_MERCHANT_VISIT_CHANCE: f32 = 0.25;
/// How much more merchant traffic an L-point hub attracts (§5).
pub const LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER: f32 = 2.0;
/// Default station-keeping fuel upkeep for a hub station.
pub const DEFAULT_STATION_UPKEEP: u32 = 10;

/// Which of a planet's five Lagrange points this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LagrangeSlot {
    /// Between star and planet.
    #[default]
    L1,
    /// Just beyond the planet.
    L2,
    /// Behind the star, opposite the planet.
    L3,
    /// Leading stable point (60 degrees ahead).
    L4,
    /// Trailing stable point (60 degrees behind).
    L5,
}

impl LagrangeSlot {
    /// All five slots, in order.
    pub const ALL: [Self; 5] = [Self::L1, Self::L2, Self::L3, Self::L4, Self::L5];

    /// Short label ("L1" .. "L5").
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L4 => "L4",
            Self::L5 => "L5",
        }
    }

    /// Parse a slot label (case-insensitive); `None` when unknown.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "L1" | "1" => Some(Self::L1),
            "L2" | "2" => Some(Self::L2),
            "L3" | "3" => Some(Self::L3),
            "L4" | "4" => Some(Self::L4),
            "L5" | "5" => Some(Self::L5),
            _ => None,
        }
    }

    /// Orbit-radius factor relative to the planet's orbit.
    #[must_use]
    pub const fn radius_factor(self) -> f32 {
        match self {
            Self::L1 => 0.85,
            Self::L2 => 1.15,
            Self::L3 | Self::L4 | Self::L5 => 1.0,
        }
    }

    /// Angle offset from the planet's orbital angle (radians).
    #[must_use]
    pub const fn angle_offset(self) -> f32 {
        match self {
            Self::L1 | Self::L2 => 0.0,
            Self::L3 => std::f32::consts::PI,
            Self::L4 => std::f32::consts::FRAC_PI_3,
            Self::L5 => -std::f32::consts::FRAC_PI_3,
        }
    }
}

/// The orbit an L-point holds: it co-rotates with its planet around
/// the planet's parent (the star), at the slot's radius/angle offset.
/// This is the §5 spatial representation — L-points sit at the
/// mathematically appropriate places relative to their planet.
#[must_use]
pub fn lagrange_orbit(planet_orbit: &Orbit, slot: LagrangeSlot) -> Orbit {
    Orbit {
        parent: planet_orbit.parent,
        radius: planet_orbit.radius * slot.radius_factor(),
        speed: planet_orbit.speed,
        angle: planet_orbit.angle + slot.angle_offset(),
    }
}

/// Marker: this system node is a gravity-neutral Lagrange point.
/// Stations orbiting it pay zero station-keeping fuel upkeep.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct LagrangePoint {
    /// Which slot this point occupies.
    pub slot: LagrangeSlot,
}

/// A station anchored to a system node (spec type, built here — see
/// the module grounding audit). Real stations additionally carry
/// [`Station`] + [`Orbit`]; the two legs agree on the parent node.
#[derive(Component, Debug, Clone, Copy)]
pub struct OrbitalStation {
    /// The node this station is anchored to.
    pub parent_node: Entity,
}

/// Station-keeping fuel upkeep for a station (spec type).
#[derive(Component, Debug, Clone, Copy)]
pub struct StationFuelUpkeep {
    /// Upkeep at an ordinary orbit.
    pub base_cost: u32,
    /// Upkeep after the L-point rule is applied.
    pub current_cost: u32,
}

/// A pirate base that has taken root at an L-point. Its node is the
/// real [`InOrbit`] parent it spawns with.
#[derive(Component, Debug, Clone, Copy)]
pub struct PirateBase {
    /// Fighting strength of the base.
    pub strength: u32,
}

/// Merchant visits attracted by a hub station (rides the station —
/// see the module grounding audit).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct HubTraffic {
    /// Merchant visits recorded so far.
    pub merchant_visits: u64,
}

/// Ledger + deterministic seed for the L-point systems.
#[derive(Resource, Debug, Clone)]
pub struct LagrangeState {
    /// Game seed for the deterministic rolls.
    pub seed: u64,
    /// Pirate bases rooted so far.
    pub pirate_bases_spawned: u64,
    /// Merchant visits recorded at hub stations so far.
    pub merchant_visits: u64,
    /// Last tick the pirate spawn check ran (same-tick guard).
    pub last_pirate_check_tick: u64,
    /// Last tick the traffic check ran (same-tick guard).
    pub last_traffic_tick: u64,
}

impl Default for LagrangeState {
    fn default() -> Self {
        Self {
            seed: DEFAULT_LAGRANGE_SEED,
            pirate_bases_spawned: 0,
            merchant_visits: 0,
            last_pirate_check_tick: 0,
            last_traffic_tick: 0,
        }
    }
}

/// Deterministic roll in `[0, 1)` from (seed, tick, ordinal) —
/// splitmix64. Same inputs always roll the same value, per spec §7.
#[must_use]
pub fn deterministic_roll(seed: u64, tick: u64, ordinal: u64) -> f32 {
    let mut z = seed
        ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ordinal.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    #[allow(clippy::cast_precision_loss)]
    let f = (z >> 11) as f32 / (1u64 << 53) as f32;
    f.min(0.999_999)
}

/// The upkeep a station actually pays: zero at an L-point, full
/// `base_cost` anywhere else.
#[must_use]
pub const fn effective_upkeep(base_cost: u32, at_lagrange: bool) -> u32 {
    if at_lagrange {
        0
    } else {
        base_cost
    }
}

/// Merchant-visit chance per traffic check: L-point hubs attract
/// [`LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER`]x the ordinary traffic.
#[must_use]
pub fn merchant_visit_chance(at_lagrange: bool) -> f32 {
    if at_lagrange {
        (BASE_MERCHANT_VISIT_CHANCE * LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER).min(1.0)
    } else {
        BASE_MERCHANT_VISIT_CHANCE
    }
}

/// Spec leg: recompute upkeep for [`OrbitalStation`]s from their
/// anchored node.
pub fn calculate_station_upkeep_system(
    mut stations: Query<(&OrbitalStation, &mut StationFuelUpkeep)>,
    l_points: Query<(), With<LagrangePoint>>,
) {
    for (station, mut upkeep) in stations.iter_mut() {
        let at_lagrange = l_points.get(station.parent_node).is_ok();
        upkeep.current_cost = effective_upkeep(upkeep.base_cost, at_lagrange);
    }
}

/// Real-station leg: the same rule for stations built by the real
/// build system ([`Station`] + [`Orbit`]), whose parent is their
/// orbit's parent body.
pub fn calculate_real_station_upkeep_system(
    mut stations: Query<(&Orbit, &mut StationFuelUpkeep), With<Station>>,
    l_points: Query<(), With<LagrangePoint>>,
) {
    for (orbit, mut upkeep) in stations.iter_mut() {
        let at_lagrange = l_points.get(orbit.parent).is_ok();
        upkeep.current_cost = effective_upkeep(upkeep.base_cost, at_lagrange);
    }
}

/// Sorted L-point entities — the ordinal order the deterministic
/// rolls are keyed to.
fn sorted_l_points(l_points: &Query<(Entity, &LagrangePoint)>) -> Vec<Entity> {
    let mut points: Vec<Entity> = l_points.iter().map(|(e, _)| e).collect();
    points.sort_unstable();
    points
}

/// Empty L-points periodically sprout pirate bases (spec §3/§5):
/// at a check tick, each point with no player station and no pirate
/// base already rooted rolls the deterministic (seed, tick, ordinal)
/// roll; a success spawns a [`PirateBase`] on the real [`InOrbit`]
/// seam and writes the §5 chronicle entry.
#[allow(clippy::too_many_arguments)]
pub fn pirate_l_point_spawn_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    state: Option<ResMut<LagrangeState>>,
    l_points: Query<(Entity, &LagrangePoint)>,
    orbital_stations: Query<&OrbitalStation>,
    real_stations: Query<&Orbit, With<Station>>,
    pirate_bases: Query<&InOrbit, With<PirateBase>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let tick = time.tick;
    if tick < PIRATE_MIN_TICK || tick == 0 || !tick.is_multiple_of(PIRATE_CHECK_INTERVAL) {
        return;
    }
    let seed = state.as_ref().map_or(DEFAULT_LAGRANGE_SEED, |s| s.seed);
    if let Some(state) = &state {
        if state.last_pirate_check_tick == tick {
            return;
        }
    }
    // Occupancy: any player station (either leg) or an existing base.
    let mut occupied: Vec<Entity> = Vec::new();
    for station in orbital_stations.iter() {
        occupied.push(station.parent_node);
    }
    for orbit in real_stations.iter() {
        occupied.push(orbit.parent);
    }
    for orbit in pirate_bases.iter() {
        occupied.push(orbit.parent);
    }
    let points = sorted_l_points(&l_points);
    let mut spawned = 0u64;
    for (ordinal, node) in points.iter().enumerate() {
        if occupied.contains(node) {
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        let roll = deterministic_roll(seed, tick, ordinal as u64);
        if roll < PIRATE_SPAWN_CHANCE {
            commands.spawn((
                PirateBase {
                    strength: PIRATE_BASE_STRENGTH,
                },
                InOrbit { parent: *node },
                OrbitalBody {
                    name: "Pirate Anchorage".to_string(),
                    radius: 0.5,
                    color: Color::Red,
                    char: 'P',
                },
            ));
            spawned += 1;
            chronicle.send(AddChronicleEvent {
                text: "Pirates have taken root at an unguarded Lagrange point — a pirate base now nests in the gravity-neutral shallows, and it will not leave on its own.".to_string(),
                importance: EventImportance::Major,
            });
        }
    }
    if let Some(mut state) = state {
        state.last_pirate_check_tick = tick;
        state.pirate_bases_spawned += spawned;
    }
}

/// Hub stations attract merchant traffic (§5): at a traffic check
/// tick, each station carrying [`HubTraffic`] rolls for a merchant
/// visit — at [`merchant_visit_chance`], so L-point hubs draw double
/// the traffic of ordinary orbits. The first visit to a station is
/// chronicled; the rest accumulate in the ledger.
#[allow(clippy::too_many_arguments)]
pub fn lagrange_merchant_traffic_system(
    time: Res<SimulationTime>,
    state: Option<ResMut<LagrangeState>>,
    mut hubs: Query<(Entity, &mut HubTraffic, Option<&OrbitalStation>, Option<&Orbit>)>,
    l_points: Query<(), With<LagrangePoint>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let tick = time.tick;
    if tick == 0 || !tick.is_multiple_of(TRAFFIC_CHECK_INTERVAL) {
        return;
    }
    let seed = state.as_ref().map_or(DEFAULT_LAGRANGE_SEED, |s| s.seed);
    if let Some(state) = &state {
        if state.last_traffic_tick == tick {
            return;
        }
    }
    let mut stations: Vec<(Entity, bool, u64)> = hubs
        .iter()
        .map(|(entity, hub, orbital, orbit)| {
            let at_lagrange = orbital.is_some_and(|s| l_points.get(s.parent_node).is_ok())
                || orbit.is_some_and(|o| l_points.get(o.parent).is_ok());
            (entity, at_lagrange, hub.merchant_visits)
        })
        .collect();
    stations.sort_unstable();
    // Decide every visit from the pre-pass snapshot first, then apply
    // them in a second pass over the same query (one query only — a
    // second HubTraffic query would conflict, B0001).
    let mut arrivals: Vec<(Entity, u64)> = Vec::new();
    for (ordinal, (entity, at_lagrange, prior_visits)) in stations.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let roll = deterministic_roll(seed, tick, ordinal as u64);
        if roll < merchant_visit_chance(*at_lagrange) {
            arrivals.push((*entity, *prior_visits));
        }
    }
    let mut visits = 0u64;
    for (entity, prior_visits) in &arrivals {
        if let Ok((_, mut hub, _, _)) = hubs.get_mut(*entity) {
            hub.merchant_visits += 1;
            visits += 1;
            if *prior_visits == 0 {
                chronicle.send(AddChronicleEvent {
                    text: "Merchant traffic is flocking to the Lagrange hub — free docking in the gravity-neutral shallows makes it the natural port of call.".to_string(),
                    importance: EventImportance::Standard,
                });
            }
        }
    }
    if let Some(mut state) = state {
        state.last_traffic_tick = tick;
        state.merchant_visits += visits;
    }
}

// ---------------------------------------------------------------------------
// World helpers (headless / census)
// ---------------------------------------------------------------------------

/// Census of the L-point layer for STATS / panels.
#[derive(Debug, Clone, Copy, Default)]
pub struct LagrangeCensus {
    /// Lagrange points on the system map.
    pub points: usize,
    /// Player hub stations anchored at L-points.
    pub hubs: usize,
    /// Pirate bases rooted at L-points.
    pub pirate_bases: usize,
    /// Merchant visits recorded at hub stations.
    pub merchant_visits: u64,
}

/// Census of points, hubs, pirate bases, and hub traffic.
#[must_use]
pub fn lagrange_census(world: &mut World) -> LagrangeCensus {
    let points = {
        let mut q = world.query::<&LagrangePoint>();
        q.iter(world).count()
    };
    let l_point_entities: Vec<Entity> = {
        let mut q = world.query::<(Entity, &LagrangePoint)>();
        q.iter(world).map(|(e, _)| e).collect()
    };
    let mut hubs = 0usize;
    let mut visits = 0u64;
    {
        let mut q = world.query::<(&HubTraffic, Option<&OrbitalStation>, Option<&Orbit>)>();
        for (hub, orbital, orbit) in q.iter(world) {
            let at_lagrange = orbital.is_some_and(|s| l_point_entities.contains(&s.parent_node))
                || orbit.is_some_and(|o| l_point_entities.contains(&o.parent));
            if at_lagrange {
                hubs += 1;
                visits += hub.merchant_visits;
            }
        }
    }
    let pirate_bases = {
        let mut q = world.query::<&PirateBase>();
        q.iter(world).count()
    };
    LagrangeCensus {
        points,
        hubs,
        pirate_bases,
        merchant_visits: visits,
    }
}

/// Compact STATS label, e.g. `2pts/1hubs/0pirates/3visits`. No spaces.
#[must_use]
pub fn lagrange_stats_label(world: &mut World) -> String {
    let census = lagrange_census(world);
    format!(
        "{}pts/{}hubs/{}pirates/{}visits",
        census.points, census.hubs, census.pirate_bases, census.merchant_visits
    )
}

/// Spawn an L-point on the system map: a [`SystemNode`] +
/// [`LagrangePoint`] + [`OrbitalBody`], co-orbiting with the first
/// orbiting body found (its planet), per [`lagrange_orbit`]. With no
/// orbiting body in the world it spawns without an `Orbit`.
pub fn spawn_lagrange_point(world: &mut World, slot: LagrangeSlot) -> Entity {
    let planet_orbit = {
        let mut q = world.query::<&Orbit>();
        q.iter(world).next().cloned()
    };
    let mut entity = world.spawn((
        SystemNode {
            node_type: NodeType::Station,
        },
        LagrangePoint { slot },
        OrbitalBody {
            name: format!("Lagrange Point {}", slot.label()),
            radius: 0.3,
            color: Color::Yellow,
            char: 'L',
        },
    ));
    if let Some(orbit) = planet_orbit {
        entity.insert(lagrange_orbit(&orbit, slot));
    }
    entity.id()
}

/// Spawn a player hub station anchored at `node`: one entity carrying
/// both upkeep legs ([`OrbitalStation`] and the real [`Station`] +
/// [`Orbit`]), a full-price [`StationFuelUpkeep`] (the upkeep systems
/// zero it when `node` is an L-point), and [`HubTraffic`].
pub fn spawn_hub_station(world: &mut World, node: Entity) -> Entity {
    world
        .spawn((
            Station {
                station_type: crate::layer2::fleet::StationType::Outpost,
            },
            OrbitalStation { parent_node: node },
            StationFuelUpkeep {
                base_cost: DEFAULT_STATION_UPKEEP,
                current_cost: DEFAULT_STATION_UPKEEP,
            },
            HubTraffic::default(),
            OrbitalBody {
                name: "Lagrange Hub".to_string(),
                radius: 0.5,
                color: Color::Cyan,
                char: 'H',
            },
            Orbit {
                parent: node,
                radius: 1.0,
                speed: 0.0,
                angle: 0.0,
            },
        ))
        .id()
}

/// Root a pirate base at `node` directly (headless force command and
/// the spawn system's spawn shape, in one place).
pub fn spawn_pirate_base_at(world: &mut World, node: Entity) -> Entity {
    world
        .spawn((
            PirateBase {
                strength: PIRATE_BASE_STRENGTH,
            },
            InOrbit { parent: node },
            OrbitalBody {
                name: "Pirate Anchorage".to_string(),
                radius: 0.5,
                color: Color::Red,
                char: 'P',
            },
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    fn pirate_app(tick: u64) -> App {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(SimulationTime {
            tick,
            speed: Default::default(),
        });
        app.insert_resource(LagrangeState::default());
        app.add_systems(
            Update,
            (
                pirate_l_point_spawn_system,
                lagrange_merchant_traffic_system,
            ),
        );
        app
    }

    #[test]
    fn test_lagrange_point_has_zero_upkeep() {
        // Spec RED test 1, adapted: SystemNode is the real spec-373
        // type; LagrangePoint / OrbitalStation / StationFuelUpkeep
        // are the spec types built by this module.
        let mut app = App::new();
        app.add_systems(Update, calculate_station_upkeep_system);

        let lagrange_node = app
            .world_mut()
            .spawn((
                SystemNode {
                    node_type: NodeType::Station,
                },
                LagrangePoint::default(),
            ))
            .id();

        let station_ent = app
            .world_mut()
            .spawn((
                OrbitalStation {
                    parent_node: lagrange_node,
                },
                StationFuelUpkeep {
                    base_cost: 10,
                    current_cost: 10,
                },
            ))
            .id();

        app.update();

        let upkeep = app.world().get::<StationFuelUpkeep>(station_ent).unwrap();
        assert_eq!(
            upkeep.current_cost, 0,
            "Stations at Lagrange points should have 0 upkeep"
        );
    }

    #[test]
    fn test_ordinary_orbit_station_pays_full_upkeep() {
        let mut app = App::new();
        app.add_systems(Update, calculate_station_upkeep_system);
        let plain_node = app
            .world_mut()
            .spawn(SystemNode {
                node_type: NodeType::Planet,
            })
            .id();
        let station_ent = app
            .world_mut()
            .spawn((
                OrbitalStation {
                    parent_node: plain_node,
                },
                StationFuelUpkeep {
                    base_cost: 10,
                    current_cost: 999,
                },
            ))
            .id();
        app.update();
        let upkeep = app.world().get::<StationFuelUpkeep>(station_ent).unwrap();
        assert_eq!(upkeep.current_cost, 10, "stale cost resets to base off L-points");
    }

    #[test]
    fn test_real_station_at_lagrange_has_zero_upkeep() {
        // The real-station leg: Station + Orbit, as build_station_system spawns.
        let mut app = App::new();
        app.add_systems(Update, calculate_real_station_upkeep_system);
        let lagrange_node = app
            .world_mut()
            .spawn(LagrangePoint {
                slot: LagrangeSlot::L4,
            })
            .id();
        let star = app.world_mut().spawn_empty().id();
        let station_ent = app
            .world_mut()
            .spawn((
                Station {
                    station_type: crate::layer2::fleet::StationType::Outpost,
                },
                Orbit {
                    parent: lagrange_node,
                    radius: 1.0,
                    speed: 0.0,
                    angle: 0.0,
                },
                StationFuelUpkeep {
                    base_cost: 10,
                    current_cost: 10,
                },
            ))
            .id();
        let _ = star;
        app.update();
        assert_eq!(
            app.world().get::<StationFuelUpkeep>(station_ent).unwrap().current_cost,
            0
        );
    }

    #[test]
    fn test_pirates_spawn_in_empty_lagrange_points() {
        // Spec RED test 2, adapted to the real types.
        let mut app = pirate_app(10_000);
        app.world_mut().spawn((
            SystemNode {
                node_type: NodeType::Station,
            },
            LagrangePoint::default(),
        ));
        app.update();
        let mut query = app.world_mut().query::<&PirateBase>();
        assert!(
            query.iter(app.world()).count() > 0,
            "Pirates should spawn in ignored L-Points"
        );
        // The base is attached to its node through the real InOrbit seam.
        let mut linked = app.world_mut().query::<&InOrbit>();
        assert_eq!(linked.iter(app.world()).count(), 1);
        assert_eq!(app.world().resource::<LagrangeState>().pirate_bases_spawned, 1);
    }

    #[test]
    fn test_pirates_do_not_spawn_in_occupied_points() {
        let mut app = pirate_app(10_000);
        let node = app
            .world_mut()
            .spawn(LagrangePoint::default())
            .id();
        app.world_mut().spawn((
            OrbitalStation { parent_node: node },
            StationFuelUpkeep {
                base_cost: 10,
                current_cost: 10,
            },
        ));
        app.update();
        let mut query = app.world_mut().query::<&PirateBase>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn test_pirates_do_not_duplicate_existing_base() {
        let mut app = pirate_app(10_000);
        let node = app.world_mut().spawn(LagrangePoint::default()).id();
        app.world_mut().spawn((
            PirateBase { strength: 50 },
            InOrbit { parent: node },
        ));
        app.update();
        let mut query = app.world_mut().query::<&PirateBase>();
        assert_eq!(query.iter(app.world()).count(), 1);
    }

    #[test]
    fn test_pirates_do_not_spawn_before_min_tick() {
        let mut app = pirate_app(100);
        app.world_mut().spawn(LagrangePoint::default());
        app.update();
        let mut query = app.world_mut().query::<&PirateBase>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn test_pirate_spawn_roll_is_deterministic() {
        // Spec §7: same (seed, tick, ordinal) always rolls the same.
        let a = deterministic_roll(DEFAULT_LAGRANGE_SEED, 10_000, 0);
        let b = deterministic_roll(DEFAULT_LAGRANGE_SEED, 10_000, 0);
        assert_eq!(a, b);
        assert!(a < PIRATE_SPAWN_CHANCE, "default seed roots a base at tick 10000");
        // Two identical worlds reach the identical outcome.
        let mut outcomes = Vec::new();
        for _ in 0..2 {
            let mut app = pirate_app(10_000);
            app.world_mut().spawn(LagrangePoint::default());
            app.update();
            let mut query = app.world_mut().query::<&PirateBase>();
            outcomes.push(query.iter(app.world()).count());
        }
        assert_eq!(outcomes[0], outcomes[1]);
        assert_eq!(outcomes[0], 1);
    }

    #[test]
    fn test_pirate_base_takes_root_chronicle() {
        let mut app = pirate_app(10_000);
        app.world_mut().spawn(LagrangePoint::default());
        app.update();
        let mut events = app
            .world_mut()
            .resource_mut::<Events<AddChronicleEvent>>();
        let texts: Vec<String> = events.drain().map(|e| e.text).collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Lagrange point"));
        assert!(texts[0].contains("pirate base"));
    }

    #[test]
    fn test_merchant_traffic_multiplier() {
        assert_eq!(
            merchant_visit_chance(true),
            BASE_MERCHANT_VISIT_CHANCE * LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER
        );
        assert_eq!(merchant_visit_chance(false), BASE_MERCHANT_VISIT_CHANCE);
        assert_eq!(LAGRANGE_MERCHANT_TRAFFIC_MULTIPLIER, 2.0);
    }

    #[test]
    fn test_lagrange_hub_attracts_merchant_visits() {
        // Tick 250 with the default seed rolls 0.471: under the
        // L-point chance (0.5) but over the ordinary chance (0.25) —
        // the same roll, priced by location.
        let mut l_app = pirate_app(TRAFFIC_CHECK_INTERVAL);
        let node = l_app.world_mut().spawn(LagrangePoint::default()).id();
        let hub = l_app
            .world_mut()
            .spawn((
                OrbitalStation { parent_node: node },
                HubTraffic::default(),
            ))
            .id();
        l_app.update();
        assert_eq!(l_app.world().get::<HubTraffic>(hub).unwrap().merchant_visits, 1);
        assert_eq!(l_app.world().resource::<LagrangeState>().merchant_visits, 1);

        let mut plain_app = pirate_app(TRAFFIC_CHECK_INTERVAL);
        let plain_node = plain_app.world_mut().spawn_empty().id();
        let plain_hub = plain_app
            .world_mut()
            .spawn((
                OrbitalStation {
                    parent_node: plain_node,
                },
                HubTraffic::default(),
            ))
            .id();
        plain_app.update();
        assert_eq!(
            plain_app.world().get::<HubTraffic>(plain_hub).unwrap().merchant_visits,
            0,
            "ordinary orbits draw half the traffic — this roll misses"
        );
    }

    #[test]
    fn test_lagrange_orbit_geometry() {
        let star = Entity::from_raw(1);
        let planet = Orbit {
            parent: star,
            radius: 100.0,
            speed: 0.01,
            angle: 0.5,
        };
        let l1 = lagrange_orbit(&planet, LagrangeSlot::L1);
        assert_eq!(l1.parent, star, "L-points co-orbit the star");
        assert!((l1.radius - 85.0).abs() < f32::EPSILON);
        assert!((l1.angle - 0.5).abs() < f32::EPSILON);
        assert!((l1.speed - 0.01).abs() < f32::EPSILON, "co-rotates with the planet");
        let l4 = lagrange_orbit(&planet, LagrangeSlot::L4);
        assert!((l4.radius - 100.0).abs() < f32::EPSILON);
        assert!((l4.angle - (0.5 + std::f32::consts::FRAC_PI_3)).abs() < 1e-6);
        let l3 = lagrange_orbit(&planet, LagrangeSlot::L3);
        assert!((l3.angle - (0.5 + std::f32::consts::PI)).abs() < 1e-6);
    }

    #[test]
    fn test_census_and_stats_label() {
        let mut world = World::new();
        assert_eq!(
            lagrange_stats_label(&mut world),
            "0pts/0hubs/0pirates/0visits"
        );
        let node = spawn_lagrange_point(&mut world, LagrangeSlot::L2);
        let hub = spawn_hub_station(&mut world, node);
        world.get_mut::<HubTraffic>(hub).unwrap().merchant_visits = 3;
        let node2 = spawn_lagrange_point(&mut world, LagrangeSlot::L5);
        spawn_pirate_base_at(&mut world, node2);
        let census = lagrange_census(&mut world);
        assert_eq!(census.points, 2);
        assert_eq!(census.hubs, 1);
        assert_eq!(census.pirate_bases, 1);
        assert_eq!(census.merchant_visits, 3);
        assert_eq!(
            lagrange_stats_label(&mut world),
            "2pts/1hubs/1pirates/3visits"
        );
        // The hub carries the full-price upkeep the systems will zero.
        assert_eq!(
            world.get::<StationFuelUpkeep>(hub).unwrap().current_cost,
            DEFAULT_STATION_UPKEEP
        );
    }

    #[test]
    fn test_ip_guard_no_borrowed_names() {
        const BANNED: [&str; 12] = [
            "star wars",
            "star trek",
            "deep space nine",
            "babylon 5",
            "mass effect",
            "citadel station",
            "the expanse",
            "battlestar",
            "firefly",
            "serenity",
            "elite dangerous",
            "eve online",
        ];
        let mut haystack = String::new();
        for slot in LagrangeSlot::ALL {
            haystack.push_str(slot.label());
            haystack.push(' ');
        }
        haystack.push_str("Lagrange Point Pirate Anchorage Lagrange Hub ");
        haystack.push_str("Pirates have taken root at an unguarded Lagrange point — a pirate base now nests in the gravity-neutral shallows, and it will not leave on its own. ");
        haystack.push_str("Merchant traffic is flocking to the Lagrange hub — free docking in the gravity-neutral shallows makes it the natural port of call.");
        let haystack = haystack.to_lowercase();
        for banned in BANNED {
            assert!(!haystack.contains(banned), "banned term present: {banned}");
        }
    }
}
