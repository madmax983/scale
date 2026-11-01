//! Diplomatic Marriage (Spec 383).
//!
//! Space feudalism: securing peace by sending your best and brightest
//! away forever. The colony can "marry off" a high-status or high-skill
//! pop to a foreign Layer 3 faction: the pop leaves Layer 1 permanently
//! and the target faction becomes an ally.
//!
//! ## Tension
//!
//! You have to send your only level-10 medic to a foreign power to stop
//! an invasion — and the infirmary collapses without them. Sacrifice a
//! key individual for the good of the state?
//!
//! ## Design notes (spec adaptation)
//!
//! * The spec's RED phase names mock `Pop { skill_level }` /
//!   `Faction { alliance }` structs and a `MarryOffEvent { pop,
//!   target_faction: Entity }`. None of those exist verbatim. The real
//!   equivalents used here:
//!   - `Pop` is the marker in `crate::layer1::pop`; skill lives in the
//!     [`Skills`] component (level = `floor(sqrt(xp / 100))` per skill).
//!     A pop's marriageable skill is their *best* skill level.
//!   - Foreign factions are not entities: Layer 3 diplomacy keys them by
//!     `u32` id in [`FactionRelations`] (scores −100 Hostile .. 100
//!     Allied, per `system_sovereignty`). So `target_faction` is a
//!     `u32` faction id, and "alliance" means setting that faction's
//!     score to [`ALLIED_RELATION_SCORE`] (100 = Allied).
//! * Eligibility (spec §3/§4 + §1 "high-status or high-skill"): the
//!   pop's best skill level is at least [`MARRIAGE_SKILL_THRESHOLD`]
//!   (8, the spec's tunable constant), **or** the pop is high-status —
//!   a [`Leader`], [`AscensionCandidate`], [`Officer`], or
//!   [`NobleScion`], or carries the Noble / Prophet trait (the same
//!   prestige markers the martyrdom pass treats as leader figures).
//!   Low-skill, low-status pops are refused: they stay, no alliance is
//!   formed, and a [`MarriageRefusedEvent`] is emitted.
//! * Departure cleanup (spec §5/§7): the married pop is despawned
//!   outright. As the Long-Haulers pass documented, everything a pop
//!   owns that matters (job, action, equipment, wallet) is a component
//!   *on* the pop, and assignments are components on the pop too — there
//!   is no separate bed/job registry holding a bare `Entity` that
//!   outlives the pop in this codebase, so despawning *is* the cleanup.
//!   A [`MarriedOffEvent`] is emitted before the despawn so future
//!   mood / relationship hooks can react to the departure.
//! * Every outcome is ledgered in [`MarriageLedger`] (headless STATS
//!   `marriages=` / panel `marriages`) and successful marriages write a
//!   **Major** chronicle entry.
//! * All names are original and generic ("Diplomatic Marriage", "the
//!   Starward Compact"); nothing is lifted from outside fiction (see
//!   the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::chain_of_command::Officer;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::leader_ascension::{AscensionCandidate, Leader};
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::skills::{SkillType, Skills};
use crate::layer1::social::cadet::NobleScion;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

/// Best-skill level a pop needs to be an acceptable marriage candidate
/// (spec §4: the hardcoded `8`, promoted to a tunable constant per §5).
pub const MARRIAGE_SKILL_THRESHOLD: u32 = 8;

/// [`FactionRelations`] score that means Allied (the map's documented
/// top end: −100 Hostile .. 100 Allied).
pub const ALLIED_RELATION_SCORE: i32 = 100;

/// Default Layer-3 faction a marriage targets when none is named: the
/// Starward Compact, an original generic partner power.
pub const MARRIAGE_FACTION_ID: u32 = 21;

/// Event: marry `pop` off to the Layer-3 faction `target_faction`.
///
/// Processed by [`process_marriage_system`]: eligible pops depart and
/// the faction becomes an ally; ineligible pops are refused in place.
#[derive(Event, Debug, Clone, Copy)]
pub struct MarryOffEvent {
    /// The pop being married off (a Layer 1 entity).
    pub pop: Entity,
    /// Layer-3 faction id in [`FactionRelations`].
    pub target_faction: u32,
}

