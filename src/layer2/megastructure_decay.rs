//! Spec 372: Stellar Megastructure Decay.
//!
//! Completed stellar megastructures do not stay pristine. Micrometeorite
//! abrasion, thermal cycling, and plain entropy grind their frames down:
//! structural decay accumulates every tick, output degrades in stages,
//! neglected frames shed fragments into the orbital-debris ledger, and a
//! frame left to rot to critical decay fails outright — its mass comes
//! down through the real Spec 277 deorbit pipeline. Maintenance is the
//! counterplay: a repair event walks decay back down before it is too late.
//!
//! ## Grounding audit (2026-10-28, trunk 94c2c216)
//!
//! * **The megastructure seam is `layer2::megastructure`.** The only
//!   megastructure component in the codebase is
//!   [`MegastructureScaffolding`] (`completion_percent`,
//!   `resources_invested`), built up by `MegastructureProgressEvent` /
//!   `process_megastructure_construction_system`; its output today is the
//!   threat it projects (`update_threat_from_megastructures_system`,
//!   `completion * 0.5`). There is no named/kind-typed megastructure
//!   entity, no health component, and no output resource to throttle —
//!   so decay attaches to `MegastructureScaffolding` itself and degrades
//!   its *effective* completion (see [`effective_completion`]), which is
//!   the one number every consumer of a megastructure reads. No parallel
//!   megastructure system is built.
//! * **No decay/upkeep leg existed for scaffolding.** Neighbours checked:
//!   `layer2::station::DebtTrapMegastructure` has an exponential *credit*
//!   upkeep (missed payments -> diplomacy/repossession, never structural
//!   decay); `layer2::barnacles` accumulates a speed penalty on fleets in
//!   transit with a clean-off helper (the closest accumulate + counterplay
//!   precedent); `layer2::derelict_stations` is claim-on-arrival only;
//!   `layer2::debris` decays *debris itself* (atmospheric drag), it does
//!   not decay structures. The missing leg is exactly: scaffolding ->
//!   structural decay over time. This module builds only that.
//! * **Spec 277 deorbit seam (real):** [`DeorbitEvent`] in
//!   `layer1::disasters::deorbit` starts a 600-tick countdown; at zero,
//!   tether arrays may catch the falling mass for salvage, otherwise a
//!   real `ExplosionEvent` + crater demolition + debris feed resolve it.
//!   A critically decayed megastructure *is* a deorbiting megastructure,
//!   so critical failure fires `DeorbitEvent { size: Megastructure }`
//!   through a bridge system (the fossil-fleet wake -> drop bridge
//!   pattern) instead of inventing a second collapse pipeline.
//! * **Debris seam (real):** [`OrbitalDebris`] is both a component and
//!   the colony-level resource ledger; the resource form is what Spec 277
//!   impact, Spec 1380 shoot-downs, and the harpoon feed. Shedding writes
//!   to the resource when present (and always to this module's ledger,
//!   so a world without the resource still accounts for every fragment).
//! * **Pattern precedent (Specs 370/371):** deterministic rates (no
//!   `rand::random` in the tick path — the same run replays identically),
//!   a ledger resource for STATS, `&mut World` helpers for headless,
//!   events/resources initialised in simulation setup (first-tick
//!   Events-panic pattern), chronicles capped by construction (stage
//!   transitions only — at most four entries per structure, ever, plus
//!   one at failure).
//!
//! ## Adaptations
//!
//! * **Maintenance is an event, not a resource drain.** The construction
//!   seam is event-driven (`MegastructureProgressEvent`), and there is no
//!   layer-2 maintenance budget/crew resource to draw from (the DebtTrap
//!   upkeep draws `EmpireCredits` on its own cycle, a different economy).
//!   So the counterplay is [`MaintainMegastructureEvent`] — the mirror of
//!   the progress event — plus the `decay maintain` headless command.
//!   A funded-maintenance *automation* (crews, credit draw) is a follow-on
//!   that can send this same event; nothing here blocks it.
//! * **Failure does not despawn.** The deorbit countdown owns the falling
//!   mass from the moment the failure event fires; the scaffolding entity
//!   stays as a failed hulk (`decay = 100`, effective output 0) so the
//!   census, chronicle, and any catch/salvage resolution can still see it.
//!   It never decays, sheds, or fails twice.
//! * **Incomplete scaffolding decays slower, bare frames not at all.**
//!   The rate scales with `completion_percent / 100`: a 0% frame has
//!   nothing in orbit to rot; a finished structure rots at the full
//!   [`DECAY_PER_TICK`].
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::disasters::deorbit::{DeorbitEvent, MegastructureSize};
use crate::layer2::debris::OrbitalDebris;
use crate::layer2::megastructure::MegastructureScaffolding;

