//! Prison Labor (Specs 118 + 399): penal zones, revolt risk, breakouts.
//!
//! This module is the Spec 118 penal-labor implementation, extended by
//! Spec 399 ("Prison Labor"). There is exactly one penal implementation
//! in the tree — this one; Spec 399 overlays it rather than duplicating.
//!
//! Inmates (Spec 072 justice system, [`Inmate`]) standing in a
//! [`ZoneType::Penal`] tile are assigned [`PenalLabor`]: they work
//! faster (the bonus is fed into the real work pipeline through
//! [`WorkSpeedBuff`], the same seam gastronomy and martyrdom use), the
//! utility AI ignores their non-survival needs while they work, and
//! they accumulate hidden [`RevoltRisk`]. A warden or sheriff working
//! nearby damps the accumulation. At the threshold the inmate revolts:
//! [`Inmate`] is removed, [`Rioter`] + [`Wanted`] are inserted (so the
//! warden arrest seam treats them as a hostile criminal again), their
//! action is forced to vandalism, a [`PrisonBreakEvent`] fires for the
//! blacksite seam, and a Major chronicle entry records the breakout.

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::gastronomy::WorkSpeedBuff;
use crate::layer1::law::justice::{Inmate, Wanted};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Job, Pop, PopName};
use crate::layer1::social::blacksite::PrisonBreakEvent;
use crate::layer1::utility_types::{ActionType, AssignmentType, PopAction};
use crate::layer1::zone::{ZoneGrid, ZoneType};
use bevy_ecs::prelude::*;

/// Revolt risk gained per tick by an unguarded penal laborer.
pub const BASE_REVOLT_RISK_PER_TICK: f32 = 0.1;
/// Revolt risk at which a penal laborer revolts.
pub const REVOLT_RISK_THRESHOLD: f32 = 100.0;
/// Chebyshev radius within which a warden/sheriff damps revolt risk.
pub const WARDEN_GUARD_RADIUS: i32 = 8;
/// Risk accumulation factor while a guard is within [`WARDEN_GUARD_RADIUS`].
pub const GUARDED_RISK_FACTOR: f32 = 0.2;
/// Ticks a synced penal [`WorkSpeedBuff`] lasts before the decay system
/// would drop it; the sync system refreshes it every tick.
pub const PENAL_WORK_SPEED_BUFF_TICKS: u32 = 3;
/// Sentence given to pops detained through the headless debug command.
pub const DEBUG_SENTENCE_TICKS: u32 = 100_000;

/// Chronicle text written when a penal laborer revolts.
pub const REVOLT_CHRONICLE_TEXT: &str =
    "PRISON REVOLT: An inmate in the penal zone has revolted and broken out.";

/// Component indicating an Inmate is currently performing forced labor.
///
/// This component is added by [`evaluate_penal_work_system`] when an Inmate is in a [`ZoneType::Penal`],
/// and removed by [`cleanup_penal_work_system`] when they leave.
#[derive(Component, Clone, Copy, Debug)]
pub struct PenalLabor {
    /// Work efficiency bonus as a fraction (0.2 = +20% speed).
    pub efficiency_bonus: f32,
}

impl Default for PenalLabor {
    fn default() -> Self {
        Self {
            efficiency_bonus: 0.2, // +20% speed
        }
    }
}

impl PenalLabor {
    /// The work-speed multiplier this laborer actually works at
    /// (1.0 = normal pace). Consumed via [`WorkSpeedBuff`] by
    /// `sync_penal_work_speed_system` -> `work_execution_system`.
    #[must_use]
    pub fn work_speed_multiplier(&self) -> f32 {
        1.0 + self.efficiency_bonus
    }
}

/// Tracks the risk of an inmate rebelling.
///
/// Accumulates over time while working in a penal zone.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct RevoltRisk {
    /// Current risk level.
    pub current: f32,
    /// Threshold at which a jailbreak occurs.
    pub threshold: f32,
}

/// Marker for an escaped inmate who has revolted (Spec 399).
///
/// Rioters also carry [`Wanted`], so the existing warden arrest seam
/// can bring them back in; their `PopAction` is forced to vandalism
/// at the moment of revolt so the breakout is immediately violent.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Rioter;

/// Colony-wide ledger of penal breakouts (for STATS / panels).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PenalLedger {
    /// Total revolts since colony founding.
    pub revolts: u32,
}