/// Event: a pop has been married off and has left the colony for good.
///
/// Emitted just before the pop entity is despawned, for any future
/// mood / relationship hooks (mirrors `TravelerDepartedEvent`).
#[derive(Event, Debug, Clone)]
pub struct MarriedOffEvent {
    /// The (about to be despawned) pop entity.
    pub entity: Entity,
    /// The departing pop's name.
    pub name: String,
    /// Layer-3 faction id they married into.
    pub faction_id: u32,
    /// Their best skill, if they had any XP at all.
    pub skill: Option<SkillType>,
    /// Level of that best skill (0 when skill-less).
    pub skill_level: u32,
    /// Whether they qualified by status rather than skill.
    pub by_status: bool,
    /// Tick on which they departed.
    pub tick: u64,
}

/// Event: a marriage offer was refused — the pop was neither skilled
/// nor high-status enough. The pop stays; no alliance is formed.
#[derive(Event, Debug, Clone)]
pub struct MarriageRefusedEvent {
    /// The pop who was refused.
    pub entity: Entity,
    /// The refused pop's name.
    pub name: String,
    /// Layer-3 faction id that refused them.
    pub faction_id: u32,
    /// Their best skill level (below [`MARRIAGE_SKILL_THRESHOLD`]).
    pub best_level: u32,
}

/// One completed diplomatic marriage, for the headless panel.
#[derive(Debug, Clone)]
pub struct MarriageRecord {
    /// Name of the pop who left.
    pub pop_name: String,
    /// Layer-3 faction id they married into.
    pub faction_id: u32,
    /// Their best skill, if any.
    pub skill: Option<SkillType>,
    /// Level of that best skill.
    pub skill_level: u32,
    /// Whether they qualified by status rather than skill.
    pub by_status: bool,
    /// Tick of the marriage.
    pub tick: u64,
}

/// Ledger of diplomatic marriages and refusals (headless STATS / panel).
#[derive(Resource, Default, Debug, Clone)]
pub struct MarriageLedger {
    /// Marriages completed.
    pub marriages: u64,
    /// Offers refused (pop not eligible).
    pub refusals: u64,
    /// Completed marriages, in order.
    pub records: Vec<MarriageRecord>,
}

/// Generic Layer-3 faction name for chronicles. Original names only.
#[must_use]
pub fn marriage_faction_name(faction_id: u32) -> &'static str {
    match faction_id {
        7 => "the Homeworld",
        13 => "the pirate raiders",
        MARRIAGE_FACTION_ID => "the Starward Compact",
        _ => "a foreign faction",
    }
}

/// The pop's best skill and its level, if they have any XP at all.
///
/// Mirrors `long_haulers::best_skill` (same fixed skill list).
#[must_use]
pub fn marriage_best_skill(skills: &Skills) -> Option<(SkillType, u32)> {
    [
        SkillType::Mining,
        SkillType::Forestry,
        SkillType::Farming,
        SkillType::Construction,
        SkillType::Crafting,
        SkillType::Husbandry,
        SkillType::Engineering,
        SkillType::Science,
    ]
    .into_iter()
    .map(|s| (s, skills.get_level(s)))
    .filter(|(_, level)| *level > 0)
    .max_by_key(|(_, level)| *level)
}

/// Pure: does this pop carry high status (leadership / nobility /
/// prophethood)? Mirrors the prestige markers in martyrdom's
/// `is_leader_figure`, minus its skill-prestige term (skill is gated
/// separately by [`MARRIAGE_SKILL_THRESHOLD`] here).
#[must_use]
pub fn is_high_status(
    is_leader: bool,
    is_candidate: bool,
    is_officer: bool,
    is_scion: bool,
    traits: Option<&Traits>,
) -> bool {
    if is_leader || is_candidate || is_officer || is_scion {
        return true;
    }
    traits.is_some_and(|t| t.has(Trait::Noble) || t.has(Trait::Prophet))
}

/// Pure: can this pop be married off? Skilled enough, or high-status.
#[must_use]
pub fn is_marriageable(skills: Option<&Skills>, high_status: bool) -> bool {
    if high_status {
        return true;
    }
    skills.is_some_and(|s| {
        marriage_best_skill(s).is_some_and(|(_, level)| level >= MARRIAGE_SKILL_THRESHOLD)
    })
}

