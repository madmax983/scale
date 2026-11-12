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

use super::{ColonyPrices, Wallet};
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
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
        ),
        With<Pop>,
    >,
    prices: Option<Res<ColonyPrices>>,
    mut ledger: Option<ResMut<ScripSystem>>,
) {
    let price = prices.map_or_else(|| ColonyPrices::default().luxury_price, |p| p.luxury_price);
    for (entity, mut needs, mut wallet, mut morale_opt, mut cooldown_opt) in &mut query {
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
        }
        if let Some(cooldown) = cooldown_opt.as_deref_mut() {
            cooldown.ticks_remaining = LUXURY_COOLDOWN_TICKS;
        } else {
            commands.entity(entity).insert(LuxuryCooldown {
                ticks_remaining: LUXURY_COOLDOWN_TICKS,
            });
        }
    }
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
}
