use crate::shared::keyboard::{Input, KeyCode};
use bevy_ecs::prelude::*;

use crate::layer1::{
    try_cancel_designation, try_designate_area, try_place_building, BuildMode, CameraTarget,
    ChronicleUiState, DesignationMode, DesignationType, GridPosition, Pop, Viewport,
};
use crate::shared::keyboard::{GameKeyCode, GameKeyEvent, GameMouseEvent};
use crate::shared::state::GameState;
use crate::shared::time::{SimSpeed, SimulationTime};
use crate::shared::view_mode::ViewMode;
use crate::layer1::culture::origins::OriginChoice;
use crate::ui::menu_state::{MenuSelector, MenuState};
use crate::ui::selection::{handle_selection_click, screen_to_world, Selection, SelectionTarget};
use crate::ui::shell::plugins::{SharedWorld, COLONY_MAP_PLUGIN_TYPE, SYSTEM_MAP_PLUGIN_TYPE};
use crate::ui::shell::UiShell;

/// Defines the current input handling context.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum InputContext {
    /// Main Menu mode.
    #[default]
    MainMenu,
    /// Normal game mode.
    Normal,
    /// Building placement mode.
    BuildMode,
    /// Designation mode.
    DesignationMode,
    /// Modal overlay.
    Overlay,
    /// Tech Tree overlay.
    TechTree,
    /// Direct control mode (Possession).
    DirectControl,
}

/// Stack-based input context manager.
#[derive(Resource, Debug)]
pub struct InputContextStack {
    stack: Vec<InputContext>,
}

impl Default for InputContextStack {
    fn default() -> Self {
        Self {
            stack: vec![InputContext::MainMenu],
        }
    }
}

impl InputContextStack {
    /// Get the current active context.
    ///
    /// # Panics
    ///
    /// Panics if the input stack is empty (which should never happen due to default init).
    #[must_use]
    pub fn current(&self) -> InputContext {
        *self
            .stack
            .last()
            .expect("Input stack should never be empty")
    }

    /// Push a new context.
    pub fn push(&mut self, context: InputContext) {
        self.stack.push(context);
    }

    /// Pop the current context, if not the base.
    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

/// Route input to the appropriate handler based on current context.
pub fn route_input(world: &mut World, key: GameKeyEvent) {
    let context = world.resource::<InputContextStack>().current();

    match context {
        InputContext::MainMenu => handle_main_menu_mode(world, key),
        InputContext::Normal => handle_normal_mode(world, key),
        InputContext::BuildMode => handle_build_mode(world, key),
        InputContext::DesignationMode => handle_designation_mode(world, key),
        InputContext::Overlay => handle_overlay_mode(world, key),
        InputContext::TechTree => handle_tech_tree_mode(world, key),
        InputContext::DirectControl => handle_direct_control_mode(world, key),
    }
}

/// Route top-level input through the shell first, then gameplay handlers.
pub fn route_root_input(world: &SharedWorld, shell: &mut UiShell, key: GameKeyEvent) {
    let is_main_menu = {
        let world = world.borrow();
        *world.resource::<GameState>() == GameState::MainMenu
    };

    if is_main_menu {
        route_input(&mut world.borrow_mut(), key);
        return;
    }

    if key.modifiers.ctrl && matches!(key.code, GameKeyCode::Char('k' | 'K')) {
        let _ = shell.open_palette();
        return;
    }

    if shell.is_palette_open() {
        let _ = shell.handle_key(key);
        return;
    }

    if shell.is_layout_mode() {
        let _ = shell.handle_key(key);
        return;
    }

    if matches!(key.code, GameKeyCode::Char('l' | 'h'))
        && shell.execute_command_by_id("pane.chronicle")
    {
        return;
    }

    if matches!(key.code, GameKeyCode::Char('t')) && shell.execute_command_by_id("pane.tech") {
        return;
    }

    if shell.handle_key(key) {
        return;
    }

    route_input(&mut world.borrow_mut(), key);
}

/// Route mouse input to the appropriate handler based on current context.
pub fn route_mouse_input(world: &mut World, mouse: GameMouseEvent) {
    let context = world.resource::<InputContextStack>().current();

    match context {
        InputContext::Normal => {
            let viewport = *world.resource::<Viewport>();
            handle_selection_click(world, mouse, &viewport);
        }
        InputContext::DesignationMode => {
            handle_designation_mouse(world, mouse);
        }
        _ => {}
    }
}

/// Route top-level mouse input, suppressing gameplay clicks while layout mode is active.
pub fn route_root_mouse_input(world: &SharedWorld, shell: &UiShell, mouse: GameMouseEvent) {
    let is_main_menu = {
        let world = world.borrow();
        *world.resource::<GameState>() == GameState::MainMenu
    };

    if is_main_menu || shell.is_layout_mode() || shell.is_palette_open() {
        return;
    }

    if !matches!(
        shell.focused_plugin_type(),
        Some(COLONY_MAP_PLUGIN_TYPE | SYSTEM_MAP_PLUGIN_TYPE)
    ) {
        return;
    }

    route_mouse_input(&mut world.borrow_mut(), mouse);
}

fn handle_direct_control_mode(world: &mut World, key: GameKeyEvent) {
    // Interact is a headless-console command (`interact`) in this build; the
    // TUI documents the gap instead of silently eating the key.
    if matches!(key.code, GameKeyCode::Char(' ')) {
        if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
            log.add("Interact is a console command in this build (headless `interact`).");
        }
        return;
    }
    // Map to KeyCode and update Input resource
    let bevy_key = map_game_key_to_bevy_key(key.code);
    if let Some(mut input) = world.get_resource_mut::<Input>() {
        input.press(bevy_key);
    }
}

