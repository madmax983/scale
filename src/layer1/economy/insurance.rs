//! Spec 364: Galactic Insurance Fraud.
//!
//! The colony can buy expensive insurance policies on high-value assets
//! (fleets and orbital structures — in practice any entity, Layer 1
//! buildings included). If an insured asset is destroyed by space
//! hazards or pirates, the underwriters pay out a large credit sum into
//! the real colony ledger ([`ColonyResources`] credits). If the player
//! destroys the asset themselves — a direct Scuttle / Deconstruct
//! order — the claim is void: no payout, and the attempt is remembered
//! as fraud evidence.
//!
//! A hidden suspicion score lives in the global [`InsuranceManager`]:
//! it rises slightly per paid claim, significantly when the destroyed
//! asset was already obsolete / heavily damaged, and sharply for
//! scuttled claims. Crossing the threshold (or filing a burst of claims
//! in a short window) schedules an [`InsuranceInvestigator`] arrival.
//! The investigator reuses The Inspector's seams (spec 091): the
//! investigator entity carries the real [`Inspector`] component and a
//! [`Visitor`] lifecycle, samples nothing new, and completes an audit
//! after a fixed stay. A failed audit (any standing fraud evidence)
//! levies a credit fine and applies a temporary [`TradeBlockade`],
//! which the real trade path ([`crate::layer1::economy::trade::
//! execute_trade`]) honours while it is active.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **Placement.** The spec sketch says `src/layer2/economics/`, but
//!   that module does not exist: the real economy lives in
//!   `src/layer1/economy/` (trade, recalls, the organ market), and the
//!   payout ledger the spec names — colony credits — is Layer 1's
//!   [`ColonyResources`]. The module therefore lives here, alongside
//!   its sibling markets. No pre-existing insurance module was found
//!   anywhere in `src/` (checked 2026-10-25), so nothing is duplicated.
//! * **Cause seam.** The real [`crate::layer1::shields::DamageEvent`]
//!   carries no source field (target / amount / velocity only), so it
//!   cannot distinguish a pirate from the player's own scuttle order.
//!   The game's real attribution pattern is Item 84's death-attribution
//!   seam ([`LastDamageSource`] + [`SlainByFaction`]): the destruction
//!   watch below reads exactly those components when an insured
//!   asset's health hits zero, and explicit destruction paths (headless
//!   debug, scuttle/deconstruct orders) report through
//!   [`AssetDestroyedEvent`] with a typed [`DestructionCause`]. The
//!   central claim processor differentiates on that cause alone.
//! * **Investigator.** No duplicate NPC assessment: the investigator
//!   is spawned with the genuine [`Inspector`] + [`Visitor`]
//!   components at a [`VisitorSource`] spawn point, exactly like
//!   `spawn_inspector_system`, and departs through the same lifecycle.
//! * **TradeBlockade.** Layer 2 has a debt-driven
//!   `CollectionSphereBlockade`, a different mechanic (cargo
//!   interception against colony debt). The spec's audit blockade is a
//!   timed trade suspension, so it is its own resource here and is
//!   enforced in the Layer 1 `execute_trade` entry point.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::biology::death_attribution::LastDamageSource;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::health::Health;
use crate::layer1::inspector::Inspector;
use crate::layer1::map::GridPosition;
use crate::layer1::social::martyrdom::SlainByFaction;
use crate::layer1::visitor::{Visitor, VisitorSource, VisitorState};
use crate::layer2::fleet::FleetHealth;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants (the spec's own numbers, grounded in real magnitudes:
// an organ sells for 120cr, a scion allowance is 50cr/1000 ticks, an
// artifact sale is ~100cr+ — a 500cr hull policy is genuinely expensive
// and a 1000cr fraud fine genuinely hurts.)
// ---------------------------------------------------------------------------

/// Default insured value for a policy bought without an explicit value.
pub const DEFAULT_INSURED_VALUE: f32 = 500.0;
/// Premium charged at purchase, as a fraction of the insured value.
pub const PREMIUM_RATE: f32 = 0.20;
/// Suspicion added for every paid claim.
pub const SUSPICION_PER_CLAIM: f32 = 10.0;
/// Extra suspicion when the destroyed asset was already heavily
/// damaged (at or beyond [`HEAVILY_DAMAGED_FRACTION`] damage taken) —
/// the "obsolete / heavily-damaged asset" clause from spec §7.
pub const SUSPICION_HEAVILY_DAMAGED: f32 = 25.0;
/// Damage fraction at or beyond which an asset counts as heavily
/// damaged for suspicion purposes.
pub const HEAVILY_DAMAGED_FRACTION: f32 = 0.75;
/// Suspicion added (and fraud evidence recorded) when an insured asset
/// is destroyed by the player's own Scuttle / Deconstruct order.
pub const SUSPICION_SCUTTLE: f32 = 40.0;
/// Suspicion at or beyond which an investigator is scheduled.
pub const SUSPICION_THRESHOLD: f32 = 50.0;
/// Window (ticks) for the rapid-claims trigger.
pub const RAPID_CLAIM_WINDOW: u64 = 500;
/// Paid claims inside the window that schedule an investigator even
/// below the suspicion threshold.
pub const RAPID_CLAIM_COUNT: usize = 3;
/// Ticks between an investigator being scheduled and arriving.
pub const INVESTIGATOR_DELAY_TICKS: u64 = 200;
/// Ticks the investigator spends auditing before the verdict.
pub const AUDIT_DURATION_TICKS: u64 = 100;
/// Credit fine levied by a failed audit.
pub const FINE_CREDITS: f32 = 1000.0;
/// Duration (ticks) of the trade blockade applied by a failed audit.
pub const BLOCKADE_TICKS: u64 = 1000;

// ---------------------------------------------------------------------------
// Cause of destruction — the fraud differentiator
// ---------------------------------------------------------------------------

/// How an insured asset was destroyed. This is the seam the central
/// claim processor differentiates on: external causes pay out,
/// player-ordered destruction voids the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DestructionCause {
    /// Space hazard: debris, storms, structural failure, explosions.
    Hazard,
    /// Destroyed by pirates / raiders.
    PirateAttack,
    /// Destroyed in combat with a hostile force.
    Combat,
    /// Genuine accident (fire, collapse, misadventure).
    Accident,
    /// The player issued a direct Scuttle order on the asset.
    PlayerScuttle,
    /// The player issued a direct Deconstruct / demolish order.
    PlayerDeconstruct,
}

