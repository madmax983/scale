use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile::{EventOutcome, HypertileEvent, KeyCode};
use ratatui_hypertile_extras::HypertilePlugin;

use crate::ui::help::{FieldGuideUiState, PAGE_SCROLL_LINES};

/// Shell pane plugin for the Field Guide (see [`crate::ui::help`]).
///
/// Rendering delegates to `render_field_guide`; key events scroll the
/// guide while the pane is focused, following the same pattern as
/// [`super::TechPlugin`] (state lives in a world resource).
pub struct FieldGuidePlugin {
    world: SharedWorld,
}

impl FieldGuidePlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for FieldGuidePlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        // Ensure the state exists so the bound published below has a
        // home even before the first key press.
        {
            let mut world = self.world.borrow_mut();
            if !world.contains_resource::<FieldGuideUiState>() {
                world.insert_resource(FieldGuideUiState::default());
            }
        }
        let mut max_scroll = 0;
        {
            let world = self.world.borrow();
            render_with_frame(area, buf, |frame| {
                max_scroll = crate::ui::help::render_field_guide(frame, frame.area(), &world);
            });
        }
        // Publish the renderer's wrapped bound: `on_event` clamps
        // against this exact value, so key scrolling and rendering
        // can never disagree about where the guide ends (item 103).
        let mut world = self.world.borrow_mut();
        if let Some(mut state) = world.get_resource_mut::<FieldGuideUiState>() {
            state.max_scroll = max_scroll;
        }
    }

    fn on_event(&mut self, event: &HypertileEvent) -> EventOutcome {
        let HypertileEvent::Key(chord) = event else {
            return EventOutcome::Ignored;
        };
        if !chord.modifiers.is_empty() {
            return EventOutcome::Ignored;
        }

        let mut world = self.world.borrow_mut();
        if !world.contains_resource::<FieldGuideUiState>() {
            world.insert_resource(FieldGuideUiState::default());
        }
        // Clamp against the bound the renderer publishes each frame
        // (the wrapped row count at the real pane width, minus the
        // visible rows). Until the first render publishes one this is
        // 0 and keys are inert -- a focused pane has always rendered,
        // so that lasts a frame at most. Sharing the renderer's bound
        // is what stops End from overshooting into rows the renderer
        // cannot show, which made Up look broken (item 103).
        let max_scroll = world.resource::<FieldGuideUiState>().max_scroll;
        let mut state = world.resource_mut::<FieldGuideUiState>();
        match chord.code {
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => {
                state.scroll_by(-1, max_scroll);
                EventOutcome::Consumed
            }
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => {
                state.scroll_by(1, max_scroll);
                EventOutcome::Consumed
            }
            KeyCode::PageUp => {
                state.scroll_by(-PAGE_SCROLL_LINES, max_scroll);
                EventOutcome::Consumed
            }
            KeyCode::PageDown => {
                state.scroll_by(PAGE_SCROLL_LINES, max_scroll);
                EventOutcome::Consumed
            }
            KeyCode::Home => {
                state.scroll_to_top();
                EventOutcome::Consumed
            }
            KeyCode::End => {
                state.scroll_to_bottom(max_scroll);
                EventOutcome::Consumed
            }
            _ => EventOutcome::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::help::guide_lines;
    use bevy_ecs::prelude::*;
    use ratatui::buffer::Buffer;
    use ratatui_hypertile::{KeyChord, Modifiers};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Rows of guide text the renderer actually produces at `width`:
    /// render unscrolled into a very tall terminal and count down to
    /// the last row carrying text inside the bordered block. This is
    /// the ground truth both scroll clamps must agree with.
    fn empirical_wrapped_rows(width: u16) -> u16 {
        use crate::ui::help::render_field_guide;
        use ratatui::{backend::TestBackend, Terminal};
        let world = World::new();
        let backend = TestBackend::new(width, 900);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_field_guide(frame, frame.area(), &world);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut last_text_row = 0u16;
        // Inner rows of the content chunk only: skip the top border
        // (row 0); the bottom border and footer chunk are the last two
        // rows of the area.
        for y in 1..buffer.area.height - 2 {
            let row_has_text = (1..width - 1).any(|x| buffer[(x, y)].symbol().trim() != "");
            if row_has_text {
                last_text_row = y;
            }
        }
        // Inner rows start at y=1, so the last text row's index IS the
        // wrapped row count (the guide ends on a text line).
        last_text_row
    }

    fn key(code: KeyCode) -> HypertileEvent {
        HypertileEvent::Key(KeyChord::with_modifiers(code, Modifiers::NONE))
    }

    #[test]
    fn end_then_up_uses_the_renderers_true_max_scroll() {
        // Item 103: the key handler clamped against the UNWRAPPED line
        // count while the renderer clamped against its own bound, so
        // End overshot into rows the renderer could never show and Up
        // presses appeared dead. Both sides must share the wrapped
        // bound the renderer computes at the real pane width.
        let world: SharedWorld = Rc::new(RefCell::new(World::new()));
        world
            .borrow_mut()
            .insert_resource(FieldGuideUiState::default());
        let mut plugin = FieldGuidePlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 44, 14);
        let mut buffer = Buffer::empty(area);
        plugin.render(area, &mut buffer, true);

        // Visible inner rows at this geometry: 14 - 1 (footer) - 2 (borders).
        let expected_max = empirical_wrapped_rows(44) - 11;
        assert_eq!(
            world.borrow().resource::<FieldGuideUiState>().max_scroll,
            expected_max,
            "render must publish its wrapped max_scroll for the key handler"
        );
        assert!(
            usize::from(expected_max) > guide_lines().len(),
            "test geometry must wrap heavily (wrapped max beyond raw line count)"
        );

        let outcome = plugin.on_event(&key(KeyCode::End));
        assert!(matches!(outcome, EventOutcome::Consumed));
        assert_eq!(
            world.borrow().resource::<FieldGuideUiState>().scroll,
            expected_max,
            "End must stop at the renderer's wrapped max_scroll, not the raw line count"
        );

        let outcome = plugin.on_event(&key(KeyCode::Up));
        assert!(matches!(outcome, EventOutcome::Consumed));
        assert_eq!(
            world.borrow().resource::<FieldGuideUiState>().scroll,
            expected_max - 1,
            "one Up from the bottom must move exactly one row (no invisible overshoot)"
        );
    }
}
