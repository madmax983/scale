use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile::{EventOutcome, HypertileEvent, KeyCode};
use ratatui_hypertile_extras::HypertilePlugin;

use crate::ui::help::{guide_lines, FieldGuideUiState, PAGE_SCROLL_LINES};

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
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            crate::ui::help::render_field_guide(frame, frame.area(), &world);
        });
    }

    fn on_event(&mut self, event: &HypertileEvent) -> EventOutcome {
        let HypertileEvent::Key(chord) = event else {
            return EventOutcome::Ignored;
        };
        if !chord.modifiers.is_empty() {
            return EventOutcome::Ignored;
        }

        // Coarse bound: the renderer clamps to the exact visible window;
        // here we only keep the offset from running away.
        let total = u16::try_from(guide_lines().len()).unwrap_or(u16::MAX);
        let max_scroll = total.saturating_sub(1);

        let mut world = self.world.borrow_mut();
        if !world.contains_resource::<FieldGuideUiState>() {
            world.insert_resource(FieldGuideUiState::default());
        }
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
