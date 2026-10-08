#![deny(unsafe_code)]
#![allow(rustdoc::invalid_rust_codeblocks)]
//! The core library for SCALE.
//!
//! This library exposes the simulation layers, shared utilities, and UI components of the game.
//! It is designed to be platform-agnostic, running both natively (crossterm) and in the browser (WASM).
//!
//! # Architecture
//!
//! The game is built on `bevy_ecs`, utilizing a strict Entity-Component-System architecture.
//!
//! ## Layers
//!
//! 1.  **Layer 1 (Colony Simulation):** The core "Dwarf Fortress" mode. Handles individual Pops, Buildings, Needs, and Terrain.
//!     See [`layer1`] for details.
//! 2.  **Shared:** Common utilities used across layers and platforms (Time, Input, Narrative).
//!     See [`shared`] for details.
//! 3.  **UI:** The `ratatui`-based interface. It reads the ECS state and renders to a terminal grid.
//!     See [`ui`] for details.
//!
//! ## Data Flow
//!
//! 1.  **Input:** Platform-specific input (Key/Mouse) is normalized into `InputContext` events.
//! 2.  **Simulation:** The [`simulation::run_simulation_tick`] function advances the world state.
//!     -   AI Decisions (GPU/CPU) -> `PopAction`
//!     -   Execution Systems -> World Mutation
//! 3.  **Render:** The UI systems query the world and draw to the screen.
//!
//! # Examples
//!
//! ## Initializing a Headless Simulation
//!
//! ```
//! use scale::prelude::*;
//!
//! // 1. Setup the world with headless configuration
//! let config = SetupConfig {
//!     headless: true,
//!     ..Default::default()
//! };
//! let mut world = setup_world_with_config(config);
//!
//! // 2. Run a few ticks
//! for _ in 0..10 {
//!     run_simulation_tick(&mut world);
//! }
//!
//! // 3. Inspect state
//! let time = world.resource::<SimulationTime>();
//! println!("Current Tick: {}", time.tick);
//! ```

/// GPU compute for utility AI evaluation (native only; the WASM build
/// schedules the CPU fallback instead).
#[cfg(not(target_arch = "wasm32"))]
pub mod gpu;
/// Layer 1: Colony Simulation (Pops, Buildings, Terrain).
pub mod layer1;
/// Layer 2: System Simulation (Orbital Bodies, Fleets).
pub mod layer2;
/// Layer 3: Galaxy Simulation (The Silence, Threats).
pub mod layer3;
/// Platform abstraction for native/WASM backends.
pub mod platform;
/// No-op text-styling shims for wasm32 (stand-in for crossterm::style).
#[cfg(target_arch = "wasm32")]
mod wasm_style;
/// Shared world setup.
pub mod setup;
/// Shared utilities (Time, Input, Narrative).
pub mod shared;
/// Shared simulation tick logic.
pub mod simulation;
/// User Interface components.
pub mod ui;

pub use shared::rendering::{RenderBuffer, RenderItem, RenderLayer};
pub use shared::state::GameState;
pub use ui::selection::{inspect_entity, inspect_tile, Selection, SelectionTarget};
pub mod cross_layer;
pub mod experimental;

/// The prelude module.
pub mod prelude;