pub fn map_game_key_to_bevy_key(key: GameKeyCode) -> KeyCode {
    match key {
        GameKeyCode::Char(c) => match c {
            'w' | 'W' => KeyCode::W,
            'a' | 'A' => KeyCode::A,
            's' | 'S' => KeyCode::S,
            'd' | 'D' => KeyCode::D,
            'q' | 'Q' => KeyCode::Q,
            'e' | 'E' => KeyCode::E,
            'r' | 'R' => KeyCode::R,
            'f' | 'F' => KeyCode::F,
            ' ' => KeyCode::Space,
            _ => KeyCode::Unidentified,
        },
        GameKeyCode::Up => KeyCode::Up,
        GameKeyCode::Down => KeyCode::Down,
        GameKeyCode::Left => KeyCode::Left,
        GameKeyCode::Right => KeyCode::Right,
        GameKeyCode::Enter => KeyCode::Return,
        GameKeyCode::Esc => KeyCode::Esc,
        GameKeyCode::Tab => KeyCode::Tab,
        GameKeyCode::Backspace => KeyCode::Back,
        GameKeyCode::Delete => KeyCode::Delete,
        GameKeyCode::BackTab => KeyCode::Tab,
    }
}

fn handle_normal_mode(world: &mut World, key: GameKeyEvent) {
    if handle_normal_mode_quit_or_speed(world, key) {
        return;
    }
    if handle_normal_mode_camera(world, key) {
        return;
    }
    handle_normal_mode_modes(world, key);
}

fn handle_normal_mode_quit_or_speed(world: &mut World, key: GameKeyEvent) -> bool {
    match key.code {
        GameKeyCode::Char('q') => {
            *world.resource_mut::<GameState>() = GameState::Quitting;
            true
        }
        GameKeyCode::Esc => {
            let mut selection = world.resource_mut::<Selection>();
            if selection.is_selected() {
                selection.clear();
            } else {
                *world.resource_mut::<GameState>() = GameState::Quitting;
            }
            true
        }
        GameKeyCode::Char(' ') => {
            let mut state = world.resource_mut::<GameState>();
            *state = match *state {
                GameState::Running => GameState::Paused,
                GameState::Paused => GameState::Running,
                GameState::Quitting => GameState::Quitting,
                GameState::MainMenu => GameState::MainMenu,
            };
            true
        }
        GameKeyCode::Char('1') => {
            world.resource_mut::<SimulationTime>().speed = SimSpeed::Normal;
            true
        }
        GameKeyCode::Char('2') => {
            world.resource_mut::<SimulationTime>().speed = SimSpeed::Fast;
            true
        }
        GameKeyCode::Char('3') => {
            world.resource_mut::<SimulationTime>().speed = SimSpeed::Faster;
            true
        }
        _ => false,
    }
}

fn handle_normal_mode_camera(world: &mut World, key: GameKeyEvent) -> bool {
    match key.code {
        GameKeyCode::Char('w') | GameKeyCode::Up => {
            let mut target = world.resource_mut::<CameraTarget>();
            target.y -= 1.0;
            true
        }
        GameKeyCode::Char('s') | GameKeyCode::Down => {
            let mut target = world.resource_mut::<CameraTarget>();
            target.y += 1.0;
            true
        }
        GameKeyCode::Char('a') | GameKeyCode::Left => {
            let mut target = world.resource_mut::<CameraTarget>();
            target.x -= 1.0;
            true
        }
        GameKeyCode::Char('d') | GameKeyCode::Right => {
            let mut target = world.resource_mut::<CameraTarget>();
            target.x += 1.0;
            true
        }
        _ => false,
    }
}

fn handle_normal_mode_modes(world: &mut World, key: GameKeyEvent) -> bool {
    match key.code {
        GameKeyCode::Char('b') => {
            // Enter build mode
            world
                .resource_mut::<InputContextStack>()
                .push(InputContext::BuildMode);

            // Sync side effects
            let (vx, vy) = {
                let viewport = world.resource::<Viewport>();
                (viewport.x, viewport.y)
            };
            let mut build_mode = world.resource_mut::<BuildMode>();
            build_mode.active = true;
            build_mode.cursor = GridPosition {
                x: vx + 10,
                y: vy + 10,
            };
            true
        }
        GameKeyCode::Char('m') => {
            // Enter Designation mode (Mine)
            enter_designation_mode(world, DesignationType::Mine);
            true
        }
        GameKeyCode::Char('x') => {
            // Enter Designation mode (Demolish)
            enter_designation_mode(world, DesignationType::Demolish);
            true
        }
        GameKeyCode::Char('c') => {
            // Enter Designation mode (Chop)
            enter_designation_mode(world, DesignationType::Chop);
            true
        }
        GameKeyCode::Tab => {
            let visibility = *world.resource::<crate::layer2::visibility::SystemVisibility>();
            let mut view_mode = world.resource_mut::<ViewMode>();
            match *view_mode {
                ViewMode::Colony => {
                    if visibility == crate::layer2::visibility::SystemVisibility::Full {
                        *view_mode = ViewMode::System;
                    } else if let Some(mut log) =
                        world.get_resource_mut::<crate::shared::log::MessageLog>()
                    {
                        log.add("Cannot switch view: Command Center Required (Powered)");
                    }
                }
                ViewMode::System => *view_mode = ViewMode::Colony,
            }
            true
        }
        GameKeyCode::Char('p') => {
            // Adventurer mode: possess the selected pop.
            // The `PossessEntityEvent` is consumed by `handle_possession` (sim)
            // and `handle_possession_ui_state` (UI suppression + input context).
            if let SelectionTarget::Entity(entity) = world.resource::<Selection>().target() {
                if world.get::<Pop>(entity).is_some() {
                    world
                        .resource_mut::<Events<PossessEntityEvent>>()
                        .send(PossessEntityEvent(entity));
                }
            }
            true
        }
        _ => false,
    }
}

