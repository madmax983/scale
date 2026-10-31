//! Layer 3: Galactic Level Simulation
//!
//! This layer handles galaxy-wide simulation aspects, connecting solar systems and factions.
//! It serves as the grand strategy ("Stellaris") layer, managing macro-scale civilization dynamics.
//!
//! # Core Concepts
//!
//! ## Interstellar Relations
//! The galaxy is shaped by the interactions of vast factions:
//! - **Diplomacy:** Treaties, council resolutions, and negotiations (`diplomacy`, `council`).
//! - **Conflict:** Sovereign armadas, pirates, and ghost ships (`fleets`, `pirates`, `ghost_ships`).
//!
//! ## The Galactic Market
//! A macro-economy that transcends individual systems:
//! - Features subjective economics and corporate bureaucracy (`market`, `economy`).
//!
//! ## Galactic Geography
//! - The galactic map and stellar cartography track the known universe (`map`, `stellar_cartography`).
//!
//! # Simulation Flow
//! This layer abstracts the minute details of Layer 1 and 2, focusing instead on high-level
//! events (`events`), galactic silence (`silence`), and the shifting power dynamics of a universe
//! inhabited by countless civilizations.
pub mod council;
pub mod events;
pub mod market;
pub mod resources;
pub mod silence;
pub use council::*;
pub mod diplomacy;
pub use diplomacy::*;
pub mod bureaucracy_of_truth;
pub mod bureaucracy_of_vanity;
pub mod digital_detritus;
pub mod diplomacy_reflection;
pub mod fleets;
pub mod ghost_ships;
pub mod integration;
pub mod linguistic_drift;
pub mod map;
pub mod physics;
pub mod pirates;
pub mod planet;
pub mod stellar_cartography;
pub mod subjective_economics;
pub mod treaty_cruisers;
pub use bureaucracy_of_vanity::*;
pub use digital_detritus::*;
pub mod bureaucracy;
pub mod economy;
pub mod intellectual_property_wars;

pub mod auditor;
pub mod guilt;
pub mod zoo_hypothesis;

pub mod galaxy;
pub mod tech;
pub mod quarantine;
pub mod sovereign_armada;
pub mod tourism_integration;
