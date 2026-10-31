//! Spec 379 RED tests, adapted to the real tree.
//!
//! The spec sketched `layer2::logistics::launch::{TransferOrbit,
//! LaunchCostCalculator, InWindow, OutOfWindow}` against a
//! `layer2::celestial::Orbit { angle, radius }`. Neither sketch type
//! exists. Adaptations:
//! * `Orbit` is the real `layer2::system::Orbit { parent, radius,
//!   speed, angle }` (spec 094) — tests construct it with all fields.
//! * The module path `layer2::logistics::launch` is real (this file's
//!   parent module is new; `layer2::ship::logistics` is the spec-117
//!   DeltaV seam and stays untouched).
//! * Fuel gating burns the real colony pool, `ColonyResources.fuel`
//!   (the spec-117 stockpile), and a successful launch reuses the
//!   spec-099 fleet seam (`InOrbit` -> `InTransit`).

#[cfg(test)]
mod tests {
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer2::fleet::{fleet_movement_system, Fleet, InOrbit, InTransit};
    use crate::layer2::logistics::launch::{
        advance_orbits_system, launch_command_system, launch_stats_label,
        transfer_completion_system, HohmannTransfer, InWindow, LaunchCommand, LaunchCost,
        LaunchCostCalculator, LaunchLedger, LaunchRefusedEvent, LaunchedEvent, OutOfWindow,
        TransferOrbit,
    };
    use crate::layer2::system::Orbit;
    use bevy_ecs::prelude::*;
    use bevy_ecs::schedule::IntoSystemConfigs;
    use bevy_ecs::system::RunSystemOnce;
    use std::f32::consts::{PI, TAU};

    fn spawn_body(
        world: &mut World,
        parent: Entity,
        angle: f32,
        radius: f32,
        speed: f32,
    ) -> Entity {
        world
            .spawn(Orbit {
                parent,
                radius,
                speed,
                angle,
            })
            .id()
    }

    // --- Spec test 1 (adapted): in window is cheap. ---
    #[test]
    fn test_launch_in_window_is_cheap() {
        let mut world = World::new();
        let star = world.spawn_empty().id();

        // Planet 1 is at 0 degrees, Planet 2 is at 45 degrees.
        let origin = spawn_body(&mut world, star, 0.0, 100.0, 0.0);
        let destination = spawn_body(&mut world, star, PI / 4.0, 200.0, 0.0);

        let cost = world
            .run_system_once(move |q_orbits: Query<&Orbit>| {
                LaunchCostCalculator::calculate(origin, destination, &q_orbits)
            })
            .unwrap();

        // Should be cheap.
        assert!(matches!(cost, Some(InWindow(fuel)) if fuel == 100));
    }

    // --- Spec test 2 (adapted): out of window is expensive. ---
    #[test]
    fn test_launch_out_of_window_is_expensive() {
        let mut world = World::new();
        let star = world.spawn_empty().id();

        // Planet 1 is at 0 degrees, Planet 2 is at 180 degrees.
        let origin = spawn_body(&mut world, star, 0.0, 100.0, 0.0);
        let destination = spawn_body(&mut world, star, PI, 200.0, 0.0);

        let cost = world
            .run_system_once(move |q_orbits: Query<&Orbit>| {
                LaunchCostCalculator::calculate(origin, destination, &q_orbits)
            })
            .unwrap();

        // Should be 10x more expensive.
        assert!(matches!(cost, Some(OutOfWindow(fuel)) if fuel == 1000));
    }

    #[test]
    fn test_window_wraps_around_full_circle() {
        // 350 degrees vs 10 degrees: the raw difference is 340 degrees,
        // but the bodies are really 20 degrees apart — in window.
        let mut world = World::new();
        let star = world.spawn_empty().id();
        let origin = spawn_body(&mut world, star, 350.0_f32.to_radians(), 100.0, 0.0);
        let destination = spawn_body(&mut world, star, 10.0_f32.to_radians(), 200.0, 0.0);

        let cost = world
            .run_system_once(move |q_orbits: Query<&Orbit>| {
                LaunchCostCalculator::calculate(origin, destination, &q_orbits)
            })
            .unwrap();

        assert!(matches!(cost, Some(InWindow(100))));
    }

    #[test]
    fn test_calculator_missing_orbit_returns_none() {
        let mut world = World::new();
        let star = world.spawn_empty().id();
        let origin = spawn_body(&mut world, star, 0.0, 100.0, 0.0);
        let destination = world.spawn_empty().id(); // no Orbit component

        let cost = world
            .run_system_once(move |q_orbits: Query<&Orbit>| {
                LaunchCostCalculator::calculate(origin, destination, &q_orbits)
            })
            .unwrap();

        assert!(cost.is_none());
    }

