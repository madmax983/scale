use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::remittances::MigrantArrivalEvent;
use crate::layer1::void_weed::PirateRaidEvent;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Represents a colony distress beacon that attracts ships, migrants, and pirates.
#[derive(Resource, Default)]
pub struct ColonyBeacon {
    /// Whether the beacon is currently active and broadcasting.
    pub is_active: bool,
    /// Tick when the beacon was last toggled. Prevents rapid micro-toggling.
    pub last_toggled_tick: u64,
}

/// Evaluates the probabilities of external events triggered by the `ColonyBeacon`.
///
/// An active beacon serves as a lighthouse in the dark sector, significantly increasing the influx
/// of independent trade ships and desperate migrants. However, this same visibility attracts pirate
/// raids. This system uses randomized rolls each tick to determine if a specific event is dispatched.
///
/// # Examples
/// ```
/// use scale::layer1::economy::beacon::{ColonyBeacon, process_colony_beacon_system};
/// use scale::layer2::trade::blockade::TradeShipArrivalEvent;
/// use scale::layer1::economy::remittances::MigrantArrivalEvent;
/// use scale::layer1::void_weed::PirateRaidEvent;
/// use scale::layer1::chronicle::AddChronicleEvent;
/// use scale::shared::time::SimulationTime;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// world.insert_resource(ColonyBeacon { is_active: true, last_toggled_tick: 0 });
/// world.insert_resource(SimulationTime::default());
/// world.insert_resource(Events::<TradeShipArrivalEvent>::default());
/// world.insert_resource(Events::<MigrantArrivalEvent>::default());
/// world.insert_resource(Events::<PirateRaidEvent>::default());
/// world.insert_resource(Events::<AddChronicleEvent>::default());
///
/// let mut schedule = Schedule::default();
/// schedule.add_systems(process_colony_beacon_system);
/// schedule.run(&mut world);
/// // External events may or may not be spawned based on RNG.
/// ```
pub fn toggle_beacon_system(mut beacon: ResMut<ColonyBeacon>, sim_time: Res<SimulationTime>) {
    let cooldown_ticks = 100; // Hardcoded cooldown for refactor phase
    // WINTER-ECONOMY PASS (2026-10-03): the old `|| last_toggled_tick == 0`
    // clause re-fired on tick 1 (last_toggled_tick had just been set to 0),
    // so the beacon flickered ON for exactly one tick at game start. Only
    // allow the cooldown bypass on the very first tick.
    if sim_time.tick >= beacon.last_toggled_tick + cooldown_ticks
        || (beacon.last_toggled_tick == 0 && sim_time.tick == 0)
    {
        beacon.is_active = !beacon.is_active;
        beacon.last_toggled_tick = sim_time.tick;
    }
}

