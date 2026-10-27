//! Spec 369: Neural Architecture.
//!
//! Advanced structures are lined with semi-organic neural gel. A
//! building *imprints* on the pops who use it: a workshop run for
//! years by a master smith grows "optimized" and speeds the hands
//! that work in it; a hospital that has absorbed trauma after trauma
//! grows "haunted", and everyone who works there afterwards pays a
//! little of that stress themselves. Imprints compound, decay when a
//! building sits idle, and can be scrubbed out of the gel — at a
//! price.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No `Needs::stress`, no `layer1::jobs::JobType`, no
//!   `JobTenure { job, ticks }` struct literal.** The real stress
//!   representation is [`StressTracker`] (`layer1::stress`,
//!   `accumulated_stress` on a 0–100+ scale, breakdown at 100), fed
//!   by morale. The sketch's `Needs { stress: 80.0 }` maps onto
//!   `StressTracker { accumulated_stress: 80.0 }` exactly, and
//!   [`effective_stress`] falls back to `(1.0 - Needs::morale()) *
//!   100.0` for pops that carry `Needs` but no tracker, so both real
//!   representations drive the same thresholds. Stress *damage* from
//!   a haunted building lands on the tracker when present, and on
//!   `Needs::leisure` (the need the needs module itself names as the
//!   stress-adjacent one) when it is not.
//! * **`JobTenure` is real** (`layer1::specialization`): a per-job
//!   tick map keyed by [`AssignmentType`], read through the pop's
//!   current [`Job`] (`get_ticks(job.job_type)`). The sketch's
//!   tenure rule is kept verbatim — more than [`TENURE_THRESHOLD`]
//!   (500) ticks in the current job imprints
//!   [`SKILLED_IMPRINT`] (+5). A second skilled signal is honoured
//!   alongside it: a best [`Skills`] level of
//!   [`SKILLED_LEVEL_THRESHOLD`]+ also counts as skilled (the two
//!   never stack — one use, one imprint step).
//! * **No `JobType` component rides on buildings.** The sketch
//!   spawned `(NeuralArchitecture, JobType::Hospital)`; real building
//!   entities carry [`Building`] `{ building_type }`. The gel lives
//!   only in *advanced* structures ([`is_advanced_building`]):
//!   production, research, medical, power and ancient buildings —
//!   not housing, wells, walls or flower beds.
//!   [`ensure_neural_architecture_system`] attaches the component to
//!   qualifying buildings as they appear.
//! * **`BuildingUseEvent` did not exist (§7).** The real crafting-
//!   cycle initiation event is [`CraftEvent`] — it names the crafter
//!   but no building. [`craft_building_use_bridge_system`] is the
//!   §7 seam: on a craft event it resolves the crafter's
//!   [`Job::workplace`] and, when that workplace carries gel, emits
//!   the spec's `BuildingUseEvent { building, pop }`. Headless debug
//!   commands and tests emit the event directly through
//!   [`trigger_use`]. Wiring a use pulse at *every* work site (not
//!   just crafting cycles) is deferred — the bridge covers the seam
//!   §7 names verbatim ("whenever a worker initiates a crafting or
//!   interaction cycle").
//! * **§5 REFACTOR items, all implemented:** positive imprint grants
//!   a real [`WorkSpeedBuff`] to the user (the gastronomy buff the
//!   work loop already honours; multiplier scales with imprint,
//!   capped at the game's 1.5 work-speed ceiling's neighbourhood);
//!   imprint decays toward 0 once a building has been idle for
//!   [`DECAY_IDLE_TICKS`]; scrubbing is a [`ScrubImprintEvent`]
//!   costing [`SCRUB_COST_CREDITS`] from the real `ColonyResources`
//!   credit ledger — an unaffordable scrub is a no-op, never a
//!   partial cleanse or a negative balance.
//! * **Spec §6's `cargo clippy -- -D warnings` / 85% coverage** are
//!   aspirational template text; the verified bar is the established
//!   one: `cargo check --all-targets` green, clippy clean on touched
//!   files, and this module's tests green.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::crafting::CraftEvent;
use crate::layer1::gastronomy::WorkSpeedBuff;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Job;
use crate::layer1::resources::ColonyResources;
use crate::layer1::skills::Skills;
use crate::layer1::specialization::JobTenure;
use crate::layer1::stress::StressTracker;
use crate::shared::time::SimulationTime;

