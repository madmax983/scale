//! # Utility AI: The Brain of the Colony
//!
//! This module implements a **Utility-based AI** system (sometimes called "Need-based AI")
//! that drives the behavior of every Pop in the colony.
//!
//! # Architecture
//!
//! The system operates in a "Gather-Think-Act" cycle designed for parallelism:
//!
//! 1.  **Gather (`ScopedEvaluationContext`):**
//!     We extract or clone necessary data (Pops, Needs, Buildings, Items) from the ECS World.
//!     This avoids borrowing conflicts during the parallel phase.
//!
//! 2.  **Think (`PopDecider`):**
//!     Each Pop is evaluated independently in a `ComputeTaskPool`.
//!     The `PopDecider` walks through a hierarchy of needs (Survival > Social > Work)
//!     and scores every possible candidate (e.g., every apple, every bed).
//!
//! 3.  **Act (`apply_evaluation_results`):**
//!     The best action (highest utility) is written back to the `PopAction` component.
//!     If the new utility is significantly higher than the current action's utility
//!     (plus a hysteresis threshold), the Pop switches tasks.
//!
//! # Examples
//!
//! Conceptually, the loop looks like this:
//!
//! ```no_run
//! // 1. System runs in the schedule
//! // evaluate_actions_system(&mut world);
//!
//! // Inside the system:
//! // a. Snapshot world state into ScopedEvaluationContext.
//! // b. Populate UtilityAIBuffer with all candidates (Food, Beds, Jobs).
//! // c. Run parallel evaluation:
//! //    for pop in pops {
//! //        let decision = PopDecider::new(pop, context).run();
//! //    }
//! // d. Apply results:
//! //    pop.action = decision.best_action;
//! ```

use crate::layer1::actions::{
    evaluate_clean, evaluate_drafted_behavior, evaluate_fetch_clothing, evaluate_fetch_tool,
    evaluate_gossip, evaluate_haul, evaluate_listen_to_hum, evaluate_mental_break,
    evaluate_research, evaluate_shower, evaluate_simple_action,
};
use crate::layer1::chemical::evaluate_consume_chemical;
use crate::layer1::factions::Factions;
use crate::layer1::hobby::evaluate_hobby;
use crate::layer1::law::justice::evaluate_warden_action;
use crate::layer1::law::predictive_policing::evaluate_pre_crime_arrest;
use crate::layer1::resources::ColonyResources;
use crate::layer1::temperature::TemperatureGrid;
use crate::layer1::traits::Trait;
use crate::layer1::utility_ai_population::{collect_pop_data, populate_ai_buffer};
use crate::layer1::utility_eval_types::evaluate_candidates;
use crate::layer1::utility_eval_types::{
    evaluate_idle, CandidateEvaluator, PopEvalData, UtilityAIBuffer, WorldContext,
};
pub use crate::layer1::utility_types::{
    calculate_context_score, manhattan_distance, need_response_curve, ActionType, PopAction,
    StartPlan, UtilityConfig, UtilityWeights,
};
use crate::layer1::zone::ZoneGrid;
use bevy_ecs::prelude::*;
use bevy_tasks::ComputeTaskPool;

/// A thread-safe snapshot of the world state for parallel evaluation.
///
/// This struct works around Bevy's `World` mutability rules. Since we need to
/// evaluate thousands of Pops in parallel (read-only access to most resources)
/// but eventually mutate the `PopAction` components, we:
///
/// 1.  **Extract**: Move resources out of the World or clone them.
/// 2.  **Evaluate**: Run read-only logic in a `ComputeTaskPool`.
/// 3.  **Apply**: Write the results back to the World.
///
/// # Architecture Context
/// By extracting things like `UtilityAIBuffer` entirely using `world.remove_resource()`,
/// we trick the borrow checker. We can then mutate this buffer in parallel while the
/// rest of the `World` remains unborrowed. We *must* guarantee the resources are re-inserted
/// in the `restore()` method.
struct ScopedEvaluationContext {
    /// Buffer containing both the Pop data (input) and Evaluation results (output).
    buffer: UtilityAIBuffer,
    zone_grid: Option<ZoneGrid>,
    temperature_grid: Option<TemperatureGrid>,
    factions: Option<Factions>,

    // Cloned resources for context
    resources: ColonyResources,
    cycle: crate::layer1::day_night::DayNightCycle,
    taboo: Option<crate::layer1::taboo::TabooState>,

    // Config needed for collection/application
    config: UtilityConfig,

    /// Work-utility bonus from a Fallen Sovereign's labor decree (0.0 normally).
    work_fervor: f32,
    /// Work-utility bonus from a Planetary Governor's quota directive (0.0 normally).
    quota_fervor: f32,
}