/// Processes [`MarryOffEvent`]s: eligible pops depart Layer 1 for good
/// and their target faction becomes an ally; ineligible pops are
/// refused (they stay, no alliance, [`MarriageRefusedEvent`] emitted).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn process_marriage_system(
    mut commands: Commands,
    mut events: EventReader<MarryOffEvent>,
    mut married: EventWriter<MarriedOffEvent>,
    mut refused: EventWriter<MarriageRefusedEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut relations: ResMut<FactionRelations>,
    mut ledger: ResMut<MarriageLedger>,
    time: Option<Res<SimulationTime>>,
    pops: Query<
        (
            Option<&PopName>,
            Option<&Skills>,
            Option<&Traits>,
            Has<Leader>,
            Has<AscensionCandidate>,
            Has<Officer>,
            Has<NobleScion>,
        ),
        With<Pop>,
    >,
) {
    let tick = time.map_or(0, |t| t.tick);
    for event in events.read() {
        let Ok((name, skills, traits, is_leader, is_candidate, is_officer, is_scion)) =
            pops.get(event.pop)
        else {
            // No such (living) pop: nothing to marry, no alliance.
            continue;
        };
        let pop_name = name.map_or_else(|| "A colonist".to_string(), |n| n.0.clone());
        let (best_skill, best_level) = skills
            .and_then(marriage_best_skill)
            .map_or((None, 0), |(s, l)| (Some(s), l));
        let high_status = is_high_status(is_leader, is_candidate, is_officer, is_scion, traits);

        if !is_marriageable(skills, high_status) {
            ledger.refusals += 1;
            refused.send(MarriageRefusedEvent {
                entity: event.pop,
                name: pop_name,
                faction_id: event.target_faction,
                best_level,
            });
            continue;
        }

        let by_status = best_level < MARRIAGE_SKILL_THRESHOLD;
        relations
            .scores
            .insert(event.target_faction, ALLIED_RELATION_SCORE);
        ledger.marriages += 1;
        ledger.records.push(MarriageRecord {
            pop_name: pop_name.clone(),
            faction_id: event.target_faction,
            skill: best_skill,
            skill_level: best_level,
            by_status,
            tick,
        });
        let faction = marriage_faction_name(event.target_faction);
        let basis = if by_status {
            "high station".to_string()
        } else {
            format!(
                "mastery of {:?} (level {best_level})",
                best_skill.expect("skilled marriage implies a best skill")
            )
        };
        chronicle.send(AddChronicleEvent {
            text: format!(
                "DIPLOMATIC MARRIAGE: {pop_name} has wed into {faction}, sealing an alliance. The colony loses {basis} — and gains a friend among the stars."
            ),
            importance: EventImportance::Major,
        });
        married.send(MarriedOffEvent {
            entity: event.pop,
            name: pop_name,
            faction_id: event.target_faction,
            skill: best_skill,
            skill_level: best_level,
            by_status,
            tick,
        });
        // Departure: despawn is the cleanup (see module docs — a pop's
        // job / bed / task state lives on the pop entity itself).
        commands.entity(event.pop).despawn();
    }
}

/// Number of completed diplomatic marriages (headless STATS).
pub fn marriage_count(world: &mut World) -> u64 {
    world
        .get_resource::<MarriageLedger>()
        .map_or(0, |l| l.marriages)
}

/// Number of Layer-3 factions currently at Allied standing because of
/// a marriage ledger on record is not tracked separately; this counts
/// factions at the Allied score in [`FactionRelations`] (headless
/// STATS `allied=`).
pub fn allied_faction_count(world: &mut World) -> usize {
    world.get_resource::<FactionRelations>().map_or(0, |r| {
        r.scores
            .values()
            .filter(|&&s| s >= ALLIED_RELATION_SCORE)
            .count()
    })
}

