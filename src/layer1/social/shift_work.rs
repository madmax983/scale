//! Spec 359: Shift Work — the consequence layer for Spec 083's
//! `ShiftSchedule`.
//!
//! # What already existed (do not duplicate)
//!
//! Spec 083 is live: buildings carry a
//! [`ShiftSchedule`](crate::layer1::building::ShiftSchedule)
//! (`day_shift` / `night_shift`), and the utility AI only offers a
//! building as a work candidate while its schedule is active for the
//! current [`TimeOfDay`](crate::layer1::day_night::TimeOfDay) (see
//! `is_active_shift` in the utility-AI buffer population). Scheduling
//! was never the missing piece — *consequences* were. A pop could work
//! a night-active farm all night at zero cost. This module is that
//! cost.
//!
//! # The seam decision (deliverable 1 — read before changing)
//!
//! A pop is **night-working** on a tick iff ALL of these hold:
//!
//! 1. `DayNightCycle.time_of_day == Night`;
//! 2. the pop has a [`Job`](crate::layer1::pop::Job) whose workplace
//!    building's `ShiftSchedule` is night-active
//!    (`night_shift && is_active(Night)`);
//! 3. the pop's current [`PopAction`](crate::layer1::utility_types::PopAction)
//!    is a work action (`Farm` / `Research` / `Admin` / `Refine` /
//!    `Work`) — which by construction excludes `SatisfyRest`.
//!
//! Why not `Job` alone? `Job` is assigned on arrival at a workplace
//! (see `assign_pop` in the execution arrival system) and is *never
//! cleared* when the pop goes to bed, eats, or wanders off. A
//! Job-only seam would penalise a pop for sleeping in a bed because
//! the farm they worked yesterday runs nights — exactly the
//! false positive spec §5 forbids ("sleeping pops are never
//! penalised for their building's schedule").
//!
//! Why not action alone? `PopAction` carries no target entity (the
//! target lives in the transient `StartPlan`), so an action cannot
//! say *which* building's schedule it serves.
//!
//! The combined seam is true in live play: the utility AI only
//! *offers* night work at night-active buildings
//! (`shift_integration_tests` prove a pop picks `Farm` / `Refine` /
//! `Research` at Night only when the building's `night_shift` is on),
//! and a pop whose `PopAction` is a work action at Night with a
//! night-active workplace is, that tick, doing that work instead of
//! sleeping.
//!
//! # Consequences (no existing constants retuned)
//!
//! While night-working, an unadapted pop suffers:
//!
//! * a refreshed-never-stacking `"Night Shift"` mood modifier on
//!   [`Morale`](crate::layer1::social::morale::Morale) — the Spec
//!   353 / Spec 1387 bridge pattern, one modifier, refreshed every
//!   tick, removed the first tick the pop is off shift; and
//! * accelerated rest drain (the primary channel): an extra
//!   [`EXTRA_REST_DRAIN_PER_TICK`] on top of the base needs decay and
//!   the circadian night drain, neither of which is touched. Rest is
//!   the honest currency of night work — it is what the Grind's
//!   RestDenial channel reads (`needs.rest < 0.30`), so night work
//!   feeds Spec 1387 through its existing seam with no Grind
//!   constants changed.
//!
//! [`Trait::NightOwl`](crate::layer1::traits::Trait::NightOwl) pops
//! are exempt — they are the adapted-by-birth analog of the spec's
//! sketch `Nocturnal` trait (which does not exist in this codebase).
//!
//! # Adaptation (§5 REFACTOR)
//!
//! Sustained night work builds [`NightShiftAdaptation`]: +0.001 per
//! night-working tick. A night is ~38 ticks at the default 250
//! ticks/day, so full adaptation takes ~26 nights of real night
//! work — the spec's "~30 nights". Adaptation scales both penalty
//! channels toward zero. It decays at 0.00005/tick while not
//! night-working (20x slower than it accrues), so a pop rotated off
//! nights keeps most of their adaptation for a season and loses it
//! over ~80 days away.

