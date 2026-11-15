//! Pop lifecycle and aging system.
//!
//! Handles aging of Pops, life stage transitions (Child -> Adult -> Elder),
//! and natural death due to old age.

use crate::layer1::balance::{AGE_ADULT, AGE_ELDER, TICKS_PER_YEAR};
use crate::layer1::designation::DesignationType;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Speed;
use crate::layer1::skills::{Skills, XpMultiplier};
use crate::layer1::utility_types::AssignmentType;

pub use crate::layer1::administration::edicts::ELDER_RATIONS_LABEL;

/// Spec 418: Chebyshev radius of an elder Wisdom aura (matches the
/// manual aura radius-5 neighbourhood in `institutional_memory`).
pub const WISDOM_AURA_RADIUS: u32 = 5;
/// Spec 418: Wisdom bonus per accumulated skill level across all of
/// the elder skills (section 5: the aura scales with a lifetime of
/// skill, it is not the sketch flat +0.2).
pub const WISDOM_PER_SKILL_LEVEL: f32 = 0.02;
/// Spec 418: hard cap on the Wisdom bonus (+50% XP at most).
pub const WISDOM_MAX_BONUS: f32 = 0.50;

/// Total skill levels accumulated across every skill a pop has XP in.
#[must_use]
pub fn total_skill_levels(skills: &Skills) -> u32 {
    skills.xp.keys().map(|s| skills.get_level(*s)).sum()
}

/// Spec 418 section 5: the Wisdom bonus an elder radiates, scaled by
/// their accumulated skill levels and capped at [`WISDOM_MAX_BONUS`].
/// A skill-less elder radiates nothing.
#[must_use]
pub fn wisdom_bonus_for(skills: &Skills) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let raw = total_skill_levels(skills) as f32 * WISDOM_PER_SKILL_LEVEL;
    raw.min(WISDOM_MAX_BONUS)
}

/// Designations that count as heavy labor for Spec 418 job validation:
/// mining, felling, demolition and destructive extraction. Everything
/// else (Repair, JuryRig, ClearFlora, CollectSample, light errands) is
/// light work an elder may still do.
#[must_use]
pub const fn is_heavy_designation(kind: DesignationType) -> bool {
    matches!(
        kind,
        DesignationType::Mine
            | DesignationType::Chop
            | DesignationType::Demolish
            | DesignationType::Destroy
            | DesignationType::Cannibalize
            | DesignationType::ExcavateDust
    )
}

/// Spec 418 section 5 job validation, adapted to the real types (the
/// sketch JobType Mining/Teaching/Socializing variants do not exist;
/// the real work lane is `DesignationType`, and socializing is an
/// action the utility AI never gates). Elders refuse heavy labor;
/// every other stage may perform every designation.
#[must_use]
pub const fn elder_can_perform(stage: LifeStage, kind: DesignationType) -> bool {
    !matches!(stage, LifeStage::Elder) || !is_heavy_designation(kind)
}

/// Spec 418 section 5 assignment validation: elders refuse the one
/// heavy assignment in the real `AssignmentType` enum (DeepMining);
/// light postings such as LibraryWorker stay open to them.
#[must_use]
pub const fn elder_can_take_assignment(stage: LifeStage, kind: AssignmentType) -> bool {
    !matches!(stage, LifeStage::Elder) || !matches!(kind, AssignmentType::DeepMining)
}

/// Spec 418 Wisdom aura: recomputes every worker XpMultiplier from
/// scratch — 1.0 plus the strongest bonus of any Elder within
/// [`WISDOM_AURA_RADIUS`] (Chebyshev, on the real `GridPosition`
/// lane; pops have no Transform). Elders never buff each other.
/// Derived state: moving away removes the buff on the next tick.
pub fn wisdom_aura_system(
    elders: Query<(&GridPosition, &Skills, &Age)>,
    mut workers: Query<(&GridPosition, &mut XpMultiplier, Option<&Age>)>,
) {
    let aura: Vec<(GridPosition, f32)> = elders
        .iter()
        .filter(|(_, _, age)| age.stage == LifeStage::Elder)
        .map(|(pos, skills, _)| (*pos, wisdom_bonus_for(skills)))
        .filter(|(_, bonus)| *bonus > 0.0)
        .collect();
    if aura.is_empty() {
        for (_, mut mult, _) in &mut workers {
            mult.value = 1.0;
        }
        return;
    }
    for (pos, mut mult, age) in &mut workers {
        if age.is_some_and(|a| a.stage == LifeStage::Elder) {
            mult.value = 1.0;
            continue;
        }
        let mut best = 0.0_f32;
        for (elder_pos, bonus) in &aura {
            if pos.distance_chebyshev(*elder_pos) <= WISDOM_AURA_RADIUS {
                best = best.max(*bonus);
            }
        }
        mult.value = 1.0 + best;
    }
}

