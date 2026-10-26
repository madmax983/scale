//! The Director (Spec 1388): a threat-budget pacing authority.
//!
//! Scale's hostile deck used to be flat rolls from tick 0. The Director
//! replaces that with a single budget: it accrues over time (scaled by era
//! temperament, colony size, wealth, season, and colony age), and every
//! hostile event kind carries a static threat level that IS its price.
//! The Director spends budget at exactly `threat_level * cost_mult` to fire
//! a kind through its existing seam, then rests — budget keeps filling
//! during rest (deferral, never forgiveness).
//!
//! LAW (decided, Mark 2026-10-26): accrual is distress-insensitive. No
//! scaler reads morale, health, deaths, or damage; deaths never refund
//! budget. There is no rubber-banding in this module.
//!
//! Structure follows the Spec 356 crossfire pattern: knob Resource
//! ([`DirectorConfig`]) + pure functions + stats ledger ([`DirectorState`])
//! + one exclusive-`World` director system.

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::pop::Pop;
use crate::layer1::environment::orbital_crossfire::CrossfireStats;
use crate::layer1::nature::seasons::{Season, SeasonState};
use crate::layer1::void_weed::PirateRaidEvent;
use crate::shared::random::GlobalRng;
use crate::shared::scenario::StartScenarioId;
use crate::shared::time::SimulationTime;

/// Era-independent base accrual, budget per tick before scalers (§2).
pub const BASE_ACCRUAL: f32 = 0.5;
/// Ticks between Director spend checks (§2).
pub const CHECK_INTERVAL: u64 = 25;
/// Wealth (food + metal) at which the wealth scaler reaches its cap.
pub const WEALTH_THRESHOLD: f32 = 100.0;

// ---------------------------------------------------------------------------
// Registry (§4.1): every exogenous event kind declares its threat level here.
// The threat level is the danger rating AND the budget price. Static per
// kind (§1.4). A kind without a rating cannot be scheduled.
// ---------------------------------------------------------------------------

/// An event kind known to the Director's registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectorEventKind {
    /// Beacon-deck pirate raid, resolved by the existing raid bridge.
    PirateRaid,
    /// Spore outbreak seeding (Spec 326 hitchhiker, token-gated).
    SporeOutbreak,
    /// Orbital crossfire strike (Spec 356) — rated, never spent here.
    CrossfireStrike,
    /// Orphaned swarm arrival — rated, spend deferred.
    OrphanedSwarm,
    /// Symbiont sabotage crisis — rated, endogenous.
    SymbiontSabotage,
    /// Building-destroying fire — rated, endogenous.
    BuildingFire,
    /// Refugee arrival — rated Minor, benign pressure.
    RefugeeArrival,
    /// Trade ship arrival — benign.
    TradeShip,
    /// Migrant arrival — benign.
    MigrantArrival,
}

impl DirectorEventKind {
    /// THE threat level: danger rating and budget price in one number.
    /// Static per kind (§1.4). This registry is the single home for
    /// threat numbers in Director code.
    #[must_use]
    pub const fn threat_level(self) -> u32 {
        match self {
            Self::PirateRaid => 60,
            Self::SporeOutbreak => 40,
            Self::CrossfireStrike => 45,
            Self::OrphanedSwarm => 35,
            Self::SymbiontSabotage => 25,
            Self::BuildingFire => 30,
            Self::RefugeeArrival => 10,
            Self::TradeShip => 0,
            Self::MigrantArrival => 0,
        }
    }

    /// Player-facing threat tier for this kind's rating.
    #[must_use]
    pub const fn tier(self) -> ThreatTier {
        ThreatTier::for_level(self.threat_level())
    }

    /// Stable machine label (STATS / panels / ledger).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PirateRaid => "pirate_raid",
            Self::SporeOutbreak => "spore_outbreak",
            Self::CrossfireStrike => "crossfire_strike",
            Self::OrphanedSwarm => "orphaned_swarm",
            Self::SymbiontSabotage => "symbiont_sabotage",
            Self::BuildingFire => "building_fire",
            Self::RefugeeArrival => "refugee_arrival",
            Self::TradeShip => "trade_ship",
            Self::MigrantArrival => "migrant_arrival",
        }
    }

    /// Kinds the Director may spend budget on in v1 (§1.3 "Priced").
    #[must_use]
    pub const fn priced_kinds() -> [Self; 2] {
        [Self::PirateRaid, Self::SporeOutbreak]
    }
}

/// Threat tiers for player-facing language (§1.3). The Severe band
/// (75+) stays EMPTY in v1 by decision — headroom, deliberately unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreatTier {
    /// 0 — benign.
    Benign,
    /// 1–24.
    Minor,
    /// 25–49.
    Standard,
    /// 50–74.
    Major,
    /// 75+ — no live kind is rated Severe in v1.
    Severe,
}

impl ThreatTier {
    /// Tier for a static threat level.
    #[must_use]
    pub const fn for_level(level: u32) -> Self {
        match level {
            0 => Self::Benign,
            1..=24 => Self::Minor,
            25..=49 => Self::Standard,
            50..=74 => Self::Major,
            _ => Self::Severe,
        }
    }

    /// Player-facing tier word (omen vocabulary == pricing vocabulary).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Benign => "Benign",
            Self::Minor => "Minor",
            Self::Standard => "Standard",
            Self::Major => "Major",
            Self::Severe => "Severe",
        }
    }
}

/// Tension bands derived from the budget itself (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TensionBand {
    /// Budget < 40% of the cheapest priced class.
    #[default]
    Calm,
    /// 40–75% of the cheapest priced class.
    Rising,
    /// At/above the era foreshadow threshold.
    High,
    /// A class is affordable and all gates are open.
    Critical,
}

impl TensionBand {
    /// Ordering rank for upward-crossing detection.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Calm => 0,
            Self::Rising => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }

    /// Stable machine label (STATS `director=` field).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Rising => "rising",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

// ---------------------------------------------------------------------------
// Temperaments (§2): era differences are DATA in DirectorConfig, never
// per-era code paths. Every knob the balance loop turns lives here.
// ---------------------------------------------------------------------------

/// All Director tuning knobs for one run, resolved from the start era.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct DirectorConfig {
    /// Master switch (bot/test lever).
    pub enabled: bool,
    /// No spends before this many ticks after scenario start.
    pub grace_ticks: u64,
    /// Base budget accrual per tick before era/scaler multipliers.
    pub base_accrual: f32,
    /// Era multiplier on base accrual.
    pub accrual_mult: f32,
    /// Ticks between spend checks.
    pub check_interval: u64,
    /// Budget cap; accrual past the cap is lost (use-it-or-lose-it).
    pub budget_max: f32,
    /// Era multiplier on registry prices (declared, STATS-visible).
    pub cost_mult: f32,
    /// Rest window after a hostile spend (no spends, accrual continues).
    pub rest_ticks: u64,
    /// Accrual growth per elapsed year (0.15 = +15%/yr).
    pub year_escalation: f32,
    /// Season accrual weights [spring, summer, autumn, winter].
    pub season_weights: [f32; 4],
    /// Class weight for pirate raids in the weighted pick.
    pub raid_weight: u32,
    /// Class weight for spore outbreaks in the weighted pick.
    pub outbreak_weight: u32,
    /// Budget fraction of the cheapest priced class at which pressure
    /// becomes High / foreshadowed (0.75 = 75%).
    pub foreshadow_threshold: f32,
    /// Layer 2 Ready hears omens at every band crossing (§2 table).
    pub omens_at_every_band: bool,
}

impl DirectorConfig {
    /// The shipped temperament table (§2), resolved from the start era.
    /// Mirrors `start_scenario_definition`: a const data fn, no branches
    /// anywhere else in Director code keyed on era.
    #[must_use]
    pub const fn temperament_for(id: StartScenarioId) -> Self {
        let base = Self {
            enabled: true,
            grace_ticks: 100,
            base_accrual: BASE_ACCRUAL,
            accrual_mult: 1.0,
            check_interval: CHECK_INTERVAL,
            budget_max: 120.0,
            cost_mult: 1.0,
            rest_ticks: 150,
            year_escalation: 0.15,
            season_weights: [1.0, 1.0, 1.0, 1.0],
            raid_weight: 3,
            outbreak_weight: 2,
            foreshadow_threshold: 0.75,
            omens_at_every_band: false,
        };
        match id {
            StartScenarioId::Classic => base,
            StartScenarioId::GroundSurvival => Self {
                grace_ticks: 150,
                accrual_mult: 0.6,
                budget_max: 100.0,
                rest_ticks: 200,
                year_escalation: 0.10,
                // The calendar is this era's villain: winter x1.5.
                season_weights: [1.0, 1.0, 1.0, 1.5],
                raid_weight: 1,
                outbreak_weight: 1,
                ..base
            },
            StartScenarioId::SocialDrama => Self {
                grace_ticks: 125,
                accrual_mult: 0.7,
                budget_max: 110.0,
                rest_ticks: 250,
                year_escalation: 0.10,
                raid_weight: 1,
                outbreak_weight: 1,
                foreshadow_threshold: 0.60,
                ..base
            },
            StartScenarioId::Layer2Ready => Self {
                grace_ticks: 400,
                accrual_mult: 0.7,
                budget_max: 90.0,
                cost_mult: 1.1,
                rest_ticks: 400,
                year_escalation: 0.05,
                raid_weight: 1,
                outbreak_weight: 1,
                foreshadow_threshold: 0.60,
                omens_at_every_band: true,
                ..base
            },
        }
    }

    /// Accrual weight for a season under this temperament.
    #[must_use]
    pub fn season_weight(&self, season: Season) -> f32 {
        let idx = match season {
            Season::Spring => 0,
            Season::Summer => 1,
            Season::Autumn => 2,
            Season::Winter => 3,
        };
        self.season_weights[idx]
    }

    /// Class weight for a priced kind (0 for unpriced kinds).
    #[must_use]
    pub const fn class_weight(&self, kind: DirectorEventKind) -> u32 {
        match kind {
            DirectorEventKind::PirateRaid => self.raid_weight,
            DirectorEventKind::SporeOutbreak => self.outbreak_weight,
            _ => 0,
        }
    }
}

impl Default for DirectorConfig {
    fn default() -> Self {
        Self::temperament_for(StartScenarioId::Classic)
    }
}

// ---------------------------------------------------------------------------
// Ledger / state
// ---------------------------------------------------------------------------

/// One ledger entry: a spend (organic or forced) or a rated recency
/// observation (crossfire, price 0 — rated, not spent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectorLedgerEntry {
    /// What fired / was observed.
    pub kind: DirectorEventKind,
    /// Tick it fired / was observed.
    pub tick: u64,
    /// Budget price paid (0 for rated-not-spent observations).
    pub price: f32,
    /// True when fired by a bot lever, bypassing gates (§4.8).
    pub forced: bool,
}

/// The Director's live state + ledger. STATS reconstructs from this alone.
#[derive(Resource, Debug, Clone)]
pub struct DirectorState {
    /// Current threat budget, clamped [0, budget_max].
    pub budget: f32,
    /// No spends before this tick (rest window).
    pub rest_until: u64,
    /// Tick of the most recent spend.
    pub last_spend_tick: u64,
    /// Kind of the most recent spend, if any.
    pub last_spend_kind: Option<DirectorEventKind>,
    /// Whether the most recent spend was forced via a bot lever.
    pub last_spend_forced: bool,
    /// Total spends (organic + forced).
    pub spends_total: u32,
    /// Total forced spends (subset of `spends_total`).
    pub forced_spends_total: u32,
    /// True while the Director is holding out for a Major (raid) spend.
    pub saving_for_major: bool,
    /// Outbreak token (§4.6): set by a Director outbreak spend, consumed
    /// by the spore hitchhiker on its first successful seeding.
    pub outbreak_token: bool,
    /// Current tension band (§4.5).
    pub tension_band: TensionBand,
    /// Resolved temperament copy (mirrors the DirectorConfig resource).
    pub temperament: DirectorConfig,
    /// Accrual rate applied on the most recent tick (STATS / panel).
    pub last_accrual_rate: f32,
    /// Bitmask of bands already omened in the current rest cycle.
    pub omened_mask: u8,
    /// Last crossfire strike tick already observed (§4.7).
    pub last_crossfire_tick_seen: u64,
    /// Tick the scenario started (grace is measured from here).
    pub scenario_start_tick: u64,
    /// Bounded recency ledger (spends + rated observations).
    pub recent: Vec<DirectorLedgerEntry>,
}

impl DirectorState {
    /// Fresh state for a resolved temperament.
    #[must_use]
    pub fn new(temperament: DirectorConfig) -> Self {
        Self {
            budget: 0.0,
            rest_until: 0,
            last_spend_tick: 0,
            last_spend_kind: None,
            last_spend_forced: false,
            spends_total: 0,
            forced_spends_total: 0,
            saving_for_major: false,
            outbreak_token: false,
            tension_band: TensionBand::Calm,
            temperament,
            last_accrual_rate: 0.0,
            omened_mask: 0,
            last_crossfire_tick_seen: 0,
            scenario_start_tick: 0,
            recent: Vec::new(),
        }
    }

    /// Append to the recency ledger (bounded; oldest drops first).
    pub fn record(&mut self, kind: DirectorEventKind, tick: u64, price: f32, forced: bool) {
        self.recent.push(DirectorLedgerEntry {
            kind,
            tick,
            price,
            forced,
        });
        if self.recent.len() > 32 {
            self.recent.remove(0);
        }
    }
}

impl Default for DirectorState {
    fn default() -> Self {
        Self::new(DirectorConfig::default())
    }
}

// ---------------------------------------------------------------------------
// Pure functions (§5): accrual and spend decisions over (config, state,
// aggregates). The system below is a thin exclusive-World shell.
// ---------------------------------------------------------------------------

/// Colony aggregates the Director is allowed to read. By law (§4.2) this
/// struct has no morale / health / deaths / damage field — distress can
/// never enter accrual because it cannot be expressed here.
#[derive(Debug, Clone, Copy)]
pub struct DirectorAggregates {
    /// Living pop count.
    pub pop_count: usize,
    /// Stockpiled food + metal.
    pub wealth: f32,
    /// Current season.
    pub season: Season,
    /// Current tick (drives year escalation).
    pub tick: u64,
}

/// Pop scaler: 1.0 + 0.02 per pop past 5, capped x1.5 (§4.2).
#[must_use]
pub fn pop_scaler(pop_count: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let extra = pop_count.saturating_sub(5) as f32;
    (1.0 + 0.02 * extra).min(1.5)
}

/// Wealth scaler: ramps to its x1.25 cap at [`WEALTH_THRESHOLD`] (§4.2).
#[must_use]
pub fn wealth_scaler(wealth: f32) -> f32 {
    1.0 + (wealth.max(0.0) / WEALTH_THRESHOLD).clamp(0.0, 1.0) * 0.25
}

/// Budget accrued per tick under this config and these aggregates (§4.2).
#[must_use]
pub fn accrual_rate(config: &DirectorConfig, aggregates: &DirectorAggregates) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let years = (aggregates.tick / TICKS_PER_YEAR) as f32;
    config.base_accrual
        * config.accrual_mult
        * pop_scaler(aggregates.pop_count)
        * wealth_scaler(aggregates.wealth)
        * config.season_weight(aggregates.season)
        * (1.0 + config.year_escalation * years)
}

/// Effective price of a kind: static threat level x era cost_mult (§4.3).
#[must_use]
pub fn effective_price(config: &DirectorConfig, kind: DirectorEventKind) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let level = kind.threat_level() as f32;
    level * config.cost_mult
}

/// Cheapest priced class's effective price (band/affordability anchor).
#[must_use]
pub fn cheapest_priced_price(config: &DirectorConfig) -> f32 {
    DirectorEventKind::priced_kinds()
        .iter()
        .map(|k| effective_price(config, *k))
        .fold(f32::INFINITY, f32::min)
}

/// Tension band for a budget at a tick (§4.5). Critical requires a class
/// to be affordable AND both gates (grace, rest) open.
#[must_use]
pub fn tension_band_for(
    config: &DirectorConfig,
    budget: f32,
    tick: u64,
    rest_until: u64,
    scenario_start_tick: u64,
) -> TensionBand {
    let cheapest = cheapest_priced_price(config);
    if cheapest <= 0.0 {
        return TensionBand::Calm;
    }
    let gates_open =
        tick >= scenario_start_tick.saturating_add(config.grace_ticks) && tick >= rest_until;
    if budget >= cheapest && gates_open {
        TensionBand::Critical
    } else if budget >= config.foreshadow_threshold * cheapest {
        TensionBand::High
    } else if budget >= 0.40 * cheapest {
        TensionBand::Rising
    } else {
        TensionBand::Calm
    }
}

