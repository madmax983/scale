//! Platform-agnostic input types.
//!
//! These types decouple the game's input handling from any specific backend
//! (crossterm for native, ratzilla for WASM). Each platform adapter implements
//! `From` conversions to translate platform-specific events into these types.

/// Platform-agnostic key codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameKeyCode {
    /// A character key.
    Char(char),
    /// Enter / Return.
    Enter,
    /// Escape.
    Esc,
    /// Tab.
    Tab,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Shift+Tab.
    BackTab,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
}

/// Platform-agnostic key modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GameKeyModifiers {
    /// Control key.
    pub ctrl: bool,
    /// Alt/Option key.
    pub alt: bool,
    /// Shift key.
    pub shift: bool,
}

impl GameKeyModifiers {
    /// Create modifiers from individual flags.
    #[must_use]
    pub const fn new(ctrl: bool, alt: bool, shift: bool) -> Self {
        Self { ctrl, alt, shift }
    }
}

/// A platform-agnostic keyboard event.
///
/// Only represents key-press events; repeat/release are filtered at the
/// platform translation boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GameKeyEvent {
    /// The key that was pressed.
    pub code: GameKeyCode,
    /// Modifier state active during the key event.
    pub modifiers: GameKeyModifiers,
}

impl GameKeyEvent {
    /// Create a new key event.
    #[must_use]
    pub const fn new(code: GameKeyCode) -> Self {
        Self {
            code,
            modifiers: GameKeyModifiers::new(false, false, false),
        }
    }

    /// Attach explicit modifier state.
    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: GameKeyModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Mark the event as control-modified.
    #[must_use]
    pub const fn with_ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    /// Mark the event as alt-modified.
    #[must_use]
    pub const fn with_alt(mut self) -> Self {
        self.modifiers.alt = true;
        self
    }

    /// Mark the event as shift-modified.
    #[must_use]
    pub const fn with_shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }
}

/// A platform-agnostic mouse button.
///
/// Only the two buttons the game binds are modelled: left is the
/// primary (selection/designation) button, right is the secondary
/// verb (clear selection). Platform translators drop any other
/// button (middle, back, forward, …) at the boundary rather than
/// letting it masquerade as a left click.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameMouseButton {
    /// Primary button.
    #[default]
    Left,
    /// Secondary button.
    Right,
}

/// A platform-agnostic mouse click event.
///
/// Represents click-down events only; releases and moves are filtered
/// at the platform translation boundary. [`GameMouseEvent::new`]
/// defaults to the left button, so every constructor call site that
/// predates right-click support keeps its exact meaning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GameMouseEvent {
    /// Screen column (x coordinate).
    pub x: u16,
    /// Screen row (y coordinate).
    pub y: u16,
    /// Which button was pressed.
    pub button: GameMouseButton,
}