impl ScopedEvaluationContext {
    fn new(world: &mut World) -> Self {
        let config = *world.resource::<UtilityConfig>();

        // 1. Buffer (must be removed before collection if we want to mutate it while world is mutable)
        let mut buffer = world
            .remove_resource::<UtilityAIBuffer>()
            .unwrap_or_default();

        collect_pop_data(world, &mut buffer, &config);

        // 2. Remove other resources
        let zone_grid = world.remove_resource::<ZoneGrid>();
        let temperature_grid = world.remove_resource::<TemperatureGrid>();
        let factions = world.remove_resource::<Factions>();

        // 3. Clone others
        let resources = *world.resource::<ColonyResources>();
        // Fallen Sovereign: labor-decree work fervor (0.0 when no decree).
        let work_fervor = world
            .get_resource::<crate::layer1::culture::sovereign::LaborFervor>()
            .map_or(0.0, |l| l.bonus);
        // Planetary Governor: quota-directive work fervor (0.0 when no quota).
        let quota_fervor = world
            .get_resource::<crate::layer1::culture::governor::QuotaOrder>()
            .map_or(0.0, |q| q.bonus);
        let cycle = *world.resource::<crate::layer1::day_night::DayNightCycle>();
        // ⚡ Bolt Optimization:
        // We remove `TabooState` rather than cloning it to avoid a clone of a `HashMap`
        // per tick. This avoids a heap allocation on the hot path.
        let taboo = world.remove_resource::<crate::layer1::taboo::TabooState>();

        Self {
            buffer,
            zone_grid,
            temperature_grid,
            factions,
            resources,
            cycle,
            taboo,
            config,
            work_fervor,
            quota_fervor,
        }
    }

    fn restore(self, world: &mut World) {
        world.insert_resource(self.buffer);
        if let Some(zg) = self.zone_grid {
            world.insert_resource(zg);
        }
        if let Some(tg) = self.temperature_grid {
            world.insert_resource(tg);
        }
        if let Some(f) = self.factions {
            world.insert_resource(f);
        }
        if let Some(t) = self.taboo {
            world.insert_resource(t);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_context<'a>(
        resources: &'a ColonyResources,
        cycle: &'a crate::layer1::day_night::DayNightCycle,
        taboo: Option<&'a crate::layer1::taboo::TabooState>,
        factions: Option<&'a Factions>,
        zone_grid: Option<&'a ZoneGrid>,
        temperature_grid: Option<&'a TemperatureGrid>,
        fallback_zone: &'a ZoneGrid,
        fallback_taboo: &'a crate::layer1::taboo::TabooState,
        work_fervor: f32,
        quota_fervor: f32,
    ) -> WorldContext<'a> {
        WorldContext {
            resources,
            cycle,
            taboo: taboo.unwrap_or(fallback_taboo),
            factions: factions.as_ref().map(|f| &f.map),
            zone_grid: zone_grid.unwrap_or(fallback_zone),
            temperature_grid,
            work_fervor,
            quota_fervor,
        }
    }

    fn populate(&mut self, world: &mut World) {
        let zone_grid_fallback = ZoneGrid::new(1, 1);
        let default_taboo = crate::layer1::taboo::TabooState::default();
        let context = Self::build_context(
            &self.resources,
            &self.cycle,
            self.taboo.as_ref(),
            self.factions.as_ref(),
            self.zone_grid.as_ref(),
            self.temperature_grid.as_ref(),
            &zone_grid_fallback,
            &default_taboo,
            self.work_fervor,
            self.quota_fervor,
        );

        populate_ai_buffer(world, &mut self.buffer, &context);
    }

    fn run(&mut self) {
        let zone_grid_fallback = ZoneGrid::new(1, 1);
        let default_taboo = crate::layer1::taboo::TabooState::default();
        let context = Self::build_context(
            &self.resources,
            &self.cycle,
            self.taboo.as_ref(),
            self.factions.as_ref(),
            self.zone_grid.as_ref(),
            self.temperature_grid.as_ref(),
            &zone_grid_fallback,
            &default_taboo,
            self.work_fervor,
            self.quota_fervor,
        );

        let mut results = std::mem::take(&mut self.buffer.results);
        results.clear();
        results.resize(self.buffer.pop_data.len(), None);

        run_evaluations(&self.buffer, &context, &mut results);

        self.buffer.results = results;
    }

    fn apply(self, world: &mut World) {
        apply_evaluation_results(
            world,
            &self.buffer.pop_data,
            &self.buffer.results,
            &self.config,
        );
        self.restore(world);
    }
}

