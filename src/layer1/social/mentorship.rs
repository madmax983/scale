use crate::layer1::skills::SkillType;
use bevy_ecs::prelude::*;

/// Component representing an active mentorship relationship.
/// The entity with this component is the "Apprentice" and receives XP bonuses.
#[derive(Component, Debug, Clone)]
pub struct Mentorship {
    /// The entity ID of the Master.
    pub master_entity: Entity,
    /// The skill being mentored.
    pub skill: SkillType,
    /// The XP multiplier (e.g., 1.5 for +50% XP).
    pub multiplier: f32,
    /// Ticks until this relationship needs re-evaluation.
    pub expiration: u32,
}

use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::execution::{AtTarget, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::skills::Skills;
use crate::layer1::utility_types::ActionType;

/// System to detect and establish mentorship relationships.
/// Runs periodically to find Master-Apprentice pairs based on proximity and skill gap.
pub fn check_mentorship_system(
    mut commands: Commands,
    // Query potential apprentices AND masters
    pops: Query<(Entity, &GridPosition, &Skills, &MovementTarget), With<AtTarget>>,
    designations: Query<&Designation>,
) {
    // 1. Collect working pops and their SkillType
    let mut workers = Vec::new();
    for (entity, pos, skills, mt) in &pops {
        if mt.for_action != ActionType::Work {
            continue;
        }

        if let Ok(designation) = designations.get(mt.target_entity) {
            let skill_type = match designation.designation_type {
                DesignationType::Mine => Some(SkillType::Mining),
                DesignationType::Chop => Some(SkillType::Forestry),
                DesignationType::Repair
                | DesignationType::Demolish
                | DesignationType::JuryRig
                | DesignationType::Cannibalize
                | DesignationType::Destroy => Some(SkillType::Construction),
                DesignationType::ExcavateDust => Some(SkillType::Mining),
                DesignationType::ClearFlora | DesignationType::CollectSample => {
                    Some(SkillType::Farming)
                }
                DesignationType::SetZone(_) => None,
                DesignationType::Tame => Some(SkillType::Husbandry),
                DesignationType::Consume => None,
            };
            if let Some(st) = skill_type {
                workers.push((entity, *pos, skills, st));
            }
        }
    }

    // 2. Find pairs (O(N^2) for MVP is fine, N is small)
    for (i, (app_entity, app_pos, app_skills, app_skill_type)) in workers.iter().enumerate() {
        let app_level = app_skills.get_level(*app_skill_type);

        // Find best master in range
        let mut best_master = None;

        for (j, (master_entity, master_pos, master_skills, master_skill_type)) in
            workers.iter().enumerate()
        {
            if i == j {
                continue;
            }

            if app_skill_type != master_skill_type {
                continue;
            }

            let dist = app_pos
                .x
                .abs_diff(master_pos.x)
                .saturating_add(app_pos.y.abs_diff(master_pos.y))
                .min(i32::MAX as u32) as i32;
            if dist > 5 {
                continue;
            }

            let master_level = master_skills.get_level(*master_skill_type);
            if master_level >= app_level + 2 {
                // Found a valid master
                best_master = Some(*master_entity);
                break; // Take first found for MVP
            }
        }

        if let Some(master) = best_master {
            commands.entity(*app_entity).insert(Mentorship {
                master_entity: master,
                skill: *app_skill_type,
                multiplier: 1.5, // 50% bonus
                expiration: 50,  // Re-check every 50 ticks
            });
        }
    }
}

/// System to apply passive XP gain to apprentices and handle expiration.
pub fn apply_mentorship_xp_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Skills, &mut Mentorship)>,
) {
    for (entity, mut skills, mut mentorship) in &mut query {
        // Add small trickle XP per tick representing "learning by watching"
        // Base work gives 1.0 XP per tick.
        // Multiplier 1.5x means we should add 0.5 extra XP per tick.

        let bonus = 0.5; // Fixed bonus for now
        skills.add_xp(mentorship.skill, bonus);

        // Decay expiration
        if mentorship.expiration > 0 {
            mentorship.expiration -= 1;
        } else {
            commands.entity(entity).remove::<Mentorship>();
        }
    }
}

