//! Spec 428: The Propaganda Black Market.
//!
//! ## Grounding audit (spec sketch vs. real code)
//!
//! The spec sketch assumed `layer1::edicts::{EdictManager,
//! IdeologyControl}`, `layer1::unrest::UnrestLevel`,
//! `layer1::morale::MoodModifier { current, label }`, and
//! `layer1::economy::PopWealth`. None of those exist under those
//! names/shapes. What the codebase actually has, and what this
//! module builds on (no parallel duplicates):
//!
//! * **Censorship / ideological control (real):** the
//!   [`Policy::CensorBroadcasts`] edict in
//!   [`ColonyPolicies`](crate::layer1::edicts::ColonyPolicies) — the
//!   same policy spec 402 calls the "Censorship Firewall" — plus the
//!   per-colony `CensorshipLevel(0.0..=1.0)` component in
//!   `economy::information_black_market` (a colony-entity component,
//!   not a global level a headless colony exposes). Neither carries
//!   the spec's global 0..100 `control_level`, so this module adds
//!   the one missing resource, [`IdeologicalControl`], and derives
//!   the *effective* control from it: an active CensorBroadcasts
//!   edict alone floors effective control at
//!   [`CENSORSHIP_EDICT_CONTROL_FLOOR`] (85 — over the spawn
//!   threshold), so the real edict drives the market even when the
//!   numeric level was never raised.
//! * **Unrest (real):** the global [`Unrest`] resource
//!   (`layer1::social::unrest`, level 0.0..=1.0 recomputed every tick
//!   from average morale + timed [`UnrestModifier`]s). Divergence is
//!   therefore expressed as a single non-stacking timed modifier —
//!   sales refresh it, they never stack it — so the market cannot
//!   instantly end the game no matter how much data sells.
//! * **Morale (real):** [`Morale`] + [`MoodModifier`]
//!   (`layer1::social::morale`); `Morale.value` is a recomputed
//!   cache, so enlightenment is a refreshed [`MoodModifier`] labelled
//!   [`ENLIGHTENED_LABEL`], never a direct cache write. The sketch's
//!   `mood.current += 30` (0..100 scale) maps to `+0.30` here.
//! * **Pop money (real):** [`Wallet`] (`layer1::economy`,
//!   `credits: f32`) — the sketch's `PopWealth`.
//! * **The other black markets (real, different mechanics):**
//!   `economy::black_market` (spec 348, smuggled luxury/corruption)
//!   and `economy::information_black_market` (colony-entity early
//!   warnings) are left untouched; this is the pop-level ideological
//!   market the spec describes, in the social layer beside spec
//!   402's ideological pressure.
//!
//! ## The loop
//!
//! * While effective control is over [`SPAWN_THRESHOLD`], a
//!   [`TruthDealer`] slips in (cooldown-gated, capped at
//!   [`MAX_DEALERS`]), carrying [`DEALER_STOCK`] copies of
//!   Uncensored Data.
//! * [`auto_purchase_system`] sells at most one copy per tick: the
//!   first pop who can afford [`DATA_COST`] and whose morale is
//!   below [`AUTO_BUY_MAX_MORALE`] buys — credits down, the
//!   "Enlightened by Truth" mood up, global divergence unrest up.
//!   [`trigger_black_market_transaction`] is the same sale as a
//!   direct world function (headless `truthbuy`, tests).
//! * A dealer whose stock runs out leaves. Counterplay: the
//!   headless `truthraid` command (and [`raid_truth_dealers`])
//!   clears every dealer and pushes a small negative unrest
//!   modifier as the divergence network scatters; repealing
//!   censorship stops new dealers spawning at all.
//!
//! All names are original/generic (the spec's own terms); see the
//! IP-guard test at the bottom of this file.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::economy::Wallet;
use crate::layer1::edicts::{ColonyPolicies, Policy};
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::unrest::{Unrest, UnrestModifier};
use crate::shared::time::SimulationTime;

/// Spawn threshold: dealers only appear while effective ideological
/// control is strictly above this level (spec §4: `> 80.0`, on the
/// spec's 0..100 scale).
pub const SPAWN_THRESHOLD: f32 = 80.0;

/// Effective control floor granted by the real CensorBroadcasts
/// edict alone — state censorship at full tilt, over the threshold.
pub const CENSORSHIP_EDICT_CONTROL_FLOOR: f32 = 85.0;

