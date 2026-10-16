#![allow(clippy::collapsible_if)]
//! Combat system logic, drafting, and attack resolution.
//!
//! This module handles the "Drafted" state of pops, weapon definitions, and the
//! mechanics of resolving attacks.
//!
//! # The Combat Flow
//!
//! 1.  **Drafting**: A pop is marked with the [`crate::layer1::combat::Drafted`] component. This overrides their
//!     normal Utility AI logic (eating, working) and forces them to prioritize [`ActionType::Fight`](crate::layer1::utility_ai::ActionType::Fight).
//! 2.  **Targeting**: The [`crate::layer1::utility_ai::evaluate_actions_system`] assigns a target (Hostile Fauna, Invaders)
//!     if one is in range.
//! 3.  **Execution**: The [`crate::layer1::combat::execute_attack`] function is called by the execution layer
//!     when the pop is in range and ready to strike.
//! 4.  **Damage**: Damage is calculated based on the equipped [`crate::layer1::combat::Weapon`] and applied to the target's [`crate::layer1::health::Health`].
//!
//! # Key Components
//!
//! *   [`crate::layer1::combat::Drafted`]: The switch that turns a worker into a soldier.
//! *   [`crate::layer1::combat::CombatState`]: Tracks internal cooldowns and last targets.
//! *   [`crate::layer1::combat::Weapon`]: Defines damage, range, and accuracy.

use crate::layer1::map::ScreenShake;
use crate::layer1::particles::{spawn_moving_particle, spawn_particle};
use crate::layer1::physics::hit_stop::{GlobalHitStop, HitStop};
use bevy_ecs::prelude::*;
use rand::Rng;
use ratatui::style::Color;

// Ludwig's Tuning Constants
const CRIT_CHANCE: f64 = 0.2; // Ludwig: Increased base crit chance for more juice
const CRIT_MULTIPLIER: f32 = 2.5; // Ludwig: Rebalanced multiplier to compensate for higher chance

// Ludwig: Adjusted hit stop times for snappier combat (Game Feel)
const HIT_STOP_CRIT: u32 = 15; // Ludwig: Emphasize massive impacts
const HIT_STOP_HEAVY: u32 = 8;
const HIT_STOP_MEDIUM: u32 = 4;
const HIT_STOP_LIGHT: u32 = 1;

/// Component marker for pops that have been drafted for military service.
///
/// When a pop is drafted:
/// *   They ignore needs like Hunger/Rest (up to a point).
/// *   They prioritize combat actions.
/// *   They move to the rally point (cursor).
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Drafted;

/// Tracks the combat cooldowns and state for an entity.
///
/// This component ensures that entities do not attack every single tick.
#[derive(Component, Default, Debug)]
pub struct CombatState {
    /// Ticks remaining until the entity can attack again.
    pub cooldown: u32,
    /// The last entity targeted by this combatant.
    pub last_target: Option<Entity>,
}

/// Defines the statistical properties of an attack or weapon.
#[derive(Clone, Copy, Debug)]
pub struct AttackProperties {
    /// Amount of damage dealt per hit.
    pub damage: f32,
    /// Range in tiles (Chebyshev distance).
    pub range: f32,
    /// Number of ticks to wait between attacks.
    pub cooldown: u32,
    /// Chance to hit (0.0 to 1.0). Currently unused in MVP.
    pub accuracy: f32,
}

/// Component defining an entity as a weapon.
///
/// Weapons are items that can be equipped by pops in the [`crate::layer1::items::Equipment`] slot.
#[derive(Component, Debug)]
pub struct Weapon {
    /// The stats of the weapon.
    pub properties: AttackProperties,
}

/// System to process Combat Cooldown durations.
/// Decrements the counter so entities can attack again.
pub fn combat_cooldown_system(mut query: Query<&mut CombatState>) {
    for mut state in &mut query {
        if state.cooldown > 0 {
            state.cooldown -= 1;
        }
    }
}

