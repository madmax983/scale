//! Spec 382: Merchant Landings — merchant ships on the Trade Pad.
//!
//! Cross-layer (Layer 1 -> Layer 2): a Layer 1 [`BuildingType::TradePad`]
//! building (carrying the [`TradePad`] marker) opens the colony to Layer 2
//! traffic. While a pad stands active and no ship is down, a merchant ship
//! lands with a *randomized* barter pair: it buys one resource from the
//! colony and sells another. The colony buys `buy_amount` of the ship's
//! `selling` resource, paying `buy_amount * exchange_rate` of the ship's
//! `buying` resource — exactly the spec's RED-test transaction (pay 20
//! Metal, receive 10 Food at rate 2.0). Prices are not fixed: the exchange
//! rate random-walks on a cadence while a ship is down, and every new ship
//! rolls a fresh pair and a fresh rate, so no two landings price alike.
//!
//! ## Relationship to existing trade (no parallel duplicate)
//! * [`crate::layer1::economy::trade`] is the *ground* merchant: a visitor
//!   in a `MerchantState` resource offering `TradeDeal`s at a Trade Depot.
//!   It stays untouched. This module is the *orbital* leg the spec asks
//!   for: ships are real entities with inventory, a stay window, and a
//!   departure, gated on a landing pad instead of a depot.
//! * All money movement goes through the real
//!   [`ColonyResources`] API (`try_consume` / `add_resource` /
//!   `get_amount`) — no direct field arithmetic, so capacity clamping and
//!   affordability checks are the colony's own.
//!
//! ## Spec adaptations
//! * The sketch's `crate::layer1::resources` path is really
//!   `crate::layer1::economy::resources` (re-exported as
//!   `crate::layer1::resources`); the types are the real ones.
//! * The sketch hardcodes Metal/Food at 2.0 ("Example hardcode"); per the
//!   task brief the pair and rate are randomized per landing from
//!   [`TRADEABLE_RESOURCES`] (which deliberately excludes the
//!   deliberately-untradeable Organs / DarkMote / AncientTech pattern).
//! * The sketch's `App`/`Update` test harness is adapted to the codebase's
//!   World + Schedule pattern, like specs 377-381.
//! * `TradeEvent` conflicts with no existing type in this module's scope;
//!   the ground trade's event is `TradeDeal`, a different shape.
//! * Arrival randomness is a small deterministic xorshift in
//!   [`MerchantLandingState`] (seeded, overridable in tests) instead of
//!   `thread_rng`, so landings are reproducible in tests and saves.
//! * All names are original and generic (see the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::shared::time::SimulationTime;

/// Ticks a landed ship stays before departing.
pub const MERCHANT_STAY_TICKS: u64 = 500;
/// Ticks after a departure before the next ship may land.
pub const MERCHANT_COOLDOWN_TICKS: u64 = 200;
/// Ticks between exchange-rate fluctuations while a ship is down.
pub const PRICE_INTERVAL_TICKS: u64 = 50;
/// Lowest an exchange rate may drift.
pub const MIN_EXCHANGE_RATE: f32 = 0.5;
/// Highest an exchange rate may drift.
pub const MAX_EXCHANGE_RATE: f32 = 4.0;
/// Default ship inventory (units of its `selling` resource).
pub const DEFAULT_INVENTORY: f32 = 75.0;

/// Resources a merchant may buy or sell. Excludes the deliberately
/// untradeable ledger resources (Organs, DarkMote, AncientTech,
/// HyperValuable, MemoryCore, waste streams).
pub const TRADEABLE_RESOURCES: [ResourceType; 11] = [
    ResourceType::Food,
    ResourceType::Wood,
    ResourceType::Stone,
    ResourceType::Ore,
    ResourceType::Metal,
    ResourceType::Planks,
    ResourceType::Blocks,
    ResourceType::Tools,
    ResourceType::Fuel,
    ResourceType::Rations,
    ResourceType::Scrap,
];