/// Price of one copy of Uncensored Data, in pop credits.
pub const DATA_COST: f32 = 20.0;

/// Morale boost from one purchase (the spec's `+30` on its 0..100
/// mood scale, mapped to the real 0..1 morale scale).
pub const ENLIGHTENMENT_MORALE_BOOST: f32 = 0.30;

/// Duration (ticks) of the "Enlightened by Truth" mood modifier.
pub const ENLIGHTENMENT_DURATION: u32 = 200;

/// Unrest added per sale, as a single non-stacking timed modifier
/// (see module docs — bounded by construction).
pub const DIVERGENCE_UNREST_PER_SALE: f32 = 0.05;

/// Duration (ticks) of the divergence unrest modifier; each sale
/// refreshes it instead of stacking.
pub const DIVERGENCE_UNREST_DURATION: u32 = 500;

/// Maximum Truth-Dealers in the colony at once.
pub const MAX_DEALERS: usize = 2;

/// Copies of Uncensored Data a dealer carries before leaving.
pub const DEALER_STOCK: u32 = 3;

/// Minimum ticks between dealer spawns.
pub const SPAWN_COOLDOWN_TICKS: u64 = 50;

/// Auto-purchase only fires for pops at or below this morale —
/// the stressed pops the spec's REFACTOR phase describes.
pub const AUTO_BUY_MAX_MORALE: f32 = 0.6;

/// Mood modifier label granted by a purchase (spec acceptance text).
pub const ENLIGHTENED_LABEL: &str = "Enlightened by Truth";

/// Unrest modifier label for ideological divergence.
pub const DIVERGENCE_LABEL: &str = "Ideological Divergence";

/// Unrest modifier label applied by a truth raid.
pub const RAID_LABEL: &str = "Truth Raid";

/// Chronicle text written the first time a dealer spawns.
pub const FIRST_DEALER_CHRONICLE_TEXT: &str =
    "A Truth-Dealer has slipped into the colony, selling Uncensored Data beneath the censors' notice.";

/// Chronicle text written the first time data sells.
pub const FIRST_SALE_CHRONICLE_TEXT: &str =
    "A colonist has bought Uncensored Data from a Truth-Dealer. The ideas in it are already spreading.";

/// The colony's ideological control level (0..100) — the spec's
/// `EdictManager { control_level }`, adapted to the one missing
/// resource (see module docs). Default 0: a free colony, no market.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct IdeologicalControl {
    /// Censorship / state-media control, 0..100.
    pub control_level: f32,
}

/// Running ledger for the propaganda black market (headless census,
/// spawn cooldown, one-shot chronicle flags).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PropagandaBlackMarketLedger {
    /// Dealers spawned over the whole run.
    pub total_spawned: u32,
    /// Copies of Uncensored Data sold over the whole run.
    pub total_sales: u32,
    /// Tick of the most recent dealer spawn (cooldown gate).
    pub last_spawn_tick: Option<u64>,
    /// Whether the first-spawn chronicle entry was written.
    pub first_spawn_logged: bool,
    /// Whether the first-sale chronicle entry was written.
    pub first_sale_logged: bool,
}

/// A Truth-Dealer: an underground vendor entity. Not a pop — it has
/// no needs, wallet, or morale of its own; it carries stock and
/// leaves when the stock (or a raid) runs out.
#[derive(Component, Debug, Clone, Copy)]
pub struct TruthDealer {
    /// Copies of Uncensored Data remaining.
    pub stock: u32,
}

/// Effective ideological control: the numeric level, floored by the
/// real CensorBroadcasts edict when that policy is active.
#[must_use]
pub fn effective_control_level(
    control: Option<&IdeologicalControl>,
    policies: Option<&ColonyPolicies>,
) -> f32 {
    let mut level = control.map_or(0.0, |c| c.control_level);
    if policies.is_some_and(|p| p.is_active(Policy::CensorBroadcasts)) {
        level = level.max(CENSORSHIP_EDICT_CONTROL_FLOOR);
    }
    level.clamp(0.0, 100.0)
}

fn current_tick(world: &World) -> u64 {
    world.get_resource::<SimulationTime>().map_or(0, |t| t.tick)
}

