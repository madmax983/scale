//! Spec 339 — Mentorship (generational knowledge transfer).
//!
//! When a highly skilled pop (a Master) works alongside a much less
//! skilled pop (a Novice) on the *same* task, close by, the novice learns
//! by watching: their XP gain for that skill is multiplied, and both
//! parties feel better about it — the master for teaching, the novice
//! for learning.
//!
//! This module is the Spec 339 upgrade of the original Spec 069
//! mentorship. What 339 changed, per its §5 REFACTOR guidance:
//!
//! - **Links are computed periodically**, not every tick:
//!   [`check_mentorship_system`] only runs on
//!   [`LINK_REFRESH_INTERVAL_TICKS`] boundaries (the O(N^2) pair scan is
//!   amortised; colonies are small and the scan is over *working* pops
//!   only). A formed link then lives for [`LINK_EXPIRATION_TICKS`] ticks
//!   and is refreshed in place while the pair still qualifies — no
//!   duplicate memories or chronicles for a continuing link.
//! - **Mood goes through the real systems**, never raw `Needs` mutation:
//!   [`mentorship_mood_system`] applies refreshing, non-stacking
//!   [`MoodModifier`]s on [`Morale`], and a *new* link writes real
//!   [`Memories`] ([`MemoryType::LearnedFromMentor`] for the novice,
//!   [`MemoryType::TaughtNovice`] for the master). The pre-339 system
//!   added +0.5 to `Needs.leisure` every tick, unbounded.
//! - **"Same job" means what the pop is actually doing**: the skill under
//!   instruction is derived from the pop's current `MovementTarget`
//!   designation (the real current-task tracking), not from their static
//!   `Job`/`AssignmentType` employment — exactly the alignment §5 asks
//!   for. (Spec §7's `JobTenure` pointer is honoured in spirit: tenure
//!   is a *history* of assignments, not current work, so it is not used
//!   to decide who is teaching whom right now.)
//!
//! ## Skill scale adaptation (spec §4 → real `Skills`)
//!
//! The spec sketch uses a flat 0–100 skill level. The real [`Skills`]
//! store XP with a square-root level curve (`level = sqrt(xp / 100)`),
//! so this module compares pops on a *proficiency* scale:
//! `proficiency = sqrt(xp / 100) * 10` — real level 5 is proficiency
//! 50, real level 8 is 80, real level 1 is 10. On that scale the spec's
//! constants carry over literally:
//!
//! - [`SKILL_THRESHOLD_DIFF`] = 40: the master must exceed the novice
//!   by 40 proficiency (the relative rule is primary, per spec §7).
//! - [`MASTER_PROFICIENCY`] = 50: the flat master floor (≈ spec's
//!   "~50") — real level 5 / 2500 XP, the same mastery bar
//!   `generational_knowledge` uses for inheritance.
//! - [`BONUS_XP_MULTIPLIER`] = 2.0: a mentored novice's XP gain for the
//!   taught skill is doubled (base work XP is 1.0 — see
//!   `general_work`/`farm` — so the link adds a 1.0 XP/tick trickle).
//!   Spec 333 seniority still scales teaching on top: a Founder master
//!   teaches at 3.0x, a Veteran at 2.4x (see `teaching_multiplier`).
//! - [`MENTORSHIP_RADIUS`] = 3.0 tiles (Chebyshev), per spec.

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::execution::{AtTarget, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::pop::PopName;
use crate::layer1::skills::{SkillType, Skills};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::seniority::{mentor_multiplier_for, Seniority};
use crate::layer1::utility_types::ActionType;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Radius (in tiles, Chebyshev) within which a master can mentor.
/// Spec §4: `MENTORSHIP_RADIUS = 3.0`.
pub const MENTORSHIP_RADIUS: f32 = 3.0;

/// Integer form of [`MENTORSHIP_RADIUS`] for grid distance checks.
pub const MENTORSHIP_RADIUS_TILES: i32 = 3;

/// Proficiency gap a master must exceed a novice by. Spec §4:
/// `SKILL_THRESHOLD_DIFF = 40.0` (relative rule, spec §7).
pub const SKILL_THRESHOLD_DIFF: f32 = 40.0;

/// Flat proficiency floor for counting as a master at all (≈ spec's
/// "~50"; real level 5 / 2500 XP on the proficiency scale).
pub const MASTER_PROFICIENCY: f32 = 50.0;

/// Total XP multiplier for a mentored novice. Spec §4:
/// `BONUS_XP_MULTIPLIER = 2.0`.
pub const BONUS_XP_MULTIPLIER: f32 = 2.0;

/// Base XP a pop earns per tick of real work on a skill (the rate
/// `general_work` and `farm` grant per completed work action). The
/// mentorship trickle adds `BASE_WORK_XP * (multiplier - 1)` per tick,
/// so at the spec multiplier the novice's total gain is doubled.
pub const BASE_WORK_XP: f32 = 1.0;

/// How often mentorship links are (re)computed, in ticks. Spec §5:
/// update links periodically rather than checking every tick.
pub const LINK_REFRESH_INTERVAL_TICKS: u64 = 25;

/// Ticks a formed link stays valid before it must be re-earned.
pub const LINK_EXPIRATION_TICKS: u32 = 50;

/// Mood label for the novice's teaching buff (real `Morale` path).
pub const NOVICE_MOOD_LABEL: &str = "Learning from a Master";

/// Mood label for the master's teaching buff (real `Morale` path).
pub const MASTER_MOOD_LABEL: &str = "Pride in Teaching";

/// Mood value of the novice buff (real morale scale, −1.0..1.0;
/// the spec sketch's 0.5 was on its 0–100 mood sketch scale).
pub const NOVICE_MOOD_VALUE: f32 = 0.05;

/// Mood value of the master buff (real morale scale).
pub const MASTER_MOOD_VALUE: f32 = 0.05;

/// Duration of a teaching mood modifier; refreshed while the link
/// lives, decays away once it lapses.
pub const MOOD_MODIFIER_TICKS: u32 = 150;

/// Chronicle template when a new mentorship link forms (the spec's
/// optional master-rank hook: a pop skilled enough to teach is
/// recognised in the colony record the first time they take a novice
/// on — and again only if the link lapses and a new one forms).
pub const CHRONICLE_LINK_TEMPLATE: &str =
    "{master} has taken {novice} under their wing, passing on hard-won craft.";

/// Component representing an active mentorship relationship.
/// The entity with this component is the "Novice" and receives XP bonuses.
#[derive(Component, Debug, Clone)]
pub struct Mentorship {
    /// The entity ID of the Master.
    pub master_entity: Entity,
    /// The skill being mentored.
    pub skill: SkillType,
    /// The XP multiplier (e.g., 2.0 for doubled XP at the spec base rate).
    pub multiplier: f32,
    /// Ticks until this relationship needs re-evaluation.
    pub expiration: u32,
}

/// A pop's proficiency in `skill` on the spec's 0–100 scale:
/// `sqrt(xp / 100) * 10` over the real square-root XP curve.
/// Level 1 → 10, level 5 → 50, level 8 → 80, level 10 → 100.
#[must_use]
pub fn skill_proficiency(skills: &Skills, skill: SkillType) -> f32 {
    (skills.get_xp(skill) / 100.0).sqrt() * 10.0
}

/// Whether `master` can mentor `novice` in `skill`: the master clears
/// the flat [`MASTER_PROFICIENCY`] floor *and* exceeds the novice by
/// [`SKILL_THRESHOLD_DIFF`] (spec §7: the relative rule is primary).
#[must_use]
pub fn is_master_for(master: &Skills, novice: &Skills, skill: SkillType) -> bool {
    let master_prof = skill_proficiency(master, skill);
    master_prof >= MASTER_PROFICIENCY
        && master_prof - skill_proficiency(novice, skill) >= SKILL_THRESHOLD_DIFF
}

/// Effective teaching multiplier for a master: the spec's
/// [`BONUS_XP_MULTIPLIER`] scaled by the master's Spec 333 seniority
/// authority (`mentor_multiplier_for` is 1.5 at base, so dividing it
/// out leaves the authority factor: NewBlood/none 1.0, Veteran 1.2,
/// Founder 1.5 → effective 2.0x / 2.4x / 3.0x).
fn teaching_multiplier(master_seniority: Option<&Seniority>) -> f32 {
    const BASE_TEACHING_MULTIPLIER: f32 = 1.5;
    BONUS_XP_MULTIPLIER * (mentor_multiplier_for(master_seniority) / BASE_TEACHING_MULTIPLIER)
}

/// The skill a designation trains, if any (the pop's *current* task —
/// spec §5: derive the job from what the pop is actually doing).
fn skill_for_designation(designation_type: DesignationType) -> Option<SkillType> {
    match designation_type {
        DesignationType::Mine | DesignationType::ExcavateDust => Some(SkillType::Mining),
        DesignationType::Chop => Some(SkillType::Forestry),
        DesignationType::Repair
        | DesignationType::Demolish
        | DesignationType::JuryRig
        | DesignationType::Cannibalize
        | DesignationType::Destroy => Some(SkillType::Construction),
        DesignationType::ClearFlora | DesignationType::CollectSample => Some(SkillType::Farming),
        DesignationType::SetZone(_) | DesignationType::Consume | DesignationType::Douse => None,
        DesignationType::Tame => Some(SkillType::Husbandry),
    }
}

/// Owned snapshot of one working pop, collected before pair matching so
/// the pair scan never fights the query borrow.
struct WorkerInfo {
    entity: Entity,
    pos: GridPosition,
    skills: Skills,
    skill: SkillType,
    teaching_multiplier: f32,
    name: Option<String>,
    existing_link: Option<(Entity, SkillType)>,
}

/// System to detect and establish mentorship relationships.
///
/// Runs only on [`LINK_REFRESH_INTERVAL_TICKS`] boundaries when a
/// `SimulationTime` resource is present (spec §5: periodic links, not
/// an every-tick O(N^2) scan). Finds Master–Novice pairs among pops
/// *at work* (`AtTarget` + a Work `MovementTarget`) whose current
/// designation trains the same skill, within [`MENTORSHIP_RADIUS`].
///
/// A still-valid link is refreshed in place (no duplicate memories or
/// chronicles). A *new* link writes a [`MemoryType::LearnedFromMentor`]
/// memory for the novice and [`MemoryType::TaughtNovice`] for the
/// master, and files a Standard chronicle entry.
#[allow(clippy::type_complexity)]
pub fn check_mentorship_system(
    mut commands: Commands,
    time: Option<Res<SimulationTime>>,
    mut pops: Query<
        (
            Entity,
            &GridPosition,
            &Skills,
            &MovementTarget,
            Option<&Seniority>,
            Option<&PopName>,
            Option<&mut Mentorship>,
        ),
        With<AtTarget>,
    >,
    designations: Query<&Designation>,
    mut memories: Query<&mut Memories>,
    mut chronicle_events: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    // Spec §5: links are recomputed periodically, not every tick.
    let tick = time.as_ref().map_or(0, |t| t.tick);
    if time.is_some() && !tick.is_multiple_of(LINK_REFRESH_INTERVAL_TICKS) {
        return;
    }

    // 1. Collect working pops and the skill their current task trains.
    let mut workers: Vec<WorkerInfo> = Vec::new();
    for (entity, pos, skills, mt, seniority, name, link) in &pops {
        if mt.for_action != ActionType::Work {
            continue;
        }
        let Ok(designation) = designations.get(mt.target_entity) else {
            continue;
        };
        let Some(skill) = skill_for_designation(designation.designation_type) else {
            continue;
        };
        workers.push(WorkerInfo {
            entity,
            pos: *pos,
            skills: skills.clone(),
            skill,
            teaching_multiplier: teaching_multiplier(seniority),
            name: name.map(|n| n.0.clone()),
            existing_link: link.map(|l| (l.master_entity, l.skill)),
        });
    }

    // 2. For each novice, find the most proficient valid master in range.
    for novice in &workers {
        let mut best: Option<&WorkerInfo> = None;
        let mut best_prof = f32::MIN;
        for candidate in &workers {
            if candidate.entity == novice.entity || candidate.skill != novice.skill {
                continue;
            }
            if !is_master_for(&candidate.skills, &novice.skills, novice.skill) {
                continue;
            }
            let dist = (novice.pos.x - candidate.pos.x)
                .abs()
                .max((novice.pos.y - candidate.pos.y).abs());
            if dist > MENTORSHIP_RADIUS_TILES {
                continue;
            }
            let prof = skill_proficiency(&candidate.skills, novice.skill);
            if prof > best_prof {
                best_prof = prof;
                best = Some(candidate);
            }
        }
        let Some(master) = best else { continue };

        // 3a. Still-valid link: refresh its lifetime in place.
        if novice.existing_link == Some((master.entity, novice.skill)) {
            if let Ok(item) = pops.get_mut(novice.entity) {
                if let Some(mut link) = item.6 {
                    link.expiration = LINK_EXPIRATION_TICKS;
                }
            }
            continue;
        }

        // 3b. New link: component + memories + chronicle.
        commands.entity(novice.entity).insert(Mentorship {
            master_entity: master.entity,
            skill: novice.skill,
            multiplier: master.teaching_multiplier,
            expiration: LINK_EXPIRATION_TICKS,
        });
        if let Ok(mut mem) = memories.get_mut(novice.entity) {
            if !mem
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::LearnedFromMentor)
            {
                mem.add(MemoryType::LearnedFromMentor, tick);
            }
        }
        if let Ok(mut mem) = memories.get_mut(master.entity) {
            if !mem
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::TaughtNovice)
            {
                mem.add(MemoryType::TaughtNovice, tick);
            }
        }
        if let Some(events) = chronicle_events.as_deref_mut() {
            let master_name = master.name.as_deref().unwrap_or("A master");
            let novice_name = novice.name.as_deref().unwrap_or("a novice");
            events.send(AddChronicleEvent {
                text: CHRONICLE_LINK_TEMPLATE
                    .replace("{master}", master_name)
                    .replace("{novice}", novice_name),
                importance: EventImportance::Standard,
            });
        }
    }
}

