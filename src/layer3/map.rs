//! Galactic Map
//!
//! Structural representation of the interstellar map. It handles the nodes (star systems),
//! connections (hyperlanes), and positional tracking of fleets moving through the galaxy.

use bevy::prelude::DespawnRecursiveExt;
use bevy::utils::HashMap;
use bevy_ecs::prelude::*;
use bevy_time::Time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectorId(pub u32);

#[derive(Debug, Clone)]
pub struct SectorData {
    pub last_scanned: u64,
    pub accuracy: f32,
}

#[derive(Resource, Default)]
pub struct MapData {
    pub sectors: HashMap<SectorId, SectorData>,
}

impl MapData {
    pub fn new() -> Self {
        Self {
            sectors: HashMap::new(),
        }
    }

    pub fn insert_sector(&mut self, id: SectorId, data: SectorData) {
        self.sectors.insert(id, data);
    }

    pub fn get_sector(&self, id: SectorId) -> Option<&SectorData> {
        self.sectors.get(&id)
    }
}

pub fn map_data_rot_system(time: Option<Res<Time>>, mut map: Option<ResMut<MapData>>) {
    let delta = if let Some(time) = time {
        time.delta_secs()
    } else {
        return;
    };

    if let Some(map) = map.as_deref_mut() {
        for data in map.sectors.values_mut() {
            // Decay accuracy over time
            data.accuracy -= delta * 0.01;
            if data.accuracy < 0.0 {
                data.accuracy = 0.0;
            }
        }
    }
}

#[derive(Component)]
pub struct ScoutShip;

#[derive(Component)]
pub struct SectorPosition(pub SectorId);

pub fn scout_ship_scan_system(
    query: Query<&SectorPosition, With<ScoutShip>>,
    mut map: Option<ResMut<MapData>>,
) {
    if let Some(map) = map.as_deref_mut() {
        for pos in query.iter() {
            if let Some(data) = map.sectors.get_mut(&pos.0) {
                data.accuracy = 1.0;
            }
        }
    }
}

#[derive(Event, Debug)]
pub struct FleetArrivalEvent {
    pub fleet: Entity,
    pub sector: SectorId,
}

#[derive(Event, Debug)]
pub struct AnomalyDiscoveredEvent {
    pub sector: SectorId,
}

pub fn fleet_arrival_anomaly_system(
    mut events: EventReader<FleetArrivalEvent>,
    map: Option<Res<MapData>>,
    mut anomalies: EventWriter<AnomalyDiscoveredEvent>,
) {
    if let Some(map) = map.as_deref() {
        for event in events.read() {
            if let Some(data) = map.get_sector(event.sector) {
                if data.accuracy < 0.5 {
                    anomalies.send(AnomalyDiscoveredEvent {
                        sector: event.sector,
                    });
                }
            }
        }
    }
}

#[derive(Component, Default)]
pub struct StarSystem {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub drift_vx: f32,
    pub drift_vy: f32,
}

#[derive(Component)]
pub struct Hyperlane {
    pub start: Entity,
    pub end: Entity,
    pub stability: f32,
}

#[derive(Event)]
pub struct HyperlaneCollapseEvent {
    pub lane_entity: Entity,
}

#[derive(Event)]
pub struct TradeRouteSeveredEvent {
    pub system_a: Entity,
    pub system_b: Entity,
}

/// Spec 352: a new hyperlane formed because stellar drift brought two
/// previously unlinked systems within range. (The spec sketch names the
/// collapse counterpart `HyperlaneCollapsedEvent`; the real, pre-existing
/// type in this module is `HyperlaneCollapseEvent`.)
#[derive(Event, Debug)]
pub struct HyperlaneFormedEvent {
    pub system_a: Entity,
    pub system_b: Entity,
}

/// Spec 352: galaxy topology ledger and pathfinding-cache invalidation seam.
///
/// There is no layer-3 pathfinding cache in the codebase today (fleet
/// routing reads live `Hyperlane` components), so this resource is the
/// invalidation seam consumers poll: `revision` increments on EVERY lane
/// formation or collapse, and any future route cache can key off it.
/// `recalculate_trade_routes_system` remains the collapse-side consumer.
#[derive(Resource, Default, Debug)]
pub struct GalaxyTopology {
    pub revision: u64,
    pub lanes_formed: u64,
    pub lanes_collapsed: u64,
}

