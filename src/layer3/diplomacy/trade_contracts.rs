//! Trade Contracts (Spec 396).
//!
//! Time-limited Layer 3 faction production contracts: a foreign faction
//! asks the colony to export a set amount of one resource by a deadline.
//! Fulfilling a contract deducts the exact amount from the colony's real
//! stockpile and pays credits plus reputation; missing the deadline fails
//! the contract and levies debt plus a relations penalty.
//!
//! ## Adaptations from the mock-style spec (§8 grounding)
//!
//! * **Storage.** The spec sketch's `Storage { capacity, current_amount,
//!   resource_type }` component does not exist. The game's real storage is
//!   the global [`ColonyResources`] ledger — the same ledger spec 039's
//!   [`execute_trade`](crate::layer1::economy::trade::execute_trade) debits
//!   via `try_consume` / `get_amount`. Fulfillment therefore carries no
//!   `storage_source` entity: [`FulfillContractEvent`] names only the
//!   contract, and delivery debits the real ledger. No parallel storage.
//! * **Resource type.** The sketch's `ResourceType::MetalBlock` does not
//!   exist in the real [`ResourceType`] enum. The generated rotation uses
//!   real refined goods — [`ResourceType::Blocks`] (the real "blocks" the
//!   sketch's name gestures at), Metal, Planks, Wood — in amounts that fit
//!   the ledger's real caps. Contracts can name any real `ResourceType`.
//! * **Amounts are `f32`.** The sketch's `u32` amounts / rewards are
//!   adapted to the ledger's native `f32`, so deduction is exact against
//!   `ColonyResources` with no cast rounding.
//! * **Faction id.** Foreign factions are not entities: Layer 3 diplomacy
//!   keys them by `u32` id in [`FactionRelations`] (scores −100 Hostile ..
//!   100 Allied, per `system_sovereignty`; the diplomatic-marriage pass
//!   uses the same keying). `TradeContract::faction_id` is that `u32` —
//!   the layer-1 guild `FactionId` enum and the biological stock market's
//!   niche `FactionId(u32)` wrapper are not the diplomacy seam.
//! * **Credits & debt.** Rewards land in the real `ColonyResources`
//!   credits via `add_credits`. Failure debt follows the insurance audit's
//!   fine pattern (spec 364): levy `min(debt, credits)` straight from the
//!   ledger so credits never go negative through this path, and record
//!   the levied amount in [`ContractLedger`].
//! * **Trade blockade.** Fulfillment honours spec 364's real
//!   [`TradeBlockade`] seam exactly as `execute_trade` does: while a
//!   blockade is active, deliveries are refused (the event is consumed,
//!   nothing moves, the contract stays Active).
//! * **Partial fulfillment (§5).** The real seams make it cheap — the
//!   ledger reports how much is on hand — so it is built: a delivery
//!   moves `min(on hand, remaining)`, accumulates into
//!   `delivered_amount`, and only the completing delivery pays out.
//! * **Generation.** Deterministic rotation (no RNG): the director offers
//!   the first contract at tick 100 (the spec GREEN example), then one
//!   every [`CONTRACT_OFFER_INTERVAL`] ticks while fewer than
//!   [`CONTRACT_MAX_ACTIVE`] contracts are Active, cycling a fixed
//!   faction/resource table. Offers are ledgered and chronicled.
//!
//! All names and strings are original and generic — no borrowed IP (see
//! the IP-guard test).

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::insurance::TradeBlockade;
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants (new for spec 396; ZERO existing constants retuned).
// Magnitudes grounded in the real economy: an organ sells for 120cr, a
// scion allowance is 50cr/1000 ticks, an artifact sale is ~100cr+ — a
// 10-unit export paying 50cr is a modest, real sum.
// ---------------------------------------------------------------------------