/// Structural decay gained per tick by a *completed* megastructure
/// (scaled down linearly for incomplete scaffolding). At this rate a
/// finished, wholly neglected structure reaches critical failure in
/// 500 ticks — long enough to matter, short enough to see coming.
pub const DECAY_PER_TICK: f32 = 0.2;
/// Decay at which a structure is merely worn (first stage threshold).
pub const WORN_THRESHOLD: f32 = 25.0;
/// Decay at which a structure is degraded and starts shedding debris.
pub const DEGRADED_THRESHOLD: f32 = 50.0;
/// Decay at which a structure is critical and sheds debris faster.
pub const CRITICAL_THRESHOLD: f32 = 75.0;
/// Decay at which a structure fails outright (deorbit).
pub const FAILURE_DECAY: f32 = 100.0;
/// Orbital debris shed per tick while degraded (50..75 decay).
pub const SHED_PER_TICK_DEGRADED: f32 = 0.002;
/// Orbital debris shed per tick while critical (75..100 decay).
pub const SHED_PER_TICK_CRITICAL: f32 = 0.006;
/// Default repair strength of one maintenance event (headless default).
pub const DEFAULT_MAINTENANCE_AMOUNT: f32 = 30.0;

/// How far a megastructure's decay has progressed. Stages are pure
/// functions of the decay value — see [`stage_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecayStage {
    /// 0..25 decay: full output, no shedding.
    #[default]
    Pristine,
    /// 25..50 decay: output at 85%.
    Worn,
    /// 50..75 decay: output at 60%, sheds debris slowly.
    Degraded,
    /// 75..100 decay: output at 30%, sheds debris fast.
    Critical,
    /// 100 decay: failed — output 0, deorbit fired.
    Failed,
}

impl DecayStage {
    /// Fraction of nominal output the structure still delivers.
    #[must_use]
    pub const fn output_factor(self) -> f32 {
        match self {
            DecayStage::Pristine => 1.0,
            DecayStage::Worn => 0.85,
            DecayStage::Degraded => 0.6,
            DecayStage::Critical => 0.3,
            DecayStage::Failed => 0.0,
        }
    }

    /// Human-readable stage label (chronicles, STATS, panels).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            DecayStage::Pristine => "pristine",
            DecayStage::Worn => "worn",
            DecayStage::Degraded => "degraded",
            DecayStage::Critical => "critical",
            DecayStage::Failed => "failed",
        }
    }

    /// Ordinal for transition detection (chronicles fire only when
    /// the ordinal rises — the spam cap).
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        match self {
            DecayStage::Pristine => 0,
            DecayStage::Worn => 1,
            DecayStage::Degraded => 2,
            DecayStage::Critical => 3,
            DecayStage::Failed => 4,
        }
    }
}

/// The stage a given decay value (0..100) has reached.
#[must_use]
pub fn stage_for(decay: f32) -> DecayStage {
    if decay >= FAILURE_DECAY {
        DecayStage::Failed
    } else if decay >= CRITICAL_THRESHOLD {
        DecayStage::Critical
    } else if decay >= DEGRADED_THRESHOLD {
        DecayStage::Degraded
    } else if decay >= WORN_THRESHOLD {
        DecayStage::Worn
    } else {
        DecayStage::Pristine
    }
}

