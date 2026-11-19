//! Spec 431: The Void-Weed Smugglers.
//!
//! A harmless, slightly relaxing native plant — void-weed — turns up in
//! the colony's green corners. Pops cultivate it privately for a tiny
//! mood boost. Passing smugglers discover it is intensely desirable to
//! off-world buyers and pay exorbitant credits for it, turning the
//! colony into an accidental cartel. Every sale raises
//! [`SmugglingHeat`]; at the threshold the heat draws a pirate raid and
//! a diplomatic penalty with the Galactic Council. The player's
//! counterplay (spec §8) is a crackdown: security raids the private
//! stashes, seizes the weed, and sours the whole colony's mood.
//!
//! ## What already existed (the spec's GREEN sketch, landed earlier)
//!
//! [`VoidWeedStash`], [`TradeNetwork`], [`SmugglingHeat`],
//! [`MerchantArrivalEvent`] / [`MerchantType`], [`PirateRaidEvent`], and
//! the two core systems were already in the tree, with the trade/heat
//! systems registered in the Observation schedule, the resources
//! initialized in `setup`, a black-market bridge that broadcasts
//! smuggler arrivals, and a layer-1 bridge that resolves pirate raids
//! into theft + morale loss. But nothing ever *spawned* a stash, so the
//! trade system could never fire in a real game.
//!
//! ## What this landing adds (spec §1/§5/§7/§8, following specs 428–430)
//!
//! * **Organic cultivation (§1, §7).** [`void_weed_cultivation_system`]
//!   gives every pop a tiny deterministic per-tick chance (hash of
//!   entity + tick, no RNG resource — the spec-430 pattern) to start a
//!   private stash, capped colony-wide. The cultivator gets a small
//!   "Void-Weed Calm" mood boost. [`void_weed_growth_system`] grows
//!   every stash slowly toward [`STASH_MAX_AMOUNT`] — the weed spreads
//!   through hydroponics-adjacent corners on its own.
//! * **The accidental cartel (§4/§6).** The trade system now also pays
//!   into the real [`ColonyResources`] credit ledger (when present),
//!   records a [`VoidWeedLedger`], and chronicles each sale. Alien
//!   addiction (§5 REFACTOR) is real: [`AlienAddiction`] rises with
//!   every unit sold and inflates the unit price up to +50% — the
//!   buyers keep coming back, and they pay more. [`AddictionLevel`] is
//!   the per-merchant component form of the same meter.
//! * **Heat consequences (§5 REFACTOR).** When heat trips a raid, the
//!   Galactic Council's [`FactionRelations`] score also drops
//!   ([`COUNCIL_DIPLOMACY_PENALTY`]). Heat and addiction both cool
//!   slowly on their own ([`smuggling_heat_decay_system`]).
//! * **Crackdown (§8).** [`execute_void_weed_crackdown`] is the
//!   world-level counterplay (the `private_stash::execute_search_rooms`
//!   pattern): every stash is seized, heat is halved, and every pop
//!   takes a long "Stash Raided" morale hit — the weed is gone and
//!   nobody forgets the knock on the door.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::pop::Pop;
use crate::layer1::resources::ColonyResources;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::time::SimulationTime;

