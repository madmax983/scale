//! Spec 365: The Mundane Cult.
//!
//! A religion forms not around gods or anomalies, but around the most
//! boring, repetitive work in the colony. Pops who spend an extended
//! tenure in a menial formal job have — deterministically, at a tenure
//! threshold, the same pattern as Spec 264's mutation threshold — a
//! collective awakening: once at least [`MIN_FOUNDERS`] pops share the
//! same menial job at threshold tenure, they found a cult venerating
//! that job itself. Cultists treat the job as sacred duty:
//!
//! * They work it at +50% speed ([`mundane_cult_efficiency_multiplier`],
//!   applied in the one production speed seam — the per-pop speed
//!   modifier assembled in `execution::general_work`'s worker
//!   collection, alongside the trait/job/buff/fog/nostalgia modifiers).
//! * They refuse any other job. A reassignment away from the sacred
//!   job is reverted by [`mundane_cult_refusal_system`] and lands the
//!   "Forsaking the Sacred Duty" mood penalty (−0.60). The utility
//!   scoring half of the spec's REFACTOR lives in
//!   [`sacred_job_utility_adjustment`] (+2.0 sacred / −100.0
//!   non-sacred): UtilityAI scores `ActionType`s, not assignments, so
//!   there is no per-assignment score in the live evaluator to adjust —
//!   the refusal is enforced at the layer where reassignment actually
//!   happens, the pop's [`Job`] component.
//! * The cult demands a shrine. An active cult with no shrine carries
//!   a real [`FactionDemand`] (the guild demand type, reused verbatim)
//!   naming its parameterized shrine — a [`BuildingType::CultShrine`]
//!   building carrying a [`CultShrineFocus`] for the cult's job.
//!   Building one satisfies the demand.
//! * Automating the sacred work angers them. While any drone is
//!   hauling (the real automation workforce doing menial haul work),
//!   cultists periodically protest: "Sacred Work Stolen by Machines"
//!   mood penalties and a rising colony [`Unrest`] level.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **`JobType` does not exist.** The spec's sketch names a `JobType`
//!   with a menial tier flag. The real job type is
//!   [`AssignmentType`] (the pop's [`Job::job_type`]), and it has no
//!   tier flag and no Hauling/Cleaning variants — hauling is a
//!   UtilityAI `ActionType`, not a formal job, and there is no cleaner
//!   job at all. [`is_menial_job`] therefore classifies the real
//!   formal jobs whose work is manual, repetitive labour —
//!   `FarmWorker`, `DeepMining`, `Funeral` (corpse burial) — and the
//!   cult focus is an `AssignmentType`, per the spec's own §5 pointer
//!   at the real tenure system.
//! * **`JobTenure` is real** (`crate::layer1::specialization`, Spec
//!   264): a per-pop `HashMap<AssignmentType, u64>` accrued by
//!   `update_tenure_system` while a pop holds a [`Job`]. Formation
//!   reads it directly; no parallel tenure is built. Grounding also
//!   found the seam *dormant*: `update_tenure_system` was registered
//!   in no production schedule and no spawn path inserts `JobTenure`,
//!   so tenure never accrued in live play. The cult chain in
//!   `simulation.rs` therefore registers the real accrual system and
//!   [`mundane_cult_ensure_tenure_system`] inserts the missing
//!   component on job-holding pops — the minimal wiring that makes
//!   both Spec 264's ledger and this cult work organically. (Spec
//!   264's `check_mutation_system` remains unregistered, as found;
//!   wiring trait mutations is out of this spec's scope. Note also
//!   that production job assignment in `execution::arrival` writes
//!   `Job` for FarmWorker/DeepMining/Administrator/Doctor arrivals
//!   but explicitly not for Funeral assignments, so Funeral cults
//!   are reachable only where a `Job` is set by other means.)
//! * **No new `Faction` type.** The guild system (`social::factions`)
//!   keys factions by a closed [`crate::layer1::social::factions::
//!   FactionId`] enum assigned from a pop's highest *skill*;
//!   re-keying it for job cults would fight the membership system.
//!   The cult is therefore its own record in [`MundaneCultState`]
//!   (one per sacred job), but its shrine demand is the genuine
//!   [`FactionDemand`] struct, so demand tooling/idioms carry over.
//! * **Chance → threshold.** The spec says long-tenured pops "have a
//!   chance" to form a cult. Every comparable system in the codebase
//!   (Spec 264 mutations at 10,000 ticks, the Nostalgia Plague's
//!   exposure threshold) is a deterministic accumulator, and dice
//!   would make the RED tests flaky, so formation is a threshold:
//!   [`CULT_TENURE_THRESHOLD`] ticks held by at least
//!   [`MIN_FOUNDERS`] pops in the same menial job. A cult that already
//!   exists accepts later converts singly.
//! * **Shrine.** Spec §5 asks for a generic shrine building
//!   parameterized by the cult's focus. [`BuildingType::CultShrine`]
//!   is that building; the parameterization is the
//!   [`CultShrineFocus`] component placed on the building entity
//!   (sacred job + shrine name), so one building type serves every
//!   cult — "The Holy Furrow" for farmhands, "The Holy Pick" for
//!   delvers, "The Holy Shroud" for gravediggers.
//! * **Automation seam.** There is no abstract "automation" event;
//!   the real automation workforce is the drone fleet
//!   (`entities::drone::Drone`, priority Charge > Repair > Haul).
//!   A drone in [`DroneState::Hauling`] is a machine doing the
//!   colony's menial haul work, and that is what the unrest system
//!   watches.
//!
//! All names and strings are original to Scale — no borrowed IP.
//! Cult and shrine names are generated from the job itself.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::entities::drone::{Drone, DroneState};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Job, Pop};
use crate::layer1::social::factions::FactionDemand;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::unrest::Unrest;
use crate::layer1::specialization::JobTenure;
use crate::layer1::utility_types::AssignmentType;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Tenure (ticks in one menial job, per the real [`JobTenure`]) at
/// which a pop is ripe for conversion. A tenth of Spec 264's 10,000-
/// tick mutation threshold: a cult is a social awakening, not a
/// biological one, and it should be reachable inside a long run.
pub const CULT_TENURE_THRESHOLD: u64 = 1_000;
/// Pops sharing one menial job at threshold tenure needed to *found*
/// a cult. A faction of one is a hobby; two is a religion. Once a
/// cult exists, later converts join singly.
pub const MIN_FOUNDERS: usize = 2;
/// Work-speed multiplier for a cultist performing their sacred job.
pub const SACRED_EFFICIENCY: f32 = 1.5;
/// Utility adjustment for a cultist's sacred job (see
/// [`sacred_job_utility_adjustment`]).
pub const SACRED_UTILITY_BONUS: f32 = 2.0;
/// Utility adjustment for any non-sacred job: effectively never
/// chosen.
pub const NONSACRED_UTILITY_PENALTY: f32 = -100.0;
/// Mood penalty value for being reassigned away from the sacred job.
pub const FORSAKING_PENALTY: f32 = -0.60;
/// Duration (ticks) of the "Forsaking the Sacred Duty" penalty.
pub const FORSAKING_DURATION: u32 = 600;
/// Label of the forsaking mood modifier.
pub const FORSAKING_LABEL: &str = "Forsaking the Sacred Duty";
/// Mood penalty value when machines do the sacred work.
pub const AUTOMATION_PENALTY: f32 = -0.30;
/// Duration (ticks) of the automation mood penalty.
pub const AUTOMATION_DURATION: u32 = 500;
/// Label of the automation mood modifier.
pub const AUTOMATION_LABEL: &str = "Sacred Work Stolen by Machines";
/// Colony unrest added per automation protest.
pub const AUTOMATION_UNREST: f32 = 0.10;
/// Minimum ticks between automation protests (per colony).
pub const AUTOMATION_PROTEST_COOLDOWN: u64 = 500;
/// Ticks a shrine demand allows before it would expire (the demand
/// does not strike — cults are patient — but the field is real).
pub const SHRINE_DEMAND_TICKS: f32 = 2_000.0;