impl GameMouseEvent {
    /// Create a new (left-button) mouse event.
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self {
            x,
            y,
            button: GameMouseButton::Left,
        }
    }

    /// Create a new mouse event with an explicit button.
    #[must_use]
    pub const fn new_with_button(x: u16, y: u16, button: GameMouseButton) -> Self {
        Self { x, y, button }
    }

    /// Attach an explicit button to this event.
    #[must_use]
    pub const fn with_button(mut self, button: GameMouseButton) -> Self {
        self.button = button;
        self
    }

    /// Whether this is a right (secondary) click.
    #[must_use]
    pub const fn is_right_click(&self) -> bool {
        matches!(self.button, GameMouseButton::Right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_key_event_creation() {
        let event = GameKeyEvent::new(GameKeyCode::Char('q'));
        assert_eq!(event.code, GameKeyCode::Char('q'));
        assert_eq!(event.modifiers, GameKeyModifiers::default());
    }

    #[test]
    fn test_game_key_event_with_modifiers() {
        let mods = GameKeyModifiers::new(true, true, true);
        let event = GameKeyEvent::new(GameKeyCode::Char('m')).with_modifiers(mods);
        assert!(event.modifiers.ctrl);
        assert!(event.modifiers.alt);
        assert!(event.modifiers.shift);
    }

    #[test]
    fn test_game_key_event_with_ctrl() {
        let event = GameKeyEvent::new(GameKeyCode::Char('k')).with_ctrl();
        assert!(event.modifiers.ctrl);
        assert!(!event.modifiers.alt);
        assert!(!event.modifiers.shift);
    }

    #[test]
    fn test_game_key_event_with_alt() {
        let event = GameKeyEvent::new(GameKeyCode::Char('a')).with_alt();
        assert!(!event.modifiers.ctrl);
        assert!(event.modifiers.alt);
        assert!(!event.modifiers.shift);
    }

    #[test]
    fn test_game_key_event_with_shift() {
        let event = GameKeyEvent::new(GameKeyCode::Char('s')).with_shift();
        assert!(!event.modifiers.ctrl);
        assert!(!event.modifiers.alt);
        assert!(event.modifiers.shift);
    }

    #[test]
    fn test_game_key_code_equality() {
        assert_eq!(GameKeyCode::Char('a'), GameKeyCode::Char('a'));
        assert_ne!(GameKeyCode::Char('a'), GameKeyCode::Char('b'));
        assert_ne!(GameKeyCode::Enter, GameKeyCode::Esc);
    }

    #[test]
    fn test_game_key_event_is_copy() {
        let event = GameKeyEvent::new(GameKeyCode::Enter);
        let copy = event;
        assert_eq!(event, copy);
    }

    #[test]
    fn test_game_mouse_event_creation() {
        let event = GameMouseEvent::new(10, 20);
        assert_eq!(event.x, 10);
        assert_eq!(event.y, 20);
    }

    #[test]
    fn test_game_mouse_event_is_copy() {
        let event = GameMouseEvent::new(5, 5);
        let copy = event;
        assert_eq!(event, copy);
    }

    #[test]
    fn test_game_mouse_event_defaults_to_left_button() {
        // Backwards compatibility: every pre-existing constructor call
        // site built left clicks, so `new` must keep meaning left.
        let event = GameMouseEvent::new(10, 20);
        assert_eq!(event.button, GameMouseButton::Left);
        assert_eq!(GameMouseButton::default(), GameMouseButton::Left);
    }

    #[test]
    fn test_game_mouse_event_carries_right_button() {
        let event = GameMouseEvent::new(10, 20).with_button(GameMouseButton::Right);
        assert_eq!(event.x, 10);
        assert_eq!(event.y, 20);
        assert_eq!(event.button, GameMouseButton::Right);
        assert_ne!(event, GameMouseEvent::new(10, 20));

        let direct = GameMouseEvent::new_with_button(3, 4, GameMouseButton::Right);
        assert_eq!(direct.button, GameMouseButton::Right);
        assert_eq!((direct.x, direct.y), (3, 4));
    }

    #[test]
    fn test_all_key_code_variants() {
        // Ensure all variants are constructible and distinct
        let codes = [
            GameKeyCode::Char('x'),
            GameKeyCode::Enter,
            GameKeyCode::Esc,
            GameKeyCode::Tab,
            GameKeyCode::Up,
            GameKeyCode::Down,
            GameKeyCode::Left,
            GameKeyCode::Right,
            GameKeyCode::Backspace,
            GameKeyCode::Delete,
            GameKeyCode::BackTab,
            GameKeyCode::PageUp,
            GameKeyCode::PageDown,
            GameKeyCode::Home,
            GameKeyCode::End,
        ];
        // Each variant should be distinct from the others
        for (i, a) in codes.iter().enumerate() {
            for (j, b) in codes.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Variants at {i} and {j} should differ");
                }
            }
        }
    }

    #[test]
    fn test_page_home_end_key_codes_exist_and_are_distinct() {
        // Item 124: PageUp/PageDown/Home/End must be representable in
        // the platform-agnostic key type, or no adapter can ever
        // deliver them to the Field Guide's scroll handlers.
        let codes = [
            GameKeyCode::PageUp,
            GameKeyCode::PageDown,
            GameKeyCode::Home,
            GameKeyCode::End,
        ];
        for (i, a) in codes.iter().enumerate() {
            for (j, b) in codes.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Variants at {i} and {j} should differ");
                }
            }
        }
        assert_ne!(GameKeyCode::PageUp, GameKeyCode::Up);
        assert_ne!(GameKeyCode::PageDown, GameKeyCode::Down);
        // Events carrying the new codes keep their modifiers.
        let event = GameKeyEvent::new(GameKeyCode::PageDown);
        assert_eq!(event.code, GameKeyCode::PageDown);
        assert_eq!(event.modifiers, GameKeyModifiers::default());
    }

    #[test]
    fn test_input_press() {
        let mut input = Input::default();

        // Initial state
        assert!(!input.pressed(KeyCode::W));
        assert!(!input.just_pressed(KeyCode::W));

        // Pressing a key
        input.press(KeyCode::W);
        assert!(input.pressed(KeyCode::W));
        assert!(input.just_pressed(KeyCode::W));

        // Pressing it again shouldn't trigger just_pressed again (if it was cleared, but here it's not cleared between calls)
        // Wait, the logic is: if !pressed.contains, insert into just_pressed.
        // So pressing again without clearing just keeps it in pressed, and just_pressed isn't modified.
        // Since we didn't clear, just_pressed still has it.
        input.press(KeyCode::W);
        assert!(input.pressed(KeyCode::W));
        assert!(input.just_pressed(KeyCode::W));
    }

    #[test]
    fn test_input_release() {
        let mut input = Input::default();
        input.press(KeyCode::W);
        assert!(input.pressed(KeyCode::W));
        assert!(!input.just_released(KeyCode::W));

        // Releasing
        input.release(KeyCode::W);
        assert!(!input.pressed(KeyCode::W));
        assert!(input.just_released(KeyCode::W));

        // Releasing again does nothing
        input.release(KeyCode::W);
        assert!(!input.pressed(KeyCode::W));
    }

    #[test]
    fn test_input_clear() {
        let mut input = Input::default();
        input.press(KeyCode::W);
        input.press(KeyCode::A);
        input.release(KeyCode::A);

        assert!(input.just_pressed(KeyCode::W));
        assert!(input.just_pressed(KeyCode::A));
        assert!(input.pressed(KeyCode::W));
        assert!(!input.pressed(KeyCode::A));
        assert!(input.just_released(KeyCode::A));

        input.clear();

        assert!(!input.just_pressed(KeyCode::W));
        assert!(!input.just_pressed(KeyCode::A));
        assert!(!input.pressed(KeyCode::W));
        assert!(!input.just_released(KeyCode::A));
    }
}

