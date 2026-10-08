//! The Engine of Reality: Layer 1 Physics
//!
//! This module houses the core physical simulation systems of the game.
//! Unlike standard gameplay mechanics, systems in this layer focus on environment dynamics,
//! energy propagation, structural stability, and "Juice" (game feel).
//!
//! # Subsystems
//!
//! - **[`acoustic`]**: Simulates the propagation and damping of noise through various terrain types.
//!   Noise stresses out pops and cannot travel through vacuum.
//! - **[`hit_stop`]**: Implements "Hit Stop" (simulation freeze on major events) to enhance impact and game feel.
//! - **[`kinetic_storage`]**: Handles gravity batteries that store energy physically. They explode spectacularly if destroyed while fully charged.
//! - **[`particles`]**: A purely visual physics system for sub-grid particle effects, including gravity, friction, and confetti bursts.
//! - **[`pressure`]**: Simulates atmospheric pressure, vacuum decay, and oxygen dispersion across the grid.
//! - **[`structural_integrity`]**: Prevents infinite tunneling by enforcing support limits on roofs and causing cave-ins when undermined.
//! - **[`suction`]**: Simulates the violent explosive decompression events that suck entities into vacuums when hulls are breached.
//!
//! # Philosophy
//!
//! The Physics module operates beneath the awareness of the colony's inhabitants.
//! It exists to create a harsh, reactive environment where actions have delayed, cascading consequences
//! (e.g., mining too wide causes a collapse, venting a room causes suction, running noisy machines stresses workers).
pub mod acoustic;
pub mod curvature;
pub mod gravity_debt;
pub mod harpoon;
pub mod hit_stop;
pub mod kinetic_storage;
pub mod particles;
pub mod pressure;
pub mod resonance;
pub mod structural_integrity;
pub mod suction;
pub mod vent;

pub use acoustic::*;
pub use curvature::*;
pub use gravity_debt::*;
pub use harpoon::*;
pub use hit_stop::*;
pub use kinetic_storage::*;
pub use particles::*;
pub use pressure::*;
pub use resonance::*;
pub use structural_integrity::*;
pub use suction::*;
pub use vent::*;

#[cfg(test)]
mod acoustic_shadow_tests;

#[cfg(test)]
mod venting_tests;

#[cfg(test)]
mod vacuum_welding_tests;

pub mod gravity_plating;
#[cfg(test)]
mod structural_integrity_overflow_tests;
pub use gravity_plating::*;
pub mod heat_death;
pub use heat_death::*;
#[cfg(test)]
mod curvature_exploit_test;
pub mod magnetic_reversal;
pub use magnetic_reversal::*;
