//! Layer 3 civilization technology level (Spec 380, dep 011 adapted).
//!
//! The spec sketch assumed a `layer3::tech` module tracking a
//! civilization's technology level; none existed. The real tech tree
//! (spec 011) is Layer 1's colony-local [`crate::layer1::tech::TechState`]
//! — a data-economy resource, not a per-civilization component — so it
//! cannot carry per-civ levels for the galaxy's many civilizations.
//! This module is the missing Layer 3 leg: a per-civilization scalar
//! level plus the advance event/system that moves it. The Great Filter
//! (`crate::layer3::events::great_filter`) watches these levels.
//!
//! A level can be derived from a colony's real `TechState` with
//! [`level_from_tech_state`] so Layer 1 research can feed Layer 3
//! advancement without duplicating the tech tree itself.

use bevy_ecs::prelude::*;

/// A civilization's aggregate technology level (Layer 3 scale).
/// The Great Filter threshold is 100 (see `great_filter::FILTER_THRESHOLD`).
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CivilizationTechLevel {
    /// Aggregate technology level.
    pub level: u32,
}

/// Advance a civilization's technology level by `amount`.
#[derive(Event, Debug, Clone, Copy)]
pub struct TechAdvanceEvent {
    /// The civilization advancing.
    pub civ: Entity,
    /// How much to advance (saturating).
    pub amount: u32,
}

/// Applies [`TechAdvanceEvent`]s to [`CivilizationTechLevel`]s.
/// The mutation marks the component `Changed`, which is what
/// `trigger_filter_events_system` watches — advancement, not mere
/// existence, is what risks the Filter.
pub fn advance_tech_system(
    mut events: EventReader<TechAdvanceEvent>,
    mut query: Query<&mut CivilizationTechLevel>,
) {
    for event in events.read() {
        if let Ok(mut tech) = query.get_mut(event.civ) {
            tech.level = tech.level.saturating_add(event.amount);
        }
    }
}

/// Derives a Layer 3 level from a colony's real Layer 1 tech state:
/// 10 points per researched technology. With the current 12-tech tree
/// a fully-researched colony sits at 120 — past the Filter threshold,
/// which is the intended endgame tension.
#[must_use]
pub fn level_from_tech_state(state: &crate::layer1::tech::TechState) -> u32 {
    let researched = state.techs.len() as u32;
    researched.saturating_mul(10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::tech::{Tech, TechState};

    #[test]
    fn level_from_empty_tech_state_is_zero() {
        assert_eq!(level_from_tech_state(&TechState::default()), 0);
    }

    #[test]
    fn level_from_tech_state_counts_researched() {
        let mut state = TechState {
            total_capacity: 10_000.0,
            ..Default::default()
        };
        state.unlock(Tech::Masonry);
        state.unlock(Tech::Terraforming);
        assert_eq!(level_from_tech_state(&state), 20);
    }
}