    #[test]
    fn test_launch_cost_accessors() {
        assert_eq!(LaunchCost::InWindow(100).fuel(), 100);
        assert_eq!(LaunchCost::OutOfWindow(1000).fuel(), 1000);
        assert!(LaunchCost::InWindow(100).is_in_window());
        assert!(!LaunchCost::OutOfWindow(1000).is_in_window());
    }

    // --- Predictor ---

    fn orbit(angle: f32, radius: f32, speed: f32) -> Orbit {
        Orbit {
            parent: Entity::PLACEHOLDER,
            radius,
            speed,
            angle,
        }
    }

    #[test]
    fn test_predictor_zero_when_already_in_window() {
        let origin = orbit(0.0, 100.0, 0.0);
        let destination = orbit(PI / 6.0, 200.0, 0.5);
        assert_eq!(
            crate::layer2::logistics::launch::next_window_start(&origin, &destination),
            Some(0)
        );
    }

    #[test]
    fn test_predictor_finds_next_window() {
        // Destination creeps from 180 degrees toward the origin at
        // 0.1 rad/tick: the gap closes to pi/4 at tick 24
        // (pi - 0.1*24 = 0.7416 <= pi/4; at tick 23 it is 0.8416 > pi/4).
        let origin = orbit(0.0, 100.0, 0.0);
        let destination = orbit(PI, 200.0, -0.1);
        assert_eq!(
            crate::layer2::logistics::launch::next_window_start(&origin, &destination),
            Some(24)
        );
    }

    #[test]
    fn test_predictor_never_for_locked_pair() {
        // Same angular speed: the 180-degree gap never closes.
        let origin = orbit(0.0, 100.0, 0.05);
        let destination = orbit(PI, 200.0, 0.05);
        assert_eq!(
            crate::layer2::logistics::launch::next_window_start(&origin, &destination),
            None
        );
    }

