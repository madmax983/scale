//! Trade system for the colony.
//!
//! This module manages visiting merchants, trade deals, and the exchange of resources.
//!
//! # The Trade Cycle
//!
//! 1.  **Requirement**: A [`TradeDepot`] building must exist in the colony.
//! 2.  **Arrival**: Every few ticks, if the cooldown has expired, a new [`Merchant`] arrives
//!     (handled by [`merchant_arrival_system`]).
//! 3.  **Deals**: The merchant brings a random set of [`TradeDeal`]s (e.g., "Give 10 Wood for 5 Metal").
//! 4.  **Trading**: The player (or AI) calls [`execute_trade`] to accept a deal.
//! 5.  **Departure**: After a set duration, the merchant leaves, and a cooldown begins before the next one arrives.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::economy::curio_trade::{is_curio_resource, TradeCompletedEvent};
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;
use std::collections::HashMap;

/// Status of a resource in the market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarketStatus {
    /// Legal to trade and possess.
    #[default]
    Legal,
    /// Banned by edict.
    Contraband,
}

/// Market data for a specific resource.
#[derive(Debug, Clone)]
pub struct MarketItem {
    /// Base price (currently unused by dynamic merchant, but tracked).
    pub base_price: f32,
    /// Legal status of the item.
    pub status: MarketStatus,
}

/// Persistent market state tracking resource status and prices.
#[derive(Resource, Default, Debug, Clone)]
pub struct TradeMarket {
    /// Map of resource types to their market data.
    pub items: HashMap<ResourceType, MarketItem>,
}

/// Component marker for the Trade Depot building.
///
/// Merchants will only visit if at least one Trade Depot exists.
#[derive(Component, Default)]
pub struct TradeDepot;

/// Represents a single trade offer from a merchant.
#[derive(Debug, Clone)]
pub struct TradeDeal {
    /// The resource required to purchase.
    pub cost_resource: ResourceType,
    /// The amount required.
    pub cost_amount: f32,
    /// The resource given in return.
    pub give_resource: ResourceType,
    /// The amount given.
    pub give_amount: f32,
}

/// A visiting merchant with a set of deals.
#[derive(Debug, Clone)]
pub struct Merchant {
    /// The merchant's name.
    pub name: String,
    /// The simulation tick when they arrived.
    pub arrival_tick: u64,
    /// The simulation tick when they will depart.
    pub departure_tick: u64,
    /// The deals they offer.
    pub deals: Vec<TradeDeal>,
}

/// Tracks the state of merchant visits.
#[derive(Resource, Default)]
pub struct MerchantState {
    /// The currently present merchant, if any.
    pub active_merchant: Option<Merchant>,
    /// The tick until which no new merchant can arrive.
    pub cooldown: u64,
}