/// Credits paid per unit of void-weed by a smuggler (spec §3: 10 units
/// must yield exactly 5,000 credits at base addiction).
pub const VOID_WEED_BASE_PRICE: f32 = 500.0;
/// Smuggling heat gained per unit sold.
pub const HEAT_PER_UNIT: f32 = 2.0;
/// Heat at or above this level trips a pirate raid (spec §3).
pub const HEAT_RAID_THRESHOLD: f32 = 100.0;
/// Per-tick heat cooling when no raid is tripped.
pub const HEAT_DECAY_PER_TICK: f32 = 0.02;
/// Alien addiction gained per unit sold.
pub const ADDICTION_PER_UNIT: f32 = 0.5;
/// Addiction ceiling; the price premium caps with it.
pub const ADDICTION_MAX: f32 = 100.0;
/// Per-tick addiction cooling.
pub const ADDICTION_DECAY_PER_TICK: f32 = 0.01;
/// Growth per tick for an established stash.
pub const STASH_GROWTH_PER_TICK: f32 = 0.02;
/// A single stash never grows past this.
pub const STASH_MAX_AMOUNT: f32 = 50.0;
/// Amount in a freshly cultivated stash.
pub const STASH_SEED_AMOUNT: f32 = 1.0;
/// Colony-wide cap on simultaneous private stashes.
pub const MAX_PRIVATE_STASHES: usize = 16;
/// Cultivation roll denominator: each pop has a 1-in-N chance per tick
/// to start cultivating (deterministic hash roll, see
/// [`cultivation_roll`]).
pub const CULTIVATION_DENOMINATOR: u64 = 5_000;
/// Mood label for the cultivator's quiet boost.
pub const CULTIVATION_MOOD_LABEL: &str = "Void-Weed Calm";
/// Mood value of the cultivation boost (spec §1: "a tiny mood boost").
pub const CULTIVATION_MOOD_VALUE: f32 = 0.05;
/// Duration of the cultivation boost, in ticks.
pub const CULTIVATION_MOOD_DURATION: u32 = 600;
/// Mood label applied colony-wide by a crackdown.
pub const CRACKDOWN_MOOD_LABEL: &str = "Stash Raided";
/// Morale hit per pop when stashes are raided (spec §8: "significant
/// localized unrest" — deliberately heavy, like Search Rooms).
pub const CRACKDOWN_MOOD_VALUE: f32 = -0.15;
/// How long crackdown resentment lasts, in ticks.
pub const CRACKDOWN_MOOD_DURATION: u32 = 1_000;
/// Layer-3 faction id of the Galactic Council in [`FactionRelations`]
/// (pirates are 13, nomadic fleets 22 — 431 is this spec's own id).
pub const GALACTIC_COUNCIL_FACTION_ID: u32 = 431;
/// Faction-relations penalty applied to the Council when smuggling
/// heat trips a pirate raid.
pub const COUNCIL_DIPLOMACY_PENALTY: i32 = 10;
/// Chronicle line for the first private cultivation.
pub const DISCOVERY_CHRONICLE: &str =
    "Someone has been quietly cultivating void-weed in a green corner of the colony. It smells faintly of rain on warm stone.";
/// Chronicle line written when a crackdown actually seizes weed.
pub const CRACKDOWN_CHRONICLE: &str =
    "Security raided the private void-weed stashes. The weed is gone — and nobody will forget the knock on the door.";

/// A private stash of void-weed. Standalone stashes are the colony's
/// unowned growth; owned stashes are a pop's private cultivation
/// (spec §5 REFACTOR: hidden private containers).
#[derive(Component, Debug, Clone, Default)]
pub struct VoidWeedStash {
    /// Units of void-weed held.
    pub amount: f32,
    /// The cultivating pop, when this stash is privately owned.
    pub owner: Option<Entity>,
}

impl VoidWeedStash {
    /// An unowned stash of `amount` units.
    pub fn new(amount: f32) -> Self {
        Self {
            amount,
            owner: None,
        }
    }

    /// A private stash of `amount` units cultivated by `owner`.
    pub fn owned(amount: f32, owner: Entity) -> Self {
        Self {
            amount,
            owner: Some(owner),
        }
    }
}

/// The trade ledger the spec sketch used for smuggler payouts. The
/// colony's real credit balance is [`ColonyResources::credits`]; this
/// resource tracks the void-weed trade network's own running total.
#[derive(Resource, Default)]
pub struct TradeNetwork {
    pub credits: f32,
}

/// Colony-wide smuggling heat. Sales raise it; it cools slowly; at
/// [`HEAT_RAID_THRESHOLD`] it trips a pirate raid and resets.
#[derive(Resource, Default)]
pub struct SmugglingHeat {
    pub level: f32,
}

/// Alien buyers' addiction to void-weed (spec §5 REFACTOR). Rises with
/// every unit sold, cools slowly, and inflates the price smugglers pay.
#[derive(Resource, Default)]
pub struct AlienAddiction {
    pub level: f32,
}

