//! The Planetary Governor — a bureaucratic adventurer origin.
//!
//! Layers on top of the adventurer-mode spike (`direct_link`): a sealed
//! appointment spawns near the starter colony. A possessed pop standing
//! adjacent can `interact` to sign in triplicate, gaining [`Governor`] status
//! plus a procedural-legitimacy meter and access to the colonial treasury.
//!
//! While possessed-and-appointed, the headless console gains `directive`,
//! `tithe`, `hearing`, `file`, `audit`, `governors`, `debate`, `purge`,
//! `resign`, and `treasury`. Directives are cooldown-gated administrative
//! orders with colony-wide effects; legitimacy is fed by *process* (hearings,
//! paperwork, clean audits) and gates directive strength. Rival administrators
//! arrive over time and must be debated, purged, or audited into irrelevance.
//! Embezzlement scandals and contradictory directives threaten legitimacy.
//!
//! All names are original. The "planetary governor" concept is concept-only
//! inspiration — no named characters, places, or distinctive IP anywhere.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::Wallet;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::terrain::{TerrainGrid, TerrainType};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Chronicle text when the seal is posted.
pub const SEAL_CHRONICLE: &str =
    "A sealed appointment is nailed to the notice board. The forms demand a signature.";
/// Chronicle text when someone signs in triplicate.
pub const SIGNING_CHRONICLE: &str =
    "The forms are signed in triplicate. The colony has a governor now.";
/// Chronicle text on resignation.
pub const RESIGNATION_CHRONICLE: &str =
    "The governor resigns, citing 'irreconcilable procedural differences'. The seal awaits.";
/// Chronicle text when the governor's body is found.
pub const GOVERNOR_DEATH_CHRONICLE: &str =
    "The governor's seal lies where its bearer fell. The paperwork outlives them all.";

/// Shared cooldown (ticks) after any directive.
pub const DIRECTIVE_COOLDOWN_TICKS: u32 = 60;

/// Quota (work quotas): duration, bonus, judgment.
pub const QUOTA_DURATION: u32 = 120;
pub const QUOTA_BONUS_BASE: f32 = 0.15;
pub const QUOTA_BONUS_LEGITIMACY_SCALE: f32 = 0.3;
/// Minimum legitimacy required to issue work quotas.
pub const QUOTA_MIN_LEGITIMACY: f32 = 0.4;
/// Ticks after a quota directive when production is judged.
pub const QUOTA_JUDGMENT_DELAY: u32 = 60;
/// Legitimacy swing for a productive / unproductive quota.
pub const QUOTA_JUDGMENT: f32 = 0.04;

/// Ration (rationing schedule): duration, morale hit, meal fraction.
pub const RATION_DURATION: u32 = 100;
pub const RATION_MORALE: f32 = -0.12;
/// Each meal costs this fraction of the normal food while rationing.
pub const RATION_MEAL_FRACTION: f32 = 0.6;

/// Requisition: fraction of each other pop's wallet seized into the treasury.
pub const REQUISITION_RATE: f32 = 0.15;
/// Legitimacy cost of a requisition order.
pub const REQUISITION_LEGITIMACY_COST: f32 = 0.08;

/// Works (public works program): treasury cost, morale, duration.
pub const WORKS_COST: f32 = 50.0;
pub const WORKS_DURATION: u32 = 150;
pub const WORKS_MORALE: f32 = 0.15;
/// Legitimacy for duly-filed public spending.
pub const WORKS_LEGITIMACY: f32 = 0.04;

/// Tithe: fraction of each pop's wallet collected into the treasury.
pub const TITHE_RATE: f32 = 0.05;
/// Above this legitimacy, tithes are "properly filed" (+); below, resented (-).
pub const TITHE_LEGITIMACY_GATE: f32 = 0.6;
pub const TITHE_LEGITIMACY_GOOD: f32 = 0.02;
pub const TITHE_LEGITIMACY_BAD: f32 = -0.01;

/// Hearing: legitimacy reward, treasury cost, cooldown.
pub const HEARING_LEGITIMACY: f32 = 0.06;
pub const HEARING_COST: f32 = 10.0;
pub const HEARING_COOLDOWN: u32 = 40;

/// Filing paperwork: legitimacy per form, cooldown.
pub const FILE_LEGITIMACY: f32 = 0.02;
pub const FILE_COOLDOWN: u32 = 8;

/// Audit: legitimacy for a clean audit, cooldown.
pub const AUDIT_LEGITIMACY: f32 = 0.03;
pub const AUDIT_COOLDOWN: u32 = 50;

/// Debate: legitimacy swing for winner/loser.
pub const DEBATE_WIN_LEGITIMACY: f32 = 0.05;
pub const DEBATE_LOSE_LEGITIMACY: f32 = -0.05;

/// Purge: legitimacy cost of removing a rival by fiat.
pub const PURGE_LEGITIMACY_COST: f32 = 0.12;

/// Caught embezzler: legitimacy for exposing one.
pub const EMBEZZLER_EXPOSED_LEGITIMACY: f32 = 0.04;
/// Fraction of skimmed credits recovered on exposure.
pub const EMBEZZLER_RECOVERY_FRACTION: f32 = 0.5;

/// Rivals: first arrival, interval, max coexisting.
pub const RIVAL_FIRST_ARRIVAL: u32 = 600;
pub const RIVAL_ARRIVAL_INTERVAL: u32 = 800;
pub const RIVAL_MAX: usize = 2;
/// Legitimacy drained per rival per 100 ticks.
pub const RIVAL_UNDERMINE_PER_100: f32 = 0.02;
/// Rival legitimacy drift per 100 ticks (they file forms too).
pub const RIVAL_LEGITIMACY_DRIFT: f32 = 0.01;
/// Credits a rival skims from the treasury per tick.
pub const RIVAL_SKIM_PER_TICK: f32 = 0.05;
/// Starting legitimacy of a fresh rival claimant.
pub const RIVAL_GOVERNOR_STARTING_LEGITIMACY: f32 = 0.35;

/// Treasury-drain scandal: legitimacy hit.
pub const DRAIN_SCANDAL_LEGITIMACY: f32 = 0.08;
/// Spending this much within the window, ending at zero, triggers the scandal.
pub const DRAIN_SCANDAL_SPEND: f32 = 30.0;
pub const DRAIN_SCANDAL_WINDOW: u32 = 300;

/// Contradictory directives: legitimacy hit.
pub const CONTRADICTION_LEGITIMACY: f32 = 0.10;
pub const QUOTA_RATION_TENSION: f32 = 0.06;

/// Starting meters on signing.
pub const GOVERNOR_STARTING_LEGITIMACY: f32 = 0.5;

/// Morale modifier labels (refreshed each tick, never stacked).
pub const RATION_MODIFIER_LABEL: &str = "Rationing Schedule";
pub const WORKS_MODIFIER_LABEL: &str = "Public Works Program";

/// Rival claims (original, bureaucratic).
pub const RIVAL_CLAIMS: [&str; 4] = [
    "Acting Governor (provisional)",
    "Governor-Emeritus (self-declared)",
    "Interim Steward of the Ledgers",
    "Custodian of the Seal (disputed)",
];

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A sealed appointment lying on the ground, waiting for a signature.
/// (While held it is despawned; the [`Governor`] component marks the bearer.)
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct AppointmentSeal;

/// Status of the appointed pop: a bureaucrat-governor ruling by procedure.
#[derive(Component, Debug, Clone, Copy)]
pub struct Governor {
    /// 0.0 (usurper) .. 1.0 (duly appointed). Starts at 0.5.
    pub legitimacy: f32,
}