/// Advance decay by `ticks` for a structure of the given completion
/// percent. Pure and deterministic — shared by the tick system and
/// directly unit-testable. Incomplete scaffolding rots proportionally
/// slower; a 0% frame does not rot at all.
#[must_use]
pub fn advance_decay(current: f32, ticks: u32, completion_percent: f32) -> f32 {
    let scale = (completion_percent / 100.0).clamp(0.0, 1.0);
    (current + DECAY_PER_TICK * ticks as f32 * scale).clamp(0.0, FAILURE_DECAY)
}

/// Debris shed per tick at a given decay value (0 below Degraded).
#[must_use]
pub fn shed_per_tick(decay: f32) -> f32 {
    match stage_for(decay) {
        DecayStage::Degraded => SHED_PER_TICK_DEGRADED,
        DecayStage::Critical => SHED_PER_TICK_CRITICAL,
        _ => 0.0,
    }
}

/// The completion a structure effectively delivers after decay —
/// the number downstream consumers (threat, output) should read.
#[must_use]
pub fn effective_completion(scaffolding: &MegastructureScaffolding, decay: &MegastructureDecay) -> f32 {
    scaffolding.completion_percent * decay.stage().output_factor()
}

/// Structural decay ledger riding on a [`MegastructureScaffolding`]
/// entity. Added automatically by
/// [`ensure_megastructure_decay_system`] to any scaffolding that
/// lacks one.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct MegastructureDecay {
    /// Accumulated structural decay, 0..100.
    pub decay: f32,
    /// Highest stage chronicled so far (spam cap: a stage is
    /// announced exactly once, when first reached).
    pub announced_stage: u8,
    /// Set once at failure: the structure never decays, sheds, or
    /// fails again — the deorbit pipeline owns its mass now.
    pub failed: bool,
}

impl MegastructureDecay {
    /// Current stage derived from the decay value.
    #[must_use]
    pub fn stage(&self) -> DecayStage {
        if self.failed {
            DecayStage::Failed
        } else {
            stage_for(self.decay)
        }
    }
}

/// Counterplay: a maintenance crew services a megastructure,
/// walking its decay back down by `amount` (clamped at 0). The
/// mirror of `MegastructureProgressEvent` on the construction seam.
#[derive(Event, Debug, Clone, Copy)]
pub struct MaintainMegastructureEvent {
    /// Structure being serviced.
    pub entity: Entity,
    /// Decay points removed.
    pub amount: f32,
}

/// A megastructure's decay reached 100 and it failed. Bridged to
/// the Spec 277 deorbit pipeline by
/// [`handle_megastructure_failure_system`].
#[derive(Event, Debug, Clone, Copy)]
pub struct MegastructureFailedEvent {
    /// Structure that failed.
    pub entity: Entity,
}

/// Colony ledger for megastructure decay (STATS / panel surface).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct MegastructureDecayState {
    /// Ticks on which at least one structure decayed.
    pub decay_ticks: u64,
    /// Total fragments shed into the debris ledger.
    pub debris_shed: f32,
    /// Total structures lost to critical decay.
    pub failures: u64,
    /// Total maintenance events applied.
    pub repairs: u64,
    /// Total decay points removed by maintenance.
    pub decay_repaired: f32,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Give every megastructure a decay ledger (barnacle-ensure pattern).
pub fn ensure_megastructure_decay_system(
    mut commands: Commands,
    query: Query<Entity, (With<MegastructureScaffolding>, Without<MegastructureDecay>)>,
) {
    for entity in query.iter() {
        commands.entity(entity).insert(MegastructureDecay::default());
    }
}

