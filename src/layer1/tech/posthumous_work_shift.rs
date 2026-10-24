//! The Posthumous Work Shift (Spec 361).
//!
//! When a pop dies *at work*, their brain-scan backup is mapped onto
//! an automated chassis at their workstation: a [`DigitalGhost`]
//! rises, finishes exactly the shift the pop was in the middle of,
//! and then decays — permanently. There is no second shift. The
//! colony gets its finished work; the living get a haunted
//! workplace; the dead get to clock out exactly once, late.
//!
//! ## Design notes (adaptations of the spec's mock-style RED tests)
//!
//! * The spec's `ActiveJob` does not exist. The real "at work" seam
//!   is the one Spec 359 established: a pop is on shift iff their
//!   [`PopAction`] is a work action (`Farm` / `Research` / `Admin` /
//!   `Refine` / `Work`, see
//!   [`is_work_action`](crate::layer1::social::shift_work::is_work_action)).
//!   A [`Job`] alone is *not* sufficient — `Job` is assigned on
//!   arrival at a workplace and never cleared when the pop goes to
//!   bed, so a Job-only seam would raise ghosts for pops who died
//!   asleep (the exact false positive Spec 359's seam decision
//!   rejected). The ghost still *captures* the pop's `Job` (and any
//!   [`CurrentTask`]) — it just doesn't rise for a stale job alone.
//! * The death hook is the real pipeline: this module's spawn system
//!   reads [`PopDied`] after `handle_pop_death_system` emits it and
//!   runs before `despawn_dead_entities_system`, so the corpse's
//!   components are still readable (the Spec 272 martyrdom pattern).
//! * A ghost is **not** a pop. It is spawned with no `Pop`, no
//!   [`Needs`], no `Health`, and no [`Morale`] — so the needs,
//!   hunger, pressure/oxygen, rest, and morale systems, which all
//!   query `With<Pop>`, cannot make it eat, breathe, sleep, or feel.
//!   It also carries no `PopAction`, so the execution layer has
//!   nothing to drive; its one shift is worked by
//!   [`ghost_work_system`] and nothing else.
//! * New-task rejection is structural *and* explicit: the utility
//!   AI's `collect_pop_data` carries a `Without<DigitalGhost>`
//!   filter (the same pattern that excludes panicked and possessed
//!   pops), and [`DigitalGhost::try_assign_new_task`] refuses every
//!   assignment at the component level.
//! * The morale cost flows through the real mood system: living pops
//!   near a working ghost carry a single refreshed-never-stacking
//!   `"Haunted Workplace"` [`MoodModifier`] (the Spec 359 refresh
//!   pattern), removed the first tick they are out of range.
//! * Completion is real when there is something to complete: a ghost
//!   that inherited a [`CurrentTask`] credits its output
//!   (`base_output × efficiency`) to [`ColonyResources`] when the
//!   shift ends. A ghost with only a workplace job leaves its mark
//!   in [`PosthumousStats`] and the chronicle — the shift itself was
//!   the product.
//! * All names are original and generic ("Digital Ghost",
//!   "Haunted Workplace"); nothing is lifted from outside fiction
//!   (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::entities::pop::{Job, Pop, PopDied};
use crate::layer1::health::Dead;
use crate::layer1::jobs::CurrentTask;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::shift_work::is_work_action;
use crate::layer1::utility_types::{ActionType, PopAction};

/// Ticks of work a ghost serves when the dead pop left no timed
/// task: the remainder of one shift, then decay.
pub const GHOST_SHIFT_TICKS: u32 = 100;
/// Floor for a ghost shift derived from a [`CurrentTask`] duration:
/// even a nearly-finished task takes a few ticks to hand over.
pub const GHOST_MIN_SHIFT_TICKS: u32 = 5;
/// Cap for a ghost shift derived from a [`CurrentTask`] duration:
/// a ghost is a backup, not a replacement worker.
pub const GHOST_MAX_SHIFT_TICKS: u32 = 500;
/// Label of the haunted-workplace mood modifier.
pub const HAUNTED_LABEL: &str = "Haunted Workplace";
/// Mood penalty for working alongside a digital ghost.
pub const HAUNTED_PENALTY: f32 = -0.20;
/// Chebyshev radius (tiles) over which a working ghost haunts.
pub const HAUNTED_RADIUS: i32 = 6;
/// Modifier duration, refreshed every haunted tick (the
/// refresh-never-stack pattern; matches Spec 359's 100).
pub const HAUNTED_DURATION: u32 = 100;

