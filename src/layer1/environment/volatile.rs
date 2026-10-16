use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::structure::Structure;
use bevy_ecs::prelude::*;

/// Component representing a volatile item that degrades over time.
#[derive(Component, Debug, Clone)]
pub struct Volatile {
    /// Current stability. Explodes at 0.
    pub stability: f32,
    /// Amount of stability lost per tick.
    pub decay_rate: f32,
    /// Damage dealt upon explosion.
    pub explosion_power: f32,
    /// Radius of the explosion in tiles (Chebyshev distance).
    pub explosion_radius: u32,
    /// If true, decay is paused (e.g. in cryo storage).
    pub paused: bool,
}

/// Event triggered when a volatile item explodes.
#[derive(Event, Debug, Clone)]
pub struct ExplosionEvent {
    /// Center of the explosion.
    pub center: GridPosition,
    /// Damage dealt to structures/pops.
    pub damage: f32,
    /// Radius of effect.
    pub radius: u32,
}

/// System that reduces stability of volatile items and triggers explosions.
pub fn volatile_decay_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Volatile, &GridPosition)>,
    mut events: EventWriter<ExplosionEvent>,
) {
    for (entity, mut volatile, pos) in &mut query {
        if volatile.paused {
            continue;
        }

        volatile.stability -= volatile.decay_rate;

        if volatile.stability <= 0.0 {
            events.send(ExplosionEvent {
                center: *pos,
                damage: volatile.explosion_power,
                radius: volatile.explosion_radius,
            });

            // Spawn Industrial Waste (Spec 1130)
            commands.spawn((
                crate::layer1::resources::ResourceItem {
                    resource_type: crate::layer1::resources::ResourceType::Waste,
                    amount: 10.0,
                },
                *pos,
            ));

            commands.entity(entity).despawn();
        }
    }
}

/// System that processes explosion events and applies damage.
#[allow(clippy::cast_sign_loss)]
pub fn handle_explosion_system(
    mut commands: Commands,
    mut events: EventReader<ExplosionEvent>,
    mut structures: Query<(&GridPosition, &mut Structure)>,
    mut healths: Query<(Entity, &GridPosition, &mut Health)>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for event in events.read() {
        // Damage Structures
        for (pos, mut structure) in &mut structures {
            if event.center.distance_chebyshev(*pos) <= event.radius {
                structure.current_hp -= event.damage;
            }
        }

        // Damage Living Entities (Pops, Fauna, etc.)
        for (entity, pos, mut health) in &mut healths {
            if event.center.distance_chebyshev(*pos) <= event.radius {
                health.take_damage(event.damage);
                // Item 84: attribute the damage source at the moment of damage.
                if event.damage > 0.0 {
                    crate::layer1::biology::death_attribution::record_damage(
                        &mut commands,
                        entity,
                        crate::layer1::biology::death_attribution::CAUSE_EXPLOSION,
                        tick,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::shared::time::SimulationTime;

    #[test]
    fn test_volatile_component_initialization() {
        let volatile = Volatile {
            stability: 100.0,
            decay_rate: 10.0,
            explosion_power: 50.0,
            explosion_radius: 2,
            paused: false,
        };
        assert_eq!(volatile.stability, 100.0);
        assert!(!volatile.paused);
    }

    #[test]
    fn test_decay_system_reduces_stability() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default()); // Time resource
                                                          // Need to register Events<ExplosionEvent> because system uses EventWriter
        world.init_resource::<Events<ExplosionEvent>>();

        let entity = world
            .spawn((
                Volatile {
                    stability: 100.0,
                    decay_rate: 10.0,
                    explosion_power: 10.0,
                    explosion_radius: 1,
                    paused: false,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(volatile_decay_system);
        schedule.run(&mut world);

        let v = world.get::<Volatile>(entity).unwrap();
        assert!(v.stability < 100.0);
    }

    #[test]
    fn test_decay_paused_does_not_reduce() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.init_resource::<Events<ExplosionEvent>>();

        let entity = world
            .spawn((
                Volatile {
                    stability: 100.0,
                    decay_rate: 10.0,
                    explosion_power: 10.0,
                    explosion_radius: 1,
                    paused: true,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(volatile_decay_system);
        schedule.run(&mut world);

        let v = world.get::<Volatile>(entity).unwrap();
        assert_eq!(v.stability, 100.0);
    }

    #[test]
    fn test_explosion_trigger_at_zero_stability() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        // Need Events resource
        world.init_resource::<Events<ExplosionEvent>>();

        let pos = GridPosition { x: 5, y: 5 };
        let entity = world
            .spawn((
                Volatile {
                    stability: 5.0,   // Low stability
                    decay_rate: 10.0, // Should reach 0 in one tick
                    explosion_power: 50.0,
                    explosion_radius: 2,
                    paused: false,
                },
                pos,
            ))
            .id();

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(volatile_decay_system);
        schedule.run(&mut world);

        // Entity should be despawned
        assert!(world.get_entity(entity).is_err());

        // Event should be fired
        let events = world.resource::<Events<ExplosionEvent>>();
        let mut reader = events.get_cursor();
        let event = reader.read(events).next();
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.center, pos);
        assert_eq!(e.damage, 50.0);

        // Assert Waste is spawned
        let mut found_waste = false;
        let mut query = world.query::<(&crate::layer1::resources::ResourceItem, &GridPosition)>();
        for (item, pos_res) in query.iter(&world) {
            if item.resource_type == crate::layer1::resources::ResourceType::Waste
                && *pos_res == pos
            {
                found_waste = true;
                break;
            }
        }
        assert!(
            found_waste,
            "Explosion should spawn Waste at the explosion location."
        );
    }

    #[test]
    fn test_explosion_damages_structures() {
        let mut world = World::new();
        // Register events
        world.init_resource::<Events<ExplosionEvent>>();

        let center = GridPosition { x: 5, y: 5 };
        let target_pos = GridPosition { x: 6, y: 5 }; // Distance 1

        // Spawn Structure
        let structure = world
            .spawn((
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                target_pos,
            ))
            .id();

        // Send Event manually
        let mut events = world.resource_mut::<Events<ExplosionEvent>>();
        events.send(ExplosionEvent {
            center,
            damage: 20.0,
            radius: 2,
        });

        // Run handle system
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_explosion_system);
        schedule.run(&mut world);

        let s = world.get::<Structure>(structure).unwrap();
        assert_eq!(s.current_hp, 80.0);
    }

    #[test]
    fn test_explosion_does_not_damage_outside_radius() {
        let mut world = World::new();
        world.init_resource::<Events<ExplosionEvent>>();

        let center = GridPosition { x: 5, y: 5 };
        let target_pos = GridPosition { x: 8, y: 5 }; // Distance 3

        let structure = world
            .spawn((
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                target_pos,
            ))
            .id();

        let mut events = world.resource_mut::<Events<ExplosionEvent>>();
        events.send(ExplosionEvent {
            center,
            damage: 20.0,
            radius: 2, // Radius 2 < Distance 3
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_explosion_system);
        schedule.run(&mut world);

        let s = world.get::<Structure>(structure).unwrap();
        assert_eq!(s.current_hp, 100.0);
    }
}
