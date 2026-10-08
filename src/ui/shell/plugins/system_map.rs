use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile_extras::HypertilePlugin;

pub struct SystemMapPlugin {
    world: SharedWorld,
}

impl SystemMapPlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for SystemMapPlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            crate::layer2::render::render_system_view(frame, frame.area(), &world);
        });
    }
}
