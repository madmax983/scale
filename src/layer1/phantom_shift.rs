//! The Phantom Shift (Spec 290).
//!
//! Under conditions of extreme systemic inefficiency (massive job backlogs,
//! neglected infrastructure), a "Phantom Shift" emerges. Pops carrying the
//! `"Fringe"` cultural tag (see
//! [`CulturalDrift`](crate::layer1::social::feral_outpost::CulturalDrift))
//! secretly work during the night cycle. They fix damaged structures, but they
//! consume colony resources without logging them and slowly build an
//! invisible, untaxable shadow economy.
//!
//! This creates the spec's central tension: free automated labor versus the
//! loss of economic transparency.
//!
//! ## Mechanics
//!
//! * **Trigger:** Nighttime + Fringe pop + colony backlog score at or above
//!   [`PhantomShiftConfig::inefficiency_threshold`].
//! * **Evaluation:** [`evaluate_phantom_shift`] scores
//!   [`ActionType::PhantomWork`] against damaged structures. It outbids
//!   ordinary rest but loses to critical exhaustion, so phantom workers still
//!   sleep when collapsing.
//! * **Execution:** [`phantom_work_execution_system`] repairs the target
//!   structure, draws materials from [`ColonyResources`] silently (no ledger,
//!   no chronicle, no wages), accrues [`ShadowEconomy::unlogged_value`], and
//!   drains rest (they labor instead of sleeping).
//! * **Tracking:** [`update_inefficiency_tracker_system`] keeps
//!   [`ColonyInefficiencyTracker::current_backlog_score`] fresh for UI/debug.
//!
//! All names are original and generic; nothing is lifted from outside fiction.

use bevy_ecs::prelude::*;

use crate::layer1::administration::designation::Designation;
use crate::layer1::architecture::structure::Structure;
use crate::layer1::day_night::TimeOfDay;
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::mind::utility_eval_types::{
    PopEvalData, UtilityAIBuffer, WorldContext, evaluate_candidates,
};
use crate::layer1::mind::utility_types::{ActionType, PopAction};
use crate::layer1::pop::Pop;
use crate::layer1::psychology::needs::Needs;

/// Hit points restored to a damaged structure per phantom-work tick.
pub const PHANTOM_REPAIR_PER_TICK: f32 = 2.0;
/// Building material silently consumed per phantom repair tick.
pub const PHANTOM_MATERIAL_COST: f32 = 0.5;
/// Shadow-economy value accrued per phantom repair tick.
pub const PHANTOM_WORK_VALUE: f32 = 1.0;
/// Rest drained per phantom-work tick (they labor instead of sleeping).
pub const PHANTOM_REST_DRAIN: f32 = 0.01;

/// Tuning for the phantom shift.
#[derive(Resource, Debug, Clone, Copy)]
pub struct PhantomShiftConfig {
    /// Backlog score (unfulfilled designations per 100 workers) at or above
    /// which the phantom shift can trigger.
    pub inefficiency_threshold: f32,
    /// Utility bid for PhantomWork. Beats ordinary rest urgency (0.5-1.0)
    /// but loses to critical exhaustion (~1.5), so collapsing pops sleep.
    pub phantom_utility: f32,
}

impl Default for PhantomShiftConfig {
    fn default() -> Self {
        Self {
            inefficiency_threshold: 80.0,
            phantom_utility: 1.35,
        }
    }
}

/// Live measure of colony dysfunction: unfulfilled designations per 100 pops.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ColonyInefficiencyTracker {
    /// Current backlog score (designations per 100 pops).
    pub current_backlog_score: f32,
}

/// The invisible economy: value of work done off the books.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ShadowEconomy {
    /// Cumulative value of unlogged phantom labor.
    pub unlogged_value: f32,
    /// Total ticks of phantom work performed.
    pub total_phantom_ticks: u64,
}

/// Computes the backlog score from the AI buffer: unfulfilled designations
/// per 100 evaluated pops.
#[must_use]
pub fn backlog_score(buffer: &UtilityAIBuffer) -> f32 {
    let designations = buffer.work_designations.len()
        + buffer.repair_designations.len()
        + buffer.tame_designations.len();
    let workers = buffer.pop_data.len().max(1);
    #[allow(clippy::cast_precision_loss)]
    let score = designations as f32 / workers as f32 * 100.0;
    score
}