impl AlienAddiction {
    /// Unit-price multiplier at the current addiction level: +0.5% per
    /// point of addiction, up to +50% at [`ADDICTION_MAX`].
    pub fn price_multiplier(&self) -> f32 {
        1.0 + self.level.clamp(0.0, ADDICTION_MAX) * 0.005
    }
}

/// Per-merchant addiction meter (spec §5 REFACTOR: an `AddictionLevel`
/// component on alien species passing through the trade network).
#[derive(Component, Debug, Clone, Default)]
pub struct AddictionLevel {
    pub level: f32,
}

/// Running ledger for the whole void-weed economy: what grew, what
/// sold, what it earned, and what the crackdowns seized.
#[derive(Resource, Default)]
pub struct VoidWeedLedger {
    /// Total units grown (cultivation seeds + organic growth).
    pub grown_total: f32,
    /// Total units sold to smugglers.
    pub sold_total: f32,
    /// Total credits earned from smuggler sales.
    pub credits_earned: f32,
    /// Number of completed smuggler sales.
    pub sales: u32,
    /// Number of pirate raids tripped by smuggling heat.
    pub raids_triggered: u32,
    /// Number of crackdowns executed.
    pub crackdowns: u32,
    /// Total units seized across all crackdowns.
    pub seized_total: f32,
    /// Whether the first-cultivation chronicle has been written.
    pub discovery_chronicled: bool,
}

/// A merchant has arrived at the colony.
#[derive(Event)]
pub struct MerchantArrivalEvent {
    pub merchant_type: MerchantType,
}

#[derive(PartialEq, Debug)]
pub enum MerchantType {
    Smuggler,
    Legitimate,
    Enigmatic,
}

/// Pirates raid the colony. The layer-1 raid bridge resolves this into
/// resource theft and a morale hit.
#[derive(Event)]
pub struct PirateRaidEvent;

/// Deterministic cultivation roll (spec-430 pattern: no RNG resource,
/// fully reproducible in tests and replays). Returns true when `pop`
/// starts cultivating on `tick` — roughly one pop in
/// [`CULTIVATION_DENOMINATOR`] per tick.
pub fn cultivation_roll(pop_bits: u64, tick: u64) -> bool {
    let mut h = pop_bits ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h.is_multiple_of(CULTIVATION_DENOMINATOR)
}

/// Total void-weed currently held across every stash in the world
/// (headless STATS / panel helper, in the `smuggler_count` style).
pub fn total_void_weed(world: &mut World) -> f32 {
    let mut query = world.query::<&VoidWeedStash>();
    query.iter(world).map(|s| s.amount).sum()
}

/// Spec §4: smugglers buy every stash at the (addiction-adjusted) unit
/// price. Credits land in both the trade network's ledger and the
/// colony's real credit balance; heat and addiction rise per unit.
#[allow(clippy::too_many_arguments)]
pub fn process_void_weed_trade_system(
    mut events: EventReader<MerchantArrivalEvent>,
    mut network: ResMut<TradeNetwork>,
    mut stashes: Query<&mut VoidWeedStash>,
    mut heat: ResMut<SmugglingHeat>,
    mut colony: Option<ResMut<ColonyResources>>,
    mut addiction: Option<ResMut<AlienAddiction>>,
    mut ledger: Option<ResMut<VoidWeedLedger>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if event.merchant_type != MerchantType::Smuggler {
            continue;
        }
        let unit_price = VOID_WEED_BASE_PRICE
            * addiction
                .as_deref()
                .map(|a| a.price_multiplier())
                .unwrap_or(1.0);
        let mut total = 0.0f32;
        for mut stash in stashes.iter_mut() {
            if stash.amount > 0.0 {
                total += stash.amount;
                stash.amount = 0.0;
            }
        }
        if total <= 0.0 {
            continue;
        }
        let profit = total * unit_price;
        network.credits += profit;
        if let Some(colony) = colony.as_deref_mut() {
            colony.credits += profit;
        }
        heat.level += total * HEAT_PER_UNIT;
        if let Some(addiction) = addiction.as_deref_mut() {
            addiction.level = (addiction.level + total * ADDICTION_PER_UNIT).min(ADDICTION_MAX);
        }
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.sold_total += total;
            ledger.credits_earned += profit;
            ledger.sales += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Smugglers paid {profit:.0} credits for {total:.1} units of void-weed. The colony is becoming an accidental cartel."
            ),
            importance: if profit >= 5_000.0 {
                EventImportance::Major
            } else {
                EventImportance::Standard
            },
        });
    }
}