/// Resolves an attack from one entity to another.
///
/// This function:
/// 1.  Checks if the attacker has a [`Weapon`] equipped.
/// 2.  Checks if the attacker is on cooldown (via [`CombatState`]).
/// 3.  Applies damage to the target's [`crate::layer1::health::Health`].
/// 4.  Triggers visual effects (particles, screen shake).
///
/// # Examples
///
/// ```
/// use scale::layer1::combat::{execute_attack, CombatState, Weapon, AttackProperties};
/// use scale::layer1::health::Health;
/// use scale::layer1::items::Equipment;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
///
/// // 1. Setup Attacker with a Weapon
/// let sword = world.spawn(Weapon {
///     properties: AttackProperties {
///         damage: 10.0,
///         range: 1.0,
///         cooldown: 5,
///         accuracy: 1.0,
///     }
/// }).id();
///
/// let attacker = world.spawn((
///     CombatState::default(),
///     Equipment { weapon: Some(sword), ..Default::default() }
/// )).id();
///
/// // 2. Setup Target
/// let target = world.spawn(Health { current: 100.0, max: 100.0, has_rust_lung: false }).id();
///
/// // 3. Execute Attack
/// execute_attack(&mut world, attacker, target);
///
/// // 4. Verify Damage
/// let health = world.get::<Health>(target).unwrap();
/// // Crit chance makes exact assertion difficult, just assert damage was taken
/// assert!(health.current < 100.0);
/// ```
fn apply_hit_stop_and_juice(
    world: &mut World,
    attacker: Entity,
    target: Entity,
    damage: f32,
    is_crit: bool,
) {
    // Ludwig: "Juice" logic
    // Scale Hit Stop based on damage severity
    let hit_stop_ticks = if is_crit {
        HIT_STOP_CRIT
    } else if damage >= 15.0 {
        HIT_STOP_HEAVY
    } else if damage >= 5.0 {
        HIT_STOP_MEDIUM
    } else {
        HIT_STOP_LIGHT
    };

    // Hit Stop: Freeze frame on impact
    if hit_stop_ticks > 0 {
        if let Ok(mut entity) = world.get_entity_mut(attacker) {
            entity.insert(HitStop {
                ticks_remaining: hit_stop_ticks,
            });
        }
        if let Ok(mut entity) = world.get_entity_mut(target) {
            entity.insert(HitStop {
                ticks_remaining: hit_stop_ticks,
            });
        }

        // Ludwig: Global freeze frame for big impacts!
        if let Some(mut global_stop) = world.get_resource_mut::<GlobalHitStop>() {
            global_stop.trigger(hit_stop_ticks);
        }
    }

    // Scale feedback based on damage
    let (shake_intensity, particle_char, particle_color, particle_lifetime) = if is_crit {
        (0.8, '!', Color::Yellow, 20)
    } else if damage >= 15.0 {
        (0.4, 'X', Color::Magenta, 12)
    } else {
        (0.15, '*', Color::Red, 5)
    };

    // Trigger Screen Shake (Ludwig: Juice)
    if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
        shake.trigger(shake_intensity);
    }

    // Ludwig: Spawn hit particle
    let Some(pos) = world
        .get::<crate::layer1::map::GridPosition>(target)
        .copied()
    else {
        return;
    };
    spawn_particle(world, pos, particle_char, particle_color, particle_lifetime);

    // Ludwig: Spawn dynamic blood/sparks
    let mut rng = rand::thread_rng();
    let count = if is_crit { 4 } else { 2 };
    // Ludwig: More explosive crits
    let spread = if is_crit { 0.8 } else { 0.5 };
    for _ in 0..count {
        let dx = rng.gen_range(-spread..spread);
        let dy = rng.gen_range(-spread..spread);
        spawn_moving_particle(
            world,
            pos,
            '.',
            particle_color,
            particle_lifetime / 2, // Fade faster
            dx,
            dy,
        );
    }
}

fn get_weapon_stats(world: &World, attacker: Entity) -> Option<(f32, u32)> {
    let equipment = world.get::<crate::layer1::items::Equipment>(attacker)?;
    let weapon_entity = equipment.weapon?;
    let weapon = world.get::<Weapon>(weapon_entity)?;
    Some((weapon.properties.damage, weapon.properties.cooldown))
}

