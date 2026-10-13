//! Spec 347: Great Works.
//!
//! Multi-stage construction projects requiring massive resources and
//! time. This module upgrades the pre-existing Great Work scaffolding
//! (a [`GreatWork`] whose phases advanced on raw work points that were
//! never actually paid for, which nothing in production ever spawned)
//! into a real, funded megaproject pipeline:
//!
//! * A [`GreatWork`] built from [`Stage`]s — each stage costs a real
//!   [`ResourceType`] amount, per the spec's RED test shape
//!   (`Stage { resource, amount }`).
//! * Resources are *applied*, never conjured: the
//!   [`apply_resources_to_great_work`] system consumes
//!   [`ContributeToGreatWorkEvent`]s and pays each stage out of the real
//!   [`ColonyResources`] stockpile ledger (`get_amount` / `try_consume`).
//!   Partial payment accumulates in `stage_applied`; an unfunded stage
//!   does not advance and the stockpile never goes negative.
//! * [`ConstructionProgress`] mirrors the current stage (total = stage
//!   amount, current = applied), so the pre-existing phase machinery and
//!   the headless `great_works` panel keep working unchanged in shape.
//! * Completing the final stage inserts [`OperationalGreatWork`], emits
//!   [`GreatWorkCompletedEvent`] (the pre-existing chronicle bridge in
//!   `core::integration` turns that into a Legendary chronicle entry),
//!   and unlocks the Layer-2 benefit below.
//!
//! ## The Layer-2 benefit (one, wired end-to-end)
//! A completed Great Work is a Layer-2-class structure (an orbital
//! array, a sky-veil, a spire visible from orbit). While any work is
//! operational, [`great_work_awe_system`] refreshes an empire-wide
//! "Great Work Awe" [`MoodModifier`] (+0.15) on every pop's real
//! [`Morale`] component — the same empire-wide-modifier pattern as
//! spec 272's martyrdom fervor. The modifier flows through the real
//! `update_morale_cache_system`, so it moves `avg_morale` in STATS and
//! every system that reads morale. It is not a dead flag: remove the
//! last operational work and the modifier decays away on its own.
//!
//! ## Catalog
//! Three works with original in-universe names (IP rule: concept-only;
//! see the IP-guard test). The spec's example names are deliberately
//! NOT used.
//!
//! ## Design decisions / spec adaptations
//! * The spec sketch's `GreatWork::new(Vec<Stage>)` +
//!   `ConstructionProgress::default()` shape is kept verbatim;
//!   `ConstructionProgress` gained a `Default` derive to allow it.
//! * The sketch's completion predicate is `is_complete()`; the
//!   pre-existing code called it `is_completed()`. Both exist now —
//!   `is_completed()` delegates to `is_complete()`.
//! * Legacy phase-cost works (`phase_costs`, no stages) keep working:
//!   `stage_count()` falls back to `phase_costs.len()`, and
//!   `process_great_work_phases` still advances them on work points.
//!   Resource-driven works (non-empty `stages`) are skipped by that
//!   system so a stage can never be double-advanced.
//! * One contribution pays into the *current* stage only; overflow
//!   does not spill into the next stage (the colony chooses when to
//!   fund each stage). Contributions are explicit — headless
//!   `greatwork start <kind>` + `contribute [amount]` — never automatic,
//!   so a work can never silently drain the stockpile.

use crate::layer1::construction::{ConstructionCost, ConstructionProgress};
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;

/// Morale value of the empire-wide awe modifier (spec 347 benefit).
pub const GREAT_WORK_AWE_VALUE: f32 = 0.15;
/// Duration the awe modifier is refreshed to, in ticks.
pub const GREAT_WORK_AWE_REFRESH_TICKS: u32 = 120;
/// Label of the awe mood modifier on every pop's [`Morale`].
pub const GREAT_WORK_AWE_LABEL: &str = "Great Work Awe";

/// One construction stage of a [`GreatWork`]: a real stockpile cost
/// that must be fully applied before the work advances (spec RED type).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stage {
    /// Resource drawn from [`ColonyResources`] for this stage.
    pub resource: ResourceType,
    /// Total amount of `resource` this stage requires.
    pub amount: f32,
}

