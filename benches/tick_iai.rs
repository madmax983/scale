//! Deterministic instruction-count benchmark for the simulation tick.
//!
//! Run with: `cargo bench --bench tick_iai` (requires valgrind and
//! `iai-callgrind-runner` 0.16.1).
#![allow(missing_docs)]

use bevy_ecs::world::World;
use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use scale::setup::{setup_world_with_config, SetupConfig};
use scale::shared::state::GameState;
use scale::simulation::run_simulation_tick;
use std::hint::black_box;

fn setup(ticks: u32) -> (World, u32) {
    scale::setup::init_task_pools();
    let mut world = setup_world_with_config(SetupConfig {
        headless: true,
        ..Default::default()
    });
    *world.resource_mut::<GameState>() = GameState::Running;
    // First tick builds/initializes the schedule; keep it out of the measurement.
    run_simulation_tick(&mut world);
    (world, ticks)
}

#[library_benchmark]
#[benches::ticks(setup = setup, args = [10, 50])]
fn simulation_ticks((mut world, ticks): (World, u32)) -> World {
    for _ in 0..black_box(ticks) {
        run_simulation_tick(&mut world);
    }
    world
}

library_benchmark_group!(name = tick; benchmarks = simulation_ticks);
main!(library_benchmark_groups = tick);