/// Scores PhantomWork for a single pop.
///
/// Returns `Some((utility, target))` when all trigger conditions hold:
/// night, Fringe tag, and backlog at/above threshold. Targets the nearest
/// damaged structure from the AI buffer's repair candidates.
#[must_use]
pub fn evaluate_phantom_shift(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
    context: &WorldContext,
) -> Option<(f32, Entity)> {
    evaluate_phantom_shift_with_config(data, buffer, context, &PhantomShiftConfig::default())
}

/// [`evaluate_phantom_shift`] with an explicit config (for tests/tuning).
#[must_use]
pub fn evaluate_phantom_shift_with_config(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
    context: &WorldContext,
    config: &PhantomShiftConfig,
) -> Option<(f32, Entity)> {
    // Only under cover of night.
    if context.cycle.time_of_day != TimeOfDay::Night {
        return None;
    }
    // Only the culturally drifted answer the phantom call.
    if !data.is_fringe {
        return None;
    }
    // Only when the colony is drowning in unfulfilled work.
    if backlog_score(buffer) < config.inefficiency_threshold {
        return None;
    }
    // Fix what the day shift neglected: nearest damaged structure.
    let (utility, target) = evaluate_candidates(
        data.pos,
        &data.weights,
        &buffer.repair_structures,
        config.phantom_utility,
    )?;
    Some((utility, target))
}

/// Refreshes [`ColonyInefficiencyTracker`] from live designation/pop counts.
pub fn update_inefficiency_tracker_system(
    designations: Query<&Designation>,
    pops: Query<&Pop>,
    tracker: Option<ResMut<ColonyInefficiencyTracker>>,
) {
    let Some(mut tracker) = tracker else { return };
    let d = designations.iter().count();
    let p = pops.iter().count().max(1);
    #[allow(clippy::cast_precision_loss)]
    let score = d as f32 / p as f32 * 100.0;
    tracker.current_backlog_score = score;
}