fn handle_tech_tree_mode(world: &mut World, key: GameKeyEvent) {
    let tech_count = crate::ui::tech::get_tech_list().len();

    match key.code {
        GameKeyCode::Esc | GameKeyCode::Char('t') => {
            world.resource_mut::<InputContextStack>().pop();
        }
        GameKeyCode::Up | GameKeyCode::Char('w') => {
            world
                .resource_mut::<crate::ui::tech::TechUiState>()
                .prev(tech_count);
        }
        GameKeyCode::Down | GameKeyCode::Char('s') => {
            world
                .resource_mut::<crate::ui::tech::TechUiState>()
                .next(tech_count);
        }
        GameKeyCode::Enter | GameKeyCode::Char(' ') => {
            let ui_state = world.resource::<crate::ui::tech::TechUiState>();
            let idx = ui_state.selected_index;
            let techs = crate::ui::tech::get_tech_list();
            if idx < techs.len() {
                crate::layer1::tech::unlock_tech(world, techs[idx]);
            }
        }
        _ => {}
    }
}

fn handle_main_menu_mode(world: &mut World, key: GameKeyEvent) {
    match key.code {
        GameKeyCode::Left | GameKeyCode::Char('a') => {
            // ←/→ edits whichever setup row (scenario / origin) has focus.
            match world.resource::<MenuState>().selector {
                MenuSelector::Scenario => world.resource_mut::<MenuState>().prev_scenario(),
                MenuSelector::Origin => world.resource_mut::<MenuState>().prev_origin(),
            }
        }
        GameKeyCode::Right | GameKeyCode::Char('d') => {
            match world.resource::<MenuState>().selector {
                MenuSelector::Scenario => world.resource_mut::<MenuState>().next_scenario(),
                MenuSelector::Origin => world.resource_mut::<MenuState>().next_origin(),
            }
        }
        GameKeyCode::Tab => {
            world.resource_mut::<MenuState>().cycle_selector();
        }
        GameKeyCode::Up | GameKeyCode::Char('w') => {
            world.resource_mut::<MenuState>().prev();
        }
        GameKeyCode::Down | GameKeyCode::Char('s') => {
            world.resource_mut::<MenuState>().next();
        }
        GameKeyCode::Enter | GameKeyCode::Char(' ') => {
            let selected = world.resource::<MenuState>().selected_index;
            match selected {
                0 => {
                    // Start Game
                    let selected_scenario = world.resource::<MenuState>().selected_scenario;
                    let selected_origin = world.resource::<MenuState>().selected_origin;
                    let scenario =
                        crate::shared::scenario::start_scenario_definition(selected_scenario);
                    *world.resource_mut::<crate::shared::scenario::ActiveStartScenario>() =
                        crate::shared::scenario::ActiveStartScenario {
                            id: scenario.id,
                            name: scenario.name,
                            difficulty: scenario.difficulty,
                        };
                    // The adventurer origin pick rides into the world here;
                    // the origin spawn director reads it on the first tick.
                    // `None` = "Surprise me": a random 2–3-origin roster.
                    world.insert_resource(OriginChoice {
                        chosen: selected_origin,
                    });
                    crate::setup::apply_selected_start_scenario(world);
                    *world.resource_mut::<GameState>() = GameState::Running;
                    let mut stack = world.resource_mut::<InputContextStack>();
                    stack.stack = vec![InputContext::Normal];
                }
                1 => {
                    // Quit
                    *world.resource_mut::<GameState>() = GameState::Quitting;
                }
                _ => {}
            }
        }
        GameKeyCode::Esc | GameKeyCode::Char('q') => {
            *world.resource_mut::<GameState>() = GameState::Quitting;
        }
        _ => {}
    }
}

fn enter_designation_mode(world: &mut World, tool: DesignationType) {
    world
        .resource_mut::<InputContextStack>()
        .push(InputContext::DesignationMode);

    let (vx, vy) = {
        let viewport = world.resource::<Viewport>();
        (viewport.x, viewport.y)
    };
    let mut mode = world.resource_mut::<DesignationMode>();
    mode.active = true;
    mode.tool = tool;
    mode.cursor = GridPosition {
        x: vx + 10,
        y: vy + 10,
    };
    mode.drag_start = None;
}

