//! Layer 2 logistics: moving fleets and their cargo between bodies.
//!
//! * `launch` — Spec 379 Launch Windows: transfer-window fuel costs,
//!   fuel-gated fleet launches, and the window predictor.

pub mod launch;
pub mod orbital_synch;

#[cfg(test)]
mod launch_window_tests;

#[cfg(test)]
mod orbital_synch_tests;
