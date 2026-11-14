//! Spec 408: Galactic Tourism — beautiful colonies attract visitor ships.
//!
//! # Grounding audit (spec §8 adaptation pattern, specs 360–407)
//!
//! * `layer1::visitor::{Visitor, VisitorSpawning}` (spec sketch) —
//!   **PARTLY MOCK.** No `VisitorSpawning` exists and the real module is
//!   `layer1::entities::visitor`: [`Visitor`] + [`VisitorSource`] are
//!   REAL (spawn/lifecycle/behavior systems; visitors deliberately spawn
//!   WITHOUT `UtilityWeights`, so the main utility AI never assigns them
//!   colony jobs — spec §7's requirement is already the real seam's
//!   behavior). Tourists are real visitor entities carrying [`Visitor`]
//!   plus the new [`Tourist`] component; pending arrivals live in the
//!   new [`TourismState`] ledger instead of a parallel spawning system.
//! * `layer1::colony::ColonyStats { global_beauty, credits }` — **MOCK,
//!   does not exist.** The real `ColonyStats` (`economy::black_market`)
//!   carries only `unmet_luxury` + `corruption`. The real beauty seam is
//!   the spatial [`BeautyGrid`]; the colony beauty score is its tile sum
//!   ([`global_beauty`]). The real credits ledger is
//!   [`ColonyResources::credits`] with `add_credits` — arrival payments
//!   land there, never in a mock stats struct.
//! * Food consumption — **REAL** via `ColonyResources::food`, the same
//!   stockpile every pop eats from. Tourists eat from it on a cadence;
//!   that shared stockpile IS the spec §1 dilemma: when it runs dry
//!   while a liner is docked, the [`TourismPolicy`] choice bites —
//!   `DenyVisitors` starves the tourists (early departure + reputation
//!   penalty, boom ends) while `FeedVisitors` keeps feeding them first.
//! * Quarantine screening (§5) — **REAL seam, cheap leg taken.**
//!   Regular immigrants arrive as `ImmigrationStatus::Pending`
//!   (screened); tourists bypass screening entirely (no
//!   `ImmigrationStatus` is ever attached) and a deterministic fraction
//!   arrive carrying the real [`MindSporeInfection`] — the spec 406
//!   quarantine zones are then the colony's only containment tool.
//! * Morale/reputation — reputation is a tourism-local ledger in
//!   [`TourismState`] (no colony-wide reputation resource exists to
//!   reuse); needs cratering uses the real [`Needs`] component visitors
//!   already carry.
//!
//! # New constants (no existing balance constants were retuned)
//!
//! * [`BEAUTY_THRESHOLD`] `300.0` (spec §4), [`TOURIST_WEALTH`] `500.0`
//!   (spec RED test), [`TOURIST_STAY_TICKS`] `800`, [`EAT_INTERVAL`]
//!   `100`, [`EAT_AMOUNT`] `1.0`, [`REP_PENALTY_EARLY_DEPARTURE`] `0.25`,
//!   [`DISEASE_CARRIER_FRACTION`] `0.10`.
//!
//! # Structural fixes (2026-11-13, balance-census follow-up)
//!
//! The 2026-11-10 census found tourism uncompletable in practice
//! (138 arrivals, 138 early departures, 0 completed stays, reputation
//! 0.00 in 24/24 colonies). Three structural defects, three legs:
//!
//! * **Reputation was a one-way ratchet** — the only mutation was
//!   −0.25 per early departure and attraction gates at 0.5, so ~3 bad
//!   departures ended tourism forever. Completed stays now restore
//!   [`REP_GAIN_COMPLETED_STAY`] (capped at 1.0).
//! * **Stays were uncompletable** — tourists carry the real [`Needs`],
//!   `decay_needs_system` decays rest/leisure for every `Needs` entity
//!   (tourists included; hygiene decay requires a `Filth` component,
//!   which tourists do not carry, so hygiene is topped up defensively
//!   rather than because it currently decays), yet only hunger had a
//!   restoration leg (meals). Tourists
//!   never join utility AI, so nothing restored rest/leisure/hygiene
//!   and `needs.worst() < 0.2` fired ~tick 400 of an 800-tick stay.
//!   [`tourist_self_care_system`] is the vacation leg: on a cadence,
//!   present tourists recover rest/leisure/hygiene (leisure scaled by
//!   local `BeautyGrid` beauty, the same seam `apply_beauty_effects`
//!   uses for pops). No parallel needs system, no colony jobs.
//! * **Payment ignored outcome** — the full [`TOURIST_WEALTH`] banked
//!   on arrival. It is now split: [`TOURIST_ARRIVAL_PAYMENT`] on
//!   landing, [`TOURIST_COMPLETION_PAYMENT`] only on a completed stay
//!   (early departure forfeits it). The portions sum to the unchanged
//!   total, and `credits_earned` counts only money actually paid.
//!
//! * **Beauty-gate audit (verdict: gate unchanged).** The only
//!   production writer of `BeautyGrid` is `update_beauty_grid_system`,
//!   which clears the grid and recomputes a single mixed sum per tile:
//!   terrain (Grass +1 / Dirt −1 / Path −2 from `TerrainGrid`), clutter
//!   penalty, `BeautySource` entities with radius falloff, and waste
//!   items. There is no built-only accumulator or resource to read —
//!   worldgen terrain alone sums to ~2,100 on an 80×50 grass map, far
//!   over the 300 gate, and traffic erosion (Grass→Dirt→Path) decays
//!   that same sum. Separating built beauty would require a parallel
//!   grid inside `update_beauty_grid_system` (touching the beauty
//!   system every room-quality/window/cabin-fever consumer reads),
//!   which is not a cheap tourism-local seam, so [`BEAUTY_THRESHOLD`]
//!   and [`global_beauty`] are deliberately left unchanged.
//!
//! # Item 203 — reputation Chronicle telegraphing
//!
//! Completed stays were Major-chronicled, but no Chronicle line named
//! reputation itself: the recovery ([`REP_GAIN_COMPLETED_STAY`] per
//! completed stay) and the 0.5 boom threshold were visible only in
//! STATS / the tourists panel — the recovery was invisible on the
//! Chronicle fairness surface. [`tourist_departure_system`] is the
//! only place reputation mutates, so it now edge-triggers ONE Major
//! line per crossing of [`REPUTATION_BOOM_THRESHOLD`]: falling below
//! it (the boom closes) via
//! [`tourism_reputation_fallen_chronicle_text`], recovering back to
//! it via [`tourism_reputation_recovered_chronicle_text`] (which
//! names the per-stay gain). Edge-triggering IS the hysteresis: a
//! crossing line cannot repeat until reputation crosses back the
//! other way, so there is no per-change spam. Text wiring only —
//! no balance constants or behaviour changed.