/// Risk gained this tick, damped when a guard is nearby.
#[must_use]
pub fn revolt_risk_increment(guarded: bool) -> f32 {
    if guarded {
        BASE_REVOLT_RISK_PER_TICK * GUARDED_RISK_FACTOR
    } else {
        BASE_REVOLT_RISK_PER_TICK
    }
}

fn chebyshev(a: GridPosition, b: GridPosition) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// A pop counts as a guard for risk damping when it is working the
/// warden action (the justice system arrest role) or holds the
/// sheriff assignment (Spec 399 section 5: Guard/Warden presence).
fn is_guard(action: Option<&PopAction>, job: Option<&Job>) -> bool {
    action.is_some_and(|a| a.current == ActionType::Warden)
        || job.is_some_and(|j| j.job_type == AssignmentType::Sheriff)
}

/// Checks if Inmates are in a Penal Zone and assigns `PenalLabor` status.
#[allow(clippy::type_complexity)]
pub fn evaluate_penal_work_system(
    mut commands: Commands,
    zone_grid: Res<ZoneGrid>,
    query: Query<(Entity, &GridPosition), (With<Inmate>, Without<PenalLabor>)>,
) {
    for (entity, pos) in &query {
        if zone_grid.get(pos.x, pos.y) == ZoneType::Penal {
            commands.entity(entity).insert((
                PenalLabor::default(),
                RevoltRisk {
                    current: 0.0,
                    threshold: REVOLT_RISK_THRESHOLD,
                },
            ));
        }
    }
}

/// Spec 399: feeds the penal work-speed bonus into the real work
/// pipeline. `work_execution_system` already multiplies work by any
/// [`WorkSpeedBuff`] on the worker (the gastronomy/martyrdom seam), so
/// this sync keeps a fresh buff on every penal laborer each tick.
pub fn sync_penal_work_speed_system(
    mut commands: Commands,
    query: Query<(Entity, &PenalLabor)>,
) {
    for (entity, labor) in &query {
        commands.entity(entity).insert(WorkSpeedBuff {
            multiplier: labor.work_speed_multiplier(),
            duration: PENAL_WORK_SPEED_BUFF_TICKS,
        });
    }
}

/// Removes `PenalLabor` if Inmate leaves zone (and drops the synced
/// work-speed buff so a freed inmate does not keep the penal pace).
pub fn cleanup_penal_work_system(
    mut commands: Commands,
    zone_grid: Res<ZoneGrid>,
    query: Query<(Entity, &GridPosition), With<PenalLabor>>,
) {
    for (entity, pos) in &query {
        if zone_grid.get(pos.x, pos.y) != ZoneType::Penal {
            commands
                .entity(entity)
                .remove::<PenalLabor>()
                .remove::<RevoltRisk>()
                .remove::<WorkSpeedBuff>();
        }
    }
}

/// Increases revolt risk for working inmates (Spec 399 section 5).
///
/// Accumulates [`BASE_REVOLT_RISK_PER_TICK`] per tick while the inmate
/// stands in a Penal zone; a warden/sheriff within
/// [`WARDEN_GUARD_RADIUS`] cuts the gain to [`GUARDED_RISK_FACTOR`].
#[allow(clippy::type_complexity)]
pub fn update_revolt_risk_system(
    zone_grid: Option<Res<ZoneGrid>>,
    mut inmates: Query<(Option<&GridPosition>, &mut RevoltRisk), With<PenalLabor>>,
    guards: Query<(&GridPosition, Option<&PopAction>, Option<&Job>), Without<PenalLabor>>,
) {
    let guard_positions: Vec<GridPosition> = guards
        .iter()
        .filter(|(_, action, job)| is_guard(*action, *job))
        .map(|(pos, _, _)| *pos)
        .collect();
    for (pos, mut risk) in &mut inmates {
        if let (Some(grid), Some(p)) = (zone_grid.as_deref(), pos) {
            if grid.get(p.x, p.y) != ZoneType::Penal {
                continue;
            }
        }
        let guarded = pos.is_some_and(|p| {
            guard_positions
                .iter()
                .any(|g| chebyshev(*p, *g) <= WARDEN_GUARD_RADIUS)
        });
        risk.current += revolt_risk_increment(guarded);
    }
}