/// The three buildable Great Works (original names only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreatWorkKind {
    /// Orbital tether-and-lift array: stone foundations, metal spine,
    /// block counterweights.
    SkyhookArray,
    /// A sky-veil that shades and shields the colony from above.
    AegisVeil,
    /// A monumental spire raised to mark the colony's founding.
    FoundersSpire,
}

impl GreatWorkKind {
    /// Every buildable kind, in catalog order.
    pub const ALL: [Self; 3] = [Self::SkyhookArray, Self::AegisVeil, Self::FoundersSpire];

    /// Display name (original, in-universe).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SkyhookArray => "Skyhook Array",
            Self::AegisVeil => "Aegis Veil",
            Self::FoundersSpire => "Founders' Spire",
        }
    }

    /// Short console slug used by `greatwork start <slug>`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::SkyhookArray => "skyhook",
            Self::AegisVeil => "aegis",
            Self::FoundersSpire => "spire",
        }
    }

    /// Parse a kind from a headless console argument (slug or name).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "skyhook" | "skyhook array" | "array" => Some(Self::SkyhookArray),
            "aegis" | "aegis veil" | "veil" => Some(Self::AegisVeil),
            "spire" | "founders" | "founders' spire" | "founders spire" => {
                Some(Self::FoundersSpire)
            }
            _ => None,
        }
    }

    /// The stage costs of this work, in build order.
    #[must_use]
    pub fn stages(self) -> Vec<Stage> {
        match self {
            Self::SkyhookArray => vec![
                Stage { resource: ResourceType::Stone, amount: 200.0 },
                Stage { resource: ResourceType::Metal, amount: 100.0 },
                Stage { resource: ResourceType::Blocks, amount: 50.0 },
            ],
            Self::AegisVeil => vec![
                Stage { resource: ResourceType::Metal, amount: 150.0 },
                Stage { resource: ResourceType::Blocks, amount: 100.0 },
                Stage { resource: ResourceType::Fuel, amount: 50.0 },
            ],
            Self::FoundersSpire => vec![
                Stage { resource: ResourceType::Stone, amount: 150.0 },
                Stage { resource: ResourceType::Wood, amount: 100.0 },
                Stage { resource: ResourceType::Blocks, amount: 75.0 },
            ],
        }
    }

    /// Build the [`GreatWork`] component for this kind.
    #[must_use]
    pub fn great_work(self) -> GreatWork {
        let mut work = GreatWork::named(self.name(), self.stages());
        work.kind = Some(self);
        work
    }
}

#[derive(Component)]
pub struct GreatWork {
    pub name: String,
    pub current_phase: usize,
    pub phase_costs: Vec<ConstructionCost>,
    /// Resource-funded stages (spec 347). Empty for legacy works that
    /// advance on work points through `phase_costs` alone.
    pub stages: Vec<Stage>,
    /// Amount of the current stage's resource already applied.
    pub stage_applied: f32,
    /// Catalog kind, when this work came from [`GreatWorkKind`].
    pub kind: Option<GreatWorkKind>,
}

impl GreatWork {
    /// Spec constructor: a work defined purely by its resource stages.
    #[must_use]
    pub fn new(stages: Vec<Stage>) -> Self {
        Self::named("Unnamed Great Work", stages)
    }

    /// A named work defined by its resource stages.
    #[must_use]
    pub fn named(name: &str, stages: Vec<Stage>) -> Self {
        Self {
            name: name.to_string(),
            current_phase: 0,
            phase_costs: Vec::new(),
            stages,
            stage_applied: 0.0,
            kind: None,
        }
    }

    /// Number of stages/phases in this work (stages win when present).
    #[must_use]
    pub fn stage_count(&self) -> usize {
        if self.stages.is_empty() {
            self.phase_costs.len()
        } else {
            self.stages.len()
        }
    }

    /// Index of the stage currently being funded (== `current_phase`).
    #[must_use]
    pub fn current_stage(&self) -> usize {
        self.current_phase
    }

