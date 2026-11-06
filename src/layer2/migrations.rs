//! Planetary Migrations (Spec 398).
//!
//! Biological space-faring entities — a great migrating herd of
//! massive, generally non-hostile creatures — cross the Layer 2
//! system on slow seasonal cycles, travelling node to node to
//! graze. They hold no malice and stop for nothing: when a herd
//! crosses an established shipping lane or orbital path, fleets in
//! the way take collision damage or lose transit time. The player
//! chooses (§5): reroute traffic around the herd (safe, but the
//! lanes run inefficiently for as long as the herd is present) or
//! cull the herd (effective, and the xeno-conservationist faction
//! does not forget it).
//!
//! ## Adaptations from the mock-style spec (§8 grounding)
//!
//! * **`layer2::movement::{Position, Velocity}` do not exist.**
//!   There is no `layer2::movement` module. The real Layer 2
//!   continuous bodies carry their own `Vec2` position/velocity
//!   (`layer2::weather::Storm`, `layer2::weather_fronts::
//!   WeatherFront`), ship physics uses `navigation::inertial::
//!   Velocity { value: Vec3 }` against `Transform`, and the real
//!   *discrete* system-map position is spec 373's
//!   [`SystemPosition`] (integer tiles, `step_toward` /
//!   `chebyshev_distance`) in `layer2::entities::nomad_fleet`.
//!   Herd locomotion follows the established 373 pattern —
//!   `SystemPosition` stepped one tile per tick toward a target —
//!   so herds and nomad fleets share one map. The spec sketch's
//!   `Velocity` is not built: velocity is implicit in the route
//!   (one tile per tick toward [`MigrationRoute::target`]).
//! * **`layer2::system::SystemNode` does not exist at that path.**
//!   The spec 373 types [`SystemNode`] / [`NodeType`] /
//!   [`NodeResources`] live in `layer2::entities::nomad_fleet`;
//!   `layer2::system` holds [`SystemBody`], `OrbitalBody`,
//!   `Orbit`, and spec 397's [`AsteroidBelt`]. Graze targets are
//!   chosen from real entities carrying a `SystemPosition` plus a
//!   live [`NodeResources`] or [`AsteroidBelt`] stock (richest
//!   first) — the same stocks the nomad lanes deplete. With no
//!   such node, the herd crosses the system centre (0, 0), the
//!   colony's shipping-lane crossing.
//! * **`layer2::fleet::{Fleet, FleetHealth}` ARE real** (spec
//!   099, `layer2::fleet`): the collision leg writes the real
//!   [`FleetHealth`] ledger, exactly as the spec's RED test
//!   intends — only its `Position` is adapted to the real
//!   [`SystemPosition`]. A fleet's position resolves three ways,
//!   in order: its own `SystemPosition` (the 373 map seam), the
//!   `SystemPosition` of the body it holds [`InOrbit`] of, or
//!   either endpoint of its [`InTransit`] lane. A herd member one
//!   tile from a transit lane's endpoint *disrupts* it instead
//!   (progress setback, no damage) — the spec's "logistical
//!   disruption" leg.
//! * **Spec 397's nomadic lane is the lifecycle template, not a
//!   parallel system.** This module reuses its shape — a director
//!   resource gating a scheduled arrival, a ledger resource for
//!   STATS, chronicle entries, [`FactionRelations`] keyed by
//!   `u32` faction id — and builds none of 397's mining/trade: the
//!   herd grazes (pauses) at nodes but takes no stock, so no
//!   resource is double-counted with the nomad lanes.
//! * **Seasonal cycle.** The year is 1000 ticks
//!   (`TICKS_PER_YEAR`, 250 per season). The herd returns every
//!   [`MIGRATION_PERIOD_TICKS`] (two years), gated by
//!   [`MigrationDirector`] cycle index (`tick / period`), so a
//!   tick skipped by a speed step cannot lose a migration and
//!   the director never double-fires within a cycle. Entry edge
//!   and graze lane are deterministic functions of the cycle.
//! * **§5 choice IS cheap on the real seams — so it is built.**
//!   Reroute: a [`MigrationPolicy`] resource; while `Rerouted`,
//!   collisions deal no damage (traffic steers around — the
//!   spec's Avoidance AI leg) and every tick with a herd present
//!   is counted in `reroute_ticks`, the inefficiency price,
//!   surfaced in the headless panel. Cull: [`CullHerdEvent`]
//!   despawns the herd and drops [`FactionRelations`] with the
//!   xeno-conservationist faction ([`CONSERVATIONIST_FACTION_ID`])
//!   by [`CULL_RELATIONS_PENALTY`], chronicled as Major.
//! * **One impact per crossing.** Collision damage is the spec
//!   GREEN formula (`mass * 0.01`) but a fleet can only be struck
//!   once per [`COLLISION_COOLDOWN_TICKS`] per the
//!   [`MigrationImpact`] stamp the collision system leaves on it
//!   — a herd *grazing* on a lane for 50 ticks is a blockade
//!   (disruption), not 50 hull breaches.
//! * **ZERO balance retunes.** No existing constant is changed;
//!   every constant below is new for spec 398. §7 rendering in
//!   the system map is left alone (headless-first, per the
//!   established pattern); the herd is visible via the
//!   `migration` panel and STATS.
//!
//! All names and strings are original and generic — no borrowed
//! IP (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer2::entities::nomad_fleet::{NodeResources, SystemPosition};
use crate::layer2::fleet::{Fleet, FleetHealth, InOrbit, InTransit};
use crate::layer2::system::AsteroidBelt;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants (new for spec 398; ZERO existing constants retuned).
// ---------------------------------------------------------------------------