/// Original, generic ship names, chosen per landing.
pub const MERCHANT_SHIP_NAMES: [&str; 6] = [
    "Free Trader Albatross",
    "Independent Hauler Brine",
    "Cooperative Freighter Ledger",
    "Free Trader Cormorant",
    "Slow Barge Ninth Ledger",
    "Independent Hauler Silt",
];

/// Marker for a landing pad. Spawned directly in tests/headless and
/// attached by `spawn_building` to [`BuildingType::TradePad`] buildings.
#[derive(Component, Debug, Clone, Copy)]
pub struct TradePad {
    /// Whether the pad is open for landings.
    pub active: bool,
}

impl Default for TradePad {
    fn default() -> Self {
        Self { active: true }
    }
}

/// A landed merchant ship: one barter offer with finite stock.
///
/// Field semantics follow the spec sketch: the colony pays `buying`
/// (the resource the merchant is buying *from* the colony) to receive
/// `selling` (what the merchant sells), at `exchange_rate` units of
/// `buying` per unit of `selling`.
#[derive(Component, Debug, Clone)]
pub struct MerchantShip {
    /// Ship name (original, generic).
    pub name: String,
    /// Resource the colony pays (the merchant buys this).
    pub buying: ResourceType,
    /// Resource the colony receives (the merchant sells this).
    pub selling: ResourceType,
    /// Units of `buying` per unit of `selling`.
    pub exchange_rate: f32,
    /// Remaining stock of `selling`.
    pub inventory: f32,
    /// Tick the ship landed.
    pub arrival_tick: u64,
    /// Tick the ship departs.
    pub departure_tick: u64,
}

/// The colony asks to buy `buy_amount` of a ship's `selling` resource.
#[derive(Event, Debug, Clone, Copy)]
pub struct TradeEvent {
    /// The merchant ship entity.
    pub merchant: Entity,
    /// Units of the ship's `selling` resource to buy.
    pub buy_amount: f32,
}

/// Landing ledger + deterministic RNG (first-tick resource).
#[derive(Resource, Debug, Clone)]
pub struct MerchantLandingState {
    /// Ships landed so far.
    pub visits: u64,
    /// Trades completed.
    pub trades_completed: u64,
    /// Trades refused (unaffordable / out of stock / invalid).
    pub trades_refused: u64,
    /// Total units of payment resources handed over.
    pub total_paid: f32,
    /// Total units of goods received.
    pub total_received: f32,
    /// Earliest tick the next ship may land.
    pub next_arrival_tick: u64,
    /// Last tick prices fluctuated.
    pub last_price_tick: u64,
    /// Deterministic RNG state (xorshift64*).
    pub rng_state: u64,
}

impl Default for MerchantLandingState {
    fn default() -> Self {
        Self {
            visits: 0,
            trades_completed: 0,
            trades_refused: 0,
            total_paid: 0.0,
            total_received: 0.0,
            next_arrival_tick: 0,
            last_price_tick: 0,
            rng_state: 0x3820_5EED_CAFE_0001,
        }
    }
}

impl MerchantLandingState {
    /// Next deterministic value in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Top 24 bits -> [0, 1).
        #[allow(clippy::cast_precision_loss)]
        let f = (v >> 40) as f32 / (1u64 << 24) as f32;
        f.min(0.999_999)
    }

    /// Next deterministic index in `0..n` (0 when `n == 0`).
    pub fn next_index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = (self.next_f32() * n as f32) as usize;
        i.min(n - 1)
    }
}

