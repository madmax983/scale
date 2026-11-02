//! Spec 387 "Workplace Hazards" — building-level accident risk.
//!
//! Specs 035/155 already landed a *per-action* hazard lane in
//! [`crate::layer1::environment::hazards`]: when a pop performs a Work /
//! Repair / Tame action, `handle_workplace_hazards` rolls against that
//! action's danger level, scaled by the pop's skill and the target
//! structure's maintenance (HP), and `trigger_accident` applies the
//! severity bands from `determine_severity` (Critical also fires
//! `AmputationEvent`). That lane is untouched by this module.
//!
//! What was missing — and what this module adds — is the *building*
//! lane from spec 387:
//!
//! * a pure, deterministic risk helper driven by a building's condition
//!   (its [`Structure`] HP fraction) and its workload (workers present);
//! * a periodic process system that rolls each occupied building and
//!   emits [`WorkplaceAccidentEvent`], gated by an [`AccidentCooldown`]
//!   so a building cannot multi-trigger in one tick or spam every tick;
//! * a separate injury system listening for that event, applying the
//!   existing severity machinery (same damage bands as
//!   `trigger_accident`; Critical also fires the existing
//!   `AmputationEvent`) plus a temporary [`WorkSpeedBuff`] debuff —
//!   the clean existing work-speed seam, decayed by the live
//!   gastronomy buff-decay system — and writing a chronicle entry;
//! * the global "Safety Protocols" edict ([`Policy::SafetyProtocols`]):
//!   hazard risk is multiplied by [`SAFETY_PROTOCOLS_RISK_MULTIPLIER`]
//!   (0.5×) here *and* in the per-action lane (via
//!   `edicts::get_hazard_modifier`, consumed by `general_work.rs`),
//!   while work speed drops by 0.10 via `get_work_speed_modifier`.
//!
//! Deviations from the spec sketch (adapted to the real codebase):
//! * The sketch's `Workplace { workers, accident_risk }` component and
//!   `crate::layer1::buildings::Workplace` do not exist. Risk is
//!   *derived* from real [`Building`] entities each tick — never stored
//!   on a parallel component only tests populate. Workers at a building
//!   are grounded in two real seams, deduplicated: pops with an
//!   [`AssignedTo`] assignment naming the building, and pops standing
//!   on the building's tile.
//! * [`AccidentRiskOverride`] is the one stored-risk exception: an
//!   optional debug/staging component (used by the headless `hazardize`
//!   command and by tests) that replaces the derived base risk for a
//!   building. It is still pre-safety-multiplier, so the Safety
//!   Protocols effect stays observable on staged buildings.
//! * Severity is carried on the event as an explicit `severity_roll`
//!   (the `determine_severity` pattern: pure functions take the roll),
//!   which keeps the injury system deterministic under test.
//! * No new health/injury system: damage goes through [`Health`], the
//!   debuff through the existing [`WorkSpeedBuff`].

use crate::layer1::actions::AssignedTo;
use crate::layer1::administration::edicts::{ColonyPolicies, Policy};
use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::environment::hazards::{determine_severity, AccidentSeverity, AmputationEvent};
use crate::layer1::gastronomy::WorkSpeedBuff;
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::structure::Structure;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Hazard-risk multiplier while Safety Protocols is active (0.5×).
pub const SAFETY_PROTOCOLS_RISK_MULTIPLIER: f32 = 0.5;
/// Work-speed delta while Safety Protocols is active (−0.10), applied
/// by `edicts::get_work_speed_modifier`.
pub const SAFETY_PROTOCOLS_WORK_SPEED_DELTA: f32 = -0.10;
/// Ticks a building is immune after an accident fires.
pub const ACCIDENT_COOLDOWN_TICKS: u32 = 100;
/// Per-tick base risk for a staffed building in perfect condition.
pub const BASE_WORKLOAD_RISK: f32 = 0.01;
/// Extra per-tick risk at zero structure HP (full damage adds this).
pub const DAMAGE_RISK_FACTOR: f32 = 0.25;
/// Workload scaling per additional worker beyond the first (+25%).
pub const EXTRA_WORKER_RISK_FACTOR: f32 = 0.25;

