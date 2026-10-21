//! Spec 1387: The Grind — Structural Dissatisfaction (Cyberpunk Misery Engine).
//!
//! # Why this ledger exists (§1.1 in brief)
//!
//! Morale is a memoryless cache: `update_morale_cache_system` recomputes
//! `Morale.value` from scratch every tick out of `calculate_raw_morale`,
//! whose base is the plain average of the pop's *current* needs. The
//! utility AI self-services needs in ~20 ticks, so the base snaps back
//! the moment a pop eats or naps — yesterday's conditions do not exist.
//! The only persistent negative channel, `Memories`, is capped at 12
//! entries with impacts that all decay to zero, and the one integrator,
//! `StressTracker`, is symmetric (+1/tick miserable, −1/tick otherwise),
//! so recovery outruns accumulation by construction. Result (Spec 357
//! audit): the vandalism lane of `graffiti_placement_system`
//! (morale < 0.2) and the negative lane of `post_grievance_system`
//! (needs morale < 0.2) were effectively unreachable in organic play.
//!
//! # The fix, by construction — not by weakening recovery
//!
//! Each pop carries a [`Dissatisfaction`] ledger: five source channels
//! that **accrue while their structural condition holds and decay ~20×
//! slower when it lifts**, so a grievance outlives its cause.
//! [`dissatisfaction_bridge_system`] maintains exactly one persistent
//! `MoodModifier` ("Ground Down") per pop, valued at
//! `MORALE_BRIDGE_COEF × total`. Because the morale cache sums
//! modifiers *after* raw morale every tick, structural misery depresses
//! cached morale regardless of how fed and rested the pop is.
//!
//! No existing gate, threshold, or recovery constant is touched:
//! `calculate_raw_morale`, needs decay, `StressTracker` rates, graffiti
//! placement chances, mental-break thresholds, and black-market
//! caps/cooldowns are all exactly as they were.
//!
//! # Mental breaks / stress: NO CHANGE (§4.5.6)
//!
//! With morale durably depressed, `StressTracker` finally integrates —
//! its −1/tick drain only wins when morale actually recovers, which now
//! requires fixing a *condition*, not taking a nap. Do not "helpfully"
//! retune recovery constants in this module's name; if recovery itself
//! is ever revisited that is a separate, evidence-led proposal.
//!
//! # Scenario notes (§4.9)
//!
//! No per-scenario constants and no dissatisfaction discounts. Classic
//! colonies (thin wages, rough starts) simmer earlier; Layer 2 Ready's
//! material head start (housing, stocked food, drones freeing pop
//! labour) is the sanctioned relief — the *conditions* differ, the
//! engine doesn't.
//!
//! # Performance (§5)
//!
//! Per-tick systems touch only the pop's own components. The expensive
//! channels (housing scan, colony wallet median) run on an epoch of
//! [`EVAL_EPOCH_TICKS`] ticks and apply ×epoch accrual/decay.

use crate::layer1::architecture::housing::Housing;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::remittances::MigrantFamilyInfo;
use crate::layer1::economy::{ColonyPrices, Wallet};
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::needs::Needs;
use crate::layer1::palette_fatigue::{calculate_palette_fatigue, DietaryHistory};
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::private_stash::PrivateStash;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Per-source saturation.
pub const CHANNEL_CAP: f32 = 1.0;
/// ~500 ticks of a held condition saturates a channel.
pub const ACCRUE_PER_TICK: f32 = 0.002;
/// Multiplier for the severe form of a condition.
pub const ACCRUE_DOUBLE: f32 = 2.0;
/// 20× slower than accrual — forgiveness takes far longer than the offence.
pub const DECAY_PER_TICK: f32 = 0.0001;
/// One saturated channel ≈ 0.34 total; three ≈ 1.0.
pub const TOTAL_WEIGHT: f32 = 0.34;
/// Display + status-bar threshold.
pub const BAND_SIMMERING: f32 = 0.25;
/// Outlet threshold (§4.5).
pub const BAND_BITTER: f32 = 0.55;
/// Display only in v1 (breaks fire via existing stress gates).
pub const BAND_BREAKING: f32 = 0.85;
/// Bridge modifier value = coef × total.
pub const MORALE_BRIDGE_COEF: f32 = -0.7;
/// Cadence for expensive channels (§4.2).
pub const EVAL_EPOCH_TICKS: u32 = 20;
/// Label of the single persistent bridge modifier.
pub const GROUND_DOWN_LABEL: &str = "Ground Down";
/// Bridge modifier duration, refreshed every tick so it never lapses
/// while the ledger says miserable (Spec 353 refresh-never-stack pattern).
pub const GROUND_DOWN_DURATION: u32 = 100;
/// Status-bar simmering segment: ON-dwell in ticks (item 104 discipline).
pub const SIMMERING_ON_DWELL_TICKS: u64 = 20;
/// Status-bar simmering segment: off-linger in ticks (item 104 discipline).
pub const SIMMERING_OFF_LINGER_TICKS: u64 = 30;
/// Share of pops at/above SIMMERING that lights the status-bar segment.
pub const SIMMERING_SHARE_THRESHOLD: f32 = 0.20;
/// Share of pops Bitter-or-worse that fires the Major chronicle entry.
pub const BITTER_SHARE_MAJOR: f32 = 0.25;
/// Share below which the Major chronicle entry re-arms.
pub const BITTER_SHARE_REARM: f32 = 0.10;