use crate::layer1::building::{Building, ShiftSchedule};
use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::pop::{Job, Pop, PopName};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::traits::{Trait, Traits};
use crate::layer1::utility_types::{ActionType, PopAction};
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Label of the single persistent night-shift mood modifier.
pub const NIGHT_SHIFT_LABEL: &str = "Night Shift";
/// Unadapted mood penalty. In line with existing one-shot mood
/// modifiers (-0.05..-0.30 across the morale systems) and well below
/// the Grind bridge's saturated -0.7: night work is a grind, not a
/// catastrophe.
pub const NIGHT_SHIFT_BASE_PENALTY: f32 = -0.15;
/// Modifier duration, refreshed every night-working tick (the
/// refresh-never-stack pattern; matches the Grind bridge's 100).
pub const NIGHT_SHIFT_DURATION: u32 = 100;
/// Extra rest drain per night-working tick for an unadapted pop, on
/// top of base rest decay (0.001) and the circadian night drain
/// (0.0005): a night worker burns rest at ~2x the normal night rate.
pub const EXTRA_REST_DRAIN_PER_TICK: f32 = 0.0015;
/// Adaptation gained per night-working tick (~26 nights to full).
pub const ADAPT_PER_TICK: f32 = 0.001;
/// Adaptation lost per tick while not night-working (20x slower
/// than accrual).
pub const ADAPT_DECAY_PER_TICK: f32 = 0.00005;

/// Per-pop night-shift adaptation tracker (Spec 359 §5).
///
/// Every pop spawned through `PopBundle` carries one; the ensure
/// system below inserts one for pops spawned any other way, the
/// same total-coverage discipline as Spec 1387's `Dissatisfaction`.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NightShiftAdaptation {
    /// 0.0 = never worked a night; 1.0 = fully adapted (penalty 0).
    pub level: f32,
    /// Lifetime night-working ticks (ledger / panel evidence).
    pub night_ticks: u64,
}

impl NightShiftAdaptation {
    /// Effective adaptation for a pop with these traits: NightOwl
    /// pops are fully adapted from birth.
    #[must_use]
    pub fn effective_level(&self, traits: Option<&Traits>) -> f32 {
        if traits.is_some_and(|t| t.has(Trait::NightOwl)) {
            1.0
        } else {
            self.level.clamp(0.0, 1.0)
        }
    }

    /// Current mood penalty value for this adaptation level
    /// (0.0 when fully adapted).
    #[must_use]
    pub fn penalty(&self, traits: Option<&Traits>) -> f32 {
        NIGHT_SHIFT_BASE_PENALTY * (1.0 - self.effective_level(traits))
    }
}

/// Colony night-work census, refreshed every tick by
/// [`night_shift_effects_system`].
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct NightShiftStats {
    /// Pops night-working on the most recent tick.
    pub night_workers: u32,
    /// Cumulative night-working pop-ticks.
    pub total_night_worker_ticks: u64,
}

/// Work actions that count as "on shift" for the seam. Deliberately
/// excludes hauling / repair / chores: those are not tied to the
/// pop's workplace building, so crediting them to the workplace's
/// schedule would be a false positive.
#[must_use]
pub const fn is_work_action(action: ActionType) -> bool {
    matches!(
        action,
        ActionType::Farm
            | ActionType::Research
            | ActionType::Admin
            | ActionType::Refine
            | ActionType::Work
    )
}

/// The seam predicate (see module docs). Pure and unit-testable.
#[must_use]
pub fn is_night_working(
    schedule: Option<ShiftSchedule>,
    action: Option<&PopAction>,
    time: TimeOfDay,
) -> bool {
    if time != TimeOfDay::Night {
        return false;
    }
    let Some(schedule) = schedule else {
        return false;
    };
    if !schedule.night_shift || !schedule.is_active(time) {
        return false;
    }
    let Some(action) = action else {
        return false;
    };
    if action.current == ActionType::SatisfyRest {
        return false;
    }
    is_work_action(action.current)
}