/// System to handle merchant arrivals and departures.
///
/// Checks if a Trade Depot exists and if the cooldown has expired.
/// If so, spawns a new merchant with random deals.
/// Also handles departure of active merchants.
#[allow(clippy::collapsible_if)]
pub fn merchant_arrival_system(world: &mut World) {
    let current_tick = world.resource::<SimulationTime>().tick;

    // Check if depot exists
    let depot_exists = world
        .query::<(&Building, &TradeDepot)>()
        .iter(world)
        .any(|(b, _)| b.building_type == BuildingType::TradeDepot);

    let (should_depart, should_arrive) = {
        let state = world.resource::<MerchantState>();
        let depart = state
            .active_merchant
            .as_ref()
            .is_some_and(|m| current_tick >= m.departure_tick);
        let arrive =
            state.active_merchant.is_none() && current_tick >= state.cooldown && depot_exists;
        (depart, arrive)
    };

    let mut event_to_log: Option<(String, EventImportance)> = None;

    if should_depart {
        let mut state = world.resource_mut::<MerchantState>();
        if let Some(merchant) = state.active_merchant.take() {
            let name = merchant.name;
            event_to_log = Some((
                format!("Merchant {name} has departed."),
                EventImportance::Standard,
            ));
        }
        state.cooldown = current_tick + 2000;
    } else if should_arrive {
        // ⚡ Bolt Optimization:
        // Getting an immutable reference to the `TradeMarket` resource prevents cloning
        // the entire HashMap of market items every time a merchant arrives.
        // This removes an O(N) heap allocation, using zero-cost abstraction for read-only access.
        let market_items = world.get_resource::<TradeMarket>().map(|m| &m.items);

        let mut rng = rand::thread_rng();

        // Randomize merchant name
        let names = ["Wanderer", "Caravan", "Trader", "Merchant", "Peddler"];
        let name = format!(
            "{} {}",
            names[rng.gen_range(0..names.len())],
            rng.gen_range(100..999)
        );

        // Generate deals
        let mut deals = Vec::new();
        for _ in 0..rng.gen_range(1..4) {
            let cost_type = match rng.gen_range(0..3) {
                0 => ResourceType::Wood,
                1 => ResourceType::Stone,
                _ => ResourceType::Food,
            };

            let give_type = match rng.gen_range(0..3) {
                0 => ResourceType::Metal,
                1 => ResourceType::Planks,
                _ => ResourceType::Ore,
            };

            let mut cost_amount = rng.gen_range(5.0f32..20.0f32).round();
            let mut give_amount = rng.gen_range(2.0f32..10.0f32).round();

            // Apply Contraband modifiers if Market exists
            if let Some(items) = market_items {
                // Buying Contraband (receiving it) -> Costs more
                if let Some(item) = items.get(&give_type) {
                    if item.status == MarketStatus::Contraband {
                        cost_amount *= 2.0;
                    }
                }
                // Selling Contraband (giving it) -> Earns more (Risk premium)
                if let Some(item) = items.get(&cost_type) {
                    if item.status == MarketStatus::Contraband {
                        give_amount *= 2.0;
                    }
                }
            }

            deals.push(TradeDeal {
                cost_resource: cost_type,
                cost_amount,
                give_resource: give_type,
                give_amount,
            });
        }

        // Spec 334: Core World elites occasionally send a buyer specifically
        // for the colony's curios (memory cores) - the curio trade.
        if rng.gen_bool(0.3) {
            let give_type = match rng.gen_range(0..3) {
                0 => ResourceType::Metal,
                1 => ResourceType::Planks,
                _ => ResourceType::Ore,
            };
            deals.push(TradeDeal {
                cost_resource: ResourceType::MemoryCore,
                cost_amount: rng.gen_range(1.0f32..4.0f32).round(),
                give_resource: give_type,
                give_amount: rng.gen_range(2.0f32..8.0f32).round(),
            });
        }

        let merchant = Merchant {
            name: name.clone(),
            arrival_tick: current_tick,
            departure_tick: current_tick + 500,
            deals,
        };

        event_to_log = Some((
            format!("Merchant {name} has arrived."),
            EventImportance::Major,
        ));

        let mut state = world.resource_mut::<MerchantState>();
        state.active_merchant = Some(merchant);
    }

    // Log events if any occurred
    if let Some((msg, importance)) = event_to_log {
        if let Some(mut chronicle) = world.get_resource_mut::<Chronicle>() {
            chronicle.add_event(current_tick, msg, importance);
        }
    }
}