/// Triggers a revolt if risk reaches the threshold (Spec 399).
///
/// The inmate loses [`Inmate`]/[`PenalLabor`]/[`RevoltRisk`] and gains
/// [`Rioter`] + [`Wanted`]; their action is forced to vandalism, the
/// breakout is counted in [`PenalLedger`], announced on the chronicle,
/// and bridged to the blacksite [`PrisonBreakEvent`] seam.
pub fn handle_prison_revolt_system(
    mut commands: Commands,
    mut ledger: Option<ResMut<PenalLedger>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
    mut breakout: Option<ResMut<Events<PrisonBreakEvent>>>,
    mut query: Query<(Entity, &RevoltRisk, Option<&mut PopAction>), With<Inmate>>,
) {
    for (entity, risk, action) in &mut query {
        if risk.current >= risk.threshold {
            commands
                .entity(entity)
                .remove::<Inmate>()
                .remove::<PenalLabor>()
                .remove::<RevoltRisk>()
                .remove::<WorkSpeedBuff>()
                .insert(Rioter)
                .insert(Wanted { severity: 1.0 });
            if let Some(mut action) = action {
                action.current = ActionType::Vandalize;
                action.current_utility = 0.0;
                action.ticks_committed = 0;
            }
            if let Some(ref mut ledger) = ledger {
                ledger.revolts += 1;
            }
            if let Some(ref mut events) = chronicle {
                events.send(AddChronicleEvent {
                    text: REVOLT_CHRONICLE_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
            if let Some(ref mut events) = breakout {
                events.send(PrisonBreakEvent);
            }
        }
    }
}

/// Spec 118 name for [`handle_prison_revolt_system`]; kept as the
/// registered alias so older call sites and tests keep working.
pub fn check_jailbreak_system(
    commands: Commands,
    ledger: Option<ResMut<PenalLedger>>,
    chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
    breakout: Option<ResMut<Events<PrisonBreakEvent>>>,
    query: Query<(Entity, &RevoltRisk, Option<&mut PopAction>), With<Inmate>>,
) {
    handle_prison_revolt_system(commands, ledger, chronicle, breakout, query);
}

/// Live census: (penal laborers, rioters, worst current revolt risk).
#[must_use]
pub fn penal_counts(world: &mut World) -> (usize, usize, f32) {
    let mut laborers = 0usize;
    let mut worst = 0.0f32;
    {
        let mut query = world.query::<&RevoltRisk>();
        for risk in query.iter(world) {
            laborers += 1;
            worst = worst.max(risk.current);
        }
    }
    let rioters = world.query::<&Rioter>().iter(world).count();
    (laborers, rioters, worst)
}

/// Census for STATS: `none`, or `labor<n>/riot<n>/revolts<n>/risk<f>`.
#[must_use]
pub fn penal_stats_label(world: &mut World) -> String {
    let (laborers, rioters, worst) = penal_counts(world);
    let revolts = world
        .get_resource::<PenalLedger>()
        .map_or(0, |l| l.revolts);
    if laborers == 0 && rioters == 0 && revolts == 0 {
        "none".to_string()
    } else {
        format!("labor{laborers}/riot{rioters}/revolts{revolts}/risk{worst:.0}")
    }
}

/// Rows for the headless `penal` panel.
#[must_use]
pub fn penal_panel_lines(world: &mut World) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rows: Vec<(u32, String, i32, i32, f32, f32, f32)> = Vec::new();
    {
        let mut query = world.query::<(
            Entity,
            Option<&PopName>,
            &GridPosition,
            &PenalLabor,
            &RevoltRisk,
        )>();
        for (entity, name, pos, labor, risk) in query.iter(world) {
            rows.push((
                entity.index(),
                name.map_or_else(|| "?".to_string(), |n| n.0.clone()),
                pos.x,
                pos.y,
                risk.current,
                risk.threshold,
                labor.work_speed_multiplier(),
            ));
        }
    }
    rows.sort_by_key(|(id, ..)| *id);
    if rows.is_empty() {
        lines.push(
            "No penal laborers. Detain a pop with `penal detain` (they are moved to a penal zone), or designate a Penal zone over jailed inmates."
                .to_string(),
        );
    }
    for (id, name, x, y, current, threshold, mult) in &rows {
        lines.push(format!(
            "id {id} {name} at ({x}, {y}): revolt risk {current:.1}/{threshold:.0}, work speed x{mult:.2}"
        ));
    }
    let (_, rioters, _) = penal_counts(world);
    let revolts = world
        .get_resource::<PenalLedger>()
        .map_or(0, |l| l.revolts);
    lines.push(format!(
        "Rioters at large: {rioters}. Breakouts so far: {revolts}."
    ));
    lines.push(format!(
        "STATS penal={} (labor/riot/revolts/risk).",
        penal_stats_label(world)
    ));
    lines.push(
        "Penal laborers work faster and ignore rest, leisure and hygiene calls while they work; hunger and medical care still reach them. A warden or sheriff nearby slows revolt risk; at the threshold the inmate revolts, breaks out, and turns hostile (wardens can re-arrest a rioter)."
            .to_string(),
    );
    lines
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn penal_public_strings() -> Vec<String> {
    vec![
        "Prison Labor".to_string(),
        REVOLT_CHRONICLE_TEXT.to_string(),
        "No penal laborers. Detain a pop with `penal detain` (they are moved to a penal zone), or designate a Penal zone over jailed inmates.".to_string(),
        "Penal laborers work faster and ignore rest, leisure and hygiene calls while they work; hunger and medical care still reach them. A warden or sheriff nearby slows revolt risk; at the threshold the inmate revolts, breaks out, and turns hostile (wardens can re-arrest a rioter).".to_string(),
    ]
}

/// Debug helper (headless `penal risk <amount>` / `revolt`): set every
/// penal laborer revolt risk to `amount`. Returns how many were set.
pub fn force_revolt_risk(world: &mut World, amount: f32) -> usize {
    let mut count = 0usize;
    let mut query = world.query::<&mut RevoltRisk>();
    for mut risk in query.iter_mut(world) {
        risk.current = amount;
        count += 1;
    }
    count
}

/// Debug helper (headless `penal zone <x> <y>`): designate a 3x3 Penal
/// zone centred on (cx, cy). Returns how many tiles were set.
pub fn designate_penal_zone(world: &mut World, cx: i32, cy: i32) -> usize {
    let mut count = 0usize;
    if let Some(mut grid) = world.get_resource_mut::<ZoneGrid>() {
        for dy in -1..=1 {
            for dx in -1..=1 {
                grid.set(cx + dx, cy + dy, ZoneType::Penal);
                count += 1;
            }
        }
    }
    count
}

/// Debug helper (headless `penal detain`): detain the first free pop —
/// insert [`Inmate`], make sure a Penal zone exists (a fresh 3x3 at the
/// pop position when the colony has none), and move the pop onto a
/// penal tile so the next tick assigns [`PenalLabor`].
pub fn detain_first_pop_in_penal_zone(world: &mut World) -> Option<(Entity, String)> {
    let mut found: Option<(Entity, String, GridPosition)> = None;
    {
        let mut query = world.query_filtered::<(Entity, &PopName, &GridPosition), (
            With<Pop>,
            Without<Inmate>,
            Without<Rioter>,
        )>();
        if let Some((entity, name, pos)) = query.iter(world).next() {
            found = Some((entity, name.0.clone(), *pos));
        }
    }
    let (entity, name, pop_pos) = found?;
    let mut penal_tile: Option<GridPosition> = None;
    if let Some(grid) = world.get_resource::<ZoneGrid>() {
        'scan: for y in 0..grid.height {
            for x in 0..grid.width {
                if grid.get(x as i32, y as i32) == ZoneType::Penal {
                    penal_tile = Some(GridPosition {
                        x: x as i32,
                        y: y as i32,
                    });
                    break 'scan;
                }
            }
        }
    }
    let target = match penal_tile {
        Some(tile) => tile,
        None => {
            designate_penal_zone(world, pop_pos.x, pop_pos.y);
            pop_pos
        }
    };
    world.entity_mut(entity).insert(Inmate {
        sentence_ticks: DEBUG_SENTENCE_TICKS,
    });
    if let Some(mut pos) = world.get_mut::<GridPosition>(entity) {
        *pos = target;
    }
    Some((entity, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::law::justice::Inmate;
    use crate::layer1::map::GridPosition;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::utility_types::PopAction;
    use crate::layer1::zone::{ZoneGrid, ZoneType};

    fn setup_world() -> World {
        let mut world = World::new();
        let mut zone_grid = ZoneGrid::new(20, 20);
        // Define a Penal Zone at (5,5)
        zone_grid.set(5, 5, ZoneType::Penal);
        world.insert_resource(zone_grid);
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world
    }

    #[test]
    fn test_penal_labor_component_defaults() {
        let labor = PenalLabor::default();
        assert!((labor.efficiency_bonus - 0.2).abs() < f32::EPSILON); // 20% faster
        assert!((labor.work_speed_multiplier() - 1.2).abs() < 1e-6);
    }

    #[test]
    fn test_inmate_evaluates_work_in_penal_zone() {
        let mut world = setup_world();

        // Inmate at (5,5) which is a Penal Zone
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                PopAction::default(),
            ))
            .id();

        // Run evaluation system
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            evaluate_penal_work_system,
        );

        // Should receive PenalLabor component
        assert!(world.get::<PenalLabor>(inmate).is_some());
        // Should receive RevoltRisk
        assert!(world.get::<RevoltRisk>(inmate).is_some());
    }

    #[test]
    fn test_inmate_outside_penal_zone_is_idle() {
        let mut world = setup_world();

        // Inmate at (0,0) - NOT a Penal Zone
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 0, y: 0 },
                PopAction::default(),
            ))
            .id();

        // Ensure system doesn't wrongly add it
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            evaluate_penal_work_system,
        );
        assert!(world.get::<PenalLabor>(inmate).is_none());

        // Test cleanup
        world
            .entity_mut(inmate)
            .insert((PenalLabor::default(), RevoltRisk::default()));

        // Still at (0,0) which is not Penal
        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, cleanup_penal_work_system);

        assert!(world.get::<PenalLabor>(inmate).is_none());
    }

    #[test]
    fn test_working_accumulates_revolt_risk() {
        let mut world = setup_world();

        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                PenalLabor::default(), // Actively working
                RevoltRisk {
                    current: 0.0,
                    threshold: 100.0,
                },
            ))
            .id();

        // Run system tick
        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, update_revolt_risk_system);

        let risk = world.get::<RevoltRisk>(inmate).unwrap();
        assert!(risk.current > 0.0);
    }

    #[test]
    fn test_jailbreak_trigger() {
        let mut world = setup_world();

        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                RevoltRisk {
                    current: 101.0,
                    threshold: 100.0,
                }, // Over threshold
            ))
            .id();

        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, check_jailbreak_system);

        // Should lose Inmate status (escaped)
        assert!(world.get::<Inmate>(inmate).is_none());
        assert!(world.get::<PenalLabor>(inmate).is_none());
        assert!(world.get::<RevoltRisk>(inmate).is_none());
    }

    // --- Spec 399 acceptance legs ---

    #[test]
    fn test_spec399_inmate_in_penal_zone_gains_revolt_risk() {
        let mut world = setup_world();
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                Needs {
                    hunger: 0.5,
                    rest: 0.5,
                    leisure: 0.5,
                    hygiene: 0.5,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((evaluate_penal_work_system, update_revolt_risk_system).chain());
        schedule.run(&mut world);

        assert!(world.get::<PenalLabor>(inmate).is_some());
        let risk = world.get::<RevoltRisk>(inmate).unwrap();
        assert!(
            risk.current > 0.0,
            "Revolt risk should accumulate while working in a penal zone"
        );
    }

    #[test]
    fn test_penal_work_speed_buff_is_synced_and_consumed() {
        let mut world = setup_world();
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                PenalLabor::default(),
            ))
            .id();

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            sync_penal_work_speed_system,
        );

        // work_execution_system multiplies work by WorkSpeedBuff.multiplier,
        // so a synced 1.2 buff is the multiplier actually consumed.
        let buff = world.get::<WorkSpeedBuff>(inmate).unwrap();
        assert!((buff.multiplier - 1.2).abs() < 1e-6);
        assert!(buff.duration > 0);
    }

    #[test]
    fn test_cleanup_removes_work_speed_buff() {
        let mut world = setup_world();
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 0, y: 0 },
                PenalLabor::default(),
                RevoltRisk::default(),
                WorkSpeedBuff {
                    multiplier: 1.2,
                    duration: 3,
                },
            ))
            .id();

        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, cleanup_penal_work_system);

        assert!(world.get::<PenalLabor>(inmate).is_none());
        assert!(world.get::<WorkSpeedBuff>(inmate).is_none());
    }

    #[test]
    fn test_revolt_risk_reduced_near_warden() {
        let mut world = setup_world();
        world.resource_mut::<ZoneGrid>().set(5, 15, ZoneType::Penal);

        let guarded = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                PenalLabor::default(),
                RevoltRisk {
                    current: 0.0,
                    threshold: 100.0,
                },
            ))
            .id();
        let unguarded = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 15 },
                PenalLabor::default(),
                RevoltRisk {
                    current: 0.0,
                    threshold: 100.0,
                },
            ))
            .id();
        // Warden on duty right beside the guarded inmate.
        world.spawn((
            Pop,
            GridPosition { x: 6, y: 5 },
            PopAction {
                current: ActionType::Warden,
                current_utility: 0.8,
                ticks_committed: 1,
            },
        ));

        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, update_revolt_risk_system);

        let guarded_risk = world.get::<RevoltRisk>(guarded).unwrap().current;
        let unguarded_risk = world.get::<RevoltRisk>(unguarded).unwrap().current;
        assert!((unguarded_risk - BASE_REVOLT_RISK_PER_TICK).abs() < 1e-6);
        assert!(
            guarded_risk < unguarded_risk,
            "warden nearby should damp risk: guarded {guarded_risk} vs unguarded {unguarded_risk}"
        );
        assert!((guarded_risk - revolt_risk_increment(true)).abs() < 1e-6);
    }

    #[test]
    fn test_revolt_risk_reduced_near_sheriff() {
        let mut world = setup_world();
        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                PenalLabor::default(),
                RevoltRisk {
                    current: 0.0,
                    threshold: 100.0,
                },
            ))
            .id();
        let workplace = world.spawn_empty().id();
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 7 },
            Job {
                workplace,
                job_type: AssignmentType::Sheriff,
            },
        ));

        let _ =
            bevy_ecs::system::RunSystemOnce::run_system_once(&mut world, update_revolt_risk_system);

        let risk = world.get::<RevoltRisk>(inmate).unwrap().current;
        assert!((risk - revolt_risk_increment(true)).abs() < 1e-6);
    }

    #[test]
    fn test_revolt_transforms_inmate_into_rioter_with_chronicle() {
        let mut world = setup_world();
        world.init_resource::<PenalLedger>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<PrisonBreakEvent>>();
        world.init_resource::<crate::layer1::core::chronicle::Chronicle>();

        let inmate = world
            .spawn((
                Pop,
                Inmate {
                    sentence_ticks: 1000,
                },
                GridPosition { x: 5, y: 5 },
                PenalLabor::default(),
                RevoltRisk {
                    current: 100.0,
                    threshold: 100.0,
                },
                PopAction::default(),
            ))
            .id();

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            handle_prison_revolt_system,
        );

        assert!(world.get::<Inmate>(inmate).is_none());
        assert!(world.get::<PenalLabor>(inmate).is_none());
        assert!(world.get::<Rioter>(inmate).is_some(), "should become a Rioter");
        let wanted = world.get::<Wanted>(inmate).unwrap();
        assert!((wanted.severity - 1.0).abs() < f32::EPSILON);
        let action = world.get::<PopAction>(inmate).unwrap();
        assert_eq!(action.current, ActionType::Vandalize);
        assert_eq!(world.resource::<PenalLedger>().revolts, 1);

        // The chronicle bridge records the breakout.
        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            crate::layer1::core::chronicle::chronicle_event_handler_system,
        );
        let chronicle = world.resource::<crate::layer1::core::chronicle::Chronicle>();
        assert_eq!(chronicle.events.len(), 1);
        assert!(chronicle.events[0].text.contains("PRISON REVOLT"));
    }

    #[test]
    fn test_penal_stats_label() {
        let mut world = setup_world();
        assert_eq!(penal_stats_label(&mut world), "none");
        world.init_resource::<PenalLedger>();
        world.spawn((
            Pop,
            Inmate {
                sentence_ticks: 1000,
            },
            GridPosition { x: 5, y: 5 },
            PenalLabor::default(),
            RevoltRisk {
                current: 42.0,
                threshold: 100.0,
            },
        ));
        world.spawn((Pop, Rioter));
        assert_eq!(
            penal_stats_label(&mut world),
            "labor1/riot1/revolts0/risk42"
        );
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        let banned = [
            "shawshank",
            "green mile",
            "alcatraz",
            "prison break",
            "orange is the new black",
            "prison architect",
            "rimworld",
            "dwarf fortress",
            "stellaris",
            "gulag",
            "cool hand",
            "papillon",
        ];
        for text in penal_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string contains banned term {term:?}: {text}"
                );
            }
        }
    }
}
use crate::layer1::administration::edicts::{ColonyPolicies, Policy};
use crate::layer1::economy::items::ItemType;
use crate::layer1::health::Dead;
use crate::layer1::psychology::traits::Trait;
use crate::layer1::social::morale::Morale;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct ColonyInventory {
    pub vital_organs: usize,
}

