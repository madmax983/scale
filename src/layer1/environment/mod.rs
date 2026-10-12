//! Environment
//!
//! Manages environmental hazards, disasters, and terraforming efforts.

pub mod events;
pub mod ignition;
pub use ignition::{process_ignition, SparkEvent, VolatileVapor};
pub mod artificial_sunspots;
pub use artificial_sunspots::*;

pub mod bio_acoustic;
pub mod bio_acoustic_miasma;
pub mod disasters;
pub mod geomes;
pub mod hazards;
pub mod hazards_tests;
pub mod light_pollution;
pub mod orbital_crossfire;
pub mod orbital_shadow;
pub mod orbital_tether;
pub mod photophobic;
pub mod terminator_habitats;
pub use terminator_habitats::*;
pub mod seismic;
pub mod terraforming;
pub mod volatile;

pub use bio_acoustic::*;
pub use bio_acoustic_miasma::*;
pub use disasters::*;
pub use geomes::*;
pub use hazards::*;
pub use light_pollution::*;
pub use orbital_crossfire::*;
pub use orbital_shadow::*;
pub use orbital_tether::*;
pub use photophobic::*;
pub use seismic::*;
pub use terraforming::*;
pub use volatile::{handle_explosion_system, volatile_decay_system, ExplosionEvent, Volatile};
pub mod geothermal;

pub mod ephemeral_moons;
pub mod impact;
pub use ephemeral_moons::*;
pub use impact::*;
pub mod thermal_camouflage;
pub use thermal_camouflage::*;
pub mod atmosphere;
pub use atmosphere::*;

pub mod orbital_junkyard;
pub use orbital_junkyard::*;

pub mod mycorrhizal_network;
pub use mycorrhizal_network::*;

pub mod chrono_stutter;
pub use chrono_stutter::*;
pub mod fungal_monoliths;
pub use fungal_monoliths::*;
pub mod blind_terraformer;
pub use blind_terraformer::*;