/// Human label for a resource (chronicles, panels, STATS).
#[must_use]
pub const fn resource_label(r: ResourceType) -> &'static str {
    match r {
        ResourceType::Food => "Food",
        ResourceType::Wood => "Wood",
        ResourceType::Stone => "Stone",
        ResourceType::Ore => "Ore",
        ResourceType::Metal => "Metal",
        ResourceType::Planks => "Planks",
        ResourceType::Blocks => "Blocks",
        ResourceType::Waste => "Waste",
        ResourceType::BiologicalWaste => "Biological Waste",
        ResourceType::NutrientPaste => "Nutrient Paste",
        ResourceType::Rations => "Rations",
        ResourceType::Fuel => "Fuel",
        ResourceType::Alcohol => "Alcohol",
        ResourceType::Scrap => "Scrap",
        ResourceType::Tools => "Tools",
        ResourceType::BuildingPermit => "Building Permits",
        ResourceType::MemoryCore => "Memory Cores",
        ResourceType::VoidAle => "Void Ale",
        ResourceType::HyperValuable => "Hyper Valuables",
        ResourceType::Organs => "Organs",
        ResourceType::DarkMote => "Dark Motes",
        ResourceType::AncientTech => "Ancient Tech",
    }
}

fn current_tick(world: &World) -> u64 {
    world.get_resource::<SimulationTime>().map_or(0, |t| t.tick)
}

fn chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

/// Number of landing pads: `TradePad` components plus any
/// `BuildingType::TradePad` building that lacks the marker.
#[must_use]
pub fn trade_pad_count(world: &mut World) -> usize {
    let mut with_marker: Vec<Entity> = Vec::new();
    {
        let mut q = world.query::<(Entity, &TradePad)>();
        for (e, pad) in q.iter(world) {
            if pad.active {
                with_marker.push(e);
            }
        }
    }
    let mut count = with_marker.len();
    let mut q = world.query::<(Entity, &Building)>();
    for (e, b) in q.iter(world) {
        if b.building_type == BuildingType::TradePad && !with_marker.contains(&e) {
            count += 1;
        }
    }
    count
}

/// Number of merchant ships currently down.
#[must_use]
pub fn merchant_ship_count(world: &mut World) -> usize {
    let mut q = world.query::<&MerchantShip>();
    q.iter(world).count()
}

/// Census for STATS / panels.
#[derive(Debug, Clone, Copy, Default)]
pub struct MerchantCensus {
    /// Active trade pads.
    pub pads: usize,
    /// Ships currently down.
    pub ships: usize,
    /// Trades completed (ledger).
    pub trades: u64,
    /// Total visits (ledger).
    pub visits: u64,
}

/// Census of pads, ships, and the landing ledger.
#[must_use]
pub fn merchant_census(world: &mut World) -> MerchantCensus {
    let pads = trade_pad_count(world);
    let ships = merchant_ship_count(world);
    let (trades, visits) = world
        .get_resource::<MerchantLandingState>()
        .map_or((0, 0), |s| (s.trades_completed, s.visits));
    MerchantCensus {
        pads,
        ships,
        trades,
        visits,
    }
}

/// Compact STATS label: `none`, or `Metal>Food@2.00` (pay > receive @ rate)
/// for the first ship down. No spaces — STATS is space-separated.
#[must_use]
pub fn merchant_stats_label(world: &mut World) -> String {
    let mut q = world.query::<&MerchantShip>();
    if let Some(ship) = q.iter(world).next() {
        format!(
            "{}>{}@{:.2}",
            resource_label(ship.buying),
            resource_label(ship.selling),
            ship.exchange_rate
        )
    } else {
        "none".to_string()
    }
}