impl GalaxyTopology {
    pub fn note_formed(&mut self) {
        self.lanes_formed += 1;
        self.revision += 1;
    }
    pub fn note_collapsed(&mut self) {
        self.lanes_collapsed += 1;
        self.revision += 1;
    }
}

/// Counts of (star systems, hyperlanes) currently in the world — the
/// headless `galaxy` panel / STATS `galaxy=` field read through this.
pub fn galaxy_counts(world: &mut World) -> (usize, usize) {
    let systems = world.query::<&StarSystem>().iter(world).count();
    let lanes = world.query::<&Hyperlane>().iter(world).count();
    (systems, lanes)
}

/// Spec 352: seed the live galaxy. Before this, `StarSystem` was only ever
/// spawned inside unit tests, so the (already-registered) drift systems had
/// nothing to act on in a real game. Eight systems in a loose chain, each
/// with a small deterministic drift velocity (<= 0.003/tick per axis:
/// imperceptible day-to-day, tens of units over a long run — spec section 7),
/// linked to their near neighbours (10 lanes at the seed layout).
pub fn spawn_initial_galaxy(world: &mut World) {
    // (id, x, y, drift_vx, drift_vy)
    const SEED: [(u32, f32, f32, f32, f32); 8] = [
        (1, 0.0, 0.0, 0.002, 0.001),
        (2, 30.0, 10.0, -0.001, 0.002),
        (3, 60.0, -5.0, 0.003, -0.001),
        (4, 25.0, 45.0, -0.002, -0.002),
        (5, 70.0, 40.0, 0.001, -0.003),
        (6, 110.0, 15.0, -0.003, 0.001),
        (7, 45.0, 80.0, 0.002, 0.002),
        (8, 105.0, 75.0, -0.001, -0.001),
    ];
    let mut entities = Vec::new();
    for (id, x, y, drift_vx, drift_vy) in SEED {
        entities.push(
            world
                .spawn(StarSystem {
                    id,
                    x,
                    y,
                    drift_vx,
                    drift_vy,
                })
                .id(),
        );
    }
    // Link every pair already within lane range (same rule the formation
    // system applies after drift): the seed layout yields exactly 10 lanes.
    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let (a, b) = (SEED[i], SEED[j]);
            let dist_sq = (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2);
            if dist_sq <= MAX_HYPERLANE_DISTANCE.powi(2) {
                world.spawn(Hyperlane {
                    start: entities[i],
                    end: entities[j],
                    stability: 100.0,
                });
            }
        }
    }
}

pub fn trigger_hyperlane_collapse_system(
    query: Query<(Entity, &Hyperlane)>,
    mut collapse_events: EventWriter<HyperlaneCollapseEvent>,
) {
    for (entity, lane) in query.iter() {
        if lane.stability <= 0.0 {
            collapse_events.send(HyperlaneCollapseEvent {
                lane_entity: entity,
            });
        }
    }
}

pub fn process_hyperlane_collapse_system(
    mut commands: Commands,
    mut events: EventReader<HyperlaneCollapseEvent>,
    lane_query: Query<&Hyperlane>,
    mut severed_events: EventWriter<TradeRouteSeveredEvent>,
    mut topology: Option<ResMut<GalaxyTopology>>,
) {
    for ev in events.read() {
        if let Ok(lane) = lane_query.get(ev.lane_entity) {
            severed_events.send(TradeRouteSeveredEvent {
                system_a: lane.start,
                system_b: lane.end,
            });
            if let Some(topo) = topology.as_deref_mut() {
                topo.note_collapsed();
            }
            commands.entity(ev.lane_entity).despawn_recursive();
        }
    }
}

pub fn recalculate_trade_routes_system(mut events: EventReader<TradeRouteSeveredEvent>) {
    for _ev in events.read() {
        // Trigger a global recalculation of paths and trade networks
        // If a route cannot be reformed, emit a Starvation/Shortage event for affected colonies
    }
}

/// Maximum endpoint distance for a hyperlane. Verified against the real
/// map scale: seeded systems sit 30-50 units from their neighbours and the
/// spec's own snap test separates endpoints to 100, so the sketch value of
/// 50.0 is the live scale — kept unchanged.
pub const MAX_HYPERLANE_DISTANCE: f32 = 50.0;

