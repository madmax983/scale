#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::drone::{
        drone_battery_system, evaluate_drone_actions_system, process_charge_system, Drone,
        DroneBattery, DroneHub,
    };
    use crate::layer1::energy::PowerConsumer;
    use crate::layer1::map::GridPosition;
    use crate::layer1::utility_ai::{ActionType, PopAction};
    use bevy_ecs::prelude::*;

    #[test]
    fn test_drone_component_initialization() {
        use crate::layer1::drone::DroneState;
        let mut world = World::new();
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        let battery = world.get::<DroneBattery>(drone).unwrap();
        assert!((battery.current - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_drone_seeks_charge_when_low() {
        use crate::layer1::drone::DroneState;
        let mut world = World::new();
        // Setup Drone with low battery
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 10.0,
                    max: 100.0,
                }, // 10%
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        // Setup Hub (Charger)
        let _hub = world
            .spawn((
                Building {
                    building_type: BuildingType::DroneHub,
                },
                DroneHub,
                GridPosition { x: 5, y: 5 },
                PowerConsumer {
                    demand: 10.0,
                    active: true,
                },
            ))
            .id();

        // Run evaluation
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_drone_actions_system);
        schedule.run(&mut world);

        let action = world.get::<PopAction>(drone).unwrap();
        assert_eq!(action.current, ActionType::Charge);
    }

    #[test]
    fn test_drone_idle_when_no_task() {
        use crate::layer1::drone::DroneState;
        let mut world = World::new();
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_drone_actions_system);
        schedule.run(&mut world);

        let action = world.get::<PopAction>(drone).unwrap();
        assert_eq!(action.current, ActionType::Idle);
    }

    #[test]
    fn test_drone_charges_at_hub() {
        use crate::layer1::drone::DroneState;
        let mut world = World::new();
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 10.0,
                    max: 100.0,
                },
                GridPosition { x: 5, y: 5 },
                PopAction {
                    current: ActionType::Charge,
                    ..Default::default()
                },
            ))
            .id();

        // Setup Hub
        world.spawn((
            Building {
                building_type: BuildingType::DroneHub,
            },
            DroneHub,
            GridPosition { x: 5, y: 5 },
            PowerConsumer {
                demand: 10.0,
                active: true,
            },
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(process_charge_system);
        schedule.run(&mut world);

        let battery = world.get::<DroneBattery>(drone).unwrap();
        assert!(battery.current > 10.0, "Battery should increase");
    }

    #[test]
    fn test_drone_battery_drains() {
        use crate::layer1::drone::DroneState;
        let mut world = World::new();
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                PopAction::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(drone_battery_system);
        schedule.run(&mut world);

        let battery = world.get::<DroneBattery>(drone).unwrap();
        assert!(battery.current < 100.0, "Battery should drain");
    }

    fn setup_app() -> bevy_app::App {
        use crate::layer1::drone::{
            check_drone_connection, process_feral_drones, DroneDisconnectedEvent,
        };
        use bevy_app::Update;
        let mut app = bevy_app::App::new();
        app.add_event::<DroneDisconnectedEvent>();
        app.add_systems(Update, (check_drone_connection, process_feral_drones));
        app
    }

    #[test]
    fn test_drone_becomes_feral_on_disconnect() {
        use crate::layer1::building::{Building, BuildingType};
        use crate::layer1::drone::Drone;
        use crate::layer1::drone::{ConnectedTo, DroneState, FeralDrone, GridConnection};
        use crate::layer1::energy::PowerConsumer;

        let mut app = setup_app();
        let command_center = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::CommandCenter,
                },
                PowerConsumer {
                    demand: 10.0,
                    active: true,
                },
            ))
            .id();

        let drone = app
            .world_mut()
            .spawn((
                Drone {
                    state: DroneState::Hauling,
                },
                ConnectedTo(command_center),
                GridConnection {
                    is_connected: true,
                    time_disconnected: 4999, // About to go feral
                },
            ))
            .id();

        // Act - Simulate power loss leading to disconnect
        app.world_mut()
            .entity_mut(command_center)
            .remove::<PowerConsumer>();
        app.update(); // check_drone_connection runs

        // Assert
        assert!(app.world().entity(drone).contains::<FeralDrone>());
        assert_eq!(
            app.world().get::<Drone>(drone).unwrap().state,
            DroneState::Feral
        );
    }

    #[test]
    fn test_drone_goes_feral_when_disconnected_from_grid() {
        use crate::layer1::drone::{ConnectedTo, Drone, GridConnection};
        use crate::shared::time::SimulationTime;
        let mut app = setup_app();
        app.init_resource::<SimulationTime>();

        let dummy_hub = app.world_mut().spawn_empty().id();

        let drone_entity = app
            .world_mut()
            .spawn((
                Drone::default(),
                ConnectedTo(dummy_hub),
                GridConnection {
                    is_connected: false,
                    time_disconnected: 0,
                },
            ))
            .id();

        // Act: Advance time beyond the feral threshold (e.g., 5000 ticks)
        let mut time = app.world_mut().resource_mut::<SimulationTime>();
        time.tick = 5001;

        let mut connection = app
            .world_mut()
            .get_mut::<GridConnection>(drone_entity)
            .unwrap();
        connection.time_disconnected = 5001;

        app.update(); // check_drone_connection runs

        // Assert
        let drone = app.world().entity(drone_entity);
        assert!(
            drone.contains::<crate::layer1::drone::FeralDrone>(),
            "Drone should become feral after prolonged disconnection"
        );
    }

    #[test]
    fn test_feral_drone_reproduces_when_hoard_is_full() {
        use crate::layer1::drone::{Drone, DroneState, FeralDrone};
        use crate::layer1::map::GridPosition;
        use crate::layer1::resources::{ResourceItem, ResourceType};

        let mut app = setup_app();
        let drone = app
            .world_mut()
            .spawn((
                Drone {
                    state: DroneState::Feral,
                },
                FeralDrone {
                    hoard: vec![
                        ResourceItem {
                            resource_type: ResourceType::Metal,
                            amount: 10.0,
                        };
                        5
                    ], // 5 items in hoard
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Act
        app.update(); // process_feral_drones runs

        // Assert
        let feral_drone = app.world().get::<FeralDrone>(drone).unwrap();
        assert!(feral_drone.hoard.is_empty(), "Hoard should be consumed");

        let mut query = app.world_mut().query::<&FeralDrone>();
        assert_eq!(
            query.iter(app.world()).count(),
            2,
            "A new Feral Drone should be spawned"
        );
    }

    #[test]
    fn test_feral_drone_hoards_resources() {
        use crate::layer1::drone::{Drone, DroneState, FeralDrone};
        use crate::layer1::map::GridPosition;
        use crate::layer1::resources::{ResourceItem, ResourceType};

        // Arrange
        let mut app = setup_app();
        let drone = app
            .world_mut()
            .spawn((
                Drone {
                    state: DroneState::Feral,
                },
                FeralDrone { hoard: vec![] },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let resource = app
            .world_mut()
            .spawn((
                ResourceItem {
                    resource_type: ResourceType::Metal,
                    amount: 10.0,
                },
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        // Act
        app.update(); // process_feral_drones runs

        // Assert - The drone picked up the nearby resource
        let feral_drone = app.world().get::<FeralDrone>(drone).unwrap();
        assert!(!feral_drone.hoard.is_empty());
        assert!(app.world().get_entity(resource).is_err()); // Resource removed from ground
    }

    #[test]
    fn test_feral_drone_attacks_nearby_pops() {
        use crate::layer1::drone::{Drone, DroneState, FeralDrone};
        use crate::layer1::health::Health;
        use crate::layer1::map::GridPosition;
        use crate::layer1::pop::Pop;

        // Arrange
        let mut app = setup_app();
        let _drone = app
            .world_mut()
            .spawn((
                Drone {
                    state: DroneState::Feral,
                },
                FeralDrone::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        // Act
        app.update();

        // Assert
        let health = app.world().get::<Health>(pop).unwrap();
        assert!(
            health.current < health.max,
            "Pop should have taken damage from Feral Drone"
        );
    }

    // ===== Spec 1385: Drone Exterior Work — repair dispatch & execution =====

    use crate::layer1::drone::{
        drone_repair_system, pick_repair_target, DroneState, GridConnection, RepairCandidate,
        DRONE_REPAIR_PER_TICK,
    };
    use crate::layer1::structure::Structure;

    fn damaged_building(world: &mut World, x: i32, y: i32, hp: f32, kind: BuildingType) -> Entity {
        world
            .spawn((
                Building {
                    building_type: kind,
                },
                GridPosition { x, y },
                Structure {
                    current_hp: hp,
                    max_hp: 100.0,
                },
            ))
            .id()
    }

    fn plain_drone(world: &mut World, x: i32, y: i32, battery: f32) -> Entity {
        world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: battery,
                    max: 100.0,
                },
                GridPosition { x, y },
                PopAction::default(),
            ))
            .id()
    }

    fn eval_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_drone_actions_system);
        schedule
    }

    #[test]
    fn test_pick_repair_target_prefers_exterior_over_worse_interior_damage() {
        let mut world = World::new();
        let interior = world.spawn_empty().id();
        let exterior = world.spawn_empty().id();
        let candidates = vec![
            RepairCandidate {
                entity: interior,
                pos: GridPosition { x: 1, y: 0 },
                damage_fraction: 0.9,
                exterior: false,
            },
            RepairCandidate {
                entity: exterior,
                pos: GridPosition { x: 9, y: 9 },
                damage_fraction: 0.5,
                exterior: true,
            },
        ];
        assert_eq!(
            pick_repair_target(&candidates, GridPosition { x: 0, y: 0 }),
            Some(exterior)
        );
    }

    #[test]
    fn test_pick_repair_target_picks_most_damaged_exterior_then_nearest() {
        let mut world = World::new();
        let far = world.spawn_empty().id();
        let near = world.spawn_empty().id();
        let worse = world.spawn_empty().id();
        let origin = GridPosition { x: 0, y: 0 };
        // Most damaged exterior wins even when farther.
        let candidates = vec![
            RepairCandidate {
                entity: near,
                pos: GridPosition { x: 1, y: 0 },
                damage_fraction: 0.5,
                exterior: true,
            },
            RepairCandidate {
                entity: worse,
                pos: GridPosition { x: 8, y: 0 },
                damage_fraction: 0.8,
                exterior: true,
            },
        ];
        assert_eq!(pick_repair_target(&candidates, origin), Some(worse));
        // Equal damage: nearest wins.
        let candidates = vec![
            RepairCandidate {
                entity: far,
                pos: GridPosition { x: 8, y: 0 },
                damage_fraction: 0.5,
                exterior: true,
            },
            RepairCandidate {
                entity: near,
                pos: GridPosition { x: 1, y: 0 },
                damage_fraction: 0.5,
                exterior: true,
            },
        ];
        assert_eq!(pick_repair_target(&candidates, origin), Some(near));
    }

    #[test]
    fn test_pick_repair_target_none_when_nothing_damaged() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let candidates = vec![RepairCandidate {
            entity,
            pos: GridPosition { x: 0, y: 0 },
            damage_fraction: 0.0,
            exterior: true,
        }];
        assert_eq!(
            pick_repair_target(&candidates, GridPosition { x: 0, y: 0 }),
            None
        );
        assert_eq!(pick_repair_target(&[], GridPosition { x: 0, y: 0 }), None);
    }

    #[test]
    fn test_drone_assigned_repair_for_damaged_building() {
        let mut world = World::new();
        let building = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        let drone = plain_drone(&mut world, 0, 0, 100.0);

        eval_schedule().run(&mut world);

        let action = world.get::<PopAction>(drone).unwrap();
        assert_eq!(action.current, ActionType::Repair);
        assert_eq!(
            world.get::<Drone>(drone).unwrap().state,
            DroneState::Repairing
        );
        let plan = world
            .get::<crate::layer1::utility_ai::StartPlan>(drone)
            .expect("repair assignment must insert a StartPlan");
        assert_eq!(plan.target, Some(building));
    }

    #[test]
    fn test_drone_repair_prefers_exterior_lifesupport_over_interior() {
        use crate::layer1::pressure::PressureGrid;
        let mut world = World::new();
        // Vacuum everywhere except the interior building's tile.
        let mut grid = PressureGrid::new(20, 20);
        grid.set(2, 0, 1.0);
        world.insert_resource(grid);
        let _interior = damaged_building(&mut world, 2, 0, 10.0, BuildingType::Housing);
        let exterior = damaged_building(&mut world, 6, 0, 50.0, BuildingType::LifeSupport);
        let drone = plain_drone(&mut world, 0, 0, 100.0);

        eval_schedule().run(&mut world);

        let plan = world
            .get::<crate::layer1::utility_ai::StartPlan>(drone)
            .expect("repair assignment must insert a StartPlan");
        assert_eq!(plan.target, Some(exterior));
    }

    #[test]
    fn test_low_battery_drone_charges_instead_of_repairing() {
        let mut world = World::new();
        let _building = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        world.spawn((
            Building {
                building_type: BuildingType::DroneHub,
            },
            DroneHub,
            GridPosition { x: 5, y: 5 },
            PowerConsumer {
                demand: 10.0,
                active: true,
            },
        ));
        let drone = plain_drone(&mut world, 0, 0, 10.0);

        eval_schedule().run(&mut world);

        assert_eq!(
            world.get::<PopAction>(drone).unwrap().current,
            ActionType::Charge
        );
    }

    #[test]
    fn test_feral_drone_is_never_assigned_repair() {
        use crate::layer1::drone::FeralDrone;
        let mut world = World::new();
        let _building = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Feral,
                },
                FeralDrone::default(),
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        eval_schedule().run(&mut world);

        assert_ne!(
            world.get::<PopAction>(drone).unwrap().current,
            ActionType::Repair
        );
        assert!(world
            .get::<crate::layer1::utility_ai::StartPlan>(drone)
            .is_none());
    }

    #[test]
    fn test_disconnected_drone_holds_instead_of_repairing() {
        let mut world = World::new();
        let _building = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                GridConnection {
                    is_connected: false,
                    time_disconnected: 10,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        eval_schedule().run(&mut world);

        assert_ne!(
            world.get::<PopAction>(drone).unwrap().current,
            ActionType::Repair
        );
    }

    #[test]
    fn test_drone_skips_deferred_and_ancient_structures() {
        use crate::layer1::structure::DeferMaintenance;
        let mut world = World::new();
        let deferred = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        world.entity_mut(deferred).insert(DeferMaintenance);
        let ancient = damaged_building(&mut world, 4, 0, 50.0, BuildingType::Housing);
        world
            .entity_mut(ancient)
            .insert(crate::layer1::heirloom::AncientStructure);
        let drone = plain_drone(&mut world, 0, 0, 100.0);

        eval_schedule().run(&mut world);

        assert_ne!(
            world.get::<PopAction>(drone).unwrap().current,
            ActionType::Repair
        );
    }

    #[test]
    fn test_drone_repair_restores_hp_and_completes_to_idle() {
        use crate::layer1::execution::components::{AtTarget, MovementTarget};
        let mut world = World::new();
        let building = damaged_building(&mut world, 3, 0, 50.0, BuildingType::Housing);
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Repairing,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 2, y: 0 },
                PopAction {
                    current: ActionType::Repair,
                    ..Default::default()
                },
                MovementTarget {
                    target_entity: building,
                    target_position: GridPosition { x: 3, y: 0 },
                    for_action: ActionType::Repair,
                },
                AtTarget,
            ))
            .id();

        let ticks = (50.0 / DRONE_REPAIR_PER_TICK) as usize;
        for _ in 0..ticks {
            drone_repair_system(&mut world);
        }

        let structure = world.get::<Structure>(building).unwrap();
        assert!((structure.current_hp - 100.0).abs() < f32::EPSILON);
        assert_eq!(
            world.get::<PopAction>(drone).unwrap().current,
            ActionType::Idle
        );
        assert_eq!(world.get::<Drone>(drone).unwrap().state, DroneState::Idle);
        assert!(world.get::<MovementTarget>(drone).is_none());
    }

    #[test]
    fn test_schedule_drone_travels_and_repairs() {
        use crate::layer1::erosion::ErosionGrid;
        use crate::layer1::execution::movement::{movement_system, process_start_plan_system};
        use crate::layer1::pop::Speed;

        let mut world = World::new();
        world.insert_resource(crate::layer1::generate_terrain(20, 20));
        {
            // Uniform walkable ground: this test is about the repair
            // pipeline, not pathfinding around random rock.
            let mut terrain = world.resource_mut::<crate::layer1::terrain::TerrainGrid>();
            for tile in &mut terrain.tiles {
                *tile = crate::layer1::terrain::TerrainType::Grass;
            }
        }
        world.insert_resource(ErosionGrid::new(20, 20));
        world.insert_resource(crate::layer1::building::OccupiedTiles::default());
        let building = damaged_building(&mut world, 4, 0, 50.0, BuildingType::Housing);
        let drone = world
            .spawn((
                Drone {
                    state: DroneState::Idle,
                },
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
                Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();
        let _ = drone;

        let mut schedule = Schedule::default();
        schedule.add_systems((
            evaluate_drone_actions_system,
            process_start_plan_system,
            movement_system,
            drone_repair_system,
        ));
        for _ in 0..40 {
            schedule.run(&mut world);
            let hp = world.get::<Structure>(building).unwrap().current_hp;
            if hp >= 100.0 {
                break;
            }
        }

        let structure = world.get::<Structure>(building).unwrap();
        assert!(
            structure.current_hp > 50.0,
            "drone should have travelled and repaired through the real movement pipeline, HP={}",
            structure.current_hp
        );
    }
}