/// Roll a fresh randomized offer: distinct buy/sell pair, rate in
/// `[0.75, 3.0]`, inventory in `[40, 100]`.
pub fn roll_offer(
    state: &mut MerchantLandingState,
) -> (ResourceType, ResourceType, f32, f32, String) {
    let selling_idx = state.next_index(TRADEABLE_RESOURCES.len());
    let mut buying_idx = state.next_index(TRADEABLE_RESOURCES.len() - 1);
    if buying_idx >= selling_idx {
        buying_idx += 1;
    }
    let selling = TRADEABLE_RESOURCES[selling_idx];
    let buying = TRADEABLE_RESOURCES[buying_idx];
    let rate = ((0.75 + state.next_f32() * 2.25) * 4.0).round() / 4.0; // quarter steps
    let rate = rate.clamp(MIN_EXCHANGE_RATE, MAX_EXCHANGE_RATE);
    let inventory = (40.0 + state.next_f32() * 60.0).round();
    let name = MERCHANT_SHIP_NAMES[state.next_index(MERCHANT_SHIP_NAMES.len())].to_string();
    (buying, selling, rate, inventory, name)
}

/// System: land a merchant ship when a pad is active, no ship is down,
/// and the arrival cooldown has passed (spec RED test 1).
pub fn spawn_merchant_ship_system(world: &mut World) {
    world.init_resource::<MerchantLandingState>();
    if merchant_ship_count(world) > 0 || trade_pad_count(world) == 0 {
        return;
    }
    let tick = current_tick(world);
    {
        let state = world.resource::<MerchantLandingState>();
        if tick < state.next_arrival_tick {
            return;
        }
    }
    let (buying, selling, rate, inventory, name) = {
        let mut state = world.resource_mut::<MerchantLandingState>();
        let offer = roll_offer(&mut state);
        state.visits += 1;
        offer
    };
    world.spawn(MerchantShip {
        name: name.clone(),
        buying,
        selling,
        exchange_rate: rate,
        inventory,
        arrival_tick: tick,
        departure_tick: tick + MERCHANT_STAY_TICKS,
    });
    chronicle(
        world,
        format!(
            "MERCHANT LANDING: the {name} has set down on the trade pad, buying {} and selling {} at {rate:.2} to 1.",
            resource_label(buying),
            resource_label(selling)
        ),
        EventImportance::Major,
    );
}

/// Execute one barter against the real colony ledger. Returns `true`
/// when the trade happened. Refusals change nothing.
pub fn execute_merchant_trade(world: &mut World, merchant: Entity, buy_amount: f32) -> bool {
    world.init_resource::<MerchantLandingState>();
    if !buy_amount.is_finite() || buy_amount <= 0.0 {
        world.resource_mut::<MerchantLandingState>().trades_refused += 1;
        return false;
    }
    let Some((buying, selling, rate, inventory, name)) =
        world.get::<MerchantShip>(merchant).map(|s| {
            (
                s.buying,
                s.selling,
                s.exchange_rate,
                s.inventory,
                s.name.clone(),
            )
        })
    else {
        world.resource_mut::<MerchantLandingState>().trades_refused += 1;
        return false;
    };
    if inventory < buy_amount {
        world.resource_mut::<MerchantLandingState>().trades_refused += 1;
        return false;
    }
    let cost = buy_amount * rate;
    let paid = {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.try_consume(buying, cost)
    };
    if !paid {
        world.resource_mut::<MerchantLandingState>().trades_refused += 1;
        return false;
    }
    // Payment taken — deliver the goods (capacity-clamped by the real API).
    let before = world.resource::<ColonyResources>().get_amount(selling);
    world
        .resource_mut::<ColonyResources>()
        .add_resource(&selling, buy_amount);
    let delivered = world.resource::<ColonyResources>().get_amount(selling) - before;
    if let Some(mut ship) = world.get_mut::<MerchantShip>(merchant) {
        ship.inventory -= buy_amount;
    }
    {
        let mut state = world.resource_mut::<MerchantLandingState>();
        state.trades_completed += 1;
        state.total_paid += cost;
        state.total_received += delivered;
    }
    chronicle(
        world,
        format!(
            "TRADE: the colony paid {cost:.1} {} to the {name} for {delivered:.1} {}.",
            resource_label(buying),
            resource_label(selling)
        ),
        EventImportance::Standard,
    );
    true
}