/// Ticks between migration seasons: two full in-game years
/// (the year is [`TICKS_PER_YEAR`] = 1000). Slow by design —
/// migrations are seasons, not events.
pub const MIGRATION_PERIOD_TICKS: u64 = 2 * TICKS_PER_YEAR;
/// Creatures in one migrating herd.
pub const HERD_SIZE: usize = 5;
/// Mass of one creature (the spec GREEN example's 5000).
pub const DEFAULT_WHALE_MASS: f32 = 5000.0;
/// Collision damage per impact as a fraction of mass (the spec
/// GREEN example's `mass * 0.01` = 50 hull for a default creature).
pub const COLLISION_DAMAGE_FACTOR: f32 = 0.01;
/// Minimum ticks between two impacts on the same fleet.
pub const COLLISION_COOLDOWN_TICKS: u64 = 100;
/// Ticks a herd spends grazing once it reaches its target node.
pub const GRAZE_TICKS: u32 = 50;
/// Transit progress lost when a herd member crowds a lane
/// endpoint (the disruption leg; no hull damage).
pub const DISRUPTION_SETBACK: f32 = 0.05;
/// Layer-3 faction id of the xeno-conservationist faction in
/// [`FactionRelations`] (1–3 contract factions, 7 Homeworld, 13
/// pirate raiders, 21 marriage, 22 nomadic — 23 is unused).
pub const CONSERVATIONIST_FACTION_ID: u32 = 23;
/// Relations lost with the conservationists when a herd is culled.
pub const CULL_RELATIONS_PENALTY: i32 = -30;
/// System-map half-extent: herds enter at x = -12 and leave at
/// x = +12 (the 373 nomad entry edge convention).
pub const MAP_EDGE: i32 = 12;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A biological space-faring creature of the migrating herd
/// (spec component, name and shape kept; `hostility` rises only
/// if the herd is attacked — culling despawns rather than
/// fighting, so in this build it stays 0.0 and is recorded for
/// future combat seams).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct SpaceWhale {
    /// Bulk of the creature; collision damage scales with it.
    pub mass: f32,
    /// Aggression level (0.0 = the herd's natural state).
    pub hostility: f32,
}

impl Default for SpaceWhale {
    fn default() -> Self {
        Self {
            mass: DEFAULT_WHALE_MASS,
            hostility: 0.0,
        }
    }
}

/// Where a herd member is in its crossing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPhase {
    /// Travelling to the graze target.
    Crossing,
    /// Paused at the target, grazing.
    Grazing {
        /// Ticks of grazing left.
        ticks_left: u32,
    },
    /// Travelling to the exit edge; despawned on arrival.
    Leaving,
}

/// The route a herd member follows: graze `target`, then leave
/// via `exit`. Both are real system-map positions.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MigrationRoute {
    /// Graze node position (or the system centre lane crossing).
    pub target: SystemPosition,
    /// Exit-edge position.
    pub exit: SystemPosition,
    /// Current leg of the crossing.
    pub phase: MigrationPhase,
}

/// Stamp left on a fleet by its last herd impact; enforces one
/// impact per [`COLLISION_COOLDOWN_TICKS`] per fleet.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationImpact {
    /// Tick of the last impact.
    pub last_tick: u64,
}

// ---------------------------------------------------------------------------
// §5 choice state
// ---------------------------------------------------------------------------

/// How colony traffic handles a migrating herd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrafficPolicy {
    /// Lanes stay open: fleets in the herd's path take impacts.
    #[default]
    Normal,
    /// Traffic steers around the herd: no impact damage, but
    /// every herd tick costs reroute inefficiency.
    Rerouted,
}