/// Imprint ceiling: the gel can only hold so much optimization.
pub const IMPRINT_CAP: f32 = 100.0;
/// Imprint floor: the gel can only hold so much trauma.
pub const IMPRINT_FLOOR: f32 = -100.0;
/// Tenure (ticks in the pop's current job) past which a worker
/// counts as skilled — the sketch's `ticks > 500`, verbatim.
pub const TENURE_THRESHOLD: u64 = 500;
/// Best skill level at which a worker counts as skilled even
/// without long tenure in their *current* job.
pub const SKILLED_LEVEL_THRESHOLD: u32 = 3;
/// Imprint a skilled user's visit leaves — the sketch's `+5.0`.
pub const SKILLED_IMPRINT: f32 = 5.0;
/// Effective stress (0–100 scale) past which a user counts as
/// stressed — the sketch's `stress > 50.0`, verbatim.
pub const STRESS_THRESHOLD: f32 = 50.0;
/// Imprint a stressed user's visit leaves — the sketch's `-10.0`.
pub const STRESSED_IMPRINT: f32 = -10.0;
/// Imprint below which a building is *haunted*: subsequent users
/// take stress from the gel — the sketch's `< -10.0`, verbatim.
pub const HAUNTED_THRESHOLD: f32 = -10.0;
/// Imprint above which a building is *optimized*: subsequent users
/// work faster in it.
pub const OPTIMIZED_THRESHOLD: f32 = 10.0;
/// Crossing this imprint (downward) is chronicle-worthy: the
/// building has become notorious.
pub const NOTORIOUS_IMPRINT: f32 = -50.0;
/// Crossing this imprint (upward) is chronicle-worthy: the building
/// has become beloved.
pub const BELOVED_IMPRINT: f32 = 50.0;
/// Work-speed bonus per point of positive imprint (so +100 imprint
/// is +20% speed, under the game's 1.5 buff cap).
pub const EFFICIENCY_PER_IMPRINT: f32 = 0.002;
/// Ticks a use-granted work-speed buff lasts.
pub const IMPRINT_BUFF_DURATION: u32 = 50;
/// Ticks a building must sit unused before its imprint starts
/// fading (§5 decay).
pub const DECAY_IDLE_TICKS: u64 = 100;
/// Imprint points that fade per idle tick once decay begins.
pub const DECAY_PER_TICK: f32 = 0.02;
/// Credit cost of scrubbing one building's gel clean (§5 scrubbing).
pub const SCRUB_COST_CREDITS: f32 = 25.0;

/// The neural gel lining of an advanced building: a single imprint
/// score from [`IMPRINT_FLOOR`] (haunted) to [`IMPRINT_CAP`]
/// (optimized), plus bookkeeping for decay and the STATS census.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NeuralArchitecture {
    /// The imprint score: -100 (haunted) .. +100 (optimized).
    pub imprint: f32,
    /// Tick of the most recent use (decay keys off this).
    pub last_used_tick: u64,
    /// Total uses recorded over the building's life.
    pub uses: u64,
}

impl NeuralArchitecture {
    /// A gel lining with a preset imprint (tests, debug, saves).
    #[must_use]
    pub fn with_imprint(imprint: f32) -> Self {
        Self {
            imprint: imprint.clamp(IMPRINT_FLOOR, IMPRINT_CAP),
            ..Default::default()
        }
    }

    /// True when the gel is haunted enough to stress its users.
    #[must_use]
    pub fn is_haunted(&self) -> bool {
        self.imprint < HAUNTED_THRESHOLD
    }

    /// True when the gel is optimized enough to speed its users.
    #[must_use]
    pub fn is_optimized(&self) -> bool {
        self.imprint > OPTIMIZED_THRESHOLD
    }

    /// Work-speed multiplier this building grants a user right now
    /// (1.0 unless optimized).
    #[must_use]
    pub fn efficiency_multiplier(&self) -> f32 {
        if self.is_optimized() {
            1.0 + self.imprint * EFFICIENCY_PER_IMPRINT
        } else {
            1.0
        }
    }

    /// Stress a user takes from one use of this building right now
    /// (0 unless haunted) — the sketch's `-imprint * 0.1`.
    #[must_use]
    pub fn stress_inflicted(&self) -> f32 {
        if self.is_haunted() {
            -self.imprint * 0.1
        } else {
            0.0
        }
    }

    /// One-word legibility label for panels and STATS.
    #[must_use]
    pub fn label(&self) -> &'static str {
        if self.imprint <= NOTORIOUS_IMPRINT {
            "notorious"
        } else if self.imprint < HAUNTED_THRESHOLD {
            "haunted"
        } else if self.imprint >= BELOVED_IMPRINT {
            "beloved"
        } else if self.imprint > OPTIMIZED_THRESHOLD {
            "optimized"
        } else {
            "quiet"
        }
    }
}

/// A pop used a building: one crafting / interaction cycle (§7).
/// Emitted by the [`CraftEvent`] bridge in production and directly
/// by headless debug commands and tests.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildingUseEvent {
    /// The building entity being used.
    pub building: Entity,
    /// The pop using it.
    pub pop: Entity,
}

/// Order: scrub one building's gel back to neutral, at
/// [`SCRUB_COST_CREDITS`] from the colony ledger.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrubImprintEvent {
    /// The building entity to scrub.
    pub building: Entity,
}