fn handle_build_mode(world: &mut World, key: GameKeyEvent) {
    match key.code {
        GameKeyCode::Esc | GameKeyCode::Char('b') => {
            // Exit build mode
            world.resource_mut::<InputContextStack>().pop();
            world.resource_mut::<BuildMode>().active = false;
        }
        GameKeyCode::Char('w') | GameKeyCode::Up => {
            let mut bm = world.resource_mut::<BuildMode>();
            bm.cursor.y = bm.cursor.y.saturating_sub(1);
        }
        GameKeyCode::Char('s') | GameKeyCode::Down => {
            let mut bm = world.resource_mut::<BuildMode>();
            bm.cursor.y = bm.cursor.y.saturating_add(1);
        }
        GameKeyCode::Char('a') | GameKeyCode::Left => {
            let mut bm = world.resource_mut::<BuildMode>();
            bm.cursor.x = bm.cursor.x.saturating_sub(1);
        }
        GameKeyCode::Char('d') | GameKeyCode::Right => {
            let mut bm = world.resource_mut::<BuildMode>();
            bm.cursor.x = bm.cursor.x.saturating_add(1);
        }
        GameKeyCode::Char(' ') | GameKeyCode::Enter => {
            let build_mode = world.resource::<BuildMode>();
            let cursor = build_mode.cursor;
            let building_type = build_mode.selected;
            try_place_building(world, cursor.x, cursor.y, building_type);
        }
        GameKeyCode::Tab => {
            let mut build_mode = world.resource_mut::<BuildMode>();
            build_mode.selected = build_mode.selected.next();
        }
        GameKeyCode::Char('m') | GameKeyCode::BackTab => {
            let mut build_mode = world.resource_mut::<BuildMode>();
            build_mode.selected_material = build_mode.selected_material.next();
        }
        _ => {}
    }
}

fn handle_designation_mode(world: &mut World, key: GameKeyEvent) {
    match key.code {
        GameKeyCode::Esc => {
            // Exit designation mode
            world.resource_mut::<InputContextStack>().pop();
            let mut mode = world.resource_mut::<DesignationMode>();
            mode.active = false;
            mode.drag_start = None;
        }
        GameKeyCode::Char('m') => {
            // Switch to Mine tool
            world.resource_mut::<DesignationMode>().tool = DesignationType::Mine;
        }
        GameKeyCode::Char('x') => {
            // Switch to Demolish tool
            world.resource_mut::<DesignationMode>().tool = DesignationType::Demolish;
        }
        GameKeyCode::Char('c') => {
            // Switch to Chop tool
            world.resource_mut::<DesignationMode>().tool = DesignationType::Chop;
        }
        GameKeyCode::Char('w') | GameKeyCode::Up => {
            let mut mode = world.resource_mut::<DesignationMode>();
            mode.cursor.y = mode.cursor.y.saturating_sub(1);
        }
        GameKeyCode::Char('s') | GameKeyCode::Down => {
            let mut mode = world.resource_mut::<DesignationMode>();
            mode.cursor.y = mode.cursor.y.saturating_add(1);
        }
        GameKeyCode::Char('a') | GameKeyCode::Left => {
            let mut mode = world.resource_mut::<DesignationMode>();
            mode.cursor.x = mode.cursor.x.saturating_sub(1);
        }
        GameKeyCode::Char('d') | GameKeyCode::Right => {
            let mut mode = world.resource_mut::<DesignationMode>();
            mode.cursor.x = mode.cursor.x.saturating_add(1);
        }
        GameKeyCode::Char(' ') | GameKeyCode::Enter => {
            let mode = world.resource::<DesignationMode>();
            let cursor = mode.cursor;
            let tool = mode.tool;
            let drag_start = mode.drag_start;

            if let Some(start) = drag_start {
                // Second press: designate the rectangle and clear drag_start
                try_designate_area(world, start.x, start.y, cursor.x, cursor.y, tool);
                world.resource_mut::<DesignationMode>().drag_start = None;
            } else {
                // First press: set drag_start
                world.resource_mut::<DesignationMode>().drag_start = Some(cursor);
            }
        }
        GameKeyCode::Backspace | GameKeyCode::Delete => {
            let mode = world.resource::<DesignationMode>();
            try_cancel_designation(world, mode.cursor.x, mode.cursor.y);
        }
        _ => {}
    }
}

fn handle_designation_mouse(world: &mut World, mouse: GameMouseEvent) {
    let viewport = *world.resource::<Viewport>();
    let (world_x, world_y) = screen_to_world(mouse.x, mouse.y, &viewport);

    let mode = world.resource::<DesignationMode>();
    let tool = mode.tool;
    let drag_start = mode.drag_start;

    if let Some(start) = drag_start {
        // Second click: designate the rectangle and clear drag_start
        try_designate_area(world, start.x, start.y, world_x, world_y, tool);
        let mut mode = world.resource_mut::<DesignationMode>();
        mode.drag_start = None;
        mode.cursor = GridPosition {
            x: world_x,
            y: world_y,
        };
    } else {
        // First click: set drag_start and move cursor
        let mut mode = world.resource_mut::<DesignationMode>();
        let pos = GridPosition {
            x: world_x,
            y: world_y,
        };
        mode.drag_start = Some(pos);
        mode.cursor = pos;
    }
}

fn handle_overlay_mode(world: &mut World, key: GameKeyEvent) {
    match key.code {
        GameKeyCode::Esc | GameKeyCode::Char('l' | 'h') => {
            world.resource_mut::<InputContextStack>().pop();
            if let Some(mut ui_state) = world.get_resource_mut::<ChronicleUiState>() {
                ui_state.is_open = false;
            }
        }
        _ => {}
    }
}

use crate::layer1::direct_link::{PossessEntityEvent, UnpossessEvent};
use crate::ui::state::UiState;