impl ColonyInventory {
    pub fn add(&mut self, _item: ItemType, amount: usize) {
        self.vital_organs += amount;
    }

    pub fn get_amount(&self, _item: &ItemType) -> usize {
        self.vital_organs
    }
}

#[derive(Event, Default)]
pub struct OrganHarvestedEvent;

pub fn process_dead_pops_for_organs_system(
    mut commands: Commands,
    policies: Option<Res<ColonyPolicies>>,
    inventory: Option<ResMut<ColonyInventory>>,
    dead_pops: Query<Entity, With<Dead>>,
    mut harvest_events: EventWriter<OrganHarvestedEvent>,
) {
    if let Some(policies) = policies {
        if policies.is_active(Policy::MandatoryOrganHarvesting) {
            if let Some(mut inv) = inventory {
                for entity in dead_pops.iter() {
                    inv.add(ItemType::VitalOrgans, 1);
                    commands.entity(entity).try_despawn_recursive();
                    harvest_events.send(OrganHarvestedEvent);
                }
            }
        }
    }
}

pub fn apply_harvesting_horror_system(
    mut events: EventReader<OrganHarvestedEvent>,
    mut pops: Query<(
        &mut Morale,
        Option<&crate::layer1::psychology::traits::Traits>,
    )>,
) {
    let mut harvested = false;
    for _ in events.read() {
        harvested = true;
    }
    if !harvested {
        return;
    }

    for (mut morale, traits_opt) in pops.iter_mut() {
        let is_psycho = traits_opt.is_some_and(|t| t.0.contains(&Trait::Psychopath));
        if !is_psycho {
            morale.value -= 20.0;
            morale.value = morale.value.max(0.0);
        }
    }
}