    /// The stage currently being funded, if any remains.
    #[must_use]
    pub fn current_stage_def(&self) -> Option<&Stage> {
        self.stages.get(self.current_phase)
    }

    /// Spec predicate: every stage has been fully applied.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.current_phase >= self.stage_count()
    }

    /// Pre-existing spelling, kept for existing callers.
    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.is_complete()
    }
}

#[derive(Component)]
pub struct OperationalGreatWork;

#[derive(Event, Debug, Clone)]
pub struct GreatWorkCompletedEvent {
    pub entity: Entity,
    pub name: String,
}

/// Event: apply up to `amount` of the current stage's resource, drawn
/// from the real [`ColonyResources`] stockpile, to a Great Work.
#[derive(Event, Debug, Clone)]
pub struct ContributeToGreatWorkEvent {
    /// The Great Work entity to fund.
    pub work: Entity,
    /// Maximum amount of the current stage's resource to apply.
    pub amount: f32,
}

pub fn spawn_great_work(
    world: &mut World,
    name: &str,
    phase_costs: Vec<ConstructionCost>,
) -> Entity {
    let initial_cost = phase_costs.first().cloned().unwrap_or(ConstructionCost {
        item_type: crate::layer1::items::ItemType::None,
        amount: 1,
    });
    world
        .spawn((
            GreatWork {
                name: name.to_string(),
                current_phase: 0,
                phase_costs,
                stages: Vec::new(),
                stage_applied: 0.0,
                kind: None,
            },
            ConstructionProgress {
                total_work_required: initial_cost.amount as f32 * 10.0,
                current_work: 0.0,
            },
        ))
        .id()
}

/// Spawn a catalog Great Work (spec 347), ready to receive contributions.
pub fn spawn_great_work_kind(world: &mut World, kind: GreatWorkKind) -> Entity {
    let work = kind.great_work();
    let first_total = work.current_stage_def().map_or(0.0, |s| s.amount);
    world
        .spawn((
            work,
            ConstructionProgress {
                total_work_required: first_total,
                current_work: 0.0,
            },
        ))
        .id()
}

/// System (spec GREEN): apply contributed resources to Great Works.
///
/// Each [`ContributeToGreatWorkEvent`] pays into the target work's
/// current stage from the real stockpile: the applied amount is the
/// minimum of the requested amount, the stage's remaining need, and
/// what the stockpile actually holds. When a stage is fully funded the
/// work advances; finishing the final stage makes it operational,
/// emits [`GreatWorkCompletedEvent`], and logs the completion.
pub fn apply_resources_to_great_work(
    mut events: EventReader<ContributeToGreatWorkEvent>,
    mut commands: Commands,
    mut works: Query<(Entity, &mut GreatWork, Option<&mut ConstructionProgress>)>,
    mut resources: ResMut<ColonyResources>,
    mut completed: EventWriter<GreatWorkCompletedEvent>,
    log: Option<ResMut<MessageLog>>,
) {
    let mut log = log;
    for event in events.read() {
        let Ok((entity, mut work, progress)) = works.get_mut(event.work) else {
            continue;
        };
        if work.is_complete() {
            continue;
        }
        let Some(stage) = work.current_stage_def().copied() else {
            continue;
        };
        let remaining = stage.amount - work.stage_applied;
        if remaining <= 0.0 || event.amount <= 0.0 {
            continue;
        }
        let want = event.amount.min(remaining);
        let applied = want.min(resources.get_amount(stage.resource));
        if applied <= 0.0 || !resources.try_consume(stage.resource, applied) {
            continue;
        }
        work.stage_applied += applied;
        let stage_done = work.stage_applied >= stage.amount - 0.001;
        if stage_done {
            work.current_phase += 1;
            work.stage_applied = 0.0;
        }
        // Mirror the (possibly new) current stage into ConstructionProgress
        // so the legacy phase panel/progress readers stay truthful.
        let finished = work.is_complete();
        if let Some(mut progress) = progress {
            if finished {
                progress.current_work = progress.total_work_required;
            } else if let Some(next) = work.current_stage_def() {
                progress.total_work_required = next.amount;
                progress.current_work = work.stage_applied;
            }
        }
        if finished {
            commands.entity(entity).insert(OperationalGreatWork);
            commands.entity(entity).remove::<ConstructionProgress>();
            completed.send(GreatWorkCompletedEvent {
                entity,
                name: work.name.clone(),
            });
            if let Some(log) = log.as_deref_mut() {
                log.add(format!(
                    "GREAT WORK COMPLETE: {} stands finished — the colony looks up in awe.",
                    work.name
                ));
            }
        }
    }
}

