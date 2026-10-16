//! Biology and Health systems.
//!
//! Contains logic for health, diseases, cloning, cybernetics, and genetics.
pub mod symbiotic_parasite;

pub mod addiction;
pub mod agony_extract;
pub mod biocompatibility;
pub mod clone_vat;
pub mod contagion;
pub mod cryo_shock;
pub mod cybernetic_ascendancy;
pub mod cybernetics;
pub mod gene_bank;
pub mod genetics;
pub mod grafting;
pub mod gut_biome;
pub mod death_attribution;
pub mod health;
pub mod invasive;
pub mod medical;
#[cfg(test)]
#[allow(missing_docs)]
pub mod medical_triage_tests;

pub use addiction::*;
pub use agony_extract::*;
pub use biocompatibility::*;
pub use clone_vat::*;
pub use contagion::*;
pub use cryo_shock::*;
pub use cybernetic_ascendancy::*;
pub use cybernetics::*;
pub use gene_bank::*;
pub use genetics::*;
pub use grafting::*;
pub use gut_biome::*;
pub use health::*;
pub use invasive::*;
pub use medical::*;
pub mod gravity_caste;
pub mod rust_lung;
pub mod symbiotic_insurgency;
pub use symbiotic_insurgency::*;
pub mod xenoflora_addiction;
pub use xenoflora_addiction::*;

pub mod symbiotic_gear;
pub use symbiotic_gear::*;

pub mod chromotaxis;
pub use chromotaxis::*;
