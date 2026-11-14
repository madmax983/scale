//! Company Scrip (Spec 415): the colony's internal wage-and-price loop.
//!
//! Grounding (audited 2026-11-11 against trunk 31ec2c20): the spec's
//! `Wallet { balance: u32 }`, `Store`, `JobEvent`, `pay_wages_system`,
//! and `PurchaseAttemptEvent` are MOCKS. The real seams already exist:
//!
//! * [`Wallet`](super::Wallet) is `Wallet { credits: f32 }` and every
//!   `PopBundle` starts with 50.0 credits.
//! * [`ColonyPrices`](super::ColonyPrices) is the real price book
//!   (food 1.0, luxury 5.0) — there is no second `Store`.
//! * There is no `JobEvent`: work completion is detected inside
//!   `general_work` (`check_work_completion`) which pays through
//!   [`pay_wage`](super::pay_wage) at [`get_wage_for_job`] rates, and
//!   `produce_food_system` pays farm wages inline.
//! * Food is already bought: `consume_food_system` refuses pops whose
//!   wallet cannot cover `food_price` and deducts the price on eating.
//!
//! The genuinely missing legs this module builds, bridged onto those
//! seams and nothing else:
//!
//! 1. **Luxury purchases.** `ColonyPrices::luxury_price` was priced,
//!    repriced (debt-of-the-dead), and resented (dissatisfaction's
//!    RelativeDeprivation / ScripSqueeze), but no system ever charged
//!    it. [`purchase_luxury_system`] is that cash register: a bored pop
//!    with enough scrip buys a small luxury, topping up leisure and
//!    morale; a broke pop gets nothing. The free tavern/beauty leisure
//!    tier is untouched — luxury is the paid tier above it.
//! 2. **The scrip ledger.** [`ScripSystem`] counts wages actually paid
//!    (hooked into `pay_wage` and the farm wage path, after the
//!    Stateless refusal and the corruption skim, so it records money
//!    that really reached a wallet) and luxury sales, and carries a
//!    per-tick census snapshot (totals, richest/poorest, broke count)
//!    maintained by [`scrip_census_system`] for headless `STATS` /
//!    the `scrip` inspect command. This is what makes the spec's
//!    emergence — high-skill pops living like kings while haulers
//!    starve — observable instead of anecdotal.
//!
//! 3. **Chronicle telegraphing (Item 202).** The loop above was
//!    Chronicle-silent end to end — zero wage/purchase lines, the only
//!    tell was STATS `scrip=`/`broke=`/`luxury_sales=`. Now the first
//!    luxury purchase emits one Minor Chronicle line naming the
//!    purchase ([`first_luxury_chronicle_text`], first-per-colony via
//!    `ScripSystem::chronicled_first_luxury`, the black-market
//!    first-docking pattern), and [`scrip_ledger_chronicle_system`]
//!    emits a periodic Minor ledger summary naming wages paid /
//!    luxury sales totals on the [`TICKS_PER_YEAR`] cadence (the
//!    seniority graduation-check / cadet allowance interval), only
//!    when the totals moved since the last summary — spam-capped by
//!    construction. Text wiring only: no wage, price, cooldown,
//!    leisure/morale, or endowment constant changed.

use super::{ColonyPrices, Wallet};
use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::needs::Needs;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Leisure below this a pop goes shopping for a luxury. Sits between
/// the "bored/stressed" line in `Needs` docs (0.2) and contentment:
/// pops shop while merely under-entertained, not only when desperate.
pub const LUXURY_LEISURE_THRESHOLD: f32 = 0.5;
/// Leisure restored by one purchased luxury.
pub const LUXURY_LEISURE_GAIN: f32 = 0.4;
/// Morale mood value granted by a purchased luxury.
pub const LUXURY_MOOD_VALUE: f32 = 0.10;
/// Morale mood duration (ticks) granted by a purchased luxury.
pub const LUXURY_MOOD_DURATION: u32 = 600;
/// Minimum ticks between one pop's luxury purchases, so a rich pop
/// cannot convert its whole wallet into leisure in a single tick.
pub const LUXURY_COOLDOWN_TICKS: u32 = 500;
/// Display label for the purchased-luxury mood modifier.
pub const LUXURY_MOOD_LABEL: &str = "Small Luxuries";
/// Cadence (ticks) for the periodic scrip ledger Chronicle summary
/// (Item 202): one summary per year — `TICKS_PER_YEAR`, the same
/// interval the seniority graduation check and the cadet allowance
/// already run on. Spam-capped by construction: at most one summary
/// line per interval, and only when the ledger totals moved.
pub const SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS: u64 = TICKS_PER_YEAR;