/// Risk multiplier contributed by the Safety Protocols edict.
#[must_use]
pub fn safety_risk_multiplier(policies: &ColonyPolicies) -> f32 {
    if policies.is_active(Policy::SafetyProtocols) {
        SAFETY_PROTOCOLS_RISK_MULTIPLIER
    } else {
        1.0
    }
}

/// Pure building accident risk (probability per tick, 0.0–1.0).
///
/// * No workers present → 0.0 (nobody to injure), however damaged.
/// * Pristine *and* idle → 0.0; a pristine staffed building carries
///   only the small [`BASE_WORKLOAD_RISK`].
/// * Risk rises linearly with the damage fraction
///   (`1 − hp_fraction`) and with each additional worker.
#[must_use]
pub fn building_accident_risk(hp_fraction: f32, worker_count: usize, safety_multiplier: f32) -> f32 {
    if worker_count == 0 {
        return 0.0;
    }
    let damage = 1.0 - hp_fraction.clamp(0.0, 1.0);
    #[allow(clippy::cast_precision_loss)]
    let workload = EXTRA_WORKER_RISK_FACTOR.mul_add((worker_count - 1) as f32, 1.0);
    (DAMAGE_RISK_FACTOR.mul_add(damage, BASE_WORKLOAD_RISK) * workload * safety_multiplier)
        .clamp(0.0, 1.0)
}

/// Risk for a building from its optional [`Structure`]: a building
/// with no Structure component is treated as pristine.
#[must_use]
pub fn building_accident_risk_for_structure(
    structure: Option<&Structure>,
    worker_count: usize,
    safety_multiplier: f32,
) -> f32 {
    let hp_fraction = structure.map_or(1.0, |s| {
        if s.max_hp > 0.0 {
            s.current_hp / s.max_hp
        } else {
            1.0
        }
    });
    building_accident_risk(hp_fraction, worker_count, safety_multiplier)
}

/// Deterministic trigger check: a roll in `[0, 1)` fires iff it falls
/// below the (clamped) risk — risk 1.0 always fires, risk 0.0 never.
#[must_use]
pub fn should_trigger_accident(risk: f32, roll: f32) -> bool {
    risk > 0.0 && roll < risk.clamp(0.0, 1.0)
}

/// Cooldown component (spec 387 §5): while `ticks_remaining > 0` the
/// building cannot trigger another accident; it ticks down once per
/// process-system run.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct AccidentCooldown {
    /// Ticks until this building can trigger again.
    pub ticks_remaining: u32,
}

/// Debug/staging override for a building's base (pre-safety) accident
/// risk. Set by the headless `hazardize` command; never inserted by
/// ordinary gameplay systems.
#[derive(Component, Debug, Clone, Copy)]
pub struct AccidentRiskOverride {
    /// Base accident risk per tick (0.0–1.0).
    pub risk: f32,
}

/// Event fired when a building-level workplace accident occurs.
#[derive(Event, Debug, Clone)]
pub struct WorkplaceAccidentEvent {
    /// The injured worker.
    pub worker: Entity,
    /// The building where the accident happened.
    pub workplace: Entity,
    /// Explicit severity roll in `[0, 1)` (see `determine_severity`).
    pub severity_roll: f32,
}

/// Colony-wide ledger of building-level workplace accidents.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct AccidentLedger {
    /// Total accidents applied by the injury system.
    pub total: u64,
    /// Minor accidents.
    pub minor: u64,
    /// Major accidents.
    pub major: u64,
    /// Critical accidents.
    pub critical: u64,
}

