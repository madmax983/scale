//! UI Shell Plugins.
//!
//! Registers various UI panes (Chronicle, Maps, Inspector) into the Hypertile runtime environment.
//!
//! UI Shell Plugins.
//!
//! UI Shell Plugins.
//!
use bevy_ecs::prelude::*;
use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Terminal};
use ratatui_hypertile_extras::{HypertileRuntime, Registry};
use std::{cell::RefCell, rc::Rc};

mod chronicle;
mod colony_map;
mod inspector;
mod log_plugin;
#[cfg(feature = "nova")]
mod oral_tradition;
mod reign_sky;
mod status;
mod system_map;
mod tech;

pub use chronicle::ChroniclePlugin;
pub use colony_map::ColonyMapPlugin;
pub use inspector::InspectorPlugin;
pub use log_plugin::LogPlugin;
#[cfg(feature = "nova")]
pub use oral_tradition::OralTraditionPlugin;
pub use reign_sky::ReignSkyPlugin;
pub use status::StatusPlugin;
pub use system_map::SystemMapPlugin;
pub use tech::TechPlugin;

pub const COLONY_MAP_PLUGIN_TYPE: &str = "colony-map";
pub const SYSTEM_MAP_PLUGIN_TYPE: &str = "system-map";
pub const INSPECTOR_PLUGIN_TYPE: &str = "inspector";
pub const STATUS_PLUGIN_TYPE: &str = "status";
pub const CHRONICLE_PLUGIN_TYPE: &str = "chronicle";
pub const LOG_PLUGIN_TYPE: &str = "log";
pub const TECH_PLUGIN_TYPE: &str = "tech";
pub const REIGN_SKY_PLUGIN_TYPE: &str = "reign-sky";
#[cfg(feature = "nova")]
pub const ORAL_TRADITION_PLUGIN_TYPE: &str = "oral-tradition";

pub type SharedWorld = Rc<RefCell<World>>;

pub fn register_default_plugins(registry: &mut Registry, world: SharedWorld) {
    let colony_world = Rc::clone(&world);
    registry.register_plugin_type(COLONY_MAP_PLUGIN_TYPE, move || {
        ColonyMapPlugin::new(Rc::clone(&colony_world))
    });

    let system_world = Rc::clone(&world);
    registry.register_plugin_type(SYSTEM_MAP_PLUGIN_TYPE, move || {
        SystemMapPlugin::new(Rc::clone(&system_world))
    });

    let inspector_world = Rc::clone(&world);
    registry.register_plugin_type(INSPECTOR_PLUGIN_TYPE, move || {
        InspectorPlugin::new(Rc::clone(&inspector_world))
    });

    let status_world = Rc::clone(&world);
    registry.register_plugin_type(STATUS_PLUGIN_TYPE, move || {
        StatusPlugin::new(Rc::clone(&status_world))
    });

    let chronicle_world = Rc::clone(&world);
    registry.register_plugin_type(CHRONICLE_PLUGIN_TYPE, move || {
        ChroniclePlugin::new(Rc::clone(&chronicle_world))
    });

    let tech_world = Rc::clone(&world);
    registry.register_plugin_type(TECH_PLUGIN_TYPE, move || {
        TechPlugin::new(Rc::clone(&tech_world))
    });

    #[cfg(feature = "nova")]
    {
        let oral_tradition_world = Rc::clone(&world);
        registry.register_plugin_type(ORAL_TRADITION_PLUGIN_TYPE, move || {
            OralTraditionPlugin::new(Rc::clone(&oral_tradition_world))
        });
    }

    let log_world = Rc::clone(&world);
    registry.register_plugin_type(LOG_PLUGIN_TYPE, move || {
        LogPlugin::new(Rc::clone(&log_world))
    });

    let reign_sky_world = Rc::clone(&world);
    registry.register_plugin_type(REIGN_SKY_PLUGIN_TYPE, move || {
        ReignSkyPlugin::new(Rc::clone(&reign_sky_world))
    });
}

pub(crate) fn register_default_plugins_with_runtime(
    runtime: &mut HypertileRuntime,
    world: SharedWorld,
) {
    let colony_world = Rc::clone(&world);
    runtime.register_plugin_type(COLONY_MAP_PLUGIN_TYPE, move || {
        ColonyMapPlugin::new(Rc::clone(&colony_world))
    });

    let system_world = Rc::clone(&world);
    runtime.register_plugin_type(SYSTEM_MAP_PLUGIN_TYPE, move || {
        SystemMapPlugin::new(Rc::clone(&system_world))
    });

    let inspector_world = Rc::clone(&world);
    runtime.register_plugin_type(INSPECTOR_PLUGIN_TYPE, move || {
        InspectorPlugin::new(Rc::clone(&inspector_world))
    });

    let status_world = Rc::clone(&world);
    runtime.register_plugin_type(STATUS_PLUGIN_TYPE, move || {
        StatusPlugin::new(Rc::clone(&status_world))
    });

    let chronicle_world = Rc::clone(&world);
    runtime.register_plugin_type(CHRONICLE_PLUGIN_TYPE, move || {
        ChroniclePlugin::new(Rc::clone(&chronicle_world))
    });

    let tech_world = Rc::clone(&world);
    runtime.register_plugin_type(TECH_PLUGIN_TYPE, move || {
        TechPlugin::new(Rc::clone(&tech_world))
    });

    #[cfg(feature = "nova")]
    {
        let oral_tradition_world = Rc::clone(&world);
        runtime.register_plugin_type(ORAL_TRADITION_PLUGIN_TYPE, move || {
            OralTraditionPlugin::new(Rc::clone(&oral_tradition_world))
        });
    }

    let log_world = Rc::clone(&world);
    runtime.register_plugin_type(LOG_PLUGIN_TYPE, move || {
        LogPlugin::new(Rc::clone(&log_world))
    });

    let reign_sky_world = Rc::clone(&world);
    runtime.register_plugin_type(REIGN_SKY_PLUGIN_TYPE, move || {
        ReignSkyPlugin::new(Rc::clone(&reign_sky_world))
    });
}