fn remove_night_shift_modifier(morale: &mut Morale) {
    morale.modifiers.retain(|m| m.label != NIGHT_SHIFT_LABEL);
}

fn refresh_night_shift_modifier(morale: &mut Morale, factor: f32) {
    if factor <= 0.01 {
        remove_night_shift_modifier(morale);
        return;
    }
    let value = NIGHT_SHIFT_BASE_PENALTY * factor;
    if let Some(existing) = morale
        .modifiers
        .iter_mut()
        .find(|m| m.label == NIGHT_SHIFT_LABEL)
    {
        existing.value = value;
        existing.duration = NIGHT_SHIFT_DURATION;
    } else {
        morale.add_modifier(MoodModifier {
            label: NIGHT_SHIFT_LABEL.to_string(),
            value,
            duration: NIGHT_SHIFT_DURATION,
        });
    }
}

/// Inserts a [`NightShiftAdaptation`] tracker on any pop missing
/// one (non-bundle spawn sites). NightOwl pops start fully adapted.
#[allow(clippy::type_complexity)]
pub fn ensure_night_shift_tracker_system(
    mut commands: Commands,
    query: Query<(Entity, Option<&Traits>), (With<Pop>, Without<NightShiftAdaptation>)>,
) {
    for (entity, traits) in &query {
        let mut tracker = NightShiftAdaptation::default();
        if traits.is_some_and(|t| t.has(Trait::NightOwl)) {
            tracker.level = 1.0;
        }
        commands.entity(entity).insert(tracker);
    }
}

/// The consequence layer: adaptation accrual/decay, the refreshed
/// "Night Shift" mood modifier, and the extra rest drain. Runs in
/// the Economy set before the morale cache, mirroring the Grind
/// bridge, so a night worked this tick costs morale this tick.
#[allow(clippy::type_complexity)]
pub fn night_shift_effects_system(
    cycle: Option<Res<DayNightCycle>>,
    stats: Option<ResMut<NightShiftStats>>,
    buildings: Query<&ShiftSchedule>,
    mut pops: Query<
        (
            &mut NightShiftAdaptation,
            Option<&Job>,
            Option<&PopAction>,
            Option<&Traits>,
            Option<&mut Needs>,
            Option<&mut Morale>,
        ),
        With<Pop>,
    >,
) {
    let time = cycle.map_or(TimeOfDay::Day, |c| c.time_of_day);
    let mut count: u32 = 0;
    for (mut adapt, job, action, traits, needs, morale) in &mut pops {
        let night_owl = traits.is_some_and(|t| t.has(Trait::NightOwl));
        let schedule = job.and_then(|j| buildings.get(j.workplace).ok().copied());
        if is_night_working(schedule, action, time) {
            count += 1;
            adapt.night_ticks = adapt.night_ticks.saturating_add(1);
            if night_owl {
                adapt.level = 1.0;
                if let Some(mut morale) = morale {
                    remove_night_shift_modifier(&mut morale);
                }
                continue;
            }
            adapt.level = (adapt.level + ADAPT_PER_TICK).clamp(0.0, 1.0);
            let factor = 1.0 - adapt.level;
            if let Some(mut needs) = needs {
                needs.rest = (needs.rest - EXTRA_REST_DRAIN_PER_TICK * factor).max(0.0);
            }
            if let Some(mut morale) = morale {
                refresh_night_shift_modifier(&mut morale, factor);
            }
        } else {
            adapt.level = (adapt.level - ADAPT_DECAY_PER_TICK).max(0.0);
            if let Some(mut morale) = morale {
                remove_night_shift_modifier(&mut morale);
            }
        }
    }
    if let Some(mut stats) = stats {
        stats.night_workers = count;
        stats.total_night_worker_ticks += u64::from(count);
    }
}