/// The §5 policy resource (headless `migration reroute` /
/// `migration resume` flips it).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct MigrationPolicy {
    /// Current traffic policy.
    pub traffic: TrafficPolicy,
}

/// Event: cull the migrating herd (§5). The herd is removed and
/// the conservationist faction's relations drop.
#[derive(Event, Debug, Clone, Copy)]
pub struct CullHerdEvent;

// ---------------------------------------------------------------------------
// Director + ledger
// ---------------------------------------------------------------------------

/// Seasonal-arrival director: fires one herd per cycle.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct MigrationDirector {
    /// Last cycle index (`tick / MIGRATION_PERIOD_TICKS`) that
    /// spawned a herd.
    pub last_cycle: u64,
    /// Whether a herd is currently in the system.
    pub herd_active: bool,
    /// Whether this herd's first collision has been chronicled.
    pub collision_chronicled: bool,
}

/// Ledger of migration seasons (headless STATS / panel).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct MigrationLedger {
    /// Herds spawned (scheduled or debug).
    pub herds: u64,
    /// Creatures spawned.
    pub spawned: u64,
    /// Creatures that completed the crossing and left.
    pub departures: u64,
    /// Fleet impacts (cooldown-gated).
    pub collisions: u64,
    /// Lane-endpoint disruptions (no-damage setbacks).
    pub disruptions: u64,
    /// Total hull damage dealt by impacts.
    pub damage_dealt: f32,
    /// Herds culled (§5).
    pub culls: u64,
    /// Creatures culled.
    pub culled: u64,
    /// Ticks traffic ran rerouted around a live herd (§5 cost).
    pub reroute_ticks: u64,
    /// Impacts avoided by rerouting.
    pub rerouted_avoidances: u64,
}

/// Deterministic entry-row for a cycle: spreads successive
/// migrations across the map's rows without any RNG state.
#[must_use]
pub fn migration_entry_y(cycle: u64) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let y = ((cycle.wrapping_mul(7)) % 13) as i32;
    y - 6
}

/// Pick the graze target from live node stocks: the richest
/// entity carrying a `SystemPosition` plus a live
/// [`NodeResources`] or [`AsteroidBelt`] stock; falls back to
/// the system centre (0, 0), the shipping-lane crossing.
fn richest_graze_target(
    nodes: &Query<(
        &SystemPosition,
        Option<&NodeResources>,
        Option<&AsteroidBelt>,
    )>,
) -> SystemPosition {
    let mut best: Option<(SystemPosition, f32)> = None;
    for (pos, resources, belt) in nodes.iter() {
        let stock = resources.map_or(0.0, NodeResources::total)
            + belt.map_or(0.0, |b| b.resources_remaining);
        if stock > 0.0 && best.is_none_or(|(_, s)| stock > s) {
            best = Some((*pos, stock));
        }
    }
    best.map_or(SystemPosition { x: 0, y: 0 }, |(pos, _)| pos)
}

/// The herd bundle for one creature at `pos` on `route`.
fn whale_bundle(pos: SystemPosition, route: MigrationRoute) -> impl Bundle {
    (SpaceWhale::default(), pos, route)
}