// ---------------------------------------------------------------------------
// Menial classification (spec §7: define "menial")
// ---------------------------------------------------------------------------

/// True when `job` is one of the colony's menial formal jobs: manual,
/// repetitive labour. See the module §8 notes for why the set is
/// `FarmWorker` / `DeepMining` / `Funeral` and not the spec sketch's
/// hauling/cleaning (neither is an [`AssignmentType`]).
#[must_use]
pub const fn is_menial_job(job: AssignmentType) -> bool {
    matches!(
        job,
        AssignmentType::FarmWorker | AssignmentType::DeepMining | AssignmentType::Funeral
    )
}

/// The cult's name for a sacred job, generated from the job itself
/// (spec §7: "The Holy Order of Haulers" pattern).
#[must_use]
pub const fn cult_name_for(job: AssignmentType) -> &'static str {
    match job {
        AssignmentType::FarmWorker => "The Holy Order of Farmhands",
        AssignmentType::DeepMining => "The Holy Order of Delvers",
        AssignmentType::Funeral => "The Holy Order of Gravediggers",
        _ => "The Holy Order of Custodians",
    }
}

/// The shrine a cult demands for its sacred job (spec's "The Holy Air
/// Filter" pattern, one per menial job).
#[must_use]
pub const fn shrine_name_for(job: AssignmentType) -> &'static str {
    match job {
        AssignmentType::FarmWorker => "The Holy Furrow",
        AssignmentType::DeepMining => "The Holy Pick",
        AssignmentType::Funeral => "The Holy Shroud",
        _ => "The Holy Relic",
    }
}