/// Recomputes the current night-worker count from live state (the
/// same predicate the per-tick system uses). Headless STATS reads
/// this so the counter is honest even between system runs.
#[must_use]
pub fn night_worker_count(world: &mut World) -> u32 {
    let time = world
        .get_resource::<DayNightCycle>()
        .map_or(TimeOfDay::Day, |c| c.time_of_day);
    let schedules: HashMap<Entity, ShiftSchedule> = {
        let mut q = world.query::<(Entity, &ShiftSchedule)>();
        q.iter(world).map(|(e, s)| (e, *s)).collect()
    };
    let mut q = world.query_filtered::<(Option<&Job>, Option<&PopAction>), With<Pop>>();
    q.iter(world)
        .filter(|(job, action)| {
            let schedule = job.and_then(|j| schedules.get(&j.workplace).copied());
            is_night_working(schedule, *action, time)
        })
        .count() as u32
}

/// Panel lines for the headless `shift` command: every building's
/// schedule, then the current night workers.
#[must_use]
pub fn shift_panel_lines(world: &mut World) -> Vec<String> {
    let time = world
        .get_resource::<DayNightCycle>()
        .map_or(TimeOfDay::Day, |c| c.time_of_day);
    let mut lines: Vec<String> = Vec::new();
    {
        let mut q = world.query::<(Entity, &Building, Option<&GridPosition>, Option<&ShiftSchedule>)>();
        let mut rows: Vec<(u32, String)> = q
            .iter(world)
            .map(|(e, b, pos, sched)| {
                let (day, night) = sched.map_or((true, false), |s| (s.day_shift, s.night_shift));
                let (x, y) = pos.map_or((0, 0), |p| (p.x, p.y));
                (
                    e.index(),
                    format!(
                        "#{} {:?} at ({}, {}) day={} night={}",
                        e.index(),
                        b.building_type,
                        x,
                        y,
                        day,
                        night
                    ),
                )
            })
            .collect();
        rows.sort_by_key(|(idx, _)| *idx);
        lines.extend(rows.into_iter().map(|(_, line)| line));
    }
    lines.push(format!("-- night workers (time of day: {time:?}) --"));
    let schedules: HashMap<Entity, ShiftSchedule> = {
        let mut q = world.query::<(Entity, &ShiftSchedule)>();
        q.iter(world).map(|(e, s)| (e, *s)).collect()
    };
    {
        let mut q = world.query_filtered::<(
            Entity,
            Option<&PopName>,
            Option<&Job>,
            Option<&PopAction>,
            Option<&NightShiftAdaptation>,
        ), With<Pop>>();
        let mut workers: Vec<String> = q
            .iter(world)
            .filter(|(_, _, job, action, _)| {
                let schedule = job.and_then(|j| schedules.get(&j.workplace).copied());
                is_night_working(schedule, *action, time)
            })
            .map(|(e, name, job, action, adapt)| {
                let name = name.map_or_else(|| "?".to_string(), |n| n.0.clone());
                let job_label = job.map_or_else(|| "?".to_string(), |j| format!("{:?}", j.job_type));
                let action_label = action.map_or_else(|| "?".to_string(), |a| format!("{:?}", a.current));
                let level = adapt.map_or(0.0, |a| a.level);
                format!(
                    "#{} {} job={} action={} adaptation={:.2}",
                    e.index(),
                    name,
                    job_label,
                    action_label,
                    level
                )
            })
            .collect();
        if workers.is_empty() {
            lines.push("(none right now)".to_string());
        } else {
            lines.append(&mut workers);
        }
    }
    lines
}

/// Inspect the schedule of the building at (x, y), if any.
#[must_use]
pub fn building_schedule_at(world: &mut World, x: i32, y: i32) -> Option<String> {
    let mut q = world.query::<(Entity, &Building, &GridPosition, Option<&ShiftSchedule>)>();
    for (e, b, pos, sched) in q.iter(world) {
        if pos.x == x && pos.y == y {
            let (day, night) = sched.map_or((true, false), |s| (s.day_shift, s.night_shift));
            return Some(format!(
                "#{} {:?} at ({}, {}): day={} night={}",
                e.index(),
                b.building_type,
                x,
                y,
                day,
                night
            ));
        }
    }
    None
}