fn dealer_count(world: &mut World) -> usize {
    let mut query = world.query::<&TruthDealer>();
    query.iter(world).count()
}

fn log_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    let tick = current_tick(world);
    if let Some(mut chronicle) = world.get_resource_mut::<Chronicle>() {
        chronicle.add_event(tick, text.to_string(), importance);
    }
}

/// Spec §4 spawn function, adapted to real types: spawns one
/// Truth-Dealer when effective control is over the threshold, the
/// dealer cap is not reached, and the spawn cooldown has elapsed.
/// Returns whether a dealer spawned. (The sketch rolled a 10%
/// chance per tick; the cooldown + cap give the same pacing
/// deterministically, which the RED tests require.)
pub fn update_truth_dealer_spawning(world: &mut World) -> bool {
    let control = world.get_resource::<IdeologicalControl>().copied();
    let policies = world.get_resource::<ColonyPolicies>().cloned();
    if effective_control_level(control.as_ref(), policies.as_ref()) <= SPAWN_THRESHOLD {
        return false;
    }
    if dealer_count(world) >= MAX_DEALERS {
        return false;
    }
    let tick = current_tick(world);
    world.init_resource::<PropagandaBlackMarketLedger>();
    {
        let ledger = world.resource::<PropagandaBlackMarketLedger>();
        if let Some(last) = ledger.last_spawn_tick {
            if tick.saturating_sub(last) < SPAWN_COOLDOWN_TICKS {
                return false;
            }
        }
    }
    world.spawn(TruthDealer { stock: DEALER_STOCK });
    let first = {
        let mut ledger = world.resource_mut::<PropagandaBlackMarketLedger>();
        ledger.total_spawned += 1;
        ledger.last_spawn_tick = Some(tick);
        let first = !ledger.first_spawn_logged;
        ledger.first_spawn_logged = true;
        first
    };
    if first {
        log_chronicle(world, FIRST_DEALER_CHRONICLE_TEXT, EventImportance::Standard);
    }
    true
}

/// Spawn system wrapper (exclusive world system — the spawn needs
/// ledger + chronicle bookkeeping in one pass).
pub fn truth_dealer_spawn_system(world: &mut World) {
    update_truth_dealer_spawning(world);
}

/// Spec §4 transaction, adapted to real types: the pop pays
/// [`DATA_COST`] from their real [`Wallet`], gains the refreshed
/// "Enlightened by Truth" [`MoodModifier`] on their real [`Morale`],
/// and global [`Unrest`] gains the single non-stacking divergence
/// modifier. The dealer's stock drops by one; at zero the dealer
/// leaves. Returns whether the sale happened (dealer present with
/// stock, pop exists with wallet + morale and can afford it).
pub fn trigger_black_market_transaction(
    world: &mut World,
    pop_entity: Entity,
    dealer_entity: Entity,
) -> bool {
    let Some(dealer) = world.get::<TruthDealer>(dealer_entity).copied() else {
        return false;
    };
    if dealer.stock == 0 {
        return false;
    }
    let affordable = world
        .get::<Wallet>(pop_entity)
        .is_some_and(|w| w.credits >= DATA_COST);
    if !affordable || world.get::<Morale>(pop_entity).is_none() {
        return false;
    }
    if let Some(mut wallet) = world.get_mut::<Wallet>(pop_entity) {
        wallet.credits -= DATA_COST;
    }
    if let Some(mut morale) = world.get_mut::<Morale>(pop_entity) {
        morale
            .modifiers
            .retain(|m| m.label != ENLIGHTENED_LABEL);
        morale.add_modifier(MoodModifier {
            label: ENLIGHTENED_LABEL.to_string(),
            value: ENLIGHTENMENT_MORALE_BOOST,
            duration: ENLIGHTENMENT_DURATION,
        });
    }
    world.init_resource::<Unrest>();
    if let Some(mut unrest) = world.get_resource_mut::<Unrest>() {
        unrest
            .modifiers
            .retain(|m| m.label != DIVERGENCE_LABEL);
        unrest.modifiers.push(UnrestModifier {
            value: DIVERGENCE_UNREST_PER_SALE,
            duration: DIVERGENCE_UNREST_DURATION,
            label: DIVERGENCE_LABEL.to_string(),
        });
    }
    let remaining = dealer.stock - 1;
    if remaining == 0 {
        world.despawn(dealer_entity);
    } else if let Some(mut dealer) = world.get_mut::<TruthDealer>(dealer_entity) {
        dealer.stock = remaining;
    }
    world.init_resource::<PropagandaBlackMarketLedger>();
    let first_sale = {
        let mut ledger = world.resource_mut::<PropagandaBlackMarketLedger>();
        ledger.total_sales += 1;
        let first = !ledger.first_sale_logged;
        ledger.first_sale_logged = true;
        first
    };
    if first_sale {
        log_chronicle(world, FIRST_SALE_CHRONICLE_TEXT, EventImportance::Standard);
    }
    true
}

