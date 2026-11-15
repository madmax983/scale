//! # Chemical Regulation
//!
//! > "Better Living Through Chemistry."
//!
//! This module implements the colony's chemical regulation system, allowing Pops to consume
//! substances that alter their physical and mental state. It's a double-edged sword:
//! temporary performance boosts come at the cost of long-term health and stability.
//!
//! ## Core Concepts
//!
//! *   **Stimulants (`Stim`):** Designed for exhaustion. They significantly boost movement speed (1.5x)
//!     but damage health over time. Pops crave them when tired or overworked.
//! *   **Sedatives (`Sedative`):** Designed for stress. They significantly reduce stress (-20)
//!     but slow movement speed (0.5x). Pops crave them when on the verge of a breakdown.
//! *   **Addiction:** Repeated consumption leads to addiction. Withdrawal causes severe penalties
//!     (-50% Speed) and overrides utility AI to prioritize seeking the chemical at all costs.
//!
//! ## Mechanics
//!
//! ### Consumption Effects
//! Consumption is instantaneous and applies immediate effects:
//! *   **Stim:** +50% Speed, -2 Health (Immediate). duration: 500 ticks.
//! *   **Sedative:** -50% Speed, -20 Stress (Immediate), +Mood via a
//!     `Sedative Calm` morale modifier (Spec 417). duration: 500 ticks.
//!
//! ### Addiction & Withdrawal
//! *   **Addiction Severity:** Increases by 0.1 per dose (max 1.0).
//! *   **Withdrawal Threshold:** 1000 ticks since last dose.
//! *   **Withdrawal Effects:**
//!     *   **Speed:** Scaled crash (Spec 417 §5): the multiplier is
//!       `1.0 - severity`, deepening by 0.05 per 1000 ticks overdue
//!       (capped at 0.25), floored at 0.1. Severity 0.5 at onset
//!       reproduces the original flat -50%.
//!     *   **Utility AI:** Forces `desire = 1.0` (Panic search).
//!
//! ## Usage
//!
//! Usually, this system is driven by `evaluate_consume_chemical` within the Utility AI loop.
//! However, you can manually force consumption via `consume_chemical`.
//!
//! ```rust
//! # use bevy_ecs::prelude::*;
//! # use scale::layer1::chemical::{consume_chemical, ChemicalType};
//! # // Setup world (assume setup code omitted)
//! # let mut world = World::new();
//! # world.insert_resource(scale::shared::time::SimulationTime::default());
//! # let pop = world.spawn_empty().id();
//! // Administer a stimulant to a pop
//! consume_chemical(&mut world, pop, ChemicalType::Stim);
//! ```

use crate::layer1::health::Health;
use crate::layer1::items::ItemType;
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::utility_eval_types::ScorableCandidate;
use crate::layer1::utility_types::{calculate_context_score, UtilityWeights};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Types of chemicals pops can consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChemicalType {
    /// Increases speed but damages health. Used to combat fatigue.
    Stim,
    /// Reduces stress but slows speed. Used to combat mental breakdowns.
    Sedative,
}

/// An active chemical effect currently modifying a Pop.
#[derive(Debug, Clone, Copy)]
pub struct ActiveEffect {
    /// The chemical causing the effect.
    pub chemical: ChemicalType,
    /// Remaining duration in ticks. Default is 500.
    pub duration: u32,
    /// Magnitude of the effect (multiplier).
    /// e.g., 1.5 for Stim (Speed boost), 0.5 for Sedative (Slowdown).
    pub magnitude: f32,
}

/// Tracks addiction to a specific chemical.
///
/// Addiction increases with use and decays very slowly (not implemented yet).
/// Withdrawal triggers if the chemical is not consumed within the threshold.
#[derive(Debug, Clone, Copy)]
pub struct Addiction {
    /// The chemical the pop is addicted to.
    pub chemical: ChemicalType,
    /// Severity of addiction (0.0 to 1.0).
    /// Currently used to track progress towards dependency.
    pub severity: f32,
    /// The simulation tick when the chemical was last consumed.
    pub last_consumed_tick: u64,
    /// Ticks until withdrawal symptoms start (Default: 1000).
    pub withdrawal_threshold: u64,
    /// Whether the pop is currently suffering from withdrawal.
    pub in_withdrawal: bool,
}

/// Player-facing label for the sedative mood modifier (Spec 417).
pub const SEDATIVE_MOOD_LABEL: &str = "Sedative Calm";

