//! Native platform adapter (crossterm → `GameKeyEvent` / `GameMouseEvent`).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};

use crate::shared::keyboard::{
    GameKeyCode, GameKeyEvent, GameKeyModifiers, GameMouseButton, GameMouseEvent,
};

/// Convert a crossterm `KeyEvent` to a platform-agnostic `GameKeyEvent`.
///
/// Only `KeyEventKind::Press` events are translated. On Windows, crossterm
/// also sends `Release` and `Repeat` events which would double-fire inputs.
impl TryFrom<KeyEvent> for GameKeyEvent {
    type Error = ();

    fn try_from(key: KeyEvent) -> Result<Self, Self::Error> {
        if key.kind != KeyEventKind::Press {
            return Err(());
        }
        let code = match key.code {
            KeyCode::Char(c) => GameKeyCode::Char(c),
            KeyCode::Enter => GameKeyCode::Enter,
            KeyCode::Esc => GameKeyCode::Esc,
            KeyCode::Tab => GameKeyCode::Tab,
            KeyCode::BackTab => GameKeyCode::BackTab,
            KeyCode::Up => GameKeyCode::Up,
            KeyCode::Down => GameKeyCode::Down,
            KeyCode::Left => GameKeyCode::Left,
            KeyCode::Right => GameKeyCode::Right,
            KeyCode::Backspace => GameKeyCode::Backspace,
            KeyCode::Delete => GameKeyCode::Delete,
            _ => return Err(()),
        };
        let modifiers = GameKeyModifiers::new(
            key.modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL),
            key.modifiers.contains(crossterm::event::KeyModifiers::ALT),
            key.modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT),
        );
        Ok(Self { code, modifiers })
    }
}

/// Convert a crossterm `MouseEvent` to a platform-agnostic `GameMouseEvent`.
///
/// Only left- and right-button mouse-down events are translated, with
/// the button preserved; all other events (moves, releases, middle or
/// other buttons) return `Err`.
impl TryFrom<MouseEvent> for GameMouseEvent {
    type Error = ();

