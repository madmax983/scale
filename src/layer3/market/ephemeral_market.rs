//! Ephemeral Markets
//!
//! Describes fleeting pop-up shadow markets that appear temporarily in lawless sectors.
//! They offer rare goods but vanish before authorities can arrive.

use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::prelude::SimulationTime;
use bevy::prelude::*;

/// Represents an Ephemeral Market that spawned in a system.
#[derive(Component)]
pub struct EphemeralMarket {
    /// The location entity.
    pub location: Entity,
    /// Number of ticks until it despawns.
    pub ticks_remaining: u64,
    /// List of offered trades: (Required Resource, Amount, Offered Resource, Amount)
    pub trades: Vec<(ResourceType, f32, ResourceType, f32)>,
}

/// Event triggered to spawn a market.
#[derive(Event)]
pub struct MarketSpawnEvent {
    /// The location entity where it spawns.
    pub system_entity: Entity,
    /// Ticks the market will stay.
    pub duration_ticks: u64,
}

/// Event triggered by player to execute a trade.
#[derive(Event)]
pub struct MarketTradeEvent {
    /// The market entity.
    pub market_entity: Entity,
    /// The buyer entity.
    pub buyer_entity: Entity,
    /// Trade index.
    pub trade_index: usize,
}

/// Event triggered when trade fails.
#[derive(Event)]
pub struct MarketTradeFailedEvent {
    /// The market entity.
    pub market_entity: Entity,
}

/// Spawns ephemeral markets on event and adds a chronicle event.
pub fn spawn_ephemeral_market_system(
    mut commands: Commands,
    mut spawn_events: EventReader<MarketSpawnEvent>,
    mut chronicle: ResMut<Chronicle>,
    time: Res<SimulationTime>,
) {
    for ev in spawn_events.read() {
        commands.spawn(EphemeralMarket {
            location: ev.system_entity,
            ticks_remaining: ev.duration_ticks,
            trades: vec![(ResourceType::Food, 100.0, ResourceType::Scrap, 1.0)],
        });

        chronicle.add_event(
            time.tick,
            "An Ephemeral Market has unexpectedly appeared!".to_string(),
            EventImportance::Major,
        );
    }
}

/// Ticks down the market lifetime and despawns it.
pub fn process_market_despawn_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut EphemeralMarket)>,
) {
    for (entity, mut market) in query.iter_mut() {
        if market.ticks_remaining == 0 {
            commands.entity(entity).despawn();
        } else {
            market.ticks_remaining -= 1;
        }
    }
}