/// Save-for-a-major (§4.3 rule 4): budget has reached 80% of the raid
/// price but cannot yet afford the raid.
#[must_use]
pub fn is_saving_for_major(config: &DirectorConfig, budget: f32) -> bool {
    let raid_price = effective_price(config, DirectorEventKind::PirateRaid);
    budget >= 0.8 * raid_price && budget < raid_price
}

/// The spend decision for one check (§4.3, in order). `roll` is a [0,1)
/// draw from the run's seeded RNG stream. Pure: no world access.
#[must_use]
pub fn spend_decision(
    config: &DirectorConfig,
    state: &DirectorState,
    tick: u64,
    roll: f32,
) -> Option<DirectorEventKind> {
    if !config.enabled {
        return None;
    }
    // 1. Grace gate — absolute.
    if tick < state.scenario_start_tick.saturating_add(config.grace_ticks) {
        return None;
    }
    // 2. Rest gate.
    if tick < state.rest_until {
        return None;
    }
    // 3. Affordability.
    let affordable: Vec<DirectorEventKind> = DirectorEventKind::priced_kinds()
        .into_iter()
        .filter(|k| effective_price(config, *k) <= state.budget)
        .collect();
    if affordable.is_empty() {
        return None;
    }
    // 4. Save-for-a-major — except at cap, where the cap rule wins and
    // the cheapest affordable class is spent rather than wasting accrual.
    let at_cap = state.budget >= config.budget_max;
    if is_saving_for_major(config, state.budget) && !at_cap {
        return None;
    }
    if is_saving_for_major(config, state.budget) && at_cap {
        return affordable
            .iter()
            .min_by(|a, b| effective_price(config, **a).total_cmp(&effective_price(config, **b)))
            .copied();
    }
    // 5. Weighted class pick among affordable classes by era weights.
    let total_weight: u32 = affordable.iter().map(|k| config.class_weight(*k)).sum();
    if total_weight == 0 {
        return affordable.first().copied();
    }
    #[allow(clippy::cast_precision_loss)]
    let mut needle = roll.clamp(0.0, 0.999_999) * total_weight as f32;
    for kind in &affordable {
        #[allow(clippy::cast_precision_loss)]
        let w = config.class_weight(*kind) as f32;
        if needle < w {
            return Some(*kind);
        }
        needle -= w;
    }
    affordable.last().copied()
}

/// Earliest tick a spend becomes possible (panel estimate, §4.8):
/// max(grace end, rest end, projected affordability at current accrual),
/// aligned up to the next check tick. None when accrual is zero and no
/// class is affordable.
#[must_use]
pub fn earliest_spend_tick(
    config: &DirectorConfig,
    state: &DirectorState,
    tick: u64,
) -> Option<u64> {
    let gates = state
        .scenario_start_tick
        .saturating_add(config.grace_ticks)
        .max(state.rest_until);
    let cheapest = cheapest_priced_price(config);
    let affordable_tick = if state.budget >= cheapest {
        tick
    } else if state.last_accrual_rate > 0.0 {
        let needed = cheapest - state.budget;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ticks = (needed / state.last_accrual_rate).ceil() as u64;
        tick.saturating_add(ticks.max(1))
    } else {
        return None;
    };
    let candidate = gates.max(affordable_tick).max(tick);
    if config.check_interval <= 1 {
        return Some(candidate);
    }
    let rem = candidate % config.check_interval;
    Some(if rem == 0 {
        candidate
    } else {
        candidate + (config.check_interval - rem)
    })
}

/// Omen text for an upward band crossing (§4.5). Threat-tier vocabulary
/// only; the class is named only when affordability already determines
/// it (exactly one affordable class). No exact countdowns, ever.
fn omen_text(config: &DirectorConfig, state: &DirectorState) -> (String, EventImportance) {
    let affordable: Vec<DirectorEventKind> = DirectorEventKind::priced_kinds()
        .into_iter()
        .filter(|k| effective_price(config, *k) <= state.budget)
        .collect();
    let (text, importance) = match affordable.as_slice() {
        [DirectorEventKind::SporeOutbreak] => (
            "Traders speak of contaminated holds — a Standard threat gathers on the next hulls."
                .to_string(),
            EventImportance::Standard,
        ),
        [DirectorEventKind::PirateRaid] => (
            "Scouts report raider sign gathering beyond the beacon's edge — a Major threat gathers."
                .to_string(),
            EventImportance::Standard,
        ),
        _ => {
            // Tier only: the cheapest priced class sets the vocabulary.
            let tier = DirectorEventKind::SporeOutbreak.tier().name();
            (
                format!(
                    "Whispers on the beacon bands speak of trouble gathering — a {tier} threat gathers in the quiet."
                ),
                EventImportance::Minor,
            )
        }
    };
    (text, importance)
}

/// Fire a priced kind through its existing seam. Raid sends the event the
/// bridge already resolves; outbreak sets the token the hitchhiker
/// consumes (§4.6). Shared by the organic system and bot levers.
fn fire_kind(world: &mut World, kind: DirectorEventKind) {
    match kind {
        DirectorEventKind::PirateRaid => {
            if let Some(mut events) =
                world.get_resource_mut::<bevy_ecs::event::Events<PirateRaidEvent>>()
            {
                events.send(PirateRaidEvent);
            }
        }
        DirectorEventKind::SporeOutbreak => {
            if let Some(mut state) = world.get_resource_mut::<DirectorState>() {
                state.outbreak_token = true;
            }
        }
        _ => {}
    }
}

