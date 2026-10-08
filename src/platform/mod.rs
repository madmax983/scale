//! Platform abstraction layer.
//!
//! This module provides platform-agnostic input types that both native (crossterm)
//! and WASM (ratzilla) backends translate into, allowing shared game logic to
//! handle input without coupling to a specific backend.

/// Native (crossterm) platform adapter.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;

/// WASM (ratzilla) platform adapter.
#[cfg(any(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm;