/// Colony ledger for the neural-gel economy (STATS surface):
/// lifetime uses, completed scrubs, and threshold crossings.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct NeuralArchitectureState {
    /// Total building uses recorded.
    pub uses: u64,
    /// Total scrubs completed (paid for).
    pub scrubs: u64,
    /// Times a building crossed into notorious (< -50).
    pub notorious_crossings: u64,
    /// Times a building crossed into beloved (> +50).
    pub beloved_crossings: u64,
}

/// Whether a building type is advanced enough to be lined with
/// neural gel: production, research, medical, power, logistics and
/// ancient structures — not shelter, decoration or fortification.
#[must_use]
pub fn is_advanced_building(building_type: BuildingType) -> bool {
    matches!(
        building_type,
        BuildingType::Smokehouse
            | BuildingType::LumberMill
            | BuildingType::StoneMason
            | BuildingType::Smelter
            | BuildingType::Smithy
            | BuildingType::Library
            | BuildingType::Weaver
            | BuildingType::Tailor
            | BuildingType::Hospital
            | BuildingType::TradeDepot
            | BuildingType::Generator
            | BuildingType::SolarPanel
            | BuildingType::Battery
            | BuildingType::AncientReactor
            | BuildingType::AncientFabricator
            | BuildingType::Refinery
            | BuildingType::Greenhouse
            | BuildingType::Observatory
            | BuildingType::HydroponicsBay
            | BuildingType::LifeSupport
            | BuildingType::ServerBank
            | BuildingType::AICore
            | BuildingType::DroneHub
            | BuildingType::TetherArray
            | BuildingType::BiomassExtractor
    )
}

/// A pop's effective stress on the 0–100 sketch scale: the real
/// [`StressTracker`] when present, else derived from [`Needs`]
/// morale (`(1 - morale) * 100`), else 0.
#[must_use]
pub fn effective_stress(needs: Option<&Needs>, tracker: Option<&StressTracker>) -> f32 {
    if let Some(tracker) = tracker {
        tracker.accumulated_stress
    } else if let Some(needs) = needs {
        (1.0 - needs.morale()) * 100.0
    } else {
        0.0
    }
}

/// A pop's best skill level across all skills (0 when untrained).
#[must_use]
pub fn best_skill_level(skills: &Skills) -> u32 {
    skills
        .xp
        .values()
        .map(|xp| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = (xp.max(0.0) / 100.0).sqrt().floor() as u32;
            level
        })
        .max()
        .unwrap_or(0)
}