/// Tick of the first contract offer (the spec GREEN example's tick 100).
pub const CONTRACT_FIRST_OFFER_TICK: u64 = 100;
/// Ticks between contract offers.
pub const CONTRACT_OFFER_INTERVAL: u64 = 2000;
/// Ticks a generated contract gives the colony to deliver.
pub const CONTRACT_DURATION_TICKS: u64 = 2000;
/// Maximum simultaneously Active generated contracts.
pub const CONTRACT_MAX_ACTIVE: usize = 3;
/// Credits paid per unit delivered on a generated contract.
pub const REWARD_CREDITS_PER_UNIT: f32 = 5.0;
/// Failure debt as a fraction of the contract's reward.
pub const FAILURE_DEBT_FRACTION: f32 = 0.5;
/// Faction-relations gain when a contract is fulfilled.
pub const FULFILL_RELATIONS_BONUS: i32 = 10;
/// Faction-relations change when a contract fails (a penalty).
pub const FAILURE_RELATIONS_PENALTY: i32 = -20;

/// Lifecycle state of a [`TradeContract`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractStatus {
    /// Offered and awaiting delivery (deadline still ahead or reached
    /// but not yet processed).
    Active,
    /// Fully delivered before the deadline; reward paid.
    Fulfilled,
    /// Deadline passed while still Active; debt and hostility applied.
    Failed,
}

/// A time-limited Layer 3 faction production contract (spec 396).
#[derive(Component, Debug, Clone)]
pub struct TradeContract {
    /// Layer-3 faction id in [`FactionRelations`].
    pub faction_id: u32,
    /// The real resource to export.
    pub required_resource: ResourceType,
    /// Total amount to export.
    pub required_amount: f32,
    /// Amount already delivered (partial fulfillment, §5).
    pub delivered_amount: f32,
    /// Tick at which an unfulfilled contract fails.
    pub deadline_tick: u64,
    /// Credits paid on completion.
    pub reward_credits: f32,
    /// Current lifecycle state.
    pub status: ContractStatus,
}

impl TradeContract {
    /// Amount still owed before this contract completes.
    #[must_use]
    pub fn remaining(&self) -> f32 {
        (self.required_amount - self.delivered_amount).max(0.0)
    }
}

/// Event: deliver toward (or complete) a contract from the colony
/// ledger. Processed by [`fulfill_contract_system`]; moves
/// `min(on hand, remaining)` — see the module docs on partial delivery.
#[derive(Event, Debug, Clone, Copy)]
pub struct FulfillContractEvent {
    /// The contract to deliver against.
    pub contract: Entity,
}

/// Event: a contract was fully delivered and paid out.
#[derive(Event, Debug, Clone, Copy)]
pub struct ContractFulfilledEvent {
    /// The completed contract.
    pub contract: Entity,
    /// Layer-3 faction id that placed the contract.
    pub faction_id: u32,
    /// Resource exported.
    pub resource: ResourceType,
    /// Total amount exported.
    pub amount: f32,
    /// Credits paid.
    pub reward_credits: f32,
    /// Tick of completion.
    pub tick: u64,
}

/// Event: a contract missed its deadline and failed.
#[derive(Event, Debug, Clone, Copy)]
pub struct ContractFailedEvent {
    /// The failed contract.
    pub contract: Entity,
    /// Layer-3 faction id that placed the contract.
    pub faction_id: u32,
    /// Debt actually levied from the colony ledger.
    pub debt_levied: f32,
    /// Tick of failure.
    pub tick: u64,
}

/// Generation director: when the next offer is due and how many offers
/// have been made (the rotation index into the offer table).
#[derive(Resource, Debug, Clone, Copy)]
pub struct ContractDirector {
    /// Tick at or after which the next offer may be generated.
    pub next_offer_tick: u64,
    /// Offers generated so far.
    pub offers_made: u64,
}

impl Default for ContractDirector {
    fn default() -> Self {
        Self {
            next_offer_tick: CONTRACT_FIRST_OFFER_TICK,
            offers_made: 0,
        }
    }
}