/// System: drain [`TradeEvent`]s into real trades (spec RED test 2).
pub fn process_trade_system(world: &mut World) {
    world.init_resource::<Events<TradeEvent>>();
    let events: Vec<TradeEvent> = {
        let mut ev = world.resource_mut::<Events<TradeEvent>>();
        ev.drain().collect()
    };
    for event in events {
        execute_merchant_trade(world, event.merchant, event.buy_amount);
    }
}

/// System: while a ship is down, its exchange rate random-walks every
/// [`PRICE_INTERVAL_TICKS`] ticks, clamped to
/// [`MIN_EXCHANGE_RATE`]..=[`MAX_EXCHANGE_RATE`].
pub fn fluctuate_merchant_prices_system(world: &mut World) {
    world.init_resource::<MerchantLandingState>();
    let tick = current_tick(world);
    if tick == 0 || !tick.is_multiple_of(PRICE_INTERVAL_TICKS) {
        return;
    }
    {
        let state = world.resource::<MerchantLandingState>();
        if state.last_price_tick == tick {
            return;
        }
    }
    let mut ships: Vec<Entity> = Vec::new();
    {
        let mut q = world.query::<(Entity, &MerchantShip)>();
        for (e, _) in q.iter(world) {
            ships.push(e);
        }
    }
    if ships.is_empty() {
        return;
    }
    for entity in ships {
        let factor = {
            let mut state = world.resource_mut::<MerchantLandingState>();
            0.9 + state.next_f32() * 0.2
        };
        if let Some(mut ship) = world.get_mut::<MerchantShip>(entity) {
            ship.exchange_rate =
                (ship.exchange_rate * factor).clamp(MIN_EXCHANGE_RATE, MAX_EXCHANGE_RATE);
        }
    }
    world.resource_mut::<MerchantLandingState>().last_price_tick = tick;
}

