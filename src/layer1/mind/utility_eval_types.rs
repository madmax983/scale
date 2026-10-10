//! Data structures for the "Gather" phase of the Utility AI.
//!
//! This module defines the types used to extract state from the ECS world
//! and prepare it for lock-free parallel evaluation.
//!
//! # Core Concept: The Universal Target
//!
//! Instead of having specific target types (e.g., `FarmTarget`, `HospitalTarget`),
//! all potential interaction points in the colony are flattened into [`ScorableCandidate`]s.
//! This allows a single evaluation function ([`evaluate_candidates`]) to score
//! completely different entities using the same underlying math.
//!
//! # Context and State
//!
//! - [`PopEvalData`]: The specific state of a single Pop (needs, traits, inventory) at the start of the tick.
//! - [`WorldContext`]: Global state (time of day, resource totals, laws) passed into the evaluation functions.
//! - [`UtilityAIBuffer`]: The massive memory buffer that holds all candidates and Pops for a single tick.
//!
//! [`ScorableCandidate`]: crate::layer1::mind::utility_eval_types::ScorableCandidate
//! [`evaluate_candidates`]: crate::layer1::mind::utility_eval_types::evaluate_candidates
//! [`PopEvalData`]: crate::layer1::mind::utility_eval_types::PopEvalData
//! [`WorldContext`]: crate::layer1::mind::utility_eval_types::WorldContext
//! [`UtilityAIBuffer`]: crate::layer1::mind::utility_eval_types::UtilityAIBuffer
//!
//! # Examples
//!
//! Building a candidate:
//!
//! ```ignore
//! // `ScorableCandidate` is used internally during the gather phase:
//! use bevy_ecs::prelude::Entity;
//! use scale::layer1::map::GridPosition;
//! use scale::layer1::mind::utility_eval_types::ScorableCandidate;
//!
//! // A farm that can hold 5 workers, currently with 2
//! let entity = Entity::from_raw(42);
//! let pos = GridPosition { x: 5, y: 5 };
//! let candidate = ScorableCandidate::with_capacity(entity, pos, 5, 2);
//!
//! assert_eq!(candidate.capacity, 5);
//! ```

#![allow(clippy::trivially_copy_pass_by_ref)]
use crate::layer1::chemical::ChemicalState;
use crate::layer1::combat::Drafted;
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::factions::{FactionData, FactionId, FactionMember};
use crate::layer1::health::Health;
use crate::layer1::hobby::Hobby;
use crate::layer1::items::{CarryingItem, Equipment, ItemType};
use crate::layer1::law::justice::Inmate;
use crate::layer1::law::penal::PenalLabor;
use crate::layer1::map::GridPosition;
use crate::layer1::memetics::MemeticCarrier;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Job;
use crate::layer1::resources::{Carrying, ColonyResources, ResourceType};
use crate::layer1::stress::{Breakdown, StressTracker, BREAKDOWN_TICKS_REQUIRED};
use crate::layer1::taboo::TabooState;
use crate::layer1::traits::Traits;
use crate::layer1::social::feral_outpost::CulturalDrift;
use crate::layer1::unrest::MentalState;
use crate::layer1::utility_types::{
    calculate_context_score, ActionType, HobbyType, PopAction, UtilityWeights,
};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use std::collections::HashMap;

/// Evaluates the utility of being idle.
///
/// Idle is a low-priority fallback action. Pops should prefer productive
/// activities (work, eating, resting) over standing around.
#[must_use]
pub const fn evaluate_idle(_needs: &Needs) -> f32 {
    0.05
}

/// Query data for pop evaluation.
///
/// This struct replaces the large tuple query in `evaluate_actions_system`,
/// improving readability and maintainability.
#[derive(QueryData)]
#[query_data(derive(Debug))]
pub struct PopEvaluationQuery {
    pub entity: Entity,
    pub pos: &'static GridPosition,
    pub needs: &'static Needs,
    pub weights: &'static UtilityWeights,
    pub action: &'static PopAction,
    pub equipment: Option<&'static Equipment>,
    pub carrying: Option<&'static Carrying>,
    pub carrying_item: Option<&'static CarryingItem>,
    pub mental_state: Option<&'static MentalState>,
    pub drafted: Option<&'static Drafted>,
    pub inmate: Option<&'static Inmate>,
    pub faction_member: Option<&'static FactionMember>,
    pub penal_labor: Option<&'static PenalLabor>,
    pub breakdown: Option<&'static Breakdown>,
    pub traits: Option<&'static Traits>,
    pub stress: Option<&'static StressTracker>,
    pub hobby: Option<&'static Hobby>,
    pub chemical: Option<&'static ChemicalState>,
    pub memetic_carrier: Option<&'static MemeticCarrier>,
    pub memetic_infection:
        Option<&'static crate::layer1::memetics::parasitic_broadcast::MemeticInfection>,
    pub health: Option<&'static Health>,
    pub job: Option<&'static Job>,
    pub is_silent: Option<&'static crate::layer1::stress::TraitSilent>,
    pub nostalgia: Option<&'static crate::layer1::culture::nostalgia::Nostalgia>,
    pub existential_crisis:
        Option<&'static crate::layer1::economy::existential_audit::ExistentialCrisis>,
    pub temporal_fugue: Option<&'static crate::layer1::mind::temporal_fugue::TemporalFugue>,
    pub cultural_drift: Option<&'static CulturalDrift>,
}

impl PopEvalData {
    /// Converts a query item into `PopEvalData`.
    pub fn from_query_item(item: PopEvaluationQueryItem<'_>) -> Self {
        Self {
            entity: item.entity,
            pos: *item.pos,
            needs: *item.needs,
            weights: *item.weights,
            action: *item.action,
            equipment: item.equipment.copied(),
            carrying: item.carrying.copied(),
            carrying_item: item.carrying_item.map(|c| c.0),
            mental_state: item.mental_state.copied(),
            drafted: item.drafted.copied(),
            faction_member: item.faction_member.copied(),
            penal_labor: item.penal_labor.copied(),
            breakdown: item.breakdown.copied(),
            traits: item.traits.cloned(),
            stress: item
                .stress
                .map_or(0.0, |s| s.accumulated_stress / BREAKDOWN_TICKS_REQUIRED),
            hobby_type: item.hobby.map(|comp| comp.hobby_type),
            chemical_state: item.chemical.cloned(),
            is_memetic_carrier: item.memetic_carrier.is_some(),
            memetic_infection: item.memetic_infection.copied(),
            health: item.health.cloned(),
            job: item.job.copied(),
            insulation: 0.0,
            carrying_item_type: None,
            is_silent: item.is_silent.is_some(),
            is_nostalgic: item.nostalgia.is_some(),
            existential_crisis: item.existential_crisis.copied(),
            has_temporal_fugue: item.temporal_fugue.is_some(),
            is_fringe: item.cultural_drift.is_some_and(|d| d.is_fringe()),
        }
    }
}

/// Evaluates visiting a Sanctuary to reduce stress.
pub fn evaluate_visit_sanctuary(
    pop_pos: GridPosition,
    weights: &UtilityWeights,
    stress: f32,
    candidates: &[ScorableCandidate],
) -> Option<(f32, Entity)> {
    if candidates.is_empty() {
        return None;
    }

    let stress_factor = stress / 100.0;

    // Only consider visiting if stress is high enough
    if stress_factor < 0.4 {
        return None;
    }

    // Since weights don't have a `social` field, we will just use a constant
    // or maybe weights.distance_weight, but it's more of a general survival need.
    // We want a very high base score so it overrides idle/wander/hum when stress is high.
    let base_score = 1.0 * stress_factor * 2.0;

    evaluate_candidates(pop_pos, weights, candidates, base_score)
}