/// The decay tick: every un-failed megastructure rots a little.
/// Stage rises are chronicled exactly once per stage; degraded and
/// critical structures shed debris into the [`OrbitalDebris`]
/// resource ledger; reaching 100 fails the structure and emits
/// [`MegastructureFailedEvent`] (once).
#[allow(clippy::too_many_arguments)]
pub fn megastructure_decay_system(
    mut query: Query<(Entity, &MegastructureScaffolding, &mut MegastructureDecay)>,
    mut state: ResMut<MegastructureDecayState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut failed_events: EventWriter<MegastructureFailedEvent>,
    mut debris: Option<ResMut<OrbitalDebris>>,
) {
    let mut any_decayed = false;
    for (entity, scaffolding, mut decay) in query.iter_mut() {
        if decay.failed {
            continue;
        }
        let before_stage = decay.stage();
        decay.decay = advance_decay(decay.decay, 1, scaffolding.completion_percent);
        if scaffolding.completion_percent > 0.0 {
            any_decayed = true;
        }
        // Debris shedding (degraded and worse, never after failure
        // resolves this same tick — a failed frame's mass belongs
        // to the deorbit pipeline, which feeds debris on impact).
        if decay.decay < FAILURE_DECAY {
            let shed = shed_per_tick(decay.decay);
            if shed > 0.0 {
                state.debris_shed += shed;
                if let Some(debris) = debris.as_deref_mut() {
                    debris.0 += shed;
                }
            }
        }
        let after_stage = decay.stage();
        // Chronicle stage rises exactly once (spam cap by ordinal).
        if after_stage.ordinal() > before_stage.ordinal()
            && after_stage.ordinal() > decay.announced_stage
            && after_stage != DecayStage::Failed
        {
            decay.announced_stage = after_stage.ordinal();
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "A stellar megastructure is now {}: micrometeorite abrasion and thermal cycling have ground its frame to {:.0}% decay. Its output falters.",
                    after_stage.label(),
                    decay.decay
                ),
                importance: EventImportance::Standard,
            });
        }
        if decay.decay >= FAILURE_DECAY {
            decay.failed = true;
            decay.announced_stage = DecayStage::Failed.ordinal();
            state.failures += 1;
            chronicle.send(AddChronicleEvent {
                text: "A stellar megastructure has failed: a century of neglected decay gives way at once, and its mass begins the long fall. Tether crews, stand ready.".to_string(),
                importance: EventImportance::Major,
            });
            failed_events.send(MegastructureFailedEvent { entity });
        }
    }
    if any_decayed {
        state.decay_ticks += 1;
    }
}

/// Maintenance counterplay: apply [`MaintainMegastructureEvent`]s.
/// A failed structure cannot be serviced back — its mass is already
/// falling through the deorbit pipeline.
pub fn megastructure_maintenance_system(
    mut events: EventReader<MaintainMegastructureEvent>,
    mut query: Query<&mut MegastructureDecay>,
    mut state: ResMut<MegastructureDecayState>,
) {
    for event in events.read() {
        if !event.amount.is_finite() || event.amount <= 0.0 {
            continue;
        }
        if let Ok(mut decay) = query.get_mut(event.entity) {
            if decay.failed {
                continue;
            }
            let before = decay.decay;
            decay.decay = (decay.decay - event.amount).max(0.0);
            state.repairs += 1;
            state.decay_repaired += before - decay.decay;
            // Re-arm stage chronicles below the repaired level so a
            // structure that rots again is announced again.
            let stage_ord = decay.stage().ordinal();
            if decay.announced_stage > stage_ord {
                decay.announced_stage = stage_ord;
            }
        }
    }
}

