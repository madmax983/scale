use bevy::prelude::*;

#[derive(Component)]
pub struct MassDriver {
    pub ready_to_fire: bool,
    pub target_planet: Entity,
    pub payload_amount: u32,
}

#[derive(Component)]
pub struct InFlightPackage {
    pub target_planet: Entity,
    pub amount: u32,
    pub eta_timer: Timer,
}

#[derive(Component)]
pub struct CatcherNetwork {
    pub success_rate: f32,
    pub last_catch_successful: bool,
}

#[derive(Event)]
pub struct LaunchEvent {
    pub driver_id: Entity,
}

#[derive(Event)]
pub struct BombardmentEvent {
    pub target: Entity,
    pub kinetic_energy: u32,
}

pub fn mass_driver_launch_system(
    mut commands: Commands,
    mut events: EventReader<LaunchEvent>,
    mut query: Query<&mut MassDriver>,
) {
    for event in events.read() {
        if let Ok(mut driver) = query.get_mut(event.driver_id) {
            if driver.ready_to_fire {
                driver.ready_to_fire = false;
                commands.spawn(InFlightPackage {
                    target_planet: driver.target_planet,
                    amount: driver.payload_amount,
                    eta_timer: Timer::from_seconds(5.0, TimerMode::Once), // minimal impl flight time
                });
            }
        }
    }
}

pub fn package_arrival_system(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut InFlightPackage)>,
    mut target_query: Query<&mut CatcherNetwork>,
    mut bomb_writer: EventWriter<BombardmentEvent>,
) {
    for (entity, mut package) in query.iter_mut() {
        package.eta_timer.tick(time.delta());
        if package.eta_timer.just_finished() {
            if let Ok(mut catcher) = target_query.get_mut(package.target_planet) {
                // RNG check for catch success
                if rand::random::<f32>() <= catcher.success_rate {
                    catcher.last_catch_successful = true;
                    // In a real impl, add goods to target inventory here
                } else {
                    catcher.last_catch_successful = false;
                    bomb_writer.send(BombardmentEvent {
                        target: package.target_planet,
                        kinetic_energy: package.amount,
                    });
                }
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Bridges BombardmentEvent (Mass Driver) to crate::layer1::core::chronicle::AddChronicleEvent (Chronicle).
pub fn mass_driver_chronicle_bridge(
    mut bomb_events: EventReader<BombardmentEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for event in bomb_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            // Item 162 audit: `event.target` is an Entity — the
            // sentence names the colony in words, not by Debug dump.
            text: format!(
                "Kinetic Bombardment! Mass driver payload struck the colony with {} energy.",
                event.kinetic_energy
            ),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mass_driver_launches_package() {
        // Arrange
        let mut app = App::new();
        app.add_systems(Update, mass_driver_launch_system);
        app.add_event::<LaunchEvent>();

        let driver = app
            .world_mut()
            .spawn(MassDriver {
                ready_to_fire: true,
                target_planet: Entity::PLACEHOLDER,
                payload_amount: 1000,
            })
            .id();

        // Act
        app.world_mut()
            .send_event(LaunchEvent { driver_id: driver });
        app.update();

        // Assert
        let driver_comp = app.world().get::<MassDriver>(driver).unwrap();
        assert!(
            !driver_comp.ready_to_fire,
            "Driver should not be ready immediately after firing"
        );

        // Ensure the package exists in flight
        let mut query = app.world_mut().query::<&InFlightPackage>();
        let packages: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(packages.len(), 1, "A package should be spawned in flight");
        assert_eq!(packages[0].amount, 1000);
    }

    #[test]
    fn test_package_arrival_catcher_success() {
        // Arrange
        let mut app = App::new();
        app.init_resource::<Time>();
        app.add_event::<BombardmentEvent>();
        app.add_systems(Update, package_arrival_system);

        let planet = app
            .world_mut()
            .spawn(CatcherNetwork {
                success_rate: 1.0,
                last_catch_successful: false,
            })
            .id(); // 100% catch rate
        let package = app
            .world_mut()
            .spawn(InFlightPackage {
                target_planet: planet,
                amount: 1000,
                eta_timer: Timer::from_seconds(0.0, TimerMode::Once), // Arrives instantly
            })
            .id();

        // Act
        app.update();

        // Assert
        assert!(
            app.world().get_entity(package).is_err(),
            "Package should be despawned on arrival"
        );
        let catcher = app.world().get::<CatcherNetwork>(planet).unwrap();
        assert!(
            catcher.last_catch_successful,
            "Catch should be marked successful"
        );
    }

    #[test]
    fn test_package_arrival_catcher_failure_causes_bombardment() {
        // Arrange
        let mut app = App::new();
        app.init_resource::<Time>();
        app.add_systems(Update, package_arrival_system);
        app.add_event::<BombardmentEvent>();

        let planet = app
            .world_mut()
            .spawn(CatcherNetwork {
                success_rate: 0.0,
                last_catch_successful: false,
            })
            .id(); // 0% catch rate
        let package = app
            .world_mut()
            .spawn(InFlightPackage {
                target_planet: planet,
                amount: 1000,
                eta_timer: Timer::from_seconds(0.0, TimerMode::Once),
            })
            .id();

        // Act
        app.update();

        // Assert
        assert!(
            app.world().get_entity(package).is_err(),
            "Package should be despawned on arrival"
        );

        let events = app.world().resource::<Events<BombardmentEvent>>();
        let mut reader = events.get_cursor();
        let bomb_event = reader.read(events).next().unwrap();

        assert_eq!(
            bomb_event.target, planet,
            "Bombardment should hit the target planet"
        );
        assert_eq!(
            bomb_event.kinetic_energy, 1000,
            "Damage should scale with payload amount"
        );
    }
}
