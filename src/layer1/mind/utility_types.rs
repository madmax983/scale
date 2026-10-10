//! The vocabulary and scoring mathematics of the Utility AI.
//!
//! This module defines the [`ActionType`] enum, which acts as the menu of possible
//! goals a Pop can have. It also contains the core math functions that drive decision
//! making, such as the [`need_response_curve`] and [`calculate_context_score`].
//!
//! # Action vs. Execution
//!
//! The Utility AI assigns an [`ActionType`] to a Pop via the [`PopAction`] component.
//! However, this is just a *Goal* (e.g., "Satisfy Hunger"). A separate HTN (Hierarchical
//! Task Network) or execution system handles the actual steps to achieve that goal
//! (e.g., "Find Food" -> "Walk to Food" -> "Eat").
//!
//! # Customization
//!
//! Each Pop's decision-making can be tweaked using the [`UtilityWeights`] component.

use crate::layer1::map::GridPosition;
use bevy_ecs::prelude::*;
use strum_macros::EnumIter;

/// The menu of high-level behaviors a Pop can choose from.
///
/// These are "Goals" rather than atomic steps. For example, [`ActionType::Work`] implies
/// finding a designation, walking to it, and performing the task until complete.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, EnumIter)]
pub enum ActionType {
    PerformAncientRoutine,

    /// Forms a violent mob due to forged memories collapsing.
    RealityCollapse,
    /// Eat food to reduce hunger.
    ///
    /// See \[`crate::layer1::actions::hunger::evaluate_satisfy_hunger`\].
    SatisfyHunger,
    /// Sleep to reduce fatigue.
    ///
    /// See \[`crate::layer1::actions::rest::evaluate_satisfy_rest`\].
    SatisfyRest,
    /// Interact with other pops to fulfill social needs.
    ///
    /// See \[`crate::layer1::actions::social::evaluate_socialize`\].
    Socialize,
    /// Wander to uncover the fog of war or investigate points of interest.
    ///
    /// See \[`crate::layer1::actions::explore::evaluate_explore`\].
    Explore,
    /// Perform designated physical labor (Mine, Build, Chop).
    ///
    /// See \[`crate::layer1::actions::work::evaluate_work`\].
    Work,
    Pollinate,
    /// Protest against current conditions in a mob.
    Protest,
    /// Repair damaged structures to prevent collapse.
    ///
    /// See \[`crate::layer1::actions::repair::evaluate_repair`\].
    Repair,
    /// Research new technologies at a library.
    ///
    /// See \[`crate::layer1::actions::research::evaluate_research`\].
    Research,
    /// Haul loose resources to a stockpile.
    ///
    /// See \[`crate::layer1::actions::haul::evaluate_haul`\].
    Haul,
    /// Seek medical care at a hospital.
    ///
    /// See \[`crate::layer1::actions::medical::evaluate_seek_medical_care`\].
    SeekMedicalCare,
    /// Bury a corpse in a grave.
    ///
    /// See \[`crate::layer1::actions::funeral::evaluate_bury_corpse`\].
    BuryCorpse,
    /// Fetch a tool from a stockpile.
    ///
    /// See \[`crate::layer1::actions::fetch_tool::evaluate_fetch_tool`\].
    FetchTool,
    /// Do nothing.
    ///
    /// See \[`crate::layer1::utility_eval_types::evaluate_idle`\].
    Idle,
    /// Destroy structures due to mental break.
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    Vandalize,
    /// Visit a Sanctuary room to reduce stress.
    ///
    /// See \[`crate::layer1::utility_eval_types::evaluate_visit_sanctuary`\].
    VisitSanctuary,
    /// Consume resources uncontrollably due to mental break.
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    Binge,
    /// Wander aimlessly in a catatonic state due to mental break.
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    Daze,
    /// Engage in combat with hostile entities.
    ///
    /// See \[`crate::layer1::actions::fight::evaluate_drafted_behavior`\].
    Fight,
    /// Refine resources at a building (e.g., Lumber Mill).
    ///
    /// See \[`crate::layer1::actions::refine::evaluate_refine`\].
    Refine,
    /// Work at a farm to produce food.
    ///
    /// See \[`crate::layer1::actions::farm::evaluate_farm`\].
    Farm,
    /// Arrest Wanted criminals and escort them to jail.
    ///
    /// See [`crate::layer1::law::justice::evaluate_warden_action`].
    Warden,
    /// Sleepwalk (Mental Break).
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    Sleepwalking,
    /// Sabotage advanced tech buildings (Nostalgia Cult).
    Sabotage,
    /// Tame a wild animal.
    ///
    Tame,
    /// Starts fires (Mental Break).
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    FireStarting,
    /// Hides in room (Mental Break).
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    HideInRoom,
    /// Wanders sadly (Mental Break).
    ///
    /// See \[`crate::layer1::actions::mental_break::evaluate_mental_break`\].
    SadWander,
    /// Fetch clothing from a stockpile.
    ///
    /// See \[`crate::layer1::actions::fetch_clothing::evaluate_fetch_clothing`\].
    FetchClothing,
    /// Undergoing surgery at a hospital.
    ///
    /// **Note**: Passive action assigned by system, not chosen by AI.
    /// See [`crate::layer1::cybernetics::surgery_system`].
    Surgery,
    /// Recharge battery at a Drone Hub (Drones only).
    Charge,
    /// Engage in a hobby to reduce stress.
    Hobby,
    /// Engage in gossip.
    ///
    /// See \[`crate::layer1::actions::gossip::evaluate_gossip`\].
    Gossip,
    /// Work as an Administrator in an Office.
    ///
    /// See \[`crate::layer1::actions::admin::evaluate_admin`\].
    Admin,
    /// Scrawl memetic sigils on walls (Memetic Hazard).
    ///
    /// See [`crate::layer1::memetics::evaluate_scrawl_memetic_sigil`].
    ScrawlMemeticSigil,
    /// Compulsion to perform a specific useless task due to memetic infection.
    MemeticObsession,
    /// Pre-emptive arrest of high-risk Suspects.
    ///
    /// See [`crate::layer1::law::predictive_policing::evaluate_pre_crime_arrest`].
    PreCrimeArrest,
    /// Consume a chemical substance (Stim/Sedative).
    ///
    /// See [`crate::layer1::chemical::evaluate_consume_chemical`].
    ConsumeChemical,
    /// Collect genetic sample from flora or fauna.
    CollectSample,
    /// Use a shower to clean filth and restore hygiene.
    UseShower,
    /// Listen to The Hum (Spec 238).
    ListenToTheHum,
    /// Clean clutter from the environment.
    Clean,
    /// Purge Ghost Code residue from a tile.
    PurgeResidue,
    /// Staring into the abyss (Void Stare manifestation).
    VoidStare,
    /// Extinguish an adjacent fire.
    ExtinguishFire,
    /// Treat wounds.
    TreatWounds,
    /// Flee from danger.
    Flee,
    /// Stop working and philosophize.
    Philosophize,
    /// Secret night-shift repair work by Fringe pops (Spec 290).
    ///
    /// See [`crate::layer1::phantom_shift::evaluate_phantom_shift`].
    PhantomWork,
}