/// Spec §4: heat at the threshold trips a pirate raid and resets.
/// Spec §5 REFACTOR: the raid also costs the colony standing with the
/// Galactic Council.
pub fn evaluate_smuggling_heat_system(
    mut heat: ResMut<SmugglingHeat>,
    mut raid_events: EventWriter<PirateRaidEvent>,
    mut relations: Option<ResMut<FactionRelations>>,
    mut ledger: Option<ResMut<VoidWeedLedger>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if heat.level >= HEAT_RAID_THRESHOLD {
        raid_events.send(PirateRaidEvent);
        heat.level = 0.0; // Reset heat after raid
        if let Some(relations) = relations.as_deref_mut() {
            let score = relations
                .scores
                .entry(GALACTIC_COUNCIL_FACTION_ID)
                .or_insert(0);
            *score = (*score - COUNCIL_DIPLOMACY_PENALTY).max(-100);
        }
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.raids_triggered += 1;
        }
        chronicle.send(AddChronicleEvent {
            text: "The void-weed trade drew pirate raiders to the colony — and the Galactic Council has taken notice.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Heat and addiction cool slowly when left alone: a quiet colony is
/// gradually forgotten by pirates and buyers alike.
pub fn smuggling_heat_decay_system(
    mut heat: ResMut<SmugglingHeat>,
    mut addiction: Option<ResMut<AlienAddiction>>,
) {
    heat.level = (heat.level - HEAT_DECAY_PER_TICK).max(0.0);
    if let Some(addiction) = addiction.as_deref_mut() {
        addiction.level = (addiction.level - ADDICTION_DECAY_PER_TICK).max(0.0);
    }
}

/// Organic growth (spec §7): every established stash spreads slowly,
/// as a weed does, up to [`STASH_MAX_AMOUNT`].
pub fn void_weed_growth_system(
    mut stashes: Query<&mut VoidWeedStash>,
    mut ledger: Option<ResMut<VoidWeedLedger>>,
) {
    let mut grown = 0.0f32;
    for mut stash in stashes.iter_mut() {
        if stash.amount > 0.0 && stash.amount < STASH_MAX_AMOUNT {
            let delta = STASH_GROWTH_PER_TICK.min(STASH_MAX_AMOUNT - stash.amount);
            stash.amount += delta;
            grown += delta;
        }
    }
    if grown > 0.0 {
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.grown_total += grown;
        }
    }
}

/// Private cultivation (spec §1/§5): pops secretly start growing
/// void-weed in hidden stashes, for a tiny mood boost. Deterministic
/// per-pop per-tick roll; a pop cultivates at most one stash, and the
/// colony-wide stash count is capped.
pub fn void_weed_cultivation_system(
    time: Res<SimulationTime>,
    mut commands: Commands,
    mut pops: Query<(Entity, &mut Morale), With<Pop>>,
    stashes: Query<&VoidWeedStash>,
    mut ledger: Option<ResMut<VoidWeedLedger>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut stash_count = 0usize;
    let mut owners: Vec<Entity> = Vec::new();
    for stash in stashes.iter() {
        stash_count += 1;
        if let Some(owner) = stash.owner {
            owners.push(owner);
        }
    }
    if stash_count >= MAX_PRIVATE_STASHES {
        return;
    }
    for (entity, mut morale) in pops.iter_mut() {
        if owners.contains(&entity) {
            continue;
        }
        if !cultivation_roll(entity.to_bits(), time.tick) {
            continue;
        }
        commands.spawn(VoidWeedStash::owned(STASH_SEED_AMOUNT, entity));
        morale.add_modifier(MoodModifier {
            label: CULTIVATION_MOOD_LABEL.to_string(),
            value: CULTIVATION_MOOD_VALUE,
            duration: CULTIVATION_MOOD_DURATION,
        });
        owners.push(entity);
        stash_count += 1;
        let mut first = false;
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.grown_total += STASH_SEED_AMOUNT;
            if !ledger.discovery_chronicled {
                ledger.discovery_chronicled = true;
                first = true;
            }
        }
        if first {
            chronicle.send(AddChronicleEvent {
                text: DISCOVERY_CHRONICLE.to_string(),
                importance: EventImportance::Standard,
            });
        }
        if stash_count >= MAX_PRIVATE_STASHES {
            break;
        }
    }
}

/// Outcome of a void-weed crackdown (spec §8).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct VoidWeedCrackdownReport {
    /// Stashes that held weed and were seized.
    pub stashes_raided: usize,
    /// Total units seized.
    pub seized: f32,
}

