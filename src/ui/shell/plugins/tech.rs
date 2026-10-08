use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile::{EventOutcome, HypertileEvent, KeyCode};
use ratatui_hypertile_extras::HypertilePlugin;

pub struct TechPlugin {
    world: SharedWorld,
}

impl TechPlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for TechPlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            crate::ui::tech::render_tech_tree(frame, frame.area(), &world);
        });
    }

    fn on_event(&mut self, event: &HypertileEvent) -> EventOutcome {
        let HypertileEvent::Key(chord) = event else {
            return EventOutcome::Ignored;
        };
        if !chord.modifiers.is_empty() {
            return EventOutcome::Ignored;
        }

        let tech_count = crate::ui::tech::get_tech_list().len();
        let mut world = self.world.borrow_mut();
        match chord.code {
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => {
                world
                    .resource_mut::<crate::ui::tech::TechUiState>()
                    .prev(tech_count);
                EventOutcome::Consumed
            }
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => {
                world
                    .resource_mut::<crate::ui::tech::TechUiState>()
                    .next(tech_count);
                EventOutcome::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let selected_index = world
                    .resource::<crate::ui::tech::TechUiState>()
                    .selected_index;
                let techs = crate::ui::tech::get_tech_list();
                if let Some(tech) = techs.get(selected_index).copied() {
                    crate::layer1::tech::unlock_tech(&mut world, tech);
                }
                EventOutcome::Consumed
            }
            KeyCode::Escape => EventOutcome::Consumed,
            _ => EventOutcome::Ignored,
        }
    }
}