/// Types of hobbies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HobbyType {
    /// Carving small objects from wood (Creative).
    Whittling,
    /// Watching the clouds go by (Lazy/Nature).
    CloudWatching,
    /// Seeking inner peace (Stoic).
    Meditation,
    /// Sharing rumors and stories (Social).
    Gossip,
    /// Messing with mechanical parts (Industrial).
    Tinkering,
}

impl ActionType {
    /// Total number of action types. Used for array sizing.
    pub const COUNT: usize = 51;

    /// Converts action type to a unique array index (0..COUNT-1).
    #[must_use]
    pub const fn as_index(self) -> usize {
        match self {
            Self::RealityCollapse => 46,
            Self::SatisfyHunger => 0,
            Self::SatisfyRest => 1,
            Self::Socialize => 2,
            Self::Explore => 3,
            Self::Work => 4,
            Self::Protest => 5,
            Self::Repair => 6,
            Self::Research => 7,
            Self::Haul => 8,
            Self::SeekMedicalCare => 9,
            Self::BuryCorpse => 10,
            Self::FetchTool => 11,
            Self::Idle => 12,
            Self::Vandalize => 13,
            Self::Binge => 14,
            Self::Daze => 15,
            Self::Fight => 16,
            Self::Refine => 17,
            Self::Farm => 18,
            Self::Warden => 19,
            Self::Sleepwalking => 20,
            Self::Tame => 21,
            Self::FireStarting => 22,
            Self::HideInRoom => 23,
            Self::SadWander => 24,
            Self::FetchClothing => 25,
            Self::Surgery => 26,
            Self::Charge => 27,
            Self::Hobby => 28,
            Self::Admin => 29,
            Self::ScrawlMemeticSigil => 30,
            Self::MemeticObsession => 48,
            Self::PreCrimeArrest => 31,
            Self::ConsumeChemical => 32,
            Self::CollectSample => 33,
            Self::UseShower => 34,
            Self::ListenToTheHum => 35,
            Self::Clean => 36,
            Self::PurgeResidue => 37,
            Self::VisitSanctuary => 38,
            Self::VoidStare => 39,
            Self::ExtinguishFire => 40,
            Self::TreatWounds => 41,
            Self::Flee => 42,
            Self::Sabotage => 43,
            Self::Gossip => 44,
            Self::Philosophize => 45,
            Self::PerformAncientRoutine => 47,
            Self::Pollinate => 48,
            Self::PhantomWork => 49,
        }
    }