/// System (spec 347 benefit): while any Great Work is operational,
/// refresh the empire-wide "Great Work Awe" morale modifier on every
/// pop. Flows through the real morale cache into `avg_morale`.
pub fn great_work_awe_system(
    operational: Query<(), With<OperationalGreatWork>>,
    mut pops: Query<&mut Morale, With<Pop>>,
) {
    if operational.is_empty() {
        return;
    }
    for mut morale in &mut pops {
        if let Some(existing) = morale
            .modifiers
            .iter_mut()
            .find(|m| m.label == GREAT_WORK_AWE_LABEL)
        {
            existing.value = GREAT_WORK_AWE_VALUE;
            existing.duration = existing.duration.max(GREAT_WORK_AWE_REFRESH_TICKS);
        } else {
            morale.add_modifier(MoodModifier {
                label: GREAT_WORK_AWE_LABEL.to_string(),
                value: GREAT_WORK_AWE_VALUE,
                duration: GREAT_WORK_AWE_REFRESH_TICKS,
            });
        }
    }
}

/// Number of Great Works ever started (active + operational).
pub fn great_work_count(world: &mut World) -> usize {
    let mut query = world.query::<&GreatWork>();
    query.iter(world).count()
}

/// Number of completed (operational) Great Works.
pub fn great_work_completed_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<&GreatWork, With<OperationalGreatWork>>();
    query.iter(world).count()
}

/// Short status for headless STATS: "none" or "<done>/<total>done"
/// (no spaces — STATS fields are space-separated).
#[must_use]
pub fn great_works_status_label(world: &mut World) -> String {
    let total = great_work_count(world);
    if total == 0 {
        return "none".to_string();
    }
    let done = great_work_completed_count(world);
    format!("{done}/{total}done")
}

/// The first still-unfinished resource-funded work, if any (the
/// headless `contribute` command's default target).
pub fn first_active_great_work(world: &mut World) -> Option<Entity> {
    let mut query = world.query::<(Entity, &GreatWork)>();
    query
        .iter(world)
        .find(|(_, w)| !w.is_complete() && !w.stages.is_empty())
        .map(|(e, _)| e)
}