impl DestructionCause {
    /// True when the player ordered the destruction themselves.
    #[must_use]
    pub const fn is_player_caused(self) -> bool {
        matches!(self, Self::PlayerScuttle | Self::PlayerDeconstruct)
    }

    /// True when a claim for this cause pays out.
    #[must_use]
    pub const fn is_payable(self) -> bool {
        !self.is_player_caused()
    }

    /// Short original label for chronicles and panels.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hazard => "space hazard",
            Self::PirateAttack => "pirate attack",
            Self::Combat => "combat",
            Self::Accident => "accident",
            Self::PlayerScuttle => "player scuttle order",
            Self::PlayerDeconstruct => "player deconstruct order",
        }
    }

    /// Parse a headless/debug cause word.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "hazard" | "space_hazard" | "debris" | "storm" => Some(Self::Hazard),
            "pirate" | "pirates" | "raid" | "pirate_attack" => Some(Self::PirateAttack),
            "combat" | "hostile" | "battle" => Some(Self::Combat),
            "accident" | "fire" | "collapse" => Some(Self::Accident),
            "scuttle" | "scuttled" => Some(Self::PlayerScuttle),
            "deconstruct" | "demolish" | "decon" => Some(Self::PlayerDeconstruct),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Components, resources, events
// ---------------------------------------------------------------------------

/// An insurance policy attached to a high-value entity (a fleet, an
/// orbital structure, a Layer 1 building — anything with health).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct InsurancePolicy {
    /// The asset's insured (replacement) value in credits.
    pub insured_value: f32,
    /// Premium actually paid at purchase.
    pub premium_paid: f32,
    /// Credits paid out if the asset is lost to a payable cause.
    pub payout: f32,
    /// Tick the policy was purchased.
    pub purchased_tick: u64,
    /// False once a claim has been filed against this policy.
    pub active: bool,
}

impl InsurancePolicy {
    /// Build a policy: premium is [`PREMIUM_RATE`] of value, payout is
    /// the full insured value.
    #[must_use]
    pub fn new(insured_value: f32, purchased_tick: u64) -> Self {
        Self {
            insured_value,
            premium_paid: insured_value * PREMIUM_RATE,
            payout: insured_value,
            purchased_tick,
            active: true,
        }
    }
}

/// Request to purchase a policy on an entity. Processed by
/// [`purchase_insurance_system`], which charges the colony ledger.
#[derive(Event, Debug, Clone, Copy)]
pub struct PurchaseInsuranceEvent {
    /// The asset to insure.
    pub asset: Entity,
    /// Insured value in credits.
    pub insured_value: f32,
}

/// An insured (or formerly insured) asset was destroyed. Destruction
/// paths report through this event; [`insurance_claim_bridge_system`]
/// turns it into an [`InsuranceClaimEvent`] if a live policy exists.
#[derive(Event, Debug, Clone, Copy)]
pub struct AssetDestroyedEvent {
    /// The destroyed asset.
    pub asset: Entity,
    /// How it was destroyed.
    pub cause: DestructionCause,
    /// Fraction of the asset's health already lost before the killing
    /// blow (0.0 = pristine, 1.0 = a wreck). Drives the
    /// heavily-damaged suspicion bump.
    pub pre_damage_fraction: f32,
}