/// A rival administrator: a migrant-spawned claimant with their own sealed
/// writ, undermining the governor's legitimacy by existing loudly.
#[derive(Component, Debug, Clone)]
pub struct RivalGovernor {
    /// 0.0 .. 1.0. Starts at [`RIVAL_GOVERNOR_STARTING_LEGITIMACY`].
    pub legitimacy: f32,
    /// Their bureaucratic claim, e.g. "Acting Governor (provisional)".
    pub claim: String,
    /// Total credits skimmed from the treasury so far.
    pub skimmed: f32,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// The colonial treasury: revenue-service credits, spendable on hearings,
/// public works, and ceremonial stationery. Persists across reigns — the
/// colony's money, not the governor's.
#[derive(Resource, Debug, Default, Clone)]
pub struct Treasury {
    /// Current credit balance.
    pub credits: f32,
}

/// Work-quotas broadcast to the utility AI while a quota directive is active.
/// Read by `utility_ai` when building evaluation contexts.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct QuotaOrder {
    /// Bonus added to work-type action utilities (0.0 when no quota).
    pub bonus: f32,
    /// Ticks of the quota remaining.
    pub remaining: u32,
}

/// Rationing schedule: while `remaining > 0`, meals cost
/// [`RATION_MEAL_FRACTION`] of normal food. Read by `consume_food_system`.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct RationOrder {
    /// Ticks of rationing remaining.
    pub remaining: u32,
}

/// Public works program: while `remaining > 0`, colony morale rises.
/// Refreshed by `apply_governor_morale`.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WorksProgram {
    /// Ticks of the program remaining.
    pub remaining: u32,
}

/// Pending quota judgment: production baseline sampled at issue, judged when
/// the countdown hits zero.
#[derive(Debug, Clone)]
pub struct QuotaJudgment {
    /// Ticks until the judgment.
    pub judge_in: u32,
    /// Cumulative farm production when the quota was issued.
    pub baseline: f32,
}

/// Machine state for the governor origin.
#[derive(Resource, Debug, Default)]
pub struct GovernorState {
    /// The seal has been posted (once per colony).
    pub seal_spawn_attempted: bool,
    /// The appointed governor, if any.
    pub governor: Option<Entity>,
    /// Last known position (for dropping the seal on death).
    pub last_known_pos: Option<GridPosition>,
    /// Shared cooldown after any directive.
    pub directive_cooldown: u32,
    /// Cooldowns for the procedural actions.
    pub hearing_cooldown: u32,
    pub file_cooldown: u32,
    pub audit_cooldown: u32,
    /// Procedural tallies.
    pub forms_filed: u32,
    pub hearings_held: u32,
    /// The currently active duration directive (quota or ration) + remaining.
    pub active_directive: Option<(DirectiveKind, u32)>,
    /// Pending quota judgment.
    pub quota_judgment: Option<QuotaJudgment>,
    /// Farm-production samples for the quota judgment.
    pub production_samples: VecDeque<f32>,
    /// Countdown to the next rival arrival.
    pub rival_timer: u32,
    /// Accumulated ticks for the rival-undermining cadence.
    pub undermine_accum: u32,
    /// Treasury spend inside the drain-scandal window.
    pub spent_in_window: f32,
    pub spend_window: u32,
    /// Drain scandal already fired this reign.
    pub drain_scandal_fired: bool,
    /// Recent treasury flows, newest last (capped).
    pub flows: Vec<(String, f32)>,
    /// Prev pop count, for liveness bookkeeping.
    pub prev_pop_count: usize,
}

/// The directives a possessed-and-appointed governor can issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    /// Work quotas: +work utility, judged on farm production.
    Quota,
    /// Rationing schedule: smaller meals, morale hit, food saved.
    Ration,
    /// Requisition order: seize wallets into the treasury, costs legitimacy.
    Requisition,
    /// Public works program: spend treasury on morale.
    Works,
}

impl DirectiveKind {
    /// Display name of the directive.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Quota => "Directive 12-A: Work Quotas",
            Self::Ration => "Directive 7-C: Rationing Schedule",
            Self::Requisition => "Directive 9-F: Requisition Order",
            Self::Works => "Directive 3-B: Public Works Program",
        }
    }

    /// Whether issuing `other` while `self` is active is a contradiction.
    #[must_use]
    pub const fn contradicts(self, other: DirectiveKind) -> bool {
        matches!(
            (self, other),
            (Self::Quota, Self::Ration)
                | (Self::Ration, Self::Quota)
                | (Self::Works, Self::Ration)
                | (Self::Ration, Self::Works)
        )
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Make sure all governor resources exist.
pub fn ensure_governor_resources(world: &mut World) {
    world.init_resource::<GovernorState>();
    world.init_resource::<Treasury>();
    world.init_resource::<QuotaOrder>();
    world.init_resource::<RationOrder>();
    world.init_resource::<WorksProgram>();
    world.init_resource::<Events<AddChronicleEvent>>();
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

/// Find the ground seal entity, if one is lying around.
#[must_use]
pub fn find_ground_seal(world: &mut World) -> Option<(Entity, GridPosition)> {
    world
        .query::<(Entity, &AppointmentSeal, &GridPosition)>()
        .iter(world)
        .map(|(e, _, p)| (e, *p))
        .next()
}

/// The ground seal within signing reach (Chebyshev distance <= 1) of `pop`.
#[must_use]
pub fn seal_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    find_ground_seal(world).and_then(|(seal, seal_pos)| {
        let dx = (seal_pos.x - pos.x).abs();
        let dy = (seal_pos.y - pos.y).abs();
        (dx <= 1 && dy <= 1).then_some(seal)
    })
}

/// Clamp a meter into 0.0..=1.0.
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Add `delta` to the governor's legitimacy, clamped.
fn adjust_legitimacy(world: &mut World, delta: f32) {
    let gov = world.resource::<GovernorState>().governor;
    if let Some(e) = gov {
        if let Some(mut g) = world.get_mut::<Governor>(e) {
            g.legitimacy = clamp01(g.legitimacy + delta);
        }
    }
}

/// Current legitimacy (0.0 when no governor).
#[must_use]
pub fn governor_legitimacy(world: &mut World) -> f32 {
    world
        .resource::<GovernorState>()
        .governor
        .and_then(|e| world.get::<Governor>(e))
        .map_or(0.0, |g| g.legitimacy)
}

/// Record a treasury flow (capped at 12 entries).
fn record_flow(world: &mut World, label: &str, amount: f32) {
    let mut state = world.resource_mut::<GovernorState>();
    state.flows.push((label.to_string(), amount));
    while state.flows.len() > 12 {
        state.flows.remove(0);
    }
}

/// Spend `amount` from the treasury; returns the amount actually spent.
fn spend_treasury(world: &mut World, label: &str, amount: f32) -> f32 {
    let available = world.resource::<Treasury>().credits;
    let spent = amount.min(available.max(0.0));
    {
        let mut treasury = world.resource_mut::<Treasury>();
        treasury.credits -= spent;
    }
    if spent > 0.0 {
        record_flow(world, label, -spent);
        let mut state = world.resource_mut::<GovernorState>();
        state.spent_in_window += spent;
    }
    spent
}

/// The possessed pop must be the appointed governor.
fn require_governor(world: &mut World, pop: Entity) -> Result<(), String> {
    let gov = world.resource::<GovernorState>().governor;
    match gov {
        Some(e) if e == pop && world.get::<Governor>(e).is_some() => Ok(()),
        _ => Err(
            "No appointed governor is possessed. Possess an appointee, move adjacent to the \
             appointment seal, and `interact` to sign in triplicate."
                .to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// The seal: spawn, sign, resign
// ---------------------------------------------------------------------------

/// Post the appointment seal once, near the starter lander.
pub fn spawn_appointment_seal_once(world: &mut World) {
    ensure_governor_resources(world);
    {
        let state = world.resource::<GovernorState>();
        if state.seal_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<GovernorState>().seal_spawn_attempted = true;

    // Only one seal per colony: skip if one lies around or is held.
    if find_ground_seal(world).is_some() {
        return;
    }
    if world.query::<&Governor>().iter(world).next().is_some() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    // Snapshot candidate tiles first: the building query below needs `&mut
    // world`, which conflicts with live resource borrows.
    let (walkable, occupied, buildings) = {
        let mut walkable = std::collections::HashSet::new();
        if let Some(terrain) = world.get_resource::<TerrainGrid>() {
            for dx in -8..=8i32 {
                for dy in -8..=8i32 {
                    let x = lander.x + dx;
                    let y = lander.y + dy;
                    if x >= 0 && y >= 0 {
                        if let Some(tt) = terrain.get(x as usize, y as usize) {
                            if !matches!(tt, TerrainType::Water | TerrainType::Void) {
                                walkable.insert((x, y));
                            }
                        }
                    }
                }
            }
        }
        let occupied: std::collections::HashSet<(i32, i32)> = world
            .get_resource::<OccupiedTiles>()
            .map(|o| o.0.iter().copied().collect())
            .unwrap_or_default();
        let buildings: std::collections::HashSet<(i32, i32)> = world
            .query::<(&Building, &GridPosition)>()
            .iter(world)
            .map(|(_, p)| (p.x, p.y))
            .collect();
        (walkable, occupied, buildings)
    };
    let target: Option<GridPosition> = {
        let mut found = None;
        'search: for radius in 1..=8i32 {
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    if dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let x = lander.x + dx;
                    let y = lander.y + dy;
                    if x < 0 || y < 0 {
                        continue;
                    }
                    if occupied.contains(&(x, y)) {
                        continue;
                    }
                    if buildings.contains(&(x, y)) {
                        continue;
                    }
                    if !walkable.contains(&(x, y)) {
                        continue;
                    }
                    found = Some(GridPosition { x, y });
                    break 'search;
                }
            }
        }
        found
    };

    if let Some(pos) = target {
        world.spawn((AppointmentSeal, pos));
        send_chronicle(
            world,
            &format!(
                "Rumor: a sealed appointment (Form 1-A, stamped in triplicate) is nailed to the \
                 notice board at ({}, {}), near the lander.",
                pos.x, pos.y
            ),
            EventImportance::Minor,
        );
    }
}

/// Sign the appointment: consume the adjacent ground seal, appoint the pop.
pub fn try_sign_seal(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_governor_resources(world);
    if world.get::<Governor>(pop).is_some() {
        return Err("Already appointed. The forms are filed.".to_string());
    }
    let seal = seal_within_reach(world, pop).ok_or_else(|| {
        "No appointment seal within reach. Stand beside the sealed appointment and try again."
            .to_string()
    })?;
    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));
    world.despawn(seal);
    world.entity_mut(pop).insert(Governor {
        legitimacy: GOVERNOR_STARTING_LEGITIMACY,
    });
    let pos = world.get::<GridPosition>(pop).copied();
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.governor = Some(pop);
        state.last_known_pos = pos;
        state.rival_timer = RIVAL_FIRST_ARRIVAL;
        state.drain_scandal_fired = false;
        state.spent_in_window = 0.0;
        state.spend_window = 0;
    }
    send_chronicle(world, SIGNING_CHRONICLE, EventImportance::Major);
    Ok(format!(
        "{name} signs in triplicate. {SIGNING_CHRONICLE} Somewhere, a clerk weeps with joy."
    ))
}