/// Structural misery sources. Each is a *condition of the colony*,
/// not an event: it accrues while it holds and forgives slowly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrindSource {
    /// Wages don't reach the food price (Wallet vs ColonyPrices).
    ScripSqueeze,
    /// No quarters, or quarters over capacity / awful.
    RoughSleeping,
    /// Rest held low by the work treadmill.
    RestDenial,
    /// Palette fatigue: the same meal forever.
    Monotony,
    /// Watching richer colonists live better.
    RelativeDeprivation,
}

impl GrindSource {
    /// All sources, in channel order.
    pub const ALL: [Self; 5] = [
        Self::ScripSqueeze,
        Self::RoughSleeping,
        Self::RestDenial,
        Self::Monotony,
        Self::RelativeDeprivation,
    ];

    /// Channel index in [`Dissatisfaction::channels`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::ScripSqueeze => 0,
            Self::RoughSleeping => 1,
            Self::RestDenial => 2,
            Self::Monotony => 3,
            Self::RelativeDeprivation => 4,
        }
    }

    /// Plain-language line for the inspector / misery panel (§4.7).
    /// UI never matches on the enum for text — it calls this.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ScripSqueeze => "Can't cover the next meals",
            Self::RoughSleeping => "Sleeps rough",
            Self::RestDenial => "Running on empty",
            Self::Monotony => "Paste again. Paste forever.",
            Self::RelativeDeprivation => "Watches richer colonists eat",
        }
    }

    /// Short name for panels and chronicle copy.
    #[must_use]
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::ScripSqueeze => "Scrip squeeze",
            Self::RoughSleeping => "Rough sleeping",
            Self::RestDenial => "Rest denial",
            Self::Monotony => "Monotony",
            Self::RelativeDeprivation => "Relative deprivation",
        }
    }
}

/// Display/threshold band of a pop's current total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum GrindBand {
    /// Below SIMMERING.
    #[default]
    Content,
    /// At/above SIMMERING.
    Simmering,
    /// At/above BITTER — outlets open.
    Bitter,
    /// At/above BREAKING — display only in v1.
    Breaking,
}

impl GrindBand {
    /// Band for a total.
    #[must_use]
    pub fn from_total(total: f32) -> Self {
        if total >= BAND_BREAKING {
            Self::Breaking
        } else if total >= BAND_BITTER {
            Self::Bitter
        } else if total >= BAND_SIMMERING {
            Self::Simmering
        } else {
            Self::Content
        }
    }

    /// UI label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::Simmering => "Simmering",
            Self::Bitter => "Bitter",
            Self::Breaking => "Breaking",
        }
    }
}

/// Per-pop dissatisfaction ledger (Spec 1387).
#[derive(Component, Debug, Clone)]
pub struct Dissatisfaction {
    /// One channel per GrindSource, 0.0..=CHANNEL_CAP, indexed by source.
    pub channels: [f32; 5],
    /// Ticks since the last expensive-channel evaluation (§4.2).
    pub eval_cooldown: u32,
    /// Whether the Bitter-crossing `GroundDown` memory is armed. Fires
    /// once per crossing; re-arms on dropping a full band below Bitter.
    pub bitter_memory_armed: bool,
}

impl Default for Dissatisfaction {
    fn default() -> Self {
        Self {
            channels: [0.0; 5],
            eval_cooldown: 0,
            bitter_memory_armed: true,
        }
    }
}

impl Dissatisfaction {
    /// Current value of one channel.
    #[must_use]
    pub fn channel(&self, source: GrindSource) -> f32 {
        self.channels[source.index()]
    }

    /// Set one channel, clamped to `0.0..=CHANNEL_CAP`.
    pub fn set_channel(&mut self, source: GrindSource, value: f32) {
        self.channels[source.index()] = value.clamp(0.0, CHANNEL_CAP);
    }

    /// Weighted colony-visible misery, 0.0..=1.0.
    /// total = clamp01(sum(channels) * TOTAL_WEIGHT)
    #[must_use]
    pub fn total(&self) -> f32 {
        (self.channels.iter().sum::<f32>() * TOTAL_WEIGHT).clamp(0.0, 1.0)
    }

    /// Display/threshold band of the current total.
    #[must_use]
    pub fn band(&self) -> GrindBand {
        GrindBand::from_total(self.total())
    }

    /// The two strongest channels, strongest first (inspector copy).
    /// Zero channels are omitted.
    #[must_use]
    pub fn top_sources(&self) -> [Option<(GrindSource, f32)>; 2] {
        let mut ranked: Vec<(GrindSource, f32)> = GrindSource::ALL
            .iter()
            .map(|s| (*s, self.channel(*s)))
            .filter(|(_, v)| *v > 0.0)
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        [
            ranked.first().copied(),
            ranked.get(1).copied(),
        ]
    }

    fn accrue(&mut self, source: GrindSource, amount: f32) {
        self.set_channel(source, self.channel(source) + amount);
    }