/// Ledger of contract outcomes (headless STATS / panel).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ContractLedger {
    /// Contracts offered (generated or spawned by debug).
    pub offered: u64,
    /// Contracts fulfilled.
    pub fulfilled: u64,
    /// Contracts failed on deadline.
    pub failed: u64,
    /// Partial deliveries (a delivery that did not complete a contract).
    pub partial_deliveries: u64,
    /// Total credits paid out for fulfilled contracts.
    pub credits_earned: f32,
    /// Total debt levied for failed contracts.
    pub debt_levied: f32,
}

/// Generic Layer-3 faction name for chronicles. Original names only.
#[must_use]
pub fn contract_faction_name(faction_id: u32) -> &'static str {
    match faction_id {
        1 => "the Homeworld Combine",
        2 => "the Outer Rim Cooperative",
        3 => "the Free Traders League",
        _ => "a foreign faction",
    }
}

/// The deterministic offer rotation: `(faction_id, resource, amount)`.
/// Amounts fit the real ledger caps (Blocks 20, Metal 20, Planks 50,
/// Wood 50 in a default colony).
#[must_use]
pub fn contract_offer(rotation: u64) -> (u32, ResourceType, f32) {
    match rotation % 4 {
        0 => (1, ResourceType::Blocks, 10.0),
        1 => (2, ResourceType::Metal, 10.0),
        2 => (3, ResourceType::Planks, 15.0),
        _ => (1, ResourceType::Wood, 20.0),
    }
}

/// Parse a headless resource name into the real [`ResourceType`]
/// (case-insensitive; `metalblocks`/`block` alias to real Blocks).
#[must_use]
pub fn parse_contract_resource(text: &str) -> Option<ResourceType> {
    match text.to_ascii_lowercase().as_str() {
        "food" => Some(ResourceType::Food),
        "wood" => Some(ResourceType::Wood),
        "stone" => Some(ResourceType::Stone),
        "ore" => Some(ResourceType::Ore),
        "metal" => Some(ResourceType::Metal),
        "planks" | "plank" => Some(ResourceType::Planks),
        "blocks" | "block" | "metalblocks" | "metalblock" => Some(ResourceType::Blocks),
        "tools" | "tool" => Some(ResourceType::Tools),
        "scrap" => Some(ResourceType::Scrap),
        "fuel" => Some(ResourceType::Fuel),
        "rations" | "ration" => Some(ResourceType::Rations),
        "alcohol" => Some(ResourceType::Alcohol),
        _ => None,
    }
}

/// Apply a clamped relations delta (−100 Hostile .. 100 Allied).
fn adjust_relations(relations: &mut FactionRelations, faction_id: u32, delta: i32) {
    let score = relations.scores.entry(faction_id).or_insert(0);
    *score = (*score + delta).clamp(-100, 100);
}

/// Generation: offers the next contract from the deterministic rotation
/// when the director's tick is due and a slot is free.
pub fn generate_contracts_system(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut director: ResMut<ContractDirector>,
    mut ledger: ResMut<ContractLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    contracts: Query<&TradeContract>,
) {
    if time.tick < director.next_offer_tick {
        return;
    }
    let active = contracts
        .iter()
        .filter(|c| c.status == ContractStatus::Active)
        .count();
    if active >= CONTRACT_MAX_ACTIVE {
        // All slots busy: retry next tick (the director's due tick stays
        // in the past, so an offer lands as soon as a slot frees).
        return;
    }
    let (faction_id, resource, amount) = contract_offer(director.offers_made);
    let reward = amount * REWARD_CREDITS_PER_UNIT;
    let deadline = time.tick + CONTRACT_DURATION_TICKS;
    commands.spawn(TradeContract {
        faction_id,
        required_resource: resource,
        required_amount: amount,
        delivered_amount: 0.0,
        deadline_tick: deadline,
        reward_credits: reward,
        status: ContractStatus::Active,
    });
    director.offers_made += 1;
    director.next_offer_tick = time.tick + CONTRACT_OFFER_INTERVAL;
    ledger.offered += 1;
    chronicle.send(AddChronicleEvent {
        text: format!(
            "TRADE CONTRACT: {} offers {reward:.0} credits for {amount:.0} {:?} delivered by tick {deadline}.",
            contract_faction_name(faction_id),
            resource,
        ),
        importance: EventImportance::Standard,
    });
}

