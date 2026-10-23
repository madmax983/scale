//! Item 125 RED tests for the cable heat-buildup telegraph.
//!
//! These run against the LIVE system: every tick calls the real
//! `evaluate_grid_load_system` (the same system the Economy schedule
//! runs, after `power_grid_system`) via `run_system_once`, then the
//! real `chronicle_event_handler_system`, so the warnings asserted
//! here are the ones a live colony would record.
//! Demand 150 vs. cable capacity 100 is a 1.5x overload — squarely in
//! the 1x–2x sustained band: heat builds every tick, but the 2.0x
//! instant-ignition threshold is never crossed, so no fire can race
//! the telegraph in these tests.

use super::cable_heat::{cable_heat_episode_active, cable_heat_ticks, CableHeatTracker};
use super::load_limits::{evaluate_grid_load_system, PowerCable};
use crate::layer1::chronicle::{chronicle_event_handler_system, Chronicle, EventImportance};
use crate::layer1::energy::PowerConsumer;
use crate::layer1::map::GridPosition;
use crate::layer1::temperature::TemperatureGrid;
use crate::shared::time::SimulationTime;
use bevy_ecs::event::Events;
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;

fn cable_world(demand: f32, capacity: f32) -> (World, Schedule, Entity) {
    let mut world = World::new();
    world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
    world.insert_resource(Chronicle::default());
    world.insert_resource(SimulationTime::default());
    world.insert_resource(TemperatureGrid::new(20, 20, 0.0));
    world.insert_resource(CableHeatTracker::default());

    let pos = GridPosition { x: 5, y: 5 };
    world.spawn((
        PowerCable {
            capacity,
            current_load: 0.0,
        },
        pos,
    ));
    let consumer = world
        .spawn((
            PowerConsumer {
                demand,
                active: true,
            },
            pos,
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(chronicle_event_handler_system);
    (world, schedule, consumer)
}

fn run_ticks(world: &mut World, schedule: &mut Schedule, n: u32) {
    for _ in 0..n {
        world.resource_mut::<SimulationTime>().tick += 1;
        let _ = world.run_system_once(evaluate_grid_load_system);
        schedule.run(world);
    }
}

fn set_demand(world: &mut World, consumer: Entity, demand: f32) {
    world.get_mut::<PowerConsumer>(consumer).unwrap().demand = demand;
}

fn heat_texts(world: &World, needle: &str) -> Vec<(u64, String, EventImportance)> {
    world
        .resource::<Chronicle>()
        .events
        .iter()
        .filter(|e| e.text.contains(needle))
        .map(|e| (e.tick, e.text.clone(), e.importance))
        .collect()
}

#[test]
fn sustained_overload_warns_before_any_fire() {
    let (mut world, mut schedule, _consumer) = cable_world(150.0, 100.0);

    // 9 overloaded ticks: still inside the sustained window, silent.
    run_ticks(&mut world, &mut schedule, 9);
    assert!(
        heat_texts(&world, "overheating").is_empty(),
        "no warning before the 10-tick sustained window"
    );

    run_ticks(&mut world, &mut schedule, 6);
    let warnings = heat_texts(&world, "overheating");
    assert_eq!(
        warnings.len(),
        1,
        "sustained overload must produce exactly one cable-heat warning, got: {warnings:?}"
    );
    assert_eq!(
        warnings[0].0, 10,
        "warning must land on the 10th consecutive overloaded tick"
    );
    assert!(
        warnings[0].1.contains("(5, 5)"),
        "warning must name the tile, got: {}",
        warnings[0].1
    );
    assert!(
        warnings[0].1.contains("150") && warnings[0].1.contains("100"),
        "warning must name load and capacity, got: {}",
        warnings[0].1
    );
    assert!(
        matches!(warnings[0].2, EventImportance::Standard),
        "first warning must be Standard (Major is reserved for ignition), got: {:?}",
        warnings[0].2
    );

    // The telegraph's whole point: it lands while nothing is burning.
    let fire_count = world
        .query::<&crate::layer1::fire::Fire>()
        .iter(&world)
        .count();
    assert_eq!(fire_count, 0, "warning must precede any fire");
    assert_eq!(cable_heat_ticks(&world), 15);
    assert!(cable_heat_episode_active(&world));
}

#[test]
fn transient_spike_produces_no_warning() {
    let (mut world, mut schedule, consumer) = cable_world(150.0, 100.0);
    // 5 overloaded ticks — below the 10-tick sustained window.
    run_ticks(&mut world, &mut schedule, 5);
    set_demand(&mut world, consumer, 50.0);
    run_ticks(&mut world, &mut schedule, 20);

    assert!(
        heat_texts(&world, "overheating").is_empty(),
        "a brief overload spike must not warn"
    );
    assert_eq!(cable_heat_ticks(&world), 0);
    assert!(!cable_heat_episode_active(&world));
}

#[test]
fn warning_does_not_repeat_every_tick_and_escalates_at_40() {
    let (mut world, mut schedule, _consumer) = cable_world(150.0, 100.0);
    run_ticks(&mut world, &mut schedule, 60);

    assert_eq!(
        heat_texts(&world, "overheating").len(),
        1,
        "one first warning per episode, not one per tick"
    );
    let escalated = heat_texts(&world, "about to ignite");
    assert_eq!(
        escalated.len(),
        1,
        "one escalated warning per episode, got: {escalated:?}"
    );
    assert_eq!(
        escalated[0].0, 40,
        "escalated warning must land on the 40th consecutive overloaded tick"
    );
    assert!(
        matches!(escalated[0].2, EventImportance::Major),
        "escalated warning must be Major, got: {:?}",
        escalated[0].2
    );
}

#[test]
fn warning_rearms_after_overload_clears() {
    let (mut world, mut schedule, consumer) = cable_world(150.0, 100.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(heat_texts(&world, "overheating").len(), 1);

    // Clear for well past the 5-tick re-arm hysteresis.
    set_demand(&mut world, consumer, 50.0);
    run_ticks(&mut world, &mut schedule, 10);
    assert!(
        !cable_heat_episode_active(&world),
        "episode must clear after the full clear window"
    );

    // Overload again: a fresh episode must warn again.
    set_demand(&mut world, consumer, 150.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(
        heat_texts(&world, "overheating").len(),
        2,
        "warning must re-arm after the overload clears"
    );
}

#[test]
fn flicker_dip_does_not_rearm_or_spam() {
    let (mut world, mut schedule, consumer) = cable_world(150.0, 100.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert!(cable_heat_episode_active(&world));

    // Brief 2-tick dip — inside the 5-tick clear hysteresis, so this is
    // still the same episode: counter reads 0, episode stays armed,
    // and returning to overload must not produce a second warning.
    set_demand(&mut world, consumer, 50.0);
    run_ticks(&mut world, &mut schedule, 2);
    assert_eq!(
        cable_heat_ticks(&world),
        0,
        "counter reads 0 during a flicker dip"
    );
    assert!(
        cable_heat_episode_active(&world),
        "episode must stay active through a dip inside the clear window"
    );

    set_demand(&mut world, consumer, 150.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(
        heat_texts(&world, "overheating").len(),
        1,
        "a flickering cable must not spam the Chronicle"
    );

    // Full clear window (5 non-overloaded ticks total): episode re-arms.
    set_demand(&mut world, consumer, 50.0);
    run_ticks(&mut world, &mut schedule, 5);
    assert!(
        !cable_heat_episode_active(&world),
        "episode must clear after the full 5-tick clear window"
    );
}

#[test]
fn tiles_are_tracked_independently() {
    let (mut world, mut schedule, _consumer) = cable_world(150.0, 100.0);
    // A second cable on another tile, comfortably under capacity.
    let pos2 = GridPosition { x: 8, y: 8 };
    world.spawn((
        PowerCable {
            capacity: 100.0,
            current_load: 0.0,
        },
        pos2,
    ));
    world.spawn((
        PowerConsumer {
            demand: 50.0,
            active: true,
        },
        pos2,
    ));

    run_ticks(&mut world, &mut schedule, 15);
    let warnings = heat_texts(&world, "overheating");
    assert_eq!(warnings.len(), 1, "only the overloaded tile warns");
    assert!(
        warnings[0].1.contains("(5, 5)") && !warnings[0].1.contains("(8, 8)"),
        "warning must name the overloaded tile only, got: {}",
        warnings[0].1
    );
    let tracker = world.resource::<CableHeatTracker>();
    assert_eq!(
        tracker
            .tiles
            .get(&(8, 8))
            .map_or(0, |t| t.consecutive_overloaded_ticks),
        0,
        "the healthy tile must not accumulate overload ticks"
    );
}

#[test]
fn tracker_is_optional_for_minimal_worlds() {
    // Pre-existing minimal worlds (and the original load_limits tests)
    // run evaluate_grid_load_system without a CableHeatTracker: the
    // system must keep working — heat and fire behaviour unchanged —
    // with the telegraph simply absent.
    let mut world = World::new();
    world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
    world.insert_resource(TemperatureGrid::new(20, 20, 0.0));
    let pos = GridPosition { x: 5, y: 5 };
    world.spawn((
        PowerCable {
            capacity: 100.0,
            current_load: 0.0,
        },
        pos,
    ));
    world.spawn((
        PowerConsumer {
            demand: 150.0,
            active: true,
        },
        pos,
    ));
    for _ in 0..15 {
        let _ = world.run_system_once(evaluate_grid_load_system);
    }
    let temperature_grid = world.resource::<TemperatureGrid>();
    assert!(
        temperature_grid.get(5, 5) > 0.0,
        "heat generation must be unchanged without a tracker"
    );
    assert_eq!(cable_heat_ticks(&world), 0);
}
