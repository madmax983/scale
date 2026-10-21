//! Profiling harness: headless simulation, N ticks (default 10).
//!
//! Run under `valgrind --tool=callgrind` and compare tick counts to separate
//! one-time schedule construction from steady-state per-tick cost.

use scale::prelude::*;

fn main() {
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let config = SetupConfig {
        headless: true,
        ..Default::default()
    };
    let mut world = setup_world_with_config(config);
    for _ in 0..ticks {
        run_simulation_tick(&mut world);
    }
    let pops = world.query::<&Pop>().iter(&world).count();
    println!("ticks={ticks} pops={pops}");
}