fn render_with_frame<F>(area: Rect, buf: &mut Buffer, render: F)
where
    F: FnOnce(&mut ratatui::Frame),
{
    if area.width == 0 || area.height == 0 {
        return;
    }

    let backend = TestBackend::new(area.width, area.height);
    let mut terminal =
        Terminal::new(backend).expect("shell plugins should create an offscreen terminal");
    terminal
        .draw(|frame| render(frame))
        .expect("shell plugins should render into the offscreen terminal");

    let rendered = terminal.backend().buffer();
    copy_buffer_into_area(rendered, area, buf);
}

fn copy_buffer_into_area(source: &Buffer, target_area: Rect, target: &mut Buffer) {
    for y in 0..source.area.height {
        for x in 0..source.area.width {
            let Some(source_cell) = source.cell((x, y)) else {
                continue;
            };
            let Some(target_cell) = target.cell_mut((target_area.x + x, target_area.y + y)) else {
                continue;
            };
            *target_cell = source_cell.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "nova")]
    use super::ORAL_TRADITION_PLUGIN_TYPE;
    use super::{
        register_default_plugins, ChroniclePlugin, ReignSkyPlugin, SharedWorld, StatusPlugin,
        TechPlugin, CHRONICLE_PLUGIN_TYPE, COLONY_MAP_PLUGIN_TYPE, INSPECTOR_PLUGIN_TYPE,
        LOG_PLUGIN_TYPE, REIGN_SKY_PLUGIN_TYPE, STATUS_PLUGIN_TYPE, SYSTEM_MAP_PLUGIN_TYPE,
        TECH_PLUGIN_TYPE,
    };
    use crate::prelude::{setup_world_with_config, SetupConfig};
    use ratatui::{buffer::Buffer, layout::Rect};
    use ratatui_hypertile_extras::{HypertilePlugin, Registry};
    use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

    #[test]
    fn register_default_plugins_registers_expected_types() {
        let world = test_world();
        let mut registry = Registry::default();

        register_default_plugins(&mut registry, world);

        let actual = registry.registered_types().collect::<BTreeSet<_>>();
        #[allow(unused_mut)]
        let mut expected = BTreeSet::from([
            CHRONICLE_PLUGIN_TYPE,
            COLONY_MAP_PLUGIN_TYPE,
            INSPECTOR_PLUGIN_TYPE,
            STATUS_PLUGIN_TYPE,
            SYSTEM_MAP_PLUGIN_TYPE,
            TECH_PLUGIN_TYPE,
            LOG_PLUGIN_TYPE,
            REIGN_SKY_PLUGIN_TYPE,
        ]);
        #[cfg(feature = "nova")]
        expected.insert(ORAL_TRADITION_PLUGIN_TYPE);

        assert_eq!(actual, expected);
    }

    #[test]
    fn registry_instantiated_status_plugin_renders_legacy_status_bar() {
        let world = test_world();
        let mut registry = Registry::default();
        register_default_plugins(&mut registry, world);

        let mut plugin = registry
            .instantiate_plugin(STATUS_PLUGIN_TYPE)
            .expect("status plugin should be registered");
        let area = Rect::new(0, 0, 48, 3);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(
            text.contains("Day") || text.contains("Souls"),
            "registry-created status plugin should delegate to the legacy status renderer"
        );
    }

    #[test]
    fn status_plugin_renders_inside_small_rect() {
        let world = test_world();
        let mut plugin = StatusPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 48, 3);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, true);

        let text = buffer_text(&buffer);
        assert!(
            text.contains("Day") || text.contains("Souls"),
            "status pane should still render summary data in a narrow pane"
        );
    }

    #[test]
    fn chronicle_plugin_renders_even_when_overlay_state_is_closed() {
        let world = test_world();

        let mut plugin = ChroniclePlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 80, 24);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(
            text.contains("Chronicle"),
            "chronicle pane should render as a normal pane without overlay gating"
        );
    }

    #[test]
    fn reign_sky_plugin_renders_all_three_panels() {
        let world = test_world();

        let mut plugin = ReignSkyPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 80, 30);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(
            text.contains("Constellation"),
            "reign-sky pane should render the constellation panel"
        );
        assert!(
            text.contains("God-Mind"),
            "reign-sky pane should render the god-mind panel"
        );
        assert!(
            text.contains("Sovereign"),
            "reign-sky pane should render the sovereign panel"
        );
    }

    #[test]
    fn tech_plugin_renders_even_when_overlay_state_is_closed() {
        let world = test_world();

        let mut plugin = TechPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(
            text.contains("Technology Tree"),
            "tech pane should render as a normal pane without overlay gating"
        );
    }

    fn test_world() -> SharedWorld {
        Rc::new(RefCell::new(setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        })))
    }

    fn buffer_text(buffer: &Buffer) -> String {
        // ⚡ Bolt Optimization: Replace intermediate .collect::<String>() chain with pre-allocated String loop
        let mut s = String::with_capacity(buffer.area.area() as usize);
        for cell in buffer.content() {
            s.push_str(cell.symbol());
        }
        s
    }
}