/// Short label for a sacred job, for panels and chronicles.
#[must_use]
pub const fn job_label(job: AssignmentType) -> &'static str {
    match job {
        AssignmentType::FarmWorker => "farm work",
        AssignmentType::DeepMining => "deep mining",
        AssignmentType::Funeral => "corpse burial",
        AssignmentType::Administrator => "administration",
        AssignmentType::LibraryWorker => "library work",
        AssignmentType::ObservatoryWorker => "observatory work",
        AssignmentType::Sheriff => "law enforcement",
        AssignmentType::Doctor => "medicine",
        _ => "work",
    }
}

/// Parse a headless job token into a menial [`AssignmentType`].
#[must_use]
pub fn parse_menial_job(token: &str) -> Option<AssignmentType> {
    match token.to_ascii_lowercase().as_str() {
        "farm" | "farmworker" | "farming" => Some(AssignmentType::FarmWorker),
        "mining" | "deepmining" | "mine" => Some(AssignmentType::DeepMining),
        "funeral" | "burial" | "gravedigger" => Some(AssignmentType::Funeral),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A pop who has joined a Mundane Cult. The sacred job is the one
/// they held when the cult took them; it never changes.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct MundaneCultMember {
    /// The job this cultist venerates and refuses to leave.
    pub sacred_job: AssignmentType,
    /// The cult's name at joining (see [`cult_name_for`]).
    pub cult_name: String,
    /// Tick at which the pop joined.
    pub joined_tick: u64,
}

/// Parameterization of a [`BuildingType::CultShrine`] building: which
/// cult's shrine this is. One building type, every cult (spec §5).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct CultShrineFocus {
    /// The sacred job this shrine serves.
    pub sacred_job: AssignmentType,
    /// The shrine's name (see [`shrine_name_for`]).
    pub shrine_name: String,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One active cult, keyed by its sacred job in [`MundaneCultState`].
#[derive(Debug, Clone)]
pub struct CultRecord {
    /// The job the cult venerates.
    pub sacred_job: AssignmentType,
    /// Generated cult name.
    pub cult_name: String,
    /// Generated shrine name.
    pub shrine_name: String,
    /// Tick the cult was founded.
    pub founded_tick: u64,
    /// Living members, recounted by the formation system.
    pub members: usize,
    /// The cult's outstanding shrine demand, if any. The genuine
    /// guild [`FactionDemand`] type, reused.
    pub demand: Option<FactionDemand>,
    /// True once a shrine for this cult stands in the colony.
    pub shrine_satisfied: bool,
}

/// Colony-wide ledger of Mundane Cults (Spec 365).
#[derive(Resource, Default, Debug)]
pub struct MundaneCultState {
    /// Active cults by sacred job (at most one cult per job).
    pub cults: HashMap<AssignmentType, CultRecord>,
    /// Total pops ever converted.
    pub conversions: u64,
    /// Total reassignments refused.
    pub refusals: u64,
    /// Total shrine demands ever issued.
    pub shrine_demands_issued: u64,
    /// Total automation protests.
    pub automation_protests: u64,
    /// Tick of the last automation protest (cooldown gate).
    pub last_automation_protest_tick: Option<u64>,
}

impl MundaneCultState {
    /// Number of active cults.
    #[must_use]
    pub fn active_cults(&self) -> usize {
        self.cults.len()
    }

    /// The cult venerating `job`, if one is active.
    #[must_use]
    pub fn cult_for(&self, job: AssignmentType) -> Option<&CultRecord> {
        self.cults.get(&job)
    }
}

// ---------------------------------------------------------------------------
// Pure scoring helpers (the spec's RED-testable core)
// ---------------------------------------------------------------------------

/// Work-speed multiplier for a pop on `current_job`: cultists fly on
/// their sacred job (+50%, spec §3 test 3) and are unremarkable
/// otherwise. `member` is `None` for ordinary pops (always 1.0).
/// Applied in the `general_work` speed seam.
#[must_use]
pub fn mundane_cult_efficiency_multiplier(
    member: Option<&MundaneCultMember>,
    current_job: AssignmentType,
) -> f32 {
    match member {
        Some(m) if m.sacred_job == current_job => SACRED_EFFICIENCY,
        _ => 1.0,
    }
}

/// Utility adjustment for assigning a cultist to `candidate` (spec
/// §5 REFACTOR): the sacred job is boosted, every other job is
/// penalized so heavily it is effectively never selected.
#[must_use]
pub fn sacred_job_utility_adjustment(member: &MundaneCultMember, candidate: AssignmentType) -> f32 {
    if candidate == member.sacred_job {
        SACRED_UTILITY_BONUS
    } else {
        NONSACRED_UTILITY_PENALTY
    }
}

/// Whether a cultist may be reassigned to `new_job`. Cultists refuse
/// everything but the sacred job (spec §3 test 2).
#[must_use]
pub fn cultist_accepts_assignment(member: &MundaneCultMember, new_job: AssignmentType) -> bool {
    sacred_job_utility_adjustment(member, new_job) > NONSACRED_UTILITY_PENALTY / 2.0
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Tenure wiring (see the module §8 notes): Spec 264's accrual
/// system exists but was never registered in the production
/// schedule, and no spawn path inserts [`JobTenure`] — the seam was
/// dormant. This system inserts a default tenure ledger on any pop
/// holding a [`Job`] who lacks one, so the real
/// `update_tenure_system` (registered alongside the cult systems in
/// `simulation.rs`) can accrue it.
pub fn mundane_cult_ensure_tenure_system(
    mut commands: Commands,
    missing: Query<Entity, (With<Job>, Without<JobTenure>)>,
) {
    for entity in &missing {
        commands.entity(entity).insert(JobTenure::default());
    }
}

/// Formation: pops whose real [`JobTenure`] in a menial job reaches
/// [`CULT_TENURE_THRESHOLD`] found (or join) that job's cult.
///
/// Founding needs [`MIN_FOUNDERS`] ripe pops in the same job; an
/// existing cult converts ripe pops singly. Founding writes a Major
/// chronicle entry; joining is quiet.
#[allow(clippy::too_many_arguments)]
pub fn mundane_cult_formation_system(
    mut commands: Commands,
    candidates: Query<(Entity, &Job, &JobTenure), Without<MundaneCultMember>>,
    members: Query<&MundaneCultMember>,
    mut state: ResMut<MundaneCultState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);

    // Recount living membership for existing cults.
    for record in state.cults.values_mut() {
        record.members = 0;
    }
    for member in &members {
        if let Some(record) = state.cults.get_mut(&member.sacred_job) {
            record.members += 1;
        }
    }

    // Group ripe candidates by menial job.
    let mut ripe: HashMap<AssignmentType, Vec<Entity>> = HashMap::new();
    for (entity, job, tenure) in &candidates {
        if !is_menial_job(job.job_type) {
            continue;
        }
        if tenure.get_ticks(job.job_type) >= CULT_TENURE_THRESHOLD {
            ripe.entry(job.job_type).or_default().push(entity);
        }
    }

    for (job, entities) in ripe {
        let existing = state.cults.contains_key(&job);
        if !existing && entities.len() < MIN_FOUNDERS {
            continue;
        }
        if !existing {
            state.cults.insert(
                job,
                CultRecord {
                    sacred_job: job,
                    cult_name: cult_name_for(job).to_string(),
                    shrine_name: shrine_name_for(job).to_string(),
                    founded_tick: tick,
                    members: 0,
                    demand: None,
                    shrine_satisfied: false,
                },
            );
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "{} has formed: the colony's {} is sacred now, and its \
                     keepers will do no other work.",
                    cult_name_for(job),
                    job_label(job),
                ),
                importance: EventImportance::Major,
            });
        }
        for entity in entities {
            commands.entity(entity).insert(MundaneCultMember {
                sacred_job: job,
                cult_name: cult_name_for(job).to_string(),
                joined_tick: tick,
            });
            state.conversions += 1;
            if let Some(record) = state.cults.get_mut(&job) {
                record.members += 1;
            }
        }
    }
}