/// Clear all active reign effects (directives, orders, programs).
fn clear_reign_effects(world: &mut World) {
    {
        let mut quota = world.resource_mut::<QuotaOrder>();
        quota.bonus = 0.0;
        quota.remaining = 0;
    }
    {
        let mut ration = world.resource_mut::<RationOrder>();
        ration.remaining = 0;
    }
    {
        let mut works = world.resource_mut::<WorksProgram>();
        works.remaining = 0;
    }
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.active_directive = None;
        state.quota_judgment = None;
    }
}

/// Resign: drop the seal on the current tile, clear reign effects.
/// The treasury and any rivals stay — they are the colony's problem now.
pub fn resign(world: &mut World, pop: Entity) -> Result<String, String> {
    require_governor(world, pop)?;
    let pos = world
        .get::<GridPosition>(pop)
        .copied()
        .unwrap_or(GridPosition { x: 0, y: 0 });
    world.entity_mut(pop).remove::<Governor>();
    world.spawn((AppointmentSeal, pos));
    clear_reign_effects(world);
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.governor = None;
        state.last_known_pos = None;
    }
    send_chronicle(world, RESIGNATION_CHRONICLE, EventImportance::Major);
    Ok("The seal is laid down. The bureaucracy continues without you.".to_string())
}

fn handle_governor_death(world: &mut World) {
    let pos = world
        .resource::<GovernorState>()
        .last_known_pos
        .unwrap_or(GridPosition { x: 0, y: 0 });
    world.spawn((AppointmentSeal, pos));
    clear_reign_effects(world);
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.governor = None;
        state.last_known_pos = None;
    }
    send_chronicle(world, GOVERNOR_DEATH_CHRONICLE, EventImportance::Major);
}

// ---------------------------------------------------------------------------
// Directives & revenue
// ---------------------------------------------------------------------------

/// Penalty for a contradictory directive pair.
fn contradiction_penalty(a: DirectiveKind, b: DirectiveKind) -> f32 {
    if matches!(
        (a, b),
        (DirectiveKind::Quota, DirectiveKind::Ration)
            | (DirectiveKind::Ration, DirectiveKind::Quota)
    ) {
        QUOTA_RATION_TENSION
    } else {
        CONTRADICTION_LEGITIMACY
    }
}

/// Check and clear the shared directive cooldown.
fn take_directive_cooldown(world: &mut World) -> Result<(), String> {
    let cd = world.resource::<GovernorState>().directive_cooldown;
    if cd > 0 {
        return Err(format!(
            "The forms are still being processed ({cd} ticks remaining)."
        ));
    }
    world.resource_mut::<GovernorState>().directive_cooldown = DIRECTIVE_COOLDOWN_TICKS;
    Ok(())
}