use crate::layer1::beauty::BeautyGrid;
use crate::layer1::biology::symbiotic_insurgency::MindSporeInfection;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::visitor::{Visitor, VisitorSource, VisitorState};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::PopName;
use crate::layer1::psychology::needs::Needs;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Beauty score (sum of `BeautyGrid` tiles) at or above which visitor
/// ships start queueing tourists (spec §4 threshold).
pub const BEAUTY_THRESHOLD: f32 = 300.0;
/// Credits a tourist pays the colony on landing (spec RED test wealth).
pub const TOURIST_WEALTH: f32 = 500.0;
/// How long a tourist stays before departing normally.
pub const TOURIST_STAY_TICKS: u64 = 800;
/// Ticks between tourist meals.
pub const EAT_INTERVAL: u64 = 100;
/// Food drawn from the shared stockpile per tourist meal.
pub const EAT_AMOUNT: f32 = 1.0;
/// Hunger restored by one tourist meal.
pub const EAT_HUNGER_GAIN: f32 = 0.5;
/// Hunger lost when a meal is missed (no food, or policy denies it).
pub const MISSED_MEAL_HUNGER_LOSS: f32 = 0.3;
/// Reputation lost per early (needs-cratered) departure.
pub const REP_PENALTY_EARLY_DEPARTURE: f32 = 0.25;
/// Reputation restored per completed stay (structural fix: the
/// recovery leg that makes reputation two-way again; capped at 1.0).
pub const REP_GAIN_COMPLETED_STAY: f32 = 0.10;
/// Portion of [`TOURIST_WEALTH`] paid on landing. The remainder is
/// contingent on a completed stay (see [`TOURIST_COMPLETION_PAYMENT`]).
pub const TOURIST_ARRIVAL_PAYMENT: f32 = 100.0;
/// Portion of [`TOURIST_WEALTH`] paid only when a tourist completes
/// their stay; forfeited on early departure. Arrival + completion
/// portions sum to the unchanged [`TOURIST_WEALTH`] total (500).
pub const TOURIST_COMPLETION_PAYMENT: f32 = 400.0;
/// Ticks between tourist self-care recoveries (vacation leg).
pub const SELF_CARE_INTERVAL: u64 = 10;
/// Rest restored per self-care tick-interval.
pub const SELF_CARE_REST_GAIN: f32 = 0.03;
/// Leisure restored per self-care tick-interval (before beauty bonus).
pub const SELF_CARE_LEISURE_GAIN: f32 = 0.03;
/// Hygiene restored per self-care tick-interval.
pub const SELF_CARE_HYGIENE_GAIN: f32 = 0.03;
/// Extra leisure per point of local tile beauty per self-care tick.
pub const SELF_CARE_BEAUTY_BONUS_PER_POINT: f32 = 0.002;
/// Cap on the local-beauty leisure bonus per self-care tick.
pub const SELF_CARE_BEAUTY_BONUS_CAP: f32 = 0.02;
/// Fraction of tourists who bypass screening carrying a mind-spore
/// infection (deterministic per-arrival roll, see
/// [`tourist_carries_disease`]).
pub const DISEASE_CARRIER_FRACTION: f32 = 0.10;
/// Ticks between attraction evaluations.
pub const ATTRACTION_INTERVAL: u64 = 200;
/// Player-facing chronicle text when the first liner of a boom docks.
pub const TOURISM_BOOM_TEXT: &str =
    "A visitor liner has docked. Word of the colony's beauty is spreading.";
/// Player-facing chronicle text when a tourist leaves early, unhappy.
pub const TOURISM_EARLY_DEPARTURE_TEXT: &str =
    "A visitor has departed early, unimpressed. Word travels fast out there.";
/// Player-facing chronicle text when a tourist is found carrying spores.
pub const TOURISM_DISEASE_TEXT: &str =
    "A visitor who bypassed screening has been found carrying spores.";
/// Player-facing chronicle text when a tourist completes their stay.
pub const TOURISM_COMPLETED_TEXT: &str =
    "A visitor has completed their stay and departed happy. Word travels fast out there.";
/// Reputation at or above which visitor ships queue and the tourism
/// boom can hold; below it attraction stops (the gate read in
/// [`evaluate_tourist_attraction_system`]). Named so the item-203
/// Chronicle telegraphing reads the same value the gate uses — the
/// gate itself is unchanged.
pub const REPUTATION_BOOM_THRESHOLD: f32 = 0.5;

/// Player-facing chronicle text when tourism reputation falls below
/// [`REPUTATION_BOOM_THRESHOLD`] (item 203): names the new value and
/// the threshold it crossed.
#[must_use]
pub fn tourism_reputation_fallen_chronicle_text(reputation: f32) -> String {
    let threshold = REPUTATION_BOOM_THRESHOLD;
    format!(
        "Tourism reputation has fallen to {reputation:.2}, below the {threshold:.2} boom threshold. The tourism boom is over until completed stays rebuild it."
    )
}