    /// Returns `true` if this action is an emergency that a Synth would ignore.
    #[must_use]
    pub const fn is_emergency(&self) -> bool {
        matches!(self, Self::ExtinguishFire | Self::TreatWounds | Self::Flee)
    }

    /// Returns the danger level of the action (probability of accident per tick).
    ///
    /// This value is used by the execution systems (e.g., `work_execution_system`)
    /// to determine if a Pop should suffer an injury while performing this task.
    ///
    /// *   **Work/Repair**: 0.1% chance per tick. (Low risk)
    /// *   **Tame**: 0.5% chance per tick. (Moderate risk - animals bite!)
    /// *   **Fight**: 0% (Combat handles damage via its own system).
    /// *   **Others**: 0% chance.
    #[must_use]
    pub const fn danger_level(&self) -> f64 {
        match self {
            Self::Work | Self::Repair => 0.001, // 0.1% chance per tick
            Self::Tame => 0.005,                // 0.5% chance per tick (animals bite!)
            _ => 0.0,
        }
    }

    /// Returns the damage inflicted if an accident occurs.
    ///
    /// If `danger_level` triggers an accident, this value determines the amount of HP
    /// lost by the Pop.
    ///
    /// *   **Work/Repair**: 10.0 HP (minor injury).
    /// *   **Tame**: 15.0 HP (animal bite/kick).
    #[must_use]
    pub const fn accident_damage(&self) -> f32 {
        match self {
            Self::Work | Self::Repair => 10.0,
            Self::Tame => 15.0,
            _ => 0.0,
        }
    }
}

/// The current state of a Pop's brain.
#[derive(Component, Debug, Clone, Copy)]
pub struct PopAction {
    /// The current action being performed.
    pub current: ActionType,
    /// The utility score of the current action (snapshot at decision time).
    pub current_utility: f32,
    /// How many ticks the pop has been doing this action (commitment timer).
    pub ticks_committed: u32,
}

impl Default for PopAction {
    fn default() -> Self {
        Self {
            current: ActionType::Idle,
            current_utility: 0.0,
            ticks_committed: 0,
        }
    }
}

/// The "Personality" or "Memory" of a Pop.
#[derive(Component, Clone, Copy, Debug)]
pub struct UtilityWeights {
    /// How much distance penalties affect scoring.
    /// *   Higher (> 1.0): Hates walking.
    /// *   Lower (< 1.0): Willing to travel.
    pub distance_weight: f32,

    /// How much crowding affects scoring.
    /// *   Higher: Hates crowds (introvert).
    /// *   Lower: Doesn't mind sharing space.
    pub availability_weight: f32,
}

impl Default for UtilityWeights {
    fn default() -> Self {
        Self {
            distance_weight: 1.0,
            availability_weight: 1.0,
        }
    }
}

/// Global tuning configuration for the Utility AI system.
#[derive(Resource, Clone, Copy)]
pub struct UtilityConfig {
    /// Hysteresis factor to prevent "dithering" (rapidly switching tasks).
    /// A new action must be `current_utility + switch_threshold` better to swap.
    pub switch_threshold: f32,

    /// How often (in ticks) to re-evaluate actions.
    /// Higher = better performance but slower reaction time.
    pub evaluation_interval: u32,
}

impl Default for UtilityConfig {
    fn default() -> Self {
        Self {
            switch_threshold: 0.15,
            evaluation_interval: 1,
        }
    }
}

