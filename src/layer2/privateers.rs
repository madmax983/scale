//! Spec 421: The Pirate Emissary — hire the wolves to guard the sheep.
//!
//! A Layer 2 pirate fleet can be hired as privateers: while the contract
//! is [`ContractState::Active`] and tribute is paid, the privateers repel
//! pirate raids on the capital and escort trade ships for a cut of their
//! cargo value. Tribute falls due on an interval and scales with colony
//! wealth, so a booming colony is farmed harder every cycle. Fail to pay
//! — by refusal or by an empty treasury past the grace window — and the
//! contract is [`ContractState::Breached`]: the privateers turn hostile
//! and raid the capital themselves.
//!
//! ## Grounding audit (spec §3/§4 mocks vs the real seams)
//!
//! * **`PlayerTreasury` is a MOCK.** The real treasury is
//!   [`ColonyResources`][crate::layer1::resources::ColonyResources],
//!   whose `credits` field is the colony's money (paid out through
//!   `add_credits`, checked/spent by direct field arithmetic exactly as
//!   the raid bridge spends `food`/`metal`). Tribute is credits-only:
//!   the spec's "resources or Pops" tribute is narrowed to the one
//!   liquid currency every other cross-layer contract already uses.
//! * **`Faction` / `Hostility` are MOCKS.** The real fleet faction is
//!   [`FleetFaction::Pirate`][crate::layer2::fleet::FleetFaction] on a
//!   real [`Fleet`][crate::layer2::fleet::Fleet] entity, and the real
//!   hostility ledger is the Layer 3
//!   [`FactionRelations`] score map (-100 hostile .. 100 allied). The
//!   pirates' id there is 13 — the same id
//!   `layer1::social::martyrdom::PIRATE_FACTION_ID` already uses; this
//!   module names it [`PRIVATEER_FACTION_ID`] (the cadet/inspector
//!   precedent: one local const per consumer, one shared id space).
//!   The spec's `Hostility(100.0)` therefore lands as relations = -100
//!   plus a real raid (below), not as a decorative component.
//! * **`ProtectionBuff` is a MOCK — and stays unwired as a flag.** The
//!   brief requires protection on real seams, so there is no buff
//!   component: while a contract is Active, (a) the real raid bridge
//!   (`beacon_pirate_raid_bridge`, INT-762) checks
//!   [`PrivateerState::protection_active`] and turns an incoming
//!   [`PirateRaidEvent`] into a repelled raid (no theft, no morale hit,
//!   ledger-counted) instead of resolving it, and (b)
//!   [`privateer_trade_escort_system`] reads the real
//!   [`TradeShipArrivalEvent`] flow and pays the colony an escort bonus
//!   of [`ESCORT_BONUS_RATE`] of cargo value in real credits. The
//!   "attack rivals" leg has no cheap real seam — Layer 2 fleet combat
//!   only resolves fleets co-located at one orbital body, and spawning
//!   rival fleets to sink on a timer would be a balance change to an
//!   unrelated system — so it is deliberately NOT built; the privateers'
//!   offensive pressure is expressed through the relations ledger and
//!   the breach raid. Documented deviation, per the brief's §5 rule.
//! * **The contract carrier is real.** The emissary is a spawned fleet
//!   entity (`Fleet` + `FleetFaction::Pirate` + `FleetComposition` of
//!   two frigates and a scout, `InOrbit` at the first orbital body when
//!   one exists) carrying [`PrivateerContract`]. The Layer 3 amnesty
//!   seam can therefore despawn it like any pirate fleet; the tribute
//!   system re-syncs `protection_active` from live contracts every
//!   tick, so a vanished fleet stops protecting immediately.
//! * **Raids are the real event.** Breach sends the real
//!   [`PirateRaidEvent`][crate::layer1::void_weed::PirateRaidEvent] —
//!   the same event the Director spends and smuggling heat send — and
//!   because protection is off by then, the unmodified bridge resolves
//!   it into real food/metal theft and a morale hit. One pipeline both
//!   protects and punishes; nothing is simulated twice.
//! * **No new event type.** Tribute demands surface through the
//!   contract fields, the [`PrivateerState`] ledger resource, the real
//!   chronicle (`AddChronicleEvent`), and the headless panel — the
//!   brief's "real event/resource" leg is carried by the resource.
//!
//! ## Contract state machine (§5)
//!
//! `Pending` (emissary waiting) → `Active` (hired; tribute clock runs)
//! → `Breached` (refused, or a demand unpaid past
//! [`TRIBUTE_GRACE_TICKS`]). Breach is terminal: hire a fresh emissary
//! to start over. Payment choice is explicit: [`PrivateerState::
//! auto_pay`] is the standing order (default ON — the automate leg),
//! `pay_tribute_now` is the manual leg, `refuse_tribute` is the reject
//! leg; all three are exposed in headless.
//!
//! ## Constants (new, Spec 421 only — no unrelated retunes)
//!
//! * [`TRIBUTE_INTERVAL_TICKS`] 500 between demands.
//! * [`TRIBUTE_GRACE_TICKS`] 100 to settle a demand before breach.
//! * [`BASE_TRIBUTE`] 50 credits even for a broke colony.
//! * [`TRIBUTE_WEALTH_RATE`] 5% of colony wealth per demand — the
//!   extortion curve: wealth is credits + food + wood + stone + metal
//!   + fuel at 1:1 (see [`colony_wealth`]).
//! * [`ESCORT_BONUS_RATE`] 1% of an arriving trade ship's cargo value.
//! * [`HIRE_RELATIONS`] +10 pirate relations on signing;
//!   [`BREACH_RELATIONS`] -100 (full hostile) on breach.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::resources::ColonyResources;
use crate::layer1::void_weed::PirateRaidEvent;
use crate::layer2::fleet::{Fleet, FleetComposition, FleetFaction, InOrbit};
use crate::layer2::ship::{Ship, ShipType};
use crate::layer2::system::OrbitalBody;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;