/// Executes phantom work: silent repairs that bypass all logging.
///
/// Pops with `PopAction.current == PhantomWork` who have arrived at their
/// target repair the damaged structure, consume materials from
/// [`ColonyResources`] without any ledger/chronicle entry, accrue
/// [`ShadowEconomy`] value, and lose rest (they labor instead of sleeping).
/// Fully repaired (or vanished) targets release the worker back to idle for
/// re-evaluation.
pub fn phantom_work_execution_system(world: &mut World) {
    // Collect first to avoid borrow conflicts with the repair writes below.
    let workers: Vec<(Entity, Entity)> = world
        .query_filtered::<(Entity, &PopAction, &MovementTarget), (With<AtTarget>, With<Pop>)>()
        .iter(world)
        .filter(|(_, action, _)| action.current == ActionType::PhantomWork)
        .map(|(e, _, mt)| (e, mt.target_entity))
        .collect();

    for (pop_entity, target_entity) in workers {
        let mut repaired = false;
        if let Some(mut structure) = world.get_mut::<Structure>(target_entity) {
            if structure.current_hp < structure.max_hp {
                structure.current_hp =
                    (structure.current_hp + PHANTOM_REPAIR_PER_TICK).min(structure.max_hp);
                repaired = true;
            }
        }

        if repaired {
            // Silent material draw - no ledger, no chronicle, no wage.
            if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
                resources.consume(ResourceType::Stone, PHANTOM_MATERIAL_COST);
            }
            if let Some(mut shadow) = world.get_resource_mut::<ShadowEconomy>() {
                shadow.unlogged_value += PHANTOM_WORK_VALUE;
                shadow.total_phantom_ticks += 1;
            }
            if let Some(mut needs) = world.get_mut::<Needs>(pop_entity) {
                needs.rest = (needs.rest - PHANTOM_REST_DRAIN).max(0.0);
            }
        }

        // Release the worker when the target is fixed (or gone).
        let done = world
            .get::<Structure>(target_entity)
            .is_none_or(|s| (s.current_hp - s.max_hp).abs() < f32::EPSILON);
        if done {
            if let Some(mut action) = world.get_mut::<PopAction>(pop_entity) {
                action.current = ActionType::Idle;
                action.current_utility = 0.0;
            }
            world.entity_mut(pop_entity).remove::<AtTarget>();
            world.entity_mut(pop_entity).remove::<MovementTarget>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::day_night::DayNightCycle;
    use crate::layer1::map::GridPosition;
    use crate::layer1::mind::utility_eval_types::ScorableCandidate;

    fn test_world_context(world: &mut World) -> WorldContext<'_> {
        test_world_context_at(world, TimeOfDay::Night)
    }

    fn test_world_context_at(world: &mut World, tod: TimeOfDay) -> WorldContext<'_> {
        world.insert_resource(ColonyResources::default());
        world.insert_resource(DayNightCycle { time_of_day: tod, ..DayNightCycle::default() });
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(1, 1));
        let resources = world.resource::<ColonyResources>();
        let cycle = world.resource::<DayNightCycle>();
        let taboo = world.resource::<crate::layer1::taboo::TabooState>();
        let zone_grid = world.resource::<crate::layer1::zone::ZoneGrid>();
        WorldContext {
            resources,
            cycle,
            taboo,
            factions: None,
            zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        }
    }

    fn fringe_pop_data() -> PopEvalData {
        let mut data = PopEvalData::test_instance();
        data.is_fringe = true;
        data
    }

    fn high_backlog_buffer(structure_entity: Entity) -> UtilityAIBuffer {
        // 100 designations vs 1 pop = score 10000, well above threshold.
        let work_designations = (0..100)
            .map(|i| ScorableCandidate::new(Entity::from_raw(i), GridPosition::default()))
            .collect();
        let candidate = ScorableCandidate::new(structure_entity, GridPosition { x: 5, y: 0 });
        UtilityAIBuffer {
            work_designations,
            pop_data: vec![PopEvalData::test_instance()],
            repair_structures: vec![candidate],
            ..UtilityAIBuffer::default()
        }
    }

    #[test]
    fn test_phantom_shift_config_defaults() {
        let config = PhantomShiftConfig::default();
        assert_eq!(config.inefficiency_threshold, 80.0);
        assert!(config.phantom_utility > 1.0, "must outbid ordinary rest");
        assert!(config.phantom_utility < 1.5, "must lose to critical exhaustion");
    }

    #[test]
    fn test_backlog_score_scales_with_designations() {
        // 8 designations / 10 pops * 100 = 80.0
        let work_designations = (0..8)
            .map(|i| ScorableCandidate::new(Entity::from_raw(i), GridPosition::default()))
            .collect();
        let buffer = UtilityAIBuffer {
            pop_data: vec![PopEvalData::test_instance(); 10],
            work_designations,
            ..UtilityAIBuffer::default()
        };
        let score = backlog_score(&buffer);
        assert!((score - 80.0).abs() < f32::EPSILON, "score was {score}");
    }

    #[test]
    fn test_backlog_score_empty_is_zero() {
        let buffer = UtilityAIBuffer::default();
        assert_eq!(backlog_score(&buffer), 0.0);
    }

    #[test]
    fn test_phantom_activates_at_night_for_fringe_when_inefficient() {
        let mut world = World::new();
        let target = world
            .spawn((Structure { current_hp: 50.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let ctx = test_world_context(&mut world);
        let data = fringe_pop_data();
        let buffer = high_backlog_buffer(target);

        let result = evaluate_phantom_shift(&data, &buffer, &ctx);
        assert!(result.is_some(), "phantom shift should trigger");
        let (utility, chosen) = result.unwrap();
        assert_eq!(chosen, target);
        assert!(utility > 0.0);
    }

    #[test]
    fn test_phantom_does_not_activate_during_day() {
        let mut world = World::new();
        let target = world
            .spawn((Structure { current_hp: 50.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let ctx = test_world_context_at(&mut world, TimeOfDay::Day);
        let data = fringe_pop_data();
        let buffer = high_backlog_buffer(target);

        assert!(evaluate_phantom_shift(&data, &buffer, &ctx).is_none());
    }

    #[test]
    fn test_phantom_does_not_activate_for_non_fringe() {
        let mut world = World::new();
        let target = world
            .spawn((Structure { current_hp: 50.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let ctx = test_world_context(&mut world);
        let mut data = PopEvalData::test_instance();
        data.is_fringe = false;
        let buffer = high_backlog_buffer(target);

        assert!(evaluate_phantom_shift(&data, &buffer, &ctx).is_none());
    }

    #[test]
    fn test_phantom_does_not_activate_when_efficient() {
        let mut world = World::new();
        let target = world
            .spawn((Structure { current_hp: 50.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let ctx = test_world_context(&mut world);
        let data = fringe_pop_data();
        let mut buffer = high_backlog_buffer(target);
        buffer.work_designations.clear(); // no backlog

        assert!(evaluate_phantom_shift(&data, &buffer, &ctx).is_none());
    }

    #[test]
    fn test_phantom_needs_a_damaged_target() {
        let mut world = World::new();
        let ctx = test_world_context(&mut world);
        let data = fringe_pop_data();
        let mut buffer = high_backlog_buffer(Entity::from_raw(999));
        buffer.repair_structures.clear(); // nothing to fix

        assert!(evaluate_phantom_shift(&data, &buffer, &ctx).is_none());
    }

    #[test]
    fn test_phantom_repairs_silently_and_grows_shadow_economy() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        world.insert_resource(ShadowEconomy::default());
        // Give the colony some stone to siphon.
        world.resource_mut::<ColonyResources>().stone = 100.0;

        let structure = world
            .spawn((Structure { current_hp: 50.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let pop = world
            .spawn((
                Pop,
                PopAction { current: ActionType::PhantomWork, current_utility: 1.35, ticks_committed: 5 },
                MovementTarget {
                    target_entity: structure,
                    target_position: GridPosition { x: 5, y: 0 },
                    for_action: ActionType::PhantomWork,
                },
                AtTarget,
                Needs { hunger: 0.8, rest: 0.8, leisure: 0.8, hygiene: 0.8 },
                GridPosition { x: 5, y: 0 },
            ))
            .id();

        phantom_work_execution_system(&mut world);

        // Structure repaired.
        let s = world.get::<Structure>(structure).unwrap();
        assert!((s.current_hp - 52.0).abs() < f32::EPSILON, "hp was {}", s.current_hp);
        // Materials siphoned silently.
        let stone = world.resource::<ColonyResources>().stone;
        assert!((stone - 99.5).abs() < f32::EPSILON, "stone was {stone}");
        // Shadow economy grew (no chronicle event possible in this bare world).
        let shadow = world.resource::<ShadowEconomy>();
        assert_eq!(shadow.unlogged_value, PHANTOM_WORK_VALUE);
        assert_eq!(shadow.total_phantom_ticks, 1);
        // Rest drained: they worked instead of sleeping.
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(needs.rest < 0.8, "rest was {}", needs.rest);
    }

    #[test]
    fn test_phantom_releases_worker_when_structure_fixed() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        world.insert_resource(ShadowEconomy::default());

        // Nearly fixed: one tick completes it.
        let structure = world
            .spawn((Structure { current_hp: 99.0, max_hp: 100.0 }, GridPosition { x: 5, y: 0 }))
            .id();
        let pop = world
            .spawn((
                Pop,
                PopAction { current: ActionType::PhantomWork, current_utility: 1.35, ticks_committed: 5 },
                MovementTarget {
                    target_entity: structure,
                    target_position: GridPosition { x: 5, y: 0 },
                    for_action: ActionType::PhantomWork,
                },
                AtTarget,
                GridPosition { x: 5, y: 0 },
            ))
            .id();

        phantom_work_execution_system(&mut world);

        let s = world.get::<Structure>(structure).unwrap();
        assert_eq!(s.current_hp, 100.0);
        let action = world.get::<PopAction>(pop).unwrap();
        assert_eq!(action.current, ActionType::Idle, "worker should be released");
        assert!(world.get::<AtTarget>(pop).is_none());
        assert!(world.get::<MovementTarget>(pop).is_none());
    }

    #[test]
    fn test_inefficiency_tracker_system_updates_score() {
        let mut world = World::new();
        world.insert_resource(ColonyInefficiencyTracker::default());
        // 4 designations, 2 pops -> 200.0
        for i in 0..4 {
            world.spawn((
                Designation { designation_type: crate::layer1::designation::DesignationType::Mine },
                GridPosition { x: i, y: 0 },
            ));
        }
        for _ in 0..2 {
            world.spawn(Pop);
        }

        let mut schedule = Schedule::default();
        schedule.add_systems(update_inefficiency_tracker_system);
        schedule.run(&mut world);

        let tracker = world.resource::<ColonyInefficiencyTracker>();
        assert!((tracker.current_backlog_score - 200.0).abs() < f32::EPSILON,
            "score was {}", tracker.current_backlog_score);
    }
}