    fn decay(&mut self, source: GrindSource, amount: f32) {
        self.set_channel(source, self.channel(source) - amount);
    }
}

/// Colony wallet median, computed once per epoch by the accrual system
/// (§4.2.5: the deprivation channel is perception, so it samples).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ColonyWalletMedian {
    /// Median effective wealth (credits + carried-stash total) last epoch.
    pub median: f32,
}

/// Chronicle bookkeeping for the Grind (§4.7): first-Bitter Standard
/// entry (once) and the quarter-colony Major entry (re-arming).
#[derive(Resource, Debug, Clone)]
pub struct GrindChronicleState {
    /// The first pop crossing into Bitter has been chronicled.
    pub first_bitter_reported: bool,
    /// The Major share entry is armed (re-arms below the re-arm share).
    pub major_armed: bool,
}

impl Default for GrindChronicleState {
    fn default() -> Self {
        Self {
            first_bitter_reported: false,
            major_armed: true,
        }
    }
}

/// Dwell-filtered display state for the status-bar simmering segment
/// (item 104 discipline: 20-tick ON-dwell, 30-tick linger, and while
/// visible the segment deterministically owns its tail slot).
#[derive(Resource, Debug, Clone, Default)]
pub struct SimmeringDisplay {
    /// Whether the segment currently owns its status-bar slot.
    pub visible: bool,
    /// Latched simmering count from the last qualifying tick.
    pub simmering: usize,
    /// Latched pop total from the last qualifying tick.
    pub total_pops: usize,
    simmer_since: Option<u64>,
    last_qualifying_tick: Option<u64>,
}

impl SimmeringDisplay {
    /// Advance the dwell state machine by one tick. Pure and
    /// deterministic — unit tests drive this directly.
    pub fn update(&mut self, tick: u64, simmering: usize, total_pops: usize) {
        #[allow(clippy::cast_precision_loss)]
        let qualifies = total_pops > 0
            && (simmering as f32 / total_pops as f32) >= SIMMERING_SHARE_THRESHOLD;
        if qualifies {
            if self.simmer_since.is_none() {
                self.simmer_since = Some(tick);
            }
            self.last_qualifying_tick = Some(tick);
            self.simmering = simmering;
            self.total_pops = total_pops;
            let held_for = tick.saturating_sub(self.simmer_since.unwrap_or(tick));
            if held_for >= SIMMERING_ON_DWELL_TICKS {
                self.visible = true;
            }
        } else {
            self.simmer_since = None;
            if self.visible {
                let linger_expired = self.last_qualifying_tick.is_some_and(|last| {
                    tick.saturating_sub(last) > SIMMERING_OFF_LINGER_TICKS
                });
                if linger_expired {
                    self.visible = false;
                    self.last_qualifying_tick = None;
                    self.simmering = 0;
                    self.total_pops = 0;
                }
            } else {
                self.last_qualifying_tick = None;
            }
        }
    }
}

type AccrualPopQuery<'a> = (
    Entity,
    &'a mut Dissatisfaction,
    Option<&'a Wallet>,
    Option<&'a Needs>,
    Option<&'a DietaryHistory>,
    Option<&'a Memories>,
    Option<&'a MigrantFamilyInfo>,
    Option<&'a PrivateStash>,
);