/// Data bundle for pop evaluation, optimized for copy.
#[derive(Clone, Debug)]
pub struct PopEvalData {
    /// The entity ID of the pop.
    pub entity: Entity,
    /// The current grid position of the pop.
    pub pos: GridPosition,
    /// The current needs (hunger, rest, etc.) of the pop.
    pub needs: Needs,
    /// The personality/memory weights for decision making.
    pub weights: UtilityWeights,
    /// The current action state.
    pub action: PopAction,
    /// Equipment held by the pop, if any.
    pub equipment: Option<Equipment>,
    /// Resource currently carried by the pop, if any.
    pub carrying: Option<Carrying>,
    /// Item currently carried by the pop (as a physical entity), if any.
    pub carrying_item: Option<Entity>,
    /// The ItemType of the carried item, if any.
    pub carrying_item_type: Option<ItemType>,
    /// Current mental state (e.g., Broken, Dazed), if any.
    pub mental_state: Option<MentalState>,
    /// Draft status (combat mode), if any.
    pub drafted: Option<Drafted>,
    /// Faction membership details, if any.
    pub faction_member: Option<FactionMember>,
    /// Penal labor status, if any.
    pub penal_labor: Option<PenalLabor>,
    /// Breakdown status, if any.
    pub breakdown: Option<Breakdown>,
    /// Personality traits, if any.
    pub traits: Option<Traits>,
    /// Accumulated stress (normalized 0.0-1.0), derived from `StressTracker`.
    pub stress: f32,
    /// Assigned hobby type, if any.
    pub hobby_type: Option<HobbyType>,
    /// Chemical addiction state, if any.
    pub chemical_state: Option<ChemicalState>,
    /// Whether the pop carries a memetic virus.
    pub is_memetic_carrier: bool,
    pub memetic_infection: Option<crate::layer1::memetics::parasitic_broadcast::MemeticInfection>,
    /// Health of the pop, if any.
    pub health: Option<Health>,
    /// The pop's assigned job.
    pub job: Option<Job>,
    /// Current insulation provided by clothing.
    pub insulation: f32,
    pub is_silent: bool,
    pub is_nostalgic: bool,
    /// Existential crisis state, if any.
    pub existential_crisis: Option<crate::layer1::economy::existential_audit::ExistentialCrisis>,
    pub has_temporal_fugue: bool,
    /// Whether the pop carries the Fringe cultural tag (Spec 290).
    pub is_fringe: bool,
}

#[cfg(test)]
impl PopEvalData {
    /// Helper to create a default `PopEvalData` for testing.
    /// This prevents tests from breaking every time we add a field.
    pub fn test_instance() -> Self {
        Self {
            entity: Entity::from_raw(0),
            pos: GridPosition::default(),
            needs: Needs::default(),
            weights: UtilityWeights::default(),
            action: PopAction::default(),
            equipment: None,
            carrying: None,
            carrying_item: None,
            carrying_item_type: None,
            mental_state: None,
            drafted: None,
            faction_member: None,
            penal_labor: None,
            breakdown: None,
            traits: None,
            stress: 0.0,
            hobby_type: None,
            chemical_state: None,
            is_memetic_carrier: false,
            memetic_infection: None,
            health: None,
            job: None,
            insulation: 0.0,
            is_silent: false,
            is_nostalgic: false,
            existential_crisis: None,
            has_temporal_fugue: false,
            is_fringe: false,
        }
    }
}

