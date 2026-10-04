//! User Interface.
//!
//! This module handles the TUI rendering using `ratatui`.
//!
//! # Architecture
//!
//! The UI is built using the `ratatui` crate, which provides a backend-agnostic way to
//! render terminal interfaces. This allows the game to run:
//! - **Natively** using `crossterm`.
//! - **In Browser** using `ratzilla` (WASM).
//!
//! # Layout
//!
//! The `render` function acts as the main entry point for the UI system. It handles
//! the high-level layout switching:
//!
//! 1. **Main Menu**: If `GameState` is `MainMenu`, it delegates to `render_main_menu`.
//! 2. **Game Interface**: Otherwise, it renders the simulation view, split into:
//!    - **Map**: The main gameplay area (`map::render_map`).
//!    - **Info Panel**: Selected entity details (`panels::render_info_panel`).
//!    - **Status Bar**: Global colony stats (`status::render_status_bar`).
//!    - **Shell Panes**: Chronicle and tech now live behind shell-managed pane plugins.
//! 3. **Hypertile Bridge**: `shell::plugins` adapts those legacy frame-based renderers into
//!    pane plugins while the runtime shell migration is in progress.

/// Chronicle overlay rendering.
pub mod chronicle;
/// Entity inspector panel.
pub mod inspector;
/// Map rendering logic.
pub mod map;
/// Main menu rendering.
pub mod menu;
/// Notifications overlay rendering.
pub mod notifications;
/// Oral Tradition UI rendering.
#[cfg(feature = "nova")]
pub mod oral_tradition;
/// Info panels (inspector, etc).
pub mod panels;
/// Reign & Sky panels: constellation, god-mind, sovereign, possess HUD.
pub mod reign_sky;
/// Seasonal graphics helpers.
pub mod seasonal_gfx;
/// Hypertile shell scaffolding.
pub mod shell;
/// UI State resource.
pub mod state;
/// Status bar rendering.
pub mod status;
/// Tech Tree UI rendering.
pub mod tech;

#[cfg(test)]
mod waste_ui_tests;

use bevy_ecs::prelude::*;
use ratatui::prelude::*;
pub use state::*;

use crate::layer2::render::render_system_view;
use crate::shared::state::GameState;
use crate::shared::view_mode::ViewMode;
use crate::ui::menu_state::MenuState;

use self::map::render_map;
use self::menu::render_main_menu;
use self::notifications::render_notifications;
use self::panels::render_info_panel;
use self::shell::UiShell;
use self::status::render_status_bar;

/// Render the full game UI for one frame.
///
/// This function is called every frame by the platform layer (native main loop or WASM animation frame).
/// It queries the ECS world for necessary state (`GameState`, `MenuState`, etc.) and draws to the
/// provided `ratatui` frame.
///
/// # Arguments
///
/// * `world` - The ECS world containing all game state and resources.
/// * `frame` - The `ratatui` frame to render into.
pub fn render(world: &World, frame: &mut Frame) {
    if *world.resource::<GameState>() == GameState::MainMenu {
        let menu_state = world.resource::<MenuState>();
        render_main_menu(frame, frame.area(), menu_state);
        return;
    }

    // Check ViewMode
    let view_mode = world.resource::<ViewMode>();

    if *view_mode == ViewMode::System {
        render_system_view(frame, frame.area(), world);
        return;
    }

    // Check Global UI suppression (Cinematic/Possession mode)
    let ui_state = world.get_resource::<UiState>();
    let suppress_ui = ui_state.is_some_and(|s| s.suppress_global_ui);

    if suppress_ui {
        // Full screen map
        render_map(frame, frame.area(), world);
        render_notifications(frame, frame.area(), world);
        // Adventurer-mode HUD: possessed pop readout + available actions.
        reign_sky::render_possess_hud(frame, frame.area(), world);
        return;
    }

    // Main vertical split: content + status bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),   // Content area
            Constraint::Length(1), // Status bar
        ])
        .split(frame.area());

    // Horizontal split: map + info panel
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),    // Map area
            Constraint::Length(20), // Info panel
        ])
        .split(main_chunks[0]);

    let map_area = content_chunks[0];
    let info_area = content_chunks[1];
    let status_area = main_chunks[1];

    // Render map
    render_map(frame, map_area, world);

    // Render notifications overlay on top of map
    render_notifications(frame, map_area, world);

    // Render info panel
    render_info_panel(frame, info_area, world);

    // Render status bar
    render_status_bar(frame, status_area, world);
}

