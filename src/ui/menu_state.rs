use crate::layer1::culture::origins::OriginKind;
use crate::shared::scenario::StartScenarioId;
use bevy_ecs::prelude::*;

/// Which new-game setup row the ←/→ keys currently edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuSelector {
    /// The start-scenario row.
    #[default]
    Scenario,
    /// The adventurer-origin row.
    Origin,
}

/// Resources for the Main Menu.
#[derive(Resource)]
pub struct MenuState {
    /// The index of the currently selected option.
    pub selected_index: usize,
    /// The list of menu options.
    pub options: Vec<String>,
    /// The currently selected built-in start scenario.
    pub selected_scenario: StartScenarioId,
    /// The chosen adventurer origin (`None` = "Surprise me", random roster).
    pub selected_origin: Option<OriginKind>,
    /// Which setup row ←/→ currently edits.
    pub selector: MenuSelector,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            selected_index: 0,
            options: vec!["Start Game".to_string(), "Quit".to_string()],
            selected_scenario: StartScenarioId::Classic,
            selected_origin: None,
            selector: MenuSelector::Scenario,
        }
    }
}

impl MenuState {
    /// Select the next option.
    #[allow(clippy::missing_const_for_fn)]
    pub fn next(&mut self) {
        if self.selected_index < self.options.len() - 1 {
            self.selected_index += 1;
        }
    }

    /// Select the previous option.
    #[allow(clippy::missing_const_for_fn)]
    pub fn prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Select the next start scenario.
    pub fn next_scenario(&mut self) {
        let scenarios = StartScenarioId::all();
        let current = scenarios
            .iter()
            .position(|id| *id == self.selected_scenario)
            .unwrap_or(0);
        if current + 1 < scenarios.len() {
            self.selected_scenario = scenarios[current + 1];
        }
    }

    /// Select the previous start scenario.
    pub fn prev_scenario(&mut self) {
        let scenarios = StartScenarioId::all();
        let current = scenarios
            .iter()
            .position(|id| *id == self.selected_scenario)
            .unwrap_or(0);
        if current > 0 {
            self.selected_scenario = scenarios[current - 1];
        }
    }

    /// Switch which setup row (scenario / origin) the ←/→ keys edit.
    pub fn cycle_selector(&mut self) {
        self.selector = match self.selector {
            MenuSelector::Scenario => MenuSelector::Origin,
            MenuSelector::Origin => MenuSelector::Scenario,
        };
    }

    /// Cycle the adventurer origin forward: Surprise me → each origin →
    /// back to Surprise me.
    pub fn next_origin(&mut self) {
        let all = OriginKind::all();
        self.selected_origin = match self.selected_origin {
            None => Some(all[0]),
            Some(current) => {
                let i = all.iter().position(|&k| k == current).unwrap_or(0);
                if i + 1 < all.len() {
                    Some(all[i + 1])
                } else {
                    None
                }
            }
        };
    }

    /// Cycle the adventurer origin backward: Surprise me ← each origin.
    pub fn prev_origin(&mut self) {
        let all = OriginKind::all();
        self.selected_origin = match self.selected_origin {
            None => Some(all[all.len() - 1]),
            Some(current) => {
                let i = all.iter().position(|&k| k == current).unwrap_or(0);
                if i > 0 {
                    Some(all[i - 1])
                } else {
                    None
                }
            }
        };
    }

    /// Player-facing label for the origin row.
    #[must_use]
    pub fn origin_label(&self) -> String {
        self.selected_origin
            .map(|k| k.name().to_string())
            .unwrap_or_else(|| "Surprise me".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::scenario::StartScenarioId;

    #[test]
    fn test_menu_state_resource() {
        let menu = MenuState::default();
        // Should default to first option (Start Game)
        assert_eq!(menu.selected_index, 0);
        assert_eq!(menu.selected_scenario, StartScenarioId::Classic);
        // Should have options
        assert!(menu.options.len() >= 2);
        assert_eq!(menu.options[0], "Start Game");
        assert_eq!(menu.options[1], "Quit");
    }

    #[test]
    fn test_menu_navigation_down() {
        let mut menu = MenuState::default();
        // default is 0

        menu.next();
        assert_eq!(menu.selected_index, 1);

        // Should wrap or clamp (design decision: clamp)
        menu.next();
        assert_eq!(menu.selected_index, 1); // Assuming 2 options
    }

    #[test]
    fn test_menu_navigation_up() {
        let mut menu = MenuState {
            selected_index: 1,
            ..Default::default()
        };

        menu.prev();
        assert_eq!(menu.selected_index, 0);

        menu.prev();
        assert_eq!(menu.selected_index, 0); // Clamp
    }

    #[test]
    fn test_menu_scenario_cycles_forward_and_back() {
        let mut menu = MenuState::default();

        menu.next_scenario();
        assert_eq!(menu.selected_scenario, StartScenarioId::GroundSurvival);

        menu.prev_scenario();
        assert_eq!(menu.selected_scenario, StartScenarioId::Classic);
    }

    #[test]
    fn test_menu_origin_defaults_to_surprise_me() {
        let menu = MenuState::default();
        assert_eq!(menu.selected_origin, None);
        assert_eq!(menu.origin_label(), "Surprise me");
        assert_eq!(menu.selector, MenuSelector::Scenario);
    }

    #[test]
    fn test_menu_origin_cycles_forward_and_back() {
        let mut menu = MenuState::default();

        menu.next_origin();
        assert_eq!(menu.selected_origin, Some(OriginKind::Sovereign));
        assert_eq!(menu.origin_label(), "Fallen Sovereign");

        menu.next_origin();
        assert_eq!(menu.selected_origin, Some(OriginKind::Corsair));

        menu.prev_origin();
        assert_eq!(menu.selected_origin, Some(OriginKind::Sovereign));

        // Backward from the first origin wraps to Surprise me.
        menu.prev_origin();
        assert_eq!(menu.selected_origin, None);

        // Backward from Surprise me wraps to the last origin.
        menu.prev_origin();
        assert_eq!(menu.selected_origin, Some(OriginKind::BloomTouched));

        // Forward from the last origin wraps to Surprise me.
        menu.next_origin();
        assert_eq!(menu.selected_origin, None);
    }

    #[test]
    fn test_menu_selector_cycles_with_tab() {
        let mut menu = MenuState::default();
        assert_eq!(menu.selector, MenuSelector::Scenario);

        menu.cycle_selector();
        assert_eq!(menu.selector, MenuSelector::Origin);

        menu.cycle_selector();
        assert_eq!(menu.selector, MenuSelector::Scenario);
    }
}
