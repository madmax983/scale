#![allow(clippy::float_cmp)]
//! The heartbeat of progress. The Skills module tracks the accumulated experience and competence of Pops, transforming raw recruits into legendary artisans.

pub mod generational_atrophy {
    //! Generational Atrophy.
    //!
    //! Explores how high automation affects the physical skills of newer generations.
    //! When automation exceeds a certain threshold, physical skills like Farming and Mining
    //! begin to atrophy in immigrants and newer generations who rely on the automated systems.

    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::social::old_guard::Generation;
    use bevy_ecs::prelude::*;

    /// Resource tracking the colony's overall level of automation.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::skills::generational_atrophy::AutomationLevel;
    ///
    /// let automation = AutomationLevel { level: 85.0 };
    /// assert!(automation.level > 50.0); // High automation
    /// ```
    #[derive(Resource, Default)]
    pub struct AutomationLevel {
        pub level: f32, // 0.0 to 100.0
    }

    /// Marker component added to Pops whose skills have been reduced by automation.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use bevy_ecs::prelude::*;
    /// use scale::layer1::skills::generational_atrophy::AtrophiedSkill;
    ///
    /// let mut world = World::new();
    /// let entity = world.spawn(AtrophiedSkill).id();
    /// assert!(world.get::<AtrophiedSkill>(entity).is_some());
    /// ```
    #[derive(Component)]
    pub struct AtrophiedSkill;