/// System to update commitment timers.
///
/// Increments `ticks_committed` for every pop. This is used to prevent
/// "jitter" (rapidly switching tasks) by enforcing a minimum commitment time.
///
/// # Architecture Context
/// By tracking commitment, the AI ensures a Pop won't bounce back and forth
/// between two equidistant actions (e.g. food on the left, bed on the right)
/// because of a minuscule 0.001 utility change.
///
/// # Examples
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::mind::utility_types::{ActionType, PopAction};
/// use scale::layer1::mind::utility_ai::update_action_timer_system;
///
/// let mut world = World::new();
/// let entity = world.spawn(PopAction {
///     current: ActionType::Idle,
///     current_utility: 0.0,
///     ticks_committed: 5,
/// }).id();
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(update_action_timer_system);
/// schedule.run(&mut world);
///
/// assert_eq!(world.get::<PopAction>(entity).expect("PopAction should exist").ticks_committed, 6);
/// ```
pub fn update_action_timer_system(mut query: Query<&mut PopAction>) {
    query.par_iter_mut().for_each(|mut action| {
        action.ticks_committed = action.ticks_committed.saturating_add(1);
    });
}

/// The "Conscience" of a Pop.
///
/// This struct holds the transient state for a single Pop's decision-making process.
/// It iterates through the **Hierarchy of Needs** (Survival -> Social -> Work)
/// and scores potential actions using the [`CandidateEvaluator`].
///
/// # Architecture Context
/// This encapsulates the actual `Utility AI` logic. It starts at the bottom of Maslow's
/// hierarchy (food, sleep) and works its way up (social, exploration). The moment an
/// action scores above the switch threshold, it becomes the new candidate. The evaluator
/// handles hysteresis internally so Pops don't rapidly flip between options with similar scores.
struct PopDecider<'a> {
    /// The evaluator that tracks the best action found so far.
    evaluator: CandidateEvaluator,
    /// The read-only data for this specific Pop.
    data: &'a PopEvalData,
    buffer: &'a UtilityAIBuffer,
    context: &'a WorldContext<'a>,
    is_striking: bool,
    is_penal: bool,
    is_noble: bool,
}

impl<'a> PopDecider<'a> {
    fn new(
        data: &'a PopEvalData,
        buffer: &'a UtilityAIBuffer,
        context: &'a WorldContext<'a>,
    ) -> Self {
        let is_striking = Self::check_striking(data, context);
        let is_penal = data.penal_labor.is_some();
        let is_noble = data.traits.as_ref().is_some_and(|t| t.has(Trait::Noble));
        let is_synth = data.traits.as_ref().is_some_and(|t| t.has(Trait::Synth));

        Self {
            evaluator: CandidateEvaluator::new(evaluate_idle(&data.needs), is_synth),
            data,
            buffer,
            context,
            is_striking,
            is_penal,
            is_noble,
        }
    }

    fn check_striking(data: &PopEvalData, context: &WorldContext) -> bool {
        data.faction_member
            .as_ref()
            .and_then(|member| member.faction_id)
            .and_then(|fid| context.factions.and_then(|factions| factions.get(&fid)))
            .is_some_and(|faction_data| {
                faction_data.state == crate::layer1::factions::FactionState::Striking
            })
    }