/// Chronicle line for the colony's FIRST luxury purchase (Item 202):
/// names the buyer when known, the purchase, and the price actually
/// paid (read from `ColonyPrices::luxury_price` at purchase time —
/// never hardcoded, so repricing stays truthful in the text).
#[must_use]
pub fn first_luxury_chronicle_text(buyer: Option<&str>, price: f32) -> String {
    match buyer {
        Some(name) => format!(
            "{name} has bought a small luxury for {price:.0} credits — the first luxury sale in the colony's scrip ledger."
        ),
        None => format!(
            "A colonist has bought a small luxury for {price:.0} credits — the first luxury sale in the colony's scrip ledger."
        ),
    }
}

/// Chronicle line for the periodic scrip ledger summary (Item 202):
/// names the cumulative wages paid and luxury sales totals from the
/// real [`ScripSystem`] ledger.
#[must_use]
pub fn scrip_ledger_chronicle_text(wages_paid: f32, luxury_sales: u32, luxury_spent: f32) -> String {
    format!(
        "Scrip ledger: {wages_paid:.0} credits paid in wages so far; {luxury_sales} luxury sale(s) totalling {luxury_spent:.0} credits."
    )
}

/// Player-facing strings for this mechanic (IP-guard tested):
/// original, generic wording only.
#[must_use]
pub fn scrip_public_strings() -> Vec<String> {
    vec![
        LUXURY_MOOD_LABEL.to_string(),
        "small luxury".to_string(),
        "Scrip ledger".to_string(),
        first_luxury_chronicle_text(None, 5.0),
        scrip_ledger_chronicle_text(0.0, 0, 0.0),
    ]
}

/// Ledger + census for the colony scrip loop (Spec 415).
///
/// Counters are cumulative for the life of the world; the census
/// fields are a snapshot rewritten every tick by
/// [`scrip_census_system`].
#[derive(Resource, Default, Debug, Clone)]
pub struct ScripSystem {
    /// Total scrip actually paid into wallets as wages (post-refusal,
    /// post-skim — see `pay_wage`).
    pub wages_paid: f32,
    /// Number of luxury purchases completed.
    pub luxury_sales: u32,
    /// Total scrip spent on luxuries.
    pub luxury_spent: f32,
    /// Census: sum of all pop wallet balances.
    pub total_credits: f32,
    /// Census: largest single wallet.
    pub richest: f32,
    /// Census: smallest single wallet.
    pub poorest: f32,
    /// Census: pops whose wallet cannot cover one meal at the current
    /// food price — the starvation-risk line.
    pub broke_count: u32,
    /// Census: pops carrying a wallet.
    pub wallet_count: u32,
    /// Item 202: whether the first-luxury-purchase Chronicle line has
    /// been written (first-per-colony, black-market first-docking
    /// pattern — set when the sale lands, never re-arms).
    pub chronicled_first_luxury: bool,
    /// Item 202: tick of the last ledger-summary cadence check that
    /// advanced the summary clock (anchor for the spam cap).
    pub last_ledger_summary_tick: u64,
    /// Item 202: `wages_paid` as of the last emitted ledger summary
    /// (summaries fire only when the totals moved past these).
    pub last_summary_wages_paid: f32,
    /// Item 202: `luxury_sales` as of the last emitted ledger summary.
    pub last_summary_luxury_sales: u32,
}

/// Per-pop cooldown between luxury purchases.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct LuxuryCooldown {
    /// Ticks until this pop may buy another luxury.
    pub ticks_remaining: u32,
}

