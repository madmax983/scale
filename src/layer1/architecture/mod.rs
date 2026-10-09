//! Architecture
//!
//! Defines building structures, housing, and structural integrity mechanics.

/// Biomimetic architecture.
pub mod biomimetic;
/// General building framework and implementations.
pub mod building;
pub mod edible;
pub use biomimetic::*;
pub use edible::*;
#[cfg(test)]
mod building_gate_test;
#[cfg(test)]
mod work_building_tests;

/// Housing and shelter definitions.
pub mod housing;
/// Common structure components.
pub mod structure;
#[cfg(test)]
mod structure_fragile_tests;
#[cfg(test)]
mod structure_jury_rig_tests;
#[cfg(test)]
mod structure_maintenance_tests;

/// Parasitic architecture definitions.
pub mod parasitic_architecture;
/// Logic for calculating room quality.
pub mod room_quality;
/// Ruins mechanisms.
pub mod ruins;
/// Spontaneous architecture mechanics.
pub mod spontaneous_architecture;
/// Symbiotic infrastructure mechanics.
pub mod symbiotic_infrastructure;
/// Defense turret logic.
pub mod turret;
/// Window mechanics.
pub mod window;

pub use building::*;
pub use housing::*;
pub use parasitic_architecture::*;
pub use room_quality::*;
pub use ruins::*;
pub use spontaneous_architecture::*;
pub use structure::*;
pub use symbiotic_infrastructure::*;
pub use turret::*;
pub use window::*;

pub mod living_architecture;
pub use living_architecture::*;
pub mod resonant_architecture;
pub use resonant_architecture::*;
pub mod embezzlement;
pub use embezzlement::*;
pub mod fossilized_fleet;
pub mod fossilized_fleet_tests;
pub mod hostage_protocol;
pub mod sunk_cost_monument;
pub use hostage_protocol::HostageProtocolPlugin;
pub mod smart_matter;
pub use smart_matter::*;
pub mod chrono_vault;
pub use chrono_vault::*;
pub mod gravity_engineering;
pub use gravity_engineering::*;
pub mod potemkin;
pub mod construction_sabotage;
pub use potemkin::*;