/// Spawn a full herd into `world` entering at row `entry_y`:
/// the world-level path used by the headless debug command and
/// shared by tests. Counts the herd in the ledger and marks the
/// director active when those resources are present. The live
/// seasonal path is [`spawn_migration_herd_system`], which also
/// chronicles the arrival.
pub fn spawn_migration_herd(world: &mut World, entry_y: i32) -> Vec<Entity> {
    let target = {
        let mut query = world.query::<(
            &SystemPosition,
            Option<&NodeResources>,
            Option<&AsteroidBelt>,
        )>();
        let rows: Vec<(SystemPosition, Option<NodeResources>, Option<f32>)> = query
            .iter(world)
            .map(|(pos, res, belt)| {
                (*pos, res.copied(), belt.map(|b| b.resources_remaining))
            })
            .collect();
        let mut best: Option<(SystemPosition, f32)> = None;
        for (pos, res, belt_stock) in rows {
            let stock = res.map_or(0.0, |r| r.total()) + belt_stock.unwrap_or(0.0);
            if stock > 0.0 && best.is_none_or(|(_, s)| stock > s) {
                best = Some((pos, stock));
            }
        }
        best.map_or(SystemPosition { x: 0, y: 0 }, |(pos, _)| pos)
    };
    let exit = SystemPosition {
        x: MAP_EDGE,
        y: target.y,
    };
    let offsets = [0_i32, -1, 1, -2, 2];
    let mut entities = Vec::new();
    for (i, offset) in offsets.iter().take(HERD_SIZE).enumerate() {
        let _ = i;
        let pos = SystemPosition {
            x: -MAP_EDGE,
            y: entry_y + offset,
        };
        let route = MigrationRoute {
            target,
            exit,
            phase: MigrationPhase::Crossing,
        };
        entities.push(world.spawn(whale_bundle(pos, route)).id());
    }
    if let Some(mut ledger) = world.get_resource_mut::<MigrationLedger>() {
        ledger.herds += 1;
        ledger.spawned += entities.len() as u64;
    }
    if let Some(mut director) = world.get_resource_mut::<MigrationDirector>() {
        director.herd_active = true;
        director.collision_chronicled = false;
    }
    entities
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Seasonal arrival (spec system, adapted name — the sketch's
/// `spawn_migration_herd_system`): at the first tick of each
/// [`MIGRATION_PERIOD_TICKS`] cycle, exactly once per cycle, a
/// herd enters at the west edge and is chronicled.
pub fn spawn_migration_herd_system(
    time: Res<SimulationTime>,
    mut director: ResMut<MigrationDirector>,
    mut ledger: ResMut<MigrationLedger>,
    nodes: Query<(
        &SystemPosition,
        Option<&NodeResources>,
        Option<&AsteroidBelt>,
    )>,
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let cycle = time.tick / MIGRATION_PERIOD_TICKS;
    if cycle == 0 || cycle <= director.last_cycle {
        return;
    }
    director.last_cycle = cycle;
    director.herd_active = true;
    director.collision_chronicled = false;
    let target = richest_graze_target(&nodes);
    let entry_y = migration_entry_y(cycle);
    let exit = SystemPosition {
        x: MAP_EDGE,
        y: target.y,
    };
    for offset in [0_i32, -1, 1, -2, 2].into_iter().take(HERD_SIZE) {
        let pos = SystemPosition {
            x: -MAP_EDGE,
            y: entry_y + offset,
        };
        commands.spawn(whale_bundle(
            pos,
            MigrationRoute {
                target,
                exit,
                phase: MigrationPhase::Crossing,
            },
        ));
    }
    ledger.herds += 1;
    ledger.spawned += HERD_SIZE as u64;
    chronicle.send(AddChronicleEvent {
        text: "PLANETARY MIGRATION: A great herd of vast drifting creatures has entered the system, crossing the shipping lanes on its slow seasonal path. Reroute traffic around it, or cull the herd and answer to the xeno-conservationists.".to_string(),
        importance: EventImportance::Major,
    });
}

/// Movement (spec system, adapted name — the sketch's
/// `update_migration_movement_system`): each creature steps one
/// tile toward its current leg's destination (the 373
/// `step_toward` pattern). Reaching the target starts grazing;
/// grazing counts down into the Leaving leg; reaching the exit
/// despawns the creature and counts the departure. When the
/// last member of a herd leaves, the departure is chronicled.
pub fn update_migration_movement_system(
    mut commands: Commands,
    mut whales: Query<(Entity, &mut SystemPosition, &mut MigrationRoute), With<SpaceWhale>>,
    mut ledger: ResMut<MigrationLedger>,
    mut director: ResMut<MigrationDirector>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let total = whales.iter().count();
    let mut leaving_now = 0_u64;
    for (entity, mut pos, mut route) in whales.iter_mut() {
        match route.phase {
            MigrationPhase::Crossing => {
                if *pos == route.target {
                    route.phase = MigrationPhase::Grazing {
                        ticks_left: GRAZE_TICKS,
                    };
                } else {
                    *pos = pos.step_toward(route.target);
                    if *pos == route.target {
                        route.phase = MigrationPhase::Grazing {
                            ticks_left: GRAZE_TICKS,
                        };
                    }
                }
            }
            MigrationPhase::Grazing { ticks_left } => {
                if ticks_left <= 1 {
                    route.phase = MigrationPhase::Leaving;
                } else {
                    route.phase = MigrationPhase::Grazing {
                        ticks_left: ticks_left - 1,
                    };
                }
            }
            MigrationPhase::Leaving => {
                if *pos == route.exit {
                    commands.entity(entity).despawn();
                    leaving_now += 1;
                } else {
                    *pos = pos.step_toward(route.exit);
                    if *pos == route.exit {
                        commands.entity(entity).despawn();
                        leaving_now += 1;
                    }
                }
            }
        }
    }
    if leaving_now > 0 {
        ledger.departures += leaving_now;
        if director.herd_active && leaving_now as usize >= total {
            director.herd_active = false;
            chronicle.send(AddChronicleEvent {
                text: "PLANETARY MIGRATION: The great herd has crossed the system and moved on. The lanes are clear again.".to_string(),
                importance: EventImportance::Standard,
            });
        }
    }
}

/// Collisions & disruption (spec system, name kept): a herd
/// member sharing a tile with a fleet's resolved position deals
/// `mass * COLLISION_DAMAGE_FACTOR` hull damage to the real
/// [`FleetHealth`], once per [`COLLISION_COOLDOWN_TICKS`] per
/// fleet. One tile from a transit lane's endpoint, it disrupts
/// the transit instead (progress setback). Under a `Rerouted`
/// policy, impacts are avoided entirely and counted.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn check_whale_collisions_system(
    time: Res<SimulationTime>,
    whales: Query<(&SpaceWhale, &SystemPosition)>,
    mut fleets: Query<
        (
            Entity,
            &mut FleetHealth,
            Option<&SystemPosition>,
            Option<&InOrbit>,
            Option<&mut InTransit>,
            Option<&mut MigrationImpact>,
        ),
        With<Fleet>,
    >,
    positions: Query<&SystemPosition>,
    policy: Res<MigrationPolicy>,
    mut ledger: ResMut<MigrationLedger>,
    mut director: ResMut<MigrationDirector>,
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let herd: Vec<(f32, SystemPosition)> = whales
        .iter()
        .map(|(whale, pos)| (whale.mass, *pos))
        .collect();
    if herd.is_empty() {
        return;
    }
    let mut first_collision = false;
    for (entity, mut health, own_pos, orbit, transit, impact) in fleets.iter_mut() {
        // Resolve the fleet's lane positions (see module docs).
        let mut lane_points: Vec<SystemPosition> = Vec::new();
        if let Some(pos) = own_pos {
            lane_points.push(*pos);
        }
        if let Some(orbit) = orbit {
            if let Ok(pos) = positions.get(orbit.parent) {
                lane_points.push(*pos);
            }
        }
        let mut transit_points: Vec<SystemPosition> = Vec::new();
        if let Some(transit) = transit.as_deref() {
            if let Ok(pos) = positions.get(transit.origin) {
                transit_points.push(*pos);
            }
            if let Ok(pos) = positions.get(transit.destination) {
                transit_points.push(*pos);
            }
        }
        // Exact overlap: impact (or an avoided impact, rerouted).
        let hit_mass = herd
            .iter()
            .find(|(_, wpos)| {
                lane_points.contains(wpos) || transit_points.contains(wpos)
            })
            .map(|(mass, _)| *mass);
        if let Some(mass) = hit_mass {
            let cooled = impact
                .as_deref()
                .is_none_or(|stamp| {
                    time.tick.saturating_sub(stamp.last_tick) >= COLLISION_COOLDOWN_TICKS
                });
            if cooled {
                match impact {
                    Some(mut stamp) => stamp.last_tick = time.tick,
                    None => {
                        commands.entity(entity).insert(MigrationImpact {
                            last_tick: time.tick,
                        });
                    }
                }
                if policy.traffic == TrafficPolicy::Rerouted {
                    ledger.rerouted_avoidances += 1;
                } else {
                    let damage = mass * COLLISION_DAMAGE_FACTOR;
                    health.current = (health.current - damage).max(0.0);
                    ledger.collisions += 1;
                    ledger.damage_dealt += damage;
                    if !director.collision_chronicled {
                        first_collision = true;
                    }
                }
            }
            continue;
        }
        // Adjacent to a transit endpoint: disruption only.
        if policy.traffic == TrafficPolicy::Normal {
            if let Some(mut transit) = transit {
                let crowded = herd.iter().any(|(_, wpos)| {
                    transit_points
                        .iter()
                        .any(|point| wpos.chebyshev_distance(*point) == 1)
                });
                if crowded {
                    transit.progress = (transit.progress - DISRUPTION_SETBACK).max(0.0);
                    ledger.disruptions += 1;
                }
            }
        }
    }
    if first_collision {
        director.collision_chronicled = true;
        chronicle.send(AddChronicleEvent {
            text: "PLANETARY MIGRATION: A drifting creature of the great herd has collided with a fleet in its path. The herd does not stop, and it does not turn.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// §5 cull: despawn the whole herd, count it, and drop
/// conservationist relations by [`CULL_RELATIONS_PENALTY`].
/// A cull with no herd present is a no-op (no penalty).
pub fn cull_herd_system(
    mut events: EventReader<CullHerdEvent>,
    mut commands: Commands,
    whales: Query<Entity, With<SpaceWhale>>,
    mut relations: ResMut<FactionRelations>,
    mut ledger: ResMut<MigrationLedger>,
    mut director: ResMut<MigrationDirector>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        let count = whales.iter().count();
        if count == 0 {
            continue;
        }
        for entity in whales.iter() {
            commands.entity(entity).despawn();
        }
        let score = relations
            .scores
            .entry(CONSERVATIONIST_FACTION_ID)
            .or_insert(0);
        *score = (*score + CULL_RELATIONS_PENALTY).clamp(-100, 100);
        ledger.culls += 1;
        ledger.culled += count as u64;
        director.herd_active = false;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "PLANETARY MIGRATION: The great herd has been culled ({count} creatures). The lanes are clear — and the xeno-conservationist faction has recorded its outrage."
            ),
            importance: EventImportance::Major,
        });
    }
}

/// §5 reroute cost: while traffic is rerouted and a herd is in
/// the system, every tick is counted as inefficiency.
pub fn migration_reroute_system(
    policy: Res<MigrationPolicy>,
    whales: Query<(), With<SpaceWhale>>,
    mut ledger: ResMut<MigrationLedger>,
) {
    if policy.traffic == TrafficPolicy::Rerouted && !whales.is_empty() {
        ledger.reroute_ticks += 1;
    }
}

// ---------------------------------------------------------------------------
// World-level helpers (headless console + tests)
// ---------------------------------------------------------------------------

/// Spawn a lone traffic fleet for lane tests / headless
/// evidence: a real [`Fleet`] with real [`FleetHealth`] at a
/// system-map position (the 373 map seam).
pub fn spawn_migration_convoy(world: &mut World, pos: SystemPosition) -> Entity {
    world
        .spawn((
            Fleet,
            FleetHealth {
                current: 100.0,
                max: 100.0,
            },
            pos,
        ))
        .id()
}

/// All herd members in entity order, cloned for display:
/// (entity, whale, position, route).
#[must_use]
pub fn migration_rows(
    world: &mut World,
) -> Vec<(Entity, SpaceWhale, SystemPosition, MigrationRoute)> {
    let mut rows: Vec<(Entity, SpaceWhale, SystemPosition, MigrationRoute)> = world
        .query::<(Entity, &SpaceWhale, &SystemPosition, &MigrationRoute)>()
        .iter(world)
        .map(|(e, w, p, r)| (e, *w, *p, *r))
        .collect();
    rows.sort_by_key(|(e, _, _, _)| e.index());
    rows
}

/// Census for STATS: `herd<n>/hits<c>/dmg<d>`, or `none` when
/// no herd is present and nothing has ever happened.
#[must_use]
pub fn migration_stats_label(world: &mut World) -> String {
    let herd = migration_rows(world).len();
    let ledger = world.get_resource::<MigrationLedger>().copied().unwrap_or_default();
    if herd == 0
        && ledger.herds == 0
        && ledger.collisions == 0
        && ledger.culls == 0
    {
        "none".to_string()
    } else {
        format!(
            "herd{herd}/hits{}/dmg{:.0}",
            ledger.collisions, ledger.damage_dealt
        )
    }
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn migration_public_strings() -> Vec<String> {
    vec![
        "Planetary Migrations".to_string(),
        "the great herd".to_string(),
        "PLANETARY MIGRATION: A great herd of vast drifting creatures has entered the system, crossing the shipping lanes on its slow seasonal path. Reroute traffic around it, or cull the herd and answer to the xeno-conservationists.".to_string(),
        "PLANETARY MIGRATION: The great herd has crossed the system and moved on. The lanes are clear again.".to_string(),
        "PLANETARY MIGRATION: A drifting creature of the great herd has collided with a fleet in its path. The herd does not stop, and it does not turn.".to_string(),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::layer2::entities::nomad_fleet::{NodeType, SystemNode};

    fn setup_world(tick: u64) -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            speed: Default::default(),
        });
        world.init_resource::<MigrationDirector>();
        world.init_resource::<MigrationLedger>();
        world.init_resource::<MigrationPolicy>();
        world.init_resource::<FactionRelations>();
        world.init_resource::<Events<CullHerdEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn collision_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(check_whale_collisions_system);
        schedule
    }

    // Spec RED test 1, adapted (Position/Velocity mocks -> the
    // real SystemPosition map seam; spawn is the seasonal
    // director, not an every-tick spawner): no herd before the
    // first migration tick, a full herd at it, the director
    // never double-fires within the cycle, and the herd moves.
    #[test]
    fn test_space_whale_spawns_and_moves() {
        let mut world = setup_world(MIGRATION_PERIOD_TICKS - 1);
        let mut schedule = Schedule::default();
        schedule.add_systems(spawn_migration_herd_system);
        schedule.run(&mut world);
        assert_eq!(migration_rows(&mut world).len(), 0);

        world.resource_mut::<SimulationTime>().tick = MIGRATION_PERIOD_TICKS;
        schedule.run(&mut world);
        let rows = migration_rows(&mut world);
        assert_eq!(rows.len(), HERD_SIZE, "a full herd should spawn");
        assert!(rows.iter().all(|(_, whale, _, _)| whale.mass > 0.0));
        assert!(rows.iter().all(|(_, whale, _, _)| whale.hostility == 0.0));

        schedule.run(&mut world);
        assert_eq!(migration_rows(&mut world).len(), HERD_SIZE, "director fires once per cycle");

        // Movement: the lead creature steps toward its target.
        let before = rows[0].2;
        let target = rows[0].3.target;
        let dist_before = before.chebyshev_distance(target);
        let mut movement = Schedule::default();
        movement.add_systems(update_migration_movement_system);
        movement.run(&mut world);
        let after = migration_rows(&mut world)[0].2;
        assert_ne!(before, after, "whale should be moving");
        assert_eq!(after.chebyshev_distance(target), dist_before - 1);
    }

    // Graze target: a live asteroid node is chosen over the
    // centre fallback (the spec §5 REFACTOR pathing leg).
    #[test]
    fn test_herd_targets_richest_node() {
        let mut world = setup_world(MIGRATION_PERIOD_TICKS);
        world.spawn((
            SystemNode {
                node_type: NodeType::Asteroid,
            },
            SystemPosition { x: 3, y: 3 },
            NodeResources {
                metal: 100.0,
                volatiles: 50.0,
            },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(spawn_migration_herd_system);
        schedule.run(&mut world);
        let rows = migration_rows(&mut world);
        assert_eq!(rows[0].3.target, SystemPosition { x: 3, y: 3 });
    }

    // Spec RED test 2, adapted (fleet Position mock -> real
    // SystemPosition; damage is the spec formula mass * 0.01):
    // a 1000-mass creature on a fleet's tile takes its real
    // FleetHealth from 100 to 90 in one impact — and the
    // cooldown stamp prevents a second impact the same crossing.
    #[test]
    fn test_whale_collision_with_fleet() {
        let mut world = setup_world(0);
        world.spawn((
            SpaceWhale {
                mass: 1000.0,
                hostility: 0.0,
            },
            SystemPosition { x: 10, y: 10 },
        ));
        let fleet = world
            .spawn((
                Fleet,
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
                SystemPosition { x: 10, y: 10 },
            ))
            .id();

        let mut schedule = collision_schedule();
        schedule.run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 90.0);

        // Same tick / within cooldown: no second impact.
        schedule.run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 90.0);

        // After the cooldown, a still-overlapping herd hits again.
        world.resource_mut::<SimulationTime>().tick = COLLISION_COOLDOWN_TICKS;
        schedule.run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 80.0);
        assert_eq!(world.resource::<MigrationLedger>().collisions, 2);
    }

    // Collision through the orbit seam: a fleet holding InOrbit
    // of a node is struck when the herd crosses that node.
    #[test]
    fn test_collision_via_in_orbit_parent() {
        let mut world = setup_world(0);
        let node = world.spawn(SystemPosition { x: 4, y: 4 }).id();
        let fleet = world
            .spawn((
                Fleet,
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
                InOrbit { parent: node },
            ))
            .id();
        world.spawn((
            SpaceWhale {
                mass: 1000.0,
                hostility: 0.0,
            },
            SystemPosition { x: 4, y: 4 },
        ));
        collision_schedule().run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 90.0);
    }

    // Disruption leg: a herd member one tile from a transit
    // lane's endpoint sets the transit back without damage.
    #[test]
    fn test_disruption_slows_transit_without_damage() {
        let mut world = setup_world(0);
        let origin = world.spawn(SystemPosition { x: 0, y: 0 }).id();
        let dest = world.spawn(SystemPosition { x: 5, y: 0 }).id();
        let fleet = world
            .spawn((
                Fleet,
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
                InTransit {
                    origin,
                    destination: dest,
                    progress: 0.5,
                    duration: 100.0,
                },
            ))
            .id();
        world.spawn((
            SpaceWhale::default(),
            SystemPosition { x: 1, y: 0 },
        ));
        collision_schedule().run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 100.0);
        let transit = world.get::<InTransit>(fleet).unwrap();
        assert!((transit.progress - (0.5 - DISRUPTION_SETBACK)).abs() < 1e-6);
        assert_eq!(world.resource::<MigrationLedger>().disruptions, 1);
    }

    // Lifecycle: graze counts down into Leaving; reaching the
    // exit despawns the creature and counts the departure.
    #[test]
    fn test_graze_then_leave_and_despawn() {
        let mut world = setup_world(0);
        let whale = world
            .spawn((
                SpaceWhale::default(),
                SystemPosition { x: 0, y: 0 },
                MigrationRoute {
                    target: SystemPosition { x: 0, y: 0 },
                    exit: SystemPosition { x: 1, y: 0 },
                    phase: MigrationPhase::Grazing { ticks_left: 1 },
                },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(update_migration_movement_system);
        schedule.run(&mut world);
        assert_eq!(
            world.get::<MigrationRoute>(whale).unwrap().phase,
            MigrationPhase::Leaving
        );
        schedule.run(&mut world);
        assert!(world.get::<SpaceWhale>(whale).is_none(), "creature despawns at the exit");
        assert_eq!(world.resource::<MigrationLedger>().departures, 1);
    }

    // §5 reroute: no damage while rerouted, avoidances counted,
    // and every rerouted herd tick is charged as inefficiency.
    #[test]
    fn test_reroute_avoids_damage_but_costs_ticks() {
        let mut world = setup_world(0);
        world.resource_mut::<MigrationPolicy>().traffic = TrafficPolicy::Rerouted;
        world.spawn((
            SpaceWhale {
                mass: 1000.0,
                hostility: 0.0,
            },
            SystemPosition { x: 2, y: 2 },
        ));
        let fleet = world
            .spawn((
                Fleet,
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
                SystemPosition { x: 2, y: 2 },
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems((check_whale_collisions_system, migration_reroute_system).chain());
        schedule.run(&mut world);
        assert_eq!(world.get::<FleetHealth>(fleet).unwrap().current, 100.0);
        assert_eq!(world.resource::<MigrationLedger>().rerouted_avoidances, 1);
        assert_eq!(world.resource::<MigrationLedger>().reroute_ticks, 1);
        assert_eq!(world.resource::<MigrationLedger>().collisions, 0);
    }

    // §5 cull: the herd is removed and conservationist relations
    // drop by the penalty. A second cull with no herd is a no-op.
    #[test]
    fn test_cull_herd_diplomacy_cost() {
        let mut world = setup_world(0);
        let spawned = spawn_migration_herd(&mut world, 0);
        assert_eq!(spawned.len(), HERD_SIZE);
        world.send_event(CullHerdEvent);
        let mut schedule = Schedule::default();
        schedule.add_systems(cull_herd_system);
        schedule.run(&mut world);
        assert_eq!(migration_rows(&mut world).len(), 0);
        assert_eq!(
            world.resource::<FactionRelations>().scores[&CONSERVATIONIST_FACTION_ID],
            CULL_RELATIONS_PENALTY
        );
        assert_eq!(world.resource::<MigrationLedger>().culls, 1);
        assert_eq!(world.resource::<MigrationLedger>().culled, HERD_SIZE as u64);

        world.send_event(CullHerdEvent);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<FactionRelations>().scores[&CONSERVATIONIST_FACTION_ID],
            CULL_RELATIONS_PENALTY,
            "empty cull must not double-charge diplomacy"
        );
    }

    // The cycle after next spawns a fresh herd (recurring
    // seasons, not a one-off event).
    #[test]
    fn test_second_cycle_spawns_again() {
        let mut world = setup_world(MIGRATION_PERIOD_TICKS);
        let mut schedule = Schedule::default();
        schedule.add_systems(spawn_migration_herd_system);
        schedule.run(&mut world);
        assert_eq!(migration_rows(&mut world).len(), HERD_SIZE);
        world.resource_mut::<SimulationTime>().tick = MIGRATION_PERIOD_TICKS * 2;
        schedule.run(&mut world);
        assert_eq!(migration_rows(&mut world).len(), HERD_SIZE * 2);
        assert_eq!(world.resource::<MigrationLedger>().herds, 2);
    }

    #[test]
    fn test_stats_label() {
        let mut world = setup_world(0);
        assert_eq!(migration_stats_label(&mut world), "none");
        let _herd = spawn_migration_herd(&mut world, 0);
        assert_eq!(migration_stats_label(&mut world), "herd5/hits0/dmg0");
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        let banned = [
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "sandworm",
            "bene gesserit",
            "hitchhiker",
            "vogon",
            "fallen empire",
            "stellaris",
            "moby",
            "monstro",
        ];
        for text in migration_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string contains banned term {term:?}: {text}"
                );
            }
        }
    }
}