pub fn process_great_work_phases(
    mut commands: Commands,
    mut query: Query<(Entity, &mut GreatWork, &mut ConstructionProgress)>,
    mut events: EventWriter<GreatWorkCompletedEvent>,
) {
    for (entity, mut work, mut progress) in query.iter_mut() {
        if work.is_completed() {
            continue;
        }
        // Resource-funded works advance only through
        // apply_resources_to_great_work; never double-advance them here.
        if !work.stages.is_empty() {
            continue;
        }

        if progress.current_work >= progress.total_work_required {
            work.current_phase += 1;

            if work.is_completed() {
                commands.entity(entity).insert(OperationalGreatWork);
                commands.entity(entity).remove::<ConstructionProgress>();
                events.send(GreatWorkCompletedEvent {
                    entity,
                    name: work.name.clone(),
                });
            } else {
                // Setup next phase
                let next_cost = &work.phase_costs[work.current_phase];
                progress.total_work_required = next_cost.amount as f32 * 10.0;
                progress.current_work = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::construction::{ConstructionCost, ConstructionProgress};

    #[test]
    fn test_great_work_initializes_in_phase_one() {
        let mut world = World::new();
        let work_id = spawn_great_work(
            &mut world,
            "Space Elevator",
            vec![
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 1000,
                },
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 500,
                },
            ],
        );

        let work = world.get::<GreatWork>(work_id).unwrap();
        assert_eq!(work.current_phase, 0);
        assert!(!work.is_completed());
    }

    #[test]
    fn test_great_work_advances_phase_when_progress_filled() {
        let mut world = World::new();
        let work_id = spawn_great_work(
            &mut world,
            "Planetary Shield",
            vec![
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 100,
                }, // Phase 0
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 5,
                }, // Phase 1
            ],
        );

        world.init_resource::<Events<GreatWorkCompletedEvent>>();

        // Manually complete phase 0
        let mut progress = world.get_mut::<ConstructionProgress>(work_id).unwrap();
        progress.current_work = progress.total_work_required;

        let mut schedule = Schedule::default();
        schedule.add_systems(process_great_work_phases);
        schedule.run(&mut world);

        let work = world.get::<GreatWork>(work_id).unwrap();
        assert_eq!(work.current_phase, 1, "Should have advanced to phase 1.");
        assert!(!work.is_completed(), "Should not be completed yet.");
    }

    #[test]
    fn test_great_work_completion_triggers_buff() {
        let mut world = World::new();
        let work_id = spawn_great_work(
            &mut world,
            "Monument",
            vec![ConstructionCost {
                item_type: crate::layer1::items::ItemType::None,
                amount: 10,
            }],
        );

        world.init_resource::<Events<GreatWorkCompletedEvent>>();

        // Complete the only phase
        let mut progress = world.get_mut::<ConstructionProgress>(work_id).unwrap();
        progress.current_work = progress.total_work_required;

        let mut schedule = Schedule::default();
        schedule.add_systems(process_great_work_phases);
        schedule.run(&mut world);

        let work = world.get::<GreatWork>(work_id).unwrap();
        assert!(work.is_completed(), "Should be completed.");
        // Verify the component changed state or emitted an event
        assert!(world.get::<OperationalGreatWork>(work_id).is_some());
    }

    #[test]
    fn test_great_work_completion_emits_event() {
        let mut world = World::new();
        let work_id = spawn_great_work(
            &mut world,
            "Monument",
            vec![ConstructionCost {
                item_type: crate::layer1::items::ItemType::None,
                amount: 10,
            }],
        );

        world.init_resource::<Events<GreatWorkCompletedEvent>>();

        let mut progress = world.get_mut::<ConstructionProgress>(work_id).unwrap();
        progress.current_work = progress.total_work_required;

        let mut schedule = Schedule::default();
        schedule.add_systems(process_great_work_phases);
        schedule.run(&mut world);

        let events = world.resource::<Events<GreatWorkCompletedEvent>>();
        let mut cursor = events.get_cursor();
        let iter = cursor.read(events);
        assert_eq!(
            iter.count(),
            1,
            "Should have emitted exactly one completion event"
        );
    }

    #[test]
    fn test_great_work_advances_multiple_phases_correctly() {
        let mut world = World::new();
        let work_id = spawn_great_work(
            &mut world,
            "Planetary Shield",
            vec![
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 100,
                }, // Phase 0
                ConstructionCost {
                    item_type: crate::layer1::items::ItemType::None,
                    amount: 5,
                }, // Phase 1
            ],
        );

        world.init_resource::<Events<GreatWorkCompletedEvent>>();

        let mut schedule = Schedule::default();
        schedule.add_systems(process_great_work_phases);

        // Manually complete phase 0
        {
            let mut progress = world.get_mut::<ConstructionProgress>(work_id).unwrap();
            progress.current_work = progress.total_work_required;
        }

        schedule.run(&mut world);

        {
            let work = world.get::<GreatWork>(work_id).unwrap();
            assert_eq!(work.current_phase, 1, "Should have advanced to phase 1.");
            assert!(!work.is_completed(), "Should not be completed yet.");
        }

        // Manually complete phase 1
        {
            let mut progress = world.get_mut::<ConstructionProgress>(work_id).unwrap();
            progress.current_work = progress.total_work_required;
        }

        schedule.run(&mut world);

        {
            let work = world.get::<GreatWork>(work_id).unwrap();
            assert_eq!(work.current_phase, 2, "Should have advanced to phase 2.");
            assert!(work.is_completed(), "Should be completed.");
            assert!(world.get::<OperationalGreatWork>(work_id).is_some());
        }
    }
}