/// Auto-purchase system (spec REFACTOR): at most one sale per tick —
/// the first stressed pop (morale at or below
/// [`AUTO_BUY_MAX_MORALE`], not already enlightened) who can afford
/// the data buys from the first dealer with stock. No pathfinding:
/// dealers are colony-wide underground contacts, like the spec 348
/// smuggler's docked trade, not a tile errand.
pub fn auto_purchase_system(world: &mut World) {
    let dealer = {
        let mut query = world.query::<(Entity, &TruthDealer)>();
        query
            .iter(world)
            .find(|(_, d)| d.stock > 0)
            .map(|(e, _)| e)
    };
    let Some(dealer) = dealer else {
        return;
    };
    let buyer = {
        let mut query = world.query::<(Entity, &Wallet, &Morale, &Pop)>();
        query
            .iter(world)
            .find(|(_, wallet, morale, _)| {
                wallet.credits >= DATA_COST
                    && morale.value <= AUTO_BUY_MAX_MORALE
                    && !morale
                        .modifiers
                        .iter()
                        .any(|m| m.label == ENLIGHTENED_LABEL)
            })
            .map(|(e, _, _, _)| e)
    };
    if let Some(buyer) = buyer {
        trigger_black_market_transaction(world, buyer, dealer);
    }
}

/// Counterplay (spec REFACTOR): a security raid clears every
/// Truth-Dealer from the colony and pushes a small negative unrest
/// modifier as the divergence network scatters. Returns how many
/// dealers were cleared. New dealers can still spawn later while
/// censorship stays high — the raid buys time, not a cure.
pub fn raid_truth_dealers(world: &mut World) -> usize {
    let dealers: Vec<Entity> = {
        let mut query = world.query::<(Entity, &TruthDealer)>();
        query.iter(world).map(|(e, _)| e).collect()
    };
    let cleared = dealers.len();
    for dealer in dealers {
        world.despawn(dealer);
    }
    if cleared > 0 {
        world.init_resource::<Unrest>();
        if let Some(mut unrest) = world.get_resource_mut::<Unrest>() {
            unrest.modifiers.retain(|m| m.label != RAID_LABEL);
            unrest.modifiers.push(UnrestModifier {
                value: -DIVERGENCE_UNREST_PER_SALE,
                duration: DIVERGENCE_UNREST_DURATION / 2,
                label: RAID_LABEL.to_string(),
            });
        }
        log_chronicle(
            world,
            "Security forces raided the propaganda black market, scattering the Truth-Dealers — for now.",
            EventImportance::Standard,
        );
    }
    cleared
}

/// Census for the headless console / STATS.
#[derive(Debug, Default, Clone, Copy)]
pub struct TruthMarketCensus {
    /// Effective ideological control (0..100).
    pub control: f32,
    /// Dealers currently in the colony.
    pub dealers: usize,
    /// Uncensored Data stock across all dealers.
    pub stock: u32,
    /// Total sales over the run.
    pub sales: u32,
}

/// Counts the live market in a raw world (headless).
#[must_use]
pub fn truth_market_census(world: &mut World) -> TruthMarketCensus {
    let control_res = world.get_resource::<IdeologicalControl>().copied();
    let policies = world.get_resource::<ColonyPolicies>().cloned();
    let mut census = TruthMarketCensus {
        control: effective_control_level(control_res.as_ref(), policies.as_ref()),
        ..Default::default()
    };
    let mut query = world.query::<&TruthDealer>();
    for dealer in query.iter(world) {
        census.dealers += 1;
        census.stock += dealer.stock;
    }
    if let Some(ledger) = world.get_resource::<PropagandaBlackMarketLedger>() {
        census.sales = ledger.total_sales;
    }
    census
}