/// Morale value of the sedative mood modifier (Spec 417: Sedatives = +Mood).
pub const SEDATIVE_MOOD_VALUE: f32 = 0.15;

/// Ticks the sedative mood modifier lasts (matches the 500-tick effect).
pub const SEDATIVE_MOOD_DURATION: u32 = 500;

/// Extra withdrawal speed loss per 1000 ticks spent overdue (Spec 417 §5).
pub const WITHDRAWAL_DEEPEN_PER_1000_TICKS: f32 = 0.05;

/// Cap on the time-based withdrawal deepening (Spec 417 §5).
pub const WITHDRAWAL_DEEPEN_CAP: f32 = 0.25;

/// Speed multiplier contributed by one addiction's withdrawal (Spec 417 §5:
/// the crash scales with severity and with time spent overdue, replacing
/// Spec 181's flat x0.5). Returns 1.0 when not in withdrawal. Severity 0.5
/// with no overdue time reproduces the original 0.5 exactly. `current_tick`
/// is `None` when the caller has no clock (unit contexts) — the severity
/// scaling still applies.
#[must_use]
pub fn withdrawal_speed_factor(addiction: &Addiction, current_tick: Option<u64>) -> f32 {
    if !addiction.in_withdrawal {
        return 1.0;
    }
    let severity = addiction.severity.clamp(0.0, 1.0);
    let mut factor = 1.0 - severity;
    if let Some(tick) = current_tick {
        let onset = addiction
            .last_consumed_tick
            .saturating_add(addiction.withdrawal_threshold);
        let overdue = tick.saturating_sub(onset) as f32;
        let deepen =
            (overdue / 1000.0 * WITHDRAWAL_DEEPEN_PER_1000_TICKS).min(WITHDRAWAL_DEEPEN_CAP);
        factor -= deepen;
    }
    factor.clamp(0.1, 1.0)
}

/// Colony-wide chemical census for STATS / the headless `chem` panel
/// (Spec 417).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChemicalCensus {
    /// Total active chemical effects across all pops.
    pub active_effects: usize,
    /// Pops carrying at least one addiction.
    pub addicted: usize,
    /// Pops currently in withdrawal from at least one chemical.
    pub in_withdrawal: usize,
}

/// Counts active effects, addicted pops, and pops in withdrawal.
pub fn chemical_census(world: &mut World) -> ChemicalCensus {
    let mut census = ChemicalCensus::default();
    let mut query = world.query::<&ChemicalState>();
    for state in query.iter(world) {
        census.active_effects += state.active_effects.len();
        if !state.addictions.is_empty() {
            census.addicted += 1;
        }
        if state.addictions.iter().any(|a| a.in_withdrawal) {
            census.in_withdrawal += 1;
        }
    }
    census
}

/// Component storing chemical effects and addictions on a Pop.
#[derive(Component, Default, Debug, Clone)]
pub struct ChemicalState {
    /// List of currently active effects (temporary buffs/debuffs).
    pub active_effects: Vec<ActiveEffect>,
    /// List of long-term addictions.
    pub addictions: Vec<Addiction>,
}

impl ChemicalState {
    /// Gets a reference to an addiction if it exists.
    #[must_use]
    pub fn get_addiction(&self, chem: ChemicalType) -> Option<&Addiction> {
        self.addictions.iter().find(|a| a.chemical == chem)
    }

    /// Gets a mutable reference to an addiction if it exists.
    #[must_use]
    pub fn get_addiction_mut(&mut self, chem: ChemicalType) -> Option<&mut Addiction> {
        self.addictions.iter_mut().find(|a| a.chemical == chem)
    }

    /// Checks if the pop is in withdrawal for a specific chemical.
    ///
    /// Returns `true` only if the pop has an addiction and `in_withdrawal` is set.
    #[must_use]
    pub fn is_in_withdrawal(&self, chem: ChemicalType) -> bool {
        if let Some(addiction) = self.get_addiction(chem) {
            return addiction.in_withdrawal;
        }
        false
    }
}