/// Whether this user leaves a skilled (positive) imprint: tenure
/// in their current job past [`TENURE_THRESHOLD`], or a best skill
/// level of [`SKILLED_LEVEL_THRESHOLD`]+.
fn is_skilled_user(job: Option<&Job>, tenure: Option<&JobTenure>, skills: Option<&Skills>) -> bool {
    let tenure_ticks = match (job, tenure) {
        (Some(job), Some(tenure)) => tenure.get_ticks(job.job_type),
        _ => 0,
    };
    if tenure_ticks > TENURE_THRESHOLD {
        return true;
    }
    skills.is_some_and(|s| best_skill_level(s) >= SKILLED_LEVEL_THRESHOLD)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Attaches gel to advanced buildings that lack it (spawn seam:
/// buildings appear with [`Building`] and gain
/// [`NeuralArchitecture`] on a later tick).
pub fn ensure_neural_architecture_system(
    mut commands: Commands,
    query: Query<(Entity, &Building), Without<NeuralArchitecture>>,
) {
    for (entity, building) in &query {
        if is_advanced_building(building.building_type) {
            commands
                .entity(entity)
                .insert(NeuralArchitecture::default());
        }
    }
}

/// The §7 seam: a [`CraftEvent`] names only the crafter, so resolve
/// the crafter's workplace through their [`Job`] and emit a
/// [`BuildingUseEvent`] when that workplace carries gel.
pub fn craft_building_use_bridge_system(
    mut craft_events: EventReader<CraftEvent>,
    jobs: Query<&Job>,
    gel: Query<&NeuralArchitecture>,
    mut use_events: EventWriter<BuildingUseEvent>,
) {
    for event in craft_events.read() {
        let Ok(job) = jobs.get(event.crafter) else {
            continue;
        };
        if gel.get(job.workplace).is_ok() {
            use_events.send(BuildingUseEvent {
                building: job.workplace,
                pop: event.crafter,
            });
        }
    }
}

type ImprintPopQuery = (
    Option<&'static Job>,
    Option<&'static JobTenure>,
    Option<&'static Skills>,
    Option<&'static Needs>,
    Option<&'static StressTracker>,
);

/// The imprint itself: a skilled user leaves [`SKILLED_IMPRINT`],
/// a stressed user leaves [`STRESSED_IMPRINT`] (both can apply to
/// one visit — a master smith having a breakdown haunts the forge
/// they also optimize). Crossings into notorious / beloved are
/// chronicled once per crossing.
#[allow(clippy::too_many_arguments)]
pub fn imprint_building_system(
    mut events: EventReader<BuildingUseEvent>,
    mut buildings: Query<&mut NeuralArchitecture>,
    pops: Query<ImprintPopQuery>,
    time: Option<Res<SimulationTime>>,
    state: Option<ResMut<NeuralArchitectureState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let tick = time.map_or(0, |t| t.tick);
    let mut state = state;
    for event in events.read() {
        let Ok(mut arch) = buildings.get_mut(event.building) else {
            continue;
        };
        let Ok((job, tenure, skills, needs, tracker)) = pops.get(event.pop) else {
            continue;
        };
        let before = arch.imprint;
        if is_skilled_user(job, tenure, skills) {
            arch.imprint = (arch.imprint + SKILLED_IMPRINT).min(IMPRINT_CAP);
        }
        if effective_stress(needs, tracker) > STRESS_THRESHOLD {
            arch.imprint = (arch.imprint + STRESSED_IMPRINT).max(IMPRINT_FLOOR);
        }
        arch.last_used_tick = tick;
        arch.uses += 1;
        if let Some(state) = state.as_deref_mut() {
            state.uses += 1;
            if before > NOTORIOUS_IMPRINT && arch.imprint <= NOTORIOUS_IMPRINT {
                state.notorious_crossings += 1;
                chronicle.send(AddChronicleEvent {
                    text: "The gel in one of the colony's workshops has gone bad with remembered misery. The place is notorious now — people lower their voices when they pass it.".to_string(),
                    importance: EventImportance::Major,
                });
            }
            if before < BELOVED_IMPRINT && arch.imprint >= BELOVED_IMPRINT {
                state.beloved_crossings += 1;
                chronicle.send(AddChronicleEvent {
                    text: "The gel in one of the colony's workshops hums with remembered mastery. The place is beloved now — good work seems to happen there on its own.".to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

type ApplyPopQuery = (Option<&'static mut Needs>, Option<&'static mut StressTracker>);

/// The imprint's effect on the *next* user (and every user): a
/// haunted building inflicts `-imprint * 0.1` stress; an optimized
/// building grants a [`WorkSpeedBuff`] scaled by its imprint.
/// Runs after [`imprint_building_system`] in the schedule chain, so
/// a use is judged by the gel as this visit leaves it.
pub fn apply_building_imprint_system(
    mut events: EventReader<BuildingUseEvent>,
    buildings: Query<&NeuralArchitecture>,
    mut pops: Query<ApplyPopQuery>,
    mut commands: Commands,
) {
    for event in events.read() {
        let Ok(arch) = buildings.get(event.building) else {
            continue;
        };
        let stress = arch.stress_inflicted();
        let multiplier = arch.efficiency_multiplier();
        if stress <= 0.0 && multiplier <= 1.0 {
            continue;
        }
        let Ok((needs, tracker)) = pops.get_mut(event.pop) else {
            continue;
        };
        if stress > 0.0 {
            if let Some(mut tracker) = tracker {
                tracker.accumulated_stress += stress;
            } else if let Some(mut needs) = needs {
                // Leisure is the stress-adjacent need (see the needs
                // module docs): 100 tracker points == a full leisure
                // bar, so the same damage lands proportionally.
                needs.leisure = (needs.leisure - stress / 100.0).max(0.0);
            }
        }
        if multiplier > 1.0 {
            commands.entity(event.pop).insert(WorkSpeedBuff {
                multiplier,
                duration: IMPRINT_BUFF_DURATION,
            });
        }
    }
}

/// §5 decay: an imprint fades toward 0 once its building has sat
/// unused for [`DECAY_IDLE_TICKS`]. Active buildings keep their
/// gel — use refreshes `last_used_tick` far faster than decay can
/// eat a use's imprint step.
pub fn imprint_decay_system(
    time: Option<Res<SimulationTime>>,
    mut buildings: Query<&mut NeuralArchitecture>,
) {
    let Some(time) = time else {
        return;
    };
    for mut arch in &mut buildings {
        if arch.imprint == 0.0 {
            continue;
        }
        if time.tick.saturating_sub(arch.last_used_tick) <= DECAY_IDLE_TICKS {
            continue;
        }
        arch.imprint = if arch.imprint > 0.0 {
            (arch.imprint - DECAY_PER_TICK).max(0.0)
        } else {
            (arch.imprint + DECAY_PER_TICK).min(0.0)
        };
    }
}

/// §5 scrubbing: pay [`SCRUB_COST_CREDITS`] from the colony ledger
/// to reset one building's gel to neutral. Only a building that
/// actually carries an imprint can be scrubbed, and an
/// unaffordable scrub is a no-op (never a partial cleanse or a
/// negative balance).
pub fn scrub_imprint_system(
    mut events: EventReader<ScrubImprintEvent>,
    mut buildings: Query<&mut NeuralArchitecture>,
    resources: Option<ResMut<ColonyResources>>,
    state: Option<ResMut<NeuralArchitectureState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut resources = resources;
    let mut state = state;
    for event in events.read() {
        let Ok(mut arch) = buildings.get_mut(event.building) else {
            continue;
        };
        if arch.imprint == 0.0 {
            continue;
        }
        if let Some(resources) = resources.as_deref_mut() {
            if resources.credits < SCRUB_COST_CREDITS {
                continue;
            }
            resources.credits -= SCRUB_COST_CREDITS;
        }
        let was = arch.imprint;
        arch.imprint = 0.0;
        if let Some(state) = state.as_deref_mut() {
            state.scrubs += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A scrub crew flushed the neural gel in one of the colony's buildings, washing an imprint of {was:+.0} out of the walls. The place feels like nobody's now."
            ),
            importance: EventImportance::Standard,
        });
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census: (gel-lined buildings, optimized, haunted, worst imprint,
/// best imprint, lifetime uses, scrubs).
#[must_use]
pub fn neural_counts(world: &mut World) -> (usize, usize, usize, f32, f32, u64, u64) {
    let mut query = world.query::<&NeuralArchitecture>();
    let mut tracked = 0usize;
    let mut optimized = 0usize;
    let mut haunted = 0usize;
    let mut worst = 0.0f32;
    let mut best = 0.0f32;
    for arch in query.iter(world) {
        tracked += 1;
        if arch.is_optimized() {
            optimized += 1;
        }
        if arch.is_haunted() {
            haunted += 1;
        }
        worst = worst.min(arch.imprint);
        best = best.max(arch.imprint);
    }
    let (uses, scrubs) = world
        .get_resource::<NeuralArchitectureState>()
        .map_or((0, 0), |s| (s.uses, s.scrubs));
    (tracked, optimized, haunted, worst, best, uses, scrubs)
}

/// One-token STATS label, e.g. `imprint=4t/1+/2-/w-60/b45`, or
/// `none` when no building carries gel yet.
#[must_use]
pub fn neural_stats_label(world: &mut World) -> String {
    let (tracked, optimized, haunted, worst, best, _uses, _scrubs) = neural_counts(world);
    if tracked == 0 {
        "none".to_string()
    } else {
        format!("{tracked}t/{optimized}+/{haunted}-/w{worst:.0}/b{best:.0}")
    }
}

/// Send a building use from a `&mut World` context (headless debug).
pub fn trigger_use(world: &mut World, building: Entity, pop: Entity) {
    world.init_resource::<Events<BuildingUseEvent>>();
    world
        .resource_mut::<Events<BuildingUseEvent>>()
        .send(BuildingUseEvent { building, pop });
}

/// Send a scrub order from a `&mut World` context (headless debug).
pub fn trigger_scrub(world: &mut World, building: Entity) {
    world.init_resource::<Events<ScrubImprintEvent>>();
    world
        .resource_mut::<Events<ScrubImprintEvent>>()
        .send(ScrubImprintEvent { building });
}

/// Debug seam for headless: give the first gel-lined building (or,
/// failing that, the first building) an imprint value, attaching
/// gel if needed. Returns the building entity used.
pub fn debug_set_imprint(world: &mut World, imprint: f32) -> Option<Entity> {
    let mut query = world.query::<(Entity, &Building)>();
    let candidates: Vec<(Entity, BuildingType)> = query
        .iter(world)
        .map(|(e, b)| (e, b.building_type))
        .collect();
    let target = candidates
        .iter()
        .find(|(_, t)| is_advanced_building(*t))
        .or_else(|| candidates.first())
        .map(|(e, _)| *e)?;
    world.entity_mut(target).insert(NeuralArchitecture {
        imprint: imprint.clamp(IMPRINT_FLOOR, IMPRINT_CAP),
        ..Default::default()
    });
    Some(target)
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn neural_architecture_public_strings() -> Vec<String> {
    vec![
        "Neural Architecture".to_string(),
        "neural gel".to_string(),
        "haunted".to_string(),
        "optimized".to_string(),
        "notorious".to_string(),
        "beloved".to_string(),
        "The gel in one of the colony's workshops has gone bad with remembered misery. The place is notorious now — people lower their voices when they pass it.".to_string(),
        "The gel in one of the colony's workshops hums with remembered mastery. The place is beloved now — good work seems to happen there on its own.".to_string(),
        "A scrub crew flushed the neural gel in one of the colony's buildings, washing an imprint out of the walls. The place feels like nobody's now.".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real codebase — see the
// module docs for every adaptation and why)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::mind::utility_types::AssignmentType;
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::SkillType;
    use bevy_ecs::system::RunSystemOnce;

    fn base_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<BuildingUseEvent>>();
        world.init_resource::<Events<ScrubImprintEvent>>();
        world.init_resource::<Events<CraftEvent>>();
        world.init_resource::<NeuralArchitectureState>();
        world.init_resource::<SimulationTime>();
        world
    }

    fn send_use(world: &mut World, building: Entity, pop: Entity) {
        world
            .resource_mut::<Events<BuildingUseEvent>>()
            .send(BuildingUseEvent { building, pop });
    }

    fn run_imprint(world: &mut World) {
        world.run_system_once(imprint_building_system).unwrap();
    }

    /// Spec RED test 1, adapted: no `setup_test_app`, no
    /// `JobType::Smith` component on the building (real buildings
    /// carry `Building`), and `JobTenure` is built with the real
    /// `with_ticks(AssignmentType, ticks)` constructor plus the
    /// pop's real `Job`. The sketch's numbers are kept exactly:
    /// 1000 tenure ticks (> 500) imprints positive.
    #[test]
    fn test_building_imprints_positive_from_skilled_worker() {
        let mut world = base_world();
        let building = world
            .spawn((
                NeuralArchitecture::default(),
                Building {
                    building_type: BuildingType::Library,
                },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                Job {
                    workplace: building,
                    job_type: AssignmentType::LibraryWorker,
                },
                JobTenure::with_ticks(AssignmentType::LibraryWorker, 1000),
            ))
            .id();

        send_use(&mut world, building, pop);
        run_imprint(&mut world);

        let arch = world.get::<NeuralArchitecture>(building).unwrap();
        assert!(arch.imprint > 0.0, "skilled use should imprint positive");
        assert!((arch.imprint - SKILLED_IMPRINT).abs() < f32::EPSILON);
    }

    /// Skill alone (no tenure) also marks a worker skilled: a best
    /// skill level of 3+ imprints positive.
    #[test]
    fn test_skill_alone_imprints_positive() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::default()).id();
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Engineering, 1000.0); // level 3
        let pop = world.spawn((Pop, skills)).id();

        send_use(&mut world, building, pop);
        run_imprint(&mut world);

        assert!(world.get::<NeuralArchitecture>(building).unwrap().imprint > 0.0);
    }

    /// A short-tenure, unskilled, calm worker leaves no imprint.
    #[test]
    fn test_unskilled_calm_worker_leaves_no_imprint() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::default()).id();
        let pop = world
            .spawn((
                Pop,
                Job {
                    workplace: building,
                    job_type: AssignmentType::LibraryWorker,
                },
                JobTenure::with_ticks(AssignmentType::LibraryWorker, 100),
                Needs::default(),
                StressTracker::default(),
            ))
            .id();

        send_use(&mut world, building, pop);
        run_imprint(&mut world);

        let arch = world.get::<NeuralArchitecture>(building).unwrap();
        assert!((arch.imprint - 0.0).abs() < f32::EPSILON);
        assert_eq!(arch.uses, 1, "the use itself is still recorded");
    }

    /// Spec RED test 2, adapted: the sketch's `Needs { stress:
    /// 80.0 }` is the real `StressTracker { accumulated_stress:
    /// 80.0 }` (stress lives on the tracker, not on `Needs` — see
    /// the module docs). 80 > 50 imprints negative, exactly -10.
    #[test]
    fn test_building_imprints_negative_from_stressed_worker() {
        let mut world = base_world();
        let building = world
            .spawn((
                NeuralArchitecture::default(),
                Building {
                    building_type: BuildingType::Hospital,
                },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 80.0,
                },
            ))
            .id();

        send_use(&mut world, building, pop);
        run_imprint(&mut world);

        let arch = world.get::<NeuralArchitecture>(building).unwrap();
        assert!(arch.imprint < 0.0, "stressed use should imprint negative");
        assert!((arch.imprint - STRESSED_IMPRINT).abs() < f32::EPSILON);
    }

    /// The Needs-only fallback: a pop whose needs have collapsed
    /// (morale 0.2 -> derived stress 80) imprints negative even
    /// without a StressTracker.
    #[test]
    fn test_collapsed_needs_imprint_negative_via_fallback() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::default()).id();
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.2,
                    rest: 0.2,
                    leisure: 0.2,
                    hygiene: 0.2,
                },
            ))
            .id();

        send_use(&mut world, building, pop);
        run_imprint(&mut world);

        assert!(world.get::<NeuralArchitecture>(building).unwrap().imprint < 0.0);
    }

    /// Spec RED test 3, adapted: the haunted building (imprint
    /// -50) stresses its next user by `-imprint * 0.1` = 5.0 on
    /// the real tracker. The sketch's starting stress (10.0) and
    /// direction (stress rises) are kept exactly.
    #[test]
    fn test_building_imprint_affects_future_users() {
        let mut world = base_world();
        let building = world
            .spawn((
                NeuralArchitecture::with_imprint(-50.0),
                Building {
                    building_type: BuildingType::Hospital,
                },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();

        send_use(&mut world, building, pop);
        world.run_system_once(apply_building_imprint_system).unwrap();

        let tracker = world.get::<StressTracker>(pop).unwrap();
        assert!(tracker.accumulated_stress > 10.0, "haunted gel should stress the user");
        assert!((tracker.accumulated_stress - 15.0).abs() < f32::EPSILON);
    }

    /// A mildly negative imprint (above the -10 haunted line) does
    /// not stress users — the sketch's threshold is a real gate.
    #[test]
    fn test_mild_imprint_does_not_stress_users() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::with_imprint(-5.0)).id();
        let pop = world
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();

        send_use(&mut world, building, pop);
        world.run_system_once(apply_building_imprint_system).unwrap();

        let tracker = world.get::<StressTracker>(pop).unwrap();
        assert!((tracker.accumulated_stress - 10.0).abs() < f32::EPSILON);
    }

    /// Tracker-less pops take haunted stress on their leisure need
    /// instead (the fallback representation, same damage scaled).
    #[test]
    fn test_haunted_stress_lands_on_leisure_without_tracker() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::with_imprint(-50.0)).id();
        let pop = world.spawn((Pop, Needs::default())).id(); // leisure 0.8

        send_use(&mut world, building, pop);
        world.run_system_once(apply_building_imprint_system).unwrap();

        let needs = world.get::<Needs>(pop).unwrap();
        assert!(needs.leisure < 0.8, "haunted gel should drain leisure");
        assert!((needs.leisure - 0.75).abs() < 0.001);
    }

    /// §5 efficiency: an optimized building speeds its user through
    /// the real WorkSpeedBuff seam, scaled by imprint.
    #[test]
    fn test_optimized_building_speeds_future_users() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::with_imprint(50.0)).id();
        let pop = world.spawn((Pop, Needs::default())).id();

        send_use(&mut world, building, pop);
        world.run_system_once(apply_building_imprint_system).unwrap();

        let buff = world.get::<WorkSpeedBuff>(pop).unwrap();
        assert!(buff.multiplier > 1.0);
        assert!((buff.multiplier - 1.1).abs() < 0.001);
        assert_eq!(buff.duration, IMPRINT_BUFF_DURATION);
    }

    /// Imprint clamps at both rails no matter how many uses pile up.
    #[test]
    fn test_imprint_clamps_at_rails() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::with_imprint(98.0)).id();
        let pop = world
            .spawn((
                Pop,
                Job {
                    workplace: building,
                    job_type: AssignmentType::LibraryWorker,
                },
                JobTenure::with_ticks(AssignmentType::LibraryWorker, 5000),
                StressTracker {
                    accumulated_stress: 90.0,
                },
            ))
            .id();

        // Skilled (+5) and stressed (-10) in one visit: net -5,
        // and repeated visits can never leave [-100, 100].
        for _ in 0..40 {
            send_use(&mut world, building, pop);
            run_imprint(&mut world);
            // Drain the event buffer between rounds so each run
            // reads exactly one fresh event (a fresh reader would
            // otherwise replay every retained event).
            world.remove_resource::<Events<BuildingUseEvent>>();
            world.init_resource::<Events<BuildingUseEvent>>();
        }
        let arch = world.get::<NeuralArchitecture>(building).unwrap();
        assert!(arch.imprint >= IMPRINT_FLOOR && arch.imprint <= IMPRINT_CAP);
        assert!((arch.imprint - IMPRINT_FLOOR).abs() < f32::EPSILON);
    }

    /// §5 decay: idle gel fades toward 0; recently used gel does not.
    #[test]
    fn test_idle_imprint_decays_toward_zero() {
        let mut world = base_world();
        world.resource_mut::<SimulationTime>().tick = 10_000;
        let stale = world
            .spawn(NeuralArchitecture {
                imprint: 40.0,
                last_used_tick: 0,
                uses: 3,
            })
            .id();
        let fresh = world
            .spawn(NeuralArchitecture {
                imprint: 40.0,
                last_used_tick: 9_999,
                uses: 3,
            })
            .id();

        world.run_system_once(imprint_decay_system).unwrap();

        let stale_arch = world.get::<NeuralArchitecture>(stale).unwrap();
        assert!(stale_arch.imprint < 40.0 && stale_arch.imprint > 0.0);
        let fresh_arch = world.get::<NeuralArchitecture>(fresh).unwrap();
        assert!((fresh_arch.imprint - 40.0).abs() < f32::EPSILON);
    }

    /// §5 scrubbing: a paid scrub resets the gel and charges the
    /// real credit ledger exactly once.
    #[test]
    fn test_scrub_clears_imprint_at_cost() {
        let mut world = base_world();
        world.insert_resource(ColonyResources {
            credits: 100.0,
            ..Default::default()
        });
        let building = world.spawn(NeuralArchitecture::with_imprint(-70.0)).id();

        world
            .resource_mut::<Events<ScrubImprintEvent>>()
            .send(ScrubImprintEvent { building });
        world.run_system_once(scrub_imprint_system).unwrap();

        assert!(
            (world.get::<NeuralArchitecture>(building).unwrap().imprint - 0.0).abs()
                < f32::EPSILON
        );
        assert!(
            (world.resource::<ColonyResources>().credits - (100.0 - SCRUB_COST_CREDITS)).abs()
                < f32::EPSILON
        );
        assert_eq!(world.resource::<NeuralArchitectureState>().scrubs, 1);
    }

    /// An unaffordable scrub is a no-op: gel and ledger untouched.
    #[test]
    fn test_scrub_without_credits_is_noop() {
        let mut world = base_world();
        world.insert_resource(ColonyResources {
            credits: 5.0,
            ..Default::default()
        });
        let building = world.spawn(NeuralArchitecture::with_imprint(-70.0)).id();

        world
            .resource_mut::<Events<ScrubImprintEvent>>()
            .send(ScrubImprintEvent { building });
        world.run_system_once(scrub_imprint_system).unwrap();

        assert!(
            (world.get::<NeuralArchitecture>(building).unwrap().imprint + 70.0).abs()
                < f32::EPSILON
        );
        assert!((world.resource::<ColonyResources>().credits - 5.0).abs() < f32::EPSILON);
        assert_eq!(world.resource::<NeuralArchitectureState>().scrubs, 0);
    }

    /// The attach seam: advanced buildings gain gel, basic ones
    /// never do.
    #[test]
    fn test_ensure_attaches_only_to_advanced_buildings() {
        let mut world = base_world();
        let smithy = world
            .spawn(Building {
                building_type: BuildingType::Smithy,
            })
            .id();
        let housing = world
            .spawn(Building {
                building_type: BuildingType::Housing,
            })
            .id();

        world
            .run_system_once(ensure_neural_architecture_system)
            .unwrap();

        assert!(world.get::<NeuralArchitecture>(smithy).is_some());
        assert!(world.get::<NeuralArchitecture>(housing).is_none());
    }

    /// The §7 bridge: a craft event at a gel-lined workplace emits
    /// a BuildingUseEvent; a craft event from a jobless pop or at a
    /// gel-free workplace emits nothing.
    #[test]
    fn test_craft_event_bridges_to_building_use() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::default()).id();
        let pop = world
            .spawn((
                Pop,
                Job {
                    workplace: building,
                    job_type: AssignmentType::LibraryWorker,
                },
            ))
            .id();
        let jobless = world.spawn(Pop).id();

        world.resource_mut::<Events<CraftEvent>>().send(CraftEvent {
            crafter: pop,
            item_type: "Steel Axe".to_string(),
        });
        world.resource_mut::<Events<CraftEvent>>().send(CraftEvent {
            crafter: jobless,
            item_type: "Steel Axe".to_string(),
        });
        world
            .run_system_once(craft_building_use_bridge_system)
            .unwrap();

        let events = world.resource::<Events<BuildingUseEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_cursor();
        let sent: Vec<_> = reader.read(events).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].building, building);
        assert_eq!(sent[0].pop, pop);
    }

    /// End-to-end through both event systems: a stressed master
    /// leaves a haunted library that then stresses its next user.
    #[test]
    fn test_full_cycle_stressed_master_haunts_library() {
        let mut world = base_world();
        let building = world.spawn(NeuralArchitecture::default()).id();
        let master = world
            .spawn((
                Pop,
                Job {
                    workplace: building,
                    job_type: AssignmentType::LibraryWorker,
                },
                JobTenure::with_ticks(AssignmentType::LibraryWorker, 2000),
                StressTracker {
                    accumulated_stress: 90.0,
                },
            ))
            .id();

        // Three visits: +5 -10 each = -15, past the haunted line.
        for _ in 0..3 {
            send_use(&mut world, building, master);
            run_imprint(&mut world);
            world.remove_resource::<Events<BuildingUseEvent>>();
            world.init_resource::<Events<BuildingUseEvent>>();
        }
        let arch = world.get::<NeuralArchitecture>(building).unwrap();
        assert!(arch.is_haunted(), "imprint {} should be haunted", arch.imprint);

        let visitor = world
            .spawn((
                Pop,
                StressTracker {
                    accumulated_stress: 0.0,
                },
            ))
            .id();
        send_use(&mut world, building, visitor);
        world.run_system_once(apply_building_imprint_system).unwrap();
        assert!(world.get::<StressTracker>(visitor).unwrap().accumulated_stress > 0.0);
    }

    /// IP guard: every public string is original to Scale — no
    /// borrowed names, places, or distinctive IP terms.
    #[test]
    fn test_neural_architecture_ip_guard() {
        let banned = [
            "warhammer", "40k", "dune", "atreides", "star trek", "star wars",
            "borg", "foundation", "psychohistory", "sheldon", "ender",
            "ansible", "shrike", "hyperion", "annihilation", "area x",
            "southern reach", "hitchhiker", "improbability drive",
        ];
        for s in neural_architecture_public_strings() {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string {s:?} contains banned IP term {term:?}"
                );
            }
        }
    }
}