/// Fulfillment: delivers toward contracts from the real colony ledger.
/// Refused wholesale while a spec-364 [`TradeBlockade`] is active (the
/// same seam `execute_trade` honours). Completing a contract pays its
/// credits into the ledger and warms [`FactionRelations`].
#[allow(clippy::too_many_arguments)]
pub fn fulfill_contract_system(
    mut events: EventReader<FulfillContractEvent>,
    mut contracts: Query<&mut TradeContract>,
    mut resources: ResMut<ColonyResources>,
    mut relations: ResMut<FactionRelations>,
    mut ledger: ResMut<ContractLedger>,
    blockade: Res<TradeBlockade>,
    time: Res<SimulationTime>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut fulfilled: EventWriter<ContractFulfilledEvent>,
) {
    for event in events.read() {
        if blockade.is_active(time.tick) {
            continue;
        }
        let Ok(mut contract) = contracts.get_mut(event.contract) else {
            continue;
        };
        if contract.status != ContractStatus::Active {
            continue;
        }
        let remaining = contract.remaining();
        if remaining <= 0.0 {
            continue;
        }
        let on_hand = resources.get_amount(contract.required_resource);
        let deliver = on_hand.min(remaining);
        if deliver <= 0.0 {
            continue;
        }
        // The real trade path's deduction seam: checked, exact, and it
        // cannot fail here (deliver <= on hand by construction).
        if !resources.try_consume(contract.required_resource, deliver) {
            continue;
        }
        contract.delivered_amount += deliver;
        let faction_id = contract.faction_id;
        let resource = contract.required_resource;
        let required = contract.required_amount;
        let delivered_total = contract.delivered_amount;
        if contract.remaining() <= 0.0 {
            contract.status = ContractStatus::Fulfilled;
            let reward = contract.reward_credits;
            resources.add_credits(reward);
            adjust_relations(&mut relations, faction_id, FULFILL_RELATIONS_BONUS);
            ledger.fulfilled += 1;
            ledger.credits_earned += reward;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "TRADE CONTRACT FULFILLED: {required:.0} {resource:?} exported to {}. {reward:.0} credits paid.",
                    contract_faction_name(faction_id),
                ),
                importance: EventImportance::Major,
            });
            fulfilled.send(ContractFulfilledEvent {
                contract: event.contract,
                faction_id,
                resource,
                amount: required,
                reward_credits: reward,
                tick: time.tick,
            });
        } else {
            ledger.partial_deliveries += 1;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "Trade delivery: {deliver:.0} {resource:?} shipped to {} ({delivered_total:.0}/{required:.0}).",
                    contract_faction_name(faction_id),
                ),
                importance: EventImportance::Minor,
            });
        }
    }
}

/// Deadline check: an Active contract at or past its deadline fails —
/// debt is levied from the real credit ledger (the insurance-fine
/// pattern: never below zero) and the faction turns hostile.
pub fn check_contract_deadlines_system(
    time: Res<SimulationTime>,
    mut contracts: Query<(Entity, &mut TradeContract)>,
    mut resources: ResMut<ColonyResources>,
    mut relations: ResMut<FactionRelations>,
    mut ledger: ResMut<ContractLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut failed: EventWriter<ContractFailedEvent>,
) {
    for (entity, mut contract) in contracts.iter_mut() {
        if contract.status != ContractStatus::Active || time.tick < contract.deadline_tick {
            continue;
        }
        contract.status = ContractStatus::Failed;
        let faction_id = contract.faction_id;
        let resource = contract.required_resource;
        let required = contract.required_amount;
        let debt = contract.reward_credits * FAILURE_DEBT_FRACTION;
        let levied = debt.min(resources.credits.max(0.0));
        resources.credits -= levied;
        adjust_relations(&mut relations, faction_id, FAILURE_RELATIONS_PENALTY);
        ledger.failed += 1;
        ledger.debt_levied += levied;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "TRADE CONTRACT FAILED: the {required:.0} {resource:?} owed to {} never shipped. {levied:.0} credits levied in debt; relations sour.",
                contract_faction_name(faction_id),
            ),
            importance: EventImportance::Major,
        });
        failed.send(ContractFailedEvent {
            contract: entity,
            faction_id,
            debt_levied: levied,
            tick: time.tick,
        });
    }
}

