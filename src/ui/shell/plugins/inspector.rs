use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile_extras::HypertilePlugin;

pub struct InspectorPlugin {
    world: SharedWorld,
}

impl InspectorPlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for InspectorPlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            // Preserve the current inspector + log composition until the shell owns
            // log/inspector as separate panes.
            crate::ui::panels::render_info_panel(frame, frame.area(), &world);
        });
    }
}
