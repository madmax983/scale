use super::{render_with_frame, SharedWorld};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile_extras::HypertilePlugin;

/// Shell pane for the Reign & Sky panels: Propaganda Constellation,
/// Degraded God-Mind, and Fallen Sovereign — the headless-console-only
/// mechanics given first-class terminal UI.
pub struct ReignSkyPlugin {
    world: SharedWorld,
}

impl ReignSkyPlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for ReignSkyPlugin {
    fn render(&self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            crate::ui::reign_sky::render_reign_sky(frame, frame.area(), &world);
        });
    }
}