    fn try_from(mouse: MouseEvent) -> Result<Self, Self::Error> {
        let button = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => GameMouseButton::Left,
            MouseEventKind::Down(MouseButton::Right) => GameMouseButton::Right,
            _ => return Err(()),
        };
        Ok(Self::new_with_button(mouse.column, mouse.row, button))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyModifiers, MouseButton};

    fn crossterm_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn test_translate_char_key() {
        let key = crossterm_key(KeyCode::Char('q'));
        let game_key = GameKeyEvent::try_from(key).unwrap();
        assert_eq!(game_key.code, GameKeyCode::Char('q'));
        assert_eq!(game_key.modifiers, GameKeyModifiers::default());
    }

    #[test]
    fn test_translate_ctrl_modifier() {
        let key = KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::CONTROL);
        let game_key = GameKeyEvent::try_from(key).unwrap();
        assert!(game_key.modifiers.ctrl);
        assert!(!game_key.modifiers.alt);
        assert!(!game_key.modifiers.shift);
    }

    #[test]
    fn test_translate_enter() {
        let key = crossterm_key(KeyCode::Enter);
        let game_key = GameKeyEvent::try_from(key).unwrap();
        assert_eq!(game_key.code, GameKeyCode::Enter);
    }

    #[test]
    fn test_translate_esc() {
        let key = crossterm_key(KeyCode::Esc);
        let game_key = GameKeyEvent::try_from(key).unwrap();
        assert_eq!(game_key.code, GameKeyCode::Esc);
    }

    #[test]
    fn test_translate_arrows() {
        assert_eq!(
            GameKeyEvent::try_from(crossterm_key(KeyCode::Up))
                .unwrap()
                .code,
            GameKeyCode::Up
        );
        assert_eq!(
            GameKeyEvent::try_from(crossterm_key(KeyCode::Down))
                .unwrap()
                .code,
            GameKeyCode::Down
        );
        assert_eq!(
            GameKeyEvent::try_from(crossterm_key(KeyCode::Left))
                .unwrap()
                .code,
            GameKeyCode::Left
        );
        assert_eq!(
            GameKeyEvent::try_from(crossterm_key(KeyCode::Right))
                .unwrap()
                .code,
            GameKeyCode::Right
        );
    }

    #[test]
    fn test_translate_tab() {
        let game_key = GameKeyEvent::try_from(crossterm_key(KeyCode::Tab)).unwrap();
        assert_eq!(game_key.code, GameKeyCode::Tab);
    }

    #[test]
    fn test_translate_backtab() {
        let game_key = GameKeyEvent::try_from(crossterm_key(KeyCode::BackTab)).unwrap();
        assert_eq!(game_key.code, GameKeyCode::BackTab);
    }

    #[test]
    fn test_translate_backspace_delete() {
        let bs = GameKeyEvent::try_from(crossterm_key(KeyCode::Backspace)).unwrap();
        assert_eq!(bs.code, GameKeyCode::Backspace);

        let del = GameKeyEvent::try_from(crossterm_key(KeyCode::Delete)).unwrap();
        assert_eq!(del.code, GameKeyCode::Delete);
    }

    #[test]
    fn test_unhandled_key_returns_none() {
        let result = GameKeyEvent::try_from(crossterm_key(KeyCode::F(1)));
        assert!(result.is_err());
    }

    #[test]
    fn test_key_release_ignored() {
        let key = KeyEvent::new_with_kind(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
            KeyEventKind::Release,
        );
        assert!(GameKeyEvent::try_from(key).is_err());
    }

    #[test]
    fn test_key_repeat_ignored() {
        let key = KeyEvent::new_with_kind(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
            KeyEventKind::Repeat,
        );
        assert!(GameKeyEvent::try_from(key).is_err());
    }

    /// Regression test for Windows double-fire bug.
    ///
    /// On Windows, crossterm sends Press + Release for every keypress.
    /// Without filtering, Space would toggle pause ON (Press) then
    /// immediately OFF (Release), making pause appear broken.
    ///
    /// This test feeds raw crossterm events through the full pipeline:
    /// crossterm `KeyEvent` → `TryFrom` → `InputRouter` → `GameState`
    #[test]
    fn test_windows_press_release_does_not_double_toggle_pause() {
        use crate::layer1::{BuildMode, DesignationMode, Viewport};
        use crate::shared::state::GameState;
        use crate::shared::time::SimulationTime;
        use crate::ui::input::{route_input, InputContext, InputContextStack};
        use crate::ui::menu_state::MenuState;
        use crate::ui::selection::Selection;

        let mut world = bevy_ecs::prelude::World::new();
        world.insert_resource(GameState::Running);
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Viewport::default());
        world.insert_resource(BuildMode::default());
        world.insert_resource(DesignationMode::default());
        world.insert_resource(MenuState::default());
        world.insert_resource(Selection::default());
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);

        // Simulate Windows keypress: Press then Release
        let press = KeyEvent::new_with_kind(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
            KeyEventKind::Press,
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
            KeyEventKind::Release,
        );

        // Feed both events through the same path as main.rs
        if let Ok(game_key) = GameKeyEvent::try_from(press) {
            route_input(&mut world, game_key);
        }
        if let Ok(game_key) = GameKeyEvent::try_from(release) {
            route_input(&mut world, game_key);
        }

        // Should be Paused — not toggled back to Running
        assert_eq!(
            *world.resource::<GameState>(),
            GameState::Paused,
            "Press+Release should only toggle once (Press), not double-toggle"
        );
    }

    #[test]
    fn test_translate_mouse_down() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        let game_mouse = GameMouseEvent::try_from(mouse).unwrap();
        assert_eq!(game_mouse.x, 10);
        assert_eq!(game_mouse.y, 20);
        assert_eq!(
            game_mouse.button,
            crate::shared::keyboard::GameMouseButton::Left
        );
    }

    #[test]
    fn test_translate_mouse_right_down() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 7,
            row: 9,
            modifiers: KeyModifiers::empty(),
        };
        let game_mouse = GameMouseEvent::try_from(mouse).unwrap();
        assert_eq!(game_mouse.x, 7);
        assert_eq!(game_mouse.y, 9);
        assert_eq!(
            game_mouse.button,
            crate::shared::keyboard::GameMouseButton::Right,
            "a right click must survive translation as a right click, not select like a left click"
        );
    }

    #[test]
    fn test_translate_mouse_middle_down_ignored() {
        // The shared type models left/right only; other buttons are
        // dropped at the boundary instead of masquerading as left.
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Middle),
            column: 10,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        assert!(GameMouseEvent::try_from(mouse).is_err());
    }

    #[test]
    fn test_translate_mouse_right_up_ignored() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Right),
            column: 10,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        assert!(GameMouseEvent::try_from(mouse).is_err());
    }

    #[test]
    fn test_translate_mouse_move_ignored() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 10,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        assert!(GameMouseEvent::try_from(mouse).is_err());
    }

    #[test]
    fn test_translate_mouse_up_ignored() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        assert!(GameMouseEvent::try_from(mouse).is_err());
    }
}