/// Accrual system (§4.2): cheap channels every tick, expensive channels
/// (RoughSleeping, RelativeDeprivation) on the epoch.
#[allow(clippy::too_many_arguments)]
pub fn dissatisfaction_accrual_system(
    mut pops: Query<AccrualPopQuery<'_>>,
    housings: Query<&Housing>,
    prices: Option<Res<ColonyPrices>>,
    mut median_res: ResMut<ColonyWalletMedian>,
) {
    let food_price = prices.map_or(1.0, |p| p.food_price);

    // Snapshot housing residency once per tick (the scan itself is the
    // epoch cost — it only *applies* on a pop's epoch tick).
    let mut housed: Vec<Entity> = Vec::new();
    let mut over_capacity: Vec<Entity> = Vec::new();
    for housing in housings.iter() {
        let over = housing.residents.len() > housing.capacity;
        for resident in &housing.residents {
            housed.push(*resident);
            if over {
                over_capacity.push(*resident);
            }
        }
    }

    // Wealth snapshot for the epoch median (credits + carried stash).
    let mut wealths: Vec<f32> = pops
        .iter()
        .map(|(_, _, wallet, _, _, _, _, stash)| {
            let credits = wallet.map_or(0.0, |w| w.credits);
            let stash_value = stash.map_or(0.0, PrivateStash::total);
            credits + stash_value
        })
        .collect();
    wealths.sort_by(f32::total_cmp);
    let median = if wealths.is_empty() {
        0.0
    } else {
        wealths[wealths.len() / 2]
    };

    for (
        entity,
        mut dissat,
        wallet,
        needs,
        history,
        memories,
        remittance,
        stash,
    ) in pops.iter_mut()
    {
        // --- Cheap channels (per tick) ---

        // 1. ScripSqueeze: can't cover the next meals.
        if let Some(wallet) = wallet {
            let spoken_for = remittance.map_or(0.0, |m| m.remittance_target);
            let effective = (wallet.credits - spoken_for).max(0.0);
            if effective < food_price {
                dissat.accrue(GrindSource::ScripSqueeze, ACCRUE_PER_TICK * ACCRUE_DOUBLE);
            } else if effective < 2.0 * food_price {
                dissat.accrue(GrindSource::ScripSqueeze, ACCRUE_PER_TICK);
            } else {
                dissat.decay(GrindSource::ScripSqueeze, DECAY_PER_TICK);
            }
        }

        // 2. RestDenial: rest held low by the treadmill.
        if let Some(needs) = needs {
            if needs.rest < 0.15 {
                dissat.accrue(GrindSource::RestDenial, ACCRUE_PER_TICK * ACCRUE_DOUBLE);
            } else if needs.rest < 0.30 {
                dissat.accrue(GrindSource::RestDenial, ACCRUE_PER_TICK);
            } else {
                dissat.decay(GrindSource::RestDenial, DECAY_PER_TICK);
            }
        }

        // 3. Monotony: the same meal forever (first live caller of
        // palette fatigue).
        if let Some(history) = history {
            if calculate_palette_fatigue(history) > 0.6 {
                dissat.accrue(GrindSource::Monotony, ACCRUE_PER_TICK);
            } else {
                dissat.decay(GrindSource::Monotony, DECAY_PER_TICK);
            }
        }

        // --- Epoch channels (every EVAL_EPOCH_TICKS) ---
        if dissat.eval_cooldown > 0 {
            dissat.eval_cooldown -= 1;
            continue;
        }
        dissat.eval_cooldown = EVAL_EPOCH_TICKS - 1;
        #[allow(clippy::cast_precision_loss)]
        let epoch = EVAL_EPOCH_TICKS as f32;
        median_res.median = median;

        // 4. RoughSleeping: no quarters, over capacity, or awful rooms.
        let is_housed = housed.contains(&entity);
        let is_over = over_capacity.contains(&entity);
        let awful_weight = memories.map_or(0.0, |m| {
            m.memory_weight(MemoryType::SleptInAwfulRoom)
                + m.memory_weight(MemoryType::SleptInDullRoom)
        });
        let rough_conditions = usize::from(!is_housed)
            + usize::from(is_over)
            + usize::from(awful_weight >= 0.5);
        if rough_conditions >= 2 {
            dissat.accrue(
                GrindSource::RoughSleeping,
                ACCRUE_PER_TICK * ACCRUE_DOUBLE * epoch,
            );
        } else if rough_conditions == 1 {
            dissat.accrue(GrindSource::RoughSleeping, ACCRUE_PER_TICK * epoch);
        } else {
            dissat.decay(GrindSource::RoughSleeping, DECAY_PER_TICK * epoch);
        }

        // 5. RelativeDeprivation: perception — own wealth vs the median.
        let own = wallet.map_or(0.0, |w| w.credits)
            + stash.map_or(0.0, PrivateStash::total);
        if median > 0.0 && own < 0.5 * median {
            dissat.accrue(
                GrindSource::RelativeDeprivation,
                ACCRUE_PER_TICK * epoch,
            );
        } else {
            dissat.decay(
                GrindSource::RelativeDeprivation,
                DECAY_PER_TICK * epoch,
            );
        }
    }
}

/// The morale bridge (§4.3): maintain exactly one "Ground Down"
/// modifier per pop, refreshed — never stacking (Spec 353 pattern).
/// Also writes the `GroundDown` memory at Bitter crossings.
pub fn dissatisfaction_bridge_system(
    mut pops: Query<(&Dissatisfaction, &mut Morale)>,
) {
    pops.par_iter_mut().for_each(|(dissat, mut morale)| {
        let total = dissat.total();
        if total > 0.01 {
            let value = MORALE_BRIDGE_COEF * total;
            if let Some(existing) = morale
                .modifiers
                .iter_mut()
                .find(|m| m.label == GROUND_DOWN_LABEL)
            {
                existing.value = value;
                existing.duration = GROUND_DOWN_DURATION;
            } else {
                morale.add_modifier(MoodModifier {
                    label: GROUND_DOWN_LABEL.to_string(),
                    value,
                    duration: GROUND_DOWN_DURATION,
                });
            }
        } else {
            morale.modifiers.retain(|m| m.label != GROUND_DOWN_LABEL);
        }
    });
}

/// Band-crossing memories (§4.3): crossing into Bitter adds a
/// `MemoryType::GroundDown` memory once; re-arms below SIMMERING.
pub fn dissatisfaction_memory_system(
    mut pops: Query<(&mut Dissatisfaction, &mut Memories)>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for (mut dissat, mut memories) in pops.iter_mut() {
        let total = dissat.total();
        if total >= BAND_BITTER && dissat.bitter_memory_armed {
            memories.add_with_intensity(MemoryType::GroundDown, tick, 1.0);
            dissat.bitter_memory_armed = false;
        } else if total < BAND_SIMMERING {
            dissat.bitter_memory_armed = true;
        }
    }
}