    /// System that applies skill atrophy to newer generations in highly automated colonies.
    ///
    /// If [`AutomationLevel`] is > 50.0, physical skills (Farming, Mining) for `Immigrant` Pops are halved.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use bevy_ecs::prelude::*;
    /// use scale::layer1::skills::{Skills, SkillType};
    /// use scale::layer1::social::old_guard::Generation;
    /// use scale::layer1::skills::generational_atrophy::{AutomationLevel, AtrophiedSkill, apply_skill_atrophy_system};
    ///
    /// let mut world = World::new();
    /// world.insert_resource(AutomationLevel { level: 80.0 });
    ///
    /// let mut skills = Skills::default();
    /// skills.add_xp(SkillType::Farming, 100.0);
    ///
    /// let pop = world.spawn((Generation::Immigrant, skills)).id();
    ///
    /// let mut schedule = Schedule::default();
    /// schedule.add_systems(apply_skill_atrophy_system);
    /// schedule.run(&mut world);
    ///
    /// assert!(world.get::<AtrophiedSkill>(pop).is_some());
    /// let updated_skills = world.get::<Skills>(pop).unwrap();
    /// assert_eq!(updated_skills.get_xp(SkillType::Farming), 50.0); // XP was halved
    /// ```
    pub fn apply_skill_atrophy_system(
        automation_level: Option<Res<AutomationLevel>>,
        mut query: Query<(Entity, &Generation, &mut Skills), Without<AtrophiedSkill>>,
        mut commands: Commands,
    ) {
        let automation = match automation_level {
            Some(res) => res.level,
            None => return,
        };

        // If automation is high, apply atrophy to physical skills of newer generations
        if automation > 50.0 {
            for (entity, generation, mut skills) in query.iter_mut() {
                if *generation == Generation::Immigrant {
                    // Applies to later generations
                    // Manually reduce the XP for physical skills
                    let current_farming = *skills.xp.get(&SkillType::Farming).unwrap_or(&0.0);
                    if current_farming > 0.0 {
                        if let Some(xp) = skills.xp.get_mut(&SkillType::Farming) {
                            *xp *= 0.5; // 50% penalty
                        }
                    }

                    let current_mining = *skills.xp.get(&SkillType::Mining).unwrap_or(&0.0);
                    if current_mining > 0.0 {
                        if let Some(xp) = skills.xp.get_mut(&SkillType::Mining) {
                            *xp *= 0.5;
                        }
                    }

                    commands.entity(entity).insert(AtrophiedSkill);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use crate::layer1::skills::generational_atrophy::{
            apply_skill_atrophy_system, AtrophiedSkill, AutomationLevel,
        };
        use crate::layer1::skills::{SkillType, Skills};
        use crate::layer1::social::old_guard::Generation;
        use bevy_ecs::prelude::*;

        #[test]
        fn test_high_automation_causes_skill_atrophy_in_new_generation() {
            let mut world = World::new();
            // Set a high automation level for the colony
            world.insert_resource(AutomationLevel { level: 80.0 }); // High automation

            // Spawn a Pop of a new generation with some starting skills
            let mut skills = Skills::default();
            skills.add_xp(SkillType::Farming, 500.0); // Would normally be level 2

            let pop = world
                .spawn((
                    Generation::Immigrant, // "New" generation
                    skills,
                ))
                .id();

            // Run the system
            let mut schedule = Schedule::default();
            schedule.add_systems(apply_skill_atrophy_system);
            schedule.run(&mut world);

            // Verify that the skill has been atrophied and a marker added
            let atrophied = world.get::<AtrophiedSkill>(pop);
            assert!(atrophied.is_some(), "Pop should have AtrophiedSkill marker");

            let updated_skills = world.get::<Skills>(pop).unwrap();
            // Since get_xp does not expose a mutable reference to reduce it, the system reduces the underlying xp map
            assert!(
                *updated_skills.xp.get(&SkillType::Farming).unwrap_or(&0.0) < 500.0,
                "Skill XP should be reduced due to atrophy"
            );
        }

        #[test]
        fn test_low_automation_does_not_cause_atrophy() {
            let mut world = World::new();
            // Low automation
            world.insert_resource(AutomationLevel { level: 10.0 });

            let mut skills = Skills::default();
            skills.add_xp(SkillType::Farming, 500.0);

            let pop = world.spawn((Generation::Immigrant, skills)).id();

            let mut schedule = Schedule::default();
            schedule.add_systems(apply_skill_atrophy_system);
            schedule.run(&mut world);

            assert!(
                world.get::<AtrophiedSkill>(pop).is_none(),
                "Pop should not have AtrophiedSkill marker in low automation"
            );
        }
    }
}
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// The various disciplines a Pop can master.
///
/// Each skill governs efficiency and unlockable capabilities within its respective domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillType {
    /// Mining rock.
    Mining,
    /// Chopping trees.
    Forestry,
    /// Producing food/fiber.
    Farming,
    /// Building and repairing structures.
    Construction,
    /// Refining resources.
    Crafting,
    /// Taming and caring for animals.
    Husbandry,
    /// Tinkering, maintaining, and upgrading machinery.
    Engineering,
    /// Scientific research and study.
    Science,
}

/// Component storing experience points for various skills.
#[derive(Component, Default, Clone, Debug)]
pub struct Skills {
    /// Map of skill type to accumulated XP.
    pub xp: HashMap<SkillType, f32>,
}

/// Event triggered when a Pop gains XP.
#[derive(Event, Debug, Clone)]
pub struct XpGainEvent {
    /// The entity gaining XP.
    pub entity: Entity,
    /// The skill type.
    pub skill: SkillType,
    /// The amount gained.
    pub amount: f32,
    /// The source of the XP gain (to prevent loops).
    pub source: XpSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XpSource {
    /// Standard action (mining, farming, etc.).
    Action,
    /// Quantum entanglement sharing.
    Entanglement,
}

impl Skills {
    /// Adds XP to a specific skill.
    ///
    /// Note: This method does NOT emit [`XpGainEvent`] automatically to avoid
    /// circular dependencies with `EventWriter`. Systems should emit the event manually
    /// if they want to trigger side effects (like Quantum Twins).
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::skills::{Skills, SkillType};
    ///
    /// let mut skills = Skills::default();
    /// skills.add_xp(SkillType::Mining, 150.0);
    ///
    /// // 150 XP means they reached level 1
    /// assert_eq!(skills.get_level(SkillType::Mining), 1);
    /// ```
    pub fn add_xp(&mut self, skill: SkillType, amount: f32) {
        let current = self.xp.entry(skill).or_insert(0.0);
        *current += amount;
    }

    /// Returns the current XP for a skill.
    #[must_use]
    pub fn get_xp(&self, skill: SkillType) -> f32 {
        *self.xp.get(&skill).unwrap_or(&0.0)
    }

    /// Calculates the level based on XP.
    /// Formula: Level = floor(sqrt(XP / 100))
    /// 0-99 XP = Lvl 0
    /// 100-399 XP = Lvl 1
    /// 400-899 XP = Lvl 2
    #[must_use]
    pub fn get_level(&self, skill: SkillType) -> u32 {
        let xp = self.get_xp(skill);
        // Using f32::sqrt requires the input to be non-negative, which XP is.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let level = (xp / 100.0).sqrt().floor() as u32;
        level
    }

    /// Calculates efficiency multiplier based on skill level.
    /// Base 1.0 + 0.1 per level.
    #[must_use]
    pub fn get_efficiency(&self, skill: SkillType) -> f32 {
        let level = self.get_level(skill);
        #[allow(clippy::cast_precision_loss)]
        let level_f32 = level as f32;
        // Optimization: Use mul_add if appropriate, though simplistic here
        level_f32.mul_add(0.1, 1.0)
    }
}

/// Temporary XP-gain multiplier on a pop (Spec 418, spec section 7).
///
/// Derived, never accumulated: the wisdom aura system in
/// `crate::layer1::lifecycle` recomputes `value` from scratch every
/// tick (1.0 = no aura), so the buff appears and expires with elder
/// proximity exactly like the spec sketch reset-to-1.0 contract.
/// XP-granting work lanes read it via [`XpMultiplier::apply`]
/// (designation work and farm production); lanes that do not read it
/// yet simply grant base XP.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct XpMultiplier {
    /// Effective multiplier (1.0 = unbuffed).
    pub value: f32,
}

impl Default for XpMultiplier {
    fn default() -> Self {
        Self { value: 1.0 }
    }
}

impl XpMultiplier {
    /// Scales a base XP amount by this multiplier.
    #[must_use]
    pub fn apply(&self, amount: f32) -> f32 {
        amount * self.value
    }
}

/// Helper to get skill efficiency from an optional Skills component.
/// Returns 1.0 if no component exists.
#[must_use]
pub fn get_skill_efficiency(skills: Option<&Skills>, skill: SkillType) -> f32 {
    skills.map_or(1.0, |s| s.get_efficiency(skill))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skills_default_empty() {
        let skills = Skills::default();
        assert_eq!(skills.get_xp(SkillType::Mining), 0.0);
        assert_eq!(skills.get_level(SkillType::Mining), 0);
    }

    #[test]
    fn test_gain_xp_increases_level() {
        let mut skills = Skills::default();

        // Level 0 -> 1 requires 100 XP
        skills.add_xp(SkillType::Mining, 50.0);
        assert_eq!(skills.get_level(SkillType::Mining), 0);

        skills.add_xp(SkillType::Mining, 50.0);
        assert_eq!(skills.get_level(SkillType::Mining), 1);
    }

    #[test]
    fn test_efficiency_scaling() {
        let mut skills = Skills::default();

        // Level 0 efficiency = 1.0
        assert!((skills.get_efficiency(SkillType::Mining) - 1.0).abs() < f32::EPSILON);

        // Level 1 efficiency = 1.1 (10% bonus)
        skills.add_xp(SkillType::Mining, 100.0);
        assert!((skills.get_efficiency(SkillType::Mining) - 1.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_skill_types_exist() {
        let _ = SkillType::Mining;
        let _ = SkillType::Forestry;
        let _ = SkillType::Farming;
        let _ = SkillType::Construction;
        let _ = SkillType::Crafting;
        let _ = SkillType::Science;
    }
}
