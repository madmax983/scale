//! WASM platform adapter (ratzilla → `GameKeyEvent` / `GameMouseEvent`).

use ratzilla::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use crate::shared::keyboard::{
    GameKeyCode, GameKeyEvent, GameKeyModifiers, GameMouseButton, GameMouseEvent,
};

/// Convert a ratzilla `KeyEvent` to a platform-agnostic `GameKeyEvent`.
///
/// Returns `Err(())` for keys we don't handle.
impl TryFrom<KeyEvent> for GameKeyEvent {
    type Error = ();

    fn try_from(key: KeyEvent) -> Result<Self, Self::Error> {
        let code = match key.code {
            KeyCode::Char(c) => GameKeyCode::Char(c),
            KeyCode::Enter => GameKeyCode::Enter,
            KeyCode::Esc => GameKeyCode::Esc,
            KeyCode::Tab => GameKeyCode::Tab,
            // KeyCode::BackTab => GameKeyCode::BackTab, // Not supported in ratzilla 0.3
            KeyCode::Up => GameKeyCode::Up,
            KeyCode::Down => GameKeyCode::Down,
            KeyCode::Left => GameKeyCode::Left,
            KeyCode::Right => GameKeyCode::Right,
            KeyCode::Backspace => GameKeyCode::Backspace,
            KeyCode::Delete => GameKeyCode::Delete,
            _ => return Err(()),
        };
        let modifiers = GameKeyModifiers::new(key.ctrl, key.alt, key.shift);
        Ok(Self { code, modifiers })
    }
}

/// Convert a ratzilla `MouseEvent` to a platform-agnostic `GameMouseEvent`.
///
/// Only left- and right-button `ButtonDown` events are translated, with
/// the button preserved; all other events (moves, releases, clicks —
/// ratzilla also synthesises those from the same press — middle or
/// other buttons) return `Err`, so one physical press routes once.
impl TryFrom<MouseEvent> for GameMouseEvent {
    type Error = ();

    fn try_from(mouse: MouseEvent) -> Result<Self, Self::Error> {
        let button = match mouse.kind {
            MouseEventKind::ButtonDown(MouseButton::Left) => GameMouseButton::Left,
            MouseEventKind::ButtonDown(MouseButton::Right) => GameMouseButton::Right,
            _ => return Err(()),
        };
        Ok(Self::new_with_button(mouse.col, mouse.row, button))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::keyboard::GameMouseButton;
    use ratzilla::event::MouseButton;

    fn ratzilla_mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            col,
            row,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    #[test]
    fn test_translate_mouse_left_down() {
        let game_mouse = GameMouseEvent::try_from(ratzilla_mouse(
            MouseEventKind::ButtonDown(MouseButton::Left),
            10,
            20,
        ))
        .unwrap();
        assert_eq!((game_mouse.x, game_mouse.y), (10, 20));
        assert_eq!(game_mouse.button, GameMouseButton::Left);
    }

    #[test]
    fn test_translate_mouse_right_down() {
        let game_mouse = GameMouseEvent::try_from(ratzilla_mouse(
            MouseEventKind::ButtonDown(MouseButton::Right),
            7,
            9,
        ))
        .unwrap();
        assert_eq!((game_mouse.x, game_mouse.y), (7, 9));
        assert_eq!(
            game_mouse.button,
            GameMouseButton::Right,
            "a right click must survive translation as a right click, not select like a left click"
        );
    }

    #[test]
    fn test_translate_mouse_middle_down_ignored() {
        assert!(GameMouseEvent::try_from(ratzilla_mouse(
            MouseEventKind::ButtonDown(MouseButton::Middle),
            10,
            20,
        ))
        .is_err());
    }

    #[test]
    fn test_translate_mouse_right_up_ignored() {
        assert!(GameMouseEvent::try_from(ratzilla_mouse(
            MouseEventKind::ButtonUp(MouseButton::Right),
            10,
            20,
        ))
        .is_err());
    }

    #[test]
    fn test_translate_ctrl_modifier() {
        let key = KeyEvent {
            code: KeyCode::Char('k'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        let game_key = GameKeyEvent::try_from(key).unwrap();
        assert!(game_key.modifiers.ctrl);
        assert!(!game_key.modifiers.alt);
        assert!(!game_key.modifiers.shift);
    }
}