/// Layer 3 [`FactionRelations`] id of the pirate faction. Same id as
/// `layer1::social::martyrdom::PIRATE_FACTION_ID` (13) — one id space.
pub const PRIVATEER_FACTION_ID: u32 = 13;
/// Ticks between tribute demands while a contract is Active.
pub const TRIBUTE_INTERVAL_TICKS: u64 = 500;
/// Ticks a demand may sit unpaid before the contract breaches.
pub const TRIBUTE_GRACE_TICKS: u64 = 100;
/// Flat tribute floor, charged even at zero wealth.
pub const BASE_TRIBUTE: f32 = 50.0;
/// Share of colony wealth demanded per tribute cycle (5%).
pub const TRIBUTE_WEALTH_RATE: f32 = 0.05;
/// Escort bonus: share of an arriving trade ship's cargo value paid to
/// the colony in credits while privateers protect the routes.
pub const ESCORT_BONUS_RATE: f32 = 0.01;
/// Pirate relations set when a contract is signed.
pub const HIRE_RELATIONS: i32 = 10;
/// Pirate relations set when a contract breaches (full hostile).
pub const BREACH_RELATIONS: i32 = -100;

/// Where a privateer contract stands (spec §5 state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContractState {
    /// The emissary has arrived; no contract signed yet.
    #[default]
    Pending,
    /// Signed: tribute clock running, protection live.
    Active,
    /// Tribute refused or left unpaid past grace: hostile. Terminal.
    Breached,
}

impl ContractState {
    /// Player-facing label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Breached => "breached",
        }
    }
}

/// The contract itself, carried by the privateer fleet entity.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct PrivateerContract {
    /// State-machine state.
    pub state: ContractState,
    /// Most recently computed tribute demand (credits).
    pub tribute_demand: f32,
    /// Demand currently awaiting payment (0.0 = nothing owed).
    pub outstanding: f32,
    /// Tick the outstanding demand fell due (grace counts from here).
    pub due_tick: u64,
    /// Next tick a fresh demand falls due.
    pub next_due_tick: u64,
    /// Demands paid over this contract's life.
    pub payments_made: u64,
}

impl Default for PrivateerContract {
    fn default() -> Self {
        Self {
            state: ContractState::Pending,
            tribute_demand: 0.0,
            outstanding: 0.0,
            due_tick: 0,
            next_due_tick: 0,
            payments_made: 0,
        }
    }
}

