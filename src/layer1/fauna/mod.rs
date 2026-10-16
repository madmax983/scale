//! Fauna
//!
//! Defines wildlife behaviors and procedural fauna generation.

use crate::layer1::execution::{AtTarget, MovementTarget};
use crate::layer1::health::{Dead, Health};
use crate::layer1::map::GridPosition;
use crate::layer1::map::ScreenShake;
use crate::layer1::particles::spawn_particle;
use crate::layer1::physics::hit_stop::HitStop;
use crate::layer1::pop::Pop;
use crate::layer1::utility_types::ActionType;
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;
use ratatui::style::Color;

/// Modular fauna components (Spec 164).
pub mod modular;
pub mod shadow;
pub use modular::*;

/// Type of fauna.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum FaunaType {
    /// A wolf.
    #[default]
    Wolf,
    /// A space rat.
    SpaceRat,
    /// A friendly colony mascot.
    Mascot,
}

/// Current state of the fauna.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaunaState {
    /// Wandering around looking for targets.
    #[default]
    Wander,
    /// Chasing a target.
    Chase,
    /// Attacking an adjacent target.
    Attack,
    /// Fleeing from danger.
    Flee,
}

/// Component representing nocturnal fauna behavior.
#[derive(Component, Default)]
pub struct NocturnalFauna {
    /// How aggressive the fauna is.
    pub aggression: f32,
}

/// Component representing a hostile animal.
#[derive(Component)]
pub struct Fauna {
    /// The type of animal.
    pub fauna_type: FaunaType,
    /// Current behavior state.
    pub state: FaunaState,
    /// Current target entity (usually a Pop).
    pub target: Option<Entity>,
    /// How far this animal can see targets.
    pub detection_range: f32,
    /// Ticks remaining until next attack.
    pub attack_cooldown: u32,
}

impl Default for Fauna {
    fn default() -> Self {
        Self {
            fauna_type: FaunaType::Wolf,
            state: FaunaState::Wander,
            target: None,
            detection_range: 8.0,
            attack_cooldown: 0,
        }
    }
}

