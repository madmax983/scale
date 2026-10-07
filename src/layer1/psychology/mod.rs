//! Psychology
//!
//! Manages the mental state of pops, including needs, stress, dreams, and traits.

pub mod cabin_fever;
pub mod cryo_dreams;
pub mod dreams;
pub mod generational_amnesia;
pub mod generational_spite;
pub use generational_spite::*;
#[cfg(feature = "nova")]
pub mod machine_consciousness;
pub mod memory;
pub mod memory_blackout;
pub mod memory_forgery;
pub mod needs;
pub mod overview_effect;
pub mod panic_spirals;
pub mod pop_memories;
pub mod psionics;
pub mod psychic;
pub mod psychic_stains;
pub mod quirks;
#[cfg(test)]
mod quirks_tests;
pub mod simulacrum;
#[cfg(feature = "nova")]
pub mod sleep_deprived_savant;
pub mod sleepwalking;
#[cfg(test)]
mod sleepwalking_tests;
pub mod somnambulism;
pub mod spiteful_will;
pub mod stress;
pub mod traits;
pub mod void_sickness;
pub mod void_stare;
pub use generational_amnesia::*;

pub use cabin_fever::*;
pub use cryo_dreams::*;
pub use dreams::*;
#[cfg(feature = "nova")]
pub use machine_consciousness::*;
pub use memory::*;
pub use memory_blackout::*;
pub use memory_forgery::*;
pub use needs::*;
pub use overview_effect::*;
pub use panic_spirals::*;
pub use pop_memories::*;
pub use psionics::*;
pub use psychic::*;
pub use psychic_stains::*;
pub use quirks::*;
pub use simulacrum::*;
#[cfg(feature = "nova")]
pub use sleep_deprived_savant::*;
pub use sleepwalking::*;
pub use somnambulism::*;
pub use spiteful_will::*;
pub use stress::*;
pub use traits::*;
pub use void_sickness::*;
pub use void_stare::*;
pub mod teleport_psychosis;
pub use teleport_psychosis::*;
pub mod void_sleep;
pub use void_sleep::*;
pub mod artifact_diet;
pub mod doomsday;
pub use doomsday::*;
pub mod dreaming_sickness;
pub use dreaming_sickness::*;