/// Colony-wide privateer ledger + standing payment policy (Spec 421).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct PrivateerState {
    /// Standing order: settle each demand automatically when the
    /// treasury can afford it (the spec §7 automate leg). Turn it off
    /// to force the explicit pay/refuse choice every cycle.
    pub auto_pay: bool,
    /// Synced from live contracts by the tribute system (and by the
    /// hire/pay/refuse helpers): true while any contract is Active.
    /// The raid bridge reads exactly this flag.
    pub protection_active: bool,
    /// Total credits paid in tribute since colony start.
    pub total_tribute_paid: f32,
    /// Demands paid (all contracts, all time).
    pub payments: u64,
    /// Contracts breached (all time).
    pub breaches: u64,
    /// Raids turned away by privateer protection (via the real bridge).
    pub raids_repelled: u64,
    /// Trade ships escorted while protection was live.
    pub convoys_escorted: u64,
    /// Credits earned from escort bonuses.
    pub escort_income: f32,
    /// Emissaries that have arrived (contracts spawned).
    pub emissaries: u64,
}

impl PrivateerState {
    /// Ledger with the default standing order (auto-pay on).
    #[must_use]
    pub fn new() -> Self {
        Self {
            auto_pay: true,
            ..Self::default()
        }
    }
}

/// Colony wealth for tribute scaling: credits plus the liquid
/// stockpile (food, wood, stone, metal, fuel) at 1:1. Deliberately
/// narrow — knowledge, tools and capped luxury goods are not wealth
/// the emissary can see from orbit.
#[must_use]
pub fn colony_wealth(resources: &ColonyResources) -> f32 {
    (resources.credits
        + resources.food
        + resources.wood
        + resources.stone
        + resources.metal
        + resources.fuel)
        .max(0.0)
}

/// Tribute demanded for one cycle at `wealth`: the flat floor plus
/// [`TRIBUTE_WEALTH_RATE`] of wealth. Pure, so the extortion curve is
/// unit-testable without a world.
#[must_use]
pub fn tribute_demand_for(wealth: f32) -> f32 {
    BASE_TRIBUTE + TRIBUTE_WEALTH_RATE * wealth.max(0.0)
}


/// Insert the ledger with the game-default standing order (auto-pay
/// ON) when absent. Never `init_resource` here: the derived
/// `Default` has auto-pay off, which would silently flip the policy.
fn ensure_state(world: &mut World) {
    if world.get_resource::<PrivateerState>().is_none() {
        world.insert_resource(PrivateerState::new());
    }
}

fn current_tick(world: &World) -> u64 {
    world.get_resource::<SimulationTime>().map_or(0, |t| t.tick)
}

fn chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(text.clone());
    }
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

fn set_pirate_relations(world: &mut World, score: i32) {
    if let Some(mut relations) = world.get_resource_mut::<FactionRelations>() {
        relations.scores.insert(PRIVATEER_FACTION_ID, score);
    }
}

/// Recompute [`PrivateerState::protection_active`] from live contracts.
fn sync_protection(world: &mut World) {
    let active = {
        let mut query = world.query::<&PrivateerContract>();
        query.iter(world).any(|c| c.state == ContractState::Active)
    };
    if let Some(mut state) = world.get_resource_mut::<PrivateerState>() {
        state.protection_active = active;
    }
}

/// Spawn the emissary: a real pirate fleet carrying a Pending
/// contract. Used by the headless `emissary` command and by
/// [`hire_privateers`] when no emissary is waiting.
pub fn spawn_emissary(world: &mut World) -> Entity {
    ensure_state(world);
    let parent = {
        let mut query = world.query_filtered::<Entity, With<OrbitalBody>>();
        query.iter(world).next()
    };
    let mut composition = FleetComposition::default();
    composition.add_ship(Ship::new(ShipType::Frigate));
    composition.add_ship(Ship::new(ShipType::Frigate));
    composition.add_ship(Ship::new(ShipType::Scout));
    let mut entity = world.spawn((
        Fleet,
        FleetFaction::Pirate,
        composition,
        PrivateerContract::default(),
    ));
    if let Some(parent) = parent {
        entity.insert(InOrbit { parent });
    }
    let fleet = entity.id();
    if let Some(mut state) = world.get_resource_mut::<PrivateerState>() {
        state.emissaries += 1;
    }
    chronicle(
        world,
        "A pirate emissary has arrived under a flag of parley, offering privateer service: protection for tribute, priced against everything the colony owns.".to_string(),
        EventImportance::Major,
    );
    fleet
}

