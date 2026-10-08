use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile_extras::HypertilePlugin;

pub struct ChroniclePlugin {
    world: SharedWorld,
}

impl ChroniclePlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for ChroniclePlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            crate::ui::chronicle::render_chronicle(frame, frame.area(), &world);
        });
    }
}