/// A snapshot of the timed task a dead pop left unfinished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GhostTask {
    /// The [`CurrentTask::task_id`] of the original task.
    pub task_id: u32,
    /// Resource credited to the colony when the ghost finishes.
    pub output: ResourceType,
    /// Output multiplier inherited from the original task.
    pub efficiency: f32,
}

impl From<&CurrentTask> for GhostTask {
    fn from(task: &CurrentTask) -> Self {
        Self {
            task_id: task.task_id,
            output: task.base_output,
            efficiency: task.efficiency,
        }
    }
}

/// A digital ghost: a dead worker's brain-scan backup mapped onto
/// an automated chassis to finish exactly one shift.
///
/// Deliberately *not* a pop: no `Pop`, `Needs`, `Health`, or
/// `Morale` components ever attach to this entity (see module docs).
#[derive(Component, Clone, Debug)]
pub struct DigitalGhost {
    /// Name of the pop this ghost is a backup of.
    pub deceased_name: String,
    /// The dead pop's entity (already despawned by the time the
    /// ghost works; kept as a record, never dereferenced live).
    pub deceased_entity: Entity,
    /// The action the pop was performing when they died.
    pub action: ActionType,
    /// The pop's persistent workplace job, if they had one.
    pub job: Option<Job>,
    /// The timed task the pop left unfinished, if any.
    pub task: Option<GhostTask>,
    /// Ticks of the inherited shift still to serve.
    pub remaining_ticks: u32,
    /// Ticks of the inherited shift already served.
    pub worked_ticks: u32,
    /// Set on the tick the shift completes; the ghost despawns the
    /// same tick (decay is not a second career).
    pub completed: bool,
}

impl DigitalGhost {
    /// A ghost's whole existence is its one inherited shift: every
    /// attempt to assign it new work is refused, and its inherited
    /// action is left untouched.
    pub fn try_assign_new_task(&mut self, _action: ActionType) -> Result<(), &'static str> {
        Err("a digital ghost cannot be assigned new tasks")
    }
}

/// Colony ledger for the posthumous work shift.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PosthumousStats {
    /// Ghosts raised since colony start.
    pub spawned: u32,
    /// Ghost shifts completed (and ghosts decayed) since start.
    pub completed: u32,
}

/// The "at work" seam (see module docs): the pop's current action
/// is a work action. Pure and unit-testable.
#[must_use]
pub fn is_active_shift(action: Option<&PopAction>) -> bool {
    action.is_some_and(|a| is_work_action(a.current))
}

/// How long the ghost of a pop with this timed task serves: the
/// task's remaining duration, clamped so a ghost is always a
/// hand-over, never a hire.
#[must_use]
pub fn ghost_shift_ticks(task: Option<&CurrentTask>) -> u32 {
    match task {
        Some(t) if t.duration > 0.0 => {
            (t.duration.ceil() as u32).clamp(GHOST_MIN_SHIFT_TICKS, GHOST_MAX_SHIFT_TICKS)
        }
        _ => GHOST_SHIFT_TICKS,
    }
}