/// First contract entity in `state`, if any.
fn contract_in_state(world: &mut World, wanted: ContractState) -> Option<Entity> {
    let mut query = world.query::<(Entity, &PrivateerContract)>();
    query
        .iter(world)
        .find(|(_, c)| c.state == wanted)
        .map(|(e, _)| e)
}

/// Sign the contract: Pending → Active, tribute clock starts, pirate
/// relations warm to [`HIRE_RELATIONS`]. With no emissary waiting, one
/// is spawned and signed on the spot (the headless fast path).
/// Returns false only if a contract is already Active or Breached
/// fleets are the only ones present.
pub fn hire_privateers(world: &mut World) -> bool {
    ensure_state(world);
    if contract_in_state(world, ContractState::Active).is_some() {
        return false;
    }
    let fleet = match contract_in_state(world, ContractState::Pending) {
        Some(entity) => entity,
        None => spawn_emissary(world),
    };
    let tick = current_tick(world);
    let wealth = world
        .get_resource::<ColonyResources>()
        .map_or(0.0, colony_wealth);
    if let Some(mut contract) = world.get_mut::<PrivateerContract>(fleet) {
        contract.state = ContractState::Active;
        contract.tribute_demand = tribute_demand_for(wealth);
        contract.next_due_tick = tick + TRIBUTE_INTERVAL_TICKS;
    }
    set_pirate_relations(world, HIRE_RELATIONS);
    sync_protection(world);
    chronicle(
        world,
        "The privateer contract is signed. The pirate fleet takes station over the trade routes; the first tribute falls due soon, and it will grow with the colony.".to_string(),
        EventImportance::Major,
    );
    true
}

/// Settle `fleet`'s outstanding demand from the real treasury.
/// Shared by auto-pay, the explicit headless `pay`, and the tests.
/// Returns false (changing nothing) when nothing is owed or the
/// treasury cannot cover the demand.
pub fn apply_payment(world: &mut World, fleet: Entity) -> bool {
    let Some(outstanding) = world.get::<PrivateerContract>(fleet).map(|c| c.outstanding) else {
        return false;
    };
    if outstanding <= 0.0 {
        return false;
    }
    let affordable = world
        .get_resource::<ColonyResources>()
        .is_some_and(|r| r.credits >= outstanding);
    if !affordable {
        return false;
    }
    if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
        resources.credits -= outstanding;
    }
    let tick = current_tick(world);
    if let Some(mut contract) = world.get_mut::<PrivateerContract>(fleet) {
        contract.outstanding = 0.0;
        contract.payments_made += 1;
        contract.next_due_tick = tick + TRIBUTE_INTERVAL_TICKS;
    }
    if let Some(mut state) = world.get_resource_mut::<PrivateerState>() {
        state.total_tribute_paid += outstanding;
        state.payments += 1;
    }
    chronicle(
        world,
        format!(
            "Tribute of {outstanding:.0} credits paid to the privateers. The wolves stay fed, and the routes stay safe — for now."
        ),
        EventImportance::Standard,
    );
    true
}

/// Pay the outstanding demand on the (first) Active contract — the
/// explicit-choice leg. False when nothing is owed or unaffordable.
pub fn pay_tribute_now(world: &mut World) -> bool {
    let Some(fleet) = contract_in_state(world, ContractState::Active) else {
        return false;
    };
    apply_payment(world, fleet)
}

/// Breach `fleet`'s contract: relations collapse to
/// [`BREACH_RELATIONS`], protection ends, and the fleet raids the
/// capital through the real [`PirateRaidEvent`] pipeline.
fn breach_contract(world: &mut World, fleet: Entity, reason: &str) {
    if let Some(mut contract) = world.get_mut::<PrivateerContract>(fleet) {
        if contract.state == ContractState::Breached {
            return;
        }
        contract.state = ContractState::Breached;
        contract.outstanding = 0.0;
    }
    set_pirate_relations(world, BREACH_RELATIONS);
    if let Some(mut state) = world.get_resource_mut::<PrivateerState>() {
        state.breaches += 1;
    }
    sync_protection(world);
    if let Some(mut events) = world.get_resource_mut::<Events<PirateRaidEvent>>() {
        events.send(PirateRaidEvent);
    }
    chronicle(
        world,
        format!(
            "The privateer contract is breached — {reason}. The fleet that guarded the routes turns its guns on the capital."
        ),
        EventImportance::Major,
    );
}