/// Suppresses fauna on planets with the SilentWorld trait.
pub fn suppress_fauna_system(
    mut commands: Commands,
    traits: Option<Res<crate::layer1::quirks::PlanetaryTraits>>,
    fauna_query: Query<Entity, With<Fauna>>,
) {
    if let Some(traits) = traits {
        if traits
            .0
            .contains(&crate::layer1::quirks::PlanetaryTrait::SilentWorld)
        {
            for entity in &fauna_query {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// System that drives fauna behavior (AI).
///
/// Implements a simple State Machine:
/// - **Wander**: Scans for Pops within `detection_range`. If found, transitions to `Chase`.
/// - **Chase**: Moves toward target. If adjacent, transitions to `Attack`. If target lost/far, `Wander`.
/// - **Attack**: Deals damage to target.
#[allow(clippy::cast_precision_loss)]
pub fn fauna_behavior_system(world: &mut World) {
    // 1. Query all Fauna
    let mut fauna_updates = Vec::new();
    let mut attacks = Vec::new();

    // Query Pops for targets
    // We collect to avoid borrowing world while iterating query
    let pops: Vec<(Entity, GridPosition)> = world
        .query_filtered::<(Entity, &GridPosition), With<Pop>>()
        .iter(world)
        .map(|(e, p)| (e, *p))
        .collect();

    let mut query = world.query::<(Entity, &mut Fauna, &GridPosition, Option<&FaunaBody>)>();

    for (entity, mut fauna, pos, body) in query.iter_mut(world) {
        if fauna.attack_cooldown > 0 {
            fauna.attack_cooldown -= 1;
        }

        match fauna.state {
            FaunaState::Wander => handle_wander_state(&mut fauna, pos, &pops),
            FaunaState::Chase => handle_chase_state(
                &mut fauna,
                entity,
                pos,
                body,
                &pops,
                &mut attacks,
                &mut fauna_updates,
            ),
            FaunaState::Attack => {
                handle_attack_state(&mut fauna, entity, pos, body, &pops, &mut attacks)
            }
            FaunaState::Flee => {}
        }
    }

    // Apply movement updates
    for (entity, target_pos) in fauna_updates {
        world.entity_mut(entity).insert(MovementTarget {
            target_entity: Entity::from_raw(0), // Dummy, not used for Idle action
            target_position: target_pos,
            for_action: ActionType::Idle, // Dummy action
        });
        // Also remove AtTarget if present, so it keeps moving
        world.entity_mut(entity).remove::<AtTarget>();
    }

    // Apply damage
    for (attacker, target, damage) in attacks {
        if let Some(mut health) = world.get_mut::<Health>(target) {
            health.take_damage(damage);

            // Item 84: attribute the damage source at the moment of damage.
            if damage > 0.0 {
                crate::layer1::biology::death_attribution::note_damage_world(
                    world,
                    target,
                    crate::layer1::biology::death_attribution::CAUSE_WILDLIFE,
                );
            }

            // Ludwig: "Juice" logic for Fauna attacks
            let hit_stop_ticks = if damage >= 10.0 {
                6
            } else if damage >= 5.0 {
                2
            } else {
                1
            };

            // Apply HitStop to both attacker and target
            if hit_stop_ticks > 0 {
                if let Ok(mut entity_mut) = world.get_entity_mut(attacker) {
                    entity_mut.insert(HitStop {
                        ticks_remaining: hit_stop_ticks,
                    });
                }
                if let Ok(mut entity_mut) = world.get_entity_mut(target) {
                    entity_mut.insert(HitStop {
                        ticks_remaining: hit_stop_ticks,
                    });
                }
            }

            if let Some(pos) = world.get::<GridPosition>(target).copied() {
                spawn_particle(world, pos, '*', Color::Red, 5);

                // Only shake if the attack is near the camera — prevents constant
                // shake when many fauna are fighting off-screen.
                let near_camera = world
                    .get_resource::<crate::layer1::map::CameraTarget>()
                    .is_none_or(|cam| {
                        let dx = (pos.x as f32 - cam.x).abs();
                        let dy = (pos.y as f32 - cam.y).abs();
                        dx <= 30.0 && dy <= 30.0
                    });

                if near_camera {
                    let shake_intensity = if damage >= 10.0 { 0.4 } else { 0.15 };
                    if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
                        shake.trigger(shake_intensity);
                    }
                }
            }

            // Log damage
            if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
                log.add("DANGER: A wild animal is attacking!");
            }
        }
    }
}

/// Handles death events specific to Fauna.
#[allow(clippy::type_complexity)]
pub fn handle_fauna_death_system(
    query: Query<(Entity, Option<&GridPosition>), (With<Fauna>, Added<Dead>)>,
    mut commands: Commands,
    mut log: Option<ResMut<MessageLog>>,
) {
    for (_entity, pos_opt) in query.iter() {
        // 1. Visuals
        if let Some(pos) = pos_opt {
            commands.spawn((
                crate::layer1::particles::Particle {
                    char: '%',
                    color: Color::Red,
                    lifetime: 10,
                },
                *pos,
            ));
        }

        // 2. Log
        if let Some(log) = log.as_mut() {
            log.add_colored("Creature slain!", Color::Red);
        }
    }
}

fn handle_wander_state(fauna: &mut Fauna, pos: &GridPosition, pops: &[(Entity, GridPosition)]) {
    let mut best_target = None;
    let mut min_dist = fauna.detection_range;

    for (target_e, target_pos) in pops {
        let dist = pos.distance_chebyshev(*target_pos) as f32;
        if dist <= min_dist {
            min_dist = dist;
            best_target = Some(*target_e);
        }
    }

    if let Some(target) = best_target {
        fauna.state = FaunaState::Chase;
        fauna.target = Some(target);
    }
}

fn handle_chase_state(
    fauna: &mut Fauna,
    entity: Entity,
    pos: &GridPosition,
    body: Option<&FaunaBody>,
    pops: &[(Entity, GridPosition)],
    attacks: &mut Vec<(Entity, Entity, f32)>,
    fauna_updates: &mut Vec<(Entity, GridPosition)>,
) {
    let Some(target) = fauna.target else {
        fauna.state = FaunaState::Wander;
        return;
    };

    let Some((_, target_pos)) = pops.iter().find(|(e, _)| *e == target) else {
        fauna.state = FaunaState::Wander;
        fauna.target = None;
        return;
    };

    let dist = pos.distance_chebyshev(*target_pos);

    if dist <= 1 {
        if fauna.attack_cooldown == 0 {
            let damage = body.map_or(5.0, |b| b.aggregate_stats().attack);
            attacks.push((entity, target, damage));
            fauna.attack_cooldown = 10;
            fauna.state = FaunaState::Attack;
        }
    } else if dist as f32 > fauna.detection_range * 1.5 {
        fauna.state = FaunaState::Wander;
        fauna.target = None;
    } else {
        fauna.state = FaunaState::Chase;
        fauna_updates.push((entity, *target_pos));
    }
}

fn handle_attack_state(
    fauna: &mut Fauna,
    entity: Entity,
    pos: &GridPosition,
    body: Option<&FaunaBody>,
    pops: &[(Entity, GridPosition)],
    attacks: &mut Vec<(Entity, Entity, f32)>,
) {
    let Some(target) = fauna.target else {
        fauna.state = FaunaState::Wander;
        return;
    };

    let Some((_, target_pos)) = pops.iter().find(|(e, _)| *e == target) else {
        fauna.state = FaunaState::Wander;
        fauna.target = None;
        return;
    };

    let dist = pos.distance_chebyshev(*target_pos);
    if dist <= 1 {
        if fauna.attack_cooldown == 0 {
            let damage = body.map_or(5.0, |b| b.aggregate_stats().attack);
            attacks.push((entity, target, damage));
            fauna.attack_cooldown = 10;
        }
    } else {
        fauna.state = FaunaState::Chase;
    }
}

pub mod immortal_mascot;

#[cfg(test)]
mod tests {
    use crate::layer1::execution::MovementTarget;
    use crate::layer1::fauna::{fauna_behavior_system, Fauna, FaunaState, FaunaType};
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use bevy_ecs::prelude::*;

    // Helper to create a basic world with necessary resources
    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world
    }

    #[test]
    fn test_fauna_spawn_defaults() {
        let rat = Fauna {
            fauna_type: FaunaType::SpaceRat,
            ..Default::default()
        };
        assert_eq!(rat.state, FaunaState::Wander);
        assert!(rat.detection_range > 0.0);
        // assert!(rat.attack_damage > 0.0); // Removed field
    }

    #[test]
    fn test_fauna_detects_target_in_range() {
        let mut world = setup_world();

        // Spawn Wolf
        let wolf = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    detection_range: 5.0,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Health::default(),
            ))
            .id();

        // Spawn Pop nearby
        let pop = world
            .spawn((Pop, GridPosition { x: 2, y: 0 }, Health::default()))
            .id();

        // Run behavior system
        fauna_behavior_system(&mut world);

        // Wolf should be Chasing pop
        let wolf_comp = world.get::<Fauna>(wolf).unwrap();
        assert_eq!(wolf_comp.state, FaunaState::Chase);
        assert_eq!(wolf_comp.target, Some(pop));
    }

    #[test]
    fn test_fauna_ignores_target_out_of_range() {
        let mut world = setup_world();

        let wolf = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    detection_range: 5.0,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Health::default(),
            ))
            .id();

        // Spawn Pop far away
        world.spawn((Pop, GridPosition { x: 10, y: 0 }, Health::default()));

        fauna_behavior_system(&mut world);

        let wolf_comp = world.get::<Fauna>(wolf).unwrap();
        assert_eq!(wolf_comp.state, FaunaState::Wander);
        assert_eq!(wolf_comp.target, None);
    }

    #[test]
    fn test_fauna_attacks_adjacent_target() {
        let mut world = setup_world();

        use crate::layer1::fauna::{BodyPartType, FaunaBody, FaunaPart, FaunaStats};
        let mut body = FaunaBody::default();
        body.add_part(FaunaPart {
            part_type: BodyPartType::Head,
            stats: FaunaStats {
                attack: 10.0,
                ..Default::default()
            },
            ..Default::default()
        });

        // Spawn Wolf adjacent to Pop
        let wolf = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    // attack_damage: 10.0, // Removed
                    ..Default::default()
                },
                body, // Added body
                GridPosition { x: 0, y: 0 },
                Health::default(),
            ))
            .id(); // Wolf

        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 1, y: 0 }, // Adjacent
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        // Manually set state to Chase/Attack for test setup
        let mut wolf_mut = world.get_mut::<Fauna>(wolf).unwrap();
        wolf_mut.state = FaunaState::Chase;
        wolf_mut.target = Some(pop);

        fauna_behavior_system(&mut world);

        // Pop should take damage
        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0);
        assert_eq!(health.current, 90.0); // 100 - 10
                                          // Wolf should stay in Chase/Attack mode
        let wolf_comp = world.get::<Fauna>(wolf).unwrap();
        assert!(matches!(
            wolf_comp.state,
            FaunaState::Chase | FaunaState::Attack
        ));
    }

    #[test]
    fn test_fauna_sets_movement_target() {
        let mut world = setup_world();

        let wolf = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Health::default(),
            ))
            .id();

        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 0 }, // Far enough to move, close enough to chase
                Health::default(),
            ))
            .id();

        // Setup chase state
        let mut wolf_mut = world.get_mut::<Fauna>(wolf).unwrap();
        wolf_mut.state = FaunaState::Chase;
        wolf_mut.target = Some(pop);
        wolf_mut.detection_range = 10.0;

        fauna_behavior_system(&mut world);

        // Should have MovementTarget component added
        assert!(world.get::<MovementTarget>(wolf).is_some());
        let target = world.get::<MovementTarget>(wolf).unwrap();
        assert_eq!(target.target_position, GridPosition { x: 5, y: 0 });
    }

    #[test]
    fn test_fauna_loses_target() {
        let mut world = setup_world();

        let wolf = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    state: FaunaState::Chase,
                    target: None, // Will set below
                    detection_range: 5.0,
                    ..Default::default()
                },
                GridPosition { x: 0, y: 0 },
                Health::default(),
            ))
            .id();

        // Target (pop) is far away (distance 10 > 5 * 1.5 = 7.5)
        let pop = world.spawn((Pop, GridPosition { x: 10, y: 0 })).id();

        // Update target entity ID
        world.get_mut::<Fauna>(wolf).unwrap().target = Some(pop);

        fauna_behavior_system(&mut world);

        let wolf_comp = world.get::<Fauna>(wolf).unwrap();
        assert_eq!(wolf_comp.state, FaunaState::Wander);
        assert_eq!(wolf_comp.target, None);
    }
}

pub mod modular_fauna;
pub use modular_fauna::*;