/// Consumes a chemical, applying immediate effects and updating addiction state.
///
/// This is a convenience wrapper that handles component fetching and state management.
///
/// # Side Effects
/// *   **Stim:** Increases speed by 50% for 500 ticks. Deals 2.0 damage to Health immediately.
/// *   **Sedative:** Decreases speed by 50% for 500 ticks. Reduces Stress by 20.0 immediately.
/// *   **Addiction:** Increases addiction severity by 0.1. Resets withdrawal timer.
///
/// # Examples
///
/// ```rust
/// # use bevy_ecs::prelude::*;
/// # use scale::layer1::chemical::{consume_chemical, ChemicalType};
/// # let mut world = World::new();
/// # world.insert_resource(scale::shared::time::SimulationTime::default());
/// # let pop = world.spawn_empty().id();
/// consume_chemical(&mut world, pop, ChemicalType::Stim);
/// ```
pub fn consume_chemical(world: &mut World, entity: Entity, chem: ChemicalType) {
    let tick = world.resource::<SimulationTime>().tick;

    let mut state = world
        .get::<ChemicalState>(entity)
        .cloned()
        .unwrap_or_default();

    let mut query = world.query::<(
        Option<&mut Health>,
        Option<&mut crate::layer1::stress::StressTracker>,
        Option<&mut Morale>,
    )>();

    if let Ok((mut health_opt, mut stress_opt, mut morale_opt)) = query.get_mut(world, entity) {
        consume_chemical_logic(
            &mut state,
            chem,
            tick,
            health_opt.as_deref_mut(),
            stress_opt.as_deref_mut(),
            morale_opt.as_deref_mut(),
        );
    } else {
        // Entity might be missing components, but we still update chemical state.
        consume_chemical_logic(&mut state, chem, tick, None, None, None);
    }

    world.entity_mut(entity).insert(state);
}

/// Core logic for consuming a chemical.
///
/// Separated for testability without `World`.
pub fn consume_chemical_logic(
    state: &mut ChemicalState,
    chem: ChemicalType,
    tick: u64,
    health: Option<&mut Health>,
    stress: Option<&mut crate::layer1::stress::StressTracker>,
    morale: Option<&mut Morale>,
) {
    // 1. Add/Refresh Active Effect
    if let Some(existing) = state.active_effects.iter_mut().find(|e| e.chemical == chem) {
        // Refresh duration
        existing.duration = 500;
    } else {
        let magnitude = match chem {
            ChemicalType::Stim => 1.5,     // +50% speed
            ChemicalType::Sedative => 0.5, // -50% speed (slowdown)
        };
        state.active_effects.push(ActiveEffect {
            chemical: chem,
            duration: 500,
            magnitude,
        });
    }

    // 2. Apply Immediate Effects
    match chem {
        ChemicalType::Stim => {
            // Immediate Health Damage (The cost of hustle)
            if let Some(h) = health {
                h.current = (h.current - 2.0).max(0.0);
            }
        }
        ChemicalType::Sedative => {
            // Immediate Stress Relief
            if let Some(s) = stress {
                s.accumulated_stress = (s.accumulated_stress - 20.0).max(0.0);
            }
            // Spec 417: Sedatives = +Mood. Bridge onto the real Morale seam:
            // refresh the same-label modifier instead of stacking doses.
            if let Some(m) = morale {
                if let Some(existing) = m
                    .modifiers
                    .iter_mut()
                    .find(|md| md.label == SEDATIVE_MOOD_LABEL)
                {
                    existing.value = SEDATIVE_MOOD_VALUE;
                    existing.duration = SEDATIVE_MOOD_DURATION;
                } else {
                    m.add_modifier(MoodModifier {
                        label: SEDATIVE_MOOD_LABEL.to_string(),
                        value: SEDATIVE_MOOD_VALUE,
                        duration: SEDATIVE_MOOD_DURATION,
                    });
                }
            }
        }
    }

    // 3. Update Addiction
    if let Some(addiction) = state.addictions.iter_mut().find(|a| a.chemical == chem) {
        addiction.severity = (addiction.severity + 0.1).min(1.0);
        addiction.last_consumed_tick = tick;
        addiction.in_withdrawal = false;
        // Reset withdrawal threshold
        addiction.withdrawal_threshold = 1000;
    } else {
        state.addictions.push(Addiction {
            chemical: chem,
            severity: 0.1,
            last_consumed_tick: tick,
            withdrawal_threshold: 1000,
            in_withdrawal: false,
        });
    }
}