/// Raises a [`DigitalGhost`] for a pop who died mid-shift.
///
/// Must run after `handle_pop_death_system` (which emits
/// [`PopDied`]) and before `despawn_dead_entities_system` (the
/// corpse's `PopAction` / `Job` / [`CurrentTask`] are read here).
pub fn spawn_digital_ghost_system(
    mut commands: Commands,
    mut events: EventReader<PopDied>,
    dead_pops: Query<(
        Option<&GridPosition>,
        Option<&PopAction>,
        Option<&Job>,
        Option<&CurrentTask>,
    )>,
    mut stats: Option<ResMut<PosthumousStats>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let died: Vec<PopDied> = events.read().cloned().collect();
    for event in &died {
        let Ok((pos, action, job, task)) = dead_pops.get(event.entity) else {
            continue;
        };
        if !is_active_shift(action) {
            continue;
        }
        let Some(pos) = pos.copied() else {
            continue;
        };
        let ghost = DigitalGhost {
            deceased_name: event.name.clone(),
            deceased_entity: event.entity,
            action: action.map_or(ActionType::Idle, |a| a.current),
            job: job.copied(),
            task: task.map(GhostTask::from),
            remaining_ticks: ghost_shift_ticks(task),
            worked_ticks: 0,
            completed: false,
        };
        commands.spawn((ghost, pos));
        if let Some(stats) = stats.as_deref_mut() {
            stats.spawned += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "POSTHUMOUS SHIFT: {} died at work — their backup has clocked in to finish the shift.",
                event.name
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Works every ghost's inherited shift by one tick, haunts the
/// living nearby, and decays ghosts whose shift is done.
///
/// Completion credits an inherited [`CurrentTask`]'s output to
/// [`ColonyResources`], records the finished shift in
/// [`PosthumousStats`], chronicles it, and despawns the ghost the
/// same tick. Ghosts have no `Needs`/`Health`, so nothing here (or
/// anywhere) makes them eat, breathe, or rest.
#[allow(clippy::type_complexity)]
pub fn ghost_work_system(
    mut commands: Commands,
    mut ghosts: Query<(Entity, &mut DigitalGhost, &GridPosition)>,
    mut living: Query<(&GridPosition, &mut Morale), (With<Pop>, Without<Dead>)>,
    mut resources: Option<ResMut<ColonyResources>>,
    mut stats: Option<ResMut<PosthumousStats>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    // Haunting is computed from where ghosts stand this tick.
    let ghost_positions: Vec<GridPosition> = ghosts.iter().map(|(_, _, p)| *p).collect();

    // The living: refresh one Haunted Workplace modifier while a
    // ghost works nearby, strip it once they are clear.
    for (pos, mut morale) in &mut living {
        let haunted = ghost_positions
            .iter()
            .any(|g| (g.x - pos.x).abs().max((g.y - pos.y).abs()) <= HAUNTED_RADIUS);
        morale.modifiers.retain(|m| m.label != HAUNTED_LABEL);
        if haunted {
            morale.add_modifier(MoodModifier {
                label: HAUNTED_LABEL.to_string(),
                value: HAUNTED_PENALTY,
                duration: HAUNTED_DURATION,
            });
        }
    }

    for (entity, mut ghost, _) in &mut ghosts {
        if ghost.completed {
            continue;
        }
        ghost.worked_ticks += 1;
        ghost.remaining_ticks = ghost.remaining_ticks.saturating_sub(1);
        if ghost.remaining_ticks > 0 {
            continue;
        }
        ghost.completed = true;
        if let (Some(task), Some(res)) = (ghost.task, resources.as_deref_mut()) {
            res.add_resource(&task.output, task.efficiency);
        }
        if let Some(stats) = stats.as_deref_mut() {
            stats.completed += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "POSTHUMOUS SHIFT COMPLETE: {} finished their final shift and faded. The backup is gone for good.",
                ghost.deceased_name
            ),
            importance: EventImportance::Standard,
        });
        commands.entity(entity).despawn();
    }
}

/// Number of digital ghosts currently on shift (headless STATS).
#[must_use]
pub fn ghost_count(world: &mut World) -> u32 {
    let mut q = world.query::<&DigitalGhost>();
    q.iter(world).count() as u32
}

/// Panel lines for the headless `posthumous` command.
#[must_use]
pub fn posthumous_panel_lines(world: &mut World) -> Vec<String> {
    let (spawned, completed) = world
        .get_resource::<PosthumousStats>()
        .map_or((0, 0), |s| (s.spawned, s.completed));
    let mut lines = vec![format!(
        "Posthumous shifts: {spawned} raised, {completed} completed and decayed."
    )];
    let mut q = world.query::<(&DigitalGhost, &GridPosition)>();
    let rows: Vec<(u32, String)> = q
        .iter(world)
        .map(|(g, p)| {
            (
                g.remaining_ticks,
                format!(
                    "{} — {:?} at ({}, {}), {} tick(s) of the shift left",
                    g.deceased_name, g.action, p.x, p.y, g.remaining_ticks
                ),
            )
        })
        .collect();
    if rows.is_empty() {
        lines.push("No digital ghosts on shift. The dead are resting.".to_string());
    } else {
        for (_, row) in rows {
            lines.push(row);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::needs::Needs;
    use crate::layer1::utility_types::AssignmentType;

    fn world_with_ghost_systems() -> (World, Schedule) {
        let mut world = World::new();
        world.insert_resource(Events::<PopDied>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(PosthumousStats::default());
        world.insert_resource(ColonyResources::default());
        let mut schedule = Schedule::default();
        schedule.add_systems((spawn_digital_ghost_system, ghost_work_system));
        (world, schedule)
    }

    fn working_pop(world: &mut World, action: ActionType) -> Entity {
        let workplace = world.spawn_empty().id();
        world
            .spawn((
                Pop,
                crate::layer1::entities::pop::PopName("Uma".to_string()),
                GridPosition { x: 10, y: 10 },
                PopAction {
                    current: action,
                    current_utility: 1.0,
                    ticks_committed: 3,
                },
                Job {
                    workplace,
                    job_type: AssignmentType::FarmWorker,
                },
            ))
            .id()
    }

    fn die(world: &mut World, entity: Entity) {
        world.resource_mut::<Events<PopDied>>().send(PopDied {
            entity,
            name: "Uma".to_string(),
            tick: 42,
            reason: "Causes unknown".to_string(),
        });
    }

    // Spec RED test 1, adapted: death during an active shift spawns
    // a DigitalGhost carrying the same job.
    #[test]
    fn test_pop_death_spawns_digital_ghost_during_active_shift() {
        let (mut world, mut schedule) = world_with_ghost_systems();
        let pop = working_pop(&mut world, ActionType::Farm);
        die(&mut world, pop);
        schedule.run(&mut world);

        let mut q = world.query::<(&DigitalGhost, &GridPosition)>();
        let ghosts: Vec<_> = q.iter(&world).collect();
        assert_eq!(ghosts.len(), 1, "one ghost should rise");
        let (ghost, pos) = ghosts[0];
        assert_eq!(ghost.deceased_name, "Uma");
        assert_eq!(ghost.action, ActionType::Farm);
        assert!(ghost.job.is_some(), "ghost carries the dead pop's job");
        assert_eq!((pos.x, pos.y), (10, 10));
        assert_eq!(world.resource::<PosthumousStats>().spawned, 1);
    }

    #[test]
    fn test_pop_death_at_rest_spawns_no_ghost() {
        let (mut world, mut schedule) = world_with_ghost_systems();
        let pop = working_pop(&mut world, ActionType::SatisfyRest);
        die(&mut world, pop);
        schedule.run(&mut world);
        assert_eq!(ghost_count(&mut world), 0);
        assert_eq!(world.resource::<PosthumousStats>().spawned, 0);
    }

    // Spec RED test 2, adapted: the ghost completes its one shift,
    // credits the inherited task's output, then decays/despawns.
    #[test]
    fn test_digital_ghost_completes_current_shift_then_decays() {
        let (mut world, mut schedule) = world_with_ghost_systems();
        let ghost = world
            .spawn((
                DigitalGhost {
                    deceased_name: "Uma".to_string(),
                    deceased_entity: Entity::PLACEHOLDER,
                    action: ActionType::Work,
                    job: None,
                    task: Some(GhostTask {
                        task_id: 7,
                        output: ResourceType::Metal,
                        efficiency: 2.0,
                    }),
                    remaining_ticks: 3,
                    worked_ticks: 0,
                    completed: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let metal_before = world.resource::<ColonyResources>().metal;
        schedule.run(&mut world);
        schedule.run(&mut world);
        assert!(
            world.get::<DigitalGhost>(ghost).is_some(),
            "ghost must not decay before the shift is done"
        );
        schedule.run(&mut world);
        assert!(
            world.get::<DigitalGhost>(ghost).is_none(),
            "ghost despawns when its one shift completes"
        );
        assert_eq!(world.resource::<PosthumousStats>().completed, 1);
        let metal_after = world.resource::<ColonyResources>().metal;
        assert!(
            (metal_after - metal_before - 2.0).abs() < 0.001,
            "finished task credits its output (metal {metal_before} -> {metal_after})"
        );
    }

    // Spec RED test 3, adapted: nearby living pops receive the
    // "Haunted Workplace" mood modifier through real Morale.
    #[test]
    fn test_digital_ghost_reduces_colony_morale_when_working() {
        let (mut world, mut schedule) = world_with_ghost_systems();
        world.spawn((
            DigitalGhost {
                deceased_name: "Uma".to_string(),
                deceased_entity: Entity::PLACEHOLDER,
                action: ActionType::Farm,
                job: None,
                task: None,
                remaining_ticks: 50,
                worked_ticks: 0,
                completed: false,
            },
            GridPosition { x: 10, y: 10 },
        ));
        let near = world
            .spawn((Pop, GridPosition { x: 12, y: 11 }, Morale::default()))
            .id();
        let far = world
            .spawn((Pop, GridPosition { x: 40, y: 40 }, Morale::default()))
            .id();
        schedule.run(&mut world);
        schedule.run(&mut world);
        let near_morale = world.get::<Morale>(near).unwrap();
        let haunted: Vec<_> = near_morale
            .modifiers
            .iter()
            .filter(|m| m.label == HAUNTED_LABEL)
            .collect();
        assert_eq!(haunted.len(), 1, "refresh must never stack");
        assert!((haunted[0].value - HAUNTED_PENALTY).abs() < f32::EPSILON);
        let far_morale = world.get::<Morale>(far).unwrap();
        assert!(
            far_morale
                .modifiers
                .iter()
                .all(|m| m.label != HAUNTED_LABEL),
            "pops out of range are not haunted"
        );
    }

    // Spec RED test 4, adapted: job assignment rejects the ghost —
    // at the component level, and via the utility AI's
    // Without<DigitalGhost> exclusion pattern.
    #[test]
    fn test_digital_ghost_cannot_be_assigned_new_tasks() {
        let mut ghost = DigitalGhost {
            deceased_name: "Uma".to_string(),
            deceased_entity: Entity::PLACEHOLDER,
            action: ActionType::Farm,
            job: None,
            task: None,
            remaining_ticks: 50,
            worked_ticks: 0,
            completed: false,
        };
        assert!(ghost.try_assign_new_task(ActionType::Work).is_err());
        assert_eq!(
            ghost.action,
            ActionType::Farm,
            "inherited shift is untouched"
        );

        let mut world = World::new();
        world.spawn((Pop, DigitalGhost { ..ghost.clone() }));
        world.spawn(Pop);
        let mut q = world.query_filtered::<Entity, (With<Pop>, Without<DigitalGhost>)>();
        assert_eq!(
            q.iter(&world).count(),
            1,
            "only the living pop is assignable"
        );
    }

    #[test]
    fn test_ghost_consumes_no_food_oxygen_or_rest() {
        let (mut world, mut schedule) = world_with_ghost_systems();
        let pop = working_pop(&mut world, ActionType::Work);
        die(&mut world, pop);
        schedule.run(&mut world);
        let mut q = world.query_filtered::<Entity, With<DigitalGhost>>();
        let ghost = q.iter(&world).next().expect("ghost should rise");
        assert!(world.get::<Pop>(ghost).is_none(), "ghost is not a pop");
        assert!(
            world.get::<Needs>(ghost).is_none(),
            "ghost has no needs to drain"
        );
        assert!(
            world.get::<Health>(ghost).is_none(),
            "ghost cannot be hurt or healed"
        );
        assert!(world.get::<Morale>(ghost).is_none(), "ghost does not feel");
        for _ in 0..10 {
            schedule.run(&mut world);
        }
        assert!(
            world.get::<Needs>(ghost).is_none(),
            "working never grows needs"
        );
        let g = world.get::<DigitalGhost>(ghost).unwrap();
        assert_eq!(g.worked_ticks, 10);
    }

    #[test]
    fn test_ip_guard_public_strings_are_original() {
        let banned = [
            "black mirror",
            "san junipero",
            "uss callister",
            "white christmas",
            "be right back",
        ];
        let public = format!("{} digital ghost posthumous shift", HAUNTED_LABEL).to_lowercase();
        for term in banned {
            assert!(
                !public.contains(term),
                "public string lifts IP term: {term}"
            );
        }
    }
}