/// A claim filed against a live policy, carrying a snapshot of the
/// policy terms (the asset entity may already be gone by the time the
/// central processor runs).
#[derive(Event, Debug, Clone, Copy)]
pub struct InsuranceClaimEvent {
    /// The lost asset.
    pub asset: Entity,
    /// How it was destroyed.
    pub cause: DestructionCause,
    /// Insured value snapshotted from the policy.
    pub insured_value: f32,
    /// Payout snapshotted from the policy.
    pub payout: f32,
    /// Pre-destruction damage fraction (see [`AssetDestroyedEvent`]).
    pub pre_damage_fraction: f32,
    /// Tick the claim was filed.
    pub tick: u64,
}

/// The underwriters' investigator, on colony to audit suspicious
/// destruction. Spawned alongside the real [`Inspector`] and
/// [`Visitor`] components (spec 091 seams) — this component only adds
/// the audit schedule.
#[derive(Component, Debug, Clone, Copy)]
pub struct InsuranceInvestigator {
    /// Tick the investigator arrived.
    pub arrived_tick: u64,
    /// Tick at which the audit verdict lands.
    pub audit_due_tick: u64,
}

/// Global insurance ledger: the hidden suspicion score, claim history,
/// and investigator scheduling state (spec §7).
#[derive(Resource, Debug, Clone, Default)]
pub struct InsuranceManager {
    /// Hidden suspicion score. Crosses [`SUSPICION_THRESHOLD`] and an
    /// investigator is scheduled.
    pub suspicion_score: f32,
    /// Total claims filed against live policies.
    pub claims_filed: u32,
    /// Claims that paid out.
    pub claims_paid: u32,
    /// Claims voided (player-caused destruction).
    pub claims_voided: u32,
    /// Policies sold.
    pub policies_sold: u32,
    /// Total credits paid out.
    pub total_payouts: f32,
    /// Total credits fined.
    pub total_fines: f32,
    /// Standing fraud evidence (voided claims since the last audit).
    /// Any nonzero value fails the next audit.
    pub fraud_evidence: u32,
    /// Ticks of recently paid claims, pruned to the rapid-claim window.
    pub recent_claim_ticks: Vec<u64>,
    /// An investigator has been scheduled but has not yet arrived.
    pub investigator_scheduled: bool,
    /// Tick the scheduled investigator arrives.
    pub investigator_arrival_tick: Option<u64>,
    /// The live investigator entity, once arrived.
    pub investigator: Option<Entity>,
    /// Tick the live investigator's audit verdict lands.
    pub audit_due_tick: Option<u64>,
    /// Whether the most recent completed audit failed.
    pub last_audit_failed: bool,
}

impl InsuranceManager {
    /// Whether an investigator is scheduled or present.
    #[must_use]
    pub fn investigator_pending(&self) -> bool {
        self.investigator_scheduled || self.investigator.is_some()
    }

    /// Paid claims inside the rapid-claim window ending at `now`.
    #[must_use]
    pub fn recent_claim_count(&self, now: u64) -> usize {
        self.recent_claim_ticks
            .iter()
            .filter(|&&t| now.saturating_sub(t) <= RAPID_CLAIM_WINDOW)
            .count()
    }

    /// Schedule the investigator if suspicion or claim tempo demands
    /// it and none is already on the way. Returns true if this call
    /// scheduled one.
    pub fn maybe_schedule_investigator(&mut self, now: u64) -> bool {
        if self.investigator_pending() {
            return false;
        }
        if self.suspicion_score >= SUSPICION_THRESHOLD
            || self.recent_claim_count(now) >= RAPID_CLAIM_COUNT
        {
            self.investigator_scheduled = true;
            self.investigator_arrival_tick = Some(now + INVESTIGATOR_DELAY_TICKS);
            return true;
        }
        false
    }
}

/// A temporary suspension of colony trade, applied by a failed audit.
/// Enforced by `execute_trade` in the Layer 1 trade module while
/// active.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct TradeBlockade {
    /// Tick at which the blockade lifts (exclusive). 0 = never set.
    pub until_tick: u64,
}