/// Context data for utility evaluation (resources, time, etc.)
pub struct WorldContext<'a> {
    /// Reference to global colony resources (food, wood, etc.).
    pub resources: &'a ColonyResources,
    /// Reference to the day/night cycle (for shift checks).
    #[allow(dead_code)]
    pub cycle: &'a DayNightCycle,
    /// Reference to current taboo/law state.
    pub taboo: &'a TabooState,
    /// Reference to faction data (for strike checks).
    pub factions: Option<&'a HashMap<FactionId, FactionData>>,
    /// Reference to zone grid (for sanctuary checks).
    pub zone_grid: &'a crate::layer1::zone::ZoneGrid,
    /// Reference to temperature grid (for clothing checks).
    pub temperature_grid: Option<&'a crate::layer1::temperature::TemperatureGrid>,
    /// Work-utility bonus broadcast by a Fallen Sovereign's labor decree
    /// (0.0 normally). Added to productive action utilities.
    pub work_fervor: f32,
    /// Work-utility bonus broadcast by a Planetary Governor's quota directive
    /// (0.0 normally). Added to productive action utilities.
    pub quota_fervor: f32,
}

// --- UNIFIED PROXY ---
// Razor's Cut: Replaced 5 repetitive proxy structs with one generic candidate struct.

/// The "Universal Target" for AI evaluation.
///
/// Instead of having separate structs for `FarmCandidate`, `ItemCandidate`, etc.,
/// we flatten everything into this `ScorableCandidate`. This allows the
/// [`evaluate_candidates`] function to process *any* list of targets using the same math.
///
/// # Fields
///
/// *   **Capacity/Usage**: Used for crowding penalties (e.g., full taverns are less attractive).
/// *   **Score Bonus**: A generic modifier (e.g., resource quality, clutter amount).
#[derive(Clone, Debug)]
pub struct ScorableCandidate {
    /// The entity ID.
    pub entity: Entity,
    /// The location on the grid.
    pub pos: GridPosition,
    /// Total capacity (default 1).
    pub capacity: usize,
    /// Current usage (default 0).
    pub usage: usize,
    /// Generic score bonus (e.g., refining progress).
    pub score_bonus: f32,
    /// Type of resource (if an item).
    pub resource_type: Option<ResourceType>,
    /// Type of generic item (if an item entity).
    pub item_type: Option<ItemType>,
    pub is_advanced_tech: bool,
}

impl ScorableCandidate {
    /// Creates a simple position-based candidate.
    pub const fn new(entity: Entity, pos: GridPosition) -> Self {
        Self {
            entity,
            pos,
            capacity: 1,
            usage: 0,
            score_bonus: 0.0,
            resource_type: None,
            item_type: None,
            is_advanced_tech: false,
        }
    }

    /// Creates a candidate with capacity.
    pub const fn with_capacity(
        entity: Entity,
        pos: GridPosition,
        capacity: usize,
        usage: usize,
    ) -> Self {
        Self {
            entity,
            pos,
            capacity,
            usage,
            score_bonus: 0.0,
            resource_type: None,
            item_type: None,
            is_advanced_tech: false,
        }
    }
}

/// Generic evaluation function for finding the best candidate.
///
/// This function applies the standard utility formula:
/// `Utility = (Base + Bonus) * ContextScore(Distance, Crowding)`
///
/// # Examples
///
/// ```ignore
/// use scale::layer1::utility_eval_types::{ScorableCandidate, evaluate_candidates};
/// use scale::layer1::map::GridPosition;
/// use scale::layer1::utility_types::UtilityWeights;
/// use bevy_ecs::prelude::Entity;
///
/// let pop_pos = GridPosition { x: 0, y: 0 };
/// let weights = UtilityWeights::default();
///
/// // Candidate A: Close (dist 1) but crowded
/// let mut cand_a = ScorableCandidate::new(Entity::from_raw(1), GridPosition { x: 1, y: 0 });
/// cand_a.capacity = 10;
/// cand_a.usage = 9; // 90% full
///
/// // Candidate B: Far (dist 10) but empty
/// let cand_b = ScorableCandidate::new(Entity::from_raw(2), GridPosition { x: 10, y: 0 });
///
/// let candidates = vec![cand_a, cand_b];
/// let result = evaluate_candidates(pop_pos, &weights, &candidates, 0.5);
///
/// assert!(result.is_some());
/// // Exact winner depends on tuning, but it will pick one.
/// ```
#[must_use]
pub fn evaluate_candidates(
    pop_pos: GridPosition,
    weights: &UtilityWeights,
    candidates: &[ScorableCandidate],
    base_utility: f32,
) -> Option<(f32, Entity)> {
    let mut best: Option<(f32, Entity)> = None;

    for candidate in candidates {
        let context = calculate_context_score(
            pop_pos,
            Some(candidate.pos),
            candidate.capacity,
            candidate.usage,
            weights,
        );

        let utility = (base_utility + candidate.score_bonus) * context;

        if best.is_none_or(|(u, _)| utility > u) {
            best = Some((utility, candidate.entity));
        }
    }
    best
}