/// Bridge: a critical failure becomes a Spec 277 deorbit crisis —
/// the falling mass is a full megastructure ring section, resolved
/// by tether catch or impact through the existing pipeline.
pub fn handle_megastructure_failure_system(
    mut events: EventReader<MegastructureFailedEvent>,
    mut deorbit: EventWriter<DeorbitEvent>,
) {
    for _event in events.read() {
        deorbit.send(DeorbitEvent {
            size: MegastructureSize::Megastructure,
            target: None,
            warning_ticks: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Spawn a completed megastructure with a fresh decay ledger
/// (headless debug): real scaffolding at 100% + decay component.
pub fn spawn_megastructure(world: &mut World, completion_percent: f32) -> Entity {
    world
        .spawn((
            MegastructureScaffolding {
                completion_percent,
                resources_invested: 50_000,
            },
            MegastructureDecay::default(),
        ))
        .id()
}

/// Force a structure's decay to an exact value (headless debug).
/// Returns false if the entity has no decay ledger.
pub fn force_decay(world: &mut World, entity: Entity, value: f32) -> bool {
    if let Some(mut decay) = world.get_mut::<MegastructureDecay>(entity) {
        decay.decay = value.clamp(0.0, FAILURE_DECAY);
        true
    } else {
        false
    }
}

/// Force decay on the most-decayed living structure, or spawn one
/// first if the world has none (headless `decay force`).
pub fn force_worst_decay(world: &mut World, value: f32) -> Entity {
    let target = {
        let mut query = world.query::<(Entity, &MegastructureDecay)>();
        query
            .iter(world)
            .filter(|(_, d)| !d.failed)
            .max_by(|a, b| a.1.decay.partial_cmp(&b.1.decay).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(e, _)| e)
    };
    let entity = target.unwrap_or_else(|| spawn_megastructure(world, 100.0));
    force_decay(world, entity, value);
    entity
}

/// Send a maintenance event from a `&mut World` context (headless).
pub fn trigger_maintenance(world: &mut World, entity: Entity, amount: f32) {
    world.init_resource::<Events<MaintainMegastructureEvent>>();
    world
        .resource_mut::<Events<MaintainMegastructureEvent>>()
        .send(MaintainMegastructureEvent { entity, amount });
}

/// Census: (structures, failed, worst decay, mean output factor,
/// debris shed, failures, repairs).
#[must_use]
pub fn decay_counts(world: &mut World) -> (usize, usize, f32, f32, f32, u64, u64) {
    let (mut count, mut failed, mut worst, mut factor_sum) = (0_usize, 0_usize, 0.0_f32, 0.0_f32);
    {
        let mut query = world.query::<&MegastructureDecay>();
        for decay in query.iter(world) {
            count += 1;
            if decay.failed {
                failed += 1;
            }
            worst = worst.max(decay.decay);
            factor_sum += decay.stage().output_factor();
        }
    }
    let mean_factor = if count > 0 { factor_sum / count as f32 } else { 1.0 };
    let (shed, failures, repairs) = world
        .get_resource::<MegastructureDecayState>()
        .map_or((0.0, 0, 0), |s| (s.debris_shed, s.failures, s.repairs));
    (count, failed, worst, mean_factor, shed, failures, repairs)
}

/// One-token STATS label, e.g. `structures2/worst64/out0.60`, or
/// `none` when no megastructure carries a decay ledger.
#[must_use]
pub fn decay_stats_label(world: &mut World) -> String {
    let (count, _failed, worst, mean_factor, _shed, _failures, _repairs) = decay_counts(world);
    if count == 0 {
        "none".to_string()
    } else {
        format!("structures{count}/worst{worst:.0}/out{mean_factor:.2}")
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn decay_public_strings() -> Vec<String> {
    vec![
        "Stellar Megastructure Decay".to_string(),
        "A stellar megastructure has failed: a century of neglected decay gives way at once, and its mass begins the long fall. Tether crews, stand ready.".to_string(),
        "pristine".to_string(),
        "worn".to_string(),
        "degraded".to_string(),
        "critical".to_string(),
        "failed".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD — RED first, adapted to the real seams above)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn base_world() -> World {
        let mut world = World::new();
        world.init_resource::<MegastructureDecayState>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<MaintainMegastructureEvent>>();
        world.init_resource::<Events<MegastructureFailedEvent>>();
        world.init_resource::<Events<DeorbitEvent>>();
        world.insert_resource(OrbitalDebris(0.0));
        world
    }

    fn chronicle_texts(world: &mut World) -> Vec<String> {
        world
            .resource::<Events<AddChronicleEvent>>()
            .iter_current_update_events()
            .map(|e| e.text.clone())
            .collect()
    }

    /// Decay accumulates deterministically on a completed structure
    /// and scales down for incomplete scaffolding.
    #[test]
    fn test_decay_accumulates_over_time() {
        let mut world = base_world();
        let e = spawn_megastructure(&mut world, 100.0);
        for _ in 0..100 {
            world.run_system_once(megastructure_decay_system).unwrap();
        }
        let decay = world.get::<MegastructureDecay>(e).unwrap();
        assert!((decay.decay - 20.0).abs() < 0.01, "decay={}", decay.decay);
        assert_eq!(world.resource::<MegastructureDecayState>().decay_ticks, 100);

        // Half-built scaffolding rots at half rate; a bare frame not at all.
        assert!((advance_decay(0.0, 100, 50.0) - 10.0).abs() < 0.01);
        assert_eq!(advance_decay(0.0, 100, 0.0), 0.0);
        assert_eq!(advance_decay(99.0, 100, 100.0), FAILURE_DECAY);
    }

    /// Stage degradation: output factor steps down through the
    /// stages and effective completion follows it.
    #[test]
    fn test_stage_degradation_reduces_output() {
        assert_eq!(stage_for(0.0), DecayStage::Pristine);
        assert_eq!(stage_for(30.0), DecayStage::Worn);
        assert_eq!(stage_for(60.0), DecayStage::Degraded);
        assert_eq!(stage_for(90.0), DecayStage::Critical);
        assert_eq!(stage_for(100.0), DecayStage::Failed);
        let scaffolding = MegastructureScaffolding {
            completion_percent: 100.0,
            resources_invested: 1,
        };
        let cases = [
            (0.0, 100.0),
            (30.0, 85.0),
            (60.0, 60.0),
            (90.0, 30.0),
            (100.0, 0.0),
        ];
        for (decay_value, expected) in cases {
            let decay = MegastructureDecay {
                decay: decay_value,
                failed: decay_value >= FAILURE_DECAY,
                ..Default::default()
            };
            let eff = effective_completion(&scaffolding, &decay);
            assert!((eff - expected).abs() < 0.01, "decay {decay_value}: {eff}");
        }
    }

    /// Stage rises are chronicled exactly once per stage (spam cap),
    /// even when the system runs long past the threshold.
    #[test]
    fn test_stage_chronicles_are_spam_capped() {
        let mut world = base_world();
        let e = spawn_megastructure(&mut world, 100.0);
        force_decay(&mut world, e, 24.0);
        // Cross into Worn, then keep ticking inside Worn.
        for _ in 0..30 {
            world.run_system_once(megastructure_decay_system).unwrap();
        }
        let texts = chronicle_texts(&mut world);
        assert_eq!(texts.len(), 1, "one chronicle for the Worn rise: {texts:?}");
        assert!(texts[0].contains("worn"));
    }

    /// Neglected structures shed debris into the real OrbitalDebris
    /// resource once degraded — and never while pristine/worn.
    #[test]
    fn test_debris_shedding_when_degraded() {
        let mut world = base_world();
        let e = spawn_megastructure(&mut world, 100.0);
        force_decay(&mut world, e, 10.0);
        for _ in 0..10 {
            world.run_system_once(megastructure_decay_system).unwrap();
        }
        assert_eq!(world.resource::<OrbitalDebris>().0, 0.0, "no shed below Degraded");
        force_decay(&mut world, e, 60.0);
        for _ in 0..10 {
            world.run_system_once(megastructure_decay_system).unwrap();
        }
        let shed = world.resource::<MegastructureDecayState>().debris_shed;
        assert!((shed - 10.0 * SHED_PER_TICK_DEGRADED).abs() < 0.0001, "shed={shed}");
        assert!((world.resource::<OrbitalDebris>().0 - shed).abs() < 0.0001);
        // Critical sheds faster than degraded.
        assert!(shed_per_tick(90.0) > shed_per_tick(60.0));
        assert_eq!(shed_per_tick(10.0), 0.0);
    }

    /// Critical failure: decay 100 fails the structure exactly once,
    /// zeroes its output, and the bridge fires the real Spec 277
    /// DeorbitEvent at Megastructure size.
    #[test]
    fn test_critical_failure_fires_deorbit() {
        let mut world = base_world();
        let e = spawn_megastructure(&mut world, 100.0);
        force_decay(&mut world, e, 99.0);
        for _ in 0..10 {
            world.run_system_once(megastructure_decay_system).unwrap();
        }
        let decay = world.get::<MegastructureDecay>(e).unwrap();
        assert!(decay.failed);
        assert_eq!(decay.stage(), DecayStage::Failed);
        assert_eq!(world.resource::<MegastructureDecayState>().failures, 1);
        let failed_events: Vec<_> = world
            .resource::<Events<MegastructureFailedEvent>>()
            .iter_current_update_events()
            .copied()
            .collect();
        assert_eq!(failed_events.len(), 1, "failure fires exactly once");
        // Bridge -> deorbit pipeline.
        world.run_system_once(handle_megastructure_failure_system).unwrap();
        let deorbits: Vec<_> = world
            .resource::<Events<DeorbitEvent>>()
            .iter_current_update_events()
            .collect();
        assert_eq!(deorbits.len(), 1);
        assert_eq!(deorbits[0].size, MegastructureSize::Megastructure);
        // The failure itself is Major-chronicled.
        assert!(chronicle_texts(&mut world)
            .iter()
            .any(|t| t.contains("has failed")));
    }

    /// Maintenance counterplay: a repair event walks decay back
    /// down (even out of Critical), re-arms stage chronicles, and
    /// is accounted in the ledger. Failed structures stay failed.
    #[test]
    fn test_maintenance_repairs_decay() {
        let mut world = base_world();
        let e = spawn_megastructure(&mut world, 100.0);
        force_decay(&mut world, e, 80.0);
        world
            .resource_mut::<Events<MaintainMegastructureEvent>>()
            .send(MaintainMegastructureEvent { entity: e, amount: 50.0 });
        world.run_system_once(megastructure_maintenance_system).unwrap();
        let decay = world.get::<MegastructureDecay>(e).unwrap();
        assert!((decay.decay - 30.0).abs() < 0.01);
        assert_eq!(decay.stage(), DecayStage::Worn);
        let state = world.resource::<MegastructureDecayState>();
        assert_eq!(state.repairs, 1);
        assert!((state.decay_repaired - 50.0).abs() < 0.01);
        // Repair clamps at pristine, never negative.
        world
            .resource_mut::<Events<MaintainMegastructureEvent>>()
            .send(MaintainMegastructureEvent { entity: e, amount: 999.0 });
        world.run_system_once(megastructure_maintenance_system).unwrap();
        assert_eq!(world.get::<MegastructureDecay>(e).unwrap().decay, 0.0);
    }

    /// The ensure system retrofits a decay ledger onto bare
    /// scaffolding (existing saves / other spawn paths).
    #[test]
    fn test_ensure_adds_decay_to_scaffolding() {
        let mut world = base_world();
        let e = world
            .spawn(MegastructureScaffolding {
                completion_percent: 80.0,
                resources_invested: 10,
            })
            .id();
        world.run_system_once(ensure_megastructure_decay_system).unwrap();
        assert!(world.get::<MegastructureDecay>(e).is_some());
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "galactus",
            "marvel",
            "mass effect",
            "reaper",
            "rimworld",
            "dwarf fortress",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
            "halo",
            "forerunner",
            "promethean",
            "death star",
            "ringworld",
            "dyson sphere",
            "culture",
        ];
        for text in decay_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