/// Pops with low leisure and enough scrip buy a luxury at
/// `ColonyPrices::luxury_price`: credits are deducted, leisure and
/// morale rise, and the sale lands in the [`ScripSystem`] ledger.
/// Pops who cannot afford the price buy nothing (the free leisure
/// tier — taverns, beauty — is a separate, untouched system).
#[allow(clippy::type_complexity)]
pub fn purchase_luxury_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &mut Needs,
            &mut Wallet,
            Option<&mut Morale>,
            Option<&mut LuxuryCooldown>,
            Option<&PopName>,
        ),
        With<Pop>,
    >,
    prices: Option<Res<ColonyPrices>>,
    mut ledger: Option<ResMut<ScripSystem>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let price = prices.map_or_else(|| ColonyPrices::default().luxury_price, |p| p.luxury_price);
    // Item 202: the colony's first luxury sale is chronicled once
    // (Minor, naming the purchase). Collected here and sent after the
    // loop so the ledger flag — not event timing — is the spam cap.
    let mut first_purchase_text: Option<String> = None;
    for (entity, mut needs, mut wallet, mut morale_opt, mut cooldown_opt, name_opt) in &mut query {
        if let Some(cooldown) = cooldown_opt.as_deref_mut() {
            if cooldown.ticks_remaining > 0 {
                cooldown.ticks_remaining -= 1;
                continue;
            }
        }
        if needs.leisure >= LUXURY_LEISURE_THRESHOLD || wallet.credits < price {
            continue;
        }
        wallet.credits -= price;
        needs.leisure = (needs.leisure + LUXURY_LEISURE_GAIN).min(1.0);
        if let Some(morale) = morale_opt.as_deref_mut() {
            morale.add_modifier(MoodModifier {
                label: LUXURY_MOOD_LABEL.to_string(),
                value: LUXURY_MOOD_VALUE,
                duration: LUXURY_MOOD_DURATION,
            });
        }
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.luxury_sales += 1;
            ledger.luxury_spent += price;
            if !ledger.chronicled_first_luxury {
                ledger.chronicled_first_luxury = true;
                first_purchase_text = Some(first_luxury_chronicle_text(
                    name_opt.map(|n| n.0.as_str()),
                    price,
                ));
            }
        }
        if let Some(cooldown) = cooldown_opt.as_deref_mut() {
            cooldown.ticks_remaining = LUXURY_COOLDOWN_TICKS;
        } else {
            commands.entity(entity).insert(LuxuryCooldown {
                ticks_remaining: LUXURY_COOLDOWN_TICKS,
            });
        }
    }
    // Optional-chronicle idiom (cryo/tourism pattern): harnesses
    // without the event resource simply skip the line.
    if let Some(text) = first_purchase_text {
        if let Some(events) = chronicle.as_deref_mut() {
            events.send(AddChronicleEvent {
                text,
                importance: EventImportance::Minor,
            });
        }
    }
}

/// Periodic scrip ledger Chronicle summary (Item 202): every
/// [`SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS`] ticks, if wages or luxury
/// sales moved since the last summary, emit ONE Minor line naming the
/// cumulative totals. The cadence anchor advances on every cadence
/// hit (even silent ones), so summaries stay on the year grid and can
/// never fire more than once per interval — spam-capped by
/// construction. Silent when the ledger is idle or the chronicle
/// event resource is absent (the anchor is NOT advanced while the
/// chronicle is absent, so a pending summary is not lost).
pub fn scrip_ledger_chronicle_system(
    time: Option<Res<SimulationTime>>,
    mut ledger: Option<ResMut<ScripSystem>>,
    mut chronicle: Option<ResMut<Events<AddChronicleEvent>>>,
) {
    let Some(time) = time else { return };
    let Some(ledger) = ledger.as_deref_mut() else {
        return;
    };
    let Some(events) = chronicle.as_deref_mut() else {
        return;
    };
    let tick = time.tick;
    if tick == 0 || tick < ledger.last_ledger_summary_tick + SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS {
        return;
    }
    ledger.last_ledger_summary_tick = tick;
    let moved = (ledger.wages_paid - ledger.last_summary_wages_paid).abs() > f32::EPSILON
        || ledger.luxury_sales != ledger.last_summary_luxury_sales;
    if !moved || (ledger.wages_paid <= 0.0 && ledger.luxury_sales == 0) {
        return;
    }
    ledger.last_summary_wages_paid = ledger.wages_paid;
    ledger.last_summary_luxury_sales = ledger.luxury_sales;
    events.send(AddChronicleEvent {
        text: scrip_ledger_chronicle_text(
            ledger.wages_paid,
            ledger.luxury_sales,
            ledger.luxury_spent,
        ),
        importance: EventImportance::Minor,
    });
}

