//! Layer 2: System/Planetary Simulation.
//!
//! This layer abstractly models the wider planetary and star system context surrounding the colony.
//! This bridges the gap between the detailed colony simulation ([`crate::layer1`]) and the
//! vast galactic theater ([`crate::layer3`]).
//!
//! # Core Concepts
//!
//! ## Celestial Bodies
//! The simulation abstracts individual celestial entities within a system:
//! - **Planets & Moons:** Simulated through modules like `planet` and `binary_star`.
//! - **Orbital Infrastructure:** Handled by `derelict_stations`.
//!
//! ## FTL & Mining
//! - FTL traverse the system (`ftl`).
//! - Resources are extracted (`mining`).
//!
//! ## Environment
//! System-wide environmental factors impact all entities within:
//! - `environment` and `planetary_scarring`.
//!
//! # Module Structure
//! This layer serves as a hub, exporting systems related to `governance`, `environment`, and `fleet`,
//! managing the interaction of multiple colonies within a single star system.
pub mod ftl;
pub mod memorial_fleet;
pub mod planet;
pub mod planetary_scarring;
pub use memorial_fleet::*;
pub mod barnacles;
pub mod binary_star;
pub mod blind_jump;
pub mod combat;
pub use blind_jump::*;
pub mod debris;
#[cfg(test)]
mod debris_tests;
pub mod designation;
pub mod environment;
pub mod nebulae;
pub mod primitives;
pub use environment::*;
pub mod auction;
pub mod cartographers_curse;
pub mod cascade;
pub use cascade::*;
pub mod dead_protocols;
pub mod derelict_stations;
pub mod events;
pub mod events_new;
pub mod fleet;
pub mod generation;
pub mod governance;
pub mod integration;
pub mod leadership;
pub use leadership::*;
pub mod mining;
#[cfg(test)]
mod mining_tests;
pub mod orphan_fleet;
pub mod phantom;
pub mod planetary_rings;
pub mod planetary_spin_up;
pub mod render;
pub mod shielding;
pub mod ship;
pub mod silent_mutiny;
pub mod station;
#[cfg(test)]
mod station_tests;
pub mod system;
pub mod syzygy;
pub mod thermal;
#[cfg(test)]
mod thermal_bloom_tests;
pub mod tourism;
pub mod trade;
pub mod visibility;
pub use cartographers_curse::*;
pub mod empathic_plague;
pub mod exploration;
pub mod moon_hermits;
pub mod mutiny;
pub mod navigation;
pub mod sensor_ambiguity;
pub mod skyhooks;

pub mod megastructure;
pub mod rogue_planets;
pub mod weather;
pub use weather::*;
pub mod bombardment;
pub mod orbit;
pub mod orbital_necropolis;

pub mod mycelial_network;

pub mod orbital_mirrors;
pub use orbital_mirrors::*;
pub mod culture;
pub mod diaspora;
pub mod sensors;
pub use diaspora::*;
pub mod piracy;
pub use piracy::*;
pub mod fauna;
pub use fauna::*;
pub mod gravitational_doldrums;
pub mod phantom_signal;
pub use gravitational_doldrums::*;
pub mod celestial_library;
pub mod communications;
pub mod signature;

use bevy::prelude::*;

pub struct PlanetarySpinUpPlugin;

pub struct TradePlugin;

impl Plugin for TradePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::layer2::trade::phantom_trade_routes::PhantomTradeRoutePlugin);
    }
}

impl Plugin for PlanetarySpinUpPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<planetary_spin_up::PlanetaryTorqueEvent>()
            .add_event::<crate::layer2::stolen_fleet::WarDeclarationEvent>()
            .add_event::<navigation::chronological_stutter::HyperlaneTransitEvent>()
            .add_event::<crate::layer2::ftl::wakes::FtlJumpEvent>()
            .add_event::<crate::layer2::ftl::wakes::SubspaceWakeEvent>()
            .add_systems(
                Update,
                (
                    planetary_spin_up::apply_planetary_torque_system,
                    planetary_spin_up::calculate_effective_gravity_system,
                    planetary_spin_up::trigger_coriolis_weather_system,
                    planetary_rings::apply_planetary_ring_effects_system,
                    orbital_ring::update_shadow_band_system,
                    navigation::chronological_stutter::apply_chronological_stutter_system,
                    navigation::warp_wake::apply_warp_wake_system,
                    navigation::warp_wake::decay_warp_wake_system,
                    navigation::warp_wake::generate_warp_wake_system,
                ),
            );
    }
}
pub mod asteroid_hermits;
pub mod cryo_mutiny;
pub mod leviathans;
pub mod orbital_ring;
pub mod refugees;
pub mod solar_sail_migration;
pub mod stolen_fleet;
pub mod void_leviathan;
pub use stolen_fleet::*;
pub mod ecophagy;
pub mod propaganda_engine;
pub mod propaganda;
pub mod pulsar;

pub mod defector;
pub mod sub_light_arrival;