// ---------------------------------------------------------------------------
// World-level helpers (headless console + tests)
// ---------------------------------------------------------------------------

/// Spawn a contract directly against the world (the headless/debug path;
/// the live path is [`generate_contracts_system`]). Also counts the
/// offer in the ledger when one is present.
pub fn spawn_trade_contract(
    world: &mut World,
    faction_id: u32,
    resource: ResourceType,
    amount: f32,
    deadline_tick: u64,
    reward_credits: f32,
) -> Entity {
    let entity = world
        .spawn(TradeContract {
            faction_id,
            required_resource: resource,
            required_amount: amount,
            delivered_amount: 0.0,
            deadline_tick,
            reward_credits,
            status: ContractStatus::Active,
        })
        .id();
    if let Some(mut ledger) = world.get_resource_mut::<ContractLedger>() {
        ledger.offered += 1;
    }
    entity
}

/// All contracts in entity order, cloned for display / indexing.
#[must_use]
pub fn contract_rows(world: &mut World) -> Vec<(Entity, TradeContract)> {
    let mut rows: Vec<(Entity, TradeContract)> = world
        .query::<(Entity, &TradeContract)>()
        .iter(world)
        .map(|(e, c)| (e, c.clone()))
        .collect();
    rows.sort_by_key(|(e, _)| e.index());
    rows
}