/// Bot lever (§4.8): fire a priced kind NOW, bypassing grace/rest/
/// affordability. Ledger-recorded as forced; does not deduct budget or
/// impose rest — a probe, not an organic pacing event. Returns false for
/// unpriced kinds.
pub fn force_director_spend(world: &mut World, kind: DirectorEventKind) -> bool {
    if !DirectorEventKind::priced_kinds().contains(&kind) {
        return false;
    }
    let config = world
        .get_resource::<DirectorConfig>()
        .copied()
        .unwrap_or_default();
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let price = effective_price(&config, kind);
    if world.get_resource::<DirectorState>().is_none() {
        world.insert_resource(DirectorState::new(config));
    }
    {
        let mut state = world.resource_mut::<DirectorState>();
        state.spends_total += 1;
        state.forced_spends_total += 1;
        state.last_spend_tick = tick;
        state.last_spend_kind = Some(kind);
        state.last_spend_forced = true;
        state.record(kind, tick, price, true);
    }
    fire_kind(world, kind);
    true
}

/// STATS `director=` field value (§4.8):
/// `{band}:{budget}/{cap} spends={n} last={kind}@{tick} rest_until={tick}
/// accrual={rate} cost_mult={m}` — the ledger reconstructs from this.
#[must_use]
pub fn director_stats_label(world: &World) -> String {
    let config = world.get_resource::<DirectorConfig>();
    let cost_mult = config.map_or(1.0, |c| c.cost_mult);
    let Some(state) = world.get_resource::<DirectorState>() else {
        let cap = config.map_or(0.0, |c| c.budget_max);
        return format!(
            "calm:0/{cap:.0} spends=0 last=none@0 rest_until=0 accrual=0.00 cost_mult={cost_mult:.2}"
        );
    };
    let last = state
        .last_spend_kind
        .map_or("none", DirectorEventKind::label);
    format!(
        "{}:{:.0}/{:.0} spends={} last={}@{} rest_until={} accrual={:.2} cost_mult={:.2}",
        state.tension_band.label(),
        state.budget,
        state.temperament.budget_max,
        state.spends_total,
        last,
        state.last_spend_tick,
        state.rest_until,
        state.last_accrual_rate,
        state.temperament.cost_mult,
    )
}

/// The Director system: per-tick accrual (cheap arithmetic), omen/band
/// tracking, crossfire half-rest observation, and the spend check on
/// `check_interval`. Exclusive `World`, mirroring
/// `crossfire_director_system`.
pub fn director_system(world: &mut World) {
    let Some(config) = world.get_resource::<DirectorConfig>().cloned() else {
        return;
    };
    if !config.enabled {
        return;
    }
    if world.get_resource::<DirectorState>().is_none() {
        world.insert_resource(DirectorState::new(config));
    }
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let crossfire_last_tick = world.get_resource::<CrossfireStats>().map(|s| s.last_tick);

    // Aggregates (§4.2): pop count + stockpiled wealth + season only.
    // Morale/health/deaths/damage are never read here — by law.
    let pop_count = {
        let mut query = world.query::<&Pop>();
        query.iter(world).count()
    };
    let wealth = world
        .get_resource::<ColonyResources>()
        .map_or(0.0, |r| r.food + r.metal);
    let season = world
        .get_resource::<SeasonState>()
        .map_or(Season::Spring, |s| s.current_season);
    let aggregates = DirectorAggregates {
        pop_count,
        wealth,
        season,
        tick,
    };
    let rate = accrual_rate(&config, &aggregates);

    // Draw from the run's seeded RNG stream only on check ticks (§7).
    let is_check_tick = config.check_interval > 0 && tick.is_multiple_of(config.check_interval);
    let roll: f32 = if is_check_tick {
        if let Some(mut rng) = world.get_resource_mut::<GlobalRng>() {
            rng.0.gen::<f32>()
        } else {
            rand::thread_rng().gen::<f32>()
        }
    } else {
        0.0
    };

    let mut omen: Option<(String, EventImportance)> = None;
    let mut spend: Option<DirectorEventKind> = None;
    {
        let mut state = world.resource_mut::<DirectorState>();
        state.temperament = config;
        state.last_accrual_rate = rate;
        // Accrual runs every tick, including during rest (§4.2).
        state.budget = (state.budget + rate).clamp(0.0, config.budget_max);

        // §4.7: a crossfire strike imposes a HALF-length rest and is
        // recorded in the recency ledger at its rated level (45).
        if let Some(cf_tick) = crossfire_last_tick {
            if cf_tick != 0 && cf_tick != state.last_crossfire_tick_seen {
                state.last_crossfire_tick_seen = cf_tick;
                let until = tick.saturating_add(config.rest_ticks / 2);
                if until > state.rest_until {
                    state.rest_until = until;
                }
                state.record(DirectorEventKind::CrossfireStrike, cf_tick, 0.0, false);
            }
        }

        // §4.5: tension band + omens, once per upward crossing per
        // rest cycle (mask clears on spend).
        let band = tension_band_for(
            &config,
            state.budget,
            tick,
            state.rest_until,
            state.scenario_start_tick,
        );
        if band.rank() > state.tension_band.rank() {
            let bit = 1u8 << band.rank();
            if state.omened_mask & bit == 0 {
                state.omened_mask |= bit;
                omen = Some(omen_text(&config, &state));
            }
        }
        state.tension_band = band;

        // §4.3: the spend check.
        if is_check_tick {
            state.saving_for_major = is_saving_for_major(&config, state.budget)
                && tick >= state.scenario_start_tick.saturating_add(config.grace_ticks)
                && tick >= state.rest_until;
            if let Some(kind) = spend_decision(&config, &state, tick, roll) {
                let price = effective_price(&config, kind);
                state.budget = (state.budget - price).max(0.0);
                state.rest_until = tick.saturating_add(config.rest_ticks);
                state.spends_total += 1;
                state.last_spend_tick = tick;
                state.last_spend_kind = Some(kind);
                state.last_spend_forced = false;
                state.saving_for_major = false;
                state.omened_mask = 0;
                state.record(kind, tick, price, false);
                if kind == DirectorEventKind::SporeOutbreak {
                    state.outbreak_token = true;
                }
                state.tension_band = tension_band_for(
                    &config,
                    state.budget,
                    tick,
                    state.rest_until,
                    state.scenario_start_tick,
                );
                spend = Some(kind);
            }
        }
    }

    if let Some((text, importance)) = omen {
        if let Some(mut events) =
            world.get_resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
        {
            events.send(AddChronicleEvent { text, importance });
        }
    }
    if let Some(DirectorEventKind::PirateRaid) = spend {
        fire_kind(world, DirectorEventKind::PirateRaid);
    }
}