/// Workers present at a building: assigned to it ([`AssignedTo`]) or
/// standing on its tile. Sorted and deduplicated for determinism.
fn present_workers(
    worker_list: &[(Entity, GridPosition, Option<Entity>)],
    building: Entity,
    building_pos: GridPosition,
) -> Vec<Entity> {
    let mut present: Vec<Entity> = worker_list
        .iter()
        .filter(|(_, pos, assigned)| *assigned == Some(building) || *pos == building_pos)
        .map(|(entity, _, _)| *entity)
        .collect();
    present.sort_unstable();
    present.dedup();
    present
}

/// Periodic process system (spec 387 §3/§4, adapted): for each real
/// [`Building`], derive its accident risk from condition + workload,
/// apply the Safety Protocols multiplier, and roll once. On a trigger
/// it emits [`WorkplaceAccidentEvent`] for the first present worker
/// and arms the building's [`AccidentCooldown`].
#[allow(clippy::type_complexity)]
pub fn process_workplace_hazards_system(
    mut commands: Commands,
    mut buildings: Query<
        (
            Entity,
            &Building,
            &GridPosition,
            Option<&Structure>,
            Option<&AccidentRiskOverride>,
            Option<&mut AccidentCooldown>,
        ),
    >,
    workers: Query<(Entity, &GridPosition, Option<&AssignedTo>), With<Pop>>,
    policies: Option<Res<ColonyPolicies>>,
    mut events: EventWriter<WorkplaceAccidentEvent>,
) {
    let safety_multiplier = policies
        .as_ref()
        .map_or(1.0, |p| safety_risk_multiplier(p));
    let worker_list: Vec<(Entity, GridPosition, Option<Entity>)> = workers
        .iter()
        .map(|(entity, pos, assigned)| (entity, *pos, assigned.map(|a| a.entity)))
        .collect();

    let mut rng = rand::thread_rng();
    for (entity, _building, pos, structure, risk_override, cooldown) in &mut buildings {
        let mut cooldown = cooldown;
        if let Some(cd) = cooldown.as_mut() {
            if cd.ticks_remaining > 0 {
                cd.ticks_remaining -= 1;
                continue;
            }
        }
        let present = present_workers(&worker_list, entity, *pos);
        if present.is_empty() {
            continue;
        }
        let risk = risk_override.map_or_else(
            || building_accident_risk_for_structure(structure, present.len(), safety_multiplier),
            |o| (o.risk * safety_multiplier).clamp(0.0, 1.0),
        );
        let roll: f32 = rng.gen();
        if should_trigger_accident(risk, roll) {
            let severity_roll: f32 = rng.gen();
            events.send(WorkplaceAccidentEvent {
                worker: present[0],
                workplace: entity,
                severity_roll,
            });
            if let Some(cd) = cooldown.as_mut() {
                cd.ticks_remaining = ACCIDENT_COOLDOWN_TICKS;
            } else {
                commands.entity(entity).insert(AccidentCooldown {
                    ticks_remaining: ACCIDENT_COOLDOWN_TICKS,
                });
            }
        }
    }
}