#[cfg(test)]
mod spec347_tests {
    use super::*;
    use crate::layer1::economy::resources::{ColonyResources, ResourceType};
    use crate::layer1::pop::Pop;
    use crate::layer1::social::morale::Morale;
    use bevy_ecs::system::RunSystemOnce;

    fn funded_world(stone: f32, metal: f32) -> World {
        let mut world = World::new();
        let mut res = ColonyResources::zeroed();
        res.stone = stone;
        res.metal = metal;
        world.insert_resource(res);
        world.init_resource::<Events<ContributeToGreatWorkEvent>>();
        world.init_resource::<Events<GreatWorkCompletedEvent>>();
        world
    }

    fn contribute(world: &mut World, work: Entity, amount: f32) {
        world
            .resource_mut::<Events<ContributeToGreatWorkEvent>>()
            .send(ContributeToGreatWorkEvent { work, amount });
        world.run_system_once(apply_resources_to_great_work).unwrap();
    }

    /// Spec 347 RED test, adapted to real types: stages are paid from the
    /// real ColonyResources stockpile via ContributeToGreatWorkEvent.
    #[test]
    fn test_great_work_multi_stage_construction() {
        let mut world = funded_world(1000.0, 500.0);
        let work = world
            .spawn((
                GreatWork::new(vec![
                    Stage { resource: ResourceType::Stone, amount: 1000.0 },
                    Stage { resource: ResourceType::Metal, amount: 500.0 },
                ]),
                ConstructionProgress::default(),
            ))
            .id();

        contribute(&mut world, work, 1000.0);
        let gw = world.get::<GreatWork>(work).unwrap();
        assert_eq!(gw.current_stage(), 1, "Should advance to next stage");
        assert!(!gw.is_complete(), "Work should not be fully complete");

        contribute(&mut world, work, 500.0);
        let gw = world.get::<GreatWork>(work).unwrap();
        assert!(gw.is_complete(), "Work should be fully complete");
        assert!(world.get::<OperationalGreatWork>(work).is_some());
    }

    #[test]
    fn test_contribution_draws_from_real_stockpile() {
        let mut world = funded_world(1000.0, 500.0);
        let work = world
            .spawn((
                GreatWork::new(vec![
                    Stage { resource: ResourceType::Stone, amount: 1000.0 },
                    Stage { resource: ResourceType::Metal, amount: 500.0 },
                ]),
                ConstructionProgress::default(),
            ))
            .id();
        contribute(&mut world, work, 1000.0);
        let res = world.resource::<ColonyResources>();
        assert_eq!(res.stone, 0.0, "stone stage must be paid from stockpile");
        assert_eq!(res.metal, 500.0, "later stages must not be pre-paid");
    }

    #[test]
    fn test_contribution_partial_when_stockpile_short() {
        let mut world = funded_world(400.0, 0.0);
        let work = world
            .spawn((
                GreatWork::new(vec![Stage { resource: ResourceType::Stone, amount: 1000.0 }]),
                ConstructionProgress::default(),
            ))
            .id();
        contribute(&mut world, work, 1000.0);
        let gw = world.get::<GreatWork>(work).unwrap();
        assert_eq!(gw.current_stage(), 0, "underfunded stage must not advance");
        assert_eq!(gw.stage_applied, 400.0, "partial payment must accumulate");
        assert_eq!(world.resource::<ColonyResources>().stone, 0.0);

        world.resource_mut::<ColonyResources>().stone = 600.0;
        contribute(&mut world, work, 1000.0);
        assert!(world.get::<GreatWork>(work).unwrap().is_complete());
    }