/// Spec §8 counterplay, in the `execute_search_rooms` world-function
/// pattern: security raids every private stash. The weed is seized,
/// smuggling heat is halved (the trade goes quiet), and every pop
/// takes a long morale hit for the knock on the door. A crackdown on
/// an empty colony is a no-op — no resentment without a raid.
pub fn execute_void_weed_crackdown(world: &mut World) -> VoidWeedCrackdownReport {
    // 1. Seize every stash.
    let mut report = VoidWeedCrackdownReport::default();
    {
        let mut query = world.query::<&mut VoidWeedStash>();
        for mut stash in query.iter_mut(world) {
            if stash.amount > 0.0 {
                report.stashes_raided += 1;
                report.seized += stash.amount;
                stash.amount = 0.0;
            }
        }
    }
    if report.stashes_raided == 0 {
        return report;
    }

    // 2. The trade goes quiet: heat halves.
    if world.get_resource::<SmugglingHeat>().is_some() {
        let mut heat = world.resource_mut::<SmugglingHeat>();
        heat.level *= 0.5;
    }
    if world.get_resource::<VoidWeedLedger>().is_some() {
        let mut ledger = world.resource_mut::<VoidWeedLedger>();
        ledger.crackdowns += 1;
        ledger.seized_total += report.seized;
    }

    // 3. Everyone resents the raid.
    {
        let mut query = world.query::<&mut Morale>();
        for mut morale in query.iter_mut(world) {
            morale.add_modifier(MoodModifier {
                label: CRACKDOWN_MOOD_LABEL.to_string(),
                value: CRACKDOWN_MOOD_VALUE,
                duration: CRACKDOWN_MOOD_DURATION,
            });
        }
    }

    // 4. Chronicle, when the event channel exists.
    if world
        .get_resource::<Events<AddChronicleEvent>>()
        .is_some()
    {
        world
            .resource_mut::<Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: CRACKDOWN_CHRONICLE.to_string(),
                importance: EventImportance::Major,
            });
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade_world() -> (World, Schedule) {
        let mut world = World::new();
        world.insert_resource(TradeNetwork { credits: 0.0 });
        world.insert_resource(SmugglingHeat { level: 0.0 });
        world.init_resource::<Events<MerchantArrivalEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        let mut schedule = Schedule::default();
        schedule.add_systems(process_void_weed_trade_system);
        (world, schedule)
    }

    // Spec §3 RED test 1, adapted to the real World/Schedule pattern.
    #[test]
    fn test_void_weed_smuggling_profits() {
        let (mut world, mut schedule) = trade_world();

        let entity = world.spawn((VoidWeedStash::new(10.0),)).id();

        world
            .resource_mut::<Events<MerchantArrivalEvent>>()
            .send(MerchantArrivalEvent {
                merchant_type: MerchantType::Smuggler,
            });

        schedule.run(&mut world);

        let network = world.resource::<TradeNetwork>();
        assert_eq!(network.credits, 5000.0); // 10.0 * 500.0 credits
        let stash = world.get::<VoidWeedStash>(entity).unwrap();
        assert_eq!(stash.amount, 0.0);
    }

    // Spec §3 RED test 2, adapted to the real World/Schedule pattern.
    #[test]
    fn test_void_weed_smuggling_attracts_pirates() {
        let mut world = World::new();
        world.insert_resource(SmugglingHeat { level: 0.0 });
        world.init_resource::<Events<PirateRaidEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();

        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_smuggling_heat_system);

        world.insert_resource(SmugglingHeat { level: 100.0 });

        schedule.run(&mut world);

        let raid_events = world.resource::<Events<PirateRaidEvent>>();
        let mut reader = raid_events.get_cursor();
        let events: Vec<_> = reader.read(raid_events).collect();

        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_legitimate_merchant_does_not_buy() {
        let (mut world, mut schedule) = trade_world();
        let entity = world.spawn((VoidWeedStash::new(10.0),)).id();
        world
            .resource_mut::<Events<MerchantArrivalEvent>>()
            .send(MerchantArrivalEvent {
                merchant_type: MerchantType::Legitimate,
            });
        schedule.run(&mut world);
        assert_eq!(world.resource::<TradeNetwork>().credits, 0.0);
        assert_eq!(world.get::<VoidWeedStash>(entity).unwrap().amount, 10.0);
        assert_eq!(world.resource::<SmugglingHeat>().level, 0.0);
    }

    #[test]
    fn test_sale_pays_colony_credits_and_raises_heat_and_addiction() {
        let (mut world, mut schedule) = trade_world();
        world.insert_resource(ColonyResources {
            credits: 100.0,
            ..Default::default()
        });
        world.insert_resource(AlienAddiction::default());
        world.insert_resource(VoidWeedLedger::default());
        world.spawn((VoidWeedStash::new(10.0),));
        world
            .resource_mut::<Events<MerchantArrivalEvent>>()
            .send(MerchantArrivalEvent {
                merchant_type: MerchantType::Smuggler,
            });
        schedule.run(&mut world);
        // Colony ledger is the real money: 100 + 10 * 500.
        assert_eq!(world.resource::<ColonyResources>().credits, 5100.0);
        assert_eq!(world.resource::<SmugglingHeat>().level, 20.0);
        assert_eq!(world.resource::<AlienAddiction>().level, 5.0);
        let ledger = world.resource::<VoidWeedLedger>();
        assert_eq!(ledger.sold_total, 10.0);
        assert_eq!(ledger.credits_earned, 5000.0);
        assert_eq!(ledger.sales, 1);
    }

    #[test]
    fn test_addiction_raises_smuggler_price() {
        let (mut world, mut schedule) = trade_world();
        // Fully addicted buyers pay +50%.
        world.insert_resource(AlienAddiction {
            level: ADDICTION_MAX,
        });
        world.spawn((VoidWeedStash::new(10.0),));
        world
            .resource_mut::<Events<MerchantArrivalEvent>>()
            .send(MerchantArrivalEvent {
                merchant_type: MerchantType::Smuggler,
            });
        schedule.run(&mut world);
        assert_eq!(world.resource::<TradeNetwork>().credits, 7500.0);
    }

    #[test]
    fn test_growth_spreads_and_caps() {
        let mut world = World::new();
        world.insert_resource(VoidWeedLedger::default());
        let mut schedule = Schedule::default();
        schedule.add_systems(void_weed_growth_system);
        let young = world.spawn((VoidWeedStash::new(1.0),)).id();
        let mature = world
            .spawn((VoidWeedStash::new(STASH_MAX_AMOUNT),))
            .id();
        schedule.run(&mut world);
        let young_amount = world.get::<VoidWeedStash>(young).unwrap().amount;
        assert!((young_amount - (1.0 + STASH_GROWTH_PER_TICK)).abs() < 1e-6);
        assert_eq!(
            world.get::<VoidWeedStash>(mature).unwrap().amount,
            STASH_MAX_AMOUNT
        );
        assert!((world.resource::<VoidWeedLedger>().grown_total - STASH_GROWTH_PER_TICK).abs() < 1e-6);
    }

    #[test]
    fn test_cultivation_spawns_private_stash_and_boosts_mood() {
        let mut world = World::new();
        world.insert_resource(VoidWeedLedger::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        let pop = world.spawn((Pop, Morale::default())).id();
        // Find a tick where this pop's deterministic roll fires.
        let tick = (0..100_000u64)
            .find(|&t| cultivation_roll(pop.to_bits(), t))
            .expect("some tick must trigger cultivation");
        world.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(void_weed_cultivation_system);
        schedule.run(&mut world);

        let mut query = world.query::<&VoidWeedStash>();
        let stashes: Vec<_> = query.iter(&world).collect();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].owner, Some(pop));
        assert_eq!(stashes[0].amount, STASH_SEED_AMOUNT);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == CULTIVATION_MOOD_LABEL && m.value > 0.0));
        assert!(world.resource::<VoidWeedLedger>().discovery_chronicled);
    }

    #[test]
    fn test_heat_and_addiction_decay() {
        let mut world = World::new();
        world.insert_resource(SmugglingHeat { level: 10.0 });
        world.insert_resource(AlienAddiction { level: 10.0 });
        let mut schedule = Schedule::default();
        schedule.add_systems(smuggling_heat_decay_system);
        schedule.run(&mut world);
        assert!(
            (world.resource::<SmugglingHeat>().level - (10.0 - HEAT_DECAY_PER_TICK)).abs() < 1e-6
        );
        assert!(
            (world.resource::<AlienAddiction>().level - (10.0 - ADDICTION_DECAY_PER_TICK)).abs()
                < 1e-6
        );
    }

    #[test]
    fn test_heat_raid_applies_council_diplomacy_penalty() {
        let mut world = World::new();
        world.insert_resource(SmugglingHeat {
            level: HEAT_RAID_THRESHOLD,
        });
        world.insert_resource(FactionRelations::default());
        world.insert_resource(VoidWeedLedger::default());
        world.init_resource::<Events<PirateRaidEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        let mut schedule = Schedule::default();
        schedule.add_systems(evaluate_smuggling_heat_system);
        schedule.run(&mut world);
        let relations = world.resource::<FactionRelations>();
        assert_eq!(
            relations.scores[&GALACTIC_COUNCIL_FACTION_ID],
            -COUNCIL_DIPLOMACY_PENALTY
        );
        assert_eq!(world.resource::<VoidWeedLedger>().raids_triggered, 1);
        assert_eq!(world.resource::<SmugglingHeat>().level, 0.0);
    }

    #[test]
    fn test_crackdown_seizes_stashes_halves_heat_and_sours_morale() {
        let mut world = World::new();
        world.insert_resource(SmugglingHeat { level: 40.0 });
        world.insert_resource(VoidWeedLedger::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        let pop = world.spawn((Pop, Morale::default())).id();
        let stash = world.spawn((VoidWeedStash::owned(12.0, pop),)).id();
        world.spawn((VoidWeedStash::new(8.0),));

        let report = execute_void_weed_crackdown(&mut world);
        assert_eq!(report.stashes_raided, 2);
        assert_eq!(report.seized, 20.0);
        assert_eq!(world.get::<VoidWeedStash>(stash).unwrap().amount, 0.0);
        assert_eq!(world.resource::<SmugglingHeat>().level, 20.0);
        let ledger = world.resource::<VoidWeedLedger>();
        assert_eq!(ledger.crackdowns, 1);
        assert_eq!(ledger.seized_total, 20.0);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == CRACKDOWN_MOOD_LABEL && m.value < 0.0));
    }

    #[test]
    fn test_crackdown_without_stashes_is_noop() {
        let mut world = World::new();
        world.insert_resource(SmugglingHeat { level: 40.0 });
        let pop = world.spawn((Pop, Morale::default())).id();
        let report = execute_void_weed_crackdown(&mut world);
        assert_eq!(report, VoidWeedCrackdownReport::default());
        assert_eq!(world.resource::<SmugglingHeat>().level, 40.0);
        assert!(world.get::<Morale>(pop).unwrap().modifiers.is_empty());
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        let public_strings = [
            DISCOVERY_CHRONICLE,
            CRACKDOWN_CHRONICLE,
            CULTIVATION_MOOD_LABEL,
            CRACKDOWN_MOOD_LABEL,
        ];
        let banned = [
            "spice",
            "melange",
            "arrakis",
            "sandworm",
            "atreides",
            "fremen",
            "mentat",
        ];
        for s in public_strings {
            for b in banned {
                assert!(
                    !s.to_lowercase().contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