pub fn execute_attack(world: &mut World, attacker: Entity, target: Entity) {
    // 1. Get Attacker stats (Weapon, CombatState)
    let (damage, cooldown_val) = get_weapon_stats(world, attacker).unwrap_or((0.0, 0));

    // Check cooldown
    if let Some(mut state) = world.get_mut::<CombatState>(attacker) {
        if state.cooldown > 0 {
            return;
        }
        state.cooldown = cooldown_val;
        state.last_target = Some(target);
    } else {
        // Sentry: Auto-initialize CombatState to prevent "machine gun" bug
        let Ok(mut entity_cmds) = world.get_entity_mut(attacker) else {
            // Attacker despawned or invalid entity. Abort attack.
            return;
        };
        entity_cmds.insert(CombatState {
            cooldown: cooldown_val,
            last_target: Some(target),
        });
    }

    // 2. Apply damage to Target
    if damage <= 0.0 {
        return;
    }

    // Ludwig: Roll for Crit
    let mut rng = rand::thread_rng();
    let is_crit = if cfg!(test) {
        false
    } else {
        rng.gen_bool(CRIT_CHANCE)
    };

    let target_pos_val = world
        .get::<crate::layer1::map::GridPosition>(target)
        .copied();
    let mut defense_bonus = 0.0;
    if let Some(target_pos) = target_pos_val {
        let mut query = world.query::<(
            &crate::layer1::architecture::fossilized_fleet::FossilizedShip,
            &crate::layer1::map::GridPosition,
        )>();
        for (ship, pos) in query.iter(world) {
            if *pos == target_pos {
                defense_bonus += ship.defense_bonus as f32;
            }
        }
    }

    let final_damage = if is_crit {
        damage * CRIT_MULTIPLIER
    } else {
        damage
    };
    let actual_damage = (final_damage - defense_bonus).max(0.0);

    let Some(mut health) = world.get_mut::<crate::layer1::health::Health>(target) else {
        return;
    };
    health.take_damage(actual_damage);

    // Item 84: attribute the damage source at the moment of damage.
    if actual_damage > 0.0 {
        crate::layer1::biology::death_attribution::note_damage_world(
            world,
            target,
            crate::layer1::biology::death_attribution::CAUSE_COMBAT,
        );
    }

    apply_hit_stop_and_juice(world, attacker, target, actual_damage, is_crit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::fauna::{Fauna, FaunaType};
    use crate::layer1::health::Health;
    use crate::layer1::items::Equipment;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::utility_ai::{ActionType, PopAction};

    fn setup_world() -> World {
        // Initialize TaskPool for parallel systems
        let _ = bevy_tasks::ComputeTaskPool::get_or_init(bevy_tasks::TaskPool::new);

        let mut world = World::new();
        // Setup standard resources (Time, etc)
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world.insert_resource(crate::layer1::utility_types::UtilityConfig::default());
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.init_resource::<GlobalHitStop>();
        world
    }

    // 1. Drafting Logic
    #[test]
    fn test_draft_toggle_overrides_behavior() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                Drafted, // The new component
                PopAction {
                    current: ActionType::Idle,
                    ticks_committed: 10,
                    ..Default::default()
                },
                Equipment::default(),
                GridPosition { x: 0, y: 0 },
                crate::layer1::needs::Needs::default(),
                crate::layer1::utility_types::UtilityWeights::default(),
                // Needs would normally drive behavior, but Drafted suppresses them
            ))
            .id();

        // Run evaluation
        // We expect normal Utility AI to run, but Drafted should force Combat logic
        // or return a specific ActionType::Fight score.

        // For this test, we check if evaluate_actions_system prioritizes Fight
        // when an enemy is present.

        // Spawn Enemy
        let _enemy = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    ..Default::default()
                },
                GridPosition { x: 1, y: 0 },
                Health::default(),
            ))
            .id();

        // Evaluate
        crate::layer1::utility_ai::evaluate_actions_system(&mut world);

        let action = world.get::<PopAction>(pop).unwrap();
        assert_eq!(action.current, ActionType::Fight);
    }

    #[test]
    fn test_undrafted_pop_flees_or_ignores() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                // Not Drafted
                PopAction::default(),
                Equipment::default(),
                GridPosition { x: 0, y: 0 },
                crate::layer1::needs::Needs::default(),
                crate::layer1::utility_types::UtilityWeights::default(),
            ))
            .id();

        let _enemy = world
            .spawn((
                Fauna {
                    fauna_type: FaunaType::Wolf,
                    ..Default::default()
                },
                GridPosition { x: 1, y: 0 },
                Health::default(),
            ))
            .id();

        crate::layer1::utility_ai::evaluate_actions_system(&mut world);

        let action = world.get::<PopAction>(pop).unwrap();
        // Should NOT be Fight. Could be Flee (future) or Idle/Work.
        assert_ne!(action.current, ActionType::Fight);
    }

    // 2. Weapon & Equipment
    #[test]
    fn test_weapon_properties() {
        let sword = Weapon {
            properties: AttackProperties {
                damage: 10.0,
                range: 1.0,
                cooldown: 10,
                accuracy: 0.9,
            },
        };
        assert!((sword.properties.damage - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_pop_uses_equipped_weapon() {
        let mut world = setup_world();

        // Create Weapon Entity
        let sword = world
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: 20.0,
                    range: 1.0,
                    cooldown: 10,
                    accuracy: 1.0,
                },
            })
            .id();

        // Create Pop with Sword
        let pop = world
            .spawn((
                Pop,
                Drafted,
                Equipment {
                    weapon: Some(sword),
                    ..Default::default()
                }, // Updated Equipment struct
                GridPosition { x: 0, y: 0 },
                CombatState::default(),
            ))
            .id();

        // Create Enemy
        let enemy = world
            .spawn((
                Fauna::default(),
                GridPosition { x: 1, y: 0 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        // Manually trigger attack (simulate execution system)
        crate::layer1::combat::execute_attack(&mut world, pop, enemy);

        // Check Enemy Health
        let health = world.get::<Health>(enemy).unwrap();
        let expected_normal = 80.0; // 100 - 20
        let expected_crit = 100.0 - (20.0 * CRIT_MULTIPLIER);
        assert!(
            (health.current - expected_normal).abs() < f32::EPSILON
                || (health.current - expected_crit).abs() < f32::EPSILON,
            "Health should be either normal ({}) or crit ({}), got {}",
            expected_normal,
            expected_crit,
            health.current
        );
    }

    #[test]
    fn test_attack_cooldown() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                Pop,
                Drafted,
                CombatState {
                    cooldown: 5,
                    ..Default::default()
                }, // On cooldown
                Equipment::default(),
            ))
            .id();

        let enemy = world.spawn((Fauna::default(), Health::default())).id();

        // Try attack
        crate::layer1::combat::execute_attack(&mut world, pop, enemy);

        // Should fail/no damage
        let health = world.get::<Health>(enemy).unwrap();
        assert!((health.current - health.max).abs() < f32::EPSILON);
    }

    #[test]
    fn test_execute_attack_aborts_on_invalid_attacker() {
        let mut world = setup_world();

        // Target with 100 Health
        let target = world
            .spawn(Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            })
            .id();

        // Create an entity and immediately despawn it so it's invalid
        let invalid_attacker = world.spawn_empty().id();
        world.despawn(invalid_attacker);

        // This should return early and NOT panic
        crate::layer1::combat::execute_attack(&mut world, invalid_attacker, target);

        // Check Target Health remains untouched
        let health = world.get::<Health>(target).unwrap();
        assert!(
            (health.current - 100.0).abs() < f32::EPSILON,
            "Target should not take damage from an invalid attacker"
        );
    }

    #[test]
    fn test_execute_attack_auto_initializes_combat_state() {
        let mut world = setup_world();

        // 1. Create Weapon (Damage 10, Cooldown 10)
        let sword = world
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: 10.0,
                    range: 1.0,
                    cooldown: 10,
                    accuracy: 1.0,
                },
            })
            .id();

        // 2. Create Attacker (NO CombatState)
        let attacker = world
            .spawn((
                Pop,
                Equipment {
                    weapon: Some(sword),
                    ..Default::default()
                },
            ))
            .id();

        // 3. Create Target
        let target = world
            .spawn(Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            })
            .id();

        // 4. First Attack: Should deal damage AND add CombatState
        crate::layer1::combat::execute_attack(&mut world, attacker, target);

        // Check Damage
        let health = world.get::<Health>(target).unwrap();
        let expected_normal = 90.0;
        let expected_crit = 100.0 - (10.0 * CRIT_MULTIPLIER);
        assert!(
            (health.current - expected_normal).abs() < f32::EPSILON
                || (health.current - expected_crit).abs() < f32::EPSILON,
            "First attack should deal damage (normal or crit)"
        );
        let hp_after_first = health.current;

        // Check CombatState Existence
        let state = world.get::<CombatState>(attacker);
        assert!(
            state.is_some(),
            "execute_attack should verify or insert CombatState to prevent rapid fire"
        );
        let state = state.unwrap();
        assert_eq!(state.cooldown, 10, "Cooldown should be set");

        // 5. Second Attack: Should be blocked by cooldown
        crate::layer1::combat::execute_attack(&mut world, attacker, target);

        // Check Damage (Should be unchanged)
        let health = world.get::<Health>(target).unwrap();
        assert!(
            (health.current - hp_after_first).abs() < f32::EPSILON,
            "Second attack should be blocked by cooldown. Health: {}",
            health.current
        );
    }

    // Re-implement test with proper system runner for Commands

    #[test]
    fn test_combat_cooldown_system() {
        use bevy_ecs::system::RunSystemOnce;
        let mut world = setup_world();

        let entity = world
            .spawn(CombatState {
                cooldown: 2,
                ..Default::default()
            })
            .id();

        // Tick 1
        world
            .run_system_once(crate::layer1::combat::combat_cooldown_system)
            .unwrap();
        let state = world.get::<CombatState>(entity).unwrap();
        assert_eq!(state.cooldown, 1);

        // Tick 2
        world
            .run_system_once(crate::layer1::combat::combat_cooldown_system)
            .unwrap();
        let state = world.get::<CombatState>(entity).unwrap();
        assert_eq!(state.cooldown, 0);

        // Tick 3 (Should not underflow)
        world
            .run_system_once(crate::layer1::combat::combat_cooldown_system)
            .unwrap();
        let state = world.get::<CombatState>(entity).unwrap();
        assert_eq!(state.cooldown, 0);
    }

    use crate::layer1::physics::hit_stop::hit_stop_system;
    #[test]
    fn test_hit_stop_system_flow() {
        use bevy_ecs::system::RunSystemOnce;
        let mut world = setup_world();

        let entity = world.spawn(HitStop { ticks_remaining: 2 }).id();

        // Tick 1
        world.run_system_once(hit_stop_system).unwrap();
        let hit_stop = world.get::<HitStop>(entity).unwrap();
        assert_eq!(hit_stop.ticks_remaining, 1);

        // Tick 2
        world.run_system_once(hit_stop_system).unwrap();
        let hit_stop = world.get::<HitStop>(entity).unwrap();
        assert_eq!(hit_stop.ticks_remaining, 0);

        // Tick 3 (Should remove)
        world.run_system_once(hit_stop_system).unwrap();
        assert!(world.get::<HitStop>(entity).is_none());
    }

    #[test]
    fn test_execute_attack_applies_hit_stop_on_heavy_hit() {
        let mut world = setup_world();

        // Heavy Weapon (Damage 20)
        // Normal: 20 dmg -> Heavy (6 ticks)
        // Crit: 50 dmg -> Crit (15 ticks)
        let weapon = world
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: 20.0,
                    range: 1.0,
                    cooldown: 10,
                    accuracy: 1.0,
                },
            })
            .id();

        let attacker = world
            .spawn((
                Pop,
                Equipment {
                    weapon: Some(weapon),
                    ..Default::default()
                },
            ))
            .id();

        let target = world
            .spawn(Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            })
            .id();

        // Attack
        execute_attack(&mut world, attacker, target);

        // Check HitStop
        let attacker_hs = world.get::<HitStop>(attacker);
        assert!(attacker_hs.is_some(), "Attacker should have HitStop");
        let ticks = attacker_hs.unwrap().ticks_remaining;
        assert!(
            ticks == 8 || ticks == 15,
            "Expected 8 or 15 ticks, got {}",
            ticks
        );

        let target_hs = world.get::<HitStop>(target);
        assert!(target_hs.is_some(), "Target should have HitStop");
        let ticks_target = target_hs.unwrap().ticks_remaining;
        assert!(
            ticks_target == 8 || ticks_target == 15,
            "Expected 8 or 15 ticks, got {}",
            ticks_target
        );

        let global_stop = world.resource::<GlobalHitStop>();
        assert!(
            global_stop.ticks == 8 || global_stop.ticks == 15,
            "GlobalHitStop should match hit stop ticks, got {}",
            global_stop.ticks
        );
    }

    #[test]
    fn test_execute_attack_hit_stop_scaling_light() {
        let mut world = setup_world();

        // Very Light Weapon (Damage 4)
        // Normal: 4 dmg -> Light (1 ticks)
        // Crit: 10 dmg -> Medium (2 ticks, overridden to 15)
        let weapon = world
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: 4.0,
                    range: 1.0,
                    cooldown: 10,
                    accuracy: 1.0,
                },
            })
            .id();

        let attacker = world
            .spawn((
                Pop,
                Equipment {
                    weapon: Some(weapon),
                    ..Default::default()
                },
            ))
            .id();

        let target = world
            .spawn(Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            })
            .id();

        execute_attack(&mut world, attacker, target);

        let hs = world.get::<HitStop>(attacker);
        assert!(hs.is_some(), "Should always have HitStop");
        let ticks = hs.unwrap().ticks_remaining;

        if ticks == 15 {
            // Crit (15 ticks)
        } else {
            // Normal (Damage 4 < 5) -> Light (1 tick)
            assert_eq!(ticks, 1, "Normal light hit should give 1 tick");
        }
    }

    #[test]
    fn test_execute_attack_hit_stop_scaling_medium() {
        let mut world = setup_world();

        // Medium Weapon (Damage 10)
        // Normal: 10 dmg -> Medium (2 ticks)
        // Crit: 25 dmg -> Crit (15 ticks) - because 25 >= 15 is Heavy, but Crit flag overrides to Crit duration?
        // Wait, implementation: if is_crit { 15 } else if dmg >= 15 { 6 } ...
        // So yes, Crit -> 15 ticks.
        let weapon = world
            .spawn(Weapon {
                properties: AttackProperties {
                    damage: 10.0,
                    range: 1.0,
                    cooldown: 10,
                    accuracy: 1.0,
                },
            })
            .id();

        let attacker = world
            .spawn((
                Pop,
                Equipment {
                    weapon: Some(weapon),
                    ..Default::default()
                },
            ))
            .id();

        let target = world
            .spawn(Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            })
            .id();

        execute_attack(&mut world, attacker, target);

        let hs = world.get::<HitStop>(attacker);
        assert!(hs.is_some());
        let ticks = hs.unwrap().ticks_remaining;
        assert!(
            ticks == 4 || ticks == 15,
            "Expected 4 (Normal) or 15 (Crit), got {}",
            ticks
        );

        let global_stop = world.resource::<GlobalHitStop>();
        assert!(
            global_stop.ticks == 4 || global_stop.ticks == 15,
            "GlobalHitStop should match hit stop ticks, got {}",
            global_stop.ticks
        );
    }
}

#[derive(Component, Default, Debug)]
pub struct CombatStats {
    pub melee_damage: f32,
    pub damage_types: Vec<DamageType>,
}

#[derive(Component, Default, Debug)]
pub struct Armor {
    pub rating: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamageType {
    Piercing,
    Venom,
}
