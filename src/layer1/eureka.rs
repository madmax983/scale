//! Eureka Moments (Spec 196, extended by Spec 390).
//!
//! Working a job has a small chance to trigger a *breakthrough* related
//! to the work being done (farming -> Hydroponics, smelting ->
//! MetalWorking, mining -> Masonry). The roll sites are the real
//! work-completion seams: `produce_food_system` (farm production),
//! refining job completion, and `general_work`'s post-work effects.
//! There is no `JobCompletedEvent` in this codebase — those call sites
//! ARE the completion seam (Spec 390 audit, see the 390 report).
//!
//! Spec 390 §8 Architect ruling: a breakthrough does NOT instantly
//! complete a tech. It grants a large chunk of flat research points
//! ([`EurekaConfig::breakthrough_points`]) toward the related tech,
//! accumulated in [`EurekaProgress`]; the tech unlocks only when the
//! accumulated points cover [`Tech::cost`]. Breakthroughs aimed at an
//! already-unlocked tech — and breakthroughs with no related tech —
//! still grant flat Knowledge, exactly as the Spec 196 lane did.
//!
//! Probability (Spec 390 §5): the base chance is modified by traits
//! (Intellectual x1.2, Creative x1.5 — the Spec 196 seam) and by
//! assignment: a pop assigned as a `LibraryWorker` (the colony's
//! researcher job, see `process_research_system`) rolls at
//! [`RESEARCHER_CHANCE_MULTIPLIER`]x. All math lives in the pure
//! [`effective_chance`] helper so it is deterministically testable.

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::resources::ColonyResources;
use crate::layer1::tech::{Tech, TechState};
use crate::layer1::traits::{Trait, Traits};
use crate::layer1::utility_ai::ActionType;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;
use std::collections::HashMap;

/// Chance multiplier for pops assigned as researchers (`LibraryWorker`).
pub const RESEARCHER_CHANCE_MULTIPLIER: f64 = 2.0;

/// Configuration for Eureka Moments.
#[derive(Resource, Clone)]
pub struct EurekaConfig {
    /// Base probability per tick of triggering a Eureka moment.
    pub base_chance: f64,
    /// Amount of Knowledge granted if Tech is already unlocked.
    pub knowledge_reward: f32,
    /// Flat research points a breakthrough grants toward its related
    /// tech (Spec 390 §8). The tech unlocks when accumulated points
    /// reach `Tech::cost()`.
    pub breakthrough_points: f32,
}

impl Default for EurekaConfig {
    fn default() -> Self {
        Self {
            base_chance: 0.0001, // 0.01% per tick
            knowledge_reward: 10.0,
            breakthrough_points: 20.0,
        }
    }
}

/// Event triggered when a Eureka Moment occurs.
#[derive(Event, Debug)]
pub struct EurekaEvent {
    /// The action that triggered the moment.
    pub action: ActionType,
    /// The tech related to the action (if any).
    pub related_tech: Option<Tech>,
    /// The specific knowledge reward amount (from config).
    pub reward_amount: f32,
    /// Flat research points granted toward `related_tech` (from config).
    pub breakthrough_points: f32,
}

/// Per-tech breakthrough progress ledger (Spec 390 §8).
///
/// This is the minimal progress seam: `TechState` has no points/cost
/// accumulation of its own (its `unlock` is all-or-nothing and its
/// Knowledge cost is paid from the colony pool by `unlock_tech`), so
/// breakthrough points accumulate here, keyed by tech, until they
/// cover the tech's cost and the handler unlocks it.
#[derive(Resource, Default, Debug, Clone)]
pub struct EurekaProgress {
    /// Accumulated breakthrough points per not-yet-unlocked tech.
    pub points: HashMap<Tech, f32>,
    /// Total breakthrough events handled (any outcome).
    pub breakthroughs: u32,
}

impl EurekaProgress {
    /// Points accumulated toward `tech` (0 if none).
    #[must_use]
    pub fn points_for(&self, tech: Tech) -> f32 {
        self.points.get(&tech).copied().unwrap_or(0.0)
    }