/// Refusal: a cultist whose [`Job`] has been changed away from the
/// sacred job immediately abandons the new job (the assignment is
/// reverted) and suffers the "Forsaking the Sacred Duty" penalty.
pub fn mundane_cult_refusal_system(
    mut query: Query<(&mut Job, &MundaneCultMember, Option<&mut Morale>), Changed<Job>>,
    mut state: ResMut<MundaneCultState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for (mut job, member, morale) in &mut query {
        if job.job_type == member.sacred_job {
            continue;
        }
        let refused = job.job_type;
        job.job_type = member.sacred_job;
        state.refusals += 1;
        if let Some(mut morale) = morale {
            if !morale.modifiers.iter().any(|m| m.label == FORSAKING_LABEL) {
                morale.add_modifier(MoodModifier {
                    label: FORSAKING_LABEL.to_string(),
                    value: FORSAKING_PENALTY,
                    duration: FORSAKING_DURATION,
                });
            }
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A keeper of {} refuses {}: the sacred duty is {}, and no other.",
                member.cult_name,
                job_label(refused),
                job_label(member.sacred_job),
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Shrine demands: an active cult with no shrine and no outstanding
/// demand issues a [`FactionDemand`] for its shrine; a standing
/// [`CultShrineFocus`] building for its job satisfies the demand.
pub fn mundane_cult_shrine_demand_system(
    shrines: Query<&CultShrineFocus>,
    mut state: ResMut<MundaneCultState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let satisfied: Vec<AssignmentType> = shrines.iter().map(|s| s.sacred_job).collect();
    let mut new_demands: u64 = 0;
    for record in state.cults.values_mut() {
        if satisfied.contains(&record.sacred_job) {
            if !record.shrine_satisfied {
                record.shrine_satisfied = true;
                record.demand = None;
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "{} stands complete. {} is content — the sacred work has a home.",
                        record.shrine_name, record.cult_name,
                    ),
                    importance: EventImportance::Standard,
                });
            }
            continue;
        }
        record.shrine_satisfied = false;
        if record.demand.is_none() && record.members > 0 {
            record.demand = Some(FactionDemand {
                policy: None,
                remaining_time: SHRINE_DEMAND_TICKS,
                description: format!(
                    "Build the shrine: {} ({} demands a {})",
                    record.shrine_name, record.cult_name, "Cult Shrine",
                ),
            });
            new_demands += 1;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "{} demands a shrine: build {} so the sacred {} may be honoured.",
                    record.cult_name,
                    record.shrine_name,
                    job_label(record.sacred_job),
                ),
                importance: EventImportance::Standard,
            });
        }
    }
    state.shrine_demands_issued += new_demands;
}