/// Sets the schedule of the building at (x, y). Modes: `day`,
/// `night`, `both`, `off`, `toggle` (flips night_shift). Returns a
/// human-readable result line, or None if no building stands there.
#[must_use]
pub fn set_building_schedule_at(
    world: &mut World,
    x: i32,
    y: i32,
    mode: &str,
) -> Option<String> {
    let found: Option<(Entity, ShiftSchedule)> = {
        let mut q = world.query::<(Entity, &GridPosition, Option<&ShiftSchedule>)>();
        let mut out = None;
        for (e, pos, sched) in q.iter(world) {
            if pos.x == x && pos.y == y {
                out = Some((e, sched.copied().unwrap_or_default()));
                break;
            }
        }
        out
    };
    let (entity, current) = found?;
    let next = match mode.to_lowercase().as_str() {
        "day" => ShiftSchedule { day_shift: true, night_shift: false },
        "night" => ShiftSchedule { day_shift: false, night_shift: true },
        "both" => ShiftSchedule { day_shift: true, night_shift: true },
        "off" => ShiftSchedule { day_shift: false, night_shift: false },
        "toggle" => ShiftSchedule { night_shift: !current.night_shift, ..current },
        _ => return Some(format!("unknown shift mode '{mode}' (use day|night|both|off|toggle)")),
    };
    world.entity_mut(entity).insert(next);
    Some(format!(
        "building #{} at ({}, {}) schedule set: day={} night={}",
        entity.index(),
        x,
        y,
        next.day_shift,
        next.night_shift
    ))
}