/// Chronicle bridges (§4.7): first Bitter pop (Standard, once) and the
/// quarter-colony Bitter share (Major, re-arming below 10%).
pub fn dissatisfaction_chronicle_system(
    pops: Query<(&Dissatisfaction, Option<&PopName>), With<Pop>>,
    mut state: ResMut<GrindChronicleState>,
    mut events: EventWriter<AddChronicleEvent>,
) {
    let mut total_pops = 0usize;
    let mut bitter = 0usize;
    let mut first_bitter: Option<(String, GrindSource)> = None;
    for (dissat, name) in pops.iter() {
        total_pops += 1;
        if dissat.total() >= BAND_BITTER {
            bitter += 1;
            if first_bitter.is_none() {
                let top = dissat
                    .top_sources()
                    .first()
                    .copied()
                    .flatten()
                    .map_or(GrindSource::ScripSqueeze, |(s, _)| s);
                first_bitter = Some((
                    name.map_or_else(|| "A colonist".to_string(), |n| n.0.clone()),
                    top,
                ));
            }
        }
    }
    if total_pops == 0 {
        return;
    }
    if !state.first_bitter_reported {
        if let Some((name, top)) = first_bitter {
            events.send(AddChronicleEvent {
                text: format!(
                    "THE GRIND: {name} is coming apart quietly — {}.",
                    top.label()
                ),
                importance: EventImportance::Standard,
            });
            state.first_bitter_reported = true;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let share = bitter as f32 / total_pops as f32;
    if share >= BITTER_SHARE_MAJOR && state.major_armed {
        events.send(AddChronicleEvent {
            text: "THE GRIND: A quarter of the colony is bitter or worse — the misery is structural now, not personal.".to_string(),
            importance: EventImportance::Major,
        });
        state.major_armed = false;
    } else if share < BITTER_SHARE_REARM {
        state.major_armed = true;
    }
}

/// Refresh the dwell-filtered simmering display (§4.7 / item 104).
pub fn update_simmering_display_system(
    time: Res<SimulationTime>,
    pops: Query<&Dissatisfaction, With<Pop>>,
    mut display: ResMut<SimmeringDisplay>,
) {
    let mut simmering = 0usize;
    let mut total = 0usize;
    for dissat in pops.iter() {
        total += 1;
        if dissat.total() >= BAND_SIMMERING {
            simmering += 1;
        }
    }
    display.update(time.tick, simmering, total);
}

/// Colony dissatisfaction snapshot for STATS / the misery panel:
/// (mean total, max total, simmering count, bitter count).
#[must_use]
pub fn dissatisfaction_stats(world: &mut World) -> (f32, f32, usize, usize) {
    let mut query = world.query_filtered::<&Dissatisfaction, With<Pop>>();
    let mut sum = 0.0f32;
    let mut max = 0.0f32;
    let mut count = 0usize;
    let mut simmering = 0usize;
    let mut bitter = 0usize;
    for dissat in query.iter(world) {
        let total = dissat.total();
        sum += total;
        max = max.max(total);
        count += 1;
        if total >= BAND_SIMMERING {
            simmering += 1;
        }
        if total >= BAND_BITTER {
            bitter += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = if count > 0 { sum / count as f32 } else { 0.0 };
    (mean, max, simmering, bitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::ItemType;
    use crate::layer1::map::GridPosition;
    use crate::layer1::palette_fatigue::record_meal;

    fn pop_entities(world: &mut World) -> Vec<Entity> {
        let mut q = world.query_filtered::<Entity, With<Pop>>();
        q.iter(world).collect()
    }

    // --- Test 1 (live schedule, item 78 lesson) ---
    #[test]
    fn test_scrip_squeeze_accrues_while_broke() {
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        world.insert_resource(ColonyPrices {
            food_price: 1.0,
            luxury_price: 5.0,
        });
        let pops = pop_entities(&mut world);
        assert!(!pops.is_empty(), "setup world must have pops");
        for _ in 0..600 {
            for e in &pops {
                if let Some(mut w) = world.get_mut::<Wallet>(*e) {
                    w.credits = 0.0;
                }
            }
            crate::simulation::run_simulation_tick(&mut world);
        }
        // At least one surviving pop must show a saturated scrip channel
        // and morale depressed well below a needs-only prediction.
        let mut checked = 0;
        for e in &pops {
            if let Some(d) = world.get::<Dissatisfaction>(*e) {
                assert!(
                    d.channel(GrindSource::ScripSqueeze) > 0.9,
                    "scrip channel must saturate while broke, got {}",
                    d.channel(GrindSource::ScripSqueeze)
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "some pop must carry Dissatisfaction");
        // Morale check on a pop whose needs are (now) healthy: pin needs
        // to 1.0, run one more tick with the wallet still drained, and
        // the bridge must hold morale far below the 1.0 needs prediction.
        let target = pops
            .iter()
            .copied()
            .find(|e| world.get::<Dissatisfaction>(*e).is_some())
            .unwrap();
        if let Some(mut n) = world.get_mut::<Needs>(target) {
            *n = Needs {
                hunger: 1.0,
                rest: 1.0,
                leisure: 1.0,
                hygiene: 1.0,
            };
        }
        if let Some(mut w) = world.get_mut::<Wallet>(target) {
            w.credits = 0.0;
        }
        crate::simulation::run_simulation_tick(&mut world);
        let morale = world.get::<Morale>(target).unwrap().value;
        assert!(
            morale < 0.75,
            "bridge must depress morale below needs-only prediction, got {morale}"
        );
    }

    // --- Test 2 (live schedule) — THE acceptance test ---
    #[test]
    fn test_misery_wins_against_recovery() {
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        world.insert_resource(ColonyPrices {
            food_price: 1.0,
            luxury_price: 5.0,
        });
        let pops = pop_entities(&mut world);
        assert!(!pops.is_empty());
        let target = pops[0];
        for _ in 0..800 {
            // Pin the target: perfect needs, broke, homeless, monoculture.
            if let Some(mut n) = world.get_mut::<Needs>(target) {
                *n = Needs {
                    hunger: 1.0,
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                };
            }
            if let Some(mut w) = world.get_mut::<Wallet>(target) {
                w.credits = 0.0;
            }
            if let Some(mut h) = world.get_mut::<DietaryHistory>(target) {
                h.recent_meals.clear();
                for _ in 0..5 {
                    record_meal(&mut h, ItemType::Potato);
                }
            }
            // No housing: clear every residency list each tick.
            let housing_entities: Vec<Entity> = {
                let mut q = world.query::<(Entity, &Housing)>();
                q.iter(&world).map(|(e, _)| e).collect()
            };
            for he in housing_entities {
                if let Some(mut housing) = world.get_mut::<Housing>(he) {
                    housing.residents.clear();
                }
            }
            crate::simulation::run_simulation_tick(&mut world);
            if world.get::<Dissatisfaction>(target).is_none() {
                break; // pop died / was replaced; assertion below fails honestly
            }
        }
        let dissat = world
            .get::<Dissatisfaction>(target)
            .expect("target pop must still carry Dissatisfaction");
        assert!(
            dissat.total() >= BAND_BITTER,
            "three forced conditions must saturate past Bitter, total={}",
            dissat.total()
        );
        let morale = world.get::<Morale>(target).unwrap().value;
        assert!(
            morale < 0.35,
            "misery must win against perfect needs: morale={morale} total={}",
            dissat.total()
        );
    }

    // --- Test 3 ---
    #[test]
    fn test_relief_decays_slowly() {
        let mut world = World::new();
        world.insert_resource(ColonyPrices::default());
        world.init_resource::<ColonyWalletMedian>();
        let mut dissat = Dissatisfaction::default();
        dissat.set_channel(GrindSource::ScripSqueeze, 1.0);
        let pop = world
            .spawn((
                Pop,
                dissat,
                Wallet { credits: 100.0 },
                Needs::default(),
                DietaryHistory::default(),
                Memories::default(),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(dissatisfaction_accrual_system);
        for _ in 0..100 {
            schedule.run(&mut world);
        }
        let after100 = world
            .get::<Dissatisfaction>(pop)
            .unwrap()
            .channel(GrindSource::ScripSqueeze);
        assert!(
            after100 > 0.8,
            "grievance must linger: retains {after100} after 100 ticks of relief"
        );
        // NOTE: at DECAY_PER_TICK = ACCRUE/20, full forgiveness of a
        // saturated channel takes ~9,000+ ticks — the spec text's
        // "5000 more" understates its own decided constants, so this
        // leg runs 10,000 (constants win over the prose estimate).
        for _ in 0..10000 {
            schedule.run(&mut world);
        }
        let later = world
            .get::<Dissatisfaction>(pop)
            .unwrap()
            .channel(GrindSource::ScripSqueeze);
        assert!(later < 0.1, "grievances are not life sentences: {later}");
    }

    // --- Test 4 ---
    #[test]
    fn test_bridge_modifier_never_stacks() {
        let mut world = World::new();
        let mut dissat = Dissatisfaction::default();
        dissat.set_channel(GrindSource::ScripSqueeze, 1.0);
        let pop = world.spawn((Pop, dissat, Morale::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems(dissatisfaction_bridge_system);
        for _ in 0..100 {
            schedule.run(&mut world);
        }
        let morale = world.get::<Morale>(pop).unwrap();
        let ground: Vec<_> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == GROUND_DOWN_LABEL)
            .collect();
        assert_eq!(ground.len(), 1, "exactly one Ground Down modifier");
        assert_eq!(ground[0].duration, GROUND_DOWN_DURATION, "duration refreshed");
        assert!((ground[0].value - MORALE_BRIDGE_COEF * 0.34).abs() < 0.001);
    }

    // --- Test 5 (live schedule) ---
    #[test]
    fn test_vandalism_lane_reachable_under_grind() {
        use crate::layer1::building::OccupiedTiles;
        use crate::layer1::graffiti::{GraffitiMap, GraffitiType};
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        let occupied: Vec<(i32, i32)> = world
            .resource::<OccupiedTiles>()
            .0
            .iter()
            .copied()
            .collect();
        assert!(!occupied.is_empty(), "setup world must have buildings");
        let pops = pop_entities(&mut world);
        assert!(!pops.is_empty());
        let mut placed = false;
        for tick in 0..1000 {
            for (i, e) in pops.iter().enumerate() {
                let (ox, oy) = occupied[i % occupied.len()];
                if let Some(mut pos) = world.get_mut::<GridPosition>(*e) {
                    pos.x = ox;
                    pos.y = oy - 1;
                }
                if let Some(mut d) = world.get_mut::<Dissatisfaction>(*e) {
                    d.channels = [1.0, 1.0, 1.0, 0.0, 0.0];
                }
                if let Some(mut n) = world.get_mut::<Needs>(*e) {
                    *n = Needs {
                        hunger: 1.0,
                        rest: 1.0,
                        leisure: 1.0,
                        hygiene: 1.0,
                    };
                }
            }
            crate::simulation::run_simulation_tick(&mut world);
            if world
                .resource::<GraffitiMap>()
                .markings
                .values()
                .any(|g| g.graffiti_type == GraffitiType::Vandalism)
            {
                placed = true;
                println!("vandalism placed under grind at tick {tick}");
                break;
            }
        }
        assert!(
            placed,
            "a saturated pop beside a building must reach the vandalism lane in the live schedule"
        );
    }

    // --- Test 6 ---
    #[test]
    fn test_sabotage_gate_opens_at_bitter() {
        use crate::layer1::actions::sabotage::evaluate_sabotage;
        use crate::layer1::mind::utility_eval_types::{
            PopEvalData, ScorableCandidate, UtilityAIBuffer,
        };
        use crate::layer1::utility_types::ActionType;

        fn buffer_with_advanced() -> UtilityAIBuffer {
            let mut buffer = UtilityAIBuffer {
                pop_data: vec![],
                results: vec![],
                farms: vec![],
                housing: vec![],
                taverns: vec![],
                libraries: vec![],
                refining: vec![],
                work_designations: vec![],
                repair_designations: vec![],
                tame_designations: vec![],
                items: vec![],
                item_entities: vec![],
                stockpiles: vec![],
                stockpile_positions: bevy_utils::HashSet::default(),
                anomalies: vec![],
                hospitals: vec![],
                corpses: vec![],
                graves: vec![],
                unpollinated_crops: vec![],
                repair_structures: vec![],
                wanted_criminals: vec![],
                suspects: vec![],
                offices: vec![],
                walls: vec![],
                enemies: vec![],
                all_structures: vec![],
                showers: vec![],
                hum_sources: vec![],
                gene_banks: vec![],
                residues: vec![],
                cleaning_targets: vec![],
                sanctuaries: vec![],
                mobs: vec![],
                loose_eva_suits: vec![],
                exterior_suit_pending: false,
                pop_pressures: std::collections::HashMap::new(),
            };
            let structure = Entity::from_raw(20);
            let mut cand = ScorableCandidate::new(structure, GridPosition { x: 3, y: 0 });
            cand.is_advanced_tech = true;
            buffer.all_structures.push(cand);
            buffer
        }

        let buffer = buffer_with_advanced();
        let mut data = PopEvalData::test_instance();
        data.is_nostalgic = false;
        data.dissatisfaction_total = BAND_BITTER;
        let result = evaluate_sabotage(&data, &buffer);
        assert!(result.is_some(), "bitter pop must evaluate sabotage");
        let (action, utility, _target) = result.unwrap();
        assert_eq!(action, ActionType::Sabotage);
        let expected = 60.0 + 40.0 * BAND_BITTER;
        assert!((utility - expected).abs() < 0.01, "utility {utility} vs {expected}");
        assert!(utility < 100.0, "cold sabotage stays below breakdown heat");

        let mut below = PopEvalData::test_instance();
        below.is_nostalgic = false;
        below.dissatisfaction_total = BAND_BITTER - 0.01;
        assert!(evaluate_sabotage(&below, &buffer).is_none());
    }

    // --- Test 7 ---
    #[test]
    fn test_bitter_pop_counts_as_unmet_luxury() {
        use crate::layer1::core::integration::update_unmet_luxury_system;
        use crate::layer1::economy::black_market::ColonyStats;
        let mut world = World::new();
        world.init_resource::<ColonyStats>();
        let mut dissat = Dissatisfaction::default();
        dissat.channels = [1.0, 1.0, 0.0, 0.0, 0.0]; // total 0.68 >= BITTER
        world.spawn((
            Pop,
            Needs {
                hunger: 0.9,
                rest: 0.9,
                leisure: 0.9,
                hygiene: 0.9,
            },
            dissat,
        ));
        // A content pop with low leisure also counts (existing lane).
        world.spawn((
            Pop,
            Needs {
                hunger: 0.9,
                rest: 0.9,
                leisure: 0.1,
                hygiene: 0.9,
            },
            Dissatisfaction::default(),
        ));
        // A content pop with high leisure does not count.
        world.spawn((
            Pop,
            Needs {
                hunger: 0.9,
                rest: 0.9,
                leisure: 0.9,
                hygiene: 0.9,
            },
            Dissatisfaction::default(),
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(update_unmet_luxury_system);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<ColonyStats>().unmet_luxury,
            2,
            "bitter pop + low-leisure pop count; content pop does not"
        );
    }

    // --- Test 8 ---
    #[test]
    fn test_grievance_board_uses_effective_morale() {
        use crate::layer1::social::grievances::{post_grievance_system, BulletinBoard, Sentiment};
        let mut world = World::new();
        let board = world
            .spawn((BulletinBoard::default(), GridPosition { x: 0, y: 0 }))
            .id();
        // Ground-down pop: needs morale 0.9, cached morale 0.1.
        let mut morale = Morale::default();
        morale.value = 0.1;
        world.spawn((
            Pop,
            Needs {
                hunger: 0.9,
                rest: 0.9,
                leisure: 0.9,
                hygiene: 0.9,
            },
            morale,
            GridPosition { x: 0, y: 0 },
        ));
        // Mirror-image pop: needs are awful (needs-morale low) but the
        // cached effective morale is 0.9 — under the old needs-based
        // read this pop posted Negative; via effective morale it must
        // post Positive. Together the two pops pin the re-point in
        // both directions.
        let mut morale2 = Morale::default();
        morale2.value = 0.9;
        world.spawn((
            Pop,
            Needs {
                hunger: 0.1,
                rest: 0.1,
                leisure: 0.1,
                hygiene: 0.1,
            },
            morale2,
            GridPosition { x: 0, y: 0 },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(post_grievance_system);
        for _ in 0..500 {
            schedule.run(&mut world);
            let b = world.get::<BulletinBoard>(board).unwrap();
            if b.notes.iter().any(|n| n.sentiment == Sentiment::Negative)
                && b.notes.iter().any(|n| n.sentiment == Sentiment::Positive)
            {
                break;
            }
        }
        let b = world.get::<BulletinBoard>(board).unwrap();
        assert!(
            b.notes.iter().any(|n| n.sentiment == Sentiment::Negative),
            "ground-down pop must post a negative note via effective morale"
        );
        assert!(
            b.notes.iter().any(|n| n.sentiment == Sentiment::Positive),
            "pop with high cached morale must post positive via effective morale, not negative via needs"
        );
    }

    // --- Test 9 ---
    #[test]
    fn test_fresh_colonist_carries_dissatisfaction() {
        use crate::layer1::pop::PopBundle;
        let mut world = World::new();
        let mut rng = rand::thread_rng();
        let pop = world.spawn(PopBundle::random(0, 0, &mut rng)).id();
        let dissat = world
            .get::<Dissatisfaction>(pop)
            .expect("fresh colonist must carry Dissatisfaction");
        assert_eq!(dissat.channels, [0.0; 5]);
        assert_eq!(dissat.total(), 0.0);
        assert_eq!(dissat.band(), GrindBand::Content);
    }

    // --- IP guard (house pattern) ---
    #[test]
    fn test_ip_guard_no_real_world_brands() {
        let banned = [
            "cyberpunk", "Cyberpunk", "night city", "Night City", "arasaka",
            "Arasaka", "weyland", "Weyland", "tyrell", "Tyrell", "blade runner",
            "Blade Runner", "neuromancer", "Neuromancer", "gibson", "Gibson",
        ];
        let mut texts: Vec<String> = Vec::new();
        for source in GrindSource::ALL {
            texts.push(source.label().to_string());
            texts.push(source.short_name().to_string());
        }
        for band in [
            GrindBand::Content,
            GrindBand::Simmering,
            GrindBand::Bitter,
            GrindBand::Breaking,
        ] {
            texts.push(band.label().to_string());
        }
        texts.push(GROUND_DOWN_LABEL.to_string());
        for text in &texts {
            for b in banned {
                assert!(
                    !text.contains(b),
                    "player-facing copy must stay original: '{text}' contains '{b}'"
                );
            }
        }
    }

    // --- Extra: top_sources ordering + band edges ---
    #[test]
    fn test_top_sources_and_bands() {
        let mut d = Dissatisfaction::default();
        d.set_channel(GrindSource::Monotony, 0.4);
        d.set_channel(GrindSource::ScripSqueeze, 0.9);
        d.set_channel(GrindSource::RestDenial, 0.2);
        let top = d.top_sources();
        assert_eq!(top[0].map(|(s, _)| s), Some(GrindSource::ScripSqueeze));
        assert_eq!(top[1].map(|(s, _)| s), Some(GrindSource::Monotony));
        assert_eq!(GrindBand::from_total(0.0), GrindBand::Content);
        assert_eq!(GrindBand::from_total(BAND_SIMMERING), GrindBand::Simmering);
        assert_eq!(GrindBand::from_total(BAND_BITTER), GrindBand::Bitter);
        assert_eq!(GrindBand::from_total(BAND_BREAKING), GrindBand::Breaking);
    }

    // --- Extra: simmering display dwell/linger (item 104 pattern) ---
    #[test]
    fn test_simmering_display_dwell_and_linger() {
        let mut display = SimmeringDisplay::default();
        // Qualifies from tick 0: not visible until 20 ticks held.
        for t in 0..20 {
            display.update(t, 1, 4);
        }
        assert!(!display.visible, "must not show before ON-dwell");
        display.update(20, 1, 4);
        assert!(display.visible, "visible after 20-tick dwell");
        // Condition lifts: linger 30 ticks.
        for t in 21..=50 {
            display.update(t, 0, 4);
        }
        assert!(display.visible, "linger keeps the slot briefly");
        display.update(51, 0, 4);
        assert!(!display.visible, "slot released after linger");
    }
}