/// Render the game UI through the hypertile shell.
pub fn render_with_shell(world: &World, shell: &mut UiShell, frame: &mut Frame) {
    if *world.resource::<GameState>() == GameState::MainMenu {
        let menu_state = world.resource::<MenuState>();
        render_main_menu(frame, frame.area(), menu_state);
        return;
    }

    let ui_state = world.get_resource::<UiState>();
    let suppress_ui = ui_state.is_some_and(|state| state.suppress_global_ui);
    if suppress_ui {
        render_map(frame, frame.area(), world);
        render_notifications(frame, frame.area(), world);
        // Adventurer-mode HUD: possessed pop readout + available actions.
        reign_sky::render_possess_hud(frame, frame.area(), world);
        return;
    }

    match *world.resource::<ViewMode>() {
        ViewMode::System => {
            let _ = shell.switch_to_workspace("System Survey");
        }
        ViewMode::Colony if shell.active_workspace_name() == "System Survey" => {
            let _ = shell.switch_to_workspace("Colony Ops");
        }
        ViewMode::Colony => {}
    }

    shell.render(frame.area(), frame.buffer_mut());
    render_notifications(frame, frame.area(), world);
}

pub mod input;
pub mod menu_state;
pub mod selection;
pub mod world_history;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shell::build_default_shell;
    use crate::ui::shell::config::ShellConfig;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(GameState::MainMenu);
        world.insert_resource(MenuState::default());
        world.insert_resource(ViewMode::default());
        world.insert_resource(UiState::default());

        // Needed for map rendering default case
        world.insert_resource(crate::layer1::resources::ColonyResources::default());
        world.insert_resource(crate::layer1::locations::NamedLocations::default());
        world.insert_resource(crate::ui::selection::Selection::default());
        world.insert_resource(crate::layer1::nature::terrain::Viewport { x: 0, y: 0 });
        world.insert_resource(crate::layer1::architecture::building::BuildMode::default());
        world.insert_resource(
            crate::layer1::administration::designation::DesignationMode::default(),
        );
        world.insert_resource(crate::ui::input::InputContextStack::default());
        world.insert_resource(crate::layer1::nature::weather::WeatherState::default());
        world.insert_resource(crate::ui::map::RenderCache::default());
        world.insert_resource(crate::shared::time::WallTime::default());
        world.insert_resource(crate::layer1::locations::NamedLocations::default());
        world.insert_resource(crate::ui::selection::Selection::default());
        world.insert_resource(crate::layer1::nature::terrain::TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![crate::layer1::nature::terrain::TerrainType::Dirt; 100],
        });
        world.insert_resource(crate::layer1::water::WaterGrid {
            width: 10,
            height: 10,
            values: vec![0; 100],
        });
        world.insert_resource(crate::layer1::Layer2State::default());
        world.insert_resource(crate::layer2::system::SystemMap);
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world.insert_resource(bevy_ecs::event::Events::<
            crate::layer1::chronicle::AddChronicleEvent,
        >::default());
        world.insert_resource(crate::layer1::Chronicle::default());

        world
    }

    #[test]
    fn test_render_main_menu() {
        let world = setup_world(); // Default is MainMenu
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(&world, f)).unwrap();
    }

    #[test]
    fn test_render_system_view() {
        let mut world = setup_world();
        *world.resource_mut::<GameState>() = GameState::Running;
        *world.resource_mut::<ViewMode>() = ViewMode::System;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(&world, f)).unwrap();
    }

    #[test]
    fn test_render_suppress_ui() {
        let mut world = setup_world();
        *world.resource_mut::<GameState>() = GameState::Running;
        world.resource_mut::<UiState>().suppress_global_ui = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(&world, f)).unwrap();
    }

    #[test]
    fn test_render_colony_view() {
        let mut world = setup_world();
        *world.resource_mut::<GameState>() = GameState::Running;
        *world.resource_mut::<ViewMode>() = ViewMode::Colony;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(&world, f)).unwrap();
    }

    #[test]
    fn test_render_with_shell_main_menu() {
        let world = setup_world(); // Default is MainMenu
        let shared_world = Rc::new(RefCell::new(setup_world()));
        let mut shell = build_default_shell(shared_world, ShellConfig::default());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_with_shell(&world, &mut shell, f))
            .unwrap();
    }

    #[test]
    fn test_render_with_shell_suppress_ui() {
        let mut world = setup_world();
        *world.resource_mut::<GameState>() = GameState::Running;
        world.resource_mut::<UiState>().suppress_global_ui = true;

        let shared_world = Rc::new(RefCell::new(setup_world()));
        let mut shell = build_default_shell(shared_world, ShellConfig::default());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_with_shell(&world, &mut shell, f))
            .unwrap();
    }

    #[test]
    fn test_render_with_shell_view_mode_switches() {
        let mut world = setup_world();
        *world.resource_mut::<GameState>() = GameState::Running;
        *world.resource_mut::<ViewMode>() = ViewMode::System;

        let shared_world = Rc::new(RefCell::new(setup_world()));
        let mut shell = build_default_shell(shared_world, ShellConfig::default());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // Should switch to "System Survey"
        terminal
            .draw(|f| render_with_shell(&world, &mut shell, f))
            .unwrap();
        assert_eq!(shell.active_workspace_name(), "System Survey");

        // Now change back to Colony view
        *world.resource_mut::<ViewMode>() = ViewMode::Colony;
        terminal
            .draw(|f| render_with_shell(&world, &mut shell, f))
            .unwrap();
        assert_eq!(shell.active_workspace_name(), "Colony Ops");
    }
}