/// Automation unrest: while a drone is hauling — a machine doing the
/// colony's menial haul work — cultists protest on a cooldown: mood
/// penalties for every member and a rising colony unrest level.
pub fn mundane_cult_automation_unrest_system(
    drones: Query<&Drone>,
    mut members: Query<&mut Morale, With<MundaneCultMember>>,
    mut state: ResMut<MundaneCultState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    unrest: Option<ResMut<Unrest>>,
    time: Option<Res<SimulationTime>>,
) {
    if state.cults.is_empty() {
        return;
    }
    let hauling = drones.iter().any(|d| d.state == DroneState::Hauling);
    if !hauling {
        return;
    }
    let tick = time.map_or(0, |t| t.tick);
    if let Some(last) = state.last_automation_protest_tick {
        if tick.saturating_sub(last) < AUTOMATION_PROTEST_COOLDOWN {
            return;
        }
    }
    let mut any = false;
    for mut morale in &mut members {
        any = true;
        if !morale.modifiers.iter().any(|m| m.label == AUTOMATION_LABEL) {
            morale.add_modifier(MoodModifier {
                label: AUTOMATION_LABEL.to_string(),
                value: AUTOMATION_PENALTY,
                duration: AUTOMATION_DURATION,
            });
        }
    }
    if !any {
        return;
    }
    state.last_automation_protest_tick = Some(tick);
    state.automation_protests += 1;
    if let Some(mut unrest) = unrest {
        unrest.level = (unrest.level + AUTOMATION_UNREST).clamp(0.0, 1.0);
    }
    chronicle.send(AddChronicleEvent {
        text: "The mundane cults protest: machines are hauling, and hauling \
               is sacred. Sacred work stolen by machines is theft from the rite."
            .to_string(),
        importance: EventImportance::Standard,
    });
}

// ---------------------------------------------------------------------------
// World helpers (headless panel / STATS / debug)
// ---------------------------------------------------------------------------

/// Number of living cultists in the world.
#[must_use]
pub fn cultist_count(world: &mut World) -> usize {
    world.query::<&MundaneCultMember>().iter(world).count()
}

/// One-line status label for the headless panel and STATS.
#[must_use]
pub fn mundane_cult_status_label(world: &mut World) -> String {
    let cultists = cultist_count(world);
    match world.get_resource::<MundaneCultState>() {
        Some(state) => format!(
            "{} cult(s), {} cultist(s), {} conversion(s), {} refusal(s), \
             {} shrine demand(s) issued, {} automation protest(s)",
            state.active_cults(),
            cultists,
            state.conversions,
            state.refusals,
            state.shrine_demands_issued,
            state.automation_protests,
        ),
        None => format!("0 cult(s), {cultists} cultist(s)"),
    }
}