/// A massive reusable buffer for AI evaluation.
///
/// # Performance Story
///
/// Allocating vectors for every Pop every frame would be a disaster.
/// Instead, we allocate *once* (and resize as needed) in this resource.
///
/// *   **Zero Allocation Loop**: During `evaluate_actions_system`, we clear these vectors,
///     populate them, read them, and clear them again—without freeing the underlying memory.
/// *   **SoA Layout**: Candidates are grouped by type (Farms, Items), effectively creating
///     a "Structure of Arrays" layout for the evaluation phase.
#[derive(Resource, Default)]
pub struct UtilityAIBuffer {
    /// Buffer for pop data.
    pub pop_data: Vec<PopEvalData>,

    /// Buffer for evaluation results (Actions decided).
    /// Used to avoid re-allocating the results vector every frame.
    pub results: Vec<Option<(ActionType, f32, Option<Entity>)>>,

    // Candidate Buffers
    /// Buffer for farm candidates.
    pub farms: Vec<ScorableCandidate>,
    /// Buffer for housing candidates.
    pub housing: Vec<ScorableCandidate>,
    /// Buffer for tavern candidates.
    pub taverns: Vec<ScorableCandidate>,
    /// Buffer for library candidates.
    pub libraries: Vec<ScorableCandidate>,
    /// Buffer for refining candidates.
    pub refining: Vec<ScorableCandidate>,
    /// Buffer for crops needing pollination.
    pub unpollinated_crops: Vec<ScorableCandidate>,
    /// Buffer for work designation candidates.
    pub work_designations: Vec<ScorableCandidate>,
    /// Buffer for repair designation candidates.
    pub repair_designations: Vec<ScorableCandidate>,
    /// Buffer for tame designation candidates.
    pub tame_designations: Vec<ScorableCandidate>,
    /// Buffer for loose item candidates.
    pub items: Vec<ScorableCandidate>,
    /// Buffer for loose generic item candidates.
    pub item_entities: Vec<ScorableCandidate>,
    /// Buffer for stockpile candidates.
    pub stockpiles: Vec<ScorableCandidate>,
    pub stockpile_positions: bevy_utils::HashSet<GridPosition>,
    /// Buffer for anomaly candidates.
    pub anomalies: Vec<ScorableCandidate>,
    /// Buffer for hospital candidates.
    pub hospitals: Vec<ScorableCandidate>,
    /// Buffer for corpse candidates.
    pub corpses: Vec<ScorableCandidate>,
    /// Buffer for grave candidates.
    pub graves: Vec<ScorableCandidate>,
    /// Buffer for structure candidates needing repair.
    pub repair_structures: Vec<ScorableCandidate>,
    /// Buffer for wanted criminals.
    pub wanted_criminals: Vec<ScorableCandidate>,
    /// Buffer for suspects (predictive policing).
    pub suspects: Vec<ScorableCandidate>,
    /// Buffer for office candidates.
    pub offices: Vec<ScorableCandidate>,
    /// Buffer for walls (Memetic Sigil targets).
    pub walls: Vec<ScorableCandidate>,
    /// Buffer for enemies (Fauna/Flora for drafted pops).
    pub enemies: Vec<ScorableCandidate>,
    /// Buffer for all structures (Mental Break targets).
    pub all_structures: Vec<ScorableCandidate>,
    /// Buffer for shower candidates.
    pub showers: Vec<ScorableCandidate>,
    /// Buffer for hum source candidates.
    pub hum_sources: Vec<ScorableCandidate>,
    /// Buffer for Gene Banks.
    pub gene_banks: Vec<ScorableCandidate>,
    /// Buffer for ghost code residue candidates (Purge job).
    pub residues: Vec<ScorableCandidate>,
    /// Buffer for cleaning targets.
    pub cleaning_targets: Vec<ScorableCandidate>,
    /// Buffer for Sanctuary candidates.
    pub sanctuaries: Vec<ScorableCandidate>,
    /// Buffer for Protest Mobs
    pub mobs: Vec<ScorableCandidate>,
}