use std::collections::HashSet;
#[derive(bevy_ecs::system::Resource, Default)]
pub struct Input {
    pressed: HashSet<KeyCode>,
    just_pressed: HashSet<KeyCode>,
    just_released: HashSet<KeyCode>,
}

impl Input {
    pub fn press(&mut self, input: KeyCode) {
        if !self.pressed.contains(&input) {
            self.just_pressed.insert(input);
        }
        self.pressed.insert(input);
    }

    pub fn release(&mut self, input: KeyCode) {
        if self.pressed.contains(&input) {
            self.pressed.remove(&input);
            self.just_released.insert(input);
        }
    }

    pub fn pressed(&self, input: KeyCode) -> bool {
        self.pressed.contains(&input)
    }

    pub fn just_pressed(&self, input: KeyCode) -> bool {
        self.just_pressed.contains(&input)
    }

    pub fn just_released(&self, input: KeyCode) -> bool {
        self.just_released.contains(&input)
    }

    pub fn clear(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        // Since the underlying platform (crossterm) does not reliably send release events
        // or we filter them out, we treat all inputs as transient triggers.
        // We must clear 'pressed' so that the next frame's press is registered as a new press.
        self.pressed.clear();
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyCode {
    W,
    A,
    S,
    D,
    Q,
    E,
    R,
    F,
    Up,
    Down,
    Left,
    Right,
    Space,
    Esc,
    Return,
    Tab,
    Back,
    Delete,
    #[default]
    Unidentified,
}