/// Separate injury system (spec 387 §5/§7): listens for
/// [`WorkplaceAccidentEvent`] and applies consequences through the
/// existing severity machinery — the same damage bands as
/// `trigger_accident` (Minor 10 / Major 40 / Critical 80), Critical
/// also firing the existing [`AmputationEvent`] — plus a temporary
/// [`WorkSpeedBuff`] debuff, a ledger entry, and a chronicle entry
/// (spam-capped in practice by the process system's cooldown).
pub fn workplace_accident_injury_system(
    mut events: EventReader<WorkplaceAccidentEvent>,
    mut health_query: Query<&mut Health>,
    mut commands: Commands,
    mut amputations: EventWriter<AmputationEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut ledger: Option<ResMut<AccidentLedger>>,
) {
    for event in events.read() {
        let severity = determine_severity(event.severity_roll);
        let (damage, speed_multiplier, debuff_ticks) = match severity {
            AccidentSeverity::Minor => (10.0, 0.75, 200),
            AccidentSeverity::Major => (40.0, 0.5, 500),
            AccidentSeverity::Critical => (80.0, 0.5, 1000),
        };
        if let Ok(mut health) = health_query.get_mut(event.worker) {
            health.take_damage(damage);
        }
        commands.entity(event.worker).insert(WorkSpeedBuff {
            multiplier: speed_multiplier,
            duration: debuff_ticks,
        });
        if severity == AccidentSeverity::Critical {
            amputations.send(AmputationEvent {
                entity: event.worker,
            });
        }
        if let Some(ledger) = ledger.as_mut() {
            ledger.total += 1;
            match severity {
                AccidentSeverity::Minor => ledger.minor += 1,
                AccidentSeverity::Major => ledger.major += 1,
                AccidentSeverity::Critical => ledger.critical += 1,
            }
        }
        chronicle.send(AddChronicleEvent {
            importance: if severity == AccidentSeverity::Critical {
                EventImportance::Major
            } else {
                EventImportance::Standard
            },
            text: format!(
                "Workplace accident at building {:?}: a worker was injured ({severity:?}, -{damage:.0} HP).",
                event.workplace
            ),
        });
    }
}

/// One risky building as listed by the headless `hazards` command.
pub struct RiskyBuilding {
    /// Building entity.
    pub entity: Entity,
    /// Building type.
    pub building_type: BuildingType,
    /// Building position.
    pub position: GridPosition,
    /// Workers present (assigned or on-tile).
    pub workers: usize,
    /// Current per-tick accident risk (post-safety-multiplier).
    pub risk: f32,
}

/// World-style census of buildings with non-zero accident risk,
/// sorted by descending risk — the `hazards` command / STATS seam.
#[must_use]
pub fn risky_buildings(world: &mut World) -> Vec<RiskyBuilding> {
    let safety_multiplier = world
        .get_resource::<ColonyPolicies>()
        .map_or(1.0, safety_risk_multiplier);
    // Only pops count as workers; AssignedTo/GridPosition alone do not.
    let worker_list: Vec<(Entity, GridPosition, Option<Entity>)> = {
        let mut q_pop =
            world.query_filtered::<(Entity, &GridPosition, Option<&AssignedTo>), With<Pop>>();
        q_pop
            .iter(world)
            .map(|(entity, pos, assigned)| (entity, *pos, assigned.map(|a| a.entity)))
            .collect()
    };
    let mut out: Vec<RiskyBuilding> = Vec::new();
    let mut q = world.query::<(
        Entity,
        &Building,
        &GridPosition,
        Option<&Structure>,
        Option<&AccidentRiskOverride>,
    )>();
    for (entity, building, pos, structure, risk_override) in q.iter(world) {
        let present = present_workers(&worker_list, entity, *pos);
        if present.is_empty() {
            continue;
        }
        let risk = risk_override.map_or_else(
            || building_accident_risk_for_structure(structure, present.len(), safety_multiplier),
            |o| (o.risk * safety_multiplier).clamp(0.0, 1.0),
        );
        if risk > 0.0 {
            out.push(RiskyBuilding {
                entity,
                building_type: building.building_type,
                position: *pos,
                workers: present.len(),
                risk,
            });
        }
    }
    out.sort_by(|a, b| b.risk.partial_cmp(&a.risk).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Total building-level accidents recorded in the ledger (0 if absent).
#[must_use]
pub fn accident_count(world: &World) -> u64 {
    world.get_resource::<AccidentLedger>().map_or(0, |l| l.total)
}

/// `safety=on|off` label for STATS / the headless `safety` command.
#[must_use]
pub fn safety_label(world: &World) -> &'static str {
    world
        .get_resource::<ColonyPolicies>()
        .map_or("off", |p| {
            if p.is_active(Policy::SafetyProtocols) {
                "on"
            } else {
                "off"
            }
        })
}