/// Helper struct to track the best action found so far.
pub(crate) struct CandidateEvaluator {
    pub(crate) action: ActionType,
    pub(crate) utility: f32,
    pub(crate) target: Option<Entity>,
    pub(crate) is_synth: bool,
}

impl CandidateEvaluator {
    pub(crate) const fn new(initial_utility: f32, is_synth: bool) -> Self {
        Self {
            action: ActionType::Idle,
            utility: initial_utility,
            target: None,
            is_synth,
        }
    }

    pub(crate) fn consider(&mut self, action: ActionType, utility: f32, target: Option<Entity>) {
        if utility > self.utility {
            self.action = action;
            self.utility = utility;
            self.target = target;
        }
    }

    pub(crate) const fn result(self) -> (ActionType, f32, Option<Entity>) {
        (self.action, self.utility, self.target)
    }

    pub(crate) fn evaluate_and_consider(
        &mut self,
        evaluation: Option<(f32, Entity)>,
        action: ActionType,
        context: &WorldContext,
        bonus: f32,
    ) {
        if let Some((mut utility, target)) = evaluation {
            if self.is_synth && action.is_emergency() {
                utility = 0.0; // Apathy: 0 priority for emergencies
            }
            let penalty = crate::layer1::taboo::evaluate_taboo_penalty(action, context.taboo);
            self.consider(action, utility + penalty + bonus, Some(target));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::map::GridPosition;
    use crate::layer1::utility_eval_types::{
        evaluate_candidates, CandidateEvaluator, ScorableCandidate,
    };
    use crate::layer1::utility_types::{ActionType, UtilityWeights};
    use bevy_ecs::prelude::Entity;

    #[test]
    fn test_candidate_evaluator_initialization() {
        let evaluator = CandidateEvaluator::new(0.5, false);
        let (action, utility, target) = evaluator.result();

        assert_eq!(action, ActionType::Idle);
        assert_eq!(utility, 0.5);
        assert_eq!(target, None);
    }

    #[test]
    fn test_candidate_evaluator_consider_higher_utility() {
        let mut evaluator = CandidateEvaluator::new(0.5, false);
        let entity = Entity::from_raw(42);

        evaluator.consider(ActionType::Work, 0.8, Some(entity));
        let (action, utility, target) = evaluator.result();

        assert_eq!(action, ActionType::Work);
        assert_eq!(utility, 0.8);
        assert_eq!(target, Some(entity));
    }

    #[test]
    fn test_candidate_evaluator_ignore_lower_utility() {
        let mut evaluator = CandidateEvaluator::new(0.8, false);
        let entity = Entity::from_raw(42);

        evaluator.consider(ActionType::Work, 0.5, Some(entity));
        let (action, utility, target) = evaluator.result();

        assert_eq!(action, ActionType::Idle);
        assert_eq!(utility, 0.8);
        assert_eq!(target, None);
    }

    #[test]
    fn test_evaluate_candidates_empty_list() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let weights = UtilityWeights::default();
        let candidates: Vec<ScorableCandidate> = vec![];

        let result = evaluate_candidates(pop_pos, &weights, &candidates, 0.5);

        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_candidates_single_candidate() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let weights = UtilityWeights::default();
        let entity = Entity::from_raw(1);
        let candidates = vec![ScorableCandidate::new(entity, GridPosition { x: 1, y: 0 })];

        let result = evaluate_candidates(pop_pos, &weights, &candidates, 0.5);

        assert!(result.is_some());
        let (utility, target) = result.unwrap();
        assert_eq!(target, entity);
        assert!(utility > 0.0);
    }

    #[test]
    fn test_candidate_evaluator_evaluate_and_consider_no_evaluation() {
        let mut evaluator = CandidateEvaluator::new(0.5, false);

        let mut world = bevy_ecs::world::World::new();
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(1, 1));

        let taboo = world.resource::<crate::layer1::taboo::TabooState>();
        let resources = world.resource::<crate::layer1::resources::ColonyResources>();
        let cycle = world.resource::<crate::layer1::day_night::DayNightCycle>();
        let zone_grid = world.resource::<crate::layer1::zone::ZoneGrid>();

        let context = crate::layer1::utility_eval_types::WorldContext {
            resources,
            cycle,
            taboo,
            factions: None,
            zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        evaluator.evaluate_and_consider(None, ActionType::Work, &context, 0.0);

        let (action, utility, _) = evaluator.result();
        assert_eq!(action, ActionType::Idle);
        assert_eq!(utility, 0.5);
    }

    #[test]
    fn test_candidate_evaluator_evaluate_and_consider_with_evaluation() {
        let mut evaluator = CandidateEvaluator::new(0.5, false);

        let mut world = bevy_ecs::world::World::new();
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(1, 1));

        let taboo = world.resource::<crate::layer1::taboo::TabooState>();
        let resources = world.resource::<crate::layer1::resources::ColonyResources>();
        let cycle = world.resource::<crate::layer1::day_night::DayNightCycle>();
        let zone_grid = world.resource::<crate::layer1::zone::ZoneGrid>();

        let context = crate::layer1::utility_eval_types::WorldContext {
            resources,
            cycle,
            taboo,
            factions: None,
            zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        let entity = Entity::from_raw(42);
        evaluator.evaluate_and_consider(Some((0.8, entity)), ActionType::Work, &context, 0.2);

        let (action, utility, target) = evaluator.result();
        assert_eq!(action, ActionType::Work);
        assert_eq!(utility, 1.0); // 0.8 + 0.2 bonus + 0.0 penalty
        assert_eq!(target, Some(entity));
    }

    #[test]
    fn test_evaluate_candidates_table_driven() {
        let pop_pos = GridPosition { x: 0, y: 0 };

        let test_cases = vec![
            (
                UtilityWeights {
                    distance_weight: 10.0,
                    availability_weight: 0.0,
                },
                0,
            ), // Prefers distance
            (
                UtilityWeights {
                    distance_weight: 0.0,
                    availability_weight: 10.0,
                },
                1,
            ), // Prefers availability
        ];

        for (weights, expected_winner) in test_cases {
            let mut cand_a =
                ScorableCandidate::new(Entity::from_raw(1), GridPosition { x: 1, y: 0 }); // Very close
            cand_a.capacity = 10;
            cand_a.usage = 9; // 90% full (low availability)

            let mut cand_b =
                ScorableCandidate::new(Entity::from_raw(2), GridPosition { x: 10, y: 0 }); // Far
            cand_b.capacity = 10;
            cand_b.usage = 0; // Empty (high availability)

            let candidates = vec![cand_a.clone(), cand_b.clone()];
            let result = evaluate_candidates(pop_pos, &weights, &candidates, 0.5);

            assert!(result.is_some());
            let (_, target) = result.unwrap();
            let expected_entity = if expected_winner == 0 {
                cand_a.entity
            } else {
                cand_b.entity
            };

            assert_eq!(
                target, expected_entity,
                "Failed table-driven test case: weights distance={}, availability={}",
                weights.distance_weight, weights.availability_weight
            );
        }
    }
}