#[cfg(test)]
mod organ_trade_tests {
    use super::*;

    use crate::layer1::entities::pop::Pop;
    use crate::layer1::psychology::traits::Traits;

    #[test]
    fn test_organ_harvesting_edict_produces_organs() {
        let mut world = World::new();

        let mut policies = ColonyPolicies::default();
        policies
            .active_policies
            .insert(Policy::MandatoryOrganHarvesting);
        world.insert_resource(policies);

        let inventory = ColonyInventory::default();
        world.insert_resource(inventory);
        world.insert_resource(Events::<OrganHarvestedEvent>::default());

        let dead_pop = world.spawn((Pop, Dead)).id();

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            process_dead_pops_for_organs_system,
        );

        let current_inventory = world.resource::<ColonyInventory>();
        assert_eq!(current_inventory.get_amount(&ItemType::VitalOrgans), 1);
        assert!(world.get_entity(dead_pop).is_err());
    }

    #[test]
    fn test_organ_harvesting_causes_horror() {
        let mut world = World::new();
        world.insert_resource(Events::<OrganHarvestedEvent>::default());

        let normal_pop = world
            .spawn((
                Pop,
                Morale {
                    value: 100.0,
                    ..Default::default()
                },
            ))
            .id();

        let mut traits = Traits::default();
        traits.0.insert(Trait::Psychopath);
        let psycho_pop = world
            .spawn((
                Pop,
                traits,
                Morale {
                    value: 100.0,
                    ..Default::default()
                },
            ))
            .id();

        world
            .resource_mut::<Events<OrganHarvestedEvent>>()
            .send(OrganHarvestedEvent);

        let _ = bevy_ecs::system::RunSystemOnce::run_system_once(
            &mut world,
            apply_harvesting_horror_system,
        );

        let normal_morale = world.get::<Morale>(normal_pop).unwrap();
        let psycho_morale = world.get::<Morale>(psycho_pop).unwrap();

        assert!(normal_morale.value < 100.0);
        assert_eq!(psycho_morale.value, 100.0);
    }
}