/// Refuse tribute: the Active (or Pending) contract breaches at once.
/// False when there is no live contract to refuse.
pub fn refuse_tribute(world: &mut World) -> bool {
    let target = contract_in_state(world, ContractState::Active)
        .or_else(|| contract_in_state(world, ContractState::Pending));
    let Some(fleet) = target else {
        return false;
    };
    breach_contract(world, fleet, "tribute was refused");
    true
}

/// Spec 421 tribute clock (exclusive system, the lagrange/ephemeral
/// pattern): re-syncs protection from live contracts, raises a
/// wealth-scaled demand on each Active contract's due tick, settles it
/// automatically under the standing auto-pay order when affordable,
/// and breaches any demand still unpaid past the grace window.
pub fn privateer_tribute_system(world: &mut World) {
    if world.get_resource::<PrivateerState>().is_none() {
        return;
    }
    sync_protection(world);
    let tick = current_tick(world);
    let fleets: Vec<Entity> = {
        let mut query = world.query::<(Entity, &PrivateerContract)>();
        query
            .iter(world)
            .filter(|(_, c)| c.state == ContractState::Active)
            .map(|(e, _)| e)
            .collect()
    };
    for fleet in fleets {
        // Raise a fresh demand when one falls due.
        let needs_demand = world.get::<PrivateerContract>(fleet).is_some_and(|c| {
            c.state == ContractState::Active && c.outstanding <= 0.0 && tick >= c.next_due_tick
        });
        if needs_demand {
            let wealth = world
                .get_resource::<ColonyResources>()
                .map_or(0.0, colony_wealth);
            let demand = tribute_demand_for(wealth);
            if let Some(mut contract) = world.get_mut::<PrivateerContract>(fleet) {
                contract.tribute_demand = demand;
                contract.outstanding = demand;
                contract.due_tick = tick;
            }
            let auto_pay = world
                .get_resource::<PrivateerState>()
                .is_some_and(|s| s.auto_pay);
            if auto_pay && apply_payment(world, fleet) {
                continue;
            }
            chronicle(
                world,
                format!(
                    "The privateers demand tribute of {demand:.0} credits. Pay before the grace window closes, or they collect it themselves."
                ),
                EventImportance::Major,
            );
        }
        // Breach a demand left unpaid past grace.
        let expired = world.get::<PrivateerContract>(fleet).is_some_and(|c| {
            c.state == ContractState::Active
                && c.outstanding > 0.0
                && tick > c.due_tick + TRIBUTE_GRACE_TICKS
        });
        if expired {
            breach_contract(world, fleet, "tribute went unpaid past the grace window");
        }
    }
    sync_protection(world);
}

/// Spec 421 trade protection: while protection is live, each arriving
/// trade ship pays the colony an escort bonus of
/// [`ESCORT_BONUS_RATE`] of its cargo value in real credits. Reads the
/// same [`TradeShipArrivalEvent`] flow the beacon and the trade bridge
/// use — an independent event cursor, so no other consumer is starved.
pub fn privateer_trade_escort_system(
    mut events: EventReader<TradeShipArrivalEvent>,
    mut ledger: Option<ResMut<PrivateerState>>,
    mut resources: Option<ResMut<ColonyResources>>,
) {
    let protected = ledger.as_deref().is_some_and(|s| s.protection_active);
    for event in events.read() {
        if !protected {
            continue;
        }
        let bonus = (event.cargo_value * ESCORT_BONUS_RATE).max(0.0);
        if let Some(resources) = resources.as_deref_mut() {
            resources.add_credits(bonus);
        }
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.convoys_escorted += 1;
            ledger.escort_income += bonus;
        }
    }
}

/// Census of privateer contracts for STATS / the headless panel.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PrivateerCensus {
    /// Live contracts by state.
    pub pending: usize,
    /// Active contracts.
    pub active: usize,
    /// Breached contracts (fleets still present).
    pub breached: usize,
    /// Demand currently outstanding across Active contracts.
    pub outstanding: f32,
    /// Next demand (credits) on the first Active contract, if any.
    pub next_demand: Option<f32>,
}