/// Spec 418 census for STATS / the `elders` headless panel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleCensus {
    /// Pops in the Child stage.
    pub children: usize,
    /// Pops in the Adult stage.
    pub adults: usize,
    /// Pops in the Elder stage.
    pub elders: usize,
    /// Non-elder pops currently carrying a Wisdom buff (> 1.0).
    pub buffed_workers: usize,
}

/// Counts life stages and Wisdom-buffed workers across all pops.
#[must_use]
pub fn lifecycle_census(world: &mut World) -> LifecycleCensus {
    let mut census = LifecycleCensus::default();
    let mut query = world.query::<(&Age, Option<&XpMultiplier>)>();
    for (age, mult) in query.iter(world) {
        match age.stage {
            LifeStage::Child => census.children += 1,
            LifeStage::Adult => census.adults += 1,
            LifeStage::Elder => census.elders += 1,
        }
        if age.stage != LifeStage::Elder && mult.is_some_and(|m| m.value > 1.0) {
            census.buffed_workers += 1;
        }
    }
    census
}
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use rand::Rng;

/// Life stages of a Pop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LifeStage {
    /// Initial stage.
    #[default]
    Child,
    /// Productive stage (18+ years).
    Adult,
    /// Senior stage (60+ years), with potential penalties.
    Elder,
}

/// Tracks the age and life stage of a Pop.
#[derive(Component, Debug, Clone, Default)]
pub struct Age {
    /// Total ticks the Pop has been alive.
    pub ticks_alive: u64,
    /// Current life stage.
    pub stage: LifeStage,
}

impl Age {
    /// Creates a new Age component from years.
    #[must_use]
    pub const fn new(years: u64) -> Self {
        let ticks = years * TICKS_PER_YEAR;
        let stage = if ticks < AGE_ADULT {
            LifeStage::Child
        } else if ticks < AGE_ELDER {
            LifeStage::Adult
        } else {
            LifeStage::Elder
        };
        Self {
            ticks_alive: ticks,
            stage,
        }
    }
}

#[derive(QueryData)]
#[query_data(mutable)]
pub struct AgingQuery {
    entity: Entity,
    age: &'static mut Age,
    speed: Option<&'static mut Speed>,
    inside_chamber: Option<&'static mut crate::layer1::temporal_chamber::InsideChamber>,
}

/// System to increment age and handle life stage transitions.
pub fn aging_system(
    mut query: Query<AgingQuery, Without<crate::layer1::cryo::CryoStasis>>,
    chambers: Query<&crate::layer1::temporal_chamber::TemporalChamber>,
    mut log: Option<ResMut<MessageLog>>,
) {
    for mut item in &mut query {
        let mut ticks_to_add = 1;

        if let Some(ref mut inside) = item.inside_chamber {
            let mut time_factor = 1.0;
            if let Ok(chamber) = chambers.get(inside.chamber_entity) {
                if chamber.active {
                    time_factor = chamber.time_dilation_factor;
                }
            }

            inside.fractional_age += time_factor;
            if inside.fractional_age >= 1.0 {
                ticks_to_add = inside.fractional_age.floor() as u64;
                inside.fractional_age = inside.fractional_age.fract();
            } else {
                ticks_to_add = 0;
            }
        }

        if ticks_to_add == 0 {
            continue;
        }

        item.age.ticks_alive += ticks_to_add;

        let new_stage = if item.age.ticks_alive >= AGE_ELDER {
            LifeStage::Elder
        } else if item.age.ticks_alive >= AGE_ADULT {
            LifeStage::Adult
        } else {
            LifeStage::Child
        };

        if new_stage != item.age.stage {
            // Apply transition effects
            if new_stage == LifeStage::Elder {
                if let Some(ref mut s) = item.speed {
                    s.base *= 0.8; // 20% slow down
                }
                if let Some(l) = log.as_mut() {
                    l.add("A colonist has become an Elder.");
                }
            }
            item.age.stage = new_stage;
        }
    }
}