/// Executes trades.
pub fn fulfill_market_trade_system(
    mut trade_events: EventReader<MarketTradeEvent>,
    mut failed_events: EventWriter<MarketTradeFailedEvent>,
    mut market_query: Query<&mut EphemeralMarket>,
    mut resources: ResMut<ColonyResources>,
) {
    for ev in trade_events.read() {
        if let Ok(mut market) = market_query.get_mut(ev.market_entity) {
            if ev.trade_index < market.trades.len() {
                let trade = market.trades.remove(ev.trade_index);

                // Process trade
                let (req_res, req_amount, off_res, off_amount) = trade;

                let has_enough = match req_res {
                    ResourceType::Food => resources.food >= req_amount,
                    ResourceType::Wood => resources.wood >= req_amount,
                    ResourceType::Stone => resources.stone >= req_amount,
                    ResourceType::Ore => resources.ore >= req_amount,
                    ResourceType::Metal => resources.metal >= req_amount,
                    ResourceType::Organs => resources.organs >= req_amount,
                    ResourceType::Planks => resources.planks >= req_amount,
                    ResourceType::Blocks => resources.blocks >= req_amount,
                    ResourceType::Waste => resources.waste >= req_amount,
                    ResourceType::Rations => resources.rations >= req_amount,
                    ResourceType::Fuel => resources.fuel >= req_amount,
                    ResourceType::Alcohol => resources.alcohol >= req_amount,
                    ResourceType::Scrap => resources.scrap >= req_amount,
                    ResourceType::Tools => resources.tools >= req_amount,
                    ResourceType::BuildingPermit => resources.building_permits >= req_amount,
                    ResourceType::MemoryCore => resources.memory_cores >= req_amount,
                    ResourceType::VoidAle
                    | ResourceType::HyperValuable
                    | ResourceType::BiologicalWaste
                    | ResourceType::NutrientPaste => false,
                };

                if has_enough {
                    // Deduct required
                    match req_res {
                        ResourceType::Food => resources.food -= req_amount,
                        ResourceType::Wood => resources.wood -= req_amount,
                        ResourceType::Stone => resources.stone -= req_amount,
                        ResourceType::Ore => resources.ore -= req_amount,
                        ResourceType::Metal => resources.metal -= req_amount,
                        ResourceType::Planks => resources.planks -= req_amount,
                        ResourceType::Blocks => resources.blocks -= req_amount,
                        ResourceType::Waste => resources.waste -= req_amount,
                        ResourceType::Rations => resources.rations -= req_amount,
                        ResourceType::Fuel => resources.fuel -= req_amount,
                        ResourceType::Alcohol => resources.alcohol -= req_amount,
                        ResourceType::Scrap => resources.scrap -= req_amount,
                        ResourceType::Tools => resources.tools -= req_amount,
                        ResourceType::BuildingPermit => resources.building_permits -= req_amount,
                        ResourceType::MemoryCore => resources.memory_cores -= req_amount,
                        ResourceType::VoidAle
                        | ResourceType::HyperValuable
                        | ResourceType::BiologicalWaste
                        | ResourceType::NutrientPaste
                        | ResourceType::Organs => {}
                    }

                    // Add offered
                    match off_res {
                        ResourceType::Food => resources.food += off_amount,
                        ResourceType::Wood => resources.wood += off_amount,
                        ResourceType::Stone => resources.stone += off_amount,
                        ResourceType::Ore => resources.ore += off_amount,
                        ResourceType::Metal => resources.metal += off_amount,
                    ResourceType::Organs => resources.organs += off_amount,
                        ResourceType::Planks => resources.planks += off_amount,
                        ResourceType::Blocks => resources.blocks += off_amount,
                        ResourceType::Waste => resources.waste += off_amount,
                        ResourceType::Rations => resources.rations += off_amount,
                        ResourceType::Fuel => resources.fuel += off_amount,
                        ResourceType::Alcohol => resources.alcohol += off_amount,
                        ResourceType::Scrap => resources.scrap += off_amount,
                        ResourceType::Tools => resources.tools += off_amount,
                        ResourceType::BuildingPermit => resources.building_permits += off_amount,
                        ResourceType::MemoryCore => resources.memory_cores += off_amount,
                        ResourceType::VoidAle
                        | ResourceType::HyperValuable
                        | ResourceType::BiologicalWaste
                        | ResourceType::NutrientPaste => {}
                    }
                } else {
                    failed_events.send(MarketTradeFailedEvent {
                        market_entity: ev.market_entity,
                    });
                    // Need to put the trade back since it failed
                    market.trades.insert(ev.trade_index, trade);
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn setup_app() -> App {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                spawn_ephemeral_market_system,
                process_market_despawn_system,
                fulfill_market_trade_system,
            ),
        );
        app.add_event::<MarketSpawnEvent>();
        app.add_event::<MarketTradeEvent>();
        app.add_event::<MarketTradeFailedEvent>();
        app.insert_resource(crate::layer1::chronicle::Chronicle::default());
        app.insert_resource(crate::prelude::SimulationTime::default());
        app.insert_resource(ColonyResources::default());
        app
    }

    #[test]
    fn test_ephemeral_market_spawns_and_despawns() {
        let mut app = setup_app();

        // Trigger spawn event
        let target_system = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(MarketSpawnEvent {
            system_entity: target_system,
            duration_ticks: 10,
        });

        app.update();

        // Check market spawned
        let mut query = app.world_mut().query::<(Entity, &EphemeralMarket)>();
        let market_count = query.iter(app.world()).count();
        assert_eq!(
            market_count, 1,
            "Ephemeral Market should spawn when event is sent"
        );

        let (market_ent, market) = query.iter(app.world()).next().unwrap();
        assert_eq!(market.ticks_remaining, 10);
        assert_eq!(market.location, target_system);

        // Run ticks to trigger despawn
        for _ in 0..11 {
            app.update();
        }

        // Market should be despawned
        assert!(
            app.world().get_entity(market_ent).is_err(),
            "Market should despawn after ticks_remaining reaches 0"
        );
    }

    #[test]
    fn test_market_trade_fulfillment() {
        let mut app = setup_app();

        let target_system = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(MarketSpawnEvent {
            system_entity: target_system,
            duration_ticks: 100,
        });
        app.update();

        let market_ent = app
            .world_mut()
            .query_filtered::<Entity, With<EphemeralMarket>>()
            .single(app.world());

        app.insert_resource(ColonyResources {
            food: 100.0,
            ..Default::default()
        });

        // Attempt Trade
        app.world_mut().send_event(MarketTradeEvent {
            market_entity: market_ent,
            buyer_entity: target_system,
            trade_index: 0,
        });

        app.update();

        let resources = app.world().resource::<ColonyResources>();
        assert_eq!(resources.food, 0.0);
        assert_eq!(resources.scrap, 1.0);
    }

    #[test]
    fn test_process_market_despawn_system() {
        let mut app = setup_app();

        let target_system = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(MarketSpawnEvent {
            system_entity: target_system,
            duration_ticks: 2,
        });

        app.update(); // Spawns market with 2 ticks remaining

        let market_ent = app
            .world_mut()
            .query_filtered::<Entity, With<EphemeralMarket>>()
            .single(app.world());

        app.update();
        assert!(app.world().get_entity(market_ent).is_ok());
        assert_eq!(
            app.world()
                .get::<EphemeralMarket>(market_ent)
                .unwrap()
                .ticks_remaining,
            1
        );

        app.update();
        assert!(app.world().get_entity(market_ent).is_ok());
        assert_eq!(
            app.world()
                .get::<EphemeralMarket>(market_ent)
                .unwrap()
                .ticks_remaining,
            0
        );

        app.update();
        assert!(app.world().get_entity(market_ent).is_err());
    }

    #[test]
    fn test_market_trade_fulfillment_failure() {
        let mut app = setup_app();

        let target_system = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(MarketSpawnEvent {
            system_entity: target_system,
            duration_ticks: 100,
        });
        app.update();

        let market_ent = app
            .world_mut()
            .query_filtered::<Entity, With<EphemeralMarket>>()
            .single(app.world());

        app.insert_resource(ColonyResources {
            food: 10.0, // Needs 100
            ..Default::default()
        });

        app.world_mut().send_event(MarketTradeEvent {
            market_entity: market_ent,
            buyer_entity: target_system,
            trade_index: 0,
        });

        app.update();

        let resources = app.world().resource::<ColonyResources>();
        assert_eq!(resources.food, 10.0);
        assert_eq!(resources.scrap, 0.0);

        let failed_events = app
            .world()
            .resource::<bevy_ecs::event::Events<MarketTradeFailedEvent>>();
        let mut reader = failed_events.get_cursor();
        assert_eq!(reader.read(failed_events).count(), 1);

        let market = app.world().get::<EphemeralMarket>(market_ent).unwrap();
        assert_eq!(market.trades.len(), 1);
    }
}