    /// **Priority 1: Survival**
    ///
    /// Evaluates immediate physiological threats. These usually generate high utility
    /// (0.8 - 1.0+) when needs are critical.
    ///
    /// *   **Hunger**: Seeks food if hungry.
    /// *   **Rest**: Seeks a bed if tired.
    /// *   **Health**: Seeks a hospital if injured.
    /// *   **Addiction**: Seeks chemicals if withdrawing.
    #[allow(clippy::collapsible_if)]
    fn evaluate_group_survival(&mut self) {
        if self.data.has_temporal_fugue {
            // Temporal Fugue ignores survival needs completely
            return;
        }

        let pop_pos = self.data.pos;
        let needs = self.data.needs;
        let weights = self.data.weights;

        // Evaluate VisitSanctuary
        self.evaluator.evaluate_and_consider(
            crate::layer1::utility_eval_types::evaluate_visit_sanctuary(
                pop_pos,
                &weights,
                self.data.stress,
                &self.buffer.sanctuaries,
            ),
            ActionType::VisitSanctuary,
            self.context,
            0.0,
        );

        // Evaluate Hunger
        let urgency = need_response_curve(needs.hunger);
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.farms, urgency),
            ActionType::SatisfyHunger,
            self.context,
            0.0,
        );

        // Evaluate SatisfyRest
        let mut rest_urgency = (1.0 - needs.rest) + 0.5;

        // Spec 761: VoidTouched pops refuse to sleep on surface
        if self
            .data
            .traits
            .as_ref()
            .is_some_and(|t| t.has(Trait::VoidTouched))
        {
            // Assume we are on surface since we are in Layer 1 evaluation
            // A true layer check would require a `Location` component, but we'll apply a
            // heavy penalty directly to their sleep urgency.
            rest_urgency -= 2.0;
        }

        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.housing, rest_urgency),
            ActionType::SatisfyRest,
            self.context,
            0.0,
        );

        // Evaluate SeekMedicalCare
        if let Some(health) = &self.data.health {
            if health.current < health.max {
                let urgency = (1.0 - (health.current / health.max)) * 2.0;
                self.evaluator.evaluate_and_consider(
                    evaluate_simple_action(pop_pos, &weights, &self.buffer.hospitals, urgency),
                    ActionType::SeekMedicalCare,
                    self.context,
                    0.0,
                );
            }
        }

        // Evaluate ConsumeChemical
        self.evaluator.evaluate_and_consider(
            evaluate_consume_chemical(
                pop_pos,
                &needs,
                &weights,
                self.data.chemical_state.as_ref(),
                self.data.stress,
                &self.buffer.item_entities,
            ),
            ActionType::ConsumeChemical,
            self.context,
            0.0,
        );

        // Evaluate UseShower
        self.evaluator.evaluate_and_consider(
            evaluate_shower(
                pop_pos,
                &needs,
                &weights,
                self.context.resources,
                &self.buffer.showers,
            ),
            ActionType::UseShower,
            self.context,
            0.0,
        );
    }

    /// **Priority 2: Social & Mental Health**
    ///
    /// Evaluates psychological needs. Ignored by penal laborers.
    ///
    /// *   **Socialize**: Visits a tavern if lonely.
    fn evaluate_group_social(&mut self) {
        if self.is_penal || self.data.is_silent {
            return;
        }

        let pop_pos = self.data.pos;
        let needs = self.data.needs;
        let weights = self.data.weights;

        // Evaluate Socialize
        let urgency = (1.0 - needs.leisure) * 1.5;
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.taverns, urgency),
            ActionType::Socialize,
            self.context,
            0.0,
        );

        // Evaluate Gossip
        let (_, score, target) = evaluate_gossip(self.data, self.buffer);
        if let Some(target_entity) = target {
            self.evaluator.evaluate_and_consider(
                Some((score, target_entity)),
                ActionType::Gossip,
                self.context,
                0.0,
            );
        }
    }

    fn evaluate_work_and_taming(&mut self) {
        let pop_pos = self.data.pos;
        let weights = self.data.weights;
        let work_bonus = if self.is_penal { 1.0 } else { 0.0 };

        // ⚡ Bolt Optimization: Removed unnecessary clone of `work_designations`

        let work_utility = evaluate_simple_action(
            pop_pos,
            &weights,
            &self.buffer.work_designations,
            if self.data.is_nostalgic { 0.2 } else { 0.5 },
        );
        self.evaluator.evaluate_and_consider(
            work_utility,
            ActionType::Work,
            self.context,
            work_bonus + self.context.work_fervor + self.context.quota_fervor,
        );

        self.evaluator.evaluate_and_consider(
            evaluate_candidates(pop_pos, &weights, &self.buffer.tame_designations, 0.6),
            ActionType::Tame,
            self.context,
            0.0,
        );
    }

    fn evaluate_production(&mut self) {
        let pop_pos = self.data.pos;
        let weights = self.data.weights;
        let is_feral = self
            .data
            .traits
            .as_ref()
            .is_some_and(|t| t.has(Trait::Feral));

        let refining_utility = if self.data.is_nostalgic {
            None
        } else {
            evaluate_simple_action(pop_pos, &weights, &self.buffer.refining, 0.5)
        };
        self.evaluator.evaluate_and_consider(
            refining_utility,
            ActionType::Refine,
            self.context,
            self.context.work_fervor + self.context.quota_fervor,
        );

        // Homeostatic food urgency: when the colony's food stockpile runs low,
        // farming outbids competing chores (e.g. Repair at 0.6) so the colony
        // staffs its farms instead of starving next to them. 1x at food >= 10,
        // 2x at 0. Kept moderate so farmers can still be pulled away by urgent
        // needs (hunger/rest) instead of getting stuck via utility staleness.
        let food_scarcity =
            1.0 + 1.0 * (1.0 - (self.context.resources.food / 10.0).clamp(0.0, 1.0));
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.farms, 0.5 * food_scarcity),
            ActionType::Farm,
            self.context,
            self.context.work_fervor + self.context.quota_fervor,
        );

        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.offices, 0.5),
            ActionType::Admin,
            self.context,
            self.context.work_fervor + self.context.quota_fervor,
        );

        if !is_feral {
            let research_utility = if self.data.is_nostalgic {
                None
            } else {
                evaluate_research(
                    self.data.is_nostalgic,
                    pop_pos,
                    &weights,
                    self.context.resources,
                    &self.buffer.libraries,
                )
            };
            self.evaluator.evaluate_and_consider(
                research_utility,
                ActionType::Research,
                self.context,
                0.0,
            );
        }
    }

    fn evaluate_policing(&mut self) {
        let pop_pos = self.data.pos;
        let weights = self.data.weights;

        self.evaluator.evaluate_and_consider(
            evaluate_warden_action(
                &pop_pos,
                &self.buffer.wanted_criminals,
                self.context.zone_grid,
            ),
            ActionType::Warden,
            self.context,
            0.0,
        );

        self.evaluator.evaluate_and_consider(
            evaluate_pre_crime_arrest(&pop_pos, &weights, &self.buffer.suspects),
            ActionType::PreCrimeArrest,
            self.context,
            0.0,
        );
    }

    /// **Priority 3: Work & Production**
    ///
    /// The core economic driver. Ignored if striking.
    ///
    /// *   **Work**: General labor (Mining, Building).
    /// *   **Refine**: Manufacturing jobs.
    /// *   **Farm**: Food production.
    /// *   **Admin**: Bureaucracy.
    /// *   **Research**: Science.
    /// *   **Tame**: Animal husbandry.
    /// *   **Warden**: Policing.
    /// *   **PreCrimeArrest**: Predictive policing.
    #[allow(clippy::collapsible_if)]
    fn evaluate_group_work(&mut self) {
        if self.is_striking {
            self.evaluator.evaluate_and_consider(
                crate::layer1::actions::evaluate_protest(
                    self.data.pos,
                    &self.data.weights,
                    &self.buffer.mobs,
                ),
                ActionType::Protest,
                self.context,
                0.0,
            );
            return;
        }

        if self.is_noble {
            return;
        }

        self.evaluate_work_and_taming();

        if self.is_penal {
            return;
        }

        self.evaluate_production();
        self.evaluate_policing();
        self.evaluate_phantom_shift();
    }

    /// **Spec 290: The Phantom Shift**
    ///
    /// Fringe pops work a secret night shift repairing damaged structures
    /// when the colony backlog is critical. Outbids ordinary rest but loses
    /// to critical exhaustion.
    fn evaluate_phantom_shift(&mut self) {
        // Striking, noble, and penal pops never work the phantom shift.
        if self.is_striking || self.is_noble || self.is_penal {
            return;
        }
        self.evaluator.evaluate_and_consider(
            crate::layer1::phantom_shift::evaluate_phantom_shift(
                self.data,
                self.buffer,
                self.context,
            ),
            ActionType::PhantomWork,
            self.context,
            0.0,
        );
    }

    /// **Priority 4: Logistics & Maintenance**
    ///
    /// Keeping the colony running.
    ///
    /// *   **FetchTool/Clothing**: Upgrading personal equipment.
    /// *   **Repair**: Fixing damaged buildings.
    /// *   **Haul**: Moving items to stockpiles.
    /// *   **BuryCorpse**: Sanitation.
    fn evaluate_group_logistics(&mut self) {
        if self.is_striking || self.is_noble {
            return;
        }

        let pop_pos = self.data.pos;
        let weights = self.data.weights;
        let equipment_opt = self.data.equipment;

        // Evaluate FetchTool
        let equipment = equipment_opt.unwrap_or_default();
        self.evaluator.evaluate_and_consider(
            evaluate_fetch_tool(
                pop_pos,
                &equipment,
                self.context.resources,
                &self.buffer.stockpiles,
            ),
            ActionType::FetchTool,
            self.context,
            0.0,
        );

        // Evaluate FetchClothing
        self.evaluator.evaluate_and_consider(
            evaluate_fetch_clothing(
                pop_pos,
                self.data.insulation,
                self.context.resources,
                &self.buffer.stockpiles,
                self.context.temperature_grid,
            ),
            ActionType::FetchClothing,
            self.context,
            0.0,
        );

        // Evaluate Repair
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.repair_structures, 0.6),
            ActionType::Repair,
            self.context,
            0.0,
        );
        // Also check designations
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.repair_designations, 0.6),
            ActionType::Repair,
            self.context,
            0.0,
        );

        // Evaluate Haul
        self.evaluator.evaluate_and_consider(
            evaluate_haul(
                pop_pos,
                &weights,
                &self.buffer.items,
                &self.buffer.item_entities,
                &self.buffer.stockpiles,
                &self.buffer.gene_banks,
                self.context.resources,
                self.data.carrying,
                self.data.carrying_item,
                self.data.carrying_item_type,
            ),
            ActionType::Haul,
            self.context,
            0.0,
        );

        // Evaluate BuryCorpse
        if !self.buffer.graves.is_empty() {
            self.evaluator.evaluate_and_consider(
                evaluate_simple_action(pop_pos, &weights, &self.buffer.corpses, 0.8),
                ActionType::BuryCorpse,
                self.context,
                0.0,
            );
        }
    }

    /// **Priority 5: Exploration**
    ///
    /// Investigating the unknown.
    ///
    /// *   **Explore**: Investigating anomalies.
    fn evaluate_group_exploration(&mut self) {
        if self.is_striking || self.is_penal {
            return;
        }

        let pop_pos = self.data.pos;
        let weights = self.data.weights;

        // Evaluate Explore
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.anomalies, 0.3),
            ActionType::Explore,
            self.context,
            0.0,
        );
    }

    /// **Priority 6: Cleanliness**
    ///
    /// *   **PurgeResidue**: Removing tech bloat.
    /// *   **Clean**: Removing clutter/filth.
    fn evaluate_group_maintenance(&mut self) {
        if self.is_striking || self.is_penal {
            return;
        }

        let pop_pos = self.data.pos;
        let weights = self.data.weights;

        // Evaluate PurgeResidue
        self.evaluator.evaluate_and_consider(
            evaluate_simple_action(pop_pos, &weights, &self.buffer.residues, 0.4),
            ActionType::PurgeResidue,
            self.context,
            0.0,
        );

        // Evaluate Clean
        self.evaluator.evaluate_and_consider(
            evaluate_clean(pop_pos, &weights, &self.buffer.cleaning_targets, false),
            ActionType::Clean,
            self.context,
            0.0,
        );
    }

    /// **Priority 7: Leisure**
    ///
    /// Optional activities when everything else is satisfied.
    ///
    /// *   **Hobby**: Personal projects.
    /// *   **The Hum**: Tuning in to the void.
    fn evaluate_group_leisure(&mut self) {
        if let Some(hobby_type) = self.data.hobby_type {
            let utility = evaluate_hobby(self.data, hobby_type);
            let penalty =
                crate::layer1::taboo::evaluate_taboo_penalty(ActionType::Hobby, self.context.taboo);
            self.evaluator
                .consider(ActionType::Hobby, utility + penalty, None);
        }

        // Evaluate ListenToTheHum
        let (_, score, target) = evaluate_listen_to_hum(self.data, self.buffer);
        if let Some(target_entity) = target {
            self.evaluator.evaluate_and_consider(
                Some((score, target_entity)),
                ActionType::ListenToTheHum,
                self.context,
                0.0,
            );
        }
    }

    fn run(mut self) -> (ActionType, f32, Option<Entity>) {
        self.evaluate_group_survival();
        self.evaluate_group_social();
        self.evaluate_group_leisure();
        self.evaluate_group_work();
        self.evaluate_group_logistics();
        self.evaluate_group_exploration();
        self.evaluate_group_maintenance();

        self.evaluator.result()
    }
}