/// Drift (and the lane maintenance/formation checks it drives) runs once
/// per 100 ticks, applying 100 ticks of velocity at once (spec sections 4/7).
pub const DRIFT_INTERVAL_TICKS: u64 = 100;

/// A drift epoch is due on every positive multiple of the interval.
/// Tick 0 is the spawn epoch and must not move anything.
#[allow(clippy::manual_is_multiple_of)]
pub fn drift_epoch_due(tick: u64) -> bool {
    tick > 0 && tick % DRIFT_INTERVAL_TICKS == 0
}

pub fn stellar_drift_system(
    time: Res<crate::shared::time::SimulationTime>,
    mut query: Query<&mut StarSystem>,
) {
    if !drift_epoch_due(time.tick) {
        return;
    }

    for mut sys in query.iter_mut() {
        sys.x += sys.drift_vx * 100.0;
        sys.y += sys.drift_vy * 100.0;
    }
}

pub fn hyperlane_maintenance_system(
    time: Option<Res<crate::shared::time::SimulationTime>>,
    mut events: EventWriter<HyperlaneCollapseEvent>,
    sys_query: Query<&StarSystem>,
    lane_query: Query<(Entity, &Hyperlane)>,
) {
    // Snap checks run on drift epochs (when a SimulationTime clock exists);
    // a bare test world without a clock runs the check unconditionally.
    if let Some(time) = time.as_deref() {
        if !drift_epoch_due(time.tick) {
            return;
        }
    }
    for (lane_entity, lane) in lane_query.iter() {
        if let (Ok(s1), Ok(s2)) = (sys_query.get(lane.start), sys_query.get(lane.end)) {
            let dist_sq = (s1.x - s2.x).powi(2) + (s1.y - s2.y).powi(2);
            if dist_sq > MAX_HYPERLANE_DISTANCE.powi(2) {
                // Better pattern: emit collapse event instead of despawning immediately
                events.send(HyperlaneCollapseEvent { lane_entity });
            }
        }
    }
}

