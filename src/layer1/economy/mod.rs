//! Economy module.
use crate::layer1::actions::AssignmentType;
use bevy_ecs::prelude::*;

/// Represents a Pop's personal funds.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Wallet {
    /// Current credit balance.
    pub credits: f32,
}

/// Global price configuration for the colony economy.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ColonyPrices {
    /// Cost of a meal.
    pub food_price: f32,
    /// Cost of luxury items.
    pub luxury_price: f32,
}

impl Default for ColonyPrices {
    fn default() -> Self {
        Self {
            food_price: 1.0,
            luxury_price: 5.0,
        }
    }
}

/// Returns the standard wage per task for a given job type.
#[must_use]
pub const fn get_wage_for_job(job_type: AssignmentType) -> f32 {
    match job_type {
        AssignmentType::FarmWorker => 1.5,
        AssignmentType::LibraryWorker => 3.0,
        AssignmentType::Administrator => 2.5,
        _ => 1.0,
    }
}

/// Adds credits to a worker's wallet.
pub fn pay_wage(world: &mut World, worker: Entity, amount: f32) {
    if let Some(faction) = world.get::<crate::layer1::social::factions::FactionMember>(worker) {
        if faction.faction_id == Some(crate::layer1::social::factions::FactionId::Stateless) {
            return; // Stateless pops don't get paid normal wages / contribute taxes
        }
    }
    if let Some(mut wallet) = world.get_mut::<Wallet>(worker) {
        wallet.credits += amount;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::resources::ColonyResources;

    /// Helper system for economy tests.
    fn consume_food_with_payment_system(world: &mut World) {
        let price = world.resource::<ColonyPrices>().food_price;
        let food_avail = world.resource::<ColonyResources>().food;
        if food_avail < 1.0 {
            return;
        }

        let mut query = world.query::<(Entity, &mut Needs, &mut Wallet)>();
        let mut transactions = Vec::new();

        for (entity, needs, wallet) in query.iter(world) {
            if needs.hunger < 0.7 && wallet.credits >= price {
                transactions.push(entity);
            }
        }

        for entity in transactions {
            let mut food_res = world.resource_mut::<ColonyResources>();
            if food_res.food >= 1.0 {
                food_res.food -= 1.0;
            } else {
                break;
            }

            if let Some(mut wallet) = world.get_mut::<Wallet>(entity) {
                wallet.credits -= price;
            }
            if let Some(mut needs) = world.get_mut::<Needs>(entity) {
                needs.hunger = (needs.hunger + 0.3).min(1.0);
            }
        }
    }

    #[test]
    fn test_wallet_default() {
        let wallet = Wallet::default();
        assert_eq!(wallet.credits, 0.0);
    }

    #[test]
    fn test_pay_wages_system() {
        let mut world = World::new();
        // Spawn a pop with a wallet
        let pop = world.spawn((Pop, Wallet { credits: 10.0 })).id();

        // Simulate job completion event
        pay_wage(&mut world, pop, 5.0);

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 15.0);
    }

    #[test]
    fn test_purchase_food_success() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 10.0 },
                Needs {
                    hunger: 0.5,
                    rest: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        world.insert_resource(ColonyPrices {
            food_price: 2.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });

        // Call modified consumption system
        consume_food_with_payment_system(&mut world);

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 8.0, "Should deduct 2.0 credits");

        let needs = world.get::<Needs>(pop).unwrap();
        assert!(needs.hunger > 0.5, "Should have eaten");
    }

    #[test]
    fn test_purchase_food_fail_poverty() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 1.0 }, // Not enough!
                Needs {
                    hunger: 0.1,
                    rest: 0.5,
                    ..Default::default()
                }, // Starving
            ))
            .id();

        world.insert_resource(ColonyPrices {
            food_price: 2.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });

        consume_food_with_payment_system(&mut world);

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 1.0, "Should not deduct if purchase failed");

        let needs = world.get::<Needs>(pop).unwrap();
        assert_eq!(needs.hunger, 0.1, "Should NOT have eaten (too poor)");
    }

    #[test]
    fn test_job_wage_configuration() {
        // Ensure jobs have a wage associated
        let job = AssignmentType::FarmWorker;
        assert!(get_wage_for_job(job) > 0.0);
    }

    #[test]
    fn test_pay_wage_stateless_faction_returns_early() {
        let mut world = World::new();
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

        // Should not have increased due to early return
        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 10.0);
    }

    #[test]
    fn test_consume_food_with_payment_system_low_food_returns_early() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Wallet { credits: 10.0 },
                Needs {
                    hunger: 0.1, // Starving
                    ..Default::default()
                },
            ))
            .id();

        world.insert_resource(ColonyPrices {
            food_price: 2.0,
            ..Default::default()
        });
        world.insert_resource(ColonyResources {
            food: 0.5, // Less than 1.0!
            ..Default::default()
        });

        // System should early return
        consume_food_with_payment_system(&mut world);

        let wallet = world.get::<Wallet>(pop).unwrap();
        assert_eq!(wallet.credits, 10.0, "Should not deduct credits");

        let needs = world.get::<Needs>(pop).unwrap();
        assert_eq!(needs.hunger, 0.1, "Should NOT have eaten");
    }
}

pub mod apex_diet;
/// Black market interactions.
pub mod black_market;
/// Hauling and transportation of resources.
pub mod work_efficiency;
pub use work_efficiency::*;
pub mod hauling;
/// Inventory management for pops and entities.
pub mod inventory;
/// In-game items and their properties.
pub mod items;
/// Photophobic Resources (Spec 1106)
pub mod photophobic;
/// Refining raw resources into usable goods.
pub mod refining;
/// Financial remittances between pops.
pub mod remittances;
/// Global resource tracking.
pub mod resources;
/// Shadow market operations and special merchants.
pub mod shadow_market;
/// Physical storage of resources.
pub mod stockpile;
/// Regular trade with off-world merchants.
pub mod trade;

pub use black_market::*;
pub use hauling::*;
pub use inventory::*;
pub use items::*;
pub use photophobic::*;
pub use refining::*;
pub use remittances::*;
pub use resources::*;
pub use shadow_market::*;
pub use stockpile::*;
pub use trade::*;

/// Colony distress beacon mechanics.
pub mod beacon;
pub use beacon::*;
pub mod smugglers_cove;
pub use smugglers_cove::*;
pub mod inflation;
pub mod recall;
pub use inflation::*;
pub mod ideological_contraband;
pub use ideological_contraband::*;

pub mod existential_audit;
pub use existential_audit::*;

pub mod deep_sleep_syndicates;
pub use deep_sleep_syndicates::*;
pub mod bone_economy;
pub mod debt_of_the_dead;
pub use bone_economy::*;
pub mod information_black_market;
pub use information_black_market::*;

pub mod bio_loom;
pub use bio_loom::*;

pub mod biomass_dividend;
pub use biomass_dividend::*;