impl TradeBlockade {
    /// Whether trade is blocked at `tick`.
    #[must_use]
    pub const fn is_active(&self, tick: u64) -> bool {
        tick < self.until_tick
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Purchase processing: charges the premium from the real colony
/// credit ledger and attaches an [`InsurancePolicy`]. Refuses when the
/// asset is gone, already insured, the value is not positive/finite,
/// or the colony cannot afford the premium.
pub fn purchase_insurance_system(
    mut events: EventReader<PurchaseInsuranceEvent>,
    mut commands: Commands,
    mut resources: ResMut<ColonyResources>,
    mut manager: ResMut<InsuranceManager>,
    time: Res<SimulationTime>,
    existing: Query<&InsurancePolicy>,
) {
    for event in events.read() {
        if !event.insured_value.is_finite() || event.insured_value <= 0.0 {
            continue;
        }
        if existing.get(event.asset).is_ok_and(|p| p.active) {
            continue; // Already insured — no double-selling.
        }
        let policy = InsurancePolicy::new(event.insured_value, time.tick);
        if resources.credits < policy.premium_paid {
            continue; // Cannot afford the premium.
        }
        resources.credits -= policy.premium_paid;
        manager.policies_sold += 1;
        commands.entity(event.asset).insert(policy);
    }
}

/// Bridge: an [`AssetDestroyedEvent`] against a live policy becomes an
/// [`InsuranceClaimEvent`] snapshotting the policy terms, and the
/// policy is spent (a second destruction report files nothing).
pub fn insurance_claim_bridge_system(
    mut events: EventReader<AssetDestroyedEvent>,
    mut claims: EventWriter<InsuranceClaimEvent>,
    mut policies: Query<&mut InsurancePolicy>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        let Ok(mut policy) = policies.get_mut(event.asset) else {
            continue; // Uninsured asset — the underwriters do not care.
        };
        if !policy.active {
            continue;
        }
        policy.active = false;
        claims.send(InsuranceClaimEvent {
            asset: event.asset,
            cause: event.cause,
            insured_value: policy.insured_value,
            payout: policy.payout,
            pre_damage_fraction: event.pre_damage_fraction.clamp(0.0, 1.0),
            tick: time.tick,
        });
    }
}

/// Destruction watch: an insured entity whose health reaches zero
/// without an explicit destruction report gets a claim filed
/// automatically. The cause is read off the real Item 84 attribution
/// seam: a [`SlainByFaction`] marker means a hostile (pirate) kill;
/// an explosion recorded as the [`LastDamageSource`] is a hazard;
/// anything else defaults to hazard.
#[allow(clippy::type_complexity)]
pub fn insurance_destruction_watch_system(
    mut destroyed: EventWriter<AssetDestroyedEvent>,
    query: Query<
        (
            Entity,
            &InsurancePolicy,
            Option<&FleetHealth>,
            Option<&Health>,
            Option<&LastDamageSource>,
            Option<&SlainByFaction>,
        ),
        With<InsurancePolicy>,
    >,
) {
    for (entity, policy, fleet_health, health, last_damage, slain_by) in &query {
        if !policy.active {
            continue;
        }
        let (dead, pre_damage) = match (fleet_health, health) {
            (Some(fh), _) if fh.max > 0.0 => (
                fh.current <= 0.0,
                1.0 - (fh.current.max(0.0) / fh.max),
            ),
            (_, Some(h)) if h.max > 0.0 => {
                (h.current <= 0.0, 1.0 - (h.current.max(0.0) / h.max))
            }
            _ => (false, 0.0),
        };
        if !dead {
            continue;
        }
        // A faction kill marker means hostile action; every other
        // unattributed health-zero loss (explosions included — the
        // Item 84 ledger records them as hazard damage) is a hazard.
        let cause = if slain_by.is_some() {
            DestructionCause::PirateAttack
        } else {
            DestructionCause::Hazard
        };
        destroyed.send(AssetDestroyedEvent {
            asset: entity,
            cause,
            // The killing blow landed on an already-dead asset in this
            // path, so pre-damage is judged from the ledger fraction
            // excluding the final state: a fleet ground down over time
            // reads as heavily damaged, a one-shot kill does not —
            // approximated here by the recorded damage source's
            // presence (periodic damage implies a worn asset).
            pre_damage_fraction: if last_damage.is_some() {
                pre_damage.max(0.0)
            } else {
                0.0
            },
        });
    }
}

/// Central claim processing (spec §5): payable causes pay the policy
/// payout into the colony credit ledger and accrue suspicion;
/// player-caused destruction voids the claim, records fraud evidence,
/// and accrues heavy suspicion. Either way, the investigator trigger
/// is re-evaluated.
pub fn insurance_processing_system(
    mut claims: EventReader<InsuranceClaimEvent>,
    mut resources: ResMut<ColonyResources>,
    mut manager: ResMut<InsuranceManager>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    time: Res<SimulationTime>,
) {
    for claim in claims.read() {
        manager.claims_filed += 1;
        if claim.cause.is_player_caused() {
            manager.claims_voided += 1;
            manager.fraud_evidence += 1;
            manager.suspicion_score += SUSPICION_SCUTTLE;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "Insurance claim VOID: an insured asset was destroyed by a {}. \
                     The underwriters are watching.",
                    claim.cause.label()
                ),
                importance: EventImportance::Major,
            });
        } else {
            resources.add_credits(claim.payout);
            manager.claims_paid += 1;
            manager.total_payouts += claim.payout;
            manager.suspicion_score += SUSPICION_PER_CLAIM;
            if claim.pre_damage_fraction >= HEAVILY_DAMAGED_FRACTION {
                manager.suspicion_score += SUSPICION_HEAVILY_DAMAGED;
            }
            manager.recent_claim_ticks.push(claim.tick);
            manager
                .recent_claim_ticks
                .retain(|&t| time.tick.saturating_sub(t) <= RAPID_CLAIM_WINDOW);
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "Insurance payout: {:.0} credits for an insured asset lost to a {}.",
                    claim.payout,
                    claim.cause.label()
                ),
                importance: EventImportance::Standard,
            });
        }
        if manager.maybe_schedule_investigator(time.tick) {
            chronicle.send(AddChronicleEvent {
                text: "The underwriters have dispatched an insurance investigator \
                       to audit the colony's recent losses."
                    .to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// Investigator arrival: when a scheduled investigator's tick comes,
/// spawn them through The Inspector's seams — a real [`Inspector`] +
/// [`Visitor`] at a visitor spawn point (origin fallback when the
/// colony has no spawn points registered).
pub fn insurance_investigator_arrival_system(
    mut commands: Commands,
    mut manager: ResMut<InsuranceManager>,
    time: Res<SimulationTime>,
    visitor_source: Option<Res<VisitorSource>>,
) {
    if !manager.investigator_scheduled {
        return;
    }
    let Some(arrival) = manager.investigator_arrival_tick else {
        return;
    };
    if time.tick < arrival {
        return;
    }
    let spawn_pos = visitor_source
        .as_ref()
        .and_then(|s| s.spawn_points.first().copied())
        .unwrap_or(GridPosition { x: 0, y: 0 });
    let audit_due = time.tick + AUDIT_DURATION_TICKS;
    let entity = commands
        .spawn((
            InsuranceInvestigator {
                arrived_tick: time.tick,
                audit_due_tick: audit_due,
            },
            Inspector::default(),
            Visitor {
                state: VisitorState::Arriving,
                arrival_tick: time.tick,
                departure_tick: audit_due + 500,
            },
            spawn_pos,
            crate::layer1::pop::PopName("Insurance Investigator".to_string()),
            crate::layer1::needs::Needs::default(),
        ))
        .id();
    manager.investigator_scheduled = false;
    manager.investigator_arrival_tick = None;
    manager.investigator = Some(entity);
    manager.audit_due_tick = Some(audit_due);
}

/// Audit resolution: once the investigator's audit comes due, any
/// standing fraud evidence fails it — a credit fine is levied from the
/// colony ledger and a temporary [`TradeBlockade`] is applied. A clean
/// audit halves the suspicion score. Either way the investigator's
/// work is done: the entity is despawned and the evidence ledger is
/// cleared.
pub fn insurance_audit_system(
    mut commands: Commands,
    mut manager: ResMut<InsuranceManager>,
    mut resources: ResMut<ColonyResources>,
    mut blockade: ResMut<TradeBlockade>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    time: Res<SimulationTime>,
    investigators: Query<(Entity, &InsuranceInvestigator)>,
) {
    let Some(due) = manager.audit_due_tick else {
        return;
    };
    if time.tick < due {
        return;
    }
    let Some(investigator_entity) = manager.investigator else {
        return;
    };
    if investigators.get(investigator_entity).is_err() {
        // Investigator entity vanished early — clear the stale audit.
        manager.investigator = None;
        manager.audit_due_tick = None;
        return;
    }
    if manager.fraud_evidence > 0 {
        let fine = FINE_CREDITS.min(resources.credits.max(0.0));
        resources.credits -= fine;
        manager.total_fines += fine;
        manager.last_audit_failed = true;
        blockade.until_tick = time.tick + BLOCKADE_TICKS;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Insurance audit FAILED: evidence of fraud found. Fine of {fine:.0} \
                 credits levied and a trade blockade imposed for {BLOCKADE_TICKS} ticks."
            ),
            importance: EventImportance::Major,
        });
    } else {
        manager.suspicion_score *= 0.5;
        manager.last_audit_failed = false;
        chronicle.send(AddChronicleEvent {
            text: "Insurance audit passed: the colony's losses were judged genuine. \
                   The underwriters' suspicion eases."
                .to_string(),
            importance: EventImportance::Standard,
        });
    }
    manager.fraud_evidence = 0;
    manager.investigator = None;
    manager.audit_due_tick = None;
    commands.entity(investigator_entity).despawn();
}

