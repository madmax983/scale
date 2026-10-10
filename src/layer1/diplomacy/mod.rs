//! Diplomacy and Faction Relations
//!
//! This module manages interactions between the colony and external factions,
//! entities, or neighboring groups. It handles treaties, diplomatic wards,
//! and the shifting allegiances within the simulation.
//!
//! # Mechanics
//! - **Wards:** Logic for managing diplomatic protections, alliances, or specific
//!   zones of control influenced by external relations.

pub mod wards;
pub use wards::*;
pub mod factions;

use bevy::prelude::*;

#[derive(Event, Debug)]
pub struct TributeDemandEvent {
    pub aggressor: Entity,
    pub system: Entity,
    pub amount: u32,
}
pub mod blackmail;
pub use blackmail::*;