/// System to process addiction withdrawal and active effect duration.
///
/// This runs every tick to decrement durations and check withdrawal thresholds.
pub fn addiction_system(world: &mut World) {
    let tick = world.resource::<SimulationTime>().tick;

    let mut query = world.query::<(Entity, &mut ChemicalState)>();

    for (_entity, mut state) in query.iter_mut(world) {
        // 1. Process Active Effects
        state.active_effects.retain_mut(|effect| {
            if effect.duration > 0 {
                effect.duration -= 1;
                true
            } else {
                false
            }
        });

        // 2. Process Addictions (Withdrawal)
        for addiction in &mut state.addictions {
            let time_since = tick.saturating_sub(addiction.last_consumed_tick);
            if time_since > addiction.withdrawal_threshold {
                if !addiction.in_withdrawal {
                    addiction.in_withdrawal = true;
                }
            } else if addiction.in_withdrawal {
                // If somehow withdrawal was true but time is less (e.g. tick wrap?), reset.
                // Mostly safety check.
                addiction.in_withdrawal = false;
            }
        }
    }
}

/// Calculates the speed modifier based on active chemical effects and withdrawal status.
///
/// # Returns
/// A float multiplier for speed.
/// *   `> 1.0`: Speed boost (e.g., Stim).
/// *   `< 1.0`: Slowdown (e.g., Sedative, Withdrawal).
/// *   Clamped between `0.1` and `5.0`.
pub fn get_speed_modifier(world: &World, entity: Entity) -> f32 {
    let mut modifier = 1.0;
    if let Some(state) = world.get::<ChemicalState>(entity) {
        // Active Effects
        for effect in &state.active_effects {
            if matches!(effect.chemical, ChemicalType::Stim | ChemicalType::Sedative) {
                modifier *= effect.magnitude;
            }
        }

        // Withdrawal Penalties (Spec 417 §5: scaled by severity + time)
        let tick = world.get_resource::<SimulationTime>().map(|t| t.tick);
        for addiction in &state.addictions {
            modifier *= withdrawal_speed_factor(addiction, tick);
        }
    }
    // Clamp modifier to sane limits to prevent physics explosion or stasis
    modifier.clamp(0.1, 5.0)
}

/// Applies chemical speed modifiers to the pop's speed component.
///
/// This system ensures that the `Speed` component reflects the current chemical state.
pub fn apply_chemical_speed_modifiers_system(world: &mut World) {
    let mut query = world.query::<(
        &crate::layer1::chemical::ChemicalState,
        &mut crate::layer1::pop::Speed,
    )>();
    let tick = world.get_resource::<SimulationTime>().map(|t| t.tick);
    for (state, mut speed) in query.iter_mut(world) {
        let mut modifier = 1.0;
        // Active Effects
        for effect in &state.active_effects {
            if matches!(effect.chemical, ChemicalType::Stim | ChemicalType::Sedative) {
                modifier *= effect.magnitude;
            }
        }
        // Withdrawal Penalties (Spec 417 §5: scaled by severity + time)
        for addiction in &state.addictions {
            modifier *= withdrawal_speed_factor(addiction, tick);
        }
        let modifier = modifier.clamp(0.1, 5.0);
        if (modifier - 1.0).abs() > f32::EPSILON {
            speed.current *= modifier;
        }
    }
}