// ---------------------------------------------------------------------------
// Spec 1388 §3 tests — behavioral proof through the LIVE schedule only
// (`setup_world_with_config` + `run_simulation_tick`), no direct-call
// substitutes. Pure-data tests cover the temperament table and registry.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::biology::invasive::{InvasiveConfig, Spore};
    use crate::layer1::core::chronicle::Chronicle;
    use crate::layer2::trade::blockade::TradeShipArrivalEvent;
    use crate::setup::{setup_world_with_config, SetupConfig};
    use crate::shared::scenario::StartScenarioId;

    fn live_world(scenario: StartScenarioId) -> World {
        setup_world_with_config(SetupConfig {
            scenario,
            ..SetupConfig::default()
        })
    }

    fn set_config(world: &mut World, config: DirectorConfig) {
        world.insert_resource(config);
        world.init_resource::<DirectorState>();
        world.resource_mut::<DirectorState>().temperament = config;
    }

    fn tick_n(world: &mut World, n: u64) {
        for _ in 0..n {
            crate::simulation::run_simulation_tick(world);
        }
    }

    fn spore_count(world: &mut World) -> usize {
        world.query::<&Spore>().iter(world).count()
    }

    #[test]
    fn temperament_ordering_matches_era_table() {
        let classic = DirectorConfig::temperament_for(StartScenarioId::Classic);
        let ground = DirectorConfig::temperament_for(StartScenarioId::GroundSurvival);
        let social = DirectorConfig::temperament_for(StartScenarioId::SocialDrama);
        let l2r = DirectorConfig::temperament_for(StartScenarioId::Layer2Ready);
        // §2: L2R grace and rest are the longest; Classic the shortest.
        assert!(l2r.grace_ticks > classic.grace_ticks);
        assert!(l2r.rest_ticks > classic.rest_ticks);
        assert!(ground.grace_ticks > classic.grace_ticks);
        assert!(social.rest_ticks > classic.rest_ticks);
        // Ground Survival's hostile accrual_mult is the lowest.
        assert!(ground.accrual_mult < classic.accrual_mult);
        assert!(ground.accrual_mult <= social.accrual_mult);
        assert!(ground.accrual_mult <= l2r.accrual_mult);
        // Classic escalation is steepest.
        assert!(classic.year_escalation > ground.year_escalation);
        assert!(classic.year_escalation > l2r.year_escalation);
        // Ground leans into winter; nobody else does.
        assert!((ground.season_weight(Season::Winter) - 1.5).abs() < f32::EPSILON);
        assert!((classic.season_weight(Season::Winter) - 1.0).abs() < f32::EPSILON);
        // Classic class weights favor raids 3:2; others are even.
        assert_eq!((classic.raid_weight, classic.outbreak_weight), (3, 2));
        assert_eq!((l2r.raid_weight, l2r.outbreak_weight), (1, 1));
        // §10 decided defaults: L2R is the only era that reprices (1.1).
        assert!((l2r.cost_mult - 1.1).abs() < f32::EPSILON);
        assert!((classic.cost_mult - 1.0).abs() < f32::EPSILON);
        // Base constants are era-independent (§2).
        for cfg in [classic, ground, social, l2r] {
            assert!((cfg.base_accrual - 0.5).abs() < f32::EPSILON);
            assert_eq!(cfg.check_interval, 25);
        }
    }

    #[test]
    fn threat_levels_and_tiers_match_registry() {
        use DirectorEventKind as K;
        assert_eq!(K::PirateRaid.threat_level(), 60);
        assert_eq!(K::SporeOutbreak.threat_level(), 40);
        assert_eq!(K::CrossfireStrike.threat_level(), 45);
        assert_eq!(K::OrphanedSwarm.threat_level(), 35);
        assert_eq!(K::SymbiontSabotage.threat_level(), 25);
        assert_eq!(K::BuildingFire.threat_level(), 30);
        assert_eq!(K::RefugeeArrival.threat_level(), 10);
        assert_eq!(K::TradeShip.threat_level(), 0);
        assert_eq!(K::MigrantArrival.threat_level(), 0);
        assert_eq!(K::PirateRaid.tier(), ThreatTier::Major);
        assert_eq!(K::SporeOutbreak.tier(), ThreatTier::Standard);
        assert_eq!(K::RefugeeArrival.tier(), ThreatTier::Minor);
        assert_eq!(K::TradeShip.tier(), ThreatTier::Benign);
        // Severe stays EMPTY by decision (§1.3): no kind is rated 75+.
        for kind in [
            K::PirateRaid,
            K::SporeOutbreak,
            K::CrossfireStrike,
            K::OrphanedSwarm,
            K::SymbiontSabotage,
            K::BuildingFire,
            K::RefugeeArrival,
            K::TradeShip,
            K::MigrantArrival,
        ] {
            assert!(kind.threat_level() < 75, "{kind:?} must not be Severe");
        }
    }

    #[test]
    fn no_spend_before_grace_under_forced_budget() {
        // Classic grace is 100 ticks. Even a budget forced to cap at t0
        // cannot spend before then (live schedule).
        let mut world = live_world(StartScenarioId::Classic);
        world.init_resource::<DirectorState>();
        world.resource_mut::<DirectorState>().budget = 120.0;
        tick_n(&mut world, 99);
        let state = world.resource::<DirectorState>();
        assert_eq!(
            state.spends_total, 0,
            "grace is absolute: no hostile spends before tick 100"
        );
        assert!(!state.outbreak_token, "no outbreak token during grace");
    }

    #[test]
    fn rest_blocks_spends_but_not_accrual() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 0,
            check_interval: 1,
            base_accrual: 1.0,
            rest_ticks: 50,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        world.resource_mut::<DirectorState>().budget = 60.0;
        tick_n(&mut world, 1);
        {
            let state = world.resource::<DirectorState>();
            assert_eq!(state.spends_total, 1, "first check must spend");
            assert_eq!(state.rest_until, 50, "rest = spend tick 0 + 50");
        }
        // Refill by hand, then sit inside the rest window: accrual must
        // continue while spends stay blocked (deferral, not forgiveness).
        world.resource_mut::<DirectorState>().budget = 50.0;
        tick_n(&mut world, 10);
        let state = world.resource::<DirectorState>();
        assert_eq!(state.spends_total, 1, "rest must block spends");
        assert!(
            state.budget > 58.0,
            "accrual must continue during rest, budget={}",
            state.budget
        );
    }

    #[test]
    fn spend_after_rest_deducts_threat_level() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 0,
            check_interval: 1,
            base_accrual: 0.0,
            cost_mult: 1.5,
            rest_ticks: 50,
            raid_weight: 1,
            outbreak_weight: 0,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        // Raid price = 60 * 1.5 = 90. Budget exactly 90.
        world.resource_mut::<DirectorState>().budget = 90.0;
        tick_n(&mut world, 1);
        let state = world.resource::<DirectorState>();
        assert_eq!(state.spends_total, 1);
        assert_eq!(state.last_spend_kind, Some(DirectorEventKind::PirateRaid));
        assert!(
            state.budget.abs() < f32::EPSILON,
            "spend must deduct threat_level * cost_mult (90), budget={}",
            state.budget
        );
        assert_eq!(state.rest_until, 50);
        let entry = state.recent.last().expect("ledger entry recorded");
        assert_eq!(entry.kind, DirectorEventKind::PirateRaid);
        assert!((entry.price - 90.0).abs() < f32::EPSILON);
        assert!(!entry.forced);
    }

    #[test]
    fn director_raid_spend_resolves_via_bridge() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 0,
            check_interval: 1,
            base_accrual: 0.0,
            raid_weight: 1,
            outbreak_weight: 0,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        world.resource_mut::<DirectorState>().budget = 60.0;
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.food = 40.0;
            res.metal = 50.0;
        }
        tick_n(&mut world, 3);
        let state = world.resource::<DirectorState>();
        assert!(state.spends_total >= 1, "director must spend on raid");
        assert_eq!(state.last_spend_kind, Some(DirectorEventKind::PirateRaid));
        // Bridge consequence, unchanged: metal -20 is the clean
        // signature (no other metal sink fires in 3 ticks; food is
        // churned by consumption/production in the living economy, so
        // it is only bounded above). The bridge also stamps every pop
        // with its unique "Pirate Raid" morale modifier.
        let res = world.resource::<ColonyResources>();
        assert!(
            res.food <= 40.0,
            "food must not grow from a raid, food={}",
            res.food
        );
        assert!(
            (res.metal - 30.0).abs() < f32::EPSILON,
            "bridge must steal exactly 20 metal, metal={}",
            res.metal
        );
        let mut q = world.query::<&crate::layer1::social::morale::Morale>();
        assert!(
            q.iter(&world)
                .any(|m| m.modifiers.iter().any(|md| md.label == "Pirate Raid")),
            "bridge must stamp pops with the Pirate Raid morale modifier"
        );
        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|e| e.text.contains("Pirates have raided the colony!")),
            "bridge Major chronicle must fire"
        );
    }

    #[test]
    fn outbreak_requires_director_token() {
        let mut world = live_world(StartScenarioId::Classic);
        // Director disabled: no organic token can appear; the lane is
        // purely the manually-set token under test.
        let config = DirectorConfig {
            enabled: false,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        world.insert_resource(InvasiveConfig {
            arrival_chance: 1.0,
            exotic_arrival_chance: 1.0,
            ..InvasiveConfig::default()
        });
        assert!(!world.resource::<DirectorState>().outbreak_token);
        for _ in 0..5 {
            world
                .resource_mut::<bevy_ecs::event::Events<TradeShipArrivalEvent>>()
                .send(TradeShipArrivalEvent {
                    cargo_value: 100.0,
                    faction: "Xeno Traders".to_string(),
                });
        }
        tick_n(&mut world, 3);
        assert_eq!(
            spore_count(&mut world),
            0,
            "no token -> arrivals never seed"
        );
        // Grant the token: the first qualifying arrival seeds, and the
        // token is consumed.
        world.resource_mut::<DirectorState>().outbreak_token = true;
        world
            .resource_mut::<bevy_ecs::event::Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 100.0,
                faction: "Xeno Traders".to_string(),
            });
        tick_n(&mut world, 2);
        assert!(
            spore_count(&mut world) >= 1,
            "token -> qualifying arrival must seed"
        );
        assert!(
            !world.resource::<DirectorState>().outbreak_token,
            "token must be consumed by the seeding"
        );
    }

    #[test]
    fn spores_senesce_and_outbreaks_recur() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            enabled: false,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        world.insert_resource(InvasiveConfig {
            arrival_chance: 1.0,
            exotic_arrival_chance: 1.0,
            max_spores: 2,
            spore_lifespan: 5,
        });
        // Saturate the cap with two aged-0 spores that cannot spread yet.
        for x in [5, 6] {
            world.spawn(Spore {
                position: crate::layer1::map::GridPosition { x, y: 5 },
                strain: crate::layer1::biology::invasive::SporeStrain::Xenofungus,
                spread_timer: 999,
                age: 0,
            });
        }
        assert_eq!(spore_count(&mut world), 2, "cap saturated");
        tick_n(&mut world, 8);
        assert_eq!(
            spore_count(&mut world),
            0,
            "spores must senesce at spore_lifespan and despawn"
        );
        // Below cap again: a later outbreak (token + arrival) fires —
        // the 40/40 saturation regression from §1.2 must not recur.
        world.resource_mut::<DirectorState>().outbreak_token = true;
        world
            .resource_mut::<bevy_ecs::event::Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 100.0,
                faction: "Xeno Traders".to_string(),
            });
        tick_n(&mut world, 2);
        assert!(
            spore_count(&mut world) >= 1,
            "outbreaks must recur after senescence drops the count below cap"
        );
    }

    #[test]
    fn crossfire_strike_imposes_half_rest() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 10_000,
            base_accrual: 0.0,
            rest_ticks: 100,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        tick_n(&mut world, 1); // settle at tick 1; no strike seen yet
        assert_eq!(world.resource::<DirectorState>().rest_until, 0);
        world.insert_resource(CrossfireStats {
            total_strikes: 1,
            buildings_destroyed: 0,
            last_target: None,
            last_tick: 1,
        });
        tick_n(&mut world, 1);
        let state = world.resource::<DirectorState>();
        assert_eq!(
            state.rest_until, 51,
            "crossfire at tick 1 must impose half of rest 100 from observation tick 1"
        );
        assert!(
            state
                .recent
                .iter()
                .any(|e| e.kind == DirectorEventKind::CrossfireStrike && e.tick == 1),
            "crossfire strike must be recorded in the recency ledger"
        );
        assert_eq!(state.spends_total, 0, "crossfire is rated, not spent");
    }

    #[test]
    fn omen_fires_once_per_band_crossing() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 10_000,
            base_accrual: 2.0,
            // Cap below the High threshold (30) and below affordability
            // (40): the band rises to Rising and dwells there.
            budget_max: 20.0,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        tick_n(&mut world, 30);
        let state = world.resource::<DirectorState>();
        assert_eq!(state.tension_band, TensionBand::Rising);
        assert_eq!(state.spends_total, 0);
        let chronicle = world.resource::<Chronicle>();
        let omens = chronicle
            .events
            .iter()
            .filter(|e| e.text.contains("threat gathers"))
            .count();
        assert_eq!(
            omens, 1,
            "exactly one omen per upward band crossing per rest cycle"
        );
    }

    #[test]
    fn accrual_is_distress_insensitive() {
        let mut world = live_world(StartScenarioId::Classic);
        let config = DirectorConfig {
            grace_ticks: 10_000,
            ..DirectorConfig::default()
        };
        set_config(&mut world, config);
        // Pin the wealth scaler at its cap so ordinary consumption
        // cannot drift the rate between the two samples.
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.food = 1000.0;
            res.metal = 1000.0;
        }
        tick_n(&mut world, 1);
        let calm_rate = world.resource::<DirectorState>().last_accrual_rate;
        assert!(calm_rate > 0.0, "accrual must be running");
        let calm_pops = {
            let mut q = world.query::<&Pop>();
            q.iter(&world).count()
        };
        // Tank morale and damage every pop: distress must not move the
        // rate at all (no rubber-banding, §Decided).
        {
            let mut q = world.query::<&mut crate::layer1::social::morale::Morale>();
            for mut morale in q.iter_mut(&mut world) {
                morale.value = 0.0;
            }
            let mut q = world.query::<&mut crate::layer1::health::Health>();
            for mut health in q.iter_mut(&mut world) {
                health.current = 25.0;
            }
        }
        tick_n(&mut world, 1);
        let distressed_rate = world.resource::<DirectorState>().last_accrual_rate;
        let distressed_pops = {
            let mut q = world.query::<&Pop>();
            q.iter(&world).count()
        };
        // Compare pop-scaler-normalized rates: colony SIZE is a legal
        // accrual input (a migrant may land between samples), distress
        // is not — after normalizing size out, the rates must match.
        let calm_norm = calm_rate / pop_scaler(calm_pops);
        let distressed_norm = distressed_rate / pop_scaler(distressed_pops);
        assert!(
            (calm_norm - distressed_norm).abs() < 1e-4,
            "accrual must be distress-insensitive: calm {calm_rate} (pops {calm_pops}) vs distressed {distressed_rate} (pops {distressed_pops})"
        );
        // Pure-fn cross-check with the actual aggregates of the world:
        // scalers take no distress inputs at all.
        let season = world
            .get_resource::<SeasonState>()
            .map_or(Season::Spring, |s| s.current_season);
        let aggs = DirectorAggregates {
            pop_count: calm_pops,
            wealth: 2000.0,
            season,
            tick: 1,
        };
        assert!(
            (accrual_rate(&config, &aggs) - calm_rate).abs() < 0.01,
            "live rate {} must match the pure fn {}",
            calm_rate,
            accrual_rate(&config, &aggs)
        );
    }

    #[test]
    fn stats_line_contains_director_fields() {
        let mut world = live_world(StartScenarioId::Classic);
        tick_n(&mut world, 1);
        let label = director_stats_label(&world);
        for needle in ["spends=", "last=", "rest_until=", "accrual=", "cost_mult="] {
            assert!(
                label.contains(needle),
                "STATS director= field must contain {needle}: {label}"
            );
        }
        assert!(
            label.contains("calm:") || label.contains("rising:"),
            "label must open with the tension band: {label}"
        );
    }
}