/// Helper function to evaluate all potential actions for a single Pop.
///
/// Returns the best `(ActionType, Utility, Target)`.
#[doc(hidden)]
#[allow(clippy::too_many_lines, clippy::collapsible_if)]
pub(crate) fn evaluate_single_pop(
    buffer: &UtilityAIBuffer,
    data: &PopEvalData,
    context: &WorldContext,
) -> (ActionType, f32, Option<Entity>) {
    // 1. Check for Existential Crisis (Returns early)
    if let Some(crisis) = data.existential_crisis {
        if crisis.duration > 0 {
            return (ActionType::Philosophize, 100.0, None);
        }
    }

    // 1a. Check for Mental Break (Returns early)
    if let Some((action, utility, target)) = evaluate_mental_break(data, buffer) {
        return (action, utility, target);
    }

    // 1b. Check for Memetic Compulsion (Returns early, overrides drafted)
    if let Some((action, utility, target)) =
        crate::layer1::memetics::evaluate_scrawl_memetic_sigil(data, buffer)
    {
        return (action, utility, target);
    }

    if let Some((action, utility, target)) =
        crate::layer1::memetics::parasitic_broadcast::evaluate_memetic_obsession(data, buffer)
    {
        return (action, utility, target);
    }

    // 1c. Check for Nostalgia Sabotage (Overrides normal behavior)
    if let Some((action, utility, target)) =
        crate::layer1::actions::sabotage::evaluate_sabotage(data, buffer)
    {
        return (action, utility, target);
    }

    // 2. Check for Drafted (Returns early)
    if let Some((action, utility, target)) = evaluate_drafted_behavior(data, buffer) {
        return (action, utility, target);
    }

    // 3. Normal evaluation (undrafted, sane)
    let decider = PopDecider::new(data, buffer, context);
    decider.run()
}