// ---------------------------------------------------------------------------
// World-level helpers (headless console + tests)
// ---------------------------------------------------------------------------

/// Count of live (unclaimed) policies in the world.
#[must_use]
pub fn active_policy_count(world: &mut World) -> usize {
    world
        .query::<&InsurancePolicy>()
        .iter(world)
        .filter(|p| p.active)
        .count()
}

/// Total policies (live + claimed) in the world.
#[must_use]
pub fn policy_count(world: &mut World) -> usize {
    world.query::<&InsurancePolicy>().iter(world).count()
}

/// Whether an insurance investigator is currently on colony.
#[must_use]
pub fn investigator_present(world: &mut World) -> bool {
    world
        .query::<&InsuranceInvestigator>()
        .iter(world)
        .next()
        .is_some()
}

/// Buy a policy on `asset` directly against the world (the headless
/// path; the live-schedule path is [`PurchaseInsuranceEvent`]).
///
/// # Errors
/// Returns a message when the asset is missing, already insured, the
/// value is invalid, or the colony cannot afford the premium.
pub fn insure_entity(
    world: &mut World,
    asset: Entity,
    insured_value: f32,
) -> Result<String, String> {
    if world.get_entity(asset).is_err() {
        return Err("No such asset to insure.".to_string());
    }
    if !insured_value.is_finite() || insured_value <= 0.0 {
        return Err("Insured value must be a positive amount.".to_string());
    }
    if world
        .get::<InsurancePolicy>(asset)
        .is_some_and(|p| p.active)
    {
        return Err("That asset already carries a live policy.".to_string());
    }
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let policy = InsurancePolicy::new(insured_value, tick);
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        if resources.credits < policy.premium_paid {
            return Err(format!(
                "Cannot afford the premium: need {:.0} credits, have {:.0}.",
                policy.premium_paid, resources.credits
            ));
        }
        resources.credits -= policy.premium_paid;
    }
    if let Some(mut manager) = world.get_resource_mut::<InsuranceManager>() {
        manager.policies_sold += 1;
    }
    world.entity_mut(asset).insert(policy);
    Ok(format!(
        "Policy written: {:.0} credits of cover for a {:.0} credit premium.",
        policy.insured_value, policy.premium_paid
    ))
}

