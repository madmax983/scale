//! Spec 378 RED tests, adapted to the real tree.
//!
//! The spec sketched `crate::layer2::map::SystemCoordinate` and
//! `PowerConsumer { consumed }`; neither exists. Adaptations:
//! `SystemCoordinate` lives in `layer2::comms`, relays position via
//! it or via `Transform`, and the real `PowerConsumer` is
//! `{ active, demand }` from `layer1::energy`.

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::chronicle::AddChronicleEvent;
    use crate::layer1::energy::PowerConsumer;
    use crate::layer1::map::GridPosition;
    use crate::layer2::comms::{
        comms_reveal_chronicle_bridge_system, process_building_comms_coverage_system,
        process_comms_coverage_system, reveal_signal_contacts_system, CommsRelay,
        CommsRevealState, EntityRevealedEvent, SignalContact, SignalNoiseMap, SystemCoordinate,
        BUILDING_RELAY_RANGE,
    };
    use crate::layer2::sensors::VisibilityStatus;
    use bevy::prelude::Transform;
    use bevy_ecs::prelude::*;
    use bevy_ecs::schedule::IntoSystemConfigs;

    fn noisy_world() -> World {
        let mut world = World::new();
        world.insert_resource(SignalNoiseMap {
            radius: 100,
            noise: vec![true; 10_000],
        });
        world
    }

    fn coverage_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                process_comms_coverage_system,
                process_building_comms_coverage_system,
            )
                .chain(),
        );
        schedule
    }

    #[test]
    fn test_noise_map_initializes_fully_noisy() {
        let map = SignalNoiseMap::default();
        assert_eq!(map.noise.len(), 100 * 100);
        assert!(map.is_noisy(0.0, 0.0));
        assert!(map.is_noisy(50.0, 50.0));
        assert!(map.is_noisy(99.0, 99.0));
        // Out of bounds is always noisy.
        assert!(map.is_noisy(500.0, 500.0));
        assert!(map.is_noisy(-1.0, 10.0));
        assert_eq!(map.clear_count(), 0);
    }

    #[test]
    fn test_comms_relay_clears_signal_noise() {
        // Spec RED test 1, adapted: real PowerConsumer shape.
        let mut world = noisy_world();
        world.spawn((
            CommsRelay {
                range: 10.0,
                active: true,
            },
            SystemCoordinate { x: 50.0, y: 50.0 },
            PowerConsumer {
                active: true,
                demand: 5.0,
            },
        ));
        coverage_schedule().run(&mut world);
        let map = world.resource::<SignalNoiseMap>();
        assert!(!map.is_noisy(50.0, 50.0), "relay centre must be clear");
        assert!(!map.is_noisy(55.0, 50.0), "inside range must be clear");
        assert!(map.is_noisy(80.0, 80.0), "far outside range stays noisy");
        assert!(map.clear_count() > 0);
    }

    #[test]
    fn test_unpowered_comms_relay_fails_to_clear_noise() {
        // Spec RED test 2, first form: the relay is switched off.
        let mut world = noisy_world();
        world.spawn((
            CommsRelay {
                range: 10.0,
                active: false,
            },
            SystemCoordinate { x: 50.0, y: 50.0 },
        ));
        coverage_schedule().run(&mut world);
        assert!(world.resource::<SignalNoiseMap>().is_noisy(50.0, 50.0));

        // Second form: relay on, but its power consumer is unpowered.
        let mut world = noisy_world();
        world.spawn((
            CommsRelay {
                range: 10.0,
                active: true,
            },
            SystemCoordinate { x: 50.0, y: 50.0 },
            PowerConsumer {
                active: false,
                demand: 5.0,
            },
        ));
        coverage_schedule().run(&mut world);
        assert!(
            world.resource::<SignalNoiseMap>().is_noisy(50.0, 50.0),
            "an unpowered relay must not clear noise"
        );
    }

    #[test]
    fn test_relay_via_transform_clears_noise() {
        // Layer 2's real positioning seam: Transform, no coordinate.
        let mut world = noisy_world();
        world.spawn((
            CommsRelay {
                range: 10.0,
                active: true,
            },
            Transform::from_xyz(20.0, 30.0, 0.0),
        ));
        coverage_schedule().run(&mut world);
        let map = world.resource::<SignalNoiseMap>();
        assert!(!map.is_noisy(20.0, 30.0));
        assert!(map.is_noisy(50.0, 50.0));
    }

    #[test]
    fn test_powered_building_relay_clears_and_unpowered_does_not() {
        // The BuildingType::CommsRelay seam (visibility.rs pattern).
        let mut world = noisy_world();
        world.spawn((
            Building {
                building_type: BuildingType::CommsRelay,
            },
            PowerConsumer {
                active: true,
                demand: 5.0,
            },
            GridPosition { x: 50, y: 50 },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::CommsRelay,
            },
            PowerConsumer {
                active: false,
                demand: 5.0,
            },
            GridPosition { x: 10, y: 10 },
        ));
        coverage_schedule().run(&mut world);
        let map = world.resource::<SignalNoiseMap>();
        assert!(!map.is_noisy(50.0, 50.0), "powered building clears");
        assert!(
            !map.is_noisy(50.0 + BUILDING_RELAY_RANGE - 1.0, 50.0),
            "building range reaches out to BUILDING_RELAY_RANGE"
        );
        assert!(map.is_noisy(10.0, 10.0), "unpowered building stays dark");
    }

    #[test]
    fn test_coverage_recomputes_each_tick() {
        // Kill the power and the static closes back in.
        let mut world = noisy_world();
        let relay = world
            .spawn((
                CommsRelay {
                    range: 10.0,
                    active: true,
                },
                SystemCoordinate { x: 50.0, y: 50.0 },
                PowerConsumer {
                    active: true,
                    demand: 5.0,
                },
            ))
            .id();
        let mut schedule = coverage_schedule();
        schedule.run(&mut world);
        assert!(!world.resource::<SignalNoiseMap>().is_noisy(50.0, 50.0));
        world.get_mut::<PowerConsumer>(relay).unwrap().active = false;
        schedule.run(&mut world);
        assert!(
            world.resource::<SignalNoiseMap>().is_noisy(50.0, 50.0),
            "noise must return when the relay loses power"
        );
    }

    #[test]
    fn test_contact_revealed_in_clear_zone_hidden_in_noise() {
        let mut world = noisy_world();
        world.init_resource::<CommsRevealState>();
        world.init_resource::<Events<EntityRevealedEvent>>();
        let contact = world
            .spawn((
                SignalContact,
                SystemCoordinate { x: 50.0, y: 50.0 },
                VisibilityStatus { is_visible: false },
            ))
            .id();
        let hidden = world
            .spawn((
                SignalContact,
                SystemCoordinate { x: 90.0, y: 90.0 },
                VisibilityStatus { is_visible: false },
            ))
            .id();
        world.spawn((
            CommsRelay {
                range: 10.0,
                active: true,
            },
            SystemCoordinate { x: 50.0, y: 50.0 },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (process_comms_coverage_system, reveal_signal_contacts_system).chain(),
        );
        schedule.run(&mut world);
        schedule.run(&mut world);

        assert!(world.get::<VisibilityStatus>(contact).unwrap().is_visible);
        assert!(!world.get::<VisibilityStatus>(hidden).unwrap().is_visible);
        let state = world.resource::<CommsRevealState>();
        assert_eq!(state.total_revealed, 1, "exactly one reveal, no spam");
        let events = world.resource::<Events<EntityRevealedEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].entity, contact);
    }

    #[test]
    fn test_reveal_bridge_writes_chronicle_event() {
        let mut world = World::new();
        world.init_resource::<Events<EntityRevealedEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        let entity = world.spawn_empty().id();
        world
            .resource_mut::<Events<EntityRevealedEvent>>()
            .send(EntityRevealedEvent {
                entity,
                x: 42.0,
                y: 7.0,
            });
        let mut schedule = Schedule::default();
        schedule.add_systems(comms_reveal_chronicle_bridge_system);
        schedule.run(&mut world);
        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<_> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1);
        assert!(fired[0].text.contains("revealed at (42, 7)"));
    }

    #[test]
    fn test_no_banned_ip_terms_in_comms_source() {
        // IP guard: names in this module stay original/generic.
        let source = include_str!("comms.rs").to_lowercase();
        for banned in ["skywalker", "vader", "startrek", "star trek", "borg", "x-wing"] {
            assert!(!source.contains(banned), "banned IP term present: {banned}");
        }
    }
}