/// One-token STATS label, e.g. `truth=ctrl85/dealers1/sales2`
/// (full panel: the headless `truthdealers` command).
#[must_use]
pub fn truth_stats_label(world: &mut World) -> String {
    let census = truth_market_census(world);
    if census.control <= SPAWN_THRESHOLD && census.dealers == 0 && census.sales == 0 {
        return "none".to_string();
    }
    format!(
        "ctrl{:.0}/dealers{}/sales{}",
        census.control, census.dealers, census.sales
    )
}

/// Every public string this module can emit (IP-guard test).
#[must_use]
pub fn propaganda_black_market_public_strings() -> Vec<String> {
    vec![
        ENLIGHTENED_LABEL.to_string(),
        DIVERGENCE_LABEL.to_string(),
        RAID_LABEL.to_string(),
        FIRST_DEALER_CHRONICLE_TEXT.to_string(),
        FIRST_SALE_CHRONICLE_TEXT.to_string(),
        "Truth-Dealer".to_string(),
        "Uncensored Data".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn world_with_control(level: f32) -> World {
        let mut world = World::new();
        world.insert_resource(IdeologicalControl {
            control_level: level,
        });
        world.init_resource::<Unrest>();
        world
    }

    fn spawn_buyer(world: &mut World, credits: f32, morale_value: f32) -> Entity {
        world
            .spawn((
                Pop,
                Wallet { credits },
                Morale {
                    value: morale_value,
                    modifiers: Vec::new(),
                },
            ))
            .id()
    }

    // Spec RED test 1, adapted: EdictManager{control_level: 100} ->
    // IdeologicalControl{100}; the sketch's bare TruthDealer query
    // is real as-is.
    #[test]
    fn test_truth_dealer_spawns_under_high_censorship() {
        let mut world = world_with_control(100.0);
        assert!(update_truth_dealer_spawning(&mut world));
        let mut query = world.query::<&TruthDealer>();
        assert_eq!(query.iter(&world).count(), 1);
    }

    // Spec RED test 2, adapted as-is in shape.
    #[test]
    fn test_no_dealers_under_low_censorship() {
        let mut world = world_with_control(10.0);
        assert!(!update_truth_dealer_spawning(&mut world));
        let mut query = world.query::<&TruthDealer>();
        assert_eq!(query.iter(&world).count(), 0);
    }

    #[test]
    fn test_censorship_edict_alone_spawns_dealers() {
        // The real edict seam: no numeric level, but CensorBroadcasts
        // active floors effective control over the threshold.
        let mut world = World::new();
        world.init_resource::<Unrest>();
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::CensorBroadcasts);
        world.insert_resource(policies);
        assert!(update_truth_dealer_spawning(&mut world));
    }

    #[test]
    fn test_spawn_cooldown_and_cap() {
        let mut world = world_with_control(100.0);
        assert!(update_truth_dealer_spawning(&mut world));
        // Same tick: cooldown blocks a second spawn.
        assert!(!update_truth_dealer_spawning(&mut world));
        // After the cooldown, a second dealer may arrive; the cap
        // then blocks a third even after another cooldown.
        world.insert_resource(SimulationTime {
            tick: SPAWN_COOLDOWN_TICKS,
            ..Default::default()
        });
        assert!(update_truth_dealer_spawning(&mut world));
        world.insert_resource(SimulationTime {
            tick: SPAWN_COOLDOWN_TICKS * 2,
            ..Default::default()
        });
        assert!(!update_truth_dealer_spawning(&mut world));
        assert_eq!(dealer_count(&mut world), MAX_DEALERS);
    }

    // Spec RED test 3, adapted: PopWealth -> Wallet, the sketch's
    // MoodModifier{current, label} -> Morale + its MoodModifier
    // list, UnrestLevel{value} -> Unrest timed modifiers (the level
    // itself is recomputed from morale + modifiers every tick).
    #[test]
    fn test_pop_buys_uncensored_data() {
        let mut world = world_with_control(100.0);
        let pop = spawn_buyer(&mut world, 50.0, 0.5);
        let dealer = world.spawn(TruthDealer { stock: DEALER_STOCK }).id();

        assert!(trigger_black_market_transaction(&mut world, pop, dealer));

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert!(wallet.credits < 50.0, "Pop must spend credits for data");
        assert!((wallet.credits - 30.0).abs() < f32::EPSILON);

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale
            .modifiers
            .iter()
            .find(|m| m.label == ENLIGHTENED_LABEL)
            .expect("purchase must add the enlightenment modifier");
        assert!(modifier.value > 0.0, "Mood must increase from data");
        assert_eq!(modifier.label, "Enlightened by Truth");

        let unrest = world.resource::<Unrest>();
        assert!(
            unrest
                .modifiers
                .iter()
                .any(|m| m.label == DIVERGENCE_LABEL && m.value > 0.0),
            "Global unrest should increase as pops diverge"
        );
        assert_eq!(
            world
                .resource::<PropagandaBlackMarketLedger>()
                .total_sales,
            1
        );
    }

    #[test]
    fn test_pop_cannot_afford_data() {
        let mut world = world_with_control(100.0);
        let pop = spawn_buyer(&mut world, 10.0, 0.5);
        let dealer = world.spawn(TruthDealer { stock: DEALER_STOCK }).id();
        assert!(!trigger_black_market_transaction(&mut world, pop, dealer));
        assert!((world.get::<Wallet>(pop).unwrap().credits - 10.0).abs() < f32::EPSILON);
        assert!(world.get::<Morale>(pop).unwrap().modifiers.is_empty());
    }

    #[test]
    fn test_dealer_leaves_when_stock_runs_out() {
        let mut world = world_with_control(100.0);
        let pop = spawn_buyer(&mut world, 500.0, 0.5);
        let dealer = world.spawn(TruthDealer { stock: 1 }).id();
        assert!(trigger_black_market_transaction(&mut world, pop, dealer));
        assert!(world.get::<TruthDealer>(dealer).is_none());
    }

    #[test]
    fn test_unrest_never_stacks_from_sales() {
        // Bounded by construction: 10 sales still leave exactly one
        // divergence modifier at the per-sale value.
        let mut world = world_with_control(100.0);
        for _ in 0..10 {
            let pop = spawn_buyer(&mut world, 500.0, 0.5);
            let dealer = world.spawn(TruthDealer { stock: 1 }).id();
            assert!(trigger_black_market_transaction(&mut world, pop, dealer));
        }
        let unrest = world.resource::<Unrest>();
        let divergence: Vec<&UnrestModifier> = unrest
            .modifiers
            .iter()
            .filter(|m| m.label == DIVERGENCE_LABEL)
            .collect();
        assert_eq!(divergence.len(), 1);
        assert!((divergence[0].value - DIVERGENCE_UNREST_PER_SALE).abs() < f32::EPSILON);
    }

    #[test]
    fn test_auto_purchase_sells_to_stressed_pop() {
        let mut world = world_with_control(100.0);
        let pop = spawn_buyer(&mut world, 50.0, 0.4);
        world.spawn(TruthDealer { stock: DEALER_STOCK });
        world.run_system_once(auto_purchase_system).unwrap();
        assert!((world.get::<Wallet>(pop).unwrap().credits - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_auto_purchase_skips_content_pop() {
        let mut world = world_with_control(100.0);
        let pop = spawn_buyer(&mut world, 50.0, 0.95);
        world.spawn(TruthDealer { stock: DEALER_STOCK });
        world.run_system_once(auto_purchase_system).unwrap();
        assert!((world.get::<Wallet>(pop).unwrap().credits - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_raid_clears_dealers() {
        let mut world = world_with_control(100.0);
        world.spawn(TruthDealer { stock: DEALER_STOCK });
        world.spawn(TruthDealer { stock: 1 });
        assert_eq!(raid_truth_dealers(&mut world), 2);
        assert_eq!(dealer_count(&mut world), 0);
        let unrest = world.resource::<Unrest>();
        assert!(unrest.modifiers.iter().any(|m| m.label == RAID_LABEL));
    }

    #[test]
    fn test_ip_guard_no_named_ip_in_public_strings() {
        let banned = [
            "orwell",
            "big brother",
            "ministry of truth",
            "psychohistory",
            "foundation",
            "dune",
            "arrakis",
            "huxley",
            "brave new world",
        ];
        for text in propaganda_black_market_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string {text:?} contains banned IP term {term:?}"
                );
            }
        }
    }
}