/// System to apply passive XP gain to novices and handle expiration.
///
/// The trickle is the *bonus* half of the doubled gain: real work
/// grants [`BASE_WORK_XP`] per action, and an active link adds
/// `BASE_WORK_XP * (multiplier - 1)` per tick, so a novice at the spec
/// multiplier learns exactly twice as fast while linked.
pub fn apply_mentorship_xp_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Skills, &mut Mentorship)>,
) {
    for (entity, mut skills, mut mentorship) in &mut query {
        let bonus = BASE_WORK_XP * (mentorship.multiplier - 1.0).max(0.0);
        if bonus > 0.0 {
            skills.add_xp(mentorship.skill, bonus);
        }

        // Decay expiration
        if mentorship.expiration > 0 {
            mentorship.expiration -= 1;
        } else {
            commands.entity(entity).remove::<Mentorship>();
        }
    }
}

/// Applies (or refreshes, never stacks) a teaching mood modifier on
/// `target` through the real [`Morale`] path (spec §5 REFACTOR).
fn apply_teaching_mood(
    commands: &mut Commands,
    morale_query: &mut Query<&mut Morale>,
    target: Entity,
    label: &str,
    value: f32,
) {
    match morale_query.get_mut(target) {
        Ok(mut morale) => {
            if let Some(existing) = morale.modifiers.iter_mut().find(|m| m.label == label) {
                existing.value = value;
                existing.duration = MOOD_MODIFIER_TICKS;
            } else {
                morale.add_modifier(MoodModifier {
                    label: label.to_string(),
                    value,
                    duration: MOOD_MODIFIER_TICKS,
                });
            }
        }
        Err(_) => {
            let mut fresh = Morale::default();
            fresh.add_modifier(MoodModifier {
                label: label.to_string(),
                value,
                duration: MOOD_MODIFIER_TICKS,
            });
            commands.entity(target).insert(fresh);
        }
    }
}