/// Marker component to trigger HTN (Hierarchical Task Network) planning.
///
/// When the Utility AI selects a new high-level [`ActionType`], it inserts this
/// component to tell the planner to generate the specific steps (Walk -> `PickUp` -> Eat).
#[derive(Component)]
pub struct StartPlan {
    /// The action to plan for.
    pub action: ActionType,
    /// The target entity (if any).
    pub target: Option<Entity>,
}

/// Calculates urgency from a need value (0.0-1.0).
///
/// **Formula**: `1.0 - (need_value^2)`
///
/// This creates a quadratic "urgency curve":
/// *   **High Need** (0.9): Urgency is low (~0.19). You're fine.
/// *   **Low Need** (0.1): Urgency is very high (~0.99). You're starving.
/// *   **Middle** (0.5): Urgency is moderate (0.75).
///
/// This curve prevents Pops from reacting too early to minor hunger, but makes them
/// panic as they get closer to 0.
///
/// # Examples
///
/// ```
/// use scale::layer1::utility_types::need_response_curve;
///
/// let urgency = need_response_curve(0.9); // Full belly
/// assert!((urgency - 0.19).abs() < 0.0001);
///
/// let panic = need_response_curve(0.1); // Starving
/// assert!(panic > 0.9);
/// ```
#[must_use]
pub fn need_response_curve(need_value: f32) -> f32 {
    need_value.mul_add(-need_value, 1.0)
}

/// Calculates Manhattan distance between two positions.
///
/// `|x1 - x2| + |y1 - y2|`
///
/// Safe against overflow (clamps to `i32::MAX`).
///
/// # Examples
///
/// ```
/// use scale::layer1::utility_types::manhattan_distance;
/// use scale::layer1::map::GridPosition;
///
/// let start = GridPosition { x: 0, y: 0 };
/// let end = GridPosition { x: 3, y: 4 };
///
/// assert_eq!(manhattan_distance(&start, &end), 7);
/// ```
#[must_use]
pub const fn manhattan_distance(pos1: &GridPosition, pos2: &GridPosition) -> i32 {
    let dx = pos1.x.abs_diff(pos2.x);
    let dy = pos1.y.abs_diff(pos2.y);
    let sum = dx.saturating_add(dy);
    if sum > i32::MAX as u32 {
        i32::MAX
    } else {
        sum as i32
    }
}