/// Debug seam (headless `cult form <job>`): found (or extend) the
/// cult for `job` by converting up to eight pops currently holding
/// that job, regardless of tenure. Returns how many were converted.
pub fn force_form_cult(world: &mut World, job: AssignmentType) -> usize {
    if !is_menial_job(job) {
        return 0;
    }
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let targets: Vec<Entity> = {
        let mut query =
            world.query_filtered::<(Entity, &Job), (With<Pop>, Without<MundaneCultMember>)>();
        query
            .iter(world)
            .filter(|(_, j)| j.job_type == job)
            .map(|(e, _)| e)
            .take(8)
            .collect()
    };
    if targets.is_empty() {
        return 0;
    }
    {
        let mut state = world.resource_mut::<MundaneCultState>();
        state.cults.entry(job).or_insert_with(|| CultRecord {
            sacred_job: job,
            cult_name: cult_name_for(job).to_string(),
            shrine_name: shrine_name_for(job).to_string(),
            founded_tick: tick,
            members: 0,
            demand: None,
            shrine_satisfied: false,
        });
        state.conversions += targets.len() as u64;
        if let Some(record) = state.cults.get_mut(&job) {
            record.members += targets.len();
        }
    }
    for entity in &targets {
        world.entity_mut(*entity).insert(MundaneCultMember {
            sacred_job: job,
            cult_name: cult_name_for(job).to_string(),
            joined_tick: tick,
        });
    }
    targets.len()
}