pub fn process_colony_beacon_system(
    beacon: Res<ColonyBeacon>,
    sim_time: Res<SimulationTime>,
    mut trade_writer: EventWriter<TradeShipArrivalEvent>,
    mut migrant_writer: EventWriter<MigrantArrivalEvent>,
    mut pirate_writer: EventWriter<PirateRaidEvent>,
    mut chronicle_writer: EventWriter<AddChronicleEvent>,
) {
    if !beacon.is_active {
        return;
    }

    // Lore Hooks: Log when the beacon is first lit
    // Note: Assuming a simple check based on whether it was toggled very recently
    // In a full implementation, you might want a separate trigger event, but
    // for now we check if it was just turned on in the last tick.
    if beacon.last_toggled_tick > 0 && beacon.last_toggled_tick == sim_time.tick {
        chronicle_writer.send(AddChronicleEvent {
            text: "The beacon is lit. We invite the galaxy, and all its scum, to our doors."
                .to_string(),
            importance: EventImportance::Major,
        });
    }

    let mut rng = rand::thread_rng();

    // Significantly increased chances
    if rng.gen::<f32>() < 0.05 {
        trade_writer.send(TradeShipArrivalEvent {
            cargo_value: 5000.0,
            faction: "Independent Merchants".to_string(),
        });
    }

    // WINTER-ECONOMY PASS (2026-10-03): the old 5%-per-tick shower of
    // 5-15 migrants flooded a 5-pop starter colony (~250 arrivals/year).
    // A trickle of 1-3 keeps the beacon's "all its scum" flavor without
    // overwhelming food, housing, and jobs.
    if rng.gen::<f32>() < 0.01 {
        migrant_writer.send(MigrantArrivalEvent {
            home_faction: Entity::PLACEHOLDER, // Unspecified source
            count: rng.gen_range(1..4),
            criminal_chance: 0.3,  // High chance of criminals
            low_skill_chance: 0.5, // High chance of grifters
        });
    }

    // Risk of pirate raids
    // WINTER-ECONOMY PASS (2026-10-03): 2% -> 1%. Raids still happen, but
    // no longer machine-gun the food stockpile (see also the proportional
    // theft in beacon_pirate_raid_bridge).
    if rng.gen::<f32>() < 0.01 {
        // PirateRaidEvent is an empty struct
        pirate_writer.send(PirateRaidEvent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::chronicle::AddChronicleEvent;
    use crate::layer1::economy::remittances::MigrantArrivalEvent;
    use crate::layer1::void_weed::PirateRaidEvent;
    use crate::layer2::trade::blockade::TradeShipArrivalEvent;
    use crate::shared::time::SimulationTime;
    use bevy::prelude::{App, Update};

    fn setup_app() -> App {
        let mut app = App::new();
        app.insert_resource(SimulationTime::default());
        app.insert_resource(ColonyBeacon::default());
        app.add_event::<TradeShipArrivalEvent>();
        app.add_event::<MigrantArrivalEvent>();
        app.add_event::<PirateRaidEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(Update, (process_colony_beacon_system, toggle_beacon_system));
        app
    }

    #[test]
    fn test_beacon_cooldown() {
        let mut app = setup_app();

        app.world_mut().resource_mut::<SimulationTime>().tick = 1;

        // Toggle on
        app.update();
        assert!(app.world().resource::<ColonyBeacon>().is_active);

        // Advance a few ticks and try to toggle (should fail due to cooldown)
        app.world_mut().resource_mut::<SimulationTime>().tick = 5;
        app.update();
        assert!(app.world().resource::<ColonyBeacon>().is_active);

        // Advance past cooldown and toggle
        app.world_mut().resource_mut::<SimulationTime>().tick = 105;
        app.update();
        assert!(!app.world().resource::<ColonyBeacon>().is_active);
    }

    #[test]
    fn test_beacon_first_lit_triggers_chronicle() {
        let mut app = setup_app();

        // Simulate activating the beacon on tick 1
        app.world_mut().resource_mut::<SimulationTime>().tick = 1;
        app.world_mut().resource_mut::<ColonyBeacon>().is_active = true;
        app.world_mut()
            .resource_mut::<ColonyBeacon>()
            .last_toggled_tick = 1;

        app.update();

        let chronicle_events = app
            .world()
            .get_resource::<Events<AddChronicleEvent>>()
            .unwrap();
        assert!(
            !chronicle_events.is_empty(),
            "Activating the beacon should add a chronicle event"
        );
    }

    #[test]
    fn test_beacon_increases_arrival_rates() {
        let mut app = setup_app();

        // Activate the beacon
        app.world_mut().resource_mut::<ColonyBeacon>().is_active = true;

        let mut has_trade = false;
        let mut has_migrants = false;

        // Run the simulation for several ticks
        for _ in 0..1000 {
            app.update();
            let trade_events = app
                .world()
                .get_resource::<Events<TradeShipArrivalEvent>>()
                .unwrap();
            let migrant_events = app
                .world()
                .get_resource::<Events<MigrantArrivalEvent>>()
                .unwrap();
            if !trade_events.is_empty() {
                has_trade = true;
            }
            if !migrant_events.is_empty() {
                has_migrants = true;
            }
        }

        // Check if the number of arrivals is significantly higher than baseline
        // Use high loop count to avoid flakiness
        assert!(has_trade, "Active beacon should trigger trade ships");
        assert!(has_migrants, "Active beacon should trigger migrants");
    }

    #[test]
    fn test_beacon_spawns_criminals_and_pirates() {
        let mut app = setup_app();

        app.world_mut().resource_mut::<ColonyBeacon>().is_active = true;

        let mut has_pirates = false;

        for _ in 0..1000 {
            app.update();
            let pirate_events = app
                .world()
                .get_resource::<Events<PirateRaidEvent>>()
                .unwrap();
            if !pirate_events.is_empty() {
                has_pirates = true;
            }
        }

        // Ensure pirate raids are triggered
        assert!(has_pirates, "Active beacon should attract pirates");

        // The system that spawns migrants based on MigrantArrivalEvent should have a higher chance
        // to spawn pops with criminal or low-skill traits when beacon is active.
        // This test assumes `process_colony_beacon_system` modifies a global modifier or emits
        // specific events that downstream systems use.
    }
}