    #[test]
    fn test_contribution_without_stockpile_makes_no_progress() {
        let mut world = funded_world(0.0, 0.0);
        let work = world
            .spawn((
                GreatWork::new(vec![Stage { resource: ResourceType::Stone, amount: 100.0 }]),
                ConstructionProgress::default(),
            ))
            .id();
        contribute(&mut world, work, 100.0);
        let gw = world.get::<GreatWork>(work).unwrap();
        assert_eq!(gw.stage_applied, 0.0);
        assert!(!gw.is_complete());
        assert!(world.resource::<ColonyResources>().stone >= 0.0);
    }

    #[test]
    fn test_completion_emits_event_via_contribution() {
        let mut world = funded_world(100.0, 0.0);
        let work = world
            .spawn((
                GreatWork::new(vec![Stage { resource: ResourceType::Stone, amount: 100.0 }]),
                ConstructionProgress::default(),
            ))
            .id();
        contribute(&mut world, work, 100.0);
        let events = world.resource::<Events<GreatWorkCompletedEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 1);
    }

    #[test]
    fn test_catalog_works_are_massive_and_parseable() {
        assert_eq!(GreatWorkKind::ALL.len(), 3);
        for kind in GreatWorkKind::ALL {
            assert!(!kind.stages().is_empty());
            let total: f32 = kind.stages().iter().map(|s| s.amount).sum();
            assert!(total >= 200.0, "{} must be massive, got {total}", kind.name());
            assert_eq!(GreatWorkKind::parse(kind.slug()), Some(kind));
            assert!(!kind.name().is_empty());
        }
    }

    #[test]
    fn test_awe_modifier_only_when_operational() {
        let mut world = World::new();
        let pop = world.spawn((Pop, Morale::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems(great_work_awe_system);
        schedule.run(&mut world);
        assert!(
            world.get::<Morale>(pop).unwrap().modifiers.iter().all(|m| m.label != GREAT_WORK_AWE_LABEL),
            "no operational work, no awe"
        );
        world.spawn((GreatWork::new(vec![]), OperationalGreatWork));
        schedule.run(&mut world);
        let morale = world.get::<Morale>(pop).unwrap();
        let awe = morale.modifiers.iter().find(|m| m.label == GREAT_WORK_AWE_LABEL)
            .expect("operational great work must grant the awe modifier");
        assert!(awe.value > 0.0);
    }

    #[test]
    fn test_great_works_status_label() {
        let mut world = funded_world(0.0, 0.0);
        assert_eq!(great_works_status_label(&mut world), "none");
        spawn_great_work_kind(&mut world, GreatWorkKind::FoundersSpire);
        assert_eq!(great_works_status_label(&mut world), "0/1done");
    }

    #[test]
    fn test_completion_reaches_chronicle_via_bridge() {
        use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
        let mut world = funded_world(100.0, 0.0);
        world.init_resource::<Events<AddChronicleEvent>>();
        let work = world
            .spawn((
                GreatWork::named(
                    "Founders' Spire",
                    vec![Stage { resource: ResourceType::Stone, amount: 100.0 }],
                ),
                ConstructionProgress::default(),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems((
            apply_resources_to_great_work,
            crate::layer1::core::integration::great_work_chronicle_bridge,
        ));
        world
            .resource_mut::<Events<ContributeToGreatWorkEvent>>()
            .send(ContributeToGreatWorkEvent { work, amount: 100.0 });
        schedule.run(&mut world);
        schedule.run(&mut world);
        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        let entries: Vec<_> = cursor.read(events).collect();
        assert_eq!(entries.len(), 1, "exactly one chronicle entry");
        assert!(matches!(entries[0].importance, EventImportance::Legendary));
        assert!(entries[0].text.contains("Founders' Spire"), "{}", entries[0].text);
    }

    #[test]
    fn test_great_work_ip_guard() {
        const BANNED: &[&str] = &["Space Elevator", "Planetary Shield", "Dyson", "Ringworld", "Death Star"];
        for kind in GreatWorkKind::ALL {
            for banned in BANNED {
                assert!(!kind.name().contains(banned), "catalog name leaks IP: {}", kind.name());
            }
        }
        assert!(!GREAT_WORK_AWE_LABEL.contains("Space Elevator"));
    }
}