    #[test]
    fn test_predictor_upcoming_windows_are_entry_edges() {
        let origin = orbit(0.0, 100.0, 0.0);
        let destination = orbit(PI, 200.0, -0.1);
        let windows = crate::layer2::logistics::launch::upcoming_windows(&origin, &destination, 3);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0], 24);
        assert!(windows[1] > windows[0]);
        assert!(windows[2] > windows[1]);
        // Each reported start really is in window, and the tick before
        // it is not (entry edge, not a point inside the window).
        for start in windows {
            assert!(crate::layer2::logistics::launch::in_window_at(
                &origin,
                &destination,
                start
            ));
            assert!(!crate::layer2::logistics::launch::in_window_at(
                &origin,
                &destination,
                start - 1
            ));
        }
    }

    // --- Orbit drift (spec Technical Guidance: positions track the tick) ---

    #[test]
    fn test_orbits_advance_with_tick_and_wrap() {
        let mut world = World::new();
        let star = world.spawn_empty().id();
        let body = spawn_body(&mut world, star, TAU - 0.1, 100.0, 0.5);

        let mut schedule = Schedule::default();
        schedule.add_systems(advance_orbits_system);
        schedule.run(&mut world);

        let angle = world.get::<Orbit>(body).unwrap().angle;
        assert!((angle - 0.4).abs() < 1e-5, "angle wrapped to {angle}");
    }

    // --- Fuel-gated launch (spec Technical Guidance) ---

    fn launch_world(
        fuel: f32,
        origin_angle: f32,
        dest_angle: f32,
    ) -> (World, Schedule, Entity, Entity, Entity) {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            fuel,
            ..ColonyResources::default()
        });
        world.insert_resource(LaunchLedger::default());
        world.init_resource::<Events<LaunchCommand>>();
        world.init_resource::<Events<LaunchRefusedEvent>>();
        world.init_resource::<Events<LaunchedEvent>>();

        let star = world.spawn_empty().id();
        let origin = spawn_body(&mut world, star, origin_angle, 100.0, 0.0);
        let destination = spawn_body(&mut world, star, dest_angle, 200.0, 0.0);
        let fleet = world.spawn((Fleet, InOrbit { parent: origin })).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(launch_command_system);
        (world, schedule, fleet, origin, destination)
    }

    fn send_launch(world: &mut World, fleet: Entity, destination: Entity) {
        world
            .resource_mut::<Events<LaunchCommand>>()
            .send(LaunchCommand { fleet, destination });
    }

    #[test]
    fn test_launch_in_window_deducts_baseline_fuel() {
        let (mut world, mut schedule, fleet, _origin, destination) =
            launch_world(500.0, 0.0, PI / 4.0);
        send_launch(&mut world, fleet, destination);
        schedule.run(&mut world);

        assert_eq!(world.resource::<ColonyResources>().fuel, 400.0);
        assert!(world.get::<InOrbit>(fleet).is_none());
        let transit = world.get::<InTransit>(fleet).expect("fleet in transit");
        assert_eq!(transit.destination, destination);
        let transfer = world
            .get::<TransferOrbit>(fleet)
            .expect("transfer tracked on the fleet");
        assert_eq!(transfer.fuel_cost, 100);
        assert!(transfer.in_window);
        // The HohmannTransfer alias names the same tracking component.
        let _alias: Option<&HohmannTransfer> = world.get::<HohmannTransfer>(fleet);
        let ledger = world.resource::<LaunchLedger>();
        assert_eq!(ledger.launches, 1);
        assert_eq!(ledger.refused, 0);
        assert_eq!(ledger.fuel_spent, 100.0);
    }

    #[test]
    fn test_launch_refused_when_fuel_insufficient_out_of_window() {
        let (mut world, mut schedule, fleet, origin, destination) = launch_world(500.0, 0.0, PI);
        send_launch(&mut world, fleet, destination);
        schedule.run(&mut world);

        // Nothing burned, nothing moved.
        assert_eq!(world.resource::<ColonyResources>().fuel, 500.0);
        let in_orbit = world.get::<InOrbit>(fleet).expect("fleet stays put");
        assert_eq!(in_orbit.parent, origin);
        assert!(world.get::<InTransit>(fleet).is_none());
        assert!(world.get::<TransferOrbit>(fleet).is_none());
        let ledger = world.resource::<LaunchLedger>();
        assert_eq!(ledger.launches, 0);
        assert_eq!(ledger.refused, 1);
    }

    #[test]
    fn test_launch_out_of_window_with_fuel_burns_10x() {
        let (mut world, mut schedule, fleet, _origin, destination) = launch_world(1500.0, 0.0, PI);
        send_launch(&mut world, fleet, destination);
        schedule.run(&mut world);

        assert_eq!(world.resource::<ColonyResources>().fuel, 500.0);
        let transfer = world.get::<TransferOrbit>(fleet).expect("transfer tracked");
        assert_eq!(transfer.fuel_cost, 1000);
        assert!(!transfer.in_window);
        assert_eq!(world.resource::<LaunchLedger>().fuel_spent, 1000.0);
    }

    #[test]
    fn test_transfer_orbit_plan_records_cost() {
        let origin = orbit(0.0, 100.0, 0.0);
        let destination = orbit(PI, 200.0, 0.0);
        let origin_entity = Entity::from_raw(10);
        let dest_entity = Entity::from_raw(11);
        let plan = TransferOrbit::plan(origin_entity, dest_entity, &origin, &destination);
        assert_eq!(plan.origin, origin_entity);
        assert_eq!(plan.destination, dest_entity);
        assert_eq!(plan.fuel_cost, 1000);
        assert!(!plan.in_window);
        assert_eq!(plan.duration_ticks, 100);
    }

    #[test]
    fn test_transfer_completion_clears_transfer_orbit() {
        let mut world = World::new();
        let origin = world.spawn_empty().id();
        let destination = world.spawn_empty().id();
        let fleet = world
            .spawn((
                Fleet,
                InTransit {
                    origin,
                    destination,
                    progress: 0.95,
                    duration: 10.0,
                },
                TransferOrbit {
                    origin,
                    destination,
                    fuel_cost: 100,
                    in_window: true,
                    duration_ticks: 10,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((
            fleet_movement_system,
            transfer_completion_system.after(fleet_movement_system),
        ));
        schedule.run(&mut world);

        assert!(world.get::<TransferOrbit>(fleet).is_none());
        let in_orbit = world.get::<InOrbit>(fleet).expect("fleet arrived");
        assert_eq!(in_orbit.parent, destination);
    }

    #[test]
    fn test_launch_stats_label() {
        let mut empty = World::new();
        assert_eq!(launch_stats_label(&mut empty), "off");

        let mut world = World::new();
        world.insert_resource(LaunchLedger {
            launches: 2,
            refused: 1,
            fuel_spent: 1100.0,
        });
        assert_eq!(launch_stats_label(&mut world), "2L,1R,1100f");
    }

    #[test]
    fn test_no_banned_ip_terms_in_launch_source() {
        // IP guard: names in this module stay original/generic.
        let source = include_str!("launch.rs").to_lowercase();
        for banned in [
            "skywalker",
            "vader",
            "star trek",
            "borg",
            "x-wing",
            "death star",
            "millennium falcon",
        ] {
            assert!(!source.contains(banned), "banned IP term present: {banned}");
        }
    }
}