pub fn handle_possession_ui_state(
    mut events: EventReader<PossessEntityEvent>,
    mut unpossess: EventReader<UnpossessEvent>,
    mut input_stack: ResMut<InputContextStack>,
    mut ui_state: ResMut<UiState>,
) {
    if !unpossess.is_empty() {
        unpossess.clear();
        ui_state.suppress_global_ui = false;
        if input_stack.current() == InputContext::DirectControl {
            input_stack.pop();
        }
    }
    for _ in events.read() {
        ui_state.suppress_global_ui = true;
        input_stack.push(InputContext::DirectControl);
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::{Chronicle, ColonyResources, Pop};
    use crate::setup::{setup_world_with_config, SetupConfig};
    use crate::shared::scenario::{
        start_scenario_definition, ActiveStartScenario, StartScenarioId,
    };

    use super::*;
    use crate::layer1::Viewport;

    use crate::shared::keyboard::{GameKeyCode, GameKeyEvent, GameMouseEvent};
    use crate::shared::state::GameState;
    use crate::shared::time::{SimSpeed, SimulationTime};
    use crate::ui::selection::{Selection, SelectionTarget};
    use crate::ui::shell::{build_default_shell, ShellConfig};
    use ratatui::{buffer::Buffer, layout::Rect};
    use std::{cell::RefCell, rc::Rc};

    fn key_event(code: GameKeyCode) -> GameKeyEvent {
        GameKeyEvent::new(code)
    }

    fn mouse_event(column: u16, row: u16) -> GameMouseEvent {
        GameMouseEvent::new(column, row)
    }

    #[test]
    fn ctrl_k_opens_palette_without_toggling_gameplay_state() {
        let (world, mut shell) = setup_shell_world_for_test();

        route_root_input(
            &world,
            &mut shell,
            GameKeyEvent::new(GameKeyCode::Char('k')).with_ctrl(),
        );

        assert!(shell_buffer_contains(&mut shell, "Command Palette"));
        assert!(shell_buffer_contains(&mut shell, "Open Chronicle"));
        assert_eq!(*world.borrow().resource::<GameState>(), GameState::Running);
    }

    #[test]
    fn possess_key_sends_possess_event_for_selected_pop() {
        let (world, _shell) = setup_shell_world_for_test();
        let pop = {
            let world_ref = world.borrow();
            let found = world_ref
                .iter_entities()
                .filter(|e| e.get::<Pop>().is_some())
                .map(|e| e.id())
                .next();
            found.expect("headless setup spawns pops")
        };
        world
            .borrow_mut()
            .resource_mut::<Selection>()
            .select_entity(pop);

        route_input(&mut world.borrow_mut(), key_event(GameKeyCode::Char('p')));

        let world_ref = world.borrow();
        let events = world_ref.resource::<Events<PossessEntityEvent>>();
        assert_eq!(
            events.len(),
            1,
            "'p' with a selected pop should send exactly one PossessEntityEvent"
        );
        let mut cursor = events.get_cursor();
        let sent: Vec<Entity> = cursor.read(&events).map(|e| e.0).collect();
        assert_eq!(sent, vec![pop]);
    }

    #[test]
    fn possession_ui_state_syncs_with_possess_events() {
        let mut world = World::new();
        world.init_resource::<Events<PossessEntityEvent>>();
        world.init_resource::<Events<UnpossessEvent>>();
        world.init_resource::<InputContextStack>();
        world.init_resource::<UiState>();
        world
            .resource_mut::<InputContextStack>()
            .push(InputContext::Normal);

        let pop = world.spawn_empty().id();
        world
            .resource_mut::<Events<PossessEntityEvent>>()
            .send(PossessEntityEvent(pop));

        let system_id = world.register_system(handle_possession_ui_state);
        world
            .run_system(system_id)
            .expect("possession UI sync system should run");

        assert!(
            world.resource::<UiState>().suppress_global_ui,
            "possess event should suppress the global UI"
        );
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::DirectControl,
            "possess event should push the DirectControl context"
        );

        world
            .resource_mut::<Events<UnpossessEvent>>()
            .send(UnpossessEvent);
        world
            .run_system(system_id)
            .expect("possession UI sync system should run");

        assert!(
            !world.resource::<UiState>().suppress_global_ui,
            "unpossess event should restore the global UI"
        );
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::Normal,
            "unpossess event should pop back to Normal"
        );
    }

    #[test]
    fn command_palette_can_execute_open_tech_tree() {
        let (world, mut shell) = setup_shell_world_for_test();

        route_root_input(
            &world,
            &mut shell,
            GameKeyEvent::new(GameKeyCode::Char('k')).with_ctrl(),
        );
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('t')));
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('e')));
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('c')));
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('h')));
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Enter));

        assert!(active_workspace_contains_plugin(&mut shell, "tech"));
        assert_eq!(
            world.borrow().resource::<InputContextStack>().current(),
            InputContext::Normal
        );
    }

    #[test]
    fn chronicle_hotkey_opens_shell_pane_without_overlay_context() {
        let (world, mut shell) = setup_shell_world_for_test();
        let switched = shell.switch_to_workspace("System Survey");

        assert!(switched);
        assert!(!active_workspace_contains_plugin(&mut shell, "chronicle"));

        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('l')));

        assert!(active_workspace_contains_plugin(&mut shell, "chronicle"));
        assert_eq!(
            world.borrow().resource::<InputContextStack>().current(),
            InputContext::Normal
        );
    }

    #[test]
    fn tech_hotkey_opens_shell_pane_without_overlay_context() {
        let (world, mut shell) = setup_shell_world_for_test();

        assert!(!active_workspace_contains_plugin(&mut shell, "tech"));

        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('t')));

        assert!(active_workspace_contains_plugin(&mut shell, "tech"));
        assert_eq!(
            world.borrow().resource::<InputContextStack>().current(),
            InputContext::Normal
        );
    }

    #[test]
    fn test_input_context_default() {
        let context = InputContext::default();
        assert_eq!(context, InputContext::MainMenu);
    }

    #[test]
    fn test_input_context_stack_push_pop() {
        let mut stack = InputContextStack::default();
        assert_eq!(stack.current(), InputContext::MainMenu);

        stack.push(InputContext::BuildMode);
        assert_eq!(stack.current(), InputContext::BuildMode);

        stack.push(InputContext::Overlay);
        assert_eq!(stack.current(), InputContext::Overlay);

        stack.pop();
        assert_eq!(stack.current(), InputContext::BuildMode);

        stack.pop();
        assert_eq!(stack.current(), InputContext::MainMenu);
    }

    #[test]
    fn test_cannot_pop_main_menu_context() {
        let mut stack = InputContextStack::default();
        assert_eq!(stack.current(), InputContext::MainMenu);

        // Popping base should do nothing (always one context)
        stack.pop();
        assert_eq!(stack.current(), InputContext::MainMenu);
    }

    #[test]
    fn test_input_router_normal_mode_quit() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);

        route_input(&mut world, key_event(GameKeyCode::Char('q')));

        assert_eq!(*world.resource::<GameState>(), GameState::Quitting);
    }

    #[test]
    fn test_input_router_normal_mode_esc_clears_selection() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        let mut selection = Selection::default();
        selection.select_tile(10, 10);
        world.insert_resource(selection);

        route_input(&mut world, key_event(GameKeyCode::Esc));

        assert!(!world.resource::<Selection>().is_selected());
        assert_eq!(*world.resource::<GameState>(), GameState::Running);
    }

    #[test]
    fn test_input_router_normal_mode_esc_quits_if_no_selection() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        world.insert_resource(Selection::default()); // No selection

        route_input(&mut world, key_event(GameKeyCode::Esc));

        assert_eq!(*world.resource::<GameState>(), GameState::Quitting);
    }

    #[test]
    fn test_input_router_build_mode_blocks_quit() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::BuildMode);
        world.insert_resource(stack);
        world.insert_resource(BuildMode {
            active: true,
            ..Default::default()
        });

        route_input(&mut world, key_event(GameKeyCode::Char('q')));

        // 'q' in build mode should NOT quit (should exit build mode instead)
        assert_eq!(*world.resource::<GameState>(), GameState::Running);
    }

    #[test]
    fn test_input_router_overlay_escape_pops_context() {
        let mut world = World::new();
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Overlay); // [MainMenu, Overlay]
        world.insert_resource(stack);
        world.insert_resource(ChronicleUiState { is_open: true });

        route_input(&mut world, key_event(GameKeyCode::Esc));

        // Escape in overlay should pop back to base
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::MainMenu
        );
        assert!(!world.resource::<ChronicleUiState>().is_open);
    }

    #[test]
    fn test_route_input_normal_mode_leaves_chronicle_hotkey_to_shell() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);

        route_input(&mut world, key_event(GameKeyCode::Char('l')));

        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::Normal
        );
        assert_eq!(*world.resource::<GameState>(), GameState::Running);
    }

    #[test]
    fn test_input_context_is_copy() {
        let ctx1 = InputContext::Normal;
        let ctx2 = ctx1; // Should copy
        assert_eq!(ctx1, ctx2);
    }

    #[test]
    #[should_panic(expected = "Input stack should never be empty")]
    fn test_current_panics_when_stack_empty() {
        let mut stack = InputContextStack::default();
        stack.stack.clear(); // Force the invalid state
        let _ = stack.current();
    }

    #[test]
    fn test_keybinding_normal_mode_all_keys() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Viewport::default());
        world.insert_resource(CameraTarget::default());
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        world.insert_resource(BuildMode::default());
        world.insert_resource(DesignationMode::default());
        world.insert_resource(MenuState::default()); // Added for GameState toggle check if needed

        // Test all normal mode bindings work
        route_input(&mut world, key_event(GameKeyCode::Char(' ')));
        assert_eq!(*world.resource::<GameState>(), GameState::Paused);

        route_input(&mut world, key_event(GameKeyCode::Char('1')));
        assert_eq!(world.resource::<SimulationTime>().speed, SimSpeed::Normal);

        route_input(&mut world, key_event(GameKeyCode::Char('w')));
        assert!((world.resource::<CameraTarget>().y - -1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_camera_target_movement() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        world.insert_resource(CameraTarget { x: 100.0, y: 100.0 });

        // Move right (x += 1)
        route_input(&mut world, key_event(GameKeyCode::Char('d')));
        assert!((world.resource::<CameraTarget>().x - 101.0).abs() < f32::EPSILON);

        // Move up (y -= 1)
        route_input(&mut world, key_event(GameKeyCode::Char('w')));
        assert!((world.resource::<CameraTarget>().y - 99.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_cursor_overflow_safety() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::BuildMode);
        world.insert_resource(stack);

        let build_mode = BuildMode {
            active: true,
            cursor: GridPosition {
                x: i32::MAX,
                y: i32::MIN,
            },
            ..Default::default()
        };
        world.insert_resource(build_mode);

        // Move right (x += 1) should saturate
        route_input(&mut world, key_event(GameKeyCode::Char('d')));
        assert_eq!(world.resource::<BuildMode>().cursor.x, i32::MAX);

        // Move up (y -= 1) should saturate
        route_input(&mut world, key_event(GameKeyCode::Char('w')));
        assert_eq!(world.resource::<BuildMode>().cursor.y, i32::MIN);
    }

    #[test]
    fn test_designation_mode_entry_and_exit() {
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        world.insert_resource(Viewport::default());
        world.insert_resource(DesignationMode::default());

        // Enter mine mode
        route_input(&mut world, key_event(GameKeyCode::Char('m')));
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::DesignationMode
        );
        assert!(world.resource::<DesignationMode>().active);
        assert_eq!(
            world.resource::<DesignationMode>().tool,
            DesignationType::Mine
        );

        // Exit
        route_input(&mut world, key_event(GameKeyCode::Esc));
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::Normal
        );
        assert!(!world.resource::<DesignationMode>().active);

        // Enter demolish mode
        route_input(&mut world, key_event(GameKeyCode::Char('x')));
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::DesignationMode
        );
        assert_eq!(
            world.resource::<DesignationMode>().tool,
            DesignationType::Demolish
        );
    }

    #[test]
    fn test_route_mouse_normal_mode_selects() {
        let mut world = World::new();
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Normal);
        world.insert_resource(stack);
        world.insert_resource(Viewport::default());
        world.insert_resource(Selection::default());
        // Need GridPosition/Entity to select? Or just select tile.
        // Selecting tile is enough to verify "something happened".

        route_mouse_input(&mut world, mouse_event(10, 10));

        let selection = world.resource::<Selection>();
        assert_eq!(selection.target(), SelectionTarget::Tile(10, 10));
    }

    #[test]
    fn test_route_mouse_build_mode_ignores_click() {
        let mut world = World::new();
        let mut stack = InputContextStack::default();
        stack.push(InputContext::BuildMode);
        world.insert_resource(stack);
        world.insert_resource(Viewport::default());
        world.insert_resource(Selection::default());

        route_mouse_input(&mut world, mouse_event(10, 10));

        let selection = world.resource::<Selection>();
        assert!(!selection.is_selected());
    }

    #[test]
    fn test_route_mouse_overlay_ignores_click() {
        let mut world = World::new();
        let mut stack = InputContextStack::default();
        stack.push(InputContext::Overlay);
        world.insert_resource(stack);
        world.insert_resource(Viewport::default());
        world.insert_resource(Selection::default());

        route_mouse_input(&mut world, mouse_event(10, 10));

        let selection = world.resource::<Selection>();
        assert!(!selection.is_selected());
    }

    #[test]
    fn test_build_mode_backtab_cycles_material() {
        use crate::layer1::{BuildingType, MaterialType};
        let mut world = World::new();
        world.insert_resource(GameState::Running);
        let mut stack = InputContextStack::default();
        stack.push(InputContext::BuildMode);
        world.insert_resource(stack);
        world.insert_resource(BuildMode {
            active: true,
            selected: BuildingType::Wall, // Walls support materials
            selected_material: MaterialType::Wood,
            ..Default::default()
        });

        // First BackTab -> Stone
        route_input(&mut world, key_event(GameKeyCode::BackTab));
        assert_eq!(
            world.resource::<BuildMode>().selected_material,
            MaterialType::Stone
        );
    }

    #[test]
    fn focused_tech_pane_handles_navigation_in_plugin_input_mode() {
        let (world, mut shell) = setup_shell_world_for_test();

        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('t')));
        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('s')));

        assert_eq!(
            world
                .borrow()
                .resource::<crate::ui::tech::TechUiState>()
                .selected_index,
            1
        );
        assert_eq!(
            world.borrow().resource::<InputContextStack>().current(),
            InputContext::Normal
        );

        route_root_input(&world, &mut shell, key_event(GameKeyCode::Char('w')));
        assert_eq!(
            world
                .borrow()
                .resource::<crate::ui::tech::TechUiState>()
                .selected_index,
            0
        );
    }

    fn setup_shell_world_for_test() -> (SharedWorld, UiShell) {
        let world = Rc::new(RefCell::new(setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        })));
        {
            let mut world_ref = world.borrow_mut();
            world_ref.insert_resource(GameState::Running);
            let mut stack = InputContextStack::default();
            stack.push(InputContext::Normal);
            world_ref.insert_resource(stack);
            world_ref.insert_resource(ViewMode::Colony);
            world_ref.insert_resource(crate::layer2::visibility::SystemVisibility::Full);
        }

        let config = world.borrow().resource::<ShellConfig>().clone();
        let shell = build_default_shell(Rc::clone(&world), config);
        (world, shell)
    }

    fn shell_buffer_contains(shell: &mut UiShell, needle: &str) -> bool {
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        shell.render(area, &mut buffer);
        // ⚡ Bolt Optimization: Replace intermediate .collect::<String>() chain with pre-allocated String loop
        let mut s = String::with_capacity(buffer.area.area() as usize);
        for cell in buffer.content() {
            s.push_str(cell.symbol());
        }
        s.contains(needle)
    }

    fn active_workspace_contains_plugin(shell: &mut UiShell, plugin_type: &str) -> bool {
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        shell.render(area, &mut buffer);
        let runtime = shell.workspaces().active_runtime();
        runtime
            .panes()
            .into_iter()
            .any(|pane| runtime.registry().plugin_type_for(pane.id) == Some(plugin_type))
    }
    #[test]
    fn test_gamestate_main_menu_variant() {
        let state = GameState::MainMenu;
        assert_ne!(state, GameState::Running);
        assert_ne!(state, GameState::Paused);
    }

    #[test]
    fn test_input_context_main_menu_variant() {
        let ctx = InputContext::MainMenu;
        assert_ne!(ctx, InputContext::Normal);
    }

    #[test]
    fn test_menu_input_start_game() {
        let mut world = World::new();
        world.insert_resource(GameState::MainMenu);
        let active = start_scenario_definition(StartScenarioId::Classic);
        world.insert_resource(ActiveStartScenario {
            id: active.id,
            name: active.name,
            difficulty: active.difficulty,
        });
        world.insert_resource(ColonyResources::default());
        world.insert_resource(bevy_ecs::event::Events::<
            crate::layer1::chronicle::AddChronicleEvent,
        >::default());
        world.insert_resource(crate::layer1::Chronicle::default());
        world.insert_resource(crate::shared::time::SimulationTime::default());
        world.insert_resource(crate::shared::scenario::AppliedStartScenario {
            id: StartScenarioId::Classic,
        });

        let mut stack = InputContextStack::default();
        stack.push(InputContext::MainMenu);
        world.insert_resource(stack);

        world.insert_resource(MenuState {
            selected_index: 0,
            selected_scenario: StartScenarioId::SocialDrama,
            ..Default::default()
        }); // "Start Game" selected

        route_input(&mut world, key_event(GameKeyCode::Enter));

        // Should transition to Running
        assert_eq!(*world.resource::<GameState>(), GameState::Running);
        assert_eq!(
            world.resource::<ActiveStartScenario>().id,
            StartScenarioId::SocialDrama
        );
        // Should switch input context to Normal
        assert_eq!(
            world.resource::<InputContextStack>().current(),
            InputContext::Normal
        );
    }

    #[test]
    fn test_menu_input_quit() {
        let mut world = World::new();
        world.insert_resource(GameState::MainMenu);
        let active = start_scenario_definition(StartScenarioId::Classic);
        world.insert_resource(ActiveStartScenario {
            id: active.id,
            name: active.name,
            difficulty: active.difficulty,
        });
        world.insert_resource(InputContextStack::default());
        world
            .resource_mut::<InputContextStack>()
            .push(InputContext::MainMenu);

        world.insert_resource(MenuState {
            selected_index: 1,
            ..Default::default()
        }); // "Quit" selected

        route_input(&mut world, key_event(GameKeyCode::Enter));

        // Should transition to Quitting
        assert_eq!(*world.resource::<GameState>(), GameState::Quitting);
    }

    #[test]
    fn test_menu_start_game_applies_ground_survival_state() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        *world.resource_mut::<GameState>() = GameState::MainMenu;

        let mut stack = InputContextStack::default();
        stack.push(InputContext::MainMenu);
        world.insert_resource(stack);

        world.insert_resource(MenuState {
            selected_index: 0,
            selected_scenario: StartScenarioId::GroundSurvival,
            ..Default::default()
        });

        route_input(&mut world, key_event(GameKeyCode::Enter));

        assert_eq!(*world.resource::<GameState>(), GameState::Running);
        assert_eq!(
            world.query::<&Pop>().iter(&world).count(),
            4,
            "Ground Survival should reduce the opening pop count on the menu path"
        );
        assert!(
            world.resource::<ColonyResources>().food < 10.0,
            "Ground Survival should reduce food on the menu path"
        );
        assert!(
            world
                .resource::<Chronicle>()
                .events
                .iter()
                .any(|event| event.text.contains("hard landing")),
            "Ground Survival intro text should be added on the menu path"
        );
    }

    #[test]
    fn test_menu_start_game_applies_social_drama_state() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        *world.resource_mut::<GameState>() = GameState::MainMenu;

        let mut stack = InputContextStack::default();
        stack.push(InputContext::MainMenu);
        world.insert_resource(stack);

        world.insert_resource(MenuState {
            selected_index: 0,
            selected_scenario: StartScenarioId::SocialDrama,
            ..Default::default()
        });

        route_input(&mut world, key_event(GameKeyCode::Enter));

        let immigrant_count = world
            .query::<&crate::layer1::social::old_guard::Generation>()
            .iter(&world)
            .filter(|generation| {
                **generation == crate::layer1::social::old_guard::Generation::Immigrant
            })
            .count();
        assert_eq!(immigrant_count, 4);
        assert!(
            world
                .resource::<Chronicle>()
                .events
                .iter()
                .any(|event| event.text.contains("powder keg")),
            "Social Drama intro text should be added on the menu path"
        );
    }

    #[test]
    fn test_menu_start_game_applies_layer2_ready_state() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        *world.resource_mut::<GameState>() = GameState::MainMenu;

        let mut stack = InputContextStack::default();
        stack.push(InputContext::MainMenu);
        world.insert_resource(stack);

        world.insert_resource(MenuState {
            selected_index: 0,
            selected_scenario: StartScenarioId::Layer2Ready,
            ..Default::default()
        });

        route_input(&mut world, key_event(GameKeyCode::Enter));

        assert_eq!(
            *world.resource::<crate::layer2::visibility::SystemVisibility>(),
            crate::layer2::visibility::SystemVisibility::Full,
            "Layer 2 Ready should unlock system visibility on the menu path"
        );
        assert!(
            world
                .resource::<Chronicle>()
                .events
                .iter()
                .any(|event| event.text.contains("orbital charter")),
            "Layer 2 Ready intro text should be added on the menu path"
        );
    }
}