/// Census for STATS: `active/fulfilled/failed` contract counts.
#[must_use]
pub fn contracts_stats_label(world: &mut World) -> String {
    let rows = contract_rows(world);
    let count = |status: ContractStatus| rows.iter().filter(|(_, c)| c.status == status).count();
    format!(
        "{}/{}/{}",
        count(ContractStatus::Active),
        count(ContractStatus::Fulfilled),
        count(ContractStatus::Failed)
    )
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn contract_public_strings() -> Vec<String> {
    vec![
        "TRADE CONTRACT: a foreign faction offers credits for goods delivered by a deadline."
            .to_string(),
        "TRADE CONTRACT FULFILLED: goods exported. Credits paid.".to_string(),
        "TRADE CONTRACT FAILED: the goods never shipped. Credits levied in debt; relations sour."
            .to_string(),
        "Trade Contracts".to_string(),
        contract_faction_name(1).to_string(),
        contract_faction_name(2).to_string(),
        contract_faction_name(3).to_string(),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn setup(tick: u64) -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        world.insert_resource(ColonyResources::default());
        world.insert_resource(FactionRelations::default());
        world.init_resource::<ContractDirector>();
        world.init_resource::<ContractLedger>();
        world.insert_resource(TradeBlockade::default());
        world.init_resource::<Events<FulfillContractEvent>>();
        world.init_resource::<Events<ContractFulfilledEvent>>();
        world.init_resource::<Events<ContractFailedEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn run_generate(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_contracts_system);
        schedule.run(world);
    }

    fn run_fulfill(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(fulfill_contract_system);
        schedule.run(world);
    }

    fn run_deadlines(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(check_contract_deadlines_system);
        schedule.run(world);
    }

    // Spec RED test 1, grounded: generation spawns a real contract.
    // (Sketch spawned via a mocked tick-100 step against a `Storage`
    // world; the real director offers at tick 100 from the rotation.)
    #[test]
    fn test_contract_generation() {
        let mut world = setup(100);
        run_generate(&mut world);
        let rows = contract_rows(&mut world);
        assert_eq!(rows.len(), 1, "one contract should be offered at tick 100");
        let (_, contract) = &rows[0];
        assert_eq!(contract.status, ContractStatus::Active);
        assert_eq!(contract.faction_id, 1);
        assert_eq!(contract.required_resource, ResourceType::Blocks);
        assert_eq!(contract.required_amount, 10.0);
        assert_eq!(contract.deadline_tick, 100 + CONTRACT_DURATION_TICKS);
        assert_eq!(
            contract.reward_credits,
            10.0 * REWARD_CREDITS_PER_UNIT
        );
        assert_eq!(world.resource::<ContractLedger>().offered, 1);
    }

    #[test]
    fn test_generation_waits_for_due_tick_and_caps_active() {
        let mut world = setup(50);
        run_generate(&mut world);
        assert!(contract_rows(&mut world).is_empty(), "no offer before tick 100");

        // Fill all slots by hand; a due generation must not exceed the cap.
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        for _ in 0..CONTRACT_MAX_ACTIVE {
            spawn_trade_contract(&mut world, 1, ResourceType::Wood, 5.0, 9999, 25.0);
        }
        run_generate(&mut world);
        assert_eq!(contract_rows(&mut world).len(), CONTRACT_MAX_ACTIVE);
    }

    // Spec RED test 2, grounded: fulfillment deducts the exact amount
    // from real storage. (Sketch: a `Storage` entity holding 1500
    // MetalBlock loses 1000. Real seam: the ColonyResources ledger —
    // 15 Blocks on hand, a 10-Block contract leaves exactly 5.)
    #[test]
    fn test_contract_fulfillment() {
        let mut world = setup(0);
        world.resource_mut::<ColonyResources>().blocks = 15.0;
        let contract = spawn_trade_contract(&mut world, 1, ResourceType::Blocks, 10.0, 5000, 500.0);
        world.send_event(FulfillContractEvent { contract });
        run_fulfill(&mut world);

        let resources = world.resource::<ColonyResources>();
        assert_eq!(resources.blocks, 5.0, "exactly 10 Blocks were deducted");
        assert_eq!(resources.credits, 500.0, "reward paid into the real ledger");
        let stored = world.get::<TradeContract>(contract).unwrap();
        assert_eq!(stored.status, ContractStatus::Fulfilled);
        assert_eq!(stored.delivered_amount, 10.0);
        assert_eq!(
            *world
                .resource::<FactionRelations>()
                .scores
                .get(&1)
                .unwrap(),
            FULFILL_RELATIONS_BONUS,
            "fulfillment warms relations"
        );
        let ledger = world.resource::<ContractLedger>();
        assert_eq!(ledger.fulfilled, 1);
        assert_eq!(ledger.credits_earned, 500.0);
    }

    #[test]
    fn test_partial_fulfillment_accumulates_then_completes() {
        // §5: with only part of the order on hand, a delivery ships what
        // exists, stays Active, and pays nothing until completion.
        let mut world = setup(0);
        world.resource_mut::<ColonyResources>().blocks = 4.0;
        let contract = spawn_trade_contract(&mut world, 2, ResourceType::Blocks, 10.0, 5000, 500.0);

        world.send_event(FulfillContractEvent { contract });
        run_fulfill(&mut world);
        let stored = world.get::<TradeContract>(contract).unwrap();
        assert_eq!(stored.status, ContractStatus::Active);
        assert_eq!(stored.delivered_amount, 4.0);
        assert_eq!(world.resource::<ColonyResources>().blocks, 0.0);
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);
        assert_eq!(world.resource::<ContractLedger>().partial_deliveries, 1);

        world.resource_mut::<ColonyResources>().blocks = 6.0;
        world.send_event(FulfillContractEvent { contract });
        run_fulfill(&mut world);
        let stored = world.get::<TradeContract>(contract).unwrap();
        assert_eq!(stored.status, ContractStatus::Fulfilled);
        assert_eq!(world.resource::<ColonyResources>().credits, 500.0);
    }

    #[test]
    fn test_fulfillment_refused_under_trade_blockade() {
        // Spec 364 seam: while the blockade is active the real trade
        // path refuses — contracts are no exception.
        let mut world = setup(10);
        world.insert_resource(TradeBlockade { until_tick: 9999 });
        world.resource_mut::<ColonyResources>().blocks = 15.0;
        let contract = spawn_trade_contract(&mut world, 1, ResourceType::Blocks, 10.0, 5000, 500.0);
        world.send_event(FulfillContractEvent { contract });
        run_fulfill(&mut world);
        assert_eq!(world.resource::<ColonyResources>().blocks, 15.0);
        assert_eq!(
            world.get::<TradeContract>(contract).unwrap().status,
            ContractStatus::Active
        );
    }

    // Spec RED test 3, grounded: the deadline fails an Active contract,
    // with the §5 consequences (debt + hostility) wired to real seams.
    #[test]
    fn test_contract_failure_on_deadline() {
        let mut world = setup(6000);
        world.resource_mut::<ColonyResources>().credits = 100.0;
        let contract = spawn_trade_contract(&mut world, 1, ResourceType::Blocks, 10.0, 5000, 500.0);
        run_deadlines(&mut world);

        let stored = world.get::<TradeContract>(contract).unwrap();
        assert_eq!(stored.status, ContractStatus::Failed);
        // Debt is 50% of the 500cr reward = 250, levied only up to the
        // 100cr on hand (insurance-fine pattern: never below zero).
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);
        assert_eq!(world.resource::<ContractLedger>().debt_levied, 100.0);
        assert_eq!(world.resource::<ContractLedger>().failed, 1);
        assert_eq!(
            *world
                .resource::<FactionRelations>()
                .scores
                .get(&1)
                .unwrap(),
            FAILURE_RELATIONS_PENALTY,
            "failure sours relations"
        );
    }

    #[test]
    fn test_fulfilled_contract_survives_deadline_check() {
        let mut world = setup(0);
        world.resource_mut::<ColonyResources>().blocks = 15.0;
        let contract = spawn_trade_contract(&mut world, 1, ResourceType::Blocks, 10.0, 5000, 500.0);
        world.send_event(FulfillContractEvent { contract });
        run_fulfill(&mut world);

        world.insert_resource(SimulationTime {
            tick: 6000,
            ..Default::default()
        });
        run_deadlines(&mut world);
        assert_eq!(
            world.get::<TradeContract>(contract).unwrap().status,
            ContractStatus::Fulfilled,
            "a fulfilled contract must never flip to Failed"
        );
        assert_eq!(world.resource::<ContractLedger>().failed, 0);
        assert_eq!(world.resource::<ColonyResources>().credits, 500.0);
    }

    #[test]
    fn test_parse_and_ip_guard() {
        assert_eq!(parse_contract_resource("blocks"), Some(ResourceType::Blocks));
        assert_eq!(
            parse_contract_resource("MetalBlock"),
            Some(ResourceType::Blocks),
            "the sketch's mock type aliases to the real Blocks"
        );
        assert_eq!(parse_contract_resource("METAL"), Some(ResourceType::Metal));
        assert_eq!(parse_contract_resource("bogus"), None);
        assert_eq!(contract_offer(0), (1, ResourceType::Blocks, 10.0));
        assert_eq!(contract_offer(4), contract_offer(0), "rotation wraps");

        // IP guard: player-facing strings are original/generic only.
        for text in contract_public_strings() {
            let lower = text.to_ascii_lowercase();
            for banned in [
                "star wars",
                "star trek",
                "warhammer",
                "dune",
                "arrakis",
                "klingon",
                "imperium of man",
                "foundation",
                "trantor",
            ] {
                assert!(!lower.contains(banned), "contract string leaks IP: {text}");
            }
        }
    }
}