/// Sets the schedule of EVERY building whose type name matches
/// `type_name` (case-insensitive, e.g. "Farm"). Modes as in
/// [`set_building_schedule_at`]. Returns how many buildings were
/// updated (0 = no such building, or unknown mode). Exists because
/// headless worlds are re-rolled per launch: a scripted playtest
/// cannot hard-code coordinates from a previous run.
#[must_use]
pub fn set_buildings_of_type(world: &mut World, type_name: &str, mode: &str) -> usize {
    let mode = mode.to_lowercase();
    if !matches!(mode.as_str(), "day" | "night" | "both" | "off" | "toggle") {
        return 0;
    }
    let targets: Vec<(Entity, ShiftSchedule)> = {
        let mut q = world.query::<(Entity, &Building, Option<&ShiftSchedule>)>();
        q.iter(world)
            .filter(|(_, b, _)| format!("{:?}", b.building_type).eq_ignore_ascii_case(type_name))
            .map(|(e, _, s)| (e, s.copied().unwrap_or_default()))
            .collect()
    };
    let mut count = 0;
    for (entity, current) in targets {
        let next = match mode.as_str() {
            "day" => ShiftSchedule {
                day_shift: true,
                night_shift: false,
            },
            "night" => ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
            "both" => ShiftSchedule {
                day_shift: true,
                night_shift: true,
            },
            "off" => ShiftSchedule {
                day_shift: false,
                night_shift: false,
            },
            _ => ShiftSchedule {
                night_shift: !current.night_shift,
                ..current
            },
        };
        world.entity_mut(entity).insert(next);
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::BuildingType;
    use crate::layer1::utility_types::AssignmentType;

    fn world_at(time: TimeOfDay) -> World {
        let mut world = World::new();
        world.insert_resource(DayNightCycle {
            time_of_day: time,
            ..Default::default()
        });
        world.insert_resource(NightShiftStats::default());
        world
    }

    fn spawn_building(world: &mut World, schedule: ShiftSchedule) -> Entity {
        world
            .spawn((
                Building {
                    building_type: BuildingType::Farm,
                },
                schedule,
            ))
            .id()
    }

    fn spawn_worker(
        world: &mut World,
        workplace: Entity,
        action: ActionType,
        traits: Option<Traits>,
    ) -> Entity {
        let mut builder = world.spawn((
            Pop,
            Job {
                workplace,
                job_type: AssignmentType::FarmWorker,
            },
            PopAction {
                current: action,
                ..Default::default()
            },
            Needs::default(),
            Morale::default(),
            NightShiftAdaptation::default(),
        ));
        if let Some(t) = traits {
            builder.insert(t);
        }
        builder.id()
    }

    fn run(world: &mut World, ticks: usize) {
        let mut schedule = Schedule::default();
        schedule.add_systems(night_shift_effects_system);
        for _ in 0..ticks {
            schedule.run(world);
        }
    }

    fn night_shift_modifiers(world: &mut World, pop: Entity) -> Vec<f32> {
        world
            .get::<Morale>(pop)
            .unwrap()
            .modifiers
            .iter()
            .filter(|m| m.label == NIGHT_SHIFT_LABEL)
            .map(|m| m.value)
            .collect()
    }

    // Spec §3 test 1, adapted: a building with the default (083)
    // schedule is day-only, so its worker gains nothing at night.
    #[test]
    fn test_default_day_shift_has_no_night_penalty() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(&mut world, ShiftSchedule::default());
        assert!(!ShiftSchedule::default().night_shift);
        let pop = spawn_worker(&mut world, building, ActionType::Farm, None);
        run(&mut world, 10);
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
        assert_eq!(world.get::<Needs>(pop).unwrap().rest, 0.8);
        assert_eq!(world.resource::<NightShiftStats>().night_workers, 0);
    }

    // Spec §3 test 2, adapted: the "stress" channel is the real
    // morale modifier + accelerated rest drain (Needs has no stress
    // field in this codebase).
    #[test]
    fn test_night_worker_gains_debuff() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let pop = spawn_worker(&mut world, building, ActionType::Farm, None);
        run(&mut world, 1);
        let mods = night_shift_modifiers(&mut world, pop);
        assert_eq!(mods.len(), 1, "exactly one Night Shift modifier");
        assert!(
            (mods[0] - NIGHT_SHIFT_BASE_PENALTY).abs() < 0.01,
            "unadapted penalty ~{}, got {}",
            NIGHT_SHIFT_BASE_PENALTY,
            mods[0]
        );
        let rest = world.get::<Needs>(pop).unwrap().rest;
        assert!(
            (rest - (0.8 - EXTRA_REST_DRAIN_PER_TICK)).abs() < 0.001,
            "extra rest drain applied, rest={rest}"
        );
        assert_eq!(world.resource::<NightShiftStats>().night_workers, 1);
        assert!(world.get::<NightShiftAdaptation>(pop).unwrap().level > 0.0);
    }

    // Spec §3 test 3, adapted: Nocturnal -> Trait::NightOwl.
    #[test]
    fn test_night_owl_ignores_night_shift_debuff() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let mut traits = Traits::default();
        traits.add(Trait::NightOwl);
        let pop = spawn_worker(&mut world, building, ActionType::Farm, Some(traits));
        run(&mut world, 10);
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
        assert_eq!(world.get::<Needs>(pop).unwrap().rest, 0.8);
        assert_eq!(
            world.get::<NightShiftAdaptation>(pop).unwrap().level,
            1.0,
            "NightOwl counts as fully adapted"
        );
    }

    #[test]
    fn test_sleeping_pop_never_penalized() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let pop = spawn_worker(&mut world, building, ActionType::SatisfyRest, None);
        run(&mut world, 10);
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
        assert_eq!(world.get::<Needs>(pop).unwrap().rest, 0.8);
        assert_eq!(world.resource::<NightShiftStats>().night_workers, 0);
    }

    #[test]
    fn test_no_penalty_during_day() {
        let mut world = world_at(TimeOfDay::Day);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let pop = spawn_worker(&mut world, building, ActionType::Farm, None);
        run(&mut world, 10);
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
        assert_eq!(world.get::<Needs>(pop).unwrap().rest, 0.8);
    }

    #[test]
    fn test_modifier_never_stacks() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let pop = spawn_worker(&mut world, building, ActionType::Farm, None);
        run(&mut world, 100);
        assert_eq!(night_shift_modifiers(&mut world, pop).len(), 1);
    }

    #[test]
    fn test_adaptation_grows_scales_penalty_and_decays() {
        let mut world = world_at(TimeOfDay::Night);
        let building = spawn_building(
            &mut world,
            ShiftSchedule {
                day_shift: false,
                night_shift: true,
            },
        );
        let pop = spawn_worker(&mut world, building, ActionType::Farm, None);
        // Keep rest topped up so the assertion is about adaptation,
        // not about the pop collapsing.
        run(&mut world, 500);
        let adapt = *world.get::<NightShiftAdaptation>(pop).unwrap();
        assert!(
            (adapt.level - 0.5).abs() < 0.02,
            "500 night ticks -> ~0.5 adaptation, got {}",
            adapt.level
        );
        assert_eq!(adapt.night_ticks, 500);
        let mods = night_shift_modifiers(&mut world, pop);
        assert_eq!(mods.len(), 1);
        assert!(
            (mods[0] - NIGHT_SHIFT_BASE_PENALTY * 0.5).abs() < 0.01,
            "penalty halved at half adaptation, got {}",
            mods[0]
        );
        // Full adaptation: penalty gone.
        run(&mut world, 600);
        assert_eq!(world.get::<NightShiftAdaptation>(pop).unwrap().level, 1.0);
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
        // Off shift (daytime): modifier stays gone, adaptation decays
        // slowly (0.00005/tick -> 0.05 over 1000 ticks).
        world.resource_mut::<DayNightCycle>().time_of_day = TimeOfDay::Day;
        run(&mut world, 1000);
        let level = world.get::<NightShiftAdaptation>(pop).unwrap().level;
        assert!(
            (level - 0.95).abs() < 0.02,
            "adaptation decays slowly off shift, got {level}"
        );
        assert!(night_shift_modifiers(&mut world, pop).is_empty());
    }

    #[test]
    fn test_ensure_system_and_bundle_coverage() {
        // Ensure: a bare Pop (non-bundle spawn) gains a tracker.
        let mut world = world_at(TimeOfDay::Day);
        let bare = world.spawn(Pop).id();
        let mut schedule = Schedule::default();
        schedule.add_systems(ensure_night_shift_tracker_system);
        schedule.run(&mut world);
        assert!(world.get::<NightShiftAdaptation>(bare).is_some());
        // Bundle: every PopBundle::random pop carries one (the
        // total-coverage spawn path, like Dissatisfaction).
        let mut world2 = World::new();
        let mut rng = rand::thread_rng();
        let pop = world2
            .spawn(crate::layer1::pop::PopBundle::random(0, 0, &mut rng))
            .id();
        assert!(world2.get::<NightShiftAdaptation>(pop).is_some());
    }

    #[test]
    fn test_ip_guard_original_strings_only() {
        let banned = [
            "nocturnal", "Nocturnal", "cyberpunk", "Cyberpunk", "dune",
            "Dune", "weyland", "Weyland", "night city", "Night City",
        ];
        for text in [NIGHT_SHIFT_LABEL, "Night Shift"] {
            for b in banned {
                assert!(
                    !text.contains(b),
                    "player-facing copy must stay original: '{text}' contains '{b}'"
                );
            }
        }
    }

    /// LIVE test: the full production schedule. A starter pop is
    /// pinned to a night-only building (Job + Farm action, needs
    /// healthy, clock held in the night segment — the graffiti-357 /
    /// Grind live-test pinning pattern; the systems under audit are
    /// unmodified production code) and must gain the Night Shift
    /// modifier through the real Economy-set registration.
    #[test]
    fn live_night_worker_gains_debuff_in_full_sim() {
        use crate::layer1::map::GridPosition;
        use crate::shared::time::SimulationTime;

        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        let pops: Vec<Entity> = {
            let mut q = world.query_filtered::<Entity, With<Pop>>();
            q.iter(&world).collect()
        };
        assert!(!pops.is_empty(), "setup world must have pops");
        let target = pops[0];
        // A real building, flipped to night-only.
        let building = {
            let mut q = world.query_filtered::<Entity, With<Building>>();
            q.iter(&world).next().expect("setup world must have buildings")
        };
        world.entity_mut(building).insert(ShiftSchedule {
            day_shift: false,
            night_shift: true,
        });
        // The target must not be a NightOwl for this audit.
        if let Some(mut traits) = world.get_mut::<Traits>(target) {
            traits.0.clear();
        }
        let _ = world.get::<GridPosition>(target);
        // A tidally locked world has no night at all - the cycle
        // system forces permanent Day. Strip that one quirk so the
        // night segment exists (worldgen variance, not behavior
        // under test).
        if let Some(mut traits) =
            world.get_resource_mut::<crate::layer1::quirks::PlanetaryTraits>()
        {
            traits
                .0
                .retain(|q| *q != crate::layer1::quirks::PlanetaryTrait::TidallyLocked);
        }
        // Hold the clock in the night segment. ticks_per_day is NOT
        // assumed and NOT read once: planetary quirks rescale it
        // during the first live ticks (a stale pre-read landed the
        // clock back in Day in ~1 of 6 setup worlds - caught by the
        // assertion diagnostics), so it is re-read every iteration.
        for _ in 0..30 {
            let ticks_per_day = world.resource::<DayNightCycle>().ticks_per_day.max(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let night_tick = ((ticks_per_day as f32 * 0.9) as u64).max(1);
            world.resource_mut::<SimulationTime>().tick = night_tick;
            world.resource_mut::<DayNightCycle>().time_of_day = TimeOfDay::Night;
            // Pin EVERY starter pop as a worker at the night building
            // (single-pop pinning proved worldgen-fragile: pop order
            // and the first-building pick vary between setup worlds).
            for &pop in &pops {
                if let Some(mut traits) = world.get_mut::<Traits>(pop) {
                    traits.0.clear();
                }
                world.entity_mut(pop).insert((
                    Job {
                        workplace: building,
                        job_type: AssignmentType::FarmWorker,
                    },
                    PopAction {
                        current: ActionType::Farm,
                        current_utility: 999.0,
                        ticks_committed: 0,
                    },
                    Needs {
                        hunger: 1.0,
                        rest: 1.0,
                        leisure: 1.0,
                        hygiene: 1.0,
                    },
                ));
            }
            crate::simulation::run_simulation_tick(&mut world);
        }

        // At least one pinned pop must carry the debuff, have accrued
        // adaptation exposure, and the census must have counted the
        // crew - all through the unmodified production schedule.
        let mut modified = 0usize;
        let mut exposed = 0usize;
        for &pop in &pops {
            if let Some(morale) = world.get::<Morale>(pop) {
                if morale.modifiers.iter().any(|m| m.label == NIGHT_SHIFT_LABEL) {
                    modified += 1;
                }
            }
            if let Some(adapt) = world.get::<NightShiftAdaptation>(pop) {
                if adapt.night_ticks >= 10 && adapt.level > 0.0 {
                    exposed += 1;
                }
            }
        }
        assert!(
            modified >= 1,
            "live schedule must apply the Night Shift modifier to night-working pops \
             (modified={modified} exposed={exposed} pops={} building_alive={} sched={:?} cycle={:?})",
            pops.len(),
            world.get::<Building>(building).is_some(),
            world.get::<ShiftSchedule>(building),
            world.resource::<DayNightCycle>().time_of_day,
        );
        assert!(
            exposed >= 1,
            "live schedule must accrue adaptation exposure on night-working pops \
             (modified={modified} exposed={exposed})"
        );
        let stats = world.resource::<NightShiftStats>();
        assert!(
            stats.night_workers >= 1,
            "STATS census resource must count the night worker"
        );
        assert!(stats.total_night_worker_ticks > 0);
    }
}