/// System to grant the teaching mood buff to both parties of every
/// active mentorship link, via the real `Morale`/`MoodModifier` system
/// (spec §5: never direct `Needs` mutation). Modifiers refresh while
/// the link lives and decay once it lapses; a master with several
/// novices is buffed once, not stacked.
pub fn mentorship_mood_system(
    mut commands: Commands,
    links: Query<(Entity, &Mentorship)>,
    mut morale_query: Query<&mut Morale>,
) {
    let mut novices: Vec<Entity> = Vec::new();
    let mut masters: Vec<Entity> = Vec::new();
    for (novice, link) in &links {
        novices.push(novice);
        if !masters.contains(&link.master_entity) {
            masters.push(link.master_entity);
        }
    }
    for novice in novices {
        apply_teaching_mood(
            &mut commands,
            &mut morale_query,
            novice,
            NOVICE_MOOD_LABEL,
            NOVICE_MOOD_VALUE,
        );
    }
    for master in masters {
        apply_teaching_mood(
            &mut commands,
            &mut morale_query,
            master,
            MASTER_MOOD_LABEL,
            MASTER_MOOD_VALUE,
        );
    }
}

/// Number of pops currently learning under a mentor. Headless STATS helper.
#[must_use]
pub fn mentored_count(world: &mut World) -> usize {
    let mut query = world.query::<&Mentorship>();
    query.iter(world).count()
}

