//! Agriculture
//!
//! Handles food production, farming, husbandry, and gastronomy.

pub mod farm;
pub mod compost;
pub mod gastronomy;
pub mod husbandry;
pub mod zero_g_flora;

#[cfg(test)]
mod greenhouse_tests;

#[cfg(test)]
mod hydroponics_tests;

#[cfg(test)]
mod preservation_tests;

pub use farm::*;
pub use gastronomy::*;
pub use husbandry::*;
pub use zero_g_flora::*;
pub mod pollination;
pub use pollination::*;

pub mod vacuum;
pub use vacuum::*;