/// Count live contracts by state, plus what is owed right now.
#[must_use]
pub fn privateer_census(world: &mut World) -> PrivateerCensus {
    let mut census = PrivateerCensus::default();
    let mut query = world.query::<&PrivateerContract>();
    for contract in query.iter(world) {
        match contract.state {
            ContractState::Pending => census.pending += 1,
            ContractState::Active => {
                census.active += 1;
                census.outstanding += contract.outstanding;
                if census.next_demand.is_none() {
                    census.next_demand = Some(contract.tribute_demand);
                }
            }
            ContractState::Breached => census.breached += 1,
        }
    }
    census
}

/// Compact STATS label, e.g. `active:paid2/repelled1`. No spaces.
#[must_use]
pub fn privateer_stats_label(world: &mut World) -> String {
    let census = privateer_census(world);
    let (paid, repelled, breaches) = world
        .get_resource::<PrivateerState>()
        .map_or((0, 0, 0), |s| (s.payments, s.raids_repelled, s.breaches));
    if census.active > 0 {
        format!("active:paid{paid}/repelled{repelled}")
    } else if census.pending > 0 {
        "pending".to_string()
    } else if census.breached > 0 || breaches > 0 {
        "breached".to_string()
    } else {
        "none".to_string()
    }
}

/// Player-facing strings introduced by Spec 421 (IP-guard test input).
#[must_use]
pub fn privateer_public_strings() -> Vec<String> {
    vec![
        "privateer".to_string(),
        "pending".to_string(),
        "active".to_string(),
        "breached".to_string(),
        "A pirate emissary has arrived under a flag of parley, offering privateer service: protection for tribute, priced against everything the colony owns.".to_string(),
        "The privateer contract is signed. The pirate fleet takes station over the trade routes; the first tribute falls due soon, and it will grow with the colony.".to_string(),
        "The privateers demand tribute".to_string(),
        "Tribute of".to_string(),
        "paid to the privateers".to_string(),
        "The privateer contract is breached".to_string(),
        "turns its guns on the capital".to_string(),
        "Privateer escorts repelled a pirate raid on the capital.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::core::integration::beacon_pirate_raid_bridge;

    fn base_world(credits: f32) -> World {
        let mut world = World::new();
        let resources = ColonyResources {
            credits,
            max_credits: 1_000_000.0,
            ..ColonyResources::default()
        };
        world.insert_resource(resources);
        world.insert_resource(SimulationTime::default());
        world.insert_resource(PrivateerState::new());
        world.insert_resource(FactionRelations::default());
        world.insert_resource(MessageLog::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<PirateRaidEvent>>();
        world.init_resource::<Events<TradeShipArrivalEvent>>();
        world
    }

    fn run_tribute(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(privateer_tribute_system);
        schedule.run(world);
    }

    fn set_tick(world: &mut World, tick: u64) {
        world.resource_mut::<SimulationTime>().tick = tick;
    }

    #[test]
    fn test_tribute_demand_scales_with_wealth() {
        assert_eq!(tribute_demand_for(0.0), BASE_TRIBUTE);
        assert_eq!(tribute_demand_for(-50.0), BASE_TRIBUTE);
        let poor = tribute_demand_for(100.0);
        let rich = tribute_demand_for(10_000.0);
        assert!((poor - 55.0).abs() < f32::EPSILON, "got {poor}");
        assert!((rich - 550.0).abs() < 0.01, "got {rich}");
        assert!(rich > poor, "richer colonies are farmed harder");
    }

    #[test]
    fn test_hire_signs_pending_emissary() {
        let mut world = base_world(500.0);
        let fleet = spawn_emissary(&mut world);
        assert_eq!(
            world.get::<PrivateerContract>(fleet).unwrap().state,
            ContractState::Pending
        );
        assert_eq!(
            world.get::<FleetFaction>(fleet).copied().unwrap(),
            FleetFaction::Pirate,
            "the emissary rides a real pirate fleet"
        );
        assert!(world.get::<Fleet>(fleet).is_some());
        assert!(hire_privateers(&mut world));
        let contract = world.get::<PrivateerContract>(fleet).unwrap();
        assert_eq!(contract.state, ContractState::Active);
        assert_eq!(contract.next_due_tick, TRIBUTE_INTERVAL_TICKS);
        assert_eq!(
            world.resource::<FactionRelations>().scores[&PRIVATEER_FACTION_ID],
            HIRE_RELATIONS
        );
        assert!(world.resource::<PrivateerState>().protection_active);
        assert!(
            !hire_privateers(&mut world),
            "a second signing while Active is refused"
        );
    }

    /// Spec RED test 1, adapted to the real seams: the sketch's
    /// `PlayerTreasury { credits: 50 }` is `ColonyResources.credits`,
    /// and `Hostility > 0` is relations at -100 + Breached + a real
    /// raid event queued for the capital.
    #[test]
    fn test_privateer_demands_tribute_and_turns_hostile_if_unpaid() {
        let mut world = base_world(50.0);
        assert!(hire_privateers(&mut world));
        // Demand falls due; 50 credits cannot cover the wealth-scaled
        // demand (starter stockpile alone pushes it past 50), and the
        // grace window lapses.
        set_tick(&mut world, TRIBUTE_INTERVAL_TICKS);
        run_tribute(&mut world);
        let fleet = contract_in_state(&mut world, ContractState::Active)
            .expect("contract still active during grace");
        assert!(
            world.get::<PrivateerContract>(fleet).unwrap().outstanding > 50.0,
            "demand must exceed the broke treasury"
        );
        set_tick(&mut world, TRIBUTE_INTERVAL_TICKS + TRIBUTE_GRACE_TICKS + 1);
        run_tribute(&mut world);
        assert_eq!(
            world.get::<PrivateerContract>(fleet).unwrap().state,
            ContractState::Breached
        );
        assert_eq!(
            world.resource::<FactionRelations>().scores[&PRIVATEER_FACTION_ID],
            BREACH_RELATIONS,
            "unpaid tribute turns the fleet hostile (spec: hostility > 0)"
        );
        assert!(!world.resource::<PrivateerState>().protection_active);
        let raids = world.resource::<Events<PirateRaidEvent>>();
        assert_eq!(
            raids.get_cursor().read(raids).count(),
            1,
            "breach raids the capital through the real raid event"
        );
    }

    /// Spec RED test 2, adapted: the sketch's `paid: true` +
    /// `ProtectionBuff.active` is an auto-paid demand plus the real
    /// protection flag the raid bridge consults.
    #[test]
    fn test_privateer_protects_player_trade_when_paid() {
        let mut world = base_world(200.0);
        // Zero the stockpile so wealth is exactly the 200 credits and
        // the demand is deterministic: 50 + 5% * 200 = 60.
        {
            let mut r = world.resource_mut::<ColonyResources>();
            r.food = 0.0;
            r.wood = 0.0;
            r.stone = 0.0;
            r.metal = 0.0;
            r.fuel = 0.0;
        }
        assert!(hire_privateers(&mut world));
        set_tick(&mut world, TRIBUTE_INTERVAL_TICKS);
        run_tribute(&mut world);
        let state = world.resource::<PrivateerState>();
        assert_eq!(state.payments, 1, "auto-pay settles the demand");
        assert!(
            state.protection_active,
            "paid privateers protect (spec: protection buff active)"
        );
        let credits = world.resource::<ColonyResources>().credits;
        assert!(
            (credits - 140.0).abs() < 0.01,
            "200 - 60 tribute = 140, got {credits}"
        );
    }

    #[test]
    fn test_explicit_pay_clears_outstanding_before_grace_expires() {
        let mut world = base_world(1_000.0);
        world.resource_mut::<PrivateerState>().auto_pay = false;
        assert!(hire_privateers(&mut world));
        set_tick(&mut world, TRIBUTE_INTERVAL_TICKS);
        run_tribute(&mut world);
        let before = world.resource::<ColonyResources>().credits;
        assert!(pay_tribute_now(&mut world), "manual pay leg settles");
        let after = world.resource::<ColonyResources>().credits;
        assert!(after < before, "credits left the treasury");
        // Past grace, a paid contract must NOT breach.
        set_tick(
            &mut world,
            TRIBUTE_INTERVAL_TICKS + TRIBUTE_GRACE_TICKS + 50,
        );
        run_tribute(&mut world);
        assert_eq!(world.resource::<PrivateerState>().breaches, 0);
        assert!(contract_in_state(&mut world, ContractState::Active).is_some());
    }

    #[test]
    fn test_refuse_breaches_immediately() {
        let mut world = base_world(1_000.0);
        assert!(hire_privateers(&mut world));
        assert!(refuse_tribute(&mut world));
        assert_eq!(world.resource::<PrivateerState>().breaches, 1);
        assert_eq!(
            world.resource::<FactionRelations>().scores[&PRIVATEER_FACTION_ID],
            BREACH_RELATIONS
        );
        assert!(!refuse_tribute(&mut world), "nothing live left to refuse");
    }

    #[test]
    fn test_escort_bonus_only_while_protection_active() {
        let mut world = base_world(0.0);
        let mut schedule = Schedule::default();
        schedule.add_systems(privateer_trade_escort_system);
        // No contract: an arrival pays no bonus.
        world
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 5_000.0,
                faction: "Independent Merchants".to_string(),
            });
        schedule.run(&mut world);
        assert_eq!(world.resource::<ColonyResources>().credits, 0.0);
        // Active contract: the same arrival pays 1% in real credits.
        assert!(hire_privateers(&mut world));
        world
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 5_000.0,
                faction: "Independent Merchants".to_string(),
            });
        schedule.run(&mut world);
        let state = world.resource::<PrivateerState>();
        assert_eq!(state.convoys_escorted, 1);
        assert!((state.escort_income - 50.0).abs() < 0.01);
        assert!(
            (world.resource::<ColonyResources>().credits - 50.0).abs() < 0.01,
            "escort bonus lands in the real treasury"
        );
    }

    #[test]
    fn test_active_privateers_repel_real_raid_via_bridge() {
        let mut world = base_world(0.0);
        assert!(hire_privateers(&mut world));
        let food_before = world.resource::<ColonyResources>().food;
        world
            .resource_mut::<Events<PirateRaidEvent>>()
            .send(PirateRaidEvent);
        let mut schedule = Schedule::default();
        schedule.add_systems(beacon_pirate_raid_bridge);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<ColonyResources>().food,
            food_before,
            "a protected colony loses nothing to the raid"
        );
        assert_eq!(world.resource::<PrivateerState>().raids_repelled, 1);
    }

    #[test]
    fn test_breached_privateers_no_longer_repel_raids() {
        let mut world = base_world(0.0);
        assert!(hire_privateers(&mut world));
        assert!(refuse_tribute(&mut world));
        // The breach's own raid event is still queued; the bridge will
        // resolve it together with the test raid below — both must
        // steal, since protection is off.
        let food_before = world.resource::<ColonyResources>().food;
        world
            .resource_mut::<Events<PirateRaidEvent>>()
            .send(PirateRaidEvent);
        let mut schedule = Schedule::default();
        schedule.add_systems(beacon_pirate_raid_bridge);
        schedule.run(&mut world);
        assert!(
            world.resource::<ColonyResources>().food < food_before,
            "unprotected, the real bridge steals food as usual"
        );
        assert_eq!(world.resource::<PrivateerState>().raids_repelled, 0);
    }

    #[test]
    fn test_tribute_system_noop_without_contracts() {
        let mut world = base_world(100.0);
        set_tick(&mut world, 10_000);
        run_tribute(&mut world);
        assert_eq!(world.resource::<ColonyResources>().credits, 100.0);
        assert_eq!(world.resource::<PrivateerState>().payments, 0);
        // Bare world (no resources at all) must not panic either.
        let mut bare = World::new();
        bare.insert_resource(PrivateerState::new());
        run_tribute(&mut bare);
    }

    #[test]
    fn test_census_and_stats_label() {
        let mut world = base_world(0.0);
        assert_eq!(privateer_stats_label(&mut world), "none");
        let _fleet = spawn_emissary(&mut world);
        assert_eq!(privateer_stats_label(&mut world), "pending");
        assert!(hire_privateers(&mut world));
        let census = privateer_census(&mut world);
        assert_eq!(census.active, 1);
        assert_eq!(privateer_stats_label(&mut world), "active:paid0/repelled0");
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        let banned = [
            "jack sparrow",
            "black pearl",
            "letter of marque",
            "sid meier",
            "barbossa",
            "davy jones",
            "tortuga",
            "port royal",
        ];
        let haystack = privateer_public_strings().join(" ").to_ascii_lowercase();
        for term in banned {
            assert!(
                !haystack.contains(term),
                "IP guard: public strings must not contain '{term}'"
            );
        }
    }
}