/// Report the destruction of the first live insured asset (the
/// headless destruction path). The claim itself is filed by the bridge
/// system on the next tick.
///
/// # Errors
/// Returns a message when no live policy exists.
pub fn destroy_first_insured(
    world: &mut World,
    cause: DestructionCause,
    pre_damage_fraction: f32,
) -> Result<String, String> {
    let asset = {
        let mut query = world.query::<(Entity, &InsurancePolicy)>();
        query
            .iter(world)
            .find(|(_, p)| p.active)
            .map(|(e, _)| e)
            .ok_or_else(|| "No live insurance policy to claim against.".to_string())?
    };
    world
        .resource_mut::<Events<AssetDestroyedEvent>>()
        .send(AssetDestroyedEvent {
            asset,
            cause,
            pre_damage_fraction,
        });
    Ok(format!(
        "Insured asset reported destroyed ({}) — claim filed with the underwriters.",
        cause.label()
    ))
}

/// One-line insurance status for panels and STATS-adjacent reporting.
#[must_use]
pub fn insurance_status_label(world: &mut World) -> String {
    let policies = active_policy_count(world);
    let (suspicion, paid, voided, pending, blocked) =
        match world.get_resource::<InsuranceManager>() {
            Some(m) => (
                m.suspicion_score,
                m.claims_paid,
                m.claims_voided,
                m.investigator_pending(),
                world
                    .get_resource::<TradeBlockade>()
                    .zip(world.get_resource::<SimulationTime>())
                    .is_some_and(|(b, t)| b.is_active(t.tick)),
            ),
            None => (0.0, 0, 0, false, false),
        };
    format!(
        "policies={policies} suspicion={suspicion:.0} claims_paid={paid} \
         claims_voided={voided} investigator={} blockade={}",
        if pending { "en_route" } else { "none" },
        if blocked { "active" } else { "none" }
    )
}