fn run_evaluations(
    buffer: &UtilityAIBuffer,
    context: &WorldContext,
    results: &mut [Option<(ActionType, f32, Option<Entity>)>],
) {
    let pool = ComputeTaskPool::get();
    let pop_count = buffer.pop_data.len();
    let thread_count = pool.thread_num();
    let chunk_size = (pop_count / thread_count).max(1);

    // Results slice is already provided and resized
    let mut rest = results;

    // NB: the closure parameter is deliberately unannotated: the native
    // multi-threaded pool passes `&Scope`, the wasm32 single-threaded pool
    // passes `&mut Scope`. Inference picks the right one per target.
    pool.scope(|scope| {
        for chunk in buffer.pop_data.chunks(chunk_size) {
            // Split the results slice to get a mutable chunk for this thread
            let (result_chunk, remaining) = rest.split_at_mut(chunk.len());
            rest = remaining;

            scope.spawn(async move {
                for (i, data) in chunk.iter().enumerate() {
                    let result = evaluate_single_pop(buffer, data, context);
                    result_chunk[i] = Some(result);
                }
            });
        }
    });
}

fn apply_evaluation_results(
    world: &mut World,
    pop_data: &[PopEvalData],
    results: &[Option<(ActionType, f32, Option<Entity>)>],
    config: &UtilityConfig,
) {
    for (i, data) in pop_data.iter().enumerate() {
        let Some((best_action, best_utility, best_target)) = results[i] else {
            continue;
        };

        // Switch if best exceeds threshold
        if best_utility <= data.action.current_utility + config.switch_threshold {
            continue;
        }

        // Update action
        let mut action = data.action;
        action.current = best_action;
        action.current_utility = best_utility;
        action.ticks_committed = 0;

        // Write back to world
        if let Some(mut pop_action) = world.get_mut::<PopAction>(data.entity) {
            *pop_action = action;
        }

        // Insert StartPlan marker (for HTN system)
        world.entity_mut(data.entity).insert(StartPlan {
            action: best_action,
            target: best_target,
        });
    }
}