/// System to handle natural death from old age.
///
/// Probability of death increases with age for Elders.
pub fn natural_death_system(
    mut commands: Commands,
    mut query: Query<(Entity, &Age, &mut crate::layer1::health::Health)>,
) {
    let mut rng = rand::thread_rng();

    for (entity, age, mut health) in &mut query {
        if age.stage == LifeStage::Elder {
            #[allow(clippy::cast_precision_loss)]
            let years = age.ticks_alive as f64 / TICKS_PER_YEAR as f64;

            // Only start applying chance after 60 years.
            if years > 60.0 {
                // Chance per tick.
                // Example: At 80 years (20 years past 60), chance is 0.0002 per tick.
                // Over 1000 ticks (1 year), chance is ~18%.
                let chance = (years - 60.0) * 0.00001;

                if rng.gen_bool(chance.clamp(0.0, 1.0)) {
                    health.current = 0.0; // Die
                    commands
                        .entity(entity)
                        .insert(crate::layer1::health::DeathCause("Old Age".to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::balance::{AGE_ADULT, AGE_ELDER, TICKS_PER_YEAR};
    use crate::layer1::health::Health;
    use crate::layer1::pop::{Pop, Speed};
    use crate::shared::time::SimulationTime;

    #[test]
    fn test_pop_has_age_component() {
        let mut world = World::new();
        let entity = world.spawn((Pop, Age::default())).id();

        let age = world.get::<Age>(entity);
        assert!(age.is_some());
        assert_eq!(age.unwrap().ticks_alive, 0);
    }

    #[test]
    fn test_aging_system_increments_age() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(aging_system);

        let entity = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: 100,
                    ..Default::default()
                },
                Speed::default(),
            ))
            .id();

        schedule.run(&mut world);

        let age = world.get::<Age>(entity).unwrap();
        assert_eq!(age.ticks_alive, 101);
    }

    #[test]
    fn test_chamber_preserves_pops_by_reducing_aging() {
        use crate::layer1::temporal_chamber::{InsideChamber, TemporalChamber};

        // Arrange
        let mut world = World::new();

        let mut schedule = Schedule::default();
        schedule.add_systems(aging_system);

        let chamber_id = world
            .spawn(TemporalChamber {
                time_dilation_factor: 0.1,
                active: true,
                energy_cost: 10.0,
                ticks_active: 0,
            })
            .id();

        let pop = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: 0,
                    ..Default::default()
                },
                InsideChamber {
                    chamber_entity: chamber_id,
                    fractional_age: 0.0,
                },
            ))
            .id();

        // Act
        schedule.run(&mut world);

        // Assert
        let pop_age = world.get::<Age>(pop).unwrap();
        // Normal aging would add 1.0, chamber adds 0.1
        assert_eq!(pop_age.ticks_alive, 0); // Need fractional tracking or tick accumulation

        // Run 9 more times to reach 1.0
        for _ in 0..9 {
            schedule.run(&mut world);
        }

        let pop_age = world.get::<Age>(pop).unwrap();
        assert_eq!(pop_age.ticks_alive, 1);
    }

    #[test]
    fn test_lifestage_transitions() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(aging_system);

        // 1. Child -> Adult
        let child = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: AGE_ADULT - 1,
                    stage: LifeStage::Child,
                },
                Speed::default(),
            ))
            .id();

        // 2. Adult -> Elder
        let adult = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: AGE_ELDER - 1,
                    stage: LifeStage::Adult,
                },
                Speed::default(),
            ))
            .id();

        schedule.run(&mut world);

        let child_age = world.get::<Age>(child).unwrap();
        assert_eq!(child_age.stage, LifeStage::Adult);

        let adult_age = world.get::<Age>(adult).unwrap();
        assert_eq!(adult_age.stage, LifeStage::Elder);
    }

    #[test]
    fn test_elder_speed_penalty() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(aging_system);

        // Spawn a pop about to become Elder
        let entity = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: AGE_ELDER - 1,
                    stage: LifeStage::Adult,
                },
                Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        schedule.run(&mut world);

        let speed = world.get::<Speed>(entity).unwrap();
        // Expect penalty (e.g. 0.8x)
        assert!(speed.base < 1.0);
        assert!((speed.base - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_natural_death_chance() {
        // Probabilistic test: excessive age should eventually kill
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(natural_death_system);

        let entity = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: 120 * TICKS_PER_YEAR, // Very old
                    stage: LifeStage::Elder,
                },
                Health {
                    current: 10.0,
                    max: 10.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        // Run many times to trigger probability.
        // With 0.00001 base chance * (120-60) = 0.0006 per tick.
        // 10,000 ticks gives ~99.7% chance of death.
        let mut died = false;
        for _ in 0..20_000 {
            schedule.run(&mut world);
            let health = world.get::<Health>(entity).unwrap();
            if !health.is_alive() {
                died = true;
                break;
            }
        }

        assert!(died, "Very old pop should eventually die naturally");
    }

    #[test]
    fn test_aging_system_without_speed() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(aging_system);

        let entity = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: 100,
                    ..Default::default()
                },
                // No Speed component
            ))
            .id();

        schedule.run(&mut world);

        let age = world.get::<Age>(entity).unwrap();
        assert_eq!(age.ticks_alive, 101);
    }
}