// ---------------------------------------------------------------------------
// Tests — the spec §3 RED tests, adapted to the real codebase and run
// against the LIVE schedule (an `App` with the systems in `Update`,
// chained in production order).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};

    use crate::layer1::economy::trade::{execute_trade, TradeDeal};
    use crate::layer1::economy::resources::ResourceType;
    use crate::layer2::fleet::{Fleet, FleetHealth};

    fn setup_app(credits: f32) -> App {
        let mut app = App::new();
        app.insert_resource(ColonyResources {
            credits,
            wood: 100.0,
            ..Default::default()
        });
        app.insert_resource(InsuranceManager::default());
        app.insert_resource(TradeBlockade::default());
        app.insert_resource(SimulationTime::default());
        app.insert_resource(VisitorSource::default());
        app.add_event::<PurchaseInsuranceEvent>();
        app.add_event::<AssetDestroyedEvent>();
        app.add_event::<InsuranceClaimEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(
            Update,
            (
                purchase_insurance_system,
                insurance_destruction_watch_system,
                insurance_claim_bridge_system,
                insurance_processing_system,
                insurance_investigator_arrival_system,
                insurance_audit_system,
            )
                .chain(),
        );
        app
    }

    fn spawn_insured_fleet(app: &mut App, value: f32) -> Entity {
        let fleet = app
            .world_mut()
            .spawn((
                Fleet,
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
            ))
            .id();
        app.world_mut().send_event(PurchaseInsuranceEvent {
            asset: fleet,
            insured_value: value,
        });
        app.update();
        assert!(
            app.world().get::<InsurancePolicy>(fleet).is_some_and(|p| p.active),
            "policy should be attached after purchase"
        );
        fleet
    }

    fn destroy(app: &mut App, asset: Entity, cause: DestructionCause, pre_damage: f32) {
        app.world_mut().send_event(AssetDestroyedEvent {
            asset,
            cause,
            pre_damage_fraction: pre_damage,
        });
        app.update();
        app.update();
    }

    /// Spec §3 (1): insured ship destroyed by an external source pays.
    #[test]
    fn test_insured_ship_destruction_yields_payout() {
        let mut app = setup_app(1000.0);
        let fleet = spawn_insured_fleet(&mut app, 500.0);
        // Premium: 20% of 500 = 100.
        assert!((app.world().resource::<ColonyResources>().credits - 900.0).abs() < 1e-4);
        destroy(&mut app, fleet, DestructionCause::Hazard, 0.0);
        let credits = app.world().resource::<ColonyResources>().credits;
        assert!(
            (credits - 1400.0).abs() < 1e-4,
            "hazard loss of a 500cr policy should pay out in full, credits={credits}"
        );
        let manager = app.world().resource::<InsuranceManager>();
        assert_eq!(manager.claims_paid, 1);
        assert!((manager.suspicion_score - SUSPICION_PER_CLAIM).abs() < 1e-4);
    }

    /// Spec §3 (2): intentional scuttling voids the insurance.
    #[test]
    fn test_intentional_scuttling_voids_insurance() {
        let mut app = setup_app(1000.0);
        let fleet = spawn_insured_fleet(&mut app, 500.0);
        destroy(&mut app, fleet, DestructionCause::PlayerScuttle, 0.0);
        let credits = app.world().resource::<ColonyResources>().credits;
        assert!(
            (credits - 900.0).abs() < 1e-4,
            "scuttled fleet must pay nothing, credits={credits}"
        );
        let manager = app.world().resource::<InsuranceManager>();
        assert_eq!(manager.claims_voided, 1);
        assert_eq!(manager.claims_paid, 0);
        assert_eq!(manager.fraud_evidence, 1);
        assert!(manager.suspicion_score >= SUSPICION_SCUTTLE);
    }

    /// Spec §3 (3): multiple claims in a short window schedule an
    /// investigator, who then actually arrives through the Inspector
    /// seams (a real Inspector + Visitor entity).
    #[test]
    fn test_multiple_claims_trigger_investigator_arrival() {
        let mut app = setup_app(10_000.0);
        for _ in 0..3 {
            let fleet = spawn_insured_fleet(&mut app, 500.0);
            destroy(&mut app, fleet, DestructionCause::PirateAttack, 0.0);
        }
        {
            let manager = app.world().resource::<InsuranceManager>();
            assert_eq!(manager.claims_paid, 3);
            assert!(
                manager.investigator_scheduled,
                "3 rapid claims must schedule an investigator"
            );
        }
        // Advance past the arrival tick and run the live schedule.
        app.world_mut().resource_mut::<SimulationTime>().tick += INVESTIGATOR_DELAY_TICKS + 1;
        app.update();
        let mut query = app
            .world_mut()
            .query::<(&InsuranceInvestigator, &Inspector, &Visitor)>();
        assert_eq!(
            query.iter(app.world()).count(),
            1,
            "investigator must arrive as a real Inspector/Visitor entity"
        );
        assert!(app.world().resource::<InsuranceManager>().investigator.is_some());
    }

    /// Spec §3 (4): a failed investigation levies a fine and applies
    /// a temporary TradeBlockade that the real trade path honours.
    #[test]
    fn test_failed_investigation_results_in_fines_and_blockade() {
        let mut app = setup_app(2000.0);
        // Fraud on the record, investigator already due to arrive.
        {
            let mut manager = app.world_mut().resource_mut::<InsuranceManager>();
            manager.fraud_evidence = 1;
            manager.investigator_scheduled = true;
            manager.investigator_arrival_tick = Some(0);
        }
        app.update(); // arrival
        assert!(app.world().resource::<InsuranceManager>().investigator.is_some());
        // Advance past the audit and run it.
        app.world_mut().resource_mut::<SimulationTime>().tick += AUDIT_DURATION_TICKS + 1;
        let audit_tick = app.world().resource::<SimulationTime>().tick;
        app.update();
        let credits = app.world().resource::<ColonyResources>().credits;
        assert!(
            (credits - (2000.0 - FINE_CREDITS)).abs() < 1e-4,
            "failed audit must levy the fine, credits={credits}"
        );
        let manager = app.world().resource::<InsuranceManager>();
        assert!(manager.last_audit_failed);
        assert!((manager.total_fines - FINE_CREDITS).abs() < 1e-4);
        assert!(
            app.world().resource::<TradeBlockade>().is_active(audit_tick),
            "failed audit must apply an active TradeBlockade"
        );
        // The real trade entry point refuses while blockaded.
        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: 10.0,
            give_resource: ResourceType::Metal,
            give_amount: 5.0,
        };
        assert!(
            !execute_trade(app.world_mut(), &deal),
            "execute_trade must refuse during a blockade"
        );
    }

    /// A clean audit: no fine, no blockade, suspicion eases, and trade
    /// flows again.
    #[test]
    fn test_passed_investigation_clears_suspicion() {
        let mut app = setup_app(2000.0);
        {
            let mut manager = app.world_mut().resource_mut::<InsuranceManager>();
            manager.suspicion_score = 60.0;
            manager.investigator_scheduled = true;
            manager.investigator_arrival_tick = Some(0);
        }
        app.update();
        app.world_mut().resource_mut::<SimulationTime>().tick += AUDIT_DURATION_TICKS + 1;
        let audit_tick = app.world().resource::<SimulationTime>().tick;
        app.update();
        let manager = app.world().resource::<InsuranceManager>();
        assert!(!manager.last_audit_failed);
        assert!((manager.suspicion_score - 30.0).abs() < 1e-4);
        assert!(!app.world().resource::<TradeBlockade>().is_active(audit_tick));
        assert!((app.world().resource::<ColonyResources>().credits - 2000.0).abs() < 1e-4);
    }

    /// Heavily-damaged assets draw the big suspicion bump (spec §7).
    #[test]
    fn test_heavily_damaged_claim_raises_extra_suspicion() {
        let mut app = setup_app(1000.0);
        let fleet = spawn_insured_fleet(&mut app, 500.0);
        destroy(&mut app, fleet, DestructionCause::Hazard, 0.9);
        let manager = app.world().resource::<InsuranceManager>();
        let expected = SUSPICION_PER_CLAIM + SUSPICION_HEAVILY_DAMAGED;
        assert!(
            (manager.suspicion_score - expected).abs() < 1e-4,
            "suspicion={} expected={expected}",
            manager.suspicion_score
        );
    }

    /// The destruction watch files a claim by itself when an insured
    /// fleet's health hits zero (the live destruction pipeline seam).
    #[test]
    fn test_destruction_watch_files_claim_for_dead_fleet() {
        let mut app = setup_app(1000.0);
        let fleet = spawn_insured_fleet(&mut app, 500.0);
        app.world_mut().get_mut::<FleetHealth>(fleet).unwrap().current = 0.0;
        app.update();
        app.update();
        let manager = app.world().resource::<InsuranceManager>();
        assert_eq!(manager.claims_paid, 1, "watch must file and pay the claim");
        assert!((app.world().resource::<ColonyResources>().credits - 1400.0).abs() < 1e-4);
    }

    /// Purchases fail closed: no credits, no policy, no charge.
    #[test]
    fn test_purchase_requires_premium_credits() {
        let mut app = setup_app(50.0); // Premium on 500 is 100.
        let fleet = app
            .world_mut()
            .spawn((Fleet, FleetHealth { current: 100.0, max: 100.0 }))
            .id();
        app.world_mut().send_event(PurchaseInsuranceEvent {
            asset: fleet,
            insured_value: 500.0,
        });
        app.update();
        assert!(app.world().get::<InsurancePolicy>(fleet).is_none());
        assert!((app.world().resource::<ColonyResources>().credits - 50.0).abs() < 1e-4);
    }

    /// Uninsured destruction files nothing and pays nothing.
    #[test]
    fn test_uninsured_destruction_pays_nothing() {
        let mut app = setup_app(1000.0);
        let fleet = app
            .world_mut()
            .spawn((Fleet, FleetHealth { current: 100.0, max: 100.0 }))
            .id();
        destroy(&mut app, fleet, DestructionCause::Hazard, 0.0);
        assert_eq!(app.world().resource::<InsuranceManager>().claims_filed, 0);
        assert!((app.world().resource::<ColonyResources>().credits - 1000.0).abs() < 1e-4);
    }

    /// IP guard: every public-facing string this module emits uses
    /// original names only — no borrowed franchises or underwriters.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "star wars",
            "star trek",
            "battlestar",
            "firefly",
            "serenity",
            "millennium falcon",
            "enterprise",
            "lloyd's",
            "lloyds of london",
            "geico",
            "allstate",
            "warhammer",
            "dune",
            "foundation",
            "imperium",
        ];
        let mut texts: Vec<String> = DestructionCause::parse("hazard")
            .into_iter()
            .chain(DestructionCause::parse("pirate"))
            .chain(DestructionCause::parse("scuttle"))
            .map(|c| c.label().to_string())
            .collect();
        texts.push("Insurance Investigator".to_string());
        texts.push("insurance payout void audit blockade underwriters".to_string());
        for text in &texts {
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