/// Rewrites the census snapshot in [`ScripSystem`]: wallet totals,
/// richest/poorest, and how many pops cannot afford one meal.
pub fn scrip_census_system(
    query: Query<&Wallet, With<Pop>>,
    prices: Option<Res<ColonyPrices>>,
    mut ledger: Option<ResMut<ScripSystem>>,
) {
    let Some(ledger) = ledger.as_deref_mut() else {
        return;
    };
    let food_price = prices.map_or_else(|| ColonyPrices::default().food_price, |p| p.food_price);
    let mut total = 0.0f32;
    let mut richest = 0.0f32;
    let mut poorest = f32::MAX;
    let mut broke = 0u32;
    let mut count = 0u32;
    for wallet in &query {
        total += wallet.credits;
        richest = richest.max(wallet.credits);
        poorest = poorest.min(wallet.credits);
        if wallet.credits < food_price {
            broke += 1;
        }
        count += 1;
    }
    ledger.total_credits = total;
    ledger.richest = richest;
    ledger.poorest = if count > 0 { poorest } else { 0.0 };
    ledger.broke_count = broke;
    ledger.wallet_count = count;
}

/// Headless census helper (the `coredrills`/`tethers` pattern):
/// returns `(total_credits, broke_count, luxury_sales)` read live
/// from the world — wallet sums computed on the spot, sales from the
/// ledger when present.
#[must_use]
pub fn scrip_stats(world: &mut World) -> (f32, u32, u32) {
    let food_price = world
        .get_resource::<ColonyPrices>()
        .map_or_else(|| ColonyPrices::default().food_price, |p| p.food_price);
    let mut total = 0.0f32;
    let mut broke = 0u32;
    let mut query = world.query_filtered::<&Wallet, With<Pop>>();
    for wallet in query.iter(world) {
        total += wallet.credits;
        if wallet.credits < food_price {
            broke += 1;
        }
    }
    let luxury_sales = world
        .get_resource::<ScripSystem>()
        .map_or(0, |s| s.luxury_sales);
    (total, broke, luxury_sales)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::actions::AssignmentType;
    use crate::layer1::economy::{get_wage_for_job, pay_wage};
    use crate::layer1::resources::ColonyResources;
    use bevy_ecs::system::RunSystemOnce;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<ScripSystem>();
        world.insert_resource(ColonyPrices::default());
        world
    }

    // --- Spec RED test 1, adapted to the real seams -----------------
    // Spec mock: JobEvent::Completed { JobType::Mining } -> Wallet
    // { balance: u32 } += 10. Reality: no JobEvent exists; completion
    // pays pay_wage(world, pop, get_wage_for_job(AssignmentType)).
    #[test]
    fn test_pop_earns_scrip_from_work() {
        let mut world = test_world();
        let pop = world.spawn((Pop, Wallet { credits: 0.0 })).id();

        // Skill-tiered pay is the real wage table (spec emergence:
        // high-skill pops live like kings).
        let wage = get_wage_for_job(AssignmentType::LibraryWorker);
        assert_eq!(wage, 3.0);
        assert!(get_wage_for_job(AssignmentType::FarmWorker) > 0.0);
        assert!(wage > get_wage_for_job(AssignmentType::FarmWorker));

        pay_wage(&mut world, pop, wage);

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 3.0, "wage should land in the wallet");
        // The ledger records what actually reached the wallet.
        assert_eq!(world.resource::<ScripSystem>().wages_paid, 3.0);
    }

    #[test]
    fn test_pay_wage_ledger_skips_stateless_refusal() {
        // pay_wage refuses Stateless pops; the ledger must not count
        // money that never moved.
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 10.0 },
                crate::layer1::social::factions::FactionMember {
                    faction_id: Some(crate::layer1::social::factions::FactionId::Stateless),
                },
            ))
            .id();
        pay_wage(&mut world, pop, 5.0);
        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 10.0);
        assert_eq!(world.resource::<ScripSystem>().wages_paid, 0.0);
    }

    // --- Spec RED test 2, adapted -----------------------------------
    // Spec mock: PurchaseAttemptEvent + Store price 5 -> hunger 1.0.
    // Reality: the real consume_food_system IS the purchase system —
    // it gates on ColonyPrices::food_price and pays from the wallet.
    #[test]
    fn test_pop_spends_scrip_for_food() {
        let mut world = World::new();
        world.insert_resource(ColonyPrices {
            food_price: 2.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 10.0 },
                Needs {
                    hunger: 0.1,
                    ..Default::default()
                },
            ))
            .id();

        world
            .run_system_once(crate::layer1::agriculture::farm::consume_food_system)
            .unwrap();

        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 8.0);
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            needs.hunger > 0.1,
            "hunger should rise after a paid meal, got {}",
            needs.hunger
        );
    }

    // --- Spec RED test 3, adapted -----------------------------------
    #[test]
    fn test_pop_starves_without_scrip() {
        let mut world = World::new();
        world.insert_resource(ColonyPrices {
            food_price: 2.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 1.0 }, // Cannot afford the 2.0 meal.
                Needs {
                    hunger: 0.1,
                    ..Default::default()
                },
            ))
            .id();

        world
            .run_system_once(crate::layer1::agriculture::farm::consume_food_system)
            .unwrap();

        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 1.0);
        assert_eq!(world.get::<Needs>(pop).unwrap().hunger, 0.1);
        assert_eq!(world.resource::<ColonyResources>().food, 10.0);
    }

    // --- New leg: luxury purchases ----------------------------------
    #[test]
    fn test_pop_buys_luxury_with_scrip() {
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 10.0 },
                Needs {
                    leisure: 0.1,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();

        world.run_system_once(purchase_luxury_system).unwrap();

        // Default luxury price is 5.0.
        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 5.0);
        let needs = world.get::<Needs>(pop).unwrap();
        assert!((needs.leisure - (0.1 + LUXURY_LEISURE_GAIN)).abs() < 1e-6);
        let morale = world.get::<Morale>(pop).unwrap();
        assert_eq!(morale.modifiers.len(), 1);
        assert_eq!(morale.modifiers[0].label, LUXURY_MOOD_LABEL);
        let ledger = world.resource::<ScripSystem>();
        assert_eq!(ledger.luxury_sales, 1);
        assert_eq!(ledger.luxury_spent, 5.0);
    }

    #[test]
    fn test_pop_cannot_buy_luxury_without_scrip() {
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 1.0 },
                Needs {
                    leisure: 0.1,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(purchase_luxury_system).unwrap();

        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 1.0);
        assert_eq!(world.get::<Needs>(pop).unwrap().leisure, 0.1);
        assert_eq!(world.resource::<ScripSystem>().luxury_sales, 0);
    }

    #[test]
    fn test_content_pop_does_not_buy_luxury() {
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 100.0 },
                Needs {
                    leisure: 0.9,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(purchase_luxury_system).unwrap();

        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 100.0);
        assert_eq!(world.resource::<ScripSystem>().luxury_sales, 0);
    }

    #[test]
    fn test_luxury_cooldown_prevents_repeat_purchase() {
        let mut world = test_world();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 100.0 },
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(purchase_luxury_system).unwrap();
        world.run_system_once(purchase_luxury_system).unwrap();
        // Second run: leisure is back under the threshold after the
        // first gain decays in this synthetic world? No — leisure is
        // 0.4 after one purchase, still under 0.5, so only the
        // cooldown can explain a single sale.
        assert_eq!(world.resource::<ScripSystem>().luxury_sales, 1);
        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 95.0);

        // Cooldown expiry re-opens the till.
        world
            .get_mut::<LuxuryCooldown>(pop)
            .unwrap()
            .ticks_remaining = 0;
        world.run_system_once(purchase_luxury_system).unwrap();
        assert_eq!(world.resource::<ScripSystem>().luxury_sales, 2);
        assert_eq!(world.get::<Wallet>(pop).unwrap().credits, 90.0);
    }

    // --- New leg: census --------------------------------------------
    #[test]
    fn test_scrip_census_counts_broke_and_totals() {
        let mut world = test_world();
        world.spawn((Pop, Wallet { credits: 0.5 }));
        world.spawn((Pop, Wallet { credits: 50.0 }));
        world.spawn((Pop, Wallet { credits: 5.0 }));

        world.run_system_once(scrip_census_system).unwrap();

        let ledger = world.resource::<ScripSystem>();
        assert_eq!(ledger.wallet_count, 3);
        assert!((ledger.total_credits - 55.5).abs() < 1e-6);
        assert_eq!(ledger.richest, 50.0);
        assert_eq!(ledger.poorest, 0.5);
        // Default food price 1.0: only the 0.5 wallet cannot eat.
        assert_eq!(ledger.broke_count, 1);
    }

    #[test]
    fn test_scrip_stats_helper_matches_census() {
        let mut world = test_world();
        world.spawn((Pop, Wallet { credits: 0.5 }));
        world.spawn((Pop, Wallet { credits: 50.0 }));
        let (total, broke, sales) = scrip_stats(&mut world);
        assert!((total - 50.5).abs() < 1e-6);
        assert_eq!(broke, 1);
        assert_eq!(sales, 0);
    }

    // --- IP guard ----------------------------------------------------
    #[test]
    fn test_scrip_public_strings_are_original() {
        // Concept-only IP rule: the company-store *mechanic* is fair
        // game; distinctive names/lines from any specific work are not.
        let banned = [
            "sixteen tons",
            "tennessee ernie",
            "saint peter",
            "company store owes",
        ];
        let label = LUXURY_MOOD_LABEL.to_lowercase();
        for term in banned {
            assert!(
                !label.contains(term),
                "public label must not contain banned term: {term}"
            );
        }
        assert_eq!(LUXURY_MOOD_LABEL, "Small Luxuries");
    }

    // --- Item 202: Scrip Chronicle telegraphing (RED) ----------------
    // Company Scrip was Chronicle-silent end to end: zero wage/purchase
    // lines. (a) The first luxury purchase emits ONE Minor line naming
    // the purchase; (b) a periodic, spam-capped ledger summary names
    // wages paid / luxury sales totals on the TICKS_PER_YEAR cadence
    // (the seniority graduation-check / cadet allowance interval).

    fn chronicle_texts(world: &mut World) -> Vec<(String, crate::layer1::chronicle::EventImportance)> {
        let events = world
            .resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        reader
            .read(events)
            .map(|e| (e.text.clone(), e.importance))
            .collect()
    }

    fn world_with_chronicle() -> World {
        let mut world = test_world();
        world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world
    }

    #[test]
    fn test_first_luxury_text_names_purchase_and_price() {
        let named = first_luxury_chronicle_text(Some("Ada"), 5.0);
        assert!(
            named.contains("Ada"),
            "first-purchase line must name the buyer when known: {named}"
        );
        assert!(
            named.to_lowercase().contains("luxury"),
            "first-purchase line must name the purchase: {named}"
        );
        assert!(
            named.contains('5'),
            "first-purchase line must name the price paid: {named}"
        );
        let anon = first_luxury_chronicle_text(None, 5.0);
        assert!(
            anon.to_lowercase().contains("luxury") && anon.contains('5'),
            "anonymous first-purchase line must still name purchase + price: {anon}"
        );
    }

    #[test]
    fn test_first_luxury_purchase_writes_minor_chronicle_once() {
        let mut world = world_with_chronicle();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 100.0 },
                Needs {
                    leisure: 0.0,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();

        world.run_system_once(purchase_luxury_system).unwrap();
        let texts = chronicle_texts(&mut world);
        assert_eq!(
            texts.len(),
            1,
            "first luxury purchase must emit exactly one chronicle line, got: {texts:?}"
        );
        assert_eq!(
            texts[0].1,
            crate::layer1::chronicle::EventImportance::Minor,
            "first luxury purchase line must be Minor"
        );
        assert!(
            texts[0].0.to_lowercase().contains("luxury"),
            "line must name the purchase: {}",
            texts[0].0
        );

        // Second purchase (cooldown cleared) must NOT re-chronicle:
        // first-per-colony, spam-safe by construction.
        world
            .get_mut::<LuxuryCooldown>(pop)
            .unwrap()
            .ticks_remaining = 0;
        if let Some(mut needs) = world.get_mut::<Needs>(pop) {
            needs.leisure = 0.0;
        }
        world.run_system_once(purchase_luxury_system).unwrap();
        assert_eq!(world.resource::<ScripSystem>().luxury_sales, 2);
        let texts = chronicle_texts(&mut world);
        assert_eq!(
            texts.len(),
            1,
            "only the FIRST purchase is chronicled, got: {texts:?}"
        );
    }

    #[test]
    fn test_no_purchase_no_first_luxury_chronicle() {
        let mut world = world_with_chronicle();
        world.spawn((
            Pop,
            Wallet { credits: 1.0 },
            Needs {
                leisure: 0.0,
                ..Default::default()
            },
        ));
        world.run_system_once(purchase_luxury_system).unwrap();
        assert!(chronicle_texts(&mut world).is_empty());
    }

    #[test]
    fn test_ledger_summary_text_names_wages_and_sales() {
        let text = scrip_ledger_chronicle_text(123.0, 4, 20.0);
        assert!(
            text.contains("123"),
            "summary must name wages paid: {text}"
        );
        assert!(
            text.contains('4') && text.to_lowercase().contains("luxury"),
            "summary must name luxury sales: {text}"
        );
        assert!(
            text.contains("20"),
            "summary must name luxury credits spent: {text}"
        );
    }

    #[test]
    fn test_ledger_summary_fires_on_cadence_and_is_spam_capped() {
        let mut world = world_with_chronicle();
        {
            let mut ledger = world.resource_mut::<ScripSystem>();
            ledger.wages_paid = 50.0;
            ledger.luxury_sales = 2;
            ledger.luxury_spent = 10.0;
        }
        let run_at = |world: &mut World, tick: u64| {
            world.resource_mut::<crate::shared::time::SimulationTime>().tick = tick;
            world
                .run_system_once(scrip_ledger_chronicle_system)
                .unwrap();
        };

        // Before the first cadence tick: silent.
        run_at(&mut world, SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS - 1);
        assert!(chronicle_texts(&mut world).is_empty());

        // On cadence: exactly one Minor summary naming the totals.
        run_at(&mut world, SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS);
        let texts = chronicle_texts(&mut world);
        assert_eq!(texts.len(), 1, "cadence summary must fire once: {texts:?}");
        assert_eq!(
            texts[0].1,
            crate::layer1::chronicle::EventImportance::Minor
        );
        assert!(texts[0].0.contains("50"), "summary names wages: {}", texts[0].0);
        assert!(texts[0].0.contains('2'), "summary names sales: {}", texts[0].0);

        // Every tick until the next cadence: still silent (spam cap).
        for tick in SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS + 1
            ..SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS + 10
        {
            run_at(&mut world, tick);
        }
        // Next cadence with NO new activity: silent (nothing to report).
        run_at(&mut world, SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS * 2);
        assert_eq!(
            chronicle_texts(&mut world).len(),
            1,
            "no new activity, no repeat summary"
        );

        // New activity + next cadence: a fresh summary with new totals.
        world.resource_mut::<ScripSystem>().wages_paid = 80.0;
        run_at(&mut world, SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS * 3);
        let texts = chronicle_texts(&mut world);
        assert_eq!(texts.len(), 2, "new activity re-arms the summary: {texts:?}");
        assert!(texts[1].0.contains("80"), "updated wages named: {}", texts[1].0);
    }

    #[test]
    fn test_ledger_summary_silent_when_no_activity() {
        let mut world = world_with_chronicle();
        world.resource_mut::<crate::shared::time::SimulationTime>().tick =
            SCRIP_LEDGER_SUMMARY_INTERVAL_TICKS;
        world
            .run_system_once(scrip_ledger_chronicle_system)
            .unwrap();
        assert!(
            chronicle_texts(&mut world).is_empty(),
            "an idle ledger must not spam the chronicle"
        );
    }

    #[test]
    fn test_ip_guard_scrip_chronicle_texts() {
        let banned = [
            "sixteen tons",
            "tennessee ernie",
            "saint peter",
            "company store owes",
            "i owe my soul",
        ];
        let mut strings = scrip_public_strings();
        strings.push(first_luxury_chronicle_text(Some("Ada"), 5.0));
        strings.push(first_luxury_chronicle_text(None, 5.0));
        strings.push(scrip_ledger_chronicle_text(10.0, 1, 5.0));
        for s in &strings {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string must not contain banned term '{term}': {s}"
                );
            }
        }
    }
}