/// Debug seam (headless `cult shrine <job>`): raise the cult's
/// shrine as a real [`BuildingType::CultShrine`] building carrying
/// its [`CultShrineFocus`]. Returns false when no such cult exists.
pub fn spawn_cult_shrine(world: &mut World, job: AssignmentType) -> bool {
    let shrine_name = match world
        .get_resource::<MundaneCultState>()
        .and_then(|s| s.cults.get(&job))
    {
        Some(record) => record.shrine_name.clone(),
        None => return false,
    };
    world.spawn((
        Building {
            building_type: BuildingType::CultShrine,
        },
        CultShrineFocus {
            sacred_job: job,
            shrine_name,
        },
        GridPosition { x: 0, y: 0 },
    ));
    true
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real types — see §8 notes)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;

    fn cult_app() -> App {
        let mut app = App::new();
        app.init_resource::<MundaneCultState>();
        app.init_resource::<Events<AddChronicleEvent>>();
        app.init_resource::<Unrest>();
        app.add_systems(
            bevy_app::Update,
            (
                mundane_cult_ensure_tenure_system,
                crate::layer1::specialization::update_tenure_system,
                mundane_cult_formation_system,
                mundane_cult_refusal_system,
                mundane_cult_shrine_demand_system,
                mundane_cult_automation_unrest_system,
            )
                .chain(),
        );
        app
    }

    fn spawn_tenured(app: &mut App, job: AssignmentType, ticks: u64) -> Entity {
        app.world_mut()
            .spawn((
                Pop,
                Job {
                    workplace: Entity::PLACEHOLDER,
                    job_type: job,
                },
                JobTenure::with_ticks(job, ticks),
                Morale::default(),
            ))
            .id()
    }

    /// Spec §3 test 1: long tenure in a menial job spawns a cult
    /// focused on that job.
    #[test]
    fn test_long_tenure_in_menial_job_spawns_cult_faction() {
        let mut app = cult_app();
        let a = spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        let b = spawn_tenured(
            &mut app,
            AssignmentType::FarmWorker,
            CULT_TENURE_THRESHOLD + 50,
        );
        let _c = spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        app.update();

        let state = app.world().resource::<MundaneCultState>();
        let record = state
            .cult_for(AssignmentType::FarmWorker)
            .expect("a cult should have formed around farm work");
        assert_eq!(record.cult_name, "The Holy Order of Farmhands");
        assert_eq!(record.shrine_name, "The Holy Furrow");
        assert!(record.members >= MIN_FOUNDERS);
        assert!(app.world().get::<MundaneCultMember>(a).is_some());
        assert!(app.world().get::<MundaneCultMember>(b).is_some());
        let member = app.world().get::<MundaneCultMember>(a).unwrap();
        assert_eq!(member.sacred_job, AssignmentType::FarmWorker);
    }

    #[test]
    fn test_short_tenure_and_lone_pop_do_not_found_cults() {
        let mut app = cult_app();
        // Below threshold: no cult.
        let _a = spawn_tenured(
            &mut app,
            AssignmentType::FarmWorker,
            CULT_TENURE_THRESHOLD - 1,
        );
        let _b = spawn_tenured(&mut app, AssignmentType::FarmWorker, 10);
        app.update();
        assert!(app.world().resource::<MundaneCultState>().cults.is_empty());

        // One ripe pop alone is a hobby, not a religion.
        let mut app = cult_app();
        let _solo = spawn_tenured(&mut app, AssignmentType::DeepMining, CULT_TENURE_THRESHOLD);
        app.update();
        assert!(app.world().resource::<MundaneCultState>().cults.is_empty());
    }

    #[test]
    fn test_non_menial_tenure_never_forms_a_cult() {
        let mut app = cult_app();
        for _ in 0..3 {
            spawn_tenured(&mut app, AssignmentType::Administrator, 50_000);
        }
        app.update();
        assert!(app.world().resource::<MundaneCultState>().cults.is_empty());
        assert!(!is_menial_job(AssignmentType::Administrator));
        assert!(is_menial_job(AssignmentType::FarmWorker));
        assert!(is_menial_job(AssignmentType::DeepMining));
        assert!(is_menial_job(AssignmentType::Funeral));
    }

    #[test]
    fn test_existing_cult_accepts_later_converts_singly() {
        let mut app = cult_app();
        spawn_tenured(&mut app, AssignmentType::Funeral, CULT_TENURE_THRESHOLD);
        spawn_tenured(&mut app, AssignmentType::Funeral, CULT_TENURE_THRESHOLD);
        app.update();
        assert!(app
            .world()
            .resource::<MundaneCultState>()
            .cult_for(AssignmentType::Funeral)
            .is_some());

        let late = spawn_tenured(&mut app, AssignmentType::Funeral, CULT_TENURE_THRESHOLD);
        app.update();
        assert!(app.world().get::<MundaneCultMember>(late).is_some());
    }

    /// Spec §3 test 2: cult members refuse job reassignment — the
    /// assignment is abandoned/reverted and the massive mood penalty
    /// for forsaking the sacred duty lands.
    #[test]
    fn test_cult_members_refuse_job_reassignment() {
        let mut app = cult_app();
        let pop = spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        app.update();
        assert!(app.world().get::<MundaneCultMember>(pop).is_some());

        // The player reassigns the cultist to Science-adjacent work.
        app.world_mut().get_mut::<Job>(pop).unwrap().job_type = AssignmentType::Administrator;
        app.update();

        // Assignment abandoned: the pop is back on the sacred job.
        let job = app.world().get::<Job>(pop).unwrap();
        assert_eq!(job.job_type, AssignmentType::FarmWorker);
        // Massive mood penalty applied.
        let morale = app.world().get::<Morale>(pop).unwrap();
        let penalty = morale
            .modifiers
            .iter()
            .find(|m| m.label == FORSAKING_LABEL)
            .expect("forsaking penalty should be present");
        assert_eq!(penalty.value, FORSAKING_PENALTY);
        assert!(penalty.value <= -0.5, "penalty must be massive");
        assert_eq!(app.world().resource::<MundaneCultState>().refusals, 1);
        // The pure acceptance check agrees.
        let member = app.world().get::<MundaneCultMember>(pop).unwrap();
        assert!(!cultist_accepts_assignment(
            member,
            AssignmentType::Administrator
        ));
        assert!(cultist_accepts_assignment(
            member,
            AssignmentType::FarmWorker
        ));
        assert_eq!(
            sacred_job_utility_adjustment(member, AssignmentType::Administrator),
            NONSACRED_UTILITY_PENALTY
        );
        assert_eq!(
            sacred_job_utility_adjustment(member, AssignmentType::FarmWorker),
            SACRED_UTILITY_BONUS
        );
    }

    /// Spec §3 test 3: cult members gain massive efficiency in the
    /// sacred job (+50% speed).
    #[test]
    fn test_cult_members_gain_massive_efficiency_in_sacred_job() {
        let member = MundaneCultMember {
            sacred_job: AssignmentType::DeepMining,
            cult_name: cult_name_for(AssignmentType::DeepMining).to_string(),
            joined_tick: 0,
        };
        assert_eq!(
            mundane_cult_efficiency_multiplier(Some(&member), AssignmentType::DeepMining),
            1.5
        );
        // No bonus off the sacred job, and none for ordinary pops.
        assert_eq!(
            mundane_cult_efficiency_multiplier(Some(&member), AssignmentType::FarmWorker),
            1.0
        );
        assert_eq!(
            mundane_cult_efficiency_multiplier(None, AssignmentType::DeepMining),
            1.0
        );
    }

    /// Spec §3 test 4: an active cult emits a FactionDemand for a
    /// shrine related to its job; building the shrine satisfies it.
    #[test]
    fn test_cult_demands_shrine_construction() {
        let mut app = cult_app();
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        app.update();
        app.update();

        let (demand_text, shrine) = {
            let state = app.world().resource::<MundaneCultState>();
            let record = state
                .cult_for(AssignmentType::FarmWorker)
                .expect("cult should exist");
            let demand = record
                .demand
                .as_ref()
                .expect("active cult should demand a shrine");
            (demand.description.clone(), record.shrine_name.clone())
        };
        assert!(
            demand_text.contains(&shrine),
            "demand names the shrine: {demand_text}"
        );
        assert!(demand_text.contains("Cult Shrine"));
        assert_eq!(
            app.world()
                .resource::<MundaneCultState>()
                .shrine_demands_issued,
            1
        );

        // Building the parameterized shrine satisfies the demand.
        assert!(spawn_cult_shrine(
            app.world_mut(),
            AssignmentType::FarmWorker
        ));
        app.update();
        let state = app.world().resource::<MundaneCultState>();
        let record = state.cult_for(AssignmentType::FarmWorker).unwrap();
        assert!(record.shrine_satisfied);
        assert!(record.demand.is_none());
    }

    /// Acceptance: automating the sacred work (a hauling drone)
    /// causes unrest among cultists — mood penalties, a protest on
    /// the ledger, and rising colony unrest, on a cooldown.
    #[test]
    fn test_automating_sacred_work_causes_cult_unrest() {
        let mut app = cult_app();
        let pop = spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        app.update();
        assert!(app.world().get::<MundaneCultMember>(pop).is_some());

        // A drone starts hauling: the machines take up menial work.
        app.world_mut().spawn(Drone {
            state: DroneState::Hauling,
        });
        app.update();

        let morale = app.world().get::<Morale>(pop).unwrap();
        assert!(
            morale.modifiers.iter().any(|m| m.label == AUTOMATION_LABEL),
            "cultist should be angered by automation"
        );
        assert_eq!(
            app.world()
                .resource::<MundaneCultState>()
                .automation_protests,
            1
        );
        assert!(app.world().resource::<Unrest>().level > 0.0);

        // Cooldown: an immediate second pass does not protest again.
        app.update();
        assert_eq!(
            app.world()
                .resource::<MundaneCultState>()
                .automation_protests,
            1
        );
    }

    #[test]
    fn test_no_drone_no_automation_unrest() {
        let mut app = cult_app();
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        spawn_tenured(&mut app, AssignmentType::FarmWorker, CULT_TENURE_THRESHOLD);
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<MundaneCultState>()
                .automation_protests,
            0
        );
    }

    /// IP guard: every generated cult/shrine string is original —
    /// no borrowed franchise terms.
    #[test]
    fn test_mundane_cult_names_are_original() {
        const BANNED: &[&str] = &[
            "warhammer",
            "jedi",
            "starfleet",
            "borg",
            "imperium",
            "adeptus",
            "arrakis",
            "klingon",
            "vulcan",
            "hitchhiker",
            "trantor",
            "machine god",
            "omnissiah",
            "emperor",
            "god-emperor",
        ];
        for job in [
            AssignmentType::FarmWorker,
            AssignmentType::DeepMining,
            AssignmentType::Funeral,
        ] {
            for text in [cult_name_for(job), shrine_name_for(job)] {
                let lower = text.to_ascii_lowercase();
                for banned in BANNED {
                    assert!(
                        !lower.contains(banned),
                        "generated name {text:?} contains banned term {banned:?}"
                    );
                }
            }
        }
    }

    /// Acceptance, the organic path: pops spawn holding a menial job
    /// with NO tenure ledger; the ensure system inserts one, Spec
    /// 264's real accrual system ticks it up, and at the threshold
    /// the cult founds itself — no test-only tenure injection.
    #[test]
    fn test_cult_forms_organically_from_accrued_tenure() {
        let mut app = cult_app();
        let spawn = |app: &mut App| {
            app.world_mut()
                .spawn((
                    Pop,
                    Job {
                        workplace: Entity::PLACEHOLDER,
                        job_type: AssignmentType::FarmWorker,
                    },
                    Morale::default(),
                ))
                .id()
        };
        let a = spawn(&mut app);
        let _b = spawn(&mut app);

        // The first update only inserts the missing tenure ledgers
        // (deferred Commands), so accrual needs one extra pass:
        // threshold + 1 updates in total.
        for _ in 0..=(CULT_TENURE_THRESHOLD as usize) {
            app.update();
        }
        let state = app.world().resource::<MundaneCultState>();
        assert!(
            state.cult_for(AssignmentType::FarmWorker).is_some(),
            "cult should form organically once accrued tenure crosses the threshold"
        );
        assert!(app.world().get::<MundaneCultMember>(a).is_some());
        let tenure = app.world().get::<JobTenure>(a).unwrap();
        assert!(tenure.get_ticks(AssignmentType::FarmWorker) >= CULT_TENURE_THRESHOLD);
    }

    #[test]
    fn test_parse_menial_job_tokens() {
        assert_eq!(parse_menial_job("farm"), Some(AssignmentType::FarmWorker));
        assert_eq!(parse_menial_job("MINING"), Some(AssignmentType::DeepMining));
        assert_eq!(parse_menial_job("funeral"), Some(AssignmentType::Funeral));
        assert_eq!(parse_menial_job("science"), None);
    }
}