/// System to apply a small mood/leisure buff to both Master and Apprentice
/// while they are engaged in a Mentorship relationship.
pub fn mentorship_mood_system(
    mentorship_query: Query<(Entity, &Mentorship)>,
    mut needs_query: Query<&mut crate::layer1::needs::Needs>,
    mut master_set: Local<bevy_utils::HashSet<Entity>>,
) {
    let mood_buff = 0.5; // From spec: MOOD_BUFF: f32 = 0.5;

    master_set.clear();

    // Pass 1: Apply to apprentices and collect unique masters
    for (apprentice_entity, mentorship) in mentorship_query.iter() {
        if let Ok(mut needs) = needs_query.get_mut(apprentice_entity) {
            needs.leisure += mood_buff;
        }
        master_set.insert(mentorship.master_entity);
    }

    // Pass 2: Apply to unique masters (prevents buff stacking if master has multiple apprentices)
    for master_entity in master_set.drain() {
        if let Ok(mut needs) = needs_query.get_mut(master_entity) {
            needs.leisure += mood_buff;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_mentorship_xp_system, check_mentorship_system, Mentorship};
    use crate::layer1::designation::{Designation, DesignationType};
    use crate::layer1::execution::{AtTarget, MovementTarget};
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::utility_types::ActionType;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_mentorship_detection_valid_pair() {
        let mut world = World::new();

        // Spawn Master (Mining Lvl 5) working on a Mine designation
        let mut master_skills = Skills::default();
        master_skills.add_xp(SkillType::Mining, 2500.0); // Level 5
        let master_designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let master = world
            .spawn((
                Pop,
                GridPosition { x: 4, y: 5 }, // Adjacent to designation
                master_skills,
                MovementTarget {
                    target_entity: master_designation,
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        // Spawn Apprentice (Mining Lvl 0) working on a Mine designation nearby
        let apprentice_designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();
        let apprentice = world
            .spawn((
                Pop,
                GridPosition { x: 7, y: 5 }, // Within 5 tiles of Master
                Skills::default(),           // Level 0
                MovementTarget {
                    target_entity: apprentice_designation,
                    target_position: GridPosition { x: 6, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        // Assert Apprentice has Mentorship component pointing to Master
        let mentorship = world.get::<Mentorship>(apprentice);
        assert!(
            mentorship.is_some(),
            "Apprentice should have Mentorship component"
        );
        let mentorship = mentorship.unwrap();
        assert_eq!(mentorship.master_entity, master);
        assert_eq!(mentorship.skill, SkillType::Mining);
    }

    #[test]
    fn test_mentorship_detection_ignores_too_far() {
        let mut world = World::new();

        // Master at (0,0)
        let mut master_skills = Skills::default();
        master_skills.add_xp(SkillType::Mining, 2500.0);
        let master_des = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.spawn((
            Pop,
            GridPosition { x: 1, y: 0 },
            master_skills,
            MovementTarget {
                target_entity: master_des,
                target_position: GridPosition { x: 0, y: 0 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        // Apprentice at (10,0) - Too far
        let app_des = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 10, y: 0 },
            ))
            .id();
        let apprentice = world
            .spawn((
                Pop,
                GridPosition { x: 9, y: 0 },
                Skills::default(),
                MovementTarget {
                    target_entity: app_des,
                    target_position: GridPosition { x: 10, y: 0 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        assert!(
            world.get::<Mentorship>(apprentice).is_none(),
            "Too far for mentorship"
        );
    }

    #[test]
    fn test_mentorship_detection_ignores_small_level_gap() {
        let mut world = World::new();

        // Master Level 1 (100 XP)
        let mut master_skills = Skills::default();
        master_skills.add_xp(SkillType::Mining, 150.0);
        let master_des = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        world.spawn((
            Pop,
            GridPosition { x: 4, y: 5 },
            master_skills,
            MovementTarget {
                target_entity: master_des,
                target_position: GridPosition { x: 5, y: 5 },
                for_action: ActionType::Work,
            },
            AtTarget,
        ));

        // Apprentice Level 0
        let app_des = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();
        let apprentice = world
            .spawn((
                Pop,
                GridPosition { x: 7, y: 5 },
                Skills::default(),
                MovementTarget {
                    target_entity: app_des,
                    target_position: GridPosition { x: 6, y: 5 },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        // Gap is only 1 level (needs >= 2)
        assert!(
            world.get::<Mentorship>(apprentice).is_none(),
            "Level gap too small"
        );
    }

    #[test]
    fn test_mentorship_apply_xp_bonus() {
        let mut world = World::new();

        // Apprentice with Mentorship active
        let apprentice = world
            .spawn((
                Pop,
                Skills::default(),
                Mentorship {
                    master_entity: Entity::from_raw(999),
                    skill: SkillType::Mining,
                    multiplier: 1.5,
                    expiration: 10,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(apply_mentorship_xp_system);
        schedule.run(&mut world);

        let skills = world.get::<Skills>(apprentice).unwrap();
        assert!(
            skills.get_xp(SkillType::Mining) > 0.0,
            "Should gain bonus XP from mentorship"
        );
    }

    #[test]
    fn test_mentorship_expiration() {
        let mut world = World::new();

        // Apprentice with Mentorship expiring soon
        let apprentice = world
            .spawn((
                Pop,
                Skills::default(),
                Mentorship {
                    master_entity: Entity::from_raw(999),
                    skill: SkillType::Mining,
                    multiplier: 1.5,
                    expiration: 1, // Will expire next tick
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(apply_mentorship_xp_system);

        // Run once: expiration decr to 0
        schedule.run(&mut world);

        assert!(world.get::<Mentorship>(apprentice).is_some());
        let m = world.get::<Mentorship>(apprentice).unwrap();
        assert_eq!(m.expiration, 0);

        // Run twice: removed
        schedule.run(&mut world);

        assert!(world.get::<Mentorship>(apprentice).is_none());
    }

    #[test]
    fn test_mentorship_grants_mood_buffs() {
        let mut world = World::new();

        let master = world
            .spawn((
                Pop,
                Skills::default(),
                crate::layer1::needs::Needs {
                    hunger: 0.8,
                    rest: 0.8,
                    leisure: 0.5,
                    hygiene: 0.8,
                },
            ))
            .id();

        let apprentice = world
            .spawn((
                Pop,
                Skills::default(),
                crate::layer1::needs::Needs {
                    hunger: 0.8,
                    rest: 0.8,
                    leisure: 0.5,
                    hygiene: 0.8,
                },
                Mentorship {
                    master_entity: master,
                    skill: SkillType::Mining,
                    multiplier: 1.5,
                    expiration: 10,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(super::mentorship_mood_system);
        schedule.run(&mut world);

        // Apprentice should gain leisure/mood buff from learning
        let app_needs = world
            .get::<crate::layer1::needs::Needs>(apprentice)
            .unwrap();
        assert!(
            app_needs.leisure > 0.5,
            "Apprentice should gain mood buff from learning."
        );

        // Master should gain leisure/mood buff from teaching
        let master_needs = world.get::<crate::layer1::needs::Needs>(master).unwrap();
        assert!(
            master_needs.leisure > 0.5,
            "Master should gain mood buff from teaching."
        );
    }
}
