//! Startup scenario configuration and state.
//!
//! This module defines the available starting conditions for a new game,
//! dictating the initial resources, population, and environment difficulty.
//!
//! The [`SetupConfig`] is used during world initialization to apply the correct
//! [`StartScenarioId`] configuration.

use bevy::prelude::*;

/// Configuration parameters for initializing a new simulation world.
///
/// # Examples
///
/// ```
/// use scale::shared::scenario::{SetupConfig, StartScenarioId};
///
/// let config = SetupConfig {
///     headless: true,
///     scenario: StartScenarioId::GroundSurvival,
/// };
///
/// assert!(config.headless);
/// assert_eq!(config.scenario, StartScenarioId::GroundSurvival);
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct SetupConfig {
    /// If true, skip GPU initialization (for headless environments).
    pub headless: bool,
    /// Which built-in start scenario to use for startup plumbing.
    pub scenario: StartScenarioId,
}

/// Coarse difficulty tags for curated start scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartScenarioDifficulty {
    Hard,
    Medium,
    Easy,
    Standard,
}

/// Stable built-in identifiers for curated startup scenarios.
///
/// Each scenario corresponds to a unique starting configuration (e.g., more
/// resources, harsher weather).
///
/// # Examples
///
/// ```
/// use scale::shared::scenario::{StartScenarioId, start_scenario_definition, StartScenarioDifficulty};
///
/// let id = StartScenarioId::Layer2Ready;
/// let def = start_scenario_definition(id);
///
/// assert_eq!(def.name, "Layer 2 Ready");
/// assert_eq!(def.difficulty, StartScenarioDifficulty::Easy);
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StartScenarioId {
    #[default]
    Classic,
    GroundSurvival,
    SocialDrama,
    Layer2Ready,
}

impl StartScenarioId {
    /// All built-in start scenarios in stable menu order.
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Classic,
            Self::GroundSurvival,
            Self::SocialDrama,
            Self::Layer2Ready,
        ]
    }

    /// Stable machine id, used in flags, STATS, and save data.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::GroundSurvival => "ground_survival",
            Self::SocialDrama => "social_drama",
            Self::Layer2Ready => "layer2_ready",
        }
    }

    /// Player-facing display name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        start_scenario_definition(*self).name
    }

    /// Parse a player-typed scenario name: stable ids, display names,
    /// and a few short aliases, all case-insensitive.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "classic" => Self::Classic,
            "ground_survival" | "ground-survival" | "ground survival" | "ground" => {
                Self::GroundSurvival
            }
            "social_drama" | "social-drama" | "social drama" | "social" => Self::SocialDrama,
            "layer2_ready" | "layer2-ready" | "layer2ready" | "layer 2 ready" | "l2r" => {
                Self::Layer2Ready
            }
            _ => return None,
        })
    }
}

/// Static metadata for a built-in start scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartScenarioDefinition {
    pub id: StartScenarioId,
    pub name: &'static str,
    pub difficulty: StartScenarioDifficulty,
}

/// The startup scenario selected for the active world.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveStartScenario {
    pub id: StartScenarioId,
    pub name: &'static str,
    pub difficulty: StartScenarioDifficulty,
}

/// The startup scenario state already baked into the current world.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedStartScenario {
    pub id: StartScenarioId,
}

#[must_use]
pub const fn start_scenario_definition(id: StartScenarioId) -> StartScenarioDefinition {
    match id {
        StartScenarioId::Classic => StartScenarioDefinition {
            id,
            name: "Classic",
            difficulty: StartScenarioDifficulty::Standard,
        },
        StartScenarioId::GroundSurvival => StartScenarioDefinition {
            id,
            name: "Ground Survival",
            difficulty: StartScenarioDifficulty::Hard,
        },
        StartScenarioId::SocialDrama => StartScenarioDefinition {
            id,
            name: "Social Drama",
            difficulty: StartScenarioDifficulty::Medium,
        },
        StartScenarioId::Layer2Ready => StartScenarioDefinition {
            id,
            name: "Layer 2 Ready",
            difficulty: StartScenarioDifficulty::Easy,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ids_and_names_round_trip() {
        let ids: Vec<&str> = StartScenarioId::all()
            .iter()
            .map(StartScenarioId::id)
            .collect();
        assert_eq!(
            ids,
            ["classic", "ground_survival", "social_drama", "layer2_ready"]
        );
        let names: Vec<&str> = StartScenarioId::all()
            .iter()
            .map(StartScenarioId::name)
            .collect();
        assert_eq!(
            names,
            [
                "Classic",
                "Ground Survival",
                "Social Drama",
                "Layer 2 Ready"
            ]
        );
        for scenario in StartScenarioId::all() {
            assert_eq!(StartScenarioId::parse(scenario.id()), Some(scenario));
            assert_eq!(StartScenarioId::parse(scenario.name()), Some(scenario));
        }
    }

    #[test]
    fn parse_accepts_ids_names_and_aliases() {
        for (input, want) in [
            ("classic", StartScenarioId::Classic),
            ("Classic", StartScenarioId::Classic),
            ("CLASSIC", StartScenarioId::Classic),
            ("ground_survival", StartScenarioId::GroundSurvival),
            ("Ground Survival", StartScenarioId::GroundSurvival),
            ("GROUND_SURVIVAL", StartScenarioId::GroundSurvival),
            ("ground", StartScenarioId::GroundSurvival),
            ("social_drama", StartScenarioId::SocialDrama),
            ("Social Drama", StartScenarioId::SocialDrama),
            ("social", StartScenarioId::SocialDrama),
            ("layer2_ready", StartScenarioId::Layer2Ready),
            ("layer2ready", StartScenarioId::Layer2Ready),
            ("Layer 2 Ready", StartScenarioId::Layer2Ready),
            ("LAYER2_READY", StartScenarioId::Layer2Ready),
            ("l2r", StartScenarioId::Layer2Ready),
            ("L2R", StartScenarioId::Layer2Ready),
            ("  layer2_ready  ", StartScenarioId::Layer2Ready),
        ] {
            assert_eq!(StartScenarioId::parse(input), Some(want), "input: {input}");
        }
    }

    #[test]
    fn parse_rejects_unknown_and_empty() {
        for input in ["", "   ", "bogus", "layer3", "ground_survival2", "l2"] {
            assert_eq!(StartScenarioId::parse(input), None, "input: {input}");
        }
    }
}