/// Issue an administrative directive as the possessed-and-appointed governor.
pub fn issue_directive(
    world: &mut World,
    pop: Entity,
    kind: DirectiveKind,
) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;

    // Validate before the cooldown is consumed: a rejected directive costs
    // nothing but embarrassment.
    let legitimacy = governor_legitimacy(world);
    if kind == DirectiveKind::Quota && legitimacy < QUOTA_MIN_LEGITIMACY {
        return Err(format!(
            "The forms are returned unsigned: legitimacy {legitimacy:.2} is below the \
             required {QUOTA_MIN_LEGITIMACY:.2}."
        ));
    }
    if kind == DirectiveKind::Works {
        let balance = world.resource::<Treasury>().credits;
        if balance < WORKS_COST {
            return Err(format!(
                "The treasury holds only {balance:.1}cr; the program requires {WORKS_COST:.1}cr. \
                 File a requisition first."
            ));
        }
    }
    take_directive_cooldown(world)?;

    // Contradiction: the administration argues with itself, in writing.
    let active = world.resource::<GovernorState>().active_directive.map(|(k, _)| k);
    let mut contradiction_note = String::new();
    if let Some(active_kind) = active {
        if active_kind.contradicts(kind) {
            let penalty = contradiction_penalty(active_kind, kind);
            adjust_legitimacy(world, -penalty);
            contradiction_note = format!(
                " The administration contradicts itself in writing (-{penalty:.2} legitimacy)."
            );
            send_chronicle(
                world,
                &format!(
                    "{} contradicts {}. The clerks' morale plummets.",
                    kind.name(),
                    active_kind.name()
                ),
                EventImportance::Standard,
            );
        }
    }

    let legitimacy = governor_legitimacy(world);
    match kind {
        DirectiveKind::Quota => {
            let bonus = QUOTA_BONUS_BASE + QUOTA_BONUS_LEGITIMACY_SCALE * legitimacy;
            {
                let mut quota = world.resource_mut::<QuotaOrder>();
                quota.bonus = bonus;
                quota.remaining = QUOTA_DURATION;
            }
            let baseline = world
                .get_resource::<crate::layer1::culture::sovereign::FarmProductionLedger>()
                .map_or(0.0, |l| l.cumulative);
            {
                let mut state = world.resource_mut::<GovernorState>();
                state.active_directive = Some((DirectiveKind::Quota, QUOTA_DURATION));
                state.quota_judgment = Some(QuotaJudgment {
                    judge_in: QUOTA_JUDGMENT_DELAY,
                    baseline,
                });
            }
            Ok(format!(
                "{} proclaimed: +{bonus:.2} work utility for {QUOTA_DURATION} ticks.{}",
                kind.name(),
                contradiction_note
            ))
        }
        DirectiveKind::Ration => {
            {
                let mut ration = world.resource_mut::<RationOrder>();
                ration.remaining = RATION_DURATION;
            }
            {
                let mut state = world.resource_mut::<GovernorState>();
                state.active_directive = Some((DirectiveKind::Ration, RATION_DURATION));
            }
            Ok(format!(
                "{} proclaimed: meals are {}% of normal size for {RATION_DURATION} ticks. The \
                 colony is asked to tighten its belts, officially.{}",
                kind.name(),
                (RATION_MEAL_FRACTION * 100.0) as u32,
                contradiction_note
            ))
        }
        DirectiveKind::Requisition => {
            let mut seized = 0.0;
            let mut victims = 0u32;
            let governor = world.resource::<GovernorState>().governor;
            let mut query = world.query::<(Entity, &mut Wallet)>();
            for (entity, mut wallet) in query.iter_mut(world) {
                if Some(entity) == governor {
                    continue;
                }
                let take = wallet.credits * REQUISITION_RATE;
                wallet.credits -= take;
                seized += take;
                victims += 1;
            }
            {
                let mut treasury = world.resource_mut::<Treasury>();
                treasury.credits += seized;
            }
            record_flow(world, "Requisition order", seized);
            adjust_legitimacy(world, -REQUISITION_LEGITIMACY_COST);
            Ok(format!(
                "{} executed: {seized:.1}cr requisitioned from {victims} pops \
                 (-{REQUISITION_LEGITIMACY_COST:.2} legitimacy).{}",
                kind.name(),
                contradiction_note
            ))
        }
        DirectiveKind::Works => {
            spend_treasury(world, "Public works program", WORKS_COST);
            {
                let mut works = world.resource_mut::<WorksProgram>();
                works.remaining = WORKS_DURATION;
            }
            {
                let mut state = world.resource_mut::<GovernorState>();
                state.active_directive = Some((DirectiveKind::Works, WORKS_DURATION));
            }
            adjust_legitimacy(world, WORKS_LEGITIMACY);
            Ok(format!(
                "{} launched: {WORKS_COST:.1}cr spent, +{WORKS_MORALE:.2} morale for \
                 {WORKS_DURATION} ticks. The ribbon is cut with ceremonial scissors \
                 (+{WORKS_LEGITIMACY:.2} legitimacy).{}",
                kind.name(),
                contradiction_note
            ))
        }
    }
}

/// Collect the colonial tithe: a revenue-service levy on every pop's wallet.
pub fn collect_tithe(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let mut collected = 0.0;
    let mut payers = 0u32;
    let governor = world.resource::<GovernorState>().governor;
    let mut query = world.query::<(Entity, &mut Wallet)>();
    for (entity, mut wallet) in query.iter_mut(world) {
        if Some(entity) == governor {
            continue;
        }
        let take = wallet.credits * TITHE_RATE;
        wallet.credits -= take;
        collected += take;
        payers += 1;
    }
    {
        let mut treasury = world.resource_mut::<Treasury>();
        treasury.credits += collected;
    }
    record_flow(world, "Tithe collection", collected);
    let legitimacy = governor_legitimacy(world);
    let (delta, verdict) = if legitimacy >= TITHE_LEGITIMACY_GATE {
        (TITHE_LEGITIMACY_GOOD, "The forms were filed correctly.")
    } else {
        (TITHE_LEGITIMACY_BAD, "The pops grumble about the forms.")
    };
    adjust_legitimacy(world, delta);
    Ok(format!(
        "Tithe collected: {collected:.1}cr from {payers} pops. {verdict} ({delta:+.2} legitimacy)."
    ))
}

/// Hold a public hearing of the Subcommittee on Matters.
pub fn hold_hearing(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let cd = world.resource::<GovernorState>().hearing_cooldown;
    if cd > 0 {
        return Err(format!(
            "The subcommittee is still deliberating ({cd} ticks remaining)."
        ));
    }
    let balance = world.resource::<Treasury>().credits;
    if balance < HEARING_COST {
        return Err(format!(
            "The court stenographer demands {HEARING_COST:.1}cr in advance; the treasury holds \
             only {balance:.1}cr."
        ));
    }
    spend_treasury(world, "Public hearing", HEARING_COST);
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.hearing_cooldown = HEARING_COOLDOWN;
        state.hearings_held += 1;
    }
    adjust_legitimacy(world, HEARING_LEGITIMACY);
    Ok(format!(
        "You convene a hearing of the Subcommittee on Matters. Four forms are filed, the minutes \
         are approved, and the matter is referred to a further subcommittee. \
         (+{HEARING_LEGITIMACY:.2} legitimacy, -{HEARING_COST:.1}cr.)"
    ))
}

/// File paperwork: Form 77-B, Request to File Forms.
pub fn file_paperwork(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let cd = world.resource::<GovernorState>().file_cooldown;
    if cd > 0 {
        return Err(format!("The filing cabinet is jammed ({cd} ticks remaining)."));
    }
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.file_cooldown = FILE_COOLDOWN;
        state.forms_filed += 1;
    }
    adjust_legitimacy(world, FILE_LEGITIMACY);
    Ok(format!(
        "You file Form 77-B (Request to File Forms). It feels correct. \
         (+{FILE_LEGITIMACY:.2} legitimacy.)"
    ))
}

/// Audit the treasury: expose embezzlers, or confirm the books balance.
pub fn audit_treasury(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let cd = world.resource::<GovernorState>().audit_cooldown;
    if cd > 0 {
        return Err(format!(
            "The auditors are still reconciling ({cd} ticks remaining)."
        ));
    }
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.audit_cooldown = AUDIT_COOLDOWN;
        state.spent_in_window = 0.0;
        state.spend_window = 0;
    }

    // Catch the first rival with dirty hands.
    let culprit = world
        .query::<(Entity, &RivalGovernor)>()
        .iter(world)
        .find(|(_, r)| r.skimmed > 0.0)
        .map(|(e, r)| (e, r.claim.clone(), r.skimmed));
    if let Some((entity, claim, skimmed)) = culprit {
        let recovered = skimmed * EMBEZZLER_RECOVERY_FRACTION;
        {
            let mut treasury = world.resource_mut::<Treasury>();
            treasury.credits += recovered;
        }
        record_flow(world, "Recovered embezzled funds", recovered);
        let name = world
            .get::<PopName>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| format!("pop #{}", entity.index()));
        world.despawn(entity);
        adjust_legitimacy(world, EMBEZZLER_EXPOSED_LEGITIMACY);
        send_chronicle(
            world,
            &format!(
                "{name}, {claim}, is led away in ceremonial handcuffs. The ledgers balance again."
            ),
            EventImportance::Standard,
        );
        return Ok(format!(
            "AUDIT: {name} ({claim}) skimmed {skimmed:.1}cr. They are led away in ceremonial \
             handcuffs; {recovered:.1}cr recovered (the rest went to ceremonial stationery). \
             (+{EMBEZZLER_EXPOSED_LEGITIMACY:.2} legitimacy.)"
        ));
    }

    adjust_legitimacy(world, AUDIT_LEGITIMACY);
    let balance = world.resource::<Treasury>().credits;
    Ok(format!(
        "AUDIT: the books are opened. Every credit is accounted for; the treasury holds \
         {balance:.1}cr. The auditors nod, once. (+{AUDIT_LEGITIMACY:.2} legitimacy.)"
    ))
}