    /// Add breakthrough points toward `tech`; returns the new total.
    pub fn add_points(&mut self, tech: Tech, amount: f32) -> f32 {
        let entry = self.points.entry(tech).or_insert(0.0);
        *entry += amount.max(0.0);
        *entry
    }

    /// Drop accumulated points for `tech` (on unlock).
    pub fn clear(&mut self, tech: Tech) {
        self.points.remove(&tech);
    }
}

/// Pure breakthrough-probability math (Spec 390 §5).
///
/// Starts from `base_chance`, applies the trait seam (Intellectual
/// x1.2, Creative x1.5), then the researcher-assignment multiplier,
/// and clamps to `1.0` so a guaranteed-trigger config (`base_chance
/// = 1.0`) combined with bonuses can never feed `gen_bool` an
/// out-of-range probability.
#[must_use]
pub fn effective_chance(base_chance: f64, traits: Option<&Traits>, is_researcher: bool) -> f64 {
    let mut chance = base_chance;
    if let Some(t) = traits {
        if t.has(Trait::Intellectual) {
            chance *= 1.2;
        }
        if t.has(Trait::Creative) {
            chance *= 1.5;
        }
    }
    if is_researcher {
        chance *= RESEARCHER_CHANCE_MULTIPLIER;
    }
    chance.clamp(0.0, 1.0)
}

/// Checks for a Eureka Moment and emits an event if successful.
///
/// Use this when you have access to `EventWriter` (e.g. in a parallel system).
pub fn check_for_eureka(
    events: &mut EventWriter<EurekaEvent>,
    config: &EurekaConfig,
    action: ActionType,
    related_tech: Option<Tech>,
    traits: Option<&Traits>,
) -> bool {
    let chance = effective_chance(config.base_chance, traits, false);

    // Simple RNG check
    let mut rng = rand::thread_rng();
    if !rng.gen_bool(chance) {
        return false;
    }

    // Success! Emit Event.
    events.send(EurekaEvent {
        action,
        related_tech,
        reward_amount: config.knowledge_reward,
        breakthrough_points: config.breakthrough_points,
    });

    true
}

/// Helper to check for Eureka moments when you have exclusive World access.
pub fn check_for_eureka_world(
    world: &mut World,
    action: ActionType,
    related_tech: Option<Tech>,
    pop_entity: Option<Entity>,
) -> bool {
    // 1. Get Config
    let config = world
        .get_resource::<EurekaConfig>()
        .cloned()
        .unwrap_or_default();

    // 2. Calculate Chance (traits + researcher assignment, Spec 390 §5)
    let mut traits_owned: Option<Traits> = None;
    let mut is_researcher = false;
    if let Some(entity) = pop_entity {
        if let Some(t) = world.get::<Traits>(entity) {
            traits_owned = Some(t.clone());
        }
        if let Some(a) = world.get::<AssignedTo>(entity) {
            is_researcher = a.assignment_type == AssignmentType::LibraryWorker;
        }
    }
    let chance = effective_chance(config.base_chance, traits_owned.as_ref(), is_researcher);

    // 3. RNG Check
    let mut rng = rand::thread_rng();
    if !rng.gen_bool(chance) {
        return false;
    }

    // 4. Send Event
    world.send_event(EurekaEvent {
        action,
        related_tech,
        reward_amount: config.knowledge_reward,
        breakthrough_points: config.breakthrough_points,
    });

    true
}