/// Public-facing strings, for the IP-guard test.
pub fn marriage_public_strings() -> Vec<String> {
    vec![
        "DIPLOMATIC MARRIAGE: a colonist has wed into a foreign faction, sealing an alliance. The colony loses mastery — and gains a friend among the stars.".to_string(),
        "Diplomatic Marriage".to_string(),
        marriage_faction_name(MARRIAGE_FACTION_ID).to_string(),
        marriage_faction_name(7).to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 500,
            ..Default::default()
        });
        world.insert_resource(FactionRelations::default());
        world.insert_resource(MarriageLedger::default());
        world.insert_resource(Events::<MarryOffEvent>::default());
        world.insert_resource(Events::<MarriedOffEvent>::default());
        world.insert_resource(Events::<MarriageRefusedEvent>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    fn run(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_marriage_system);
        schedule.run(world);
    }

    /// XP for an exact skill level: level = floor(sqrt(xp/100)).
    fn xp_for_level(level: u32) -> f32 {
        (level * level) as f32 * 100.0
    }

    fn spawn_pop(world: &mut World, name: &str, skill: SkillType, level: u32) -> Entity {
        let mut skills = Skills::default();
        if level > 0 {
            skills.add_xp(skill, xp_for_level(level));
        }
        world
            .spawn((Pop, PopName(name.to_string()), skills))
            .id()
    }

    fn chronicle_texts(world: &World) -> Vec<String> {
        let events = world.resource::<Events<AddChronicleEvent>>();
        events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect()
    }

    // --- Spec RED test 1, adapted to real types -------------------------
    #[test]
    fn test_diplomatic_marriage_removes_pop_and_adds_alliance() {
        let mut world = setup();
        // Spec sketch: Pop { skill_level: 10 } — real form: best skill 10.
        let pop_id = spawn_pop(&mut world, "Ada", SkillType::Science, 10);

        world
            .resource_mut::<Events<MarryOffEvent>>()
            .send(MarryOffEvent {
                pop: pop_id,
                target_faction: MARRIAGE_FACTION_ID,
            });
        run(&mut world);

        assert!(
            world.get_entity(pop_id).is_err(),
            "Pop should be removed from the colony"
        );
        let relations = world.resource::<FactionRelations>();
        assert_eq!(
            relations.scores[&MARRIAGE_FACTION_ID], ALLIED_RELATION_SCORE,
            "Faction should now be an ally"
        );
        assert_eq!(world.resource::<MarriageLedger>().marriages, 1);
        let married = world.resource::<Events<MarriedOffEvent>>();
        assert_eq!(married.get_cursor().read(married).count(), 1);
        let texts = chronicle_texts(&world);
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("DIPLOMATIC MARRIAGE"));
        assert!(texts[0].contains("Ada"));
    }

    // --- Spec RED test 2, adapted to real types -------------------------
    #[test]
    fn test_marriage_requires_high_skill() {
        let mut world = setup();
        // Spec sketch: Pop { skill_level: 2 } — low skill.
        let pop_id = spawn_pop(&mut world, "Bryn", SkillType::Mining, 2);

        world
            .resource_mut::<Events<MarryOffEvent>>()
            .send(MarryOffEvent {
                pop: pop_id,
                target_faction: MARRIAGE_FACTION_ID,
            });
        run(&mut world);

        assert!(
            world.get_entity(pop_id).is_ok(),
            "Low skill Pop should NOT be married off"
        );
        let relations = world.resource::<FactionRelations>();
        assert!(
            !relations
                .scores
                .get(&MARRIAGE_FACTION_ID)
                .is_some_and(|&s| s >= ALLIED_RELATION_SCORE),
            "Alliance should fail due to low skill"
        );
        let ledger = world.resource::<MarriageLedger>();
        assert_eq!(ledger.marriages, 0);
        assert_eq!(ledger.refusals, 1);
        let refused_events = world.resource::<Events<MarriageRefusedEvent>>();
        let mut cursor = refused_events.get_cursor();
        let refused: Vec<_> = cursor.read(refused_events).collect();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].best_level, 2);
        assert!(chronicle_texts(&world).is_empty());
    }

    #[test]
    fn test_marriage_threshold_boundary() {
        let mut world = setup();
        let seven = spawn_pop(&mut world, "Cole", SkillType::Farming, 7);
        let eight = spawn_pop(&mut world, "Dara", SkillType::Farming, 8);
        for pop in [seven, eight] {
            world
                .resource_mut::<Events<MarryOffEvent>>()
                .send(MarryOffEvent {
                    pop,
                    target_faction: 42,
                });
        }
        run(&mut world);

        assert!(world.get_entity(seven).is_ok(), "level 7 is refused");
        assert!(world.get_entity(eight).is_err(), "level 8 is accepted");
        assert_eq!(world.resource::<MarriageLedger>().marriages, 1);
        assert_eq!(world.resource::<MarriageLedger>().refusals, 1);
        assert_eq!(
            world.resource::<FactionRelations>().scores[&42],
            ALLIED_RELATION_SCORE
        );
    }

    #[test]
    fn test_high_status_low_skill_pop_is_accepted() {
        let mut world = setup();
        // A Noble Scion has zero skills by design (Spec 1208) but is
        // exactly the high-status candidate §1 describes.
        let pop_id = {
            let mut traits = Traits::default();
            traits.add(Trait::Noble);
            world
                .spawn((
                    Pop,
                    PopName("Eli".to_string()),
                    Skills::default(),
                    traits,
                    NobleScion {
                        allowance: 50.0,
                        faction_id: 7,
                    },
                ))
                .id()
        };
        world
            .resource_mut::<Events<MarryOffEvent>>()
            .send(MarryOffEvent {
                pop: pop_id,
                target_faction: 9,
            });
        run(&mut world);

        assert!(world.get_entity(pop_id).is_err());
        assert_eq!(
            world.resource::<FactionRelations>().scores[&9],
            ALLIED_RELATION_SCORE
        );
        let ledger = world.resource::<MarriageLedger>();
        assert_eq!(ledger.marriages, 1);
        assert!(ledger.records[0].by_status);
    }

    #[test]
    fn test_marriage_for_missing_pop_is_noop() {
        let mut world = setup();
        let ghost = world.spawn_empty().id();
        world.despawn(ghost);
        world
            .resource_mut::<Events<MarryOffEvent>>()
            .send(MarryOffEvent {
                pop: ghost,
                target_faction: MARRIAGE_FACTION_ID,
            });
        run(&mut world);

        assert!(world.resource::<FactionRelations>().scores.is_empty());
        assert_eq!(world.resource::<MarriageLedger>().marriages, 0);
        assert_eq!(world.resource::<MarriageLedger>().refusals, 0);
    }

    #[test]
    fn test_multiple_marriages_accumulate_records() {
        let mut world = setup();
        let a = spawn_pop(&mut world, "Fern", SkillType::Engineering, 9);
        let b = spawn_pop(&mut world, "Gale", SkillType::Crafting, 12);
        for pop in [a, b] {
            world
                .resource_mut::<Events<MarryOffEvent>>()
                .send(MarryOffEvent {
                    pop,
                    target_faction: MARRIAGE_FACTION_ID,
                });
        }
        run(&mut world);

        let ledger = world.resource::<MarriageLedger>();
        assert_eq!(ledger.marriages, 2);
        assert_eq!(ledger.records.len(), 2);
        assert_eq!(ledger.records[0].pop_name, "Fern");
        assert_eq!(ledger.records[1].skill, Some(SkillType::Crafting));
        assert_eq!(ledger.records[1].tick, 500);
    }

    #[test]
    fn test_best_skill_uses_highest_level() {
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Mining, xp_for_level(3));
        skills.add_xp(SkillType::Science, xp_for_level(9));
        assert_eq!(marriage_best_skill(&skills), Some((SkillType::Science, 9)));
        assert!(is_marriageable(Some(&skills), false));
        let mut low = Skills::default();
        low.add_xp(SkillType::Mining, xp_for_level(3));
        assert!(!is_marriageable(Some(&low), false));
        assert!(is_marriageable(Some(&low), true), "status qualifies alone");
        assert!(!is_marriageable(None, false));
    }

    // --- IP guard ---------------------------------------------------------
    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Diplomatic Marriage is concept-only: generic naming, nothing
        // lifted from outside fiction.
        let banned = [
            "sad king billy",
            "windsor",
            "game of thrones",
            "lannister",
            "stark",
            "targaryen",
            "dune",
            "atreides",
            "corrino",
            "arrakis",
            "red wedding",
            "winterfell",
        ];
        for s in marriage_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
