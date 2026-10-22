//! Item 117 RED tests for the grid-straining telegraph.
//!
//! These run against the LIVE path: every tick calls the real
//! `power_grid_system` (the same exclusive system the Economy schedule
//! runs) and then the real `chronicle_event_handler_system`, so the
//! warnings asserted here are the ones a live colony would record.
//! Demand 16 vs. nameplate 10 is ratio 1.6 — strained (> 1.5) with an
//! ignition chance of only 0.005/tick, so the ignition roll cannot
//! race the telegraph in these tests.

use super::grid_strain::{grid_strain_episode_active, grid_strain_ticks, GridStrainTracker};
use super::{power_grid_system, PowerConsumer, PowerSource};
use crate::layer1::chronicle::{chronicle_event_handler_system, Chronicle};
use crate::layer1::map::GridPosition;
use crate::shared::time::SimulationTime;
use bevy_ecs::event::Events;
use bevy_ecs::prelude::*;

fn strained_world() -> (World, Schedule, Entity) {
    let mut world = World::new();
    world.init_resource::<Events<super::GridOverloadEvent>>();
    world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
    world.insert_resource(Chronicle::default());
    world.insert_resource(SimulationTime::default());
    world.insert_resource(GridStrainTracker::default());

    world.spawn((
        PowerSource {
            output: 10.0,
            active: true,
        },
        GridPosition { x: 0, y: 0 },
    ));
    let consumer = world
        .spawn((
            PowerConsumer {
                demand: 16.0,
                active: true,
            },
            GridPosition { x: 1, y: 0 },
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(chronicle_event_handler_system);
    (world, schedule, consumer)
}

fn run_ticks(world: &mut World, schedule: &mut Schedule, n: u32) {
    for _ in 0..n {
        world.resource_mut::<SimulationTime>().tick += 1;
        power_grid_system(world);
        schedule.run(world);
    }
}

fn set_demand(world: &mut World, consumer: Entity, demand: f32) {
    world.get_mut::<PowerConsumer>(consumer).unwrap().demand = demand;
}

fn strain_texts(world: &World, needle: &str) -> Vec<(u64, String)> {
    world
        .resource::<Chronicle>()
        .events
        .iter()
        .filter(|e| e.text.contains(needle))
        .map(|e| (e.tick, e.text.clone()))
        .collect()
}

#[test]
fn sustained_overload_warns_before_any_fire() {
    let (mut world, mut schedule, _consumer) = strained_world();
    run_ticks(&mut world, &mut schedule, 15);

    let warnings = strain_texts(&world, "straining");
    assert_eq!(
        warnings.len(),
        1,
        "sustained overload must produce exactly one strain warning, got: {warnings:?}"
    );
    assert_eq!(
        warnings[0].0, 10,
        "warning must land on the 10th consecutive strained tick"
    );
    assert!(
        warnings[0].1.contains("160%"),
        "warning should name the overload ratio, got: {}",
        warnings[0].1
    );

    // The telegraph's whole point: it lands while nothing is burning.
    let fire_count = world
        .query::<&crate::layer1::fire::Fire>()
        .iter(&world)
        .count();
    assert_eq!(fire_count, 0, "warning must precede any fire");
    assert_eq!(grid_strain_ticks(&world), 15);
}

#[test]
fn transient_spike_produces_no_warning() {
    let (mut world, mut schedule, consumer) = strained_world();
    // 5 strained ticks — below the 10-tick sustained window.
    run_ticks(&mut world, &mut schedule, 5);
    set_demand(&mut world, consumer, 5.0);
    run_ticks(&mut world, &mut schedule, 20);

    assert!(
        strain_texts(&world, "straining").is_empty(),
        "a brief overload spike must not warn"
    );
    assert_eq!(grid_strain_ticks(&world), 0);
}

#[test]
fn warning_does_not_repeat_every_tick() {
    let (mut world, mut schedule, _consumer) = strained_world();
    run_ticks(&mut world, &mut schedule, 60);

    assert_eq!(
        strain_texts(&world, "straining").len(),
        1,
        "one first warning per episode, not one per tick"
    );
    assert_eq!(
        strain_texts(&world, "breaking point").len(),
        1,
        "one escalated warning per episode"
    );
}

#[test]
fn warning_rearms_after_strain_clears() {
    let (mut world, mut schedule, consumer) = strained_world();
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(strain_texts(&world, "straining").len(), 1);

    // Clear for well past the 5-tick re-arm hysteresis.
    set_demand(&mut world, consumer, 5.0);
    run_ticks(&mut world, &mut schedule, 10);

    // Strain again: a fresh episode must warn again.
    set_demand(&mut world, consumer, 16.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(
        strain_texts(&world, "straining").len(),
        2,
        "warning must re-arm after the strain clears"
    );
}

#[test]
fn episode_stays_active_through_flicker_dip() {
    // Item 120 / balance Bug #3: during a <=4-tick flicker dip the
    // consecutive counter reads 0, but the episode is still armed —
    // STATS must be able to say so via grid_strain_episode_active.
    let (mut world, mut schedule, consumer) = strained_world();
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(grid_strain_ticks(&world), 15);
    assert!(
        grid_strain_episode_active(&world),
        "episode must be active while strained past the warning"
    );

    // 2-tick dip: counter resets, episode must still read active.
    set_demand(&mut world, consumer, 5.0);
    run_ticks(&mut world, &mut schedule, 2);
    assert_eq!(
        grid_strain_ticks(&world),
        0,
        "counter reads 0 during a flicker dip (pre-existing behavior)"
    );
    assert!(
        grid_strain_episode_active(&world),
        "episode must stay active through a dip inside the clear window"
    );

    // Full clear window (5 unstrained ticks total): episode re-arms.
    run_ticks(&mut world, &mut schedule, 3);
    assert!(
        !grid_strain_episode_active(&world),
        "episode must clear after the full 5-tick clear window"
    );
}

#[test]
fn flicker_does_not_rearm_or_spam() {
    let (mut world, mut schedule, consumer) = strained_world();
    run_ticks(&mut world, &mut schedule, 15);
    // Brief 2-tick dip — inside the 5-tick clear hysteresis, so this is
    // still the same episode and must not produce a second warning.
    set_demand(&mut world, consumer, 5.0);
    run_ticks(&mut world, &mut schedule, 2);
    set_demand(&mut world, consumer, 16.0);
    run_ticks(&mut world, &mut schedule, 15);
    assert_eq!(
        strain_texts(&world, "straining").len(),
        1,
        "a flickering grid must not spam the Chronicle"
    );
}