/// Evaluates all AI actions and updates `PopAction` for every entity.
///
/// This system drives the main behavior loop of the colony. It runs every tick to evaluate
/// whether a Pop should switch to a higher-utility task. Because evaluating hundreds of
/// options for thousands of Pops is expensive, this system uses a "Gather-Think-Act" cycle
/// to run evaluations in parallel across all CPU cores.
///
/// # Examples
///
/// ```
/// use scale::prelude::*;
/// use bevy_ecs::prelude::*;
///
/// let mut world = setup_world_with_config(SetupConfig::default());
///
/// // In your app setup:
/// // app.add_systems(Update, scale::layer1::mind::evaluate_actions_system);
/// ```
///
/// # Panics
///
/// This system will not panic under normal circumstances, but relies on
/// `UtilityConfig` being present in the ECS world.
pub fn evaluate_actions_system(world: &mut World) {
    let mut ctx = ScopedEvaluationContext::new(world);

    if ctx.buffer.pop_data.is_empty() {
        ctx.restore(world);
        return;
    }

    ctx.populate(world);
    ctx.run();
    ctx.apply(world);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::farm::Farm;
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world
    }

    #[test]
    fn test_update_action_timer() {
        let mut world = setup();
        let pop = world.spawn(PopAction::default()).id();

        // Run system
        world
            .run_system_once(update_action_timer_system)
            .expect("System should not panic");

        assert_eq!(
            world
                .get::<PopAction>(pop)
                .expect("PopAction should exist")
                .ticks_committed,
            1
        );
    }

    #[test]
    fn test_evaluate_actions_switches_when_threshold_exceeded() {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(UtilityConfig::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(10, 10));

        // Starving pop currently idle
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Needs {
                    hunger: 0.1,
                    rest: 0.8,
                    leisure: 0.8,
                    hygiene: 0.8,
                }, // Very hungry!
                UtilityWeights::default(),
                PopAction {
                    current: ActionType::Idle,
                    current_utility: 0.2,
                    ticks_committed: 10, // Past evaluation interval
                },
            ))
            .id();

        // Available farm
        world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 3, y: 0 },
            Farm::default(),
        ));

        evaluate_actions_system(&mut world);

        // Should have switched to SatisfyHunger
        let action = world.get::<PopAction>(pop).unwrap();
        assert_eq!(action.current, ActionType::SatisfyHunger);
        assert!(
            action.current_utility > 0.5,
            "Hungry pop should have high hunger utility"
        );
        assert_eq!(action.ticks_committed, 0, "Should reset commitment counter");
    }

    #[test]
    fn test_evaluate_actions_respects_threshold() {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(UtilityConfig {
            switch_threshold: 0.5, // High threshold
            ..Default::default()
        });
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(10, 10));

        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Needs {
                    hunger: 0.6,
                    rest: 0.6,
                    leisure: 0.6,
                    hygiene: 0.6,
                }, // Moderate needs
                UtilityWeights::default(),
                PopAction {
                    current: ActionType::Idle,
                    current_utility: 0.4,
                    ticks_committed: 10,
                },
            ))
            .id();

        world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 3, y: 0 },
            Farm::default(),
        ));

        evaluate_actions_system(&mut world);

        // Should NOT switch (utility difference < threshold)
        let action = world.get::<PopAction>(pop).unwrap();
        assert_eq!(
            action.current,
            ActionType::Idle,
            "Should not switch with high threshold"
        );
    }

    #[test]
    fn test_penal_labor_prioritizes_work() {
        use crate::layer1::designation::{Designation, DesignationType};
        use crate::layer1::law::justice::Inmate;
        use crate::layer1::law::penal::PenalLabor;

        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(UtilityConfig::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(10, 10));

        // Inmate with PenalLabor
        let inmate = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Needs::default(),
                UtilityWeights::default(),
                PopAction {
                    ticks_committed: 10,
                    ..Default::default()
                },
                Inmate {
                    sentence_ticks: 100,
                },
                PenalLabor::default(),
            ))
            .id();

        // Work designation available
        world.spawn((
            Designation {
                designation_type: DesignationType::Mine,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Available tavern (normally attractive)
        world.spawn((
            crate::layer1::social::Tavern::default(),
            GridPosition { x: 1, y: 1 },
        ));

        evaluate_actions_system(&mut world);

        let action = world.get::<PopAction>(inmate).unwrap();
        assert_eq!(
            action.current,
            ActionType::Work,
            "Penal Labor should prioritize Work over other actions"
        );
        assert!(
            action.current_utility > 1.0,
            "Should have high utility bonus"
        );
    }
}

#[cfg(test)]
mod synth_tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::traits::{Trait, Traits};
    use crate::layer1::utility_types::ActionType;

    #[test]
    fn test_synth_pop_ignores_fire_emergency() {
        let mut world = World::new();

        // Arrange
        let synth_traits = {
            let mut t = Traits::default();
            t.add(Trait::Synth);
            t
        };

        let mut rng = rand::thread_rng();
        let mut bundle = crate::layer1::pop::PopBundle::random(10, 10, &mut rng);
        bundle.traits = synth_traits;

        let _synth = world.spawn(bundle).id();

        let fire = world
            .spawn((
                crate::layer1::nature::fire::Fire::default(),
                GridPosition { x: 10, y: 11 }, // Adjacent
            ))
            .id();

        let hauling_job = world.spawn((GridPosition { x: 20, y: 20 },)).id();

        let mut evaluator = CandidateEvaluator::new(0.0, true);

        // Fake world context
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        world.insert_resource(crate::layer1::taboo::TabooState::default());
        world.insert_resource(crate::layer1::zone::ZoneGrid::new(1, 1));

        let context = crate::layer1::utility_eval_types::WorldContext {
            resources: world.resource::<crate::layer1::resources::ColonyResources>(),
            cycle: world.resource::<crate::layer1::day_night::DayNightCycle>(),
            taboo: world.resource::<crate::layer1::taboo::TabooState>(),
            factions: None,
            zone_grid: world.resource::<crate::layer1::zone::ZoneGrid>(),
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        // Try to consider ExtinguishFire with high utility (0.9)
        evaluator.evaluate_and_consider(
            Some((0.9, fire)),
            ActionType::ExtinguishFire,
            &context,
            0.0,
        );

        // Try to consider Haul with moderate utility (0.5)
        evaluator.evaluate_and_consider(Some((0.5, hauling_job)), ActionType::Haul, &context, 0.0);

        let (best_action, best_util, best_target) = evaluator.result();

        // The synth should choose to haul rather than extinguish the fire, because ExtinguishFire utility was zeroed.
        assert_eq!(best_action, ActionType::Haul);
        assert_eq!(best_target, Some(hauling_job));
        assert_eq!(best_util, 0.5);
    }
}