/// Spec 418 RED-phase tests, adapted to the real codebase (Spec 062
/// already provides Age/LifeStage/aging_system; these tests pin the
/// missing legs: wisdom aura, elder job validation, elder rationing).
#[cfg(test)]
mod spec_418_tests {
    use super::*;
    use crate::layer1::administration::edicts::Policy;
    use crate::layer1::designation::DesignationType;
    use crate::layer1::map::GridPosition;
    use crate::layer1::skills::{SkillType, Skills, XpMultiplier};
    use crate::layer1::utility_types::AssignmentType;

    use crate::layer1::pop::Pop;
    macro_rules! run {
        ($world:expr, $sys:expr) => {{
            let mut schedule = Schedule::default();
            schedule.add_systems($sys);
            schedule.run($world);
        }};
    }

    // Spec RED test 1 (adapted): pops age continuously. Real Age is
    // tick-based (TICKS_PER_YEAR per year), not the sketch float years.
    #[test]
    fn spec418_pop_ages_over_time() {
        let mut world = World::new();
        let pop = world.spawn((Pop, Age::new(20))).id();
        run!(&mut world, aging_system);
        let age = world.get::<Age>(pop).unwrap();
        assert_eq!(age.ticks_alive, 20 * TICKS_PER_YEAR + 1);
    }