/// Describe the treasury: balance and recent flows.
#[must_use]
pub fn describe_treasury(world: &mut World) -> String {
    ensure_governor_resources(world);
    let balance = world.resource::<Treasury>().credits;
    let state = world.resource::<GovernorState>();
    let mut out = format!("TREASURY: {balance:.1}cr\n");
    if state.flows.is_empty() {
        out.push_str("No flows recorded. The ledger is blank and faintly judgmental.");
    } else {
        for (label, amount) in &state.flows {
            let sign = if *amount >= 0.0 { "+" } else { "" };
            out.push_str(&format!("\n  {label}: {sign}{amount:.1}cr"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Rival administrators
// ---------------------------------------------------------------------------

/// Spawn a rival claimant near the lander (migrant-spawned, sealed writ).
pub fn spawn_rival_governor(world: &mut World) -> Option<Entity> {
    ensure_governor_resources(world);
    let rivals = world.query::<&RivalGovernor>().iter(world).count();
    if rivals >= RIVAL_MAX {
        return None;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let lander = lander?;

    // Find a walkable tile within a small ring of the lander.
    let pos = {
        let terrain = world.get_resource::<TerrainGrid>()?;
        let mut found = None;
        'search: for radius in 1..=4i32 {
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    if dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let x = lander.x + dx;
                    let y = lander.y + dy;
                    if x < 0 || y < 0 {
                        continue;
                    }
                    if let Some(tt) = terrain.get(x as usize, y as usize) {
                        if !matches!(tt, TerrainType::Water | TerrainType::Void) {
                            found = Some(GridPosition { x, y });
                            break 'search;
                        }
                    }
                }
            }
        }
        found?
    };

    let mut rng = rand::thread_rng();
    let claim = RIVAL_CLAIMS[rng.gen_range(0..RIVAL_CLAIMS.len())].to_string();
    let mut bundle = PopBundle::random(pos.x, pos.y, &mut rng);
    // Rivals arrive with a stake: a funded wallet, ready to file forms.
    bundle.wallet = Wallet { credits: 100.0 };
    let name = bundle.name.0.clone();
    let entity = world.spawn(bundle).id();
    world.entity_mut(entity).insert(RivalGovernor {
        legitimacy: RIVAL_GOVERNOR_STARTING_LEGITIMACY,
        claim: claim.clone(),
        skimmed: 0.0,
    });
    send_chronicle(
        world,
        &format!(
            "{name} arrives with a sealed writ of their own, claiming to be the {claim}. \
             The bureaucracy now has two heads, and neither is consulting the other."
        ),
        EventImportance::Standard,
    );
    Some(entity)
}

/// Pure debate resolution: `roll` in 0.0..1.0; the player wins when the roll
/// falls inside their share of the combined legitimacy.
#[must_use]
pub fn resolve_debate(player_legitimacy: f32, rival_legitimacy: f32, roll: f32) -> bool {
    let total = (player_legitimacy + rival_legitimacy).max(f32::EPSILON);
    roll < player_legitimacy / total
}

/// Debate a rival: a public contest of procedure.
pub fn debate_rival(world: &mut World, pop: Entity, rival_id: u32) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let rival = world
        .query::<(Entity, &RivalGovernor)>()
        .iter(world)
        .find(|(e, _)| e.index() == rival_id)
        .map(|(e, r)| (e, r.clone()));
    let Some((entity, rival_comp)) = rival else {
        return Err(format!("No rival governor with id {rival_id}."));
    };
    let player_legitimacy = governor_legitimacy(world);
    let mut rng = rand::thread_rng();
    let roll: f32 = rng.gen();
    if resolve_debate(player_legitimacy, rival_comp.legitimacy, roll) {
        let name = world
            .get::<PopName>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| format!("pop #{}", entity.index()));
        world.despawn(entity);
        adjust_legitimacy(world, DEBATE_WIN_LEGITIMACY);
        send_chronicle(
            world,
            &format!(
                "{name}'s writ is found to be missing Appendix C. They withdraw to file a complaint."
            ),
            EventImportance::Standard,
        );
        Ok(format!(
            "DEBATE: {name}'s writ is missing Appendix C. They withdraw to file a complaint. \
             (+{DEBATE_WIN_LEGITIMACY:.2} legitimacy.)"
        ))
    } else {
        if let Some(mut r) = world.get_mut::<RivalGovernor>(entity) {
            r.legitimacy = clamp01(r.legitimacy + DEBATE_WIN_LEGITIMACY);
        }
        adjust_legitimacy(world, DEBATE_LOSE_LEGITIMACY);
        let name = world
            .get::<PopName>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| format!("pop #{}", entity.index()));
        Ok(format!(
            "DEBATE: {name} cites precedent you have never heard of. The crowd of clerks murmurs. \
             ({DEBATE_LOSE_LEGITIMACY:.2} legitimacy.)"
        ))
    }
}

/// Purge a rival: remove them by fiat.
pub fn purge_rival(world: &mut World, pop: Entity, rival_id: u32) -> Result<String, String> {
    ensure_governor_resources(world);
    require_governor(world, pop)?;
    let rival = world
        .query::<(Entity, &RivalGovernor)>()
        .iter(world)
        .find(|(e, _)| e.index() == rival_id)
        .map(|(e, r)| (e, r.claim.clone()));
    let Some((entity, claim)) = rival else {
        return Err(format!("No rival governor with id {rival_id}."));
    };
    let name = world
        .get::<PopName>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", entity.index()));
    world.despawn(entity);
    adjust_legitimacy(world, -PURGE_LEGITIMACY_COST);
    send_chronicle(
        world,
        &format!(
            "{name}, {claim}, is purged by administrative fiat. The forms were signed in a \
             locked room."
        ),
        EventImportance::Standard,
    );
    Ok(format!(
        "PURGE: {name} ({claim}) is removed by administrative fiat. The forms were signed in a \
         locked room. (-{PURGE_LEGITIMACY_COST:.2} legitimacy.)"
    ))
}

/// Describe the political field: the governor and every rival claimant.
#[must_use]
pub fn describe_political_field(world: &mut World) -> String {
    ensure_governor_resources(world);
    let mut out = String::from("POLITICAL FIELD\n");
    match world.resource::<GovernorState>().governor {
        Some(e) => {
            let name = world
                .get::<PopName>(e)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| format!("pop #{}", e.index()));
            let legitimacy = governor_legitimacy(world);
            out.push_str(&format!(
                "  YOU: {name} (id {}), duly appointed, legitimacy {legitimacy:.2}\n",
                e.index()
            ));
        }
        None => out.push_str("  YOU: nobody holds the seal.\n"),
    }
    let rivals: Vec<(u32, String, String, f32, f32)> = world
        .query::<(Entity, &RivalGovernor, &PopName)>()
        .iter(world)
        .map(|(e, r, n)| (e.index(), n.0.clone(), r.claim.clone(), r.legitimacy, r.skimmed))
        .collect();
    if rivals.is_empty() {
        out.push_str("  Rivals: none. The field is clear and the forms are quiet.");
    } else {
        out.push_str(&format!("  Rivals ({}):", rivals.len()));
        for (id, name, claim, legitimacy, skimmed) in rivals {
            out.push_str(&format!(
                "\n    [{id}] {name} — {claim}, legitimacy {legitimacy:.2}, skimmed {skimmed:.1}cr"
            ));
        }
        out.push_str("\n  Counter them with `debate <id>`, `purge <id>`, or `audit`.");
    }
    out
}

// ---------------------------------------------------------------------------
// Tick, morale, stats
// ---------------------------------------------------------------------------