/// Executes a trade deal, exchanging resources if affordable.
///
/// Returns `true` if the trade was successful.
///
/// # Examples
///
/// ```
/// use scale::layer1::trade::{execute_trade, TradeDeal};
/// use scale::layer1::resources::{ColonyResources, ResourceType};
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
///
/// // Setup resources
/// world.insert_resource(ColonyResources {
///     wood: 100.0,
///     metal: 0.0,
///     ..Default::default()
/// });
///
/// // Define a deal: 10 Wood -> 5 Metal
/// let deal = TradeDeal {
///     cost_resource: ResourceType::Wood,
///     cost_amount: 10.0,
///     give_resource: ResourceType::Metal,
///     give_amount: 5.0,
/// };
///
/// // Execute
/// let success = execute_trade(&mut world, &deal);
///
/// assert!(success);
///
/// // Verify exchange
/// let res = world.resource::<ColonyResources>();
/// assert_eq!(res.wood, 90.0);
/// assert_eq!(res.metal, 5.0);
/// ```
pub fn execute_trade(world: &mut World, deal: &TradeDeal) -> bool {
    let is_hoarding = world
        .query::<&crate::layer1::cassandra_protocol::ResourceHoarding>()
        .iter(world)
        .next()
        .is_some();
    if is_hoarding {
        return false;
    }

    // Validate inputs (security hardening)
    if deal.cost_amount < 0.0 || !deal.cost_amount.is_finite() {
        return false;
    }
    if deal.give_amount < 0.0 || !deal.give_amount.is_finite() {
        return false;
    }

    let mut resources = world.resource_mut::<ColonyResources>();

    if matches!(
        deal.cost_resource,
        ResourceType::HyperValuable | ResourceType::BiologicalWaste | ResourceType::NutrientPaste
    ) {
        return false;
    }

    // Check affordability and Deduct
    if !resources.try_consume(deal.cost_resource, deal.cost_amount) {
        return false;
    }

    // Add
    resources.add_resource(&deal.give_resource, deal.give_amount);

    // Spec 334: announce completed trades so curio exports (memory cores sold
    // to Core World elites) can pay their credit premium and inflict cultural
    // loss. `init_resource` keeps this safe on bare worlds (doctests).
    world.init_resource::<Events<TradeCompletedEvent>>();
    world.send_event(TradeCompletedEvent {
        resource: deal.cost_resource,
        is_curio: is_curio_resource(deal.cost_resource),
        amount: deal.cost_amount,
    });

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::chronicle::Chronicle;
    use crate::layer1::resources::{ColonyResources, ResourceType};
    use crate::shared::time::SimulationTime;

    #[test]
    fn test_trade_depot_component_exists() {
        let depot = TradeDepot;
        let _ = depot;
    }

    #[test]
    fn test_merchant_state_resource_defaults() {
        let state = MerchantState::default();
        assert!(state.active_merchant.is_none());
        assert!(state.cooldown == 0); // Default is 0
    }

    #[test]
    fn test_merchant_arrival_requires_depot() {
        let mut world = World::new();
        world.insert_resource(MerchantState::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Chronicle::default());
        // No TradeDepot spawned

        merchant_arrival_system(&mut world);

        let state = world.resource::<MerchantState>();
        assert!(state.active_merchant.is_none());
    }

    #[test]
    fn test_merchant_arrival_with_depot() {
        let mut world = World::new();
        // Set cooldown to 0 to force arrival check
        world.insert_resource(MerchantState {
            cooldown: 0,
            ..Default::default()
        });
        world.insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        world.insert_resource(Chronicle::default());

        // Spawn Trade Depot
        world.spawn((
            Building {
                building_type: BuildingType::TradeDepot,
            },
            TradeDepot,
        ));

        merchant_arrival_system(&mut world);

        let state = world.resource::<MerchantState>();
        assert!(state.active_merchant.is_some());

        let merchant = state.active_merchant.as_ref().unwrap();
        assert!(merchant.departure_tick > 100);
        assert!(!merchant.deals.is_empty());
    }

    #[test]
    fn test_merchant_departs_after_time() {
        let mut world = World::new();
        // Setup state with active merchant
        let merchant = Merchant {
            name: "Leaver".to_string(),
            arrival_tick: 0,
            departure_tick: 100,
            deals: vec![],
        };
        world.insert_resource(MerchantState {
            active_merchant: Some(merchant),
            cooldown: 0,
        });

        // Advance time past departure
        world.insert_resource(SimulationTime {
            tick: 101,
            ..Default::default()
        });
        world.insert_resource(Chronicle::default());

        // Depot exists
        world.spawn((
            Building {
                building_type: BuildingType::TradeDepot,
            },
            TradeDepot,
        ));

        merchant_arrival_system(&mut world);

        let state = world.resource::<MerchantState>();
        assert!(state.active_merchant.is_none());
        assert!(state.cooldown > 101);

        // Check log
        let chronicle = world.resource::<Chronicle>();
        assert!(!chronicle.events.is_empty());
        assert!(chronicle.events[0].text.contains("has departed"));
    }

    #[test]
    fn test_execute_trade_fails_when_hoarding() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 50.0,
            metal: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);
        world.spawn(crate::layer1::cassandra_protocol::ResourceHoarding);

        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: 10.0,
            give_resource: ResourceType::Metal,
            give_amount: 5.0,
        };

        let success = execute_trade(&mut world, &deal);

        assert!(!success);
    }

    #[test]
    fn test_execute_trade_success() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 50.0,
            metal: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: 10.0,
            give_resource: ResourceType::Metal,
            give_amount: 5.0,
        };

        let success = execute_trade(&mut world, &deal);

        assert!(success);
        let res = world.resource::<ColonyResources>();
        assert!((res.wood - 40.0).abs() < f32::EPSILON);
        assert!((res.metal - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_execute_trade_insufficient_funds() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 5.0, // Need 10
            ..Default::default()
        };
        world.insert_resource(resources);

        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: 10.0,
            give_resource: ResourceType::Metal,
            give_amount: 5.0,
        };

        let success = execute_trade(&mut world, &deal);

        assert!(!success);
        let res = world.resource::<ColonyResources>();
        assert!((res.wood - 5.0).abs() < f32::EPSILON);
        assert!((res.metal - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_exploit_negative_cost_prevented() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 100.0,
            metal: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        // Malicious deal: "Pay" -50 wood (gain 50 wood) to get 10 metal
        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: -50.0,
            give_resource: ResourceType::Metal,
            give_amount: 10.0,
        };

        // Should be rejected by validation
        let success = execute_trade(&mut world, &deal);

        assert!(!success, "Trade with negative cost should fail");

        let res = world.resource::<ColonyResources>();
        // Resources should remain unchanged
        assert!((res.wood - 100.0).abs() < f32::EPSILON);
        assert!((res.metal - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_exploit_infinite_cost_prevented() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 100.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: f32::INFINITY,
            give_resource: ResourceType::Metal,
            give_amount: 10.0,
        };

        let success = execute_trade(&mut world, &deal);

        assert!(!success, "Trade with infinite cost should fail");
    }

    #[test]
    fn test_execute_trade_emits_curio_completed_event() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            memory_cores: 5.0,
            ..Default::default()
        });
        let deal = TradeDeal {
            cost_resource: ResourceType::MemoryCore,
            cost_amount: 2.0,
            give_resource: ResourceType::Metal,
            give_amount: 3.0,
        };
        assert!(execute_trade(&mut world, &deal));
        let events = world
            .get_resource::<Events<TradeCompletedEvent>>()
            .expect("execute_trade should register TradeCompletedEvent");
        let mut iter = events.iter_current_update_events();
        let first = iter
            .next()
            .expect("execute_trade should emit TradeCompletedEvent");
        assert!(first.is_curio, "memory core exports are curio exports");
        assert_eq!(first.amount, 2.0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_execute_trade_emits_non_curio_completed_event() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            wood: 100.0,
            ..Default::default()
        });
        let deal = TradeDeal {
            cost_resource: ResourceType::Wood,
            cost_amount: 10.0,
            give_resource: ResourceType::Metal,
            give_amount: 5.0,
        };
        assert!(execute_trade(&mut world, &deal));
        let events = world
            .get_resource::<Events<TradeCompletedEvent>>()
            .expect("execute_trade should register TradeCompletedEvent");
        let mut iter = events.iter_current_update_events();
        let first = iter
            .next()
            .expect("execute_trade should emit TradeCompletedEvent");
        assert!(!first.is_curio, "wood exports are not curio exports");
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_curio_deals_appear_for_merchants() {
        // RNG-driven deal generation: loop until a curio buyer is observed
        // (established repo pattern for unmockable RNG, cf. hobby.rs).
        let mut saw_curio_deal = false;
        for _ in 0..50 {
            let mut world = World::new();
            world.insert_resource(MerchantState::default());
            world.insert_resource(SimulationTime::default());
            world.insert_resource(Chronicle::default());
            world.spawn((
                Building {
                    building_type: BuildingType::TradeDepot,
                },
                TradeDepot,
            ));
            merchant_arrival_system(&mut world);
            if world
                .resource::<MerchantState>()
                .active_merchant
                .as_ref()
                .is_some_and(|m| {
                    m.deals
                        .iter()
                        .any(|d| d.cost_resource == ResourceType::MemoryCore)
                })
            {
                saw_curio_deal = true;
                break;
            }
        }
        assert!(saw_curio_deal, "merchants should sometimes bring curio buyers");
    }
}