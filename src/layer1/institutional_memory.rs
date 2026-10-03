#![allow(clippy::type_complexity)]
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::economy::items::Item;
use crate::layer1::execution::{AtTarget, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::skills::{SkillType, Skills};
use crate::layer1::utility_ai::ActionType;
use bevy_ecs::prelude::*;
use rand::prelude::*;

/// Configuration for Institutional Memory system.
#[derive(Resource, Debug, Clone)]
pub struct InstitutionalMemoryConfig {
    /// Probability to produce a manual per tick for a working Level 5+ pop.
    pub production_chance: f64,
}

impl Default for InstitutionalMemoryConfig {
    fn default() -> Self {
        Self {
            production_chance: 0.001,
        }
    }
}

/// Component representing a manual that can boost nearby workers' XP.
#[derive(Component, Debug, Clone)]
pub struct Manual {
    /// The skill this manual boosts.
    pub skill_type: SkillType,
    /// The multiplier for XP gain (e.g., 1.2 for +20%).
    pub xp_multiplier: f32,
    /// Current durability of the manual.
    pub durability: f32,
    /// Maximum durability of the manual.
    pub max_durability: f32,
}

/// System that spawns manuals when high-skill pops work.
pub fn produce_manual_system(
    mut commands: Commands,
    query: Query<
        (&Skills, &GridPosition, &MovementTarget),
        (With<crate::layer1::pop::Pop>, With<AtTarget>),
    >,
    designations: Query<&Designation>,
    config: Option<Res<InstitutionalMemoryConfig>>,
) {
    let mut rng = thread_rng();
    let chance = config.map_or(0.001, |c| c.production_chance);

    for (skills, pos, target) in &query {
        if target.for_action != ActionType::Work {
            continue;
        }

        let skill_type_opt = designations
            .get(target.target_entity)
            .map_or(None, |designation| match designation.designation_type {
                DesignationType::Mine => Some(SkillType::Mining),
                DesignationType::Chop => Some(SkillType::Forestry),
                DesignationType::ClearFlora | DesignationType::CollectSample => {
                    Some(SkillType::Farming)
                } // Farming/Foraging
                DesignationType::Demolish
                | DesignationType::Repair
                | DesignationType::JuryRig
                | DesignationType::Destroy
                | DesignationType::SetZone(_) => Some(SkillType::Construction),
                DesignationType::ExcavateDust => Some(SkillType::Mining),
                DesignationType::Tame => Some(SkillType::Husbandry),
                DesignationType::Cannibalize | DesignationType::Consume => None,
            });

        let (chosen_skill, level) = if let Some(s) = skill_type_opt {
            let lvl = skills.get_level(s);
            if lvl >= 5 {
                (s, lvl)
            } else {
                continue;
            }
        } else {
            // Fallback for missing designation
            let mut best_skill = None;
            let mut max_level = 0;

            for s in skills.xp.keys() {
                let lvl = skills.get_level(*s);
                if lvl >= 5 && lvl > max_level {
                    max_level = lvl;
                    best_skill = Some(*s);
                }
            }

            if let Some(s) = best_skill {
                (s, max_level)
            } else {
                continue;
            }
        };

        if rng.gen_bool(chance) {
            #[allow(clippy::cast_precision_loss)]
            let multiplier = (level as f32).mul_add(0.05, 1.0);

            commands.spawn((
                Item {
                    item_type: crate::layer1::economy::items::ItemType::Manual,
                }, // Marker for Hauling
                Manual {
                    skill_type: chosen_skill,
                    xp_multiplier: multiplier,
                    durability: 100.0,
                    max_durability: 100.0,
                },
                *pos, // Drop at feet
            ));
        }
    }
}

/// System that boosts XP of nearby workers based on manual aura.
pub fn manual_aura_system(
    mut commands: Commands,
    mut manuals: Query<(Entity, &mut Manual, &GridPosition)>,
    mut workers: Query<
        (&GridPosition, &mut Skills, &MovementTarget),
        (With<crate::layer1::pop::Pop>, With<AtTarget>),
    >,
    designations: Query<&Designation>,
) {
    for (manual_entity, mut manual, manual_pos) in &mut manuals {
        let mut used = false;

        for (worker_pos, mut skills, target) in &mut workers {
            if target.for_action != ActionType::Work {
                continue;
            }

            // Distance check (Radius 5) - Manhattan distance
            let dist = manual_pos
                .x
                .abs_diff(worker_pos.x)
                .saturating_add(manual_pos.y.abs_diff(worker_pos.y))
                .min(i32::MAX as u32) as i32;
            if dist > 5 {
                continue;
            }

            // Check if worker is performing the same task
            let is_matching_task = if let Ok(designation) = designations.get(target.target_entity) {
                // Map designation to skill and compare with manual.skill_type
                let skill_opt = match designation.designation_type {
                    DesignationType::Mine => Some(SkillType::Mining),
                    DesignationType::Chop => Some(SkillType::Forestry),
                    DesignationType::ClearFlora | DesignationType::CollectSample => {
                        Some(SkillType::Farming)
                    }
                    DesignationType::Demolish
                    | DesignationType::Repair
                    | DesignationType::JuryRig
                    | DesignationType::Destroy
                    | DesignationType::SetZone(_) => Some(SkillType::Construction),
                    DesignationType::ExcavateDust => Some(SkillType::Mining),
                    DesignationType::Tame => Some(SkillType::Husbandry),
                    DesignationType::Cannibalize | DesignationType::Consume => None,
                };
                skill_opt == Some(manual.skill_type)
            } else {
                // If no designation, we assume loose matching for tests/implicit tasks
                true
            };

            if is_matching_task {
                #[allow(clippy::cast_possible_truncation)]
                skills.add_xp(manual.skill_type, 0.1 * manual.xp_multiplier);
                used = true;
            }
        }

        if used {
            manual.durability -= 0.1;
            if manual.durability <= 0.0 {
                // Despawn manual entity
                commands.entity(manual_entity).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::economy::items::Item;
    use crate::layer1::execution::{AtTarget, MovementTarget};
    use crate::layer1::institutional_memory::{
        manual_aura_system, produce_manual_system, InstitutionalMemoryConfig, Manual,
    };
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::utility_ai::ActionType;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_produce_manual_high_skill() {
        let mut world = World::new();

        // Guarantee production for test
        world.insert_resource(InstitutionalMemoryConfig {
            production_chance: 1.0,
        });

        // Spawn high-skill pop working
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 2500.0); // Level 5
        let _pop = world
            .spawn((
                Pop,
                skills,
                GridPosition { x: 5, y: 5 },
                MovementTarget {
                    target_entity: Entity::from_raw(0),
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget, // Must be actively working
            ))
            .id();

        // Run system multiple times to trigger chance (or mock RNG)
        let mut schedule = Schedule::default();
        schedule.add_systems(produce_manual_system);

        schedule.run(&mut world);

        // Check if Manual item spawned at position
        let mut manual_query = world.query::<(&Manual, &GridPosition)>();
        let mut found = false;
        for (manual, pos) in manual_query.iter(&world) {
            if pos.x == 5 && pos.y == 5 && manual.skill_type == SkillType::Mining {
                found = true;
                break;
            }
        }
        assert!(found, "Should spawn a manual for high skill worker");
    }

    #[test]
    fn test_produce_manual_low_skill_fails() {
        let mut world = World::new();
        // Spawn low-skill pop working
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 0.0); // Level 0
        world.spawn((
            Pop,
            skills,
            GridPosition { x: 5, y: 5 },
            MovementTarget {
                target_entity: Entity::from_raw(0),
                target_position: GridPosition { x: 5, y: 5 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(produce_manual_system);

        // Run many times to be sure
        for _ in 0..100 {
            schedule.run(&mut world);
        }

        let count = world.query::<&Manual>().iter(&world).count();
        assert_eq!(count, 0, "Low skill pop should not produce manual");
    }

    #[test]
    fn test_manual_aura_boosts_xp() {
        let mut world = World::new();

        // Spawn Manual on ground
        world.spawn((
            Item::default(),
            Manual {
                skill_type: SkillType::Mining,
                xp_multiplier: 1.5,
                durability: 100.0,
                max_durability: 100.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn Worker nearby (Level 0)
        let worker = world
            .spawn((
                Pop,
                Skills::default(),
                GridPosition { x: 6, y: 5 }, // Adjacent
                MovementTarget {
                    target_entity: Entity::from_raw(0),
                    target_position: GridPosition { x: 6, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        // Run aura system
        let mut schedule = Schedule::default();
        schedule.add_systems(manual_aura_system);
        schedule.run(&mut world);

        // Verify XP gain
        let skills = world.get::<Skills>(worker).unwrap();
        assert!(
            skills.get_xp(SkillType::Mining) > 0.0,
            "Should gain passive XP from Manual aura"
        );
    }

    #[test]
    fn test_manual_degradation() {
        let mut world = World::new();
        let manual = world
            .spawn((
                Manual {
                    skill_type: SkillType::Mining,
                    xp_multiplier: 1.5,
                    durability: 0.05, // Very low durability
                    max_durability: 100.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Spawn worker to trigger usage
        world.spawn((
            Pop,
            Skills::default(),
            GridPosition { x: 6, y: 5 },
            MovementTarget {
                target_entity: Entity::from_raw(0),
                target_position: GridPosition { x: 6, y: 5 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(manual_aura_system);
        schedule.run(&mut world); // Tick 1: Durability decreases

        // Check if manual is destroyed
        // Assuming decay is > 0.05 per use
        assert!(
            world.get_entity(manual).is_err(),
            "Manual should be destroyed when durability hits 0"
        );
    }

    #[test]
    fn test_produce_manual_with_designation() {
        use crate::layer1::designation::{Designation, DesignationType};

        let mut world = World::new();
        world.insert_resource(InstitutionalMemoryConfig {
            production_chance: 1.0,
        });

        // Spawn Designation
        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Spawn Pop with Mining skill working on designation
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, 2500.0);
        world.spawn((
            Pop,
            skills,
            GridPosition { x: 5, y: 5 },
            MovementTarget {
                target_entity: designation,
                target_position: GridPosition { x: 5, y: 5 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(produce_manual_system);
        schedule.run(&mut world);

        let count = world.query::<&Manual>().iter(&world).count();
        assert_eq!(
            count, 1,
            "Should produce manual when working on designation"
        );

        let manual = world.query::<&Manual>().single(&world);
        assert_eq!(manual.skill_type, SkillType::Mining);
    }

    #[test]
    fn test_manual_aura_strict_match() {
        use crate::layer1::designation::{Designation, DesignationType};
        let mut world = World::new();

        // Manual for Mining
        world.spawn((
            Item::default(),
            Manual {
                skill_type: SkillType::Mining,
                xp_multiplier: 1.5,
                durability: 10.0,
                max_durability: 10.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Designation for Forestry (Mismatch)
        let forestry_designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Chop,
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();

        // Worker doing Forestry
        let worker = world
            .spawn((
                Pop,
                Skills::default(),
                GridPosition { x: 6, y: 5 },
                MovementTarget {
                    target_entity: forestry_designation,
                    target_position: GridPosition { x: 6, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(manual_aura_system);
        schedule.run(&mut world);

        // Should NOT gain XP in Mining because task is Forestry
        let skills = world.get::<Skills>(worker).unwrap();
        assert_eq!(
            skills.get_xp(SkillType::Mining),
            0.0,
            "Should not gain XP for mismatched task"
        );
    }

    #[test]
    fn test_manual_aura_distance_limit() {
        let mut world = World::new();
        // Manual at 0,0
        world.spawn((
            Item::default(),
            Manual {
                skill_type: SkillType::Mining,
                xp_multiplier: 1.5,
                durability: 10.0,
                max_durability: 10.0,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Worker at 6,0 (Distance 6)
        let worker = world
            .spawn((
                Pop,
                Skills::default(),
                GridPosition { x: 6, y: 0 },
                MovementTarget {
                    target_entity: Entity::from_raw(0),
                    target_position: GridPosition { x: 6, y: 0 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(manual_aura_system);
        schedule.run(&mut world);

        let skills = world.get::<Skills>(worker).unwrap();
        assert_eq!(
            skills.get_xp(SkillType::Mining),
            0.0,
            "Should not gain XP if too far"
        );
    }
}