/// Advance the governor origin one tick: cooldowns, directive timers, quota
/// judgments, rival arrivals and undermining, embezzlement, scandals.
#[allow(clippy::cast_precision_loss)]
pub fn governor_tick(world: &mut World) {
    ensure_governor_resources(world);

    // Cooldowns.
    {
        let mut state = world.resource_mut::<GovernorState>();
        state.directive_cooldown = state.directive_cooldown.saturating_sub(1);
        state.hearing_cooldown = state.hearing_cooldown.saturating_sub(1);
        state.file_cooldown = state.file_cooldown.saturating_sub(1);
        state.audit_cooldown = state.audit_cooldown.saturating_sub(1);
    }

    // Quota broadcast expiry.
    {
        let mut quota = world.resource_mut::<QuotaOrder>();
        if quota.remaining > 0 {
            quota.remaining -= 1;
            if quota.remaining == 0 {
                quota.bonus = 0.0;
            }
        }
    }
    // Ration expiry.
    {
        let mut ration = world.resource_mut::<RationOrder>();
        if ration.remaining > 0 {
            ration.remaining -= 1;
        }
    }
    // Works program expiry.
    {
        let mut works = world.resource_mut::<WorksProgram>();
        if works.remaining > 0 {
            works.remaining -= 1;
        }
    }
    // Active-directive bookkeeping (drives the contradiction matrix).
    {
        let mut state = world.resource_mut::<GovernorState>();
        if let Some((kind, remaining)) = state.active_directive {
            let next = remaining.saturating_sub(1);
            state.active_directive = if next == 0 { None } else { Some((kind, next)) };
        }
    }

    // Quota judgment: did production actually rise?
    let judgment = {
        let mut state = world.resource_mut::<GovernorState>();
        let mut fired = None;
        if let Some(j) = state.quota_judgment.as_mut() {
            j.judge_in = j.judge_in.saturating_sub(1);
            if j.judge_in == 0 {
                fired = Some(j.baseline);
                state.quota_judgment = None;
            }
        }
        fired
    };
    if let Some(baseline) = judgment {
        let current = world
            .get_resource::<crate::layer1::culture::sovereign::FarmProductionLedger>()
            .map_or(0.0, |l| l.cumulative);
        let landed = current > baseline;
        adjust_legitimacy(
            world,
            if landed { QUOTA_JUDGMENT } else { -QUOTA_JUDGMENT },
        );
        send_chronicle(
            world,
            if landed {
                "The quota figures are in: production rose. The directive is vindicated, in triplicate."
            } else {
                "The quota figures are in: production did not rise. The forms are quietly refiled."
            },
            EventImportance::Minor,
        );
    }

    // Governor liveness: death drops the seal where they fell.
    let gov_alive = {
        let gov = world.resource::<GovernorState>().governor;
        match gov {
            Some(e) => {
                if world.get_entity(e).is_ok() {
                    if let Some(pos) = world.get::<GridPosition>(e).copied() {
                        world.resource_mut::<GovernorState>().last_known_pos = Some(pos);
                    }
                    true
                } else {
                    handle_governor_death(world);
                    false
                }
            }
            None => false,
        }
    };
    {
        let count = world.query::<&Pop>().iter(world).count();
        world.resource_mut::<GovernorState>().prev_pop_count = count;
    }

    // Rival arrivals (only while someone holds the seal).
    if gov_alive {
        let due = {
            let mut state = world.resource_mut::<GovernorState>();
            if state.rival_timer > 0 {
                state.rival_timer -= 1;
            }
            let due = state.rival_timer == 0;
            if due {
                state.rival_timer = RIVAL_ARRIVAL_INTERVAL;
            }
            due
        };
        if due {
            spawn_rival_governor(world);
        }
    }

    // Rival undermining: every 100 ticks each rival drains legitimacy and
    // files their own forms (drifting upward).
    if gov_alive {
        let fire = {
            let mut state = world.resource_mut::<GovernorState>();
            state.undermine_accum += 1;
            let fire = state.undermine_accum >= 100;
            if fire {
                state.undermine_accum = 0;
            }
            fire
        };
        if fire {
            let rival_count = world.query::<&RivalGovernor>().iter(world).count() as f32;
            if rival_count > 0.0 {
                adjust_legitimacy(world, -RIVAL_UNDERMINE_PER_100 * rival_count);
                let mut query = world.query::<&mut RivalGovernor>();
                for mut r in query.iter_mut(world) {
                    r.legitimacy = clamp01(r.legitimacy + RIVAL_LEGITIMACY_DRIFT);
                }
            }
        }
    }

    // Rival embezzlement: skim the treasury into their own wallets.
    if gov_alive {
        let balance = world.resource::<Treasury>().credits;
        if balance > 0.0 {
            let mut skimmed_total = 0.0;
            let mut query = world.query::<(Entity, &mut RivalGovernor, &mut Wallet)>();
            for (_, mut rival, mut wallet) in query.iter_mut(world) {
                let skim = RIVAL_SKIM_PER_TICK.min(balance - skimmed_total).max(0.0);
                if skim <= 0.0 {
                    break;
                }
                rival.skimmed += skim;
                wallet.credits += skim;
                skimmed_total += skim;
            }
            if skimmed_total > 0.0 {
                let mut treasury = world.resource_mut::<Treasury>();
                treasury.credits -= skimmed_total;
                record_flow(world, "Unexplained shrinkage", -skimmed_total);
            }
        }
    }

    // Drain scandal: spend a lot, end at zero, the ledgers open to the public.
    if gov_alive {
        let (spent, fired) = {
            let mut state = world.resource_mut::<GovernorState>();
            if state.spent_in_window > 0.0 && state.spend_window == 0 {
                state.spend_window = DRAIN_SCANDAL_WINDOW;
            }
            if state.spend_window > 0 {
                state.spend_window -= 1;
                if state.spend_window == 0 {
                    state.spent_in_window = 0.0;
                }
            }
            (state.spent_in_window, state.drain_scandal_fired)
        };
        let balance = world.resource::<Treasury>().credits;
        if !fired && spent >= DRAIN_SCANDAL_SPEND && balance <= 0.001 {
            adjust_legitimacy(world, -DRAIN_SCANDAL_LEGITIMACY);
            {
                let mut state = world.resource_mut::<GovernorState>();
                state.drain_scandal_fired = true;
                state.spent_in_window = 0.0;
                state.spend_window = 0;
            }
            send_chronicle(
                world,
                "The treasury is empty. The ledgers are opened to the public, and the public \
                 has questions.",
                EventImportance::Standard,
            );
        }
    }
}

/// Refresh the governor's morale modifiers (rationing, public works).
/// Runs before the morale cache update, like the constellation broadcaster.
pub fn apply_governor_morale(world: &mut World) {
    ensure_governor_resources(world);
    let (ration_on, works_on) = (
        world.resource::<RationOrder>().remaining > 0,
        world.resource::<WorksProgram>().remaining > 0,
    );
    let mut query = world.query_filtered::<&mut Morale, With<Pop>>();
    for mut morale in query.iter_mut(world) {
        morale.modifiers.retain(|m| {
            m.label != RATION_MODIFIER_LABEL && m.label != WORKS_MODIFIER_LABEL
        });
        if ration_on {
            morale.add_modifier(MoodModifier {
                label: RATION_MODIFIER_LABEL.to_string(),
                value: RATION_MORALE,
                duration: 2,
            });
        }
        if works_on {
            morale.add_modifier(MoodModifier {
                label: WORKS_MODIFIER_LABEL.to_string(),
                value: WORKS_MORALE,
                duration: 2,
            });
        }
    }
}