/// System: ships depart at their departure tick; the next landing waits
/// out [`MERCHANT_COOLDOWN_TICKS`].
pub fn merchant_departure_system(world: &mut World) {
    world.init_resource::<MerchantLandingState>();
    let tick = current_tick(world);
    let mut departing: Vec<(Entity, String)> = Vec::new();
    {
        let mut q = world.query::<(Entity, &MerchantShip)>();
        for (e, ship) in q.iter(world) {
            if tick >= ship.departure_tick {
                departing.push((e, ship.name.clone()));
            }
        }
    }
    for (entity, name) in &departing {
        world.despawn(*entity);
        chronicle(
            world,
            format!("The {name} has lifted off from the trade pad, its hold lighter and its ledger settled."),
            EventImportance::Standard,
        );
    }
    if !departing.is_empty() {
        world
            .resource_mut::<MerchantLandingState>()
            .next_arrival_tick = tick + MERCHANT_COOLDOWN_TICKS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::chronicle::AddChronicleEvent as ChronicleEvent;

    fn test_world() -> World {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        world.insert_resource(SimulationTime::default());
        world.init_resource::<MerchantLandingState>();
        world.init_resource::<Events<TradeEvent>>();
        world.init_resource::<Events<ChronicleEvent>>();
        world
    }

    fn run(world: &mut World, systems: fn(&mut Schedule)) {
        let mut schedule = Schedule::default();
        systems(&mut schedule);
        schedule.run(world);
    }

    #[test]
    fn test_merchant_ship_arrival() {
        // Spec RED test 1, adapted to World+Schedule.
        let mut world = test_world();
        world.spawn(TradePad { active: true });
        run(&mut world, |s| {
            s.add_systems(spawn_merchant_ship_system);
        });
        let mut q = world.query::<&MerchantShip>();
        assert_eq!(q.iter(&world).count(), 1);
    }

    #[test]
    fn test_no_arrival_without_pad() {
        let mut world = test_world();
        run(&mut world, |s| {
            s.add_systems(spawn_merchant_ship_system);
        });
        assert_eq!(merchant_ship_count(&mut world), 0);
    }

    #[test]
    fn test_no_arrival_with_inactive_pad() {
        let mut world = test_world();
        world.spawn(TradePad { active: false });
        run(&mut world, |s| {
            s.add_systems(spawn_merchant_ship_system);
        });
        assert_eq!(merchant_ship_count(&mut world), 0);
    }

    #[test]
    fn test_trade_transaction() {
        // Spec RED test 2, verbatim amounts: pay 20 Metal for 10 Food @2.0.
        let mut world = test_world();
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.add_metal(100.0);
            res.metal = 100.0;
            res.food = 0.0;
        }
        let merchant_id = world
            .spawn(MerchantShip {
                name: "Test Trader".to_string(),
                buying: ResourceType::Metal,
                selling: ResourceType::Food,
                exchange_rate: 2.0,
                inventory: 50.0,
                arrival_tick: 0,
                departure_tick: 500,
            })
            .id();
        world.send_event(TradeEvent {
            merchant: merchant_id,
            buy_amount: 10.0,
        });
        run(&mut world, |s| {
            s.add_systems(process_trade_system);
        });
        let res = world.resource::<ColonyResources>();
        assert_eq!(res.metal, 80.0);
        assert_eq!(res.food, 10.0);
        let ship = world.get::<MerchantShip>(merchant_id).unwrap();
        assert_eq!(ship.inventory, 40.0);
        assert_eq!(world.resource::<MerchantLandingState>().trades_completed, 1);
    }

    #[test]
    fn test_trade_refused_when_unaffordable() {
        let mut world = test_world();
        world.resource_mut::<ColonyResources>().metal = 5.0;
        let merchant_id = world
            .spawn(MerchantShip {
                name: "Test Trader".to_string(),
                buying: ResourceType::Metal,
                selling: ResourceType::Food,
                exchange_rate: 2.0,
                inventory: 50.0,
                arrival_tick: 0,
                departure_tick: 500,
            })
            .id();
        assert!(!execute_merchant_trade(&mut world, merchant_id, 10.0));
        let res = world.resource::<ColonyResources>();
        assert_eq!(res.metal, 5.0);
        assert_eq!(
            world.get::<MerchantShip>(merchant_id).unwrap().inventory,
            50.0
        );
        assert_eq!(world.resource::<MerchantLandingState>().trades_refused, 1);
    }

    #[test]
    fn test_trade_refused_when_out_of_stock() {
        let mut world = test_world();
        world.resource_mut::<ColonyResources>().metal = 100.0;
        let merchant_id = world
            .spawn(MerchantShip {
                name: "Test Trader".to_string(),
                buying: ResourceType::Metal,
                selling: ResourceType::Food,
                exchange_rate: 1.0,
                inventory: 3.0,
                arrival_tick: 0,
                departure_tick: 500,
            })
            .id();
        assert!(!execute_merchant_trade(&mut world, merchant_id, 10.0));
        assert_eq!(world.resource::<ColonyResources>().metal, 100.0);
    }

    #[test]
    fn test_randomized_offers_are_distinct_pairs_in_range() {
        let mut state = MerchantLandingState::default();
        let mut pairs = std::collections::HashSet::new();
        for _ in 0..40 {
            let (buying, selling, rate, inventory, _name) = roll_offer(&mut state);
            assert_ne!(buying, selling, "pair must be distinct");
            assert!((MIN_EXCHANGE_RATE..=MAX_EXCHANGE_RATE).contains(&rate));
            assert!((40.0..=100.0).contains(&inventory));
            assert!(TRADEABLE_RESOURCES.contains(&buying));
            assert!(TRADEABLE_RESOURCES.contains(&selling));
            pairs.insert((buying, selling));
        }
        assert!(pairs.len() > 3, "offers should vary, got {pairs:?}");
    }

    #[test]
    fn test_prices_fluctuate_and_stay_clamped() {
        let mut world = test_world();
        let id = world
            .spawn(MerchantShip {
                name: "Test Trader".to_string(),
                buying: ResourceType::Metal,
                selling: ResourceType::Food,
                exchange_rate: 3.9,
                inventory: 50.0,
                arrival_tick: 0,
                departure_tick: 10_000,
            })
            .id();
        let mut changed = false;
        for step in 1..=20u64 {
            world.resource_mut::<SimulationTime>().tick = step * PRICE_INTERVAL_TICKS;
            run(&mut world, |s| {
                s.add_systems(fluctuate_merchant_prices_system);
            });
            let rate = world.get::<MerchantShip>(id).unwrap().exchange_rate;
            assert!((MIN_EXCHANGE_RATE..=MAX_EXCHANGE_RATE).contains(&rate));
            if (rate - 3.9).abs() > f32::EPSILON {
                changed = true;
            }
        }
        assert!(changed, "rate should random-walk over 20 intervals");
    }

    #[test]
    fn test_departure_and_cooldown() {
        let mut world = test_world();
        world.spawn(TradePad { active: true });
        world.spawn(MerchantShip {
            name: "Test Trader".to_string(),
            buying: ResourceType::Metal,
            selling: ResourceType::Food,
            exchange_rate: 2.0,
            inventory: 50.0,
            arrival_tick: 0,
            departure_tick: 10,
        });
        world.resource_mut::<SimulationTime>().tick = 10;
        run(&mut world, |s| {
            s.add_systems((merchant_departure_system, spawn_merchant_ship_system).chain());
        });
        assert_eq!(
            merchant_ship_count(&mut world),
            0,
            "departed, cooldown blocks respawn"
        );
        world.resource_mut::<SimulationTime>().tick = 10 + MERCHANT_COOLDOWN_TICKS;
        run(&mut world, |s| {
            s.add_systems(spawn_merchant_ship_system);
        });
        assert_eq!(
            merchant_ship_count(&mut world),
            1,
            "new ship after cooldown"
        );
    }

    #[test]
    fn test_trade_pad_building_type() {
        assert_eq!(BuildingType::TradePad.label(), "Trade Pad");
        assert!(!BuildingType::TradePad.is_obstacle());
        let cost = BuildingType::TradePad.cost(crate::layer1::building::MaterialType::Wood);
        assert_eq!(cost.metal, 30.0);
        assert_eq!(cost.stone, 20.0);
    }

    #[test]
    fn test_stats_label_and_census() {
        let mut world = test_world();
        assert_eq!(merchant_stats_label(&mut world), "none");
        world.spawn(TradePad { active: true });
        world.spawn(MerchantShip {
            name: "Test Trader".to_string(),
            buying: ResourceType::Metal,
            selling: ResourceType::Food,
            exchange_rate: 2.0,
            inventory: 50.0,
            arrival_tick: 0,
            departure_tick: 500,
        });
        assert_eq!(merchant_stats_label(&mut world), "Metal>Food@2.00");
        let census = merchant_census(&mut world);
        assert_eq!(census.pads, 1);
        assert_eq!(census.ships, 1);
    }

    #[test]
    fn test_ip_guard_no_borrowed_names() {
        // Original names only: no distinctive borrowed IP in public strings.
        const BANNED: [&str; 10] = [
            "millennium falcon",
            "enterprise",
            "serenity",
            "nostromo",
            "star wars",
            "star trek",
            "firefly",
            "battlestar",
            "rocinante",
            "normandy",
        ];
        let mut haystack = MERCHANT_SHIP_NAMES.join(" ").to_lowercase();
        haystack.push_str(&resource_label(ResourceType::Food).to_lowercase());
        for banned in BANNED {
            assert!(!haystack.contains(banned), "banned term present: {banned}");
        }
    }
}