pub fn hyperlane_formation_system(
    time: Option<Res<crate::shared::time::SimulationTime>>,
    mut commands: Commands,
    sys_query: Query<(Entity, &StarSystem)>,
    lane_query: Query<&Hyperlane>,
    mut formed_events: EventWriter<HyperlaneFormedEvent>,
    mut topology: Option<ResMut<GalaxyTopology>>,
) {
    // Formation is an O(N^2) sweep; like maintenance it runs on drift
    // epochs when a clock exists (bare test worlds run it directly).
    if let Some(time) = time.as_deref() {
        if !drift_epoch_due(time.tick) {
            return;
        }
    }
    let mut existing_lanes = std::collections::HashSet::new();
    for lane in lane_query.iter() {
        let min_ent = lane.start.min(lane.end);
        let max_ent = lane.start.max(lane.end);
        existing_lanes.insert((min_ent, max_ent));
    }

    let systems: Vec<(Entity, &StarSystem)> = sys_query.iter().collect();

    for i in 0..systems.len() {
        for j in (i + 1)..systems.len() {
            let (e1, s1) = systems[i];
            let (e2, s2) = systems[j];

            let dist_sq = (s1.x - s2.x).powi(2) + (s1.y - s2.y).powi(2);
            if dist_sq <= MAX_HYPERLANE_DISTANCE.powi(2) {
                let min_ent = e1.min(e2);
                let max_ent = e1.max(e2);

                if !existing_lanes.contains(&(min_ent, max_ent)) {
                    commands.spawn(Hyperlane {
                        start: min_ent,
                        end: max_ent,
                        stability: 100.0,
                    });
                    formed_events.send(HyperlaneFormedEvent {
                        system_a: min_ent,
                        system_b: max_ent,
                    });
                    if let Some(topo) = topology.as_deref_mut() {
                        topo.note_formed();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};
    use bevy_time::TimePlugin;
    use std::time::Duration;

    #[test]
    fn test_map_data_rots_over_time() {
        let mut app = App::new();
        app.add_plugins(TimePlugin);

        let mut map = MapData::new();
        map.insert_sector(
            SectorId(1),
            SectorData {
                last_scanned: 0,
                accuracy: 1.0,
            },
        );
        app.insert_resource(map);

        app.add_systems(Update, map_data_rot_system);

        // Initial tick to set up time
        app.update();

        // Advance time
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(Duration::from_secs(100));

        // Tick to run systems
        app.update();

        let map = app.world().resource::<MapData>();
        assert!(map.get_sector(SectorId(1)).unwrap().accuracy < 1.0);
    }

    #[test]
    fn test_scout_ship_refreshes_map_data() {
        let mut app = App::new();
        let mut map = MapData::new();
        map.insert_sector(
            SectorId(1),
            SectorData {
                last_scanned: 0,
                accuracy: 0.5,
            },
        );
        app.insert_resource(map);

        app.world_mut()
            .spawn((ScoutShip, SectorPosition(SectorId(1))));

        app.add_systems(Update, scout_ship_scan_system);
        app.update();

        let map = app.world().resource::<MapData>();
        assert_eq!(map.get_sector(SectorId(1)).unwrap().accuracy, 1.0);
    }

    #[test]
    fn test_fleet_arrival_on_inaccurate_map_triggers_anomaly() {
        let mut app = App::new();
        app.add_event::<FleetArrivalEvent>();
        app.add_event::<AnomalyDiscoveredEvent>();

        let mut map = MapData::new();
        map.insert_sector(
            SectorId(1),
            SectorData {
                last_scanned: 0,
                accuracy: 0.2,
            },
        );
        app.insert_resource(map);

        app.add_systems(Update, fleet_arrival_anomaly_system);

        app.world_mut()
            .resource_mut::<Events<FleetArrivalEvent>>()
            .send(FleetArrivalEvent {
                fleet: Entity::PLACEHOLDER,
                sector: SectorId(1),
            });

        app.update();

        let anomalies = app.world().resource::<Events<AnomalyDiscoveredEvent>>();
        let mut reader = anomalies.get_cursor();
        assert!(reader.read(anomalies).count() > 0);
    }

    fn setup_app() -> App {
        use crate::shared::time::SimulationTime;
        let mut app = App::new();
        app.insert_resource(SimulationTime::default());
        app.add_event::<HyperlaneCollapseEvent>();
        app.add_event::<TradeRouteSeveredEvent>();
        app.add_systems(
            Update,
            (
                trigger_hyperlane_collapse_system,
                process_hyperlane_collapse_system,
                recalculate_trade_routes_system,
            ),
        );
        app
    }

    #[test]
    fn test_hyperlane_collapse_severs_connection() {
        let mut app = setup_app();

        let sys_a = app
            .world_mut()
            .spawn(StarSystem {
                id: 1,
                ..Default::default()
            })
            .id();
        let sys_b = app
            .world_mut()
            .spawn(StarSystem {
                id: 2,
                ..Default::default()
            })
            .id();

        // Spawn a hyperlane connecting A and B
        let lane = app
            .world_mut()
            .spawn(Hyperlane {
                start: sys_a,
                end: sys_b,
                stability: 100.0,
            })
            .id();

        // Trigger a collapse event
        app.world_mut()
            .resource_mut::<Events<HyperlaneCollapseEvent>>()
            .send(HyperlaneCollapseEvent { lane_entity: lane });

        app.update();

        // The hyperlane should be despawned or marked as collapsed
        assert!(
            app.world().get::<Hyperlane>(lane).is_none(),
            "Hyperlane should be destroyed after a collapse"
        );
    }

    #[test]
    fn test_fleet_pathfinding_fails_when_lane_collapses() {
        let mut app = setup_app();

        let sys_a = app
            .world_mut()
            .spawn(StarSystem {
                id: 1,
                ..Default::default()
            })
            .id();
        let sys_b = app
            .world_mut()
            .spawn(StarSystem {
                id: 2,
                ..Default::default()
            })
            .id();

        let lane = app
            .world_mut()
            .spawn(Hyperlane {
                start: sys_a,
                end: sys_b,
                stability: 0.0, // trigger system requires stability <= 0.0
            })
            .id();

        app.update();

        // Need an extra update tick to process the despawn commands that were
        // queued during process_hyperlane_collapse_system.
        app.update();

        // After update, recalculate_trade_routes_system or similar should run
        assert!(app.world().get::<Hyperlane>(lane).is_none());
        let events = app.world().resource::<Events<TradeRouteSeveredEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).count() > 0);
    }

    #[test]
    fn test_systems_drift_over_time() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(stellar_drift_system);

        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 0,
            ..Default::default()
        });

        let sys1 = world
            .spawn(StarSystem {
                x: 10.0,
                y: 10.0,
                drift_vx: 0.1,
                drift_vy: -0.1,
                ..Default::default()
            })
            .id();
        let sys2 = world
            .spawn(StarSystem {
                x: 20.0,
                y: 20.0,
                drift_vx: -0.2,
                drift_vy: 0.0,
                ..Default::default()
            })
            .id();

        // Run simulation for 100 ticks
        world
            .resource_mut::<crate::shared::time::SimulationTime>()
            .tick = 100;
        schedule.run(&mut world);

        let s1 = world.get::<StarSystem>(sys1).unwrap();
        let s2 = world.get::<StarSystem>(sys2).unwrap();

        assert_eq!(s1.x, 20.0); // 10 + (0.1 * 100)
        assert_eq!(s1.y, 0.0); // 10 + (-0.1 * 100)
        assert_eq!(s2.x, 0.0); // 20 + (-0.2 * 100)
        assert_eq!(s2.y, 20.0); // 20 + (0.0 * 100)
    }

    #[test]
    fn test_hyperlanes_snap_when_distance_exceeds_max() {
        let mut world = World::new();
        world.init_resource::<Events<HyperlaneCollapseEvent>>();

        let mut schedule = Schedule::default();
        schedule.add_systems(hyperlane_maintenance_system);

        let sys1 = world
            .spawn(StarSystem {
                x: 0.0,
                y: 0.0,
                drift_vx: -1.0,
                drift_vy: 0.0,
                ..Default::default()
            })
            .id();
        let sys2 = world
            .spawn(StarSystem {
                x: 10.0,
                y: 0.0,
                drift_vx: 1.0,
                drift_vy: 0.0,
                ..Default::default()
            })
            .id();

        let _lane = world
            .spawn(Hyperlane {
                start: sys1,
                end: sys2,
                stability: 100.0,
            })
            .id();

        // Move them far apart manually to simulate drift
        world.get_mut::<StarSystem>(sys1).unwrap().x = -50.0;
        world.get_mut::<StarSystem>(sys2).unwrap().x = 50.0; // Distance = 100

        schedule.run(&mut world);

        // A HyperlaneCollapseEvent should be emitted
        let events = world.resource::<Events<HyperlaneCollapseEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).count() > 0);
    }

    #[test]
    fn test_new_hyperlanes_form_when_systems_drift_close() {
        let mut world = World::new();
        world.init_resource::<Events<HyperlaneFormedEvent>>();
        let mut schedule = Schedule::default();
        schedule.add_systems(hyperlane_formation_system);

        let _sys1 = world
            .spawn(StarSystem {
                x: 0.0,
                y: 0.0,
                drift_vx: 0.0,
                drift_vy: 0.0,
                ..Default::default()
            })
            .id();
        let _sys2 = world
            .spawn(StarSystem {
                x: 10.0,
                y: 0.0,
                drift_vx: 0.0,
                drift_vy: 0.0,
                ..Default::default()
            })
            .id();

        // Initially no lanes
        let lane_count = world.query::<&Hyperlane>().iter(&world).count();
        assert_eq!(lane_count, 0);

        schedule.run(&mut world);

        // They are close enough, a lane should form
        let lane_count = world.query::<&Hyperlane>().iter(&world).count();
        assert_eq!(lane_count, 1);
    }

    // --- Spec 352 (Stellar Drift) completion tests ---
    // Adaptations from the spec sketch: the real types live in
    // `crate::layer3::map` (not `crate::layer3::galaxy_map`), `StarSystem`
    // carries an extra `id` and uses `..Default::default()`, `Hyperlane`
    // uses `start`/`end` + `stability` (not `source`/`target`), the real
    // collapse event is `HyperlaneCollapseEvent` (not
    // `HyperlaneCollapsedEvent`), and `SimulationTime` is
    // `crate::shared::time::SimulationTime` (not `layer1::time`).

    #[test]
    fn test_no_drift_at_tick_zero() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(stellar_drift_system);
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 0,
            ..Default::default()
        });
        let sys = world
            .spawn(StarSystem {
                x: 10.0,
                y: 10.0,
                drift_vx: 0.1,
                drift_vy: -0.1,
                ..Default::default()
            })
            .id();
        schedule.run(&mut world);
        let s = world.get::<StarSystem>(sys).unwrap();
        assert_eq!(s.x, 10.0, "tick 0 is the spawn epoch: no drift yet");
        assert_eq!(s.y, 10.0);
    }

    #[test]
    fn test_maintenance_waits_for_drift_epoch() {
        let mut world = World::new();
        world.init_resource::<Events<HyperlaneCollapseEvent>>();
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 50,
            ..Default::default()
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(hyperlane_maintenance_system);
        let sys1 = world
            .spawn(StarSystem {
                x: -50.0,
                y: 0.0,
                ..Default::default()
            })
            .id();
        let sys2 = world
            .spawn(StarSystem {
                x: 50.0,
                y: 0.0,
                ..Default::default()
            })
            .id();
        world.spawn(Hyperlane {
            start: sys1,
            end: sys2,
            stability: 100.0,
        });
        schedule.run(&mut world);
        {
            let events = world.resource::<Events<HyperlaneCollapseEvent>>();
            let mut reader = events.get_cursor();
            assert_eq!(
                reader.read(events).count(),
                0,
                "no snap check between drift epochs"
            );
        }
        world
            .resource_mut::<crate::shared::time::SimulationTime>()
            .tick = 100;
        schedule.run(&mut world);
        let events = world.resource::<Events<HyperlaneCollapseEvent>>();
        let mut reader = events.get_cursor();
        assert_eq!(
            reader.read(events).count(),
            1,
            "snap check fires on the drift epoch"
        );
    }

    #[test]
    fn test_formation_emits_event_bumps_topology_and_never_duplicates() {
        let mut world = World::new();
        world.init_resource::<Events<HyperlaneFormedEvent>>();
        world.insert_resource(GalaxyTopology::default());
        let mut schedule = Schedule::default();
        schedule.add_systems(hyperlane_formation_system);
        world.spawn(StarSystem {
            x: 0.0,
            y: 0.0,
            ..Default::default()
        });
        world.spawn(StarSystem {
            x: 10.0,
            y: 0.0,
            ..Default::default()
        });
        schedule.run(&mut world);
        schedule.run(&mut world);
        let lane_count = world.query::<&Hyperlane>().iter(&world).count();
        assert_eq!(lane_count, 1, "second run must not duplicate the lane");
        let events = world.resource::<Events<HyperlaneFormedEvent>>();
        let mut reader = events.get_cursor();
        assert_eq!(reader.read(events).count(), 1);
        let topo = world.resource::<GalaxyTopology>();
        assert_eq!(topo.lanes_formed, 1);
        assert_eq!(topo.revision, 1);
    }

    #[test]
    fn test_collapse_processing_bumps_topology_revision() {
        let mut app = setup_app();
        app.insert_resource(GalaxyTopology::default());
        let sys_a = app
            .world_mut()
            .spawn(StarSystem {
                id: 1,
                ..Default::default()
            })
            .id();
        let sys_b = app
            .world_mut()
            .spawn(StarSystem {
                id: 2,
                ..Default::default()
            })
            .id();
        let lane = app
            .world_mut()
            .spawn(Hyperlane {
                start: sys_a,
                end: sys_b,
                stability: 100.0,
            })
            .id();
        app.world_mut()
            .resource_mut::<Events<HyperlaneCollapseEvent>>()
            .send(HyperlaneCollapseEvent { lane_entity: lane });
        app.update();
        assert!(app.world().get::<Hyperlane>(lane).is_none());
        let topo = app.world().resource::<GalaxyTopology>();
        assert_eq!(topo.lanes_collapsed, 1);
        assert_eq!(topo.revision, 1);
    }

    #[test]
    fn test_seed_galaxy_spawns_drifting_systems_and_lanes() {
        let mut world = World::new();
        spawn_initial_galaxy(&mut world);
        let (systems, lanes) = galaxy_counts(&mut world);
        assert_eq!(systems, 8);
        assert_eq!(
            lanes, 10,
            "seed layout links each system to its near neighbours"
        );
        let mut query = world.query::<&StarSystem>();
        for sys in query.iter(&world) {
            assert!(
                sys.drift_vx.abs() + sys.drift_vy.abs() > 0.0,
                "every seeded system drifts"
            );
            assert!(
                sys.drift_vx.abs() <= 0.01 && sys.drift_vy.abs() <= 0.01,
                "drift is imperceptible per-tick (spec section 7)"
            );
        }
    }
}