/// Player-facing strings, for the IP-guard test.
#[must_use]
pub fn mentorship_public_strings() -> Vec<&'static str> {
    vec![
        NOVICE_MOOD_LABEL,
        MASTER_MOOD_LABEL,
        CHRONICLE_LINK_TEMPLATE,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::designation::{Designation, DesignationType};
    use crate::layer1::execution::{AtTarget, MovementTarget};
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::utility_types::ActionType;

    fn skills_with(skill: SkillType, xp: f32) -> Skills {
        let mut skills = Skills::default();
        skills.add_xp(skill, xp);
        skills
    }

    /// Spawns a pop at work on a fresh designation of `designation_type`.
    fn spawn_worker(
        world: &mut World,
        x: i32,
        y: i32,
        skills: Skills,
        designation_type: DesignationType,
    ) -> Entity {
        let designation = world
            .spawn((Designation { designation_type }, GridPosition { x, y }))
            .id();
        world
            .spawn((
                Pop,
                GridPosition { x, y },
                skills,
                MovementTarget {
                    target_entity: designation,
                    target_position: GridPosition { x, y },
                    for_action: ActionType::Work,
                },
                AtTarget,
            ))
            .id()
    }

    #[test]
    fn test_proficiency_scale_matches_spec_mapping() {
        // Real level 5 (2500 XP) is proficiency 50 — the master floor;
        // real level 8 (6400 XP) is 80; real level 1 (100 XP) is 10.
        assert!(
            (skill_proficiency(&skills_with(SkillType::Mining, 2500.0), SkillType::Mining) - 50.0)
                .abs()
                < 0.01
        );
        assert!(
            (skill_proficiency(&skills_with(SkillType::Mining, 6400.0), SkillType::Mining) - 80.0)
                .abs()
                < 0.01
        );
        assert!(
            (skill_proficiency(&skills_with(SkillType::Mining, 100.0), SkillType::Mining) - 10.0)
                .abs()
                < 0.01
        );
        // Relative rule primary: a prof-50 master over a prof-10
        // novice has a gap of exactly 40, so this pair qualifies.
        assert!(is_master_for(
            &skills_with(SkillType::Mining, 2500.0),
            &skills_with(SkillType::Mining, 100.0),
            SkillType::Mining
        ));
        // ...but a prof-40 pop (1600 XP) is nobody's master, even over
        // a total beginner (flat floor).
        assert!(!is_master_for(
            &skills_with(SkillType::Mining, 1600.0),
            &Skills::default(),
            SkillType::Mining
        ));
    }

    #[test]
    fn test_mentorship_detection_valid_pair() {
        let mut world = World::new();

        // Master (Mining proficiency 50) and novice (0), 3 tiles apart
        // (exactly at the spec radius).
        let master = spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 2500.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 7, 5, Skills::default(), DesignationType::Mine);

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        let mentorship = world.get::<Mentorship>(apprentice);
        assert!(
            mentorship.is_some(),
            "Apprentice should have Mentorship component"
        );
        let mentorship = mentorship.unwrap();
        assert_eq!(mentorship.master_entity, master);
        assert_eq!(mentorship.skill, SkillType::Mining);
        // No seniority: exactly the spec multiplier.
        assert!((mentorship.multiplier - BONUS_XP_MULTIPLIER).abs() < 1e-6);
    }

    #[test]
    fn test_novice_gains_xp_bonus_near_master() {
        // Spec 339 RED test 1, adapted to real types: master at
        // proficiency 80 (6400 XP, real level 8) at (10,10); novice at
        // proficiency 10 (100 XP, real level 1) adjacent at (11,11),
        // both on Mine work.
        let mut world = World::new();
        spawn_worker(
            &mut world,
            10,
            10,
            skills_with(SkillType::Mining, 6400.0),
            DesignationType::Mine,
        );
        let novice = spawn_worker(
            &mut world,
            11,
            11,
            skills_with(SkillType::Mining, 100.0),
            DesignationType::Mine,
        );
        // Control: an identical novice working alone, far away.
        let control = spawn_worker(
            &mut world,
            40,
            40,
            skills_with(SkillType::Mining, 100.0),
            DesignationType::Mine,
        );

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        let mut xp_schedule = Schedule::default();
        xp_schedule.add_systems(apply_mentorship_xp_system);
        xp_schedule.run(&mut world);

        // The mentored novice's bonus alone equals a full tick of base
        // work XP — with the work itself, they gain at 2x the base rate.
        let novice_gain = world
            .get::<Skills>(novice)
            .unwrap()
            .get_xp(SkillType::Mining)
            - 100.0;
        assert!(
            novice_gain >= BASE_WORK_XP,
            "novice should gain bonus XP near a master, gained {novice_gain}"
        );
        assert!((novice_gain - BASE_WORK_XP * (BONUS_XP_MULTIPLIER - 1.0)).abs() < 1e-6);
        // The control novice gained nothing from the XP system.
        let control_gain = world
            .get::<Skills>(control)
            .unwrap()
            .get_xp(SkillType::Mining)
            - 100.0;
        assert_eq!(control_gain, 0.0);
    }

    #[test]
    fn test_mentorship_grants_mood_buffs() {
        // Spec 339 RED test 2, adapted: mood is asserted on the real
        // Morale/MoodModifier path (spec §5), not on raw Needs. Master
        // proficiency 90 would be 8100 XP; the link itself is inserted
        // directly, as in the pre-339 mood test.
        let mut world = World::new();

        let master = world.spawn((Pop, Morale::default())).id();
        let apprentice = world
            .spawn((
                Pop,
                Morale::default(),
                Mentorship {
                    master_entity: master,
                    skill: SkillType::Crafting,
                    multiplier: BONUS_XP_MULTIPLIER,
                    expiration: 10,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(mentorship_mood_system);
        schedule.run(&mut world);
        // Run again: refresh must not stack a second modifier.
        schedule.run(&mut world);

        let master_morale = world.get::<Morale>(master).unwrap();
        let master_mods: Vec<_> = master_morale
            .modifiers
            .iter()
            .filter(|m| m.label == MASTER_MOOD_LABEL)
            .collect();
        assert_eq!(master_mods.len(), 1, "master buff must not stack");
        assert!(
            master_mods[0].value > 0.0,
            "master should gain mood buff from teaching"
        );

        let app_morale = world.get::<Morale>(apprentice).unwrap();
        let app_mods: Vec<_> = app_morale
            .modifiers
            .iter()
            .filter(|m| m.label == NOVICE_MOOD_LABEL)
            .collect();
        assert_eq!(app_mods.len(), 1, "novice buff must not stack");
        assert!(
            app_mods[0].value > 0.0,
            "novice should gain mood buff from learning"
        );
    }

    #[test]
    fn test_link_formation_adds_memories_and_chronicle_once() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();

        let master = spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 2500.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 5, 5, Skills::default(), DesignationType::Mine);
        world.entity_mut(master).insert(Memories::default());
        world.entity_mut(apprentice).insert(Memories::default());
        world.entity_mut(master).insert(PopName("Ada".to_string()));
        world
            .entity_mut(apprentice)
            .insert(PopName("Ben".to_string()));

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);
        // Second pass: the link still qualifies — refresh only.
        schedule.run(&mut world);

        let novice_mem = world.get::<Memories>(apprentice).unwrap();
        assert_eq!(
            novice_mem
                .items
                .iter()
                .filter(|m| m.memory_type == MemoryType::LearnedFromMentor)
                .count(),
            1,
            "novice should remember learning exactly once"
        );
        let master_mem = world.get::<Memories>(master).unwrap();
        assert_eq!(
            master_mem
                .items
                .iter()
                .filter(|m| m.memory_type == MemoryType::TaughtNovice)
                .count(),
            1,
            "master should remember teaching exactly once"
        );

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let texts: Vec<String> = reader.read(events).map(|e| e.text.clone()).collect();
        assert_eq!(texts.len(), 1, "one chronicle per new link, got {texts:?}");
        assert!(texts[0].contains("Ada") && texts[0].contains("Ben"));
        assert!(texts[0].contains("under their wing"));
    }

    #[test]
    fn test_periodic_link_refresh_gating() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());

        spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 2500.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 5, 5, Skills::default(), DesignationType::Mine);

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);

        // Tick 1 is not a refresh boundary: no link yet.
        world.resource_mut::<SimulationTime>().tick = 1;
        schedule.run(&mut world);
        assert!(world.get::<Mentorship>(apprentice).is_none());

        // Tick 25 is a refresh boundary: the link forms.
        world.resource_mut::<SimulationTime>().tick = LINK_REFRESH_INTERVAL_TICKS;
        schedule.run(&mut world);
        assert!(world.get::<Mentorship>(apprentice).is_some());
    }

    #[test]
    fn test_mentorship_detection_ignores_too_far() {
        let mut world = World::new();

        spawn_worker(
            &mut world,
            1,
            0,
            skills_with(SkillType::Mining, 2500.0),
            DesignationType::Mine,
        );
        // Apprentice at (9,0) — well outside the 3-tile radius.
        let apprentice = spawn_worker(&mut world, 9, 0, Skills::default(), DesignationType::Mine);

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

        // "Master" at proficiency ~12 (150 XP) — neither the flat
        // floor nor the +40 gap is met.
        spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 150.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 7, 5, Skills::default(), DesignationType::Mine);

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        assert!(
            world.get::<Mentorship>(apprentice).is_none(),
            "Proficiency gap too small"
        );
    }

    #[test]
    fn test_no_link_below_flat_master_threshold() {
        let mut world = World::new();

        // Proficiency 40 (1600 XP): gap over a beginner is exactly 40,
        // but the flat master floor (50) is not met — no link.
        spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 1600.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 5, 5, Skills::default(), DesignationType::Mine);

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        assert!(world.get::<Mentorship>(apprentice).is_none());
    }

    #[test]
    fn test_no_link_for_different_work() {
        let mut world = World::new();

        // A genuine Mining master cannot mentor Forestry work.
        spawn_worker(
            &mut world,
            4,
            5,
            skills_with(SkillType::Mining, 6400.0),
            DesignationType::Mine,
        );
        let apprentice = spawn_worker(&mut world, 5, 5, Skills::default(), DesignationType::Chop);

        let mut schedule = Schedule::default();
        schedule.add_systems(check_mentorship_system);
        schedule.run(&mut world);

        assert!(world.get::<Mentorship>(apprentice).is_none());
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
        // Bonus is base XP scaled by (multiplier - 1): 1.0 * 0.5.
        assert!((skills.get_xp(SkillType::Mining) - 0.5).abs() < 1e-6);
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
    fn test_ip_guard_no_banned_terms() {
        // Concept-only: "Mentorship", "Master" and "Novice" are generic
        // words; no named characters, places, or distinctive entities
        // from outside fiction may appear in player-facing strings.
        const BANNED: &[&str] = &[
            "arrakis",
            "dune",
            "atreides",
            "harkonnen",
            "fremen",
            "sandworm",
            "tatooine",
            "coruscant",
            "jedi",
            "sith",
            "wookiee",
            "mordor",
            "gondor",
            "hobbit",
            "shire",
            "sauron",
            "gandalf",
            "rivendell",
            "westeros",
            "stark",
            "lannister",
            "targaryen",
            "winterfell",
            "klingon",
            "vulcan",
            "enterprise",
            "borg",
            "romulan",
            "xenomorph",
            "ripley",
            "weyland",
            "nostromo",
            "megatron",
            "optimus",
            "cybertron",
        ];
        for s in mentorship_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
    }
}