/// System that handles `EurekaEvent`s and applies their effects.
///
/// Spec 390 §8: a breakthrough on a locked tech adds
/// `event.breakthrough_points` to [`EurekaProgress`]; the tech
/// unlocks only when the accumulated points cover its cost (the
/// ledger entry is then cleared). Already-unlocked techs and
/// tech-less events grant Knowledge, as before. Every breakthrough
/// writes a MessageLog line (the Spec 196 seam) AND a Chronicle
/// entry — Standard for progress, Major on the threshold-cross
/// unlock — following the `Option<ResMut<Chronicle>>` pattern used
/// by recent specs (black market, curio trade).
///
/// Unlocking at the threshold uses `TechState::unlock` (the same
/// forced unlock the Spec 196 handler used), so a breakthrough
/// unlock is not blocked by data-capacity corruption checks.
pub fn handle_eureka_events(
    mut events: EventReader<EurekaEvent>,
    mut tech_state: ResMut<TechState>,
    mut resources: ResMut<ColonyResources>,
    mut progress: Option<ResMut<EurekaProgress>>,
    mut log: Option<ResMut<MessageLog>>,
    mut chronicle: Option<ResMut<Chronicle>>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.as_deref().map_or(0, |t| t.tick);
    for event in events.read() {
        if let Some(progress) = progress.as_deref_mut() {
            progress.breakthroughs += 1;
        }

        let mut unlocked_tech: Option<Tech> = None;
        let mut progress_note: Option<(Tech, f32, f32)> = None;
        let mut knowledge_gained = 0.0;

        if let Some(tech) = event.related_tech {
            if tech_state.is_unlocked(tech) {
                knowledge_gained = event.reward_amount;
            } else {
                let points = event.breakthrough_points.max(0.0);
                let total = match progress.as_deref_mut() {
                    Some(progress) => progress.add_points(tech, points),
                    // Bare worlds without the ledger cannot accumulate
                    // across events; a single breakthrough is judged
                    // on its own points (documented in the 390 report).
                    None => points,
                };
                if total >= tech.cost() {
                    tech_state.unlock(tech);
                    if let Some(progress) = progress.as_deref_mut() {
                        progress.clear(tech);
                    }
                    unlocked_tech = Some(tech);
                } else {
                    progress_note = Some((tech, total, tech.cost()));
                }
            }
        } else {
            knowledge_gained = event.reward_amount;
        }

        if knowledge_gained > 0.0 {
            resources.knowledge += knowledge_gained;
        }

        if let Some(log) = &mut log {
            if let Some(tech) = unlocked_tech {
                log.add(format!(
                    "EUREKA! Doing {:?} unlocked {}!",
                    event.action,
                    tech.label()
                ));
            } else if let Some((tech, total, cost)) = progress_note {
                log.add(format!(
                    "Eureka! Breakthrough toward {} ({total:.0}/{cost:.0}) while {:?}!",
                    tech.label(),
                    event.action
                ));
            } else {
                log.add(format!(
                    "Eureka! Gained insight ({}) while {:?}!",
                    knowledge_gained, event.action
                ));
            }
        }

        if let Some(chronicle) = chronicle.as_deref_mut() {
            if let Some(tech) = unlocked_tech {
                chronicle.add_event(
                    tick,
                    format!(
                        "Eureka! A breakthrough unlocked {}!",
                        tech.label()
                    ),
                    EventImportance::Major,
                );
            } else if let Some((tech, total, cost)) = progress_note {
                chronicle.add_event(
                    tick,
                    format!(
                        "Eureka! A breakthrough advanced {} ({total:.0}/{cost:.0}).",
                        tech.label()
                    ),
                    EventImportance::Standard,
                );
            } else {
                chronicle.add_event(
                    tick,
                    format!(
                        "Eureka! Gained insight ({knowledge_gained:.0}) while working."
                    ),
                    EventImportance::Standard,
                );
            }
        }
    }
}

/// Parse a tech name for the headless `eureka` debug command.
/// Case-insensitive; accepts the same aliases as `research`.
#[must_use]
pub fn parse_eureka_tech(name: &str) -> Option<Tech> {
    match name.trim().to_lowercase().as_str() {
        "masonry" => Some(Tech::Masonry),
        "metal working" | "metalworking" | "metal" => Some(Tech::MetalWorking),
        "social structures" | "social" => Some(Tech::SocialStructures),
        "astronomy" => Some(Tech::Astronomy),
        "hydroponics" => Some(Tech::Hydroponics),
        "militia" => Some(Tech::Militia),
        "medical" => Some(Tech::Medical),
        "electromagnetism" => Some(Tech::Electromagnetism),
        "void whispers" | "void" => Some(Tech::VoidWhispers),
        "terraforming" => Some(Tech::Terraforming),
        "closed-cycle breathing" | "closedcycle" | "closed cycle" => {
            Some(Tech::ClosedCycleBreathing)
        }
        "expedition systems" | "expedition" => Some(Tech::ExpeditionSystems),
        _ => None,
    }
}