/// Governor stats for the headless STATS line:
/// (governor pop id, legitimacy, treasury balance, rival count).
#[must_use]
pub fn governor_stats(world: &mut World) -> (Option<u32>, f32, f32, usize) {
    ensure_governor_resources(world);
    let id = world
        .resource::<GovernorState>()
        .governor
        .map(|e| e.index());
    let legitimacy = governor_legitimacy(world);
    let treasury = world.resource::<Treasury>().credits;
    let rivals = world.query::<&RivalGovernor>().iter(world).count();
    (id, legitimacy, treasury, rivals)
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::culture::sovereign::FarmProductionLedger;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::utility_types::PopAction;
    use crate::shared::time::SimulationTime;

    // --- test scaffolding -------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(GovernorState::default());
        world.insert_resource(Treasury::default());
        world.insert_resource(QuotaOrder::default());
        world.insert_resource(RationOrder::default());
        world.insert_resource(WorksProgram::default());
        world.insert_resource(FarmProductionLedger::default());
        world
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName("Testy".to_string()),
                GridPosition { x, y },
                Morale::default(),
                Wallet::default(),
                PopAction::default(),
            ))
            .id()
    }

    fn spawn_seal(world: &mut World, x: i32, y: i32) -> Entity {
        world.spawn((AppointmentSeal, GridPosition { x, y })).id()
    }

    /// Spawn a pop next to a seal and sign: returns the appointed governor.
    fn appoint(world: &mut World) -> Entity {
        let pop = spawn_pop(world, 4, 4);
        spawn_seal(world, 5, 5);
        try_sign_seal(world, pop).expect("signing should succeed");
        pop
    }

    fn set_legitimacy(world: &mut World, v: f32) {
        let gov = world.resource::<GovernorState>().governor.unwrap();
        world.get_mut::<Governor>(gov).unwrap().legitimacy = v;
    }

    fn tick_n(world: &mut World, n: u32) {
        for _ in 0..n {
            governor_tick(world);
        }
    }

    // --- the seal ---------------------------------------------------------

    #[test]
    fn sign_fails_with_no_seal_in_reach() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 0, 0);
        spawn_seal(&mut world, 50, 50);
        assert!(try_sign_seal(&mut world, pop).is_err());
    }

    #[test]
    fn sign_appoints_governor() {
        let mut world = setup();
        let pop = appoint(&mut world);
        let gov = world.get::<Governor>(pop).expect("governor component");
        assert!((gov.legitimacy - GOVERNOR_STARTING_LEGITIMACY).abs() < f32::EPSILON);
        assert_eq!(world.query::<&AppointmentSeal>().iter(&world).count(), 0);
        assert_eq!(
            world.resource::<GovernorState>().governor,
            Some(pop)
        );
    }

    #[test]
    fn sign_twice_fails() {
        let mut world = setup();
        let pop = appoint(&mut world);
        spawn_seal(&mut world, 5, 5);
        assert!(try_sign_seal(&mut world, pop).is_err());
    }

    #[test]
    fn resign_drops_seal_and_clears_effects() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<QuotaOrder>().bonus = 0.4;
        resign(&mut world, pop).expect("resign");
        assert!(world.get::<Governor>(pop).is_none());
        assert_eq!(world.query::<&AppointmentSeal>().iter(&world).count(), 1);
        assert_eq!(world.resource::<QuotaOrder>().bonus, 0.0);
        assert_eq!(world.resource::<GovernorState>().governor, None);
    }

    #[test]
    fn governor_death_drops_seal() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.despawn(pop);
        governor_tick(&mut world);
        assert_eq!(world.resource::<GovernorState>().governor, None);
        assert_eq!(world.query::<&AppointmentSeal>().iter(&world).count(), 1);
    }

    #[test]
    fn seal_spawn_guard_is_idempotent() {
        let mut world = setup();
        world.resource_mut::<GovernorState>().seal_spawn_attempted = true;
        spawn_appointment_seal_once(&mut world);
        assert_eq!(world.query::<&AppointmentSeal>().iter(&world).count(), 0);
    }

    // --- directives --------------------------------------------------------

    #[test]
    fn directive_requires_governor() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 0, 0);
        assert!(issue_directive(&mut world, pop, DirectiveKind::Quota).is_err());
    }

    #[test]
    fn directive_cooldown_blocks_second_issue() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("first");
        assert!(issue_directive(&mut world, pop, DirectiveKind::Ration).is_err());
    }

    #[test]
    fn quota_requires_minimum_legitimacy() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.2);
        let err = issue_directive(&mut world, pop, DirectiveKind::Quota).unwrap_err();
        assert!(err.contains("unsigned"), "unexpected: {err}");
    }

    #[test]
    fn rejected_directive_does_not_consume_cooldown() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.2);
        assert!(issue_directive(&mut world, pop, DirectiveKind::Quota).is_err());
        assert_eq!(world.resource::<GovernorState>().directive_cooldown, 0);
        assert!(issue_directive(&mut world, pop, DirectiveKind::Works).is_err());
        assert_eq!(world.resource::<GovernorState>().directive_cooldown, 0);
    }

    #[test]
    fn quota_sets_legitimacy_scaled_bonus() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("quota");
        let quota = world.resource::<QuotaOrder>();
        let expected = QUOTA_BONUS_BASE + QUOTA_BONUS_LEGITIMACY_SCALE * 0.8;
        assert!((quota.bonus - expected).abs() < 1e-5);
        assert_eq!(quota.remaining, QUOTA_DURATION);
    }

    #[test]
    fn quota_judgment_rewards_rising_production() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("quota");
        world.resource_mut::<FarmProductionLedger>().cumulative = 250.0;
        tick_n(&mut world, QUOTA_JUDGMENT_DELAY);
        let legit = governor_legitimacy(&mut world);
        assert!((legit - (0.8 + QUOTA_JUDGMENT)).abs() < 1e-5, "legit={legit}");
    }

    #[test]
    fn quota_judgment_punishes_flat_production() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("quota");
        // Ledger stays at 0: no production happened.
        tick_n(&mut world, QUOTA_JUDGMENT_DELAY);
        let legit = governor_legitimacy(&mut world);
        assert!((legit - (0.8 - QUOTA_JUDGMENT)).abs() < 1e-5, "legit={legit}");
    }

    #[test]
    fn quota_expiry_clears_bonus() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("quota");
        tick_n(&mut world, QUOTA_DURATION);
        let quota = world.resource::<QuotaOrder>();
        assert_eq!(quota.bonus, 0.0);
        assert_eq!(quota.remaining, 0);
    }

    #[test]
    fn ration_activates_order() {
        let mut world = setup();
        let pop = appoint(&mut world);
        issue_directive(&mut world, pop, DirectiveKind::Ration).expect("ration");
        assert_eq!(world.resource::<RationOrder>().remaining, RATION_DURATION);
    }

    #[test]
    fn requisition_seizes_wallets_into_treasury() {
        let mut world = setup();
        let gov = appoint(&mut world);
        world.get_mut::<Wallet>(gov).unwrap().credits = 1000.0;
        let other = spawn_pop(&mut world, 9, 9);
        world.get_mut::<Wallet>(other).unwrap().credits = 200.0;
        issue_directive(&mut world, gov, DirectiveKind::Requisition).expect("requisition");
        // Governor's own wallet untouched; 15% of the other's 200 seized.
        assert!((world.get::<Wallet>(gov).unwrap().credits - 1000.0).abs() < 1e-4);
        assert!((world.get::<Wallet>(other).unwrap().credits - 170.0).abs() < 1e-4);
        assert!((world.resource::<Treasury>().credits - 30.0).abs() < 1e-4);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY - REQUISITION_LEGITIMACY_COST)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn works_requires_treasury_funds() {
        let mut world = setup();
        let pop = appoint(&mut world);
        let err = issue_directive(&mut world, pop, DirectiveKind::Works).unwrap_err();
        assert!(err.contains("treasury"), "unexpected: {err}");
    }

    #[test]
    fn works_spends_and_boosts_morale_program() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 100.0;
        issue_directive(&mut world, pop, DirectiveKind::Works).expect("works");
        assert!((world.resource::<Treasury>().credits - 50.0).abs() < 1e-4);
        assert_eq!(world.resource::<WorksProgram>().remaining, WORKS_DURATION);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + WORKS_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn contradiction_quota_then_ration_costs_legitimacy() {
        let mut world = setup();
        let pop = appoint(&mut world);
        set_legitimacy(&mut world, 0.8);
        issue_directive(&mut world, pop, DirectiveKind::Quota).expect("quota");
        world.resource_mut::<GovernorState>().directive_cooldown = 0;
        issue_directive(&mut world, pop, DirectiveKind::Ration).expect("ration");
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (0.8 - QUOTA_RATION_TENSION)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn contradiction_works_then_ration_costs_more() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 100.0;
        issue_directive(&mut world, pop, DirectiveKind::Works).expect("works");
        world.resource_mut::<GovernorState>().directive_cooldown = 0;
        issue_directive(&mut world, pop, DirectiveKind::Ration).expect("ration");
        let legit = governor_legitimacy(&mut world);
        let expected = GOVERNOR_STARTING_LEGITIMACY + WORKS_LEGITIMACY - CONTRADICTION_LEGITIMACY;
        assert!((legit - expected).abs() < 1e-5, "legit={legit}");
    }

    #[test]
    fn directive_names_are_original_bureaucracy() {
        assert_eq!(DirectiveKind::Quota.name(), "Directive 12-A: Work Quotas");
        assert_eq!(DirectiveKind::Ration.name(), "Directive 7-C: Rationing Schedule");
        assert!(!DirectiveKind::Quota.contradicts(DirectiveKind::Works));
        assert!(DirectiveKind::Quota.contradicts(DirectiveKind::Ration));
        assert!(DirectiveKind::Ration.contradicts(DirectiveKind::Works));
    }

    // --- revenue & procedure -------------------------------------------------

    #[test]
    fn tithe_collects_and_judges_process() {
        let mut world = setup();
        let gov = appoint(&mut world);
        let other = spawn_pop(&mut world, 9, 9);
        world.get_mut::<Wallet>(other).unwrap().credits = 200.0;
        collect_tithe(&mut world, gov).expect("tithe");
        assert!((world.resource::<Treasury>().credits - 10.0).abs() < 1e-4);
        // 0.5 legitimacy < gate: the pops grumble.
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + TITHE_LEGITIMACY_BAD)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn tithe_rewards_proper_procedure() {
        let mut world = setup();
        let gov = appoint(&mut world);
        set_legitimacy(&mut world, 0.9);
        collect_tithe(&mut world, gov).expect("tithe");
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (0.9 + TITHE_LEGITIMACY_GOOD)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn hearing_costs_and_rewards() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 100.0;
        hold_hearing(&mut world, pop).expect("hearing");
        assert!((world.resource::<Treasury>().credits - 90.0).abs() < 1e-4);
        assert_eq!(world.resource::<GovernorState>().hearings_held, 1);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + HEARING_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
        assert!(hold_hearing(&mut world, pop).is_err());
    }

    #[test]
    fn hearing_requires_stenographer_funds() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 5.0;
        assert!(hold_hearing(&mut world, pop).is_err());
    }

    #[test]
    fn filing_paperwork_feels_correct() {
        let mut world = setup();
        let pop = appoint(&mut world);
        file_paperwork(&mut world, pop).expect("file");
        assert_eq!(world.resource::<GovernorState>().forms_filed, 1);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + FILE_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
        assert!(file_paperwork(&mut world, pop).is_err());
    }

    #[test]
    fn clean_audit_rewards_procedure() {
        let mut world = setup();
        let pop = appoint(&mut world);
        audit_treasury(&mut world, pop).expect("audit");
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + AUDIT_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn audit_exposes_embezzler() {
        let mut world = setup();
        let pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "Acting Governor (provisional)".to_string(),
            skimmed: 10.0,
        });
        audit_treasury(&mut world, pop).expect("audit");
        assert_eq!(world.query::<&RivalGovernor>().iter(&world).count(), 0);
        assert!((world.resource::<Treasury>().credits - 5.0).abs() < 1e-4);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY + EMBEZZLER_EXPOSED_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    // --- rivals ----------------------------------------------------------------

    #[test]
    fn rival_spawn_capped() {
        let mut world = setup();
        for _ in 0..RIVAL_MAX {
            let r = spawn_pop(&mut world, 1, 1);
            world.entity_mut(r).insert(RivalGovernor {
                legitimacy: 0.35,
                claim: "x".to_string(),
                skimmed: 0.0,
            });
        }
        assert!(spawn_rival_governor(&mut world).is_none());
    }

    #[test]
    fn debate_resolution_is_legitimacy_weighted() {
        assert!(resolve_debate(0.9, 0.1, 0.5));
        assert!(!resolve_debate(0.1, 0.9, 0.5));
        assert!(!resolve_debate(0.0, 0.0, 0.5));
    }

    #[test]
    fn debate_unknown_id_errors() {
        let mut world = setup();
        let pop = appoint(&mut world);
        assert!(debate_rival(&mut world, pop, 4242).is_err());
    }

    #[test]
    fn debate_moves_legitimacy_somewhere() {
        let mut world = setup();
        let pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "x".to_string(),
            skimmed: 0.0,
        });
        debate_rival(&mut world, pop, rival.index()).expect("debate");
        let gone = world.query::<&RivalGovernor>().iter(&world).count() == 0;
        let rival_up = world
            .get::<RivalGovernor>(rival)
            .is_some_and(|r| r.legitimacy > 0.35);
        assert!(gone || rival_up, "debate must resolve somewhere");
    }

    #[test]
    fn purge_removes_rival_at_legitimacy_cost() {
        let mut world = setup();
        let pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "x".to_string(),
            skimmed: 0.0,
        });
        purge_rival(&mut world, pop, rival.index()).expect("purge");
        assert_eq!(world.query::<&RivalGovernor>().iter(&world).count(), 0);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY - PURGE_LEGITIMACY_COST)).abs() < 1e-5,
            "legit={legit}"
        );
    }

    #[test]
    fn rival_undermining_drains_legitimacy() {
        let mut world = setup();
        let _pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "x".to_string(),
            skimmed: 0.0,
        });
        world.resource_mut::<GovernorState>().undermine_accum = 99;
        governor_tick(&mut world);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY - RIVAL_UNDERMINE_PER_100)).abs() < 1e-5,
            "legit={legit}"
        );
        let r = world.get::<RivalGovernor>(rival).unwrap();
        assert!((r.legitimacy - (0.35 + RIVAL_LEGITIMACY_DRIFT)).abs() < 1e-5);
    }

    #[test]
    fn rival_skims_treasury() {
        let mut world = setup();
        let _pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "x".to_string(),
            skimmed: 0.0,
        });
        world.resource_mut::<Treasury>().credits = 100.0;
        governor_tick(&mut world);
        assert!((world.resource::<Treasury>().credits - 99.95).abs() < 1e-4);
        assert!((world.get::<Wallet>(rival).unwrap().credits - 0.05).abs() < 1e-5);
        assert!((world.get::<RivalGovernor>(rival).unwrap().skimmed - 0.05).abs() < 1e-5);
    }

    #[test]
    fn drain_scandal_fires_on_empty_treasury() {
        let mut world = setup();
        let _pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 0.0;
        {
            let mut state = world.resource_mut::<GovernorState>();
            state.spent_in_window = DRAIN_SCANDAL_SPEND + 5.0;
        }
        governor_tick(&mut world);
        let legit = governor_legitimacy(&mut world);
        assert!(
            (legit - (GOVERNOR_STARTING_LEGITIMACY - DRAIN_SCANDAL_LEGITIMACY)).abs() < 1e-5,
            "legit={legit}"
        );
        assert!(world.resource::<GovernorState>().drain_scandal_fired);
    }

    // --- field reports ------------------------------------------------------------

    #[test]
    fn political_field_lists_rivals() {
        let mut world = setup();
        let _pop = appoint(&mut world);
        let rival = spawn_pop(&mut world, 9, 9);
        world.entity_mut(rival).insert(RivalGovernor {
            legitimacy: 0.35,
            claim: "Interim Steward of the Ledgers".to_string(),
            skimmed: 0.0,
        });
        let text = describe_political_field(&mut world);
        assert!(text.contains("Interim Steward of the Ledgers"));
        assert!(text.contains("Rivals (1)"));
    }

    #[test]
    fn treasury_describes_flows() {
        let mut world = setup();
        let gov = appoint(&mut world);
        collect_tithe(&mut world, gov).ok();
        let text = describe_treasury(&mut world);
        assert!(text.contains("TREASURY:"));
        assert!(text.contains("Tithe collection"));
    }

    #[test]
    fn governor_stats_snapshot() {
        let mut world = setup();
        let pop = appoint(&mut world);
        world.resource_mut::<Treasury>().credits = 42.0;
        let (id, legit, treasury, rivals) = governor_stats(&mut world);
        assert_eq!(id, Some(pop.index()));
        assert!((legit - GOVERNOR_STARTING_LEGITIMACY).abs() < f32::EPSILON);
        assert!((treasury - 42.0).abs() < f32::EPSILON);
        assert_eq!(rivals, 0);
    }
}