/// Player-facing chronicle text when tourism reputation recovers
/// back to [`REPUTATION_BOOM_THRESHOLD`] (item 203): names the new
/// value, the threshold it crossed, and the per-stay recovery gain
/// (read from [`REP_GAIN_COMPLETED_STAY`], never hardcoded).
#[must_use]
pub fn tourism_reputation_recovered_chronicle_text(reputation: f32) -> String {
    let threshold = REPUTATION_BOOM_THRESHOLD;
    let gain = REP_GAIN_COMPLETED_STAY;
    format!(
        "Tourism reputation has recovered to {reputation:.2}, back above the {threshold:.2} boom threshold. Completed stays rebuild reputation (+{gain:.2} each) — visitors are spreading the word again."
    )
}

/// All item-203 player-facing reputation chronicle strings, sampled
/// at representative values, for the IP guard.
#[must_use]
pub fn tourism_reputation_public_strings() -> Vec<String> {
    vec![
        tourism_reputation_fallen_chronicle_text(0.25),
        tourism_reputation_recovered_chronicle_text(REPUTATION_BOOM_THRESHOLD),
    ]
}

/// The colony's food policy toward docked tourists (spec §1 dilemma).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TourismPolicy {
    /// Tourists eat from the shared stockpile like everyone else —
    /// workers may go hungry so the rich do not.
    #[default]
    FeedVisitors,
    /// Tourists are denied food: they crater, leave early, and the
    /// reputation hit ends the boom.
    DenyVisitors,
}

/// A paying guest. Always paired with the real [`Visitor`] tag, so the
/// visitor lifecycle owns arrival/departure states and the main utility
/// AI (which visitors never join) never assigns colony jobs.
#[derive(Component, Debug, Clone, Copy)]
pub struct Tourist {
    /// Total credits the tourist can pay the colony: the arrival
    /// portion on landing, the remainder only on a completed stay.
    pub wealth: f32,
    /// Tick the tourist landed.
    pub arrival_tick: u64,
    /// Tick the tourist plans to leave.
    pub departure_tick: u64,
}

/// Fired when a tourist entity lands; the arrival system pays its
/// wealth into the real credits ledger.
#[derive(Event, Debug, Clone, Copy)]
pub struct TouristArrivalEvent {
    /// The tourist entity that landed.
    pub tourist: Entity,
}

/// Tourism ledger + policy (first-tick pattern: initialized in
/// `simulation.rs`).
#[derive(Resource, Debug)]
pub struct TourismState {
    /// Tourists queued by attraction, waiting to land.
    pub pending_tourists: u32,
    /// Lifetime tourists landed.
    pub total_arrived: u32,
    /// Lifetime tourists who left early with cratered needs.
    pub total_departed_early: u32,
    /// Lifetime tourists who completed their stay.
    pub total_departed: u32,
    /// Colony tourism reputation, 0.0..=1.0. Below 0.5 the boom ends.
    pub reputation: f32,
    /// Whether a tourism boom is currently active.
    pub boom_active: bool,
    /// Lifetime meals denied by policy or an empty stockpile.
    pub denied_meals: u32,
    /// Lifetime tourists found carrying disease past screening.
    pub disease_imports: u32,
    /// Lifetime credits earned from tourism.
    pub credits_earned: f32,
    /// Last tick attraction was evaluated (cadence bookkeeping).
    pub last_attraction_tick: u64,
    /// Current food policy toward tourists.
    pub policy: TourismPolicy,
}

impl Default for TourismState {
    fn default() -> Self {
        Self {
            pending_tourists: 0,
            total_arrived: 0,
            total_departed_early: 0,
            total_departed: 0,
            reputation: 1.0,
            boom_active: false,
            denied_meals: 0,
            disease_imports: 0,
            credits_earned: 0.0,
            last_attraction_tick: 0,
            policy: TourismPolicy::FeedVisitors,
        }
    }
}

/// The colony beauty score: sum of all `BeautyGrid` tiles. A handful of
/// statues (10.0 each) plus gardens clears the spec threshold; a bare
/// dirt landing pad does not.
#[must_use]
pub fn global_beauty(grid: &BeautyGrid) -> f32 {
    grid.values.iter().sum()
}

/// Deterministic disease roll for the screening bypass: `roll` in
/// 0.0..1.0 carries iff it falls under the carrier fraction.
#[must_use]
pub fn tourist_carries_disease(roll: f32) -> bool {
    roll < DISEASE_CARRIER_FRACTION
}

/// Credits paid on landing for a tourist of the given wealth: the
/// fixed arrival portion, never more than the tourist carries.
#[must_use]
pub fn arrival_payment(wealth: f32) -> f32 {
    TOURIST_ARRIVAL_PAYMENT.min(wealth)
}

/// Credits paid on a completed stay for a tourist of the given
/// wealth: everything not already paid on arrival (forfeited on
/// early departure).
#[must_use]
pub fn completion_payment(wealth: f32) -> f32 {
    (wealth - arrival_payment(wealth)).max(0.0)
}