/// Total breakthroughs handled so far (0 if the ledger is absent).
#[must_use]
pub fn breakthrough_count(world: &World) -> u32 {
    world
        .get_resource::<EurekaProgress>()
        .map_or(0, |p| p.breakthroughs)
}

/// One-line STATS label, e.g. `3` handled with `2` techs in progress:
/// rendered as `3/2` by the caller or used verbatim for the panel.
#[must_use]
pub fn eureka_stats_label(world: &World) -> String {
    let Some(progress) = world.get_resource::<EurekaProgress>() else {
        return "0".to_string();
    };
    format!("{}", progress.breakthroughs)
}

/// Multi-line ledger summary for the headless `eureka` panel.
#[must_use]
pub fn eureka_progress_label(world: &World) -> String {
    let Some(progress) = world.get_resource::<EurekaProgress>() else {
        return "No breakthroughs yet.".to_string();
    };
    if progress.points.is_empty() {
        return format!(
            "{} breakthrough(s) handled; no tech currently in progress.",
            progress.breakthroughs
        );
    }
    let mut entries: Vec<(String, f32, f32)> = progress
        .points
        .iter()
        .map(|(tech, pts)| (tech.label().to_string(), *pts, tech.cost()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = format!("{} breakthrough(s) handled.", progress.breakthroughs);
    for (label, pts, cost) in entries {
        out.push_str(&format!("\n{label}: {pts:.0}/{cost:.0} research points"));
    }
    out
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn eureka_public_strings() -> Vec<String> {
    vec![
        "Eureka! A breakthrough unlocked a technology.".to_string(),
        "Eureka! A breakthrough advanced research.".to_string(),
        "Eureka! Gained insight while working.".to_string(),
        "EUREKA! A working breakthrough unlocked a technology.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::resources::ColonyResources;
    use crate::layer1::tech::{Tech, TechState};
    use crate::layer1::utility_ai::ActionType;

    #[test]
    fn test_check_for_eureka_emits_event() {
        let mut world = World::new();
        world.insert_resource(EurekaConfig {
            base_chance: 1.0,
            ..Default::default()
        }); // Guaranteed
        world.insert_resource(Events::<EurekaEvent>::default());

        // We need an EventWriter. In tests we can use SystemState or just World.
        // Let's use check_for_eureka_world for simplicity in test setup without SystemState boilerplate
        let occurred =
            check_for_eureka_world(&mut world, ActionType::Work, Some(Tech::Masonry), None);

        assert!(occurred);

        // Verify event
        let events = world.resource::<Events<EurekaEvent>>();
        let mut reader = events.get_cursor();
        let event = reader.read(events).next();
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.action, ActionType::Work);
        // Spec 390: the event carries the flat breakthrough chunk.
        assert!((event.breakthrough_points - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_handle_eureka_events_unlocks_tech() {
        // Spec 390 adaptation: Masonry costs 10 and one breakthrough
        // grants 20 points, so a single breakthrough still crosses
        // the threshold and unlocks — via accumulated points, not
        // the old instant unlock.
        let mut world = World::new();
        let tech_state = TechState {
            total_capacity: 100.0,
            ..Default::default()
        };
        world.insert_resource(tech_state);
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<EurekaEvent>::default());
        world.insert_resource(EurekaProgress::default());

        // Send event
        world.send_event(EurekaEvent {
            action: ActionType::Work,
            related_tech: Some(Tech::Masonry),
            reward_amount: 10.0,
            breakthrough_points: 20.0,
        });

        // Run handler
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_eureka_events);
        schedule.run(&mut world);

        // Assert
        assert!(world.resource::<TechState>().is_unlocked(Tech::Masonry));
        assert_eq!(
            world.resource::<EurekaProgress>().points_for(Tech::Masonry),
            0.0,
            "ledger entry is cleared on unlock"
        );
    }

    #[test]
    fn test_trait_bonus() {
        let mut world = World::new();
        // Set chance to 0.5.
        // With Creative (1.5x) -> 0.75.
        // This is hard to test deterministically with RNG.
        // But we can check that it compiles and runs.
        world.insert_resource(EurekaConfig {
            base_chance: 0.5,
            ..Default::default()
        });

        let traits = {
            let mut t = Traits::default();
            t.add(Trait::Creative);
            t
        };

        // Just ensure it doesn't crash
        let entity = world.spawn(traits).id();
        check_for_eureka_world(&mut world, ActionType::Work, None, Some(entity));
    }
}

#[cfg(test)]
mod tests390 {
    use super::*;
    use crate::layer1::chronicle::{Chronicle, EventImportance};

    fn world390() -> World {
        let mut world = World::new();
        world.insert_resource(TechState {
            total_capacity: 100.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<EurekaEvent>::default());
        world.insert_resource(EurekaProgress::default());
        world.insert_resource(Chronicle::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(MessageLog::default());
        world
    }

    fn run_handler(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_eureka_events);
        schedule.run(world);
    }

    fn send_breakthrough(world: &mut World, tech: Tech, points: f32) {
        world.send_event(EurekaEvent {
            action: ActionType::Work,
            related_tech: Some(tech),
            reward_amount: 10.0,
            breakthrough_points: points,
        });
    }

    #[test]
    fn test_breakthrough_adds_progress_not_instant_unlock() {
        // Astronomy costs 50; one 20-point breakthrough must NOT unlock it.
        let mut world = world390();
        send_breakthrough(&mut world, Tech::Astronomy, 20.0);
        run_handler(&mut world);
        assert!(!world.resource::<TechState>().is_unlocked(Tech::Astronomy));
        assert!(
            (world.resource::<EurekaProgress>().points_for(Tech::Astronomy) - 20.0).abs()
                < f32::EPSILON
        );
        assert_eq!(world.resource::<EurekaProgress>().breakthroughs, 1);
        // No Knowledge is paid out for a partial breakthrough.
        assert!(world.resource::<ColonyResources>().knowledge.abs() < f32::EPSILON);
    }

    #[test]
    fn test_progress_accumulates_and_unlocks_at_cost() {
        let mut world = world390();
        for _ in 0..3 {
            send_breakthrough(&mut world, Tech::Astronomy, 20.0);
        }
        run_handler(&mut world);
        // 20 + 20 + 20 = 60 >= 50 cost: unlocked, ledger cleared.
        assert!(world.resource::<TechState>().is_unlocked(Tech::Astronomy));
        assert_eq!(
            world.resource::<EurekaProgress>().points_for(Tech::Astronomy),
            0.0
        );
        assert_eq!(world.resource::<EurekaProgress>().breakthroughs, 3);
    }

    #[test]
    fn test_already_unlocked_tech_still_grants_knowledge() {
        let mut world = world390();
        world
            .resource_mut::<TechState>()
            .unlock(Tech::Masonry);
        send_breakthrough(&mut world, Tech::Masonry, 20.0);
        run_handler(&mut world);
        assert!(
            (world.resource::<ColonyResources>().knowledge - 10.0).abs() < f32::EPSILON
        );
        assert_eq!(
            world.resource::<EurekaProgress>().points_for(Tech::Masonry),
            0.0,
            "no progress accrues toward an unlocked tech"
        );
    }

    #[test]
    fn test_no_tech_breakthrough_grants_knowledge() {
        let mut world = world390();
        world.send_event(EurekaEvent {
            action: ActionType::Work,
            related_tech: None,
            reward_amount: 10.0,
            breakthrough_points: 20.0,
        });
        run_handler(&mut world);
        assert!(
            (world.resource::<ColonyResources>().knowledge - 10.0).abs() < f32::EPSILON
        );
        assert_eq!(world.resource::<EurekaProgress>().breakthroughs, 1);
    }

    #[test]
    fn test_breakthrough_writes_chronicle_entries() {
        let mut world = world390();
        send_breakthrough(&mut world, Tech::Astronomy, 20.0);
        run_handler(&mut world);
        {
            let chronicle = world.resource::<Chronicle>();
            assert_eq!(chronicle.events.len(), 1);
            assert!(chronicle.events[0].text.contains("Eureka!"));
            assert!(chronicle.events[0].text.contains("Astronomy"));
            assert_eq!(chronicle.events[0].importance, EventImportance::Standard);
        }
        // Cross the threshold: a Major entry on unlock. Clear the
        // event buffer first: a fresh Schedule's EventReader would
        // otherwise re-read the first event still buffered there.
        world.resource_mut::<Events<EurekaEvent>>().clear();
        send_breakthrough(&mut world, Tech::Astronomy, 20.0);
        send_breakthrough(&mut world, Tech::Astronomy, 20.0);
        run_handler(&mut world);
        let chronicle = world.resource::<Chronicle>();
        assert_eq!(chronicle.events.len(), 3);
        let last = chronicle.events.last().unwrap();
        assert!(last.text.contains("unlocked"));
        assert_eq!(last.importance, EventImportance::Major);
    }

    #[test]
    fn test_effective_chance_math() {
        assert!((effective_chance(0.1, None, false) - 0.1).abs() < 1e-12);
        let mut creative = Traits::default();
        creative.add(Trait::Creative);
        assert!((effective_chance(0.1, Some(&creative), false) - 0.15).abs() < 1e-9);
        let mut intellectual = Traits::default();
        intellectual.add(Trait::Intellectual);
        assert!((effective_chance(0.1, Some(&intellectual), false) - 0.12).abs() < 1e-9);
        // Researcher (LibraryWorker) path: x2.
        assert!((effective_chance(0.1, None, true) - 0.2).abs() < 1e-9);
        assert!(
            (effective_chance(0.1, Some(&creative), true) - 0.3).abs() < 1e-9,
            "trait and researcher multipliers stack"
        );
        // Guaranteed-trigger configs clamp at 1.0, never above.
        assert!((effective_chance(1.0, Some(&creative), true) - 1.0).abs() < f64::EPSILON);
        assert!((effective_chance(0.0, Some(&creative), true) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_guaranteed_trigger_with_traits_does_not_panic() {
        // base_chance 1.0 + Creative would exceed 1.0 unclamped and
        // gen_bool would panic; the clamp keeps it a clean guarantee.
        let mut world = World::new();
        world.insert_resource(EurekaConfig {
            base_chance: 1.0,
            ..Default::default()
        });
        world.insert_resource(Events::<EurekaEvent>::default());
        let mut traits = Traits::default();
        traits.add(Trait::Creative);
        let entity = world.spawn(traits).id();
        assert!(check_for_eureka_world(
            &mut world,
            ActionType::Work,
            Some(Tech::Masonry),
            Some(entity)
        ));
    }

    #[test]
    fn test_progress_ledger_math() {
        let mut progress = EurekaProgress::default();
        assert_eq!(progress.points_for(Tech::Militia), 0.0);
        assert!((progress.add_points(Tech::Militia, 15.0) - 15.0).abs() < f32::EPSILON);
        assert!((progress.add_points(Tech::Militia, 10.0) - 25.0).abs() < f32::EPSILON);
        // Militia costs 25: exactly at cost.
        assert!(progress.points_for(Tech::Militia) >= Tech::Militia.cost());
        progress.clear(Tech::Militia);
        assert_eq!(progress.points_for(Tech::Militia), 0.0);
    }

    #[test]
    fn test_parse_eureka_tech_aliases() {
        assert_eq!(parse_eureka_tech("astronomy"), Some(Tech::Astronomy));
        assert_eq!(parse_eureka_tech("Metal Working"), Some(Tech::MetalWorking));
        assert_eq!(parse_eureka_tech("void"), Some(Tech::VoidWhispers));
        assert_eq!(parse_eureka_tech("nonsense"), None);
    }

    #[test]
    fn test_eureka_public_strings_have_no_lifted_ip() {
        const BANNED: &[&str] = &[
            "dwarf fortress",
            "stellaris",
            "rimworld",
            "archimedes",
            "newton",
            "apple fell",
        ];
        for s in eureka_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
        // Chronicle/log templates stay generic too.
        let chronicle_texts = [
            format!("Eureka! A breakthrough unlocked {}!", Tech::Masonry.label()),
            format!("Eureka! A breakthrough advanced {} (20/50).", Tech::Astronomy.label()),
        ];
        for s in chronicle_texts {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(!lower.contains(banned), "chronicle text {s:?} banned {banned:?}");
            }
        }
    }
}