/// Calculates a context score (0.0 - 1.0) based on distance and crowding.
///
/// This score represents "how convenient" a target is.
///
/// # Formula
///
/// 1.  **Distance**: Hyperbolic decay: `1.0 / (1.0 + 0.1 * distance)`.
///     *   At distance 0, score is 1.0.
///     *   At distance 10, score is 0.5 (half utility).
///     *   At distance 90, score is 0.1.
///     *   Raised to the power of `weights.distance_weight`.
///
/// 2.  **Availability**: Linear fraction: `1.0 - (occupied / capacity)`.
///     *   Raised to the power of `weights.availability_weight`.
///
/// # Examples
///
/// ```
/// use scale::layer1::utility_types::{calculate_context_score, UtilityWeights};
/// use scale::layer1::map::GridPosition;
///
/// let pop_pos = GridPosition { x: 0, y: 0 };
/// let farm_pos = GridPosition { x: 10, y: 0 };
/// let weights = UtilityWeights::default();
///
/// // Scenario 1: Empty farm, moderate distance
/// let score_empty = calculate_context_score(
///     pop_pos,
///     Some(farm_pos),
///     10, // Capacity
///     0,  // Occupied
///     &weights
/// );
/// // Distance factor: 1.0 / (1.0 + 0.1*10) = 0.5
/// assert!((score_empty - 0.5).abs() < f32::EPSILON);
///
/// // Scenario 2: Crowded farm (50% full)
/// let score_crowded = calculate_context_score(
///     pop_pos,
///     Some(farm_pos),
///     10,
///     5,  // 5/10 occupied
///     &weights
/// );
/// // Distance (0.5) * Availability (0.5) = 0.25
/// assert!((score_crowded - 0.25).abs() < f32::EPSILON);
/// ```
#[must_use]
pub fn calculate_context_score(
    pop_pos: GridPosition,
    target_pos: Option<GridPosition>,
    building_capacity: usize,
    building_occupied: usize,
    weights: &UtilityWeights,
) -> f32 {
    let mut score = 1.0;

    // Distance factor (closer = better)
    if let Some(target) = target_pos {
        let distance = manhattan_distance(&pop_pos, &target);
        if distance > 0 {
            #[allow(clippy::cast_precision_loss)]
            let distance_factor = 1.0 / (distance as f32).mul_add(0.1, 1.0);

            // OPTIMIZATION: Avoid powf if weight is 1.0 (very common)
            if (weights.distance_weight - 1.0).abs() < f32::EPSILON {
                score *= distance_factor;
            } else {
                score *= distance_factor.powf(weights.distance_weight);
            }
        }
    }

    // Availability factor (less crowded = better)
    if building_capacity > 0 {
        #[allow(clippy::cast_precision_loss)]
        let availability = (1.0 - (building_occupied as f32 / building_capacity as f32)).max(0.0);

        // OPTIMIZATION: Avoid powf if weight is 1.0 (very common)
        if (weights.availability_weight - 1.0).abs() < f32::EPSILON {
            score *= availability;
        } else {
            score *= availability.powf(weights.availability_weight);
        }
    }

    // OPTIMIZATION: Removed redundant clamp. Inputs are strictly within [0.0, 1.0].
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use strum::IntoEnumIterator;

    #[test]
    fn test_action_type_completeness() {
        for action in ActionType::iter() {
            assert!(
                action.as_index() < ActionType::COUNT,
                "Action index out of bounds: {:?}",
                action
            );
        }
    }

    #[test]
    fn test_action_type_count_matches() {
        assert_eq!(
            ActionType::iter().count(),
            ActionType::COUNT,
            "ActionType::COUNT mismatch"
        );
    }

    #[test]
    fn test_need_response_curve_boundaries() {
        // Curve: 1.0 - x^2
        // If x=1.5, 1.0 - 2.25 = -1.25.
        // If x=-0.5, 1.0 - 0.25 = 0.75.
        let val_over = need_response_curve(1.5);
        assert!(val_over < 0.0);

        let val_neg = need_response_curve(-0.5);
        assert!(val_neg < 1.0 && val_neg > 0.0);
    }

    #[test]
    fn test_calculate_context_score_zero_capacity() {
        let weights = UtilityWeights::default();
        let pop_pos = GridPosition { x: 0, y: 0 };
        // Capacity 0 should not panic and return valid score (considering availability logic skipped)
        let score = calculate_context_score(pop_pos, None, 0, 0, &weights);
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_manhattan_distance_extreme() {
        // Test overflow protection
        let min = GridPosition {
            x: i32::MIN,
            y: i32::MIN,
        };
        let max = GridPosition {
            x: i32::MAX,
            y: i32::MAX,
        };

        // Distance should be huge but clamped to i32::MAX
        let dist = manhattan_distance(&min, &max);
        assert_eq!(dist, i32::MAX);
    }

    #[test]
    fn test_manhattan_distance_overflow() {
        let pos1 = GridPosition { x: i32::MIN, y: 0 };
        let pos2 = GridPosition { x: 1, y: 0 };
        let d = manhattan_distance(&pos1, &pos2);
        assert_eq!(d, i32::MAX);
    }

    #[test]
    fn test_action_type_variants() {
        let hunger = ActionType::SatisfyHunger;
        let rest = ActionType::SatisfyRest;
        let idle = ActionType::Idle;

        assert_ne!(hunger, rest);
        assert_ne!(hunger, idle);
        assert_eq!(idle, ActionType::Idle);
    }

    #[test]
    fn test_pop_action_default() {
        let action = PopAction::default();
        assert_eq!(action.current, ActionType::Idle);
        assert!((action.current_utility - 0.0).abs() < f32::EPSILON);
        assert_eq!(action.ticks_committed, 0);
    }

    #[test]
    fn test_utility_weights_default() {
        let weights = UtilityWeights::default();
        assert!((weights.distance_weight - 1.0).abs() < f32::EPSILON);
        assert!((weights.availability_weight - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_need_response_curve() {
        let urgency_high = need_response_curve(0.9);
        assert!(urgency_high < 0.2);

        let urgency_med = need_response_curve(0.5);
        assert!(urgency_med > 0.6 && urgency_med < 0.8);

        let urgency_low = need_response_curve(0.1);
        assert!(urgency_low > 0.95);

        assert!(urgency_low > urgency_med);
        assert!(urgency_med > urgency_high);
    }

    #[test]
    fn test_calculate_context_score_distance() {
        let weights = UtilityWeights::default();

        let close_score = calculate_context_score(
            GridPosition { x: 0, y: 0 },
            Some(GridPosition { x: 1, y: 0 }),
            4,
            0,
            &weights,
        );

        let far_score = calculate_context_score(
            GridPosition { x: 0, y: 0 },
            Some(GridPosition { x: 10, y: 0 }),
            4,
            0,
            &weights,
        );

        assert!(close_score > far_score);
    }

    #[test]
    fn test_calculate_context_score_availability() {
        let weights = UtilityWeights::default();
        let pos = GridPosition { x: 0, y: 0 };

        let empty_score =
            calculate_context_score(pos, Some(GridPosition { x: 5, y: 5 }), 4, 0, &weights);

        let full_score =
            calculate_context_score(pos, Some(GridPosition { x: 5, y: 5 }), 4, 3, &weights);

        assert!(empty_score > full_score);
    }

    #[test]
    fn test_manhattan_distance() {
        let pos1 = GridPosition { x: 0, y: 0 };
        let pos2 = GridPosition { x: 3, y: 4 };
        assert_eq!(manhattan_distance(&pos1, &pos2), 7);

        let pos3 = GridPosition { x: -2, y: 5 };
        assert_eq!(manhattan_distance(&pos1, &pos3), 7);
    }

    #[test]
    fn test_utility_config_default() {
        let config = UtilityConfig::default();
        assert!((config.switch_threshold - 0.15).abs() < f32::EPSILON);
        assert_eq!(config.evaluation_interval, 1);
    }

    #[test]
    fn test_calculate_context_score_over_capacity_nan() {
        let weights = UtilityWeights {
            availability_weight: 0.5,
            ..Default::default()
        };

        let pop_pos = GridPosition { x: 0, y: 0 };
        // Occupied (20) > Capacity (10) => availability = 1.0 - 2.0 = -1.0
        // -1.0.powf(0.5) => NaN
        let score = calculate_context_score(pop_pos, None, 10, 20, &weights);

        // This assertion ensures we don't propagate NaNs
        assert!(
            !score.is_nan(),
            "Score should not be NaN even if over capacity"
        );
        assert_eq!(score, 0.0, "Score should be clamped to 0.0");
    }

    #[test]
    fn test_calculate_context_score_full_capacity() {
        let weights = UtilityWeights::default();
        let pop_pos = GridPosition { x: 0, y: 0 };
        // Occupied == Capacity
        let score = calculate_context_score(pop_pos, None, 10, 10, &weights);
        assert_eq!(score, 0.0, "Score should be 0.0 when full");
    }

    #[test]
    fn test_calculate_context_score_extreme_weights() {
        let mut weights = UtilityWeights {
            availability_weight: 100.0,
            ..Default::default()
        };
        let pop_pos = GridPosition { x: 0, y: 0 };

        // 50% full
        let score = calculate_context_score(pop_pos, None, 10, 5, &weights);
        // Availability 0.5. 0.5^100 should be tiny.
        assert!(
            score < 0.0001,
            "Score should be tiny with high availability weight"
        );

        weights.availability_weight = 0.0; // Doesn't care about crowds
        let score_ignore = calculate_context_score(pop_pos, None, 10, 5, &weights);
        // Availability 0.5. 0.5^0 = 1.0.
        assert!(
            (score_ignore - 1.0).abs() < f32::EPSILON,
            "Score should be 1.0 when weight is 0"
        );
    }

    #[test]
    fn test_assignment_type_variants() {
        let farm = AssignmentType::FarmWorker;
        let tavern = AssignmentType::TavernVisitor;
        let housing = AssignmentType::HousingResident;

        assert_ne!(farm, tavern);
        assert_ne!(farm, housing);
        assert_eq!(farm, AssignmentType::FarmWorker);
    }
}

/// Types of assignments a pop can have.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum AssignmentType {
    /// Enforces the law.
    Sheriff,
    /// Working at a farm.
    FarmWorker,
    /// Residing in housing.
    HousingResident,
    /// Socializing at a tavern.
    TavernVisitor,
    /// Working at a library.
    LibraryWorker,
    /// Recovering in a hospital.
    Patient,
    /// Burying a corpse.
    Funeral,
    /// Working at an observatory.
    ObservatoryWorker,
    /// Administrator job.
    Administrator,
    /// Emergency rationing bureaucrat.
    RationingBureaucrat,
    /// Undergoing surgery.
    Surgery,
    /// Working in a deep mine.
    DeepMining,
}