/// Spec §6: high beauty queues tourists. Evaluated on a cadence; each
/// evaluation queues one tourist per full threshold multiple, capped,
/// and only while reputation holds the boom together.
pub fn evaluate_tourist_attraction_system(
    grid: Option<Res<BeautyGrid>>,
    time: Res<SimulationTime>,
    mut state: ResMut<TourismState>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let Some(grid) = grid else { return };
    if time.tick < state.last_attraction_tick + ATTRACTION_INTERVAL {
        return;
    }
    state.last_attraction_tick = time.tick;
    if state.reputation < REPUTATION_BOOM_THRESHOLD {
        state.boom_active = false;
        return;
    }
    let beauty = global_beauty(&grid);
    if beauty < BEAUTY_THRESHOLD {
        return;
    }
    let queued = ((beauty / BEAUTY_THRESHOLD).floor() as u32).clamp(1, 5);
    state.pending_tourists += queued;
    if !state.boom_active {
        state.boom_active = true;
        if let Some(events) = chronicle.as_deref_mut() {
            events.send(AddChronicleEvent {
                text: TOURISM_BOOM_TEXT.to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// Lands queued tourists as real visitor entities and fires
/// [`TouristArrivalEvent`] for each. Tourists spawn with `Visitor` +
/// `Needs` + `PopName` (the real visitor bundle shape) and NO
/// `ImmigrationStatus` — the screening bypass of spec §5.
pub fn spawn_tourist_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    source: Option<Res<VisitorSource>>,
    mut state: ResMut<TourismState>,
    mut arrivals: EventWriter<TouristArrivalEvent>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    if state.pending_tourists == 0 {
        return;
    }
    let spawn_pos = source
        .as_deref()
        .and_then(|s| s.spawn_points.first().copied())
        .unwrap_or(GridPosition { x: 0, y: 0 });
    let mut rng = rand::thread_rng();
    while state.pending_tourists > 0 {
        state.pending_tourists -= 1;
        let entity = commands
            .spawn((
                Visitor {
                    state: VisitorState::Arriving,
                    arrival_tick: time.tick,
                    departure_tick: time.tick + TOURIST_STAY_TICKS,
                },
                Tourist {
                    wealth: TOURIST_WEALTH,
                    arrival_tick: time.tick,
                    departure_tick: time.tick + TOURIST_STAY_TICKS,
                },
                spawn_pos,
                PopName::random(&mut rng),
                Needs::default(),
            ))
            .id();
        arrivals.send(TouristArrivalEvent { tourist: entity });
        // Screening bypass: a fraction slip through carrying spores.
        if tourist_carries_disease(rng.gen::<f32>()) {
            commands
                .entity(entity)
                .insert(MindSporeInfection { active: true });
            state.disease_imports += 1;
            if let Some(events) = chronicle.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: TOURISM_DISEASE_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// Spec RED test 2, bridged + structural fix: arrival pays only the
/// arrival portion of the tourist's wealth into the real
/// `ColonyResources` credits ledger. The completion portion is paid
/// by [`tourist_departure_system`] on a completed stay only, and
/// `credits_earned` counts only money actually paid.
pub fn process_tourist_arrival_system(
    mut events: EventReader<TouristArrivalEvent>,
    tourists: Query<&Tourist>,
    mut resources: ResMut<ColonyResources>,
    mut state: ResMut<TourismState>,
) {
    for event in events.read() {
        if let Ok(tourist) = tourists.get(event.tourist) {
            let paid = arrival_payment(tourist.wealth);
            resources.add_credits(paid);
            state.credits_earned += paid;
            state.total_arrived += 1;
        }
    }
}

/// Tourists eat from the shared stockpile on a cadence. Under
/// `DenyVisitors`, or when the stockpile is empty, the meal is missed
/// and hunger craters — the feed-the-rich dilemma of spec §1.
pub fn tourist_consumption_system(
    time: Res<SimulationTime>,
    mut resources: ResMut<ColonyResources>,
    mut state: ResMut<TourismState>,
    mut tourists: Query<&mut Needs, With<Tourist>>,
) {
    if time.tick == 0 || time.tick % EAT_INTERVAL != 0 {
        return;
    }
    for mut needs in &mut tourists {
        let fed = state.policy == TourismPolicy::FeedVisitors && resources.food >= EAT_AMOUNT;
        if fed {
            resources.food -= EAT_AMOUNT;
            needs.hunger = (needs.hunger + EAT_HUNGER_GAIN).min(1.0);
        } else {
            needs.hunger = (needs.hunger - MISSED_MEAL_HUNGER_LOSS).max(0.0);
            state.denied_meals += 1;
        }
    }
}

/// Structural fix — the vacation leg. Tourists are on holiday: on a
/// [`SELF_CARE_INTERVAL`] cadence, present tourists recover rest,
/// leisure, and hygiene on the real [`Needs`] component. This bridges
/// the restoration gap left by tourists never joining utility AI
/// (colony pops restore these needs through jobs/actions; visitors
/// deliberately have none). Leisure recovery scales with the beauty
/// of the tile the tourist stands on — the same `BeautyGrid` seam
/// `apply_beauty_effects_system` uses for pops — so pretty colonies
/// keep guests happier. Hunger is NOT touched here; meals own it.
pub fn tourist_self_care_system(
    time: Res<SimulationTime>,
    grid: Option<Res<BeautyGrid>>,
    mut tourists: Query<(&mut Needs, Option<&GridPosition>), With<Tourist>>,
) {
    if time.tick == 0 || !time.tick.is_multiple_of(SELF_CARE_INTERVAL) {
        return;
    }
    for (mut needs, pos) in &mut tourists {
        needs.rest = (needs.rest + SELF_CARE_REST_GAIN).min(1.0);
        needs.hygiene = (needs.hygiene + SELF_CARE_HYGIENE_GAIN).min(1.0);
        let beauty_bonus = match (grid.as_deref(), pos) {
            (Some(grid), Some(pos)) => {
                let beauty = match (usize::try_from(pos.x), usize::try_from(pos.y)) {
                    (Ok(x), Ok(y)) => grid.get(x, y),
                    _ => 0.0,
                };
                if beauty > 0.0 {
                    (beauty * SELF_CARE_BEAUTY_BONUS_PER_POINT).min(SELF_CARE_BEAUTY_BONUS_CAP)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        needs.leisure = (needs.leisure + SELF_CARE_LEISURE_GAIN + beauty_bonus).min(1.0);
    }
}

/// Spec §5 anger leg + structural fixes: a tourist whose worst need
/// craters departs early, costs reputation (ending the boom under
/// 0.5), and forfeits the completion payment; a tourist who reaches
/// their departure tick leaves satisfied — the colony banks the
/// completion payment and reputation recovers by
/// [`REP_GAIN_COMPLETED_STAY`] (capped at 1.0).
///
/// Item 203: this is the only system that mutates reputation, so it
/// also edge-triggers the reputation Chronicle lines — one Major
/// line when a mutation crosses [`REPUTATION_BOOM_THRESHOLD`] in
/// either direction, none otherwise (see the module docs).
pub fn tourist_departure_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut state: ResMut<TourismState>,
    mut resources: Option<ResMut<ColonyResources>>,
    tourists: Query<(Entity, &Tourist, &Needs)>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    for (entity, tourist, needs) in &tourists {
        if needs.worst() < 0.2 {
            state.total_departed_early += 1;
            let reputation_before = state.reputation;
            state.reputation = (state.reputation - REP_PENALTY_EARLY_DEPARTURE).max(0.0);
            if state.reputation < REPUTATION_BOOM_THRESHOLD {
                state.boom_active = false;
            }
            if let Some(events) = chronicle.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: TOURISM_EARLY_DEPARTURE_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
            // Item 203: name the fall across the boom threshold,
            // once per crossing (re-arms only after a recovery
            // crossing, so repeated early departures stay silent).
            if reputation_before >= REPUTATION_BOOM_THRESHOLD
                && state.reputation < REPUTATION_BOOM_THRESHOLD
            {
                if let Some(events) = chronicle.as_deref_mut() {
                    events.send(AddChronicleEvent {
                        text: tourism_reputation_fallen_chronicle_text(state.reputation),
                        importance: EventImportance::Major,
                    });
                }
            }
            commands.entity(entity).despawn();
        } else if time.tick >= tourist.departure_tick {
            state.total_departed += 1;
            let reputation_before = state.reputation;
            state.reputation = (state.reputation + REP_GAIN_COMPLETED_STAY).min(1.0);
            let paid = completion_payment(tourist.wealth);
            if paid > 0.0 {
                if let Some(resources) = resources.as_deref_mut() {
                    resources.add_credits(paid);
                    state.credits_earned += paid;
                }
            }
            if let Some(events) = chronicle.as_deref_mut() {
                events.send(AddChronicleEvent {
                    text: TOURISM_COMPLETED_TEXT.to_string(),
                    importance: EventImportance::Major,
                });
            }
            // Item 203: name the recovery across the boom threshold,
            // once per crossing (the +0.10 steps below the threshold
            // stay silent until they add up to the crossing).
            if reputation_before < REPUTATION_BOOM_THRESHOLD
                && state.reputation >= REPUTATION_BOOM_THRESHOLD
            {
                if let Some(events) = chronicle.as_deref_mut() {
                    events.send(AddChronicleEvent {
                        text: tourism_reputation_recovered_chronicle_text(state.reputation),
                        importance: EventImportance::Major,
                    });
                }
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Census for the headless panel / STATS.
#[derive(Debug, Clone, Copy, Default)]
pub struct TourismCensus {
    /// Tourists currently in the colony.
    pub present: usize,
    /// Tourists queued to land.
    pub pending: u32,
    /// Current reputation.
    pub reputation: f32,
    /// Lifetime credits earned.
    pub credits_earned: f32,
    /// Lifetime early departures.
    pub departed_early: u32,
    /// Lifetime disease imports.
    pub disease_imports: u32,
}

/// Read the tourism census from a raw world (headless helper pattern).
pub fn tourism_census(world: &mut World) -> TourismCensus {
    let present = {
        let mut q = world.query::<&Tourist>();
        q.iter(world).count()
    };
    let (pending, reputation, credits_earned, departed_early, disease_imports) = world
        .get_resource::<TourismState>()
        .map_or((0, 1.0, 0.0, 0, 0), |s| {
            (
                s.pending_tourists,
                s.reputation,
                s.credits_earned,
                s.total_departed_early,
                s.disease_imports,
            )
        });
    TourismCensus {
        present,
        pending,
        reputation,
        credits_earned,
        departed_early,
        disease_imports,
    }
}

/// Compact STATS label: `present/pending/rep`.
pub fn tourism_stats_label(world: &mut World) -> String {
    let c = tourism_census(world);
    format!("{}/{}/{:.2}", c.present, c.pending, c.reputation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn beauty_grid_with_total(total: f32) -> BeautyGrid {
        let mut grid = BeautyGrid::new(10, 10);
        grid.set(0, 0, total);
        grid
    }

    /// Spec RED test 1, adapted: `ColonyStats{global_beauty}` mock →
    /// real `BeautyGrid` sum; `VisitorSpawning` mock → `TourismState`.
    #[test]
    fn test_high_beauty_triggers_tourist_spawn() {
        let mut world = World::new();
        world.insert_resource(beauty_grid_with_total(500.0));
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        // Pre-age the cadence so the first evaluation fires.
        world.resource_mut::<TourismState>().last_attraction_tick = 0;
        world.run_system_once(evaluate_tourist_attraction_system).unwrap();
        let state = world.resource::<TourismState>();
        assert!(state.pending_tourists > 0, "High beauty should queue tourists");
        assert!(state.boom_active);
    }

    #[test]
    fn test_low_beauty_queues_nothing() {
        let mut world = World::new();
        world.insert_resource(beauty_grid_with_total(50.0));
        world.insert_resource(SimulationTime {
            tick: 1000,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.run_system_once(evaluate_tourist_attraction_system).unwrap();
        assert_eq!(world.resource::<TourismState>().pending_tourists, 0);
    }

    /// Spec RED test 2, adapted: credits land in the real ledger.
    #[test]
    fn test_tourist_arrival_grants_credits() {
        let mut world = World::new();
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.insert_resource(ColonyResources {
            credits: 100.0,
            max_credits: 1_000_000.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        let tourist_ent = world
            .spawn(Tourist {
                wealth: 500.0,
                arrival_tick: 0,
                departure_tick: 800,
            })
            .id();
        world
            .resource_mut::<Events<TouristArrivalEvent>>()
            .send(TouristArrivalEvent { tourist: tourist_ent });
        world.run_system_once(process_tourist_arrival_system).unwrap();
        // Structural fix: only the arrival portion (100 of 500) banks
        // on landing; the rest is contingent on a completed stay.
        assert_eq!(world.resource::<ColonyResources>().credits, 200.0);
        assert_eq!(
            world.resource::<TourismState>().credits_earned,
            TOURIST_ARRIVAL_PAYMENT
        );
        assert_eq!(world.resource::<TourismState>().total_arrived, 1);
    }

    #[test]
    fn test_spawned_tourist_carries_real_visitor_tag() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().pending_tourists = 1;
        world.run_system_once(spawn_tourist_system).unwrap();
        let mut q = world.query::<(&Tourist, &Visitor, &Needs)>();
        assert_eq!(q.iter(&world).count(), 1, "tourist must be a real Visitor");
    }

    #[test]
    fn test_deny_policy_starves_tourist_and_costs_reputation() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 50.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().policy = TourismPolicy::DenyVisitors;
        let ent = world
            .spawn((
                Tourist { wealth: 500.0, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.4, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
            ))
            .id();
        world.run_system_once(tourist_consumption_system).unwrap();
        assert_eq!(world.resource::<ColonyResources>().food, 50.0, "denied meal draws nothing");
        world.run_system_once(tourist_departure_system).unwrap();
        // hunger 0.4 - 0.3 = 0.1 < 0.2 → early departure + penalty.
        assert!(world.get::<Tourist>(ent).is_none());
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed_early, 1);
        assert!((state.reputation - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_feed_policy_consumes_shared_food() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        world.spawn((
            Tourist { wealth: 500.0, arrival_tick: 0, departure_tick: 800 },
            Needs { hunger: 0.3, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
        ));
        world.run_system_once(tourist_consumption_system).unwrap();
        assert_eq!(world.resource::<ColonyResources>().food, 9.0);
    }

    #[test]
    fn test_disease_roll_fraction() {
        assert!(tourist_carries_disease(0.0));
        assert!(tourist_carries_disease(0.09));
        assert!(!tourist_carries_disease(0.10));
        assert!(!tourist_carries_disease(0.99));
    }

    // ---- Spec 408 structural fixes (2026-11-13): RED tests ----

    #[test]
    fn test_arrival_pays_only_arrival_portion() {
        let mut world = World::new();
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.insert_resource(ColonyResources {
            credits: 100.0,
            max_credits: 1_000_000.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        let tourist_ent = world
            .spawn(Tourist {
                wealth: TOURIST_WEALTH,
                arrival_tick: 0,
                departure_tick: 800,
            })
            .id();
        world
            .resource_mut::<Events<TouristArrivalEvent>>()
            .send(TouristArrivalEvent { tourist: tourist_ent });
        world.run_system_once(process_tourist_arrival_system).unwrap();
        // Only the arrival portion lands on landing; the completion
        // portion is withheld until a completed stay.
        assert_eq!(
            world.resource::<ColonyResources>().credits,
            100.0 + TOURIST_ARRIVAL_PAYMENT
        );
        assert_eq!(
            world.resource::<TourismState>().credits_earned,
            TOURIST_ARRIVAL_PAYMENT
        );
        assert_eq!(
            TOURIST_ARRIVAL_PAYMENT + TOURIST_COMPLETION_PAYMENT,
            TOURIST_WEALTH,
            "split portions must sum to the unchanged total wealth"
        );
    }

    #[test]
    fn test_completed_stay_pays_completion_and_restores_reputation() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 800,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().reputation = 0.40;
        world.spawn((
            Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
            Needs { hunger: 0.9, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
        ));
        world.run_system_once(tourist_departure_system).unwrap();
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed, 1);
        assert_eq!(state.total_departed_early, 0);
        assert!(
            (state.reputation - (0.40 + REP_GAIN_COMPLETED_STAY)).abs() < 1e-6,
            "completed stay must restore reputation, got {}",
            state.reputation
        );
        assert_eq!(
            world.resource::<ColonyResources>().credits,
            TOURIST_COMPLETION_PAYMENT
        );
        assert_eq!(state.credits_earned, TOURIST_COMPLETION_PAYMENT);
    }

    #[test]
    fn test_completed_stay_reputation_caps_at_one() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 800,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.init_resource::<TourismState>();
        world.resource_mut::<TourismState>().reputation = 0.97;
        world.spawn((
            Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
            Needs { hunger: 0.9, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
        ));
        world.run_system_once(tourist_departure_system).unwrap();
        assert_eq!(world.resource::<TourismState>().reputation, 1.0);
    }

    #[test]
    fn test_early_departure_forfeits_completion_payment() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.init_resource::<TourismState>();
        world.spawn((
            Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
            Needs { hunger: 0.1, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
        ));
        world.run_system_once(tourist_departure_system).unwrap();
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed_early, 1);
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);
        assert_eq!(state.credits_earned, 0.0);
        assert!((state.reputation - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_full_stay_total_payment_equals_full_wealth() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.init_resource::<Events<TouristArrivalEvent>>();
        world.insert_resource(ColonyResources {
            credits: 0.0,
            max_credits: 1_000_000.0,
            ..Default::default()
        });
        world.init_resource::<TourismState>();
        let ent = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.9, rest: 0.9, leisure: 0.9, hygiene: 0.9 },
            ))
            .id();
        world
            .resource_mut::<Events<TouristArrivalEvent>>()
            .send(TouristArrivalEvent { tourist: ent });
        world.run_system_once(process_tourist_arrival_system).unwrap();
        world.resource_mut::<SimulationTime>().tick = 800;
        world.run_system_once(tourist_departure_system).unwrap();
        assert_eq!(
            world.resource::<ColonyResources>().credits,
            TOURIST_WEALTH,
            "arrival + completion portions must total the full wealth"
        );
        assert_eq!(world.resource::<TourismState>().credits_earned, TOURIST_WEALTH);
    }

    #[test]
    fn test_self_care_restores_vacation_needs() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: SELF_CARE_INTERVAL,
            ..Default::default()
        });
        let ent = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.9, rest: 0.3, leisure: 0.3, hygiene: 0.3 },
            ))
            .id();
        world.run_system_once(tourist_self_care_system).unwrap();
        let needs = world.get::<Needs>(ent).unwrap();
        assert!(needs.rest > 0.3, "self-care must restore rest");
        assert!(needs.leisure > 0.3, "self-care must restore leisure");
        assert!(needs.hygiene > 0.3, "self-care must restore hygiene");
        assert_eq!(needs.hunger, 0.9, "self-care must not touch hunger (meals own it)");
    }

    #[test]
    fn test_self_care_off_cadence_is_noop() {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: SELF_CARE_INTERVAL - 3,
            ..Default::default()
        });
        let ent = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.9, rest: 0.3, leisure: 0.3, hygiene: 0.3 },
            ))
            .id();
        world.run_system_once(tourist_self_care_system).unwrap();
        let needs = world.get::<Needs>(ent).unwrap();
        assert_eq!(needs.rest, 0.3);
        assert_eq!(needs.leisure, 0.3);
        assert_eq!(needs.hygiene, 0.3);
    }

    #[test]
    fn test_self_care_leisure_scales_with_local_beauty() {
        let mut world = World::new();
        let mut grid = BeautyGrid::new(10, 10);
        grid.set(5, 5, 10.0);
        world.insert_resource(grid);
        world.insert_resource(SimulationTime {
            tick: SELF_CARE_INTERVAL,
            ..Default::default()
        });
        let scenic = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.9, rest: 0.9, leisure: 0.3, hygiene: 0.9 },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let plain = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs { hunger: 0.9, rest: 0.9, leisure: 0.3, hygiene: 0.9 },
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world.run_system_once(tourist_self_care_system).unwrap();
        let scenic_leisure = world.get::<Needs>(scenic).unwrap().leisure;
        let plain_leisure = world.get::<Needs>(plain).unwrap().leisure;
        assert!(
            scenic_leisure > plain_leisure,
            "local beauty must boost leisure recovery ({scenic_leisure} vs {plain_leisure})"
        );
    }

    #[test]
    fn test_self_care_sustains_full_stay_then_completes() {
        // Simulate a full 800-tick stay under the real decay rates
        // (needs.rs: rest 0.001/tick, leisure 0.0015/tick; hygiene.rs:
        // 0.001/tick base) with meals every EAT_INTERVAL restoring
        // hunger (the FeedVisitors leg) and self-care on cadence.
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.init_resource::<TourismState>();
        let ent = world
            .spawn((
                Tourist { wealth: TOURIST_WEALTH, arrival_tick: 0, departure_tick: 800 },
                Needs::default(),
            ))
            .id();
        for tick in 1..=TOURIST_STAY_TICKS {
            world.resource_mut::<SimulationTime>().tick = tick;
            {
                let mut needs = world.get_mut::<Needs>(ent).unwrap();
                needs.hunger = (needs.hunger - 0.001).max(0.0);
                needs.rest = (needs.rest - 0.001).max(0.0);
                needs.leisure = (needs.leisure - 0.0015).max(0.0);
                needs.hygiene = (needs.hygiene - 0.001).max(0.0);
                if tick % EAT_INTERVAL == 0 {
                    needs.hunger = (needs.hunger + EAT_HUNGER_GAIN).min(1.0);
                }
            }
            world.run_system_once(tourist_self_care_system).unwrap();
        }
        let needs = world.get::<Needs>(ent).unwrap();
        assert!(
            needs.worst() >= 0.2,
            "after a full stay worst need must stay completable, got {}",
            needs.worst()
        );
        world.run_system_once(tourist_departure_system).unwrap();
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed, 1, "stay must complete, not depart early");
        assert_eq!(state.total_departed_early, 0);
    }

    /// IP guard: every new player-facing string is original/generic.
    #[test]
    fn test_ip_guard_tourism_strings() {
        let banned = [
            "star wars", "star trek", "firefly", "serenity", "babylon 5",
            "mass effect", "citadel", "coruscant", "tatooine", "enterprise",
            "fhloston", "fifth element", "total recall", "mars",
        ];
        let strings = [
            TOURISM_BOOM_TEXT,
            TOURISM_EARLY_DEPARTURE_TEXT,
            TOURISM_DISEASE_TEXT,
            TOURISM_COMPLETED_TEXT,
        ];
        for s in strings {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "IP-guard: '{b}' in '{s}'");
            }
        }
    }

    // ---- Item 203: reputation Chronicle telegraphing (RED) ----
    // Spec 408 reputation was Chronicle-silent: completed stays were
    // Major-chronicled, but no line named reputation, its recovery
    // (+0.10 per completed stay), or the 0.5 boom threshold. Only
    // threshold *crossings* are chronicled (edge-triggered, so the
    // line re-arms only after reputation crosses back) — no spam.

    fn chronicle_texts(world: &mut World) -> Vec<(String, EventImportance)> {
        let events = world.resource::<Events<AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        reader
            .read(events)
            .map(|e| (e.text.clone(), e.importance))
            .collect()
    }

    fn world_with_chronicle(tick: u64, reputation: f32) -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.init_resource::<TourismState>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.resource_mut::<TourismState>().reputation = reputation;
        world
    }

    fn spawn_tourist(world: &mut World, worst_need: f32, departure_tick: u64) {
        world.spawn((
            Tourist {
                wealth: TOURIST_WEALTH,
                arrival_tick: 0,
                departure_tick,
            },
            Needs {
                hunger: worst_need,
                rest: 0.9,
                leisure: 0.9,
                hygiene: 0.9,
            },
        ));
    }

    #[test]
    fn test_reputation_fallen_text_names_reputation_value_and_threshold() {
        assert_eq!(REPUTATION_BOOM_THRESHOLD, 0.5, "boom gate must stay 0.5");
        let text = tourism_reputation_fallen_chronicle_text(0.25);
        let lower = text.to_lowercase();
        assert!(lower.contains("reputation"), "line must name reputation: {text}");
        assert!(text.contains("0.25"), "line must name the new value: {text}");
        assert!(text.contains("0.50"), "line must name the boom threshold: {text}");
        assert!(lower.contains("below"), "line must name the crossing: {text}");
    }

    #[test]
    fn test_reputation_recovered_text_names_reputation_gain_and_threshold() {
        let text = tourism_reputation_recovered_chronicle_text(0.50);
        let lower = text.to_lowercase();
        assert!(lower.contains("reputation"), "line must name reputation: {text}");
        assert!(lower.contains("recover"), "line must name the recovery: {text}");
        assert!(text.contains("0.50"), "line must name the boom threshold: {text}");
        assert!(
            text.contains(&format!("{:.2}", REP_GAIN_COMPLETED_STAY)),
            "line must name the per-stay recovery (+{:.2}): {text}",
            REP_GAIN_COMPLETED_STAY
        );
    }

    #[test]
    fn test_early_departure_crossing_below_threshold_chronicles_fall_once() {
        let mut world = world_with_chronicle(100, 0.60);
        world.resource_mut::<TourismState>().boom_active = true;
        // Two early departures in one run: 0.60 -> 0.35 (crossing)
        // -> 0.10 (already below, must NOT re-chronicle the fall).
        spawn_tourist(&mut world, 0.1, 800);
        spawn_tourist(&mut world, 0.1, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed_early, 2);
        assert!((state.reputation - 0.10).abs() < 1e-6);
        assert!(!state.boom_active);
        let texts = chronicle_texts(&mut world);
        assert_eq!(
            texts.len(),
            3,
            "2 early-departure lines + exactly 1 reputation-fall line, got: {texts:?}"
        );
        let fallen: Vec<_> = texts
            .iter()
            .filter(|(t, _)| t.to_lowercase().contains("reputation"))
            .collect();
        assert_eq!(fallen.len(), 1, "fall must be chronicled once: {texts:?}");
        assert_eq!(fallen[0].1, EventImportance::Major);
        assert!(
            fallen[0].0.contains("0.35"),
            "fall line must name the value at the crossing: {}",
            fallen[0].0
        );
    }

    #[test]
    fn test_early_departure_without_crossing_no_reputation_line() {
        let mut world = world_with_chronicle(100, 1.0);
        spawn_tourist(&mut world, 0.1, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        assert!((world.resource::<TourismState>().reputation - 0.75).abs() < f32::EPSILON);
        let texts = chronicle_texts(&mut world);
        assert_eq!(texts.len(), 1, "only the early-departure line: {texts:?}");
        assert!(
            !texts[0].0.to_lowercase().contains("reputation"),
            "no reputation line without a crossing: {}",
            texts[0].0
        );
    }

    #[test]
    fn test_completed_stay_crossing_threshold_chronicles_recovery_once() {
        let mut world = world_with_chronicle(800, 0.40);
        // Two completions in one run: 0.40 -> 0.50 (crossing)
        // -> 0.60 (already above, must NOT re-chronicle the recovery).
        spawn_tourist(&mut world, 0.9, 800);
        spawn_tourist(&mut world, 0.9, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        let state = world.resource::<TourismState>();
        assert_eq!(state.total_departed, 2);
        assert!((state.reputation - 0.60).abs() < 1e-6);
        let texts = chronicle_texts(&mut world);
        assert_eq!(
            texts.len(),
            3,
            "2 completed-stay lines + exactly 1 recovery line, got: {texts:?}"
        );
        let recovered: Vec<_> = texts
            .iter()
            .filter(|(t, _)| t.to_lowercase().contains("reputation"))
            .collect();
        assert_eq!(recovered.len(), 1, "recovery chronicled once: {texts:?}");
        assert_eq!(recovered[0].1, EventImportance::Major);
        assert!(
            recovered[0].0.contains("0.50"),
            "recovery line must name the value at the crossing: {}",
            recovered[0].0
        );
    }

    #[test]
    fn test_completed_stay_below_threshold_no_recovery_line_yet() {
        let mut world = world_with_chronicle(800, 0.0);
        spawn_tourist(&mut world, 0.9, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        assert!(
            (world.resource::<TourismState>().reputation - REP_GAIN_COMPLETED_STAY).abs() < 1e-6
        );
        let texts = chronicle_texts(&mut world);
        assert_eq!(texts.len(), 1, "only the completed-stay line: {texts:?}");
        assert!(
            !texts[0].0.to_lowercase().contains("reputation"),
            "recovery is named at the threshold crossing, not per stay: {}",
            texts[0].0
        );
    }

    #[test]
    fn test_reputation_chronicle_rearms_after_opposite_crossing() {
        // Recovery crossing, then a fall crossing: each direction
        // fires exactly once per crossing (hysteresis, not one-shot
        // forever and not per-change spam).
        let mut world = world_with_chronicle(800, 0.40);
        spawn_tourist(&mut world, 0.9, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        assert!((world.resource::<TourismState>().reputation - 0.50).abs() < 1e-6);
        world.resource_mut::<SimulationTime>().tick = 100;
        spawn_tourist(&mut world, 0.1, 800);
        world.run_system_once(tourist_departure_system).unwrap();
        assert!((world.resource::<TourismState>().reputation - 0.25).abs() < 1e-6);
        let texts = chronicle_texts(&mut world);
        let rep_lines: Vec<_> = texts
            .iter()
            .filter(|(t, _)| t.to_lowercase().contains("reputation"))
            .collect();
        assert_eq!(
            rep_lines.len(),
            2,
            "one recovery line + one fall line across the cycle: {texts:?}"
        );
    }

    /// IP guard: the new reputation chronicle texts are original/generic.
    #[test]
    fn test_ip_guard_tourism_reputation_chronicle_texts() {
        let banned = [
            "star wars", "star trek", "firefly", "serenity", "babylon 5",
            "mass effect", "citadel", "coruscant", "tatooine", "enterprise",
            "fhloston", "fifth element", "total recall", "mars",
        ];
        let mut strings = tourism_reputation_public_strings();
        strings.push(tourism_reputation_fallen_chronicle_text(0.25));
        strings.push(tourism_reputation_recovered_chronicle_text(0.50));
        for s in &strings {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "IP-guard: '{b}' in '{s}'");
            }
        }
    }
}