/// Evaluates the utility of consuming chemicals.
///
/// # Logic
/// *   **Stim:** Desired when `Needs::rest` is low (< 0.2) or `Withdrawal` is active.
/// *   **Sedative:** Desired when `Stress` is high (> 0.8) or `Withdrawal` is active.
/// *   **Withdrawal:** Increases base desire to 1.0 (Panic).
#[must_use]
#[allow(clippy::useless_let_if_seq)]
pub fn evaluate_consume_chemical(
    pop_pos: GridPosition,
    needs: &Needs,
    weights: &UtilityWeights,
    chemical_state: Option<&ChemicalState>,
    stress: f32,                         // Normalized 0-1
    item_entities: &[ScorableCandidate], // Available items
) -> Option<(f32, Entity)> {
    // 1. Determine Desire
    let mut desire_stim = 0.05_f32; // Base desire (curiosity/habit)
    let mut desire_sedative = 0.05_f32;

    // Withdrawal (Panic!)
    if let Some(state) = chemical_state {
        if state.is_in_withdrawal(ChemicalType::Stim) {
            desire_stim = 1.0;
        }
        if state.is_in_withdrawal(ChemicalType::Sedative) {
            desire_sedative = 1.0;
        }
    }

    // Needs based desire
    if (desire_stim - 1.0).abs() > f32::EPSILON && needs.rest < 0.2 {
        // Very tired -> Stim
        desire_stim += 0.5;
    }

    if (desire_sedative - 1.0).abs() > f32::EPSILON && stress > 0.8 {
        // High stress -> Sedative
        desire_sedative += 0.5;
    }

    // ⚡ Bolt Optimization:
    // Previously, this function collected candidates into two separate `Vec`s
    // (`candidates_stim` and `candidates_sedative`) using `.clone()` on each item.
    // By evaluating candidates inline during a single pass, we eliminate 2 heap
    // allocations and O(N) clones per pop, per evaluation tick, significantly
    // reducing memory pressure on the hot path of the Utility AI.
    let mut best_score = 0.0;
    let mut best_target = None;

    for candidate in item_entities {
        if let Some(item_type) = &candidate.item_type {
            let base_desire = match item_type {
                ItemType::Stim if desire_stim > 0.1 => desire_stim,
                ItemType::Sedative if desire_sedative > 0.1 => desire_sedative,
                _ => continue,
            };

            let context = calculate_context_score(
                pop_pos,
                Some(candidate.pos),
                candidate.capacity,
                candidate.usage,
                weights,
            );

            let utility = (base_desire + candidate.score_bonus) * context;

            if utility > best_score {
                best_score = utility;
                best_target = Some(candidate.entity);
            }
        }
    }

    best_target.map(|target| (best_score, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::chemical::{ActiveEffect, Addiction, ChemicalState, ChemicalType};
    use crate::layer1::health::Health;
    use crate::layer1::pop::Pop;
    use crate::layer1::stress::StressTracker;
    use crate::shared::time::SimulationTime;
    #[test]
    fn test_chemical_state_default() {
        let state = ChemicalState::default();
        assert!(state.active_effects.is_empty());
        assert!(state.addictions.is_empty());
    }

    #[test]
    fn test_consume_stim_adds_effect() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((Pop, ChemicalState::default(), Health::default()))
            .id();

        // Simulate consuming a Stim
        crate::layer1::chemical::consume_chemical(&mut world, pop, ChemicalType::Stim);

        let state = world.get::<ChemicalState>(pop).unwrap();
        assert!(state
            .active_effects
            .iter()
            .any(|e| e.chemical == ChemicalType::Stim));
    }

    #[test]
    fn test_stim_effect_modifies_speed() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![ActiveEffect {
                        chemical: ChemicalType::Stim,
                        duration: 100,
                        magnitude: 1.5, // +50% speed
                    }],
                    addictions: vec![],
                },
            ))
            .id();

        let speed = crate::layer1::chemical::get_speed_modifier(&world, pop);
        assert!((speed - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_addiction_buildup() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((Pop, ChemicalState::default(), Health::default()))
            .id();

        // Consume multiple times
        for _ in 0..5 {
            crate::layer1::chemical::consume_chemical(&mut world, pop, ChemicalType::Stim);
        }

        let state = world.get::<ChemicalState>(pop).unwrap();
        let addiction = state.get_addiction(ChemicalType::Stim);
        assert!(addiction.is_some());
        assert!(addiction.unwrap().severity > 0.0);
    }

    #[test]
    fn test_withdrawal_triggers() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 2000,
            ..Default::default()
        });

        let pop = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![Addiction {
                        chemical: ChemicalType::Stim,
                        severity: 0.8,
                        last_consumed_tick: 500, // 2000 - 500 = 1500 > 1000
                        withdrawal_threshold: 1000,
                        in_withdrawal: false,
                    }],
                },
            ))
            .id();

        // Run system
        crate::layer1::chemical::addiction_system(&mut world);

        let state = world.get::<ChemicalState>(pop).unwrap();
        assert!(state.is_in_withdrawal(ChemicalType::Stim));
    }

    #[test]
    fn test_evaluate_consume_chemical_withdrawal() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let needs = Needs::default();
        let weights = UtilityWeights::default();
        let addictions = vec![Addiction {
            chemical: ChemicalType::Stim,
            severity: 0.8,
            last_consumed_tick: 0,
            withdrawal_threshold: 100,
            in_withdrawal: true,
        }];

        let state = ChemicalState {
            active_effects: vec![],
            addictions,
        };

        let mut item = ScorableCandidate::new(Entity::from_raw(1), GridPosition { x: 0, y: 0 });
        item.item_type = Some(ItemType::Stim);

        let result =
            evaluate_consume_chemical(pop_pos, &needs, &weights, Some(&state), 0.0, &[item]);

        assert!(result.is_some());
        let (score, _) = result.unwrap();
        assert!(score >= 1.0, "Withdrawal should produce high score");
    }

    #[test]
    fn test_evaluate_consume_chemical_needs() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let needs = Needs {
            rest: 0.1, // Very tired
            ..Default::default()
        };
        let weights = UtilityWeights::default();

        let mut item = ScorableCandidate::new(Entity::from_raw(1), GridPosition { x: 0, y: 0 });
        item.item_type = Some(ItemType::Stim);

        let result = evaluate_consume_chemical(pop_pos, &needs, &weights, None, 0.0, &[item]);

        assert!(result.is_some());
        let (score, _) = result.unwrap();
        assert!(score > 0.5, "Tired pop should want Stim");
    }

    #[test]
    fn test_evaluate_consume_chemical_stress() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let needs = Needs::default();
        let weights = UtilityWeights::default();

        let mut item = ScorableCandidate::new(Entity::from_raw(1), GridPosition { x: 0, y: 0 });
        item.item_type = Some(ItemType::Sedative);

        let result = evaluate_consume_chemical(
            pop_pos,
            &needs,
            &weights,
            None,
            0.9, // High stress
            &[item],
        );

        assert!(result.is_some());
        let (score, _) = result.unwrap();
        assert!(score > 0.5, "Stressed pop should want Sedative");
    }

    #[test]
    fn test_stim_health_damage() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((
                Pop,
                ChemicalState::default(),
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        crate::layer1::chemical::consume_chemical(&mut world, pop, ChemicalType::Stim);

        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0);
        assert!((health.current - 98.0).abs() < f32::EPSILON); // 100 - 2
    }

    #[test]
    fn test_sedative_stress_reduction() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((
                Pop,
                ChemicalState::default(),
                StressTracker {
                    accumulated_stress: 50.0,
                },
            ))
            .id();

        crate::layer1::chemical::consume_chemical(&mut world, pop, ChemicalType::Sedative);

        let stress = world.get::<StressTracker>(pop).unwrap();
        assert!(stress.accumulated_stress < 50.0);
        assert!((stress.accumulated_stress - 30.0).abs() < f32::EPSILON); // 50 - 20
    }

    #[test]
    fn test_active_effect_expiration() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![ActiveEffect {
                        chemical: ChemicalType::Stim,
                        duration: 1, // Will expire after 1 tick
                        magnitude: 1.0,
                    }],
                    addictions: vec![],
                },
            ))
            .id();

        crate::layer1::chemical::addiction_system(&mut world);

        let state = world.get::<ChemicalState>(pop).unwrap();
        // Duration 1 -> 0, still kept (retained if duration > 0 BEFORE decrement? No.
        // Logic: if duration > 0 { duration -= 1; true } else { false }
        // So duration 1 -> duration 0 -> kept.
        assert_eq!(state.active_effects.len(), 1);
        assert_eq!(state.active_effects[0].duration, 0);

        crate::layer1::chemical::addiction_system(&mut world);

        let state = world.get::<ChemicalState>(pop).unwrap();
        // Duration 0 -> else branch -> removed.
        assert!(state.active_effects.is_empty());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn test_apply_chemical_speed_modifiers_system() {
        use bevy::prelude::*;

        let mut app = App::new();
        app.add_systems(
            Update,
            crate::layer1::chemical::apply_chemical_speed_modifiers_system,
        );

        // 1. No modifiers -> stays same
        let pop_normal = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                ChemicalState::default(),
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        // 2. Withdrawal -> halves speed
        let pop_withdrawal = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![crate::layer1::chemical::Addiction {
                        chemical: ChemicalType::Stim,
                        severity: 0.5,
                        in_withdrawal: true,
                        last_consumed_tick: 0,
                        withdrawal_threshold: 100,
                    }],
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        // 3. Stims -> multiples speed
        let pop_stims = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                ChemicalState {
                    active_effects: vec![crate::layer1::chemical::ActiveEffect {
                        chemical: ChemicalType::Stim,
                        duration: 10,
                        magnitude: 2.0,
                    }],
                    addictions: vec![],
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        // 4. Over clamp limits
        let pop_extreme_slow = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                ChemicalState {
                    active_effects: vec![crate::layer1::chemical::ActiveEffect {
                        chemical: ChemicalType::Sedative,
                        duration: 10,
                        magnitude: 0.05,
                    }],
                    addictions: vec![],
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        let pop_extreme_fast = app
            .world_mut()
            .spawn((
                crate::layer1::pop::Pop,
                ChemicalState {
                    active_effects: vec![crate::layer1::chemical::ActiveEffect {
                        chemical: ChemicalType::Stim,
                        duration: 10,
                        magnitude: 10.0,
                    }],
                    addictions: vec![],
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        app.update();

        assert_eq!(
            app.world()
                .get::<crate::layer1::pop::Speed>(pop_normal)
                .unwrap()
                .current,
            1.0
        );
        assert_eq!(
            app.world()
                .get::<crate::layer1::pop::Speed>(pop_withdrawal)
                .unwrap()
                .current,
            0.5
        );
        assert_eq!(
            app.world()
                .get::<crate::layer1::pop::Speed>(pop_stims)
                .unwrap()
                .current,
            2.0
        );
        assert_eq!(
            app.world()
                .get::<crate::layer1::pop::Speed>(pop_extreme_slow)
                .unwrap()
                .current,
            0.1
        );
        assert_eq!(
            app.world()
                .get::<crate::layer1::pop::Speed>(pop_extreme_fast)
                .unwrap()
                .current,
            5.0
        );
    }
}

/// Spec 417 "Chemical Regulation" — RED tests adapted from the spec's mock
/// sketch (`Pop`/`Stats`/`drugs` types do not exist) onto the real Spec 181
/// seams: [`ChemicalState`], [`consume_chemical`], [`get_speed_modifier`],
/// `Health`, `StressTracker`, and the `Morale`/`MoodModifier` pipeline.
/// Spec 417's legs that Spec 181 left unbuilt: withdrawal scaling with
/// severity/time (§5) and the Sedative +Mood leg (181 cut stress only).
#[cfg(test)]
mod spec417_tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::pop::Pop;
    use crate::layer1::social::morale::Morale;
    use crate::layer1::stress::StressTracker;
    use crate::shared::time::SimulationTime;

    fn stim_addiction(severity: f32, in_withdrawal: bool) -> Addiction {
        Addiction {
            chemical: ChemicalType::Stim,
            severity,
            last_consumed_tick: 0,
            withdrawal_threshold: 1000,
            in_withdrawal,
        }
    }

    #[test]
    fn test_417_consume_stim_applies_buffs() {
        // Spec RED test 1, adapted: Stats { speed_modifier, health_drain_rate }
        // is a mock — the real expression is get_speed_modifier() > 1.0 and
        // immediate Health damage (-2.0, Spec 181).
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((Pop, ChemicalState::default(), Health::default()))
            .id();
        consume_chemical(&mut world, pop, ChemicalType::Stim);
        let speed = get_speed_modifier(&world, pop);
        assert!(speed > 1.0, "Stim should raise speed modifier, got {speed}");
        let health = world.get::<Health>(pop).unwrap();
        assert!(
            health.current < health.max,
            "Stim should cost health, got {}/{}",
            health.current,
            health.max
        );
    }

    #[test]
    fn test_417_addiction_development() {
        // Spec RED test 2, adapted: severity is 0.0-1.0 (+0.1/dose), not 0-100.
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((Pop, ChemicalState::default(), Health::default()))
            .id();
        for _ in 0..5 {
            consume_chemical(&mut world, pop, ChemicalType::Stim);
        }
        let state = world.get::<ChemicalState>(pop).unwrap();
        let addiction = state.get_addiction(ChemicalType::Stim).unwrap();
        assert!(addiction.severity > 0.0);
        assert!((addiction.severity - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_417_withdrawal_symptoms_scale_with_severity() {
        // Spec RED test 3 + §5, adapted: mock severity 80.0 == real 0.8.
        // The crash must scale: severe addiction crashes far below the old
        // flat x0.5; mild addiction barely slows the pop.
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let severe = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![stim_addiction(0.8, true)],
                },
            ))
            .id();
        let mild = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![stim_addiction(0.2, true)],
                },
            ))
            .id();
        let severe_speed = get_speed_modifier(&world, severe);
        let mild_speed = get_speed_modifier(&world, mild);
        assert!(severe_speed < 1.0, "withdrawal must debuff speed");
        assert!(
            severe_speed < 0.5,
            "severity 0.8 crash should beat the flat 0.5, got {severe_speed}"
        );
        assert!(
            mild_speed > severe_speed,
            "mild addiction ({mild_speed}) should crash softer than severe ({severe_speed})"
        );
    }

    #[test]
    fn test_417_withdrawal_scales_with_time_overdue() {
        // §5: the longer past the withdrawal threshold, the deeper the crash
        // (capped). Severity 0.5 at onset == the old flat 0.5 baseline.
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        let pop = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![stim_addiction(0.5, true)],
                },
            ))
            .id();
        let at_onset = get_speed_modifier(&world, pop);
        assert!(
            (at_onset - 0.5).abs() < f32::EPSILON,
            "onset should match the Spec 181 baseline 0.5, got {at_onset}"
        );
        world.insert_resource(SimulationTime {
            tick: 6000,
            ..Default::default()
        });
        let later = get_speed_modifier(&world, pop);
        assert!(
            later < at_onset - 0.2,
            "5000 ticks overdue should deepen the crash: {at_onset} -> {later}"
        );
    }

    #[test]
    fn test_417_sedative_applies_mood_buff_and_slowdown() {
        // Spec 417 headline: Sedatives = +Mood, -Speed. Spec 181 delivered
        // -Speed and -20 Stress but never touched the real Morale seam.
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        let pop = world
            .spawn((
                Pop,
                ChemicalState::default(),
                Morale::default(),
                StressTracker {
                    accumulated_stress: 50.0,
                },
            ))
            .id();
        consume_chemical(&mut world, pop, ChemicalType::Sedative);
        let speed = get_speed_modifier(&world, pop);
        assert!(speed < 1.0, "Sedative should slow speed, got {speed}");
        let stress = world.get::<StressTracker>(pop).unwrap();
        assert!((stress.accumulated_stress - 30.0).abs() < f32::EPSILON);
        let morale = world.get::<Morale>(pop).unwrap();
        let calm: Vec<_> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == SEDATIVE_MOOD_LABEL)
            .collect();
        assert_eq!(calm.len(), 1, "sedative should add one mood modifier");
        assert!(
            calm[0].value > 0.0,
            "sedative mood modifier must be positive"
        );
        // Second dose refreshes the modifier instead of stacking it.
        consume_chemical(&mut world, pop, ChemicalType::Sedative);
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(
            morale
                .modifiers
                .iter()
                .filter(|m| m.label == SEDATIVE_MOOD_LABEL)
                .count(),
            1,
            "repeat doses refresh, never stack, the mood modifier"
        );
    }

    #[test]
    fn test_417_dose_resets_withdrawal() {
        // Spec §7: WithdrawalTimer resets on a fresh dose of the same drug.
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 5000,
            ..Default::default()
        });
        let pop = world
            .spawn((
                Pop,
                ChemicalState {
                    active_effects: vec![],
                    addictions: vec![stim_addiction(0.8, true)],
                },
                Health::default(),
            ))
            .id();
        assert!(world
            .get::<ChemicalState>(pop)
            .unwrap()
            .is_in_withdrawal(ChemicalType::Stim));
        consume_chemical(&mut world, pop, ChemicalType::Stim);
        let state = world.get::<ChemicalState>(pop).unwrap();
        assert!(!state.is_in_withdrawal(ChemicalType::Stim));
        assert_eq!(
            state
                .get_addiction(ChemicalType::Stim)
                .unwrap()
                .last_consumed_tick,
            5000
        );
    }

    #[test]
    fn test_417_withdrawal_factor_ignores_sober_addiction() {
        // No withdrawal flag -> no penalty, whatever the severity.
        let addiction = stim_addiction(0.9, false);
        assert!((withdrawal_speed_factor(&addiction, Some(999_999)) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_417_ip_guard_public_strings_original() {
        let public_strings = [
            SEDATIVE_MOOD_LABEL,
            "Stim",
            "Sedative",
            "Chemical Regulation",
        ];
        let banned = [
            "morphine",
            "heroin",
            "cocaine",
            "methamphetamine",
            "adderall",
            "ritalin",
            "xanax",
            "valium",
            "oxycodone",
            "fentanyl",
        ];
        for s in public_strings {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "IP-guard: banned term '{term}' in public string '{s}'"
                );
            }
        }
    }
}