    // Spec RED test 2 (adapted): a 65-year-old is an Elder. The real
    // threshold is AGE_ELDER = 60 years (Spec 062 balance, not retuned).
    #[test]
    fn spec418_pop_becomes_elder() {
        assert_eq!(Age::new(65).stage, LifeStage::Elder);
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Age {
                    ticks_alive: AGE_ELDER - 1,
                    stage: LifeStage::Adult,
                },
                Speed::default(),
            ))
            .id();
        run!(&mut world, aging_system);
        assert_eq!(world.get::<Age>(pop).unwrap().stage, LifeStage::Elder);
        // Elder speed penalty rides the real Speed pipeline (0.8x base).
        assert!((world.get::<Speed>(pop).unwrap().base - 0.8).abs() < f32::EPSILON);
    }

    fn elder_with_mining(level_xp: f32) -> Skills {
        let mut s = Skills::default();
        if level_xp > 0.0 {
            s.add_xp(SkillType::Mining, level_xp);
        }
        s
    }

    // Spec RED test 3 (adapted): nearby adult worker gains XpMultiplier.
    // Real pops locate via GridPosition, not Transform.
    #[test]
    fn spec418_elder_wisdom_aura_buffs_workers() {
        let mut world = World::new();
        world.spawn((
            Pop,
            Age::new(70),
            GridPosition { x: 0, y: 0 },
            elder_with_mining(2500.0), // Mining level 5
        ));
        let worker = world
            .spawn((
                Pop,
                Age::new(30),
                GridPosition { x: 2, y: 0 },
                Skills::default(),
                XpMultiplier::default(),
            ))
            .id();
        run!(&mut world, wisdom_aura_system);
        let mult = world.get::<XpMultiplier>(worker).unwrap();
        assert!(mult.value > 1.0, "worker near elder must be buffed");
        // 5 total levels * 0.02 = +0.10
        assert!((mult.value - 1.10).abs() < 0.001, "got {}", mult.value);
    }

    #[test]
    fn spec418_wisdom_aura_expires_when_far() {
        let mut world = World::new();
        world.spawn((
            Pop,
            Age::new(70),
            GridPosition { x: 0, y: 0 },
            elder_with_mining(2500.0),
        ));
        let worker = world
            .spawn((
                Pop,
                Age::new(30),
                GridPosition { x: 2, y: 0 },
                Skills::default(),
                XpMultiplier::default(),
            ))
            .id();
        run!(&mut world, wisdom_aura_system);
        assert!(world.get::<XpMultiplier>(worker).unwrap().value > 1.0);
        world.get_mut::<GridPosition>(worker).unwrap().x = 40;
        run!(&mut world, wisdom_aura_system);
        assert!((world.get::<XpMultiplier>(worker).unwrap().value - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn spec418_wisdom_scales_with_elder_skill_and_caps() {
        // Unskilled elder: no wisdom to share.
        assert!((wisdom_bonus_for(&Skills::default())).abs() < f32::EPSILON);
        // Master elder: Mining 10 + Crafting 10 = 20 levels -> +0.40.
        let mut master = Skills::default();
        master.add_xp(SkillType::Mining, 10_000.0);
        master.add_xp(SkillType::Crafting, 10_000.0);
        assert!((wisdom_bonus_for(&master) - 0.40).abs() < 0.001);
        // Cap: 80 levels of raw bonus (1.6) clamps to WISDOM_MAX_BONUS.
        let mut sage = Skills::default();
        for st in [
            SkillType::Mining,
            SkillType::Forestry,
            SkillType::Farming,
            SkillType::Construction,
            SkillType::Crafting,
            SkillType::Husbandry,
            SkillType::Engineering,
            SkillType::Science,
        ] {
            sage.add_xp(st, 10_000.0);
        }
        assert!((wisdom_bonus_for(&sage) - WISDOM_MAX_BONUS).abs() < f32::EPSILON);
    }

    #[test]
    fn spec418_elders_do_not_buff_elders() {
        let mut world = World::new();
        world.spawn((
            Pop,
            Age::new(70),
            GridPosition { x: 0, y: 0 },
            elder_with_mining(2500.0),
        ));
        let other_elder = world
            .spawn((
                Pop,
                Age::new(80),
                GridPosition { x: 1, y: 0 },
                Skills::default(),
                XpMultiplier::default(),
            ))
            .id();
        run!(&mut world, wisdom_aura_system);
        assert!((world.get::<XpMultiplier>(other_elder).unwrap().value - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn spec418_elder_job_validation() {
        // Heavy labor refused for elders, allowed for adults.
        assert!(!elder_can_perform(LifeStage::Elder, DesignationType::Mine));
        assert!(!elder_can_perform(LifeStage::Elder, DesignationType::Chop));
        assert!(!elder_can_perform(
            LifeStage::Elder,
            DesignationType::Demolish
        ));
        assert!(elder_can_perform(LifeStage::Adult, DesignationType::Mine));
        // Light work remains open to elders.
        assert!(elder_can_perform(LifeStage::Elder, DesignationType::Repair));
        assert!(elder_can_perform(
            LifeStage::Elder,
            DesignationType::ClearFlora
        ));
        // Assignments: deep mining refused, library work allowed.
        assert!(!elder_can_take_assignment(
            LifeStage::Elder,
            AssignmentType::DeepMining
        ));
        assert!(elder_can_take_assignment(
            LifeStage::Elder,
            AssignmentType::LibraryWorker
        ));
        assert!(elder_can_take_assignment(
            LifeStage::Adult,
            AssignmentType::DeepMining
        ));
    }

    #[test]
    fn spec418_census_counts_stages_and_buffed() {
        let mut world = World::new();
        world.spawn((
            Pop,
            Age::new(70),
            GridPosition { x: 0, y: 0 },
            elder_with_mining(2500.0),
        ));
        world.spawn((
            Pop,
            Age::new(30),
            GridPosition { x: 2, y: 0 },
            XpMultiplier { value: 1.1 },
        ));
        world.spawn((Pop, Age::new(10), GridPosition { x: 9, y: 9 }));
        let census = lifecycle_census(&mut world);
        assert_eq!(census.elders, 1);
        assert_eq!(census.adults, 1);
        assert_eq!(census.children, 1);
        assert_eq!(census.buffed_workers, 1);
    }

    #[test]
    fn spec418_ip_guard() {
        // Player-facing strings for this spec use only generic terms.
        let banned = ["geriatric", "senile", "boomer", "wrinkly"];
        for s in [ELDER_RATIONS_LABEL] {
            for b in banned {
                assert!(
                    !s.to_lowercase().contains(b),
                    "{s} contains banned term {b}"
                );
            }
        }
        assert!(ELDER_RATIONS_LABEL.contains("Elder"));
        let _ = Policy::ElderHalfRations;
    }
    #[test]
    fn spec418_elder_rations_halve_elder_hunger_decay_only() {
        crate::setup::init_task_pools();
        use crate::layer1::administration::edicts::ColonyPolicies;
        use crate::layer1::needs::{decay_needs_system, Needs};
        let mut world = World::new();
        let mut policies = ColonyPolicies::default();
        policies.enable(Policy::ElderHalfRations);
        world.insert_resource(policies);
        let elder = world.spawn((Pop, Age::new(70), Needs::default())).id();
        let adult = world.spawn((Pop, Age::new(30), Needs::default())).id();
        run!(&mut world, decay_needs_system);
        let elder_hunger = world.get::<Needs>(elder).unwrap().hunger;
        let adult_hunger = world.get::<Needs>(adult).unwrap().hunger;
        // Adult decays the full 0.001; elder decays half under the edict.
        assert!(
            (adult_hunger - 0.799).abs() < 0.0001,
            "adult {adult_hunger}"
        );
        assert!(
            (elder_hunger - 0.7995).abs() < 0.0001,
            "elder {elder_hunger}"
        );
    }

    #[test]
    fn spec418_elder_rations_morale_scar_is_elder_only_and_reversible() {
        use crate::layer1::administration::edicts::{apply_edict_modifiers, ColonyPolicies};
        use crate::layer1::social::morale::Morale;
        let mut world = World::new();
        let mut policies = ColonyPolicies::default();
        policies.enable(Policy::ElderHalfRations);
        world.insert_resource(policies);
        let elder = world.spawn((Pop, Age::new(70), Morale::default())).id();
        let adult = world.spawn((Pop, Age::new(30), Morale::default())).id();
        run!(&mut world, apply_edict_modifiers);
        let elder_morale = world.get::<Morale>(elder).unwrap();
        assert!(elder_morale
            .modifiers
            .iter()
            .any(|m| m.label == ELDER_RATIONS_LABEL && m.value < 0.0));
        let adult_morale = world.get::<Morale>(adult).unwrap();
        assert!(!adult_morale
            .modifiers
            .iter()
            .any(|m| m.label == ELDER_RATIONS_LABEL));
        // Repeal removes the scar on the next tick (refresh-never-stack).
        world
            .resource_mut::<ColonyPolicies>()
            .disable(Policy::ElderHalfRations);
        run!(&mut world, apply_edict_modifiers);
        let elder_morale = world.get::<Morale>(elder).unwrap();
        assert!(!elder_morale
            .modifiers
            .iter()
            .any(|m| m.label == ELDER_RATIONS_LABEL));
    }
}
