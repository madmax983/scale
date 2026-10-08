#[cfg(feature = "nova")]
use super::plugins::ORAL_TRADITION_PLUGIN_TYPE;
use super::{
    commands::{
        build_default_command_registry, CommandRegistry, ShellCommand, ShellCommandAction,
        ShellCommandDomain,
    },
    config::{
        PersistedPaneBinding, PersistedShellLayout, PersistedWorkspaceLayout, ShellConfig,
        SHELL_LAYOUT_VERSION,
    },
    plugins::{
        register_default_plugins_with_runtime, SharedWorld, CHRONICLE_PLUGIN_TYPE,
        COLONY_MAP_PLUGIN_TYPE, INSPECTOR_PLUGIN_TYPE, REIGN_SKY_PLUGIN_TYPE, STATUS_PLUGIN_TYPE,
        SYSTEM_MAP_PLUGIN_TYPE, TECH_PLUGIN_TYPE,
    },
};
use crate::shared::keyboard::{GameKeyCode, GameKeyEvent};
use crate::{
    shared::view_mode::ViewMode,
    shared::{
        state::GameState,
        time::{SimSpeed, SimulationTime},
    },
};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};
use ratatui_hypertile::{
    raw::Node as LayoutNode, HypertileEvent, KeyChord, KeyCode, Modifiers, PaneId, SplitPolicy,
};
use ratatui_hypertile_extras::{HypertileRuntime, InputMode, SplitBehavior, WorkspaceRuntime};

const COLONY_OPS_WORKSPACE: &str = "Colony Ops";
const SYSTEM_SURVEY_WORKSPACE: &str = "System Survey";
const DIRECTOR_WORKSPACE: &str = "Director";
const LOGS_WORKSPACE: &str = "Logs";

/// Bootstrapped shell state for the hypertile migration.
///
/// This stays outside the ECS world for now because the runtime's plugin factories capture the
/// world handle; storing that back inside `World` would create a self-referential knot.
pub struct UiShell {
    world: SharedWorld,
    workspaces: WorkspaceRuntime,
    workspace_names: Vec<String>,
    config: ShellConfig,
    commands: CommandRegistry,
    command_palette: CommandPaletteState,
}

#[derive(Default)]
struct CommandPaletteState {
    is_open: bool,
    filter: String,
    selected: usize,
}

impl UiShell {
    #[must_use]
    pub fn has_workspace(&self, name: &str) -> bool {
        self.workspace_names
            .iter()
            .any(|workspace| workspace == name)
    }

    #[must_use]
    pub fn workspaces(&self) -> &WorkspaceRuntime {
        &self.workspaces
    }

    #[must_use]
    pub fn workspaces_mut(&mut self) -> &mut WorkspaceRuntime {
        &mut self.workspaces
    }

    #[must_use]
    pub fn active_workspace_name(&self) -> &str {
        &self.workspace_names[self.workspaces.active_tab_index()]
    }

    #[must_use]
    pub fn focused_plugin_type(&self) -> Option<&str> {
        let runtime = self.workspaces.active_runtime();
        let pane_id = runtime.focused_pane()?;
        runtime.registry().plugin_type_for(pane_id)
    }

    #[must_use]
    pub fn config(&self) -> &ShellConfig {
        &self.config
    }

    #[must_use]
    pub fn is_palette_open(&self) -> bool {
        self.command_palette.is_open
    }

    #[must_use]
    pub fn snapshot_config(&mut self) -> ShellConfig {
        self.sync_persisted_layout();
        self.config.clone()
    }

    #[must_use]
    pub fn command_registry(&self) -> &CommandRegistry {
        &self.commands
    }

    #[must_use]
    pub fn is_layout_mode(&self) -> bool {
        self.workspaces.active_runtime().mode() == InputMode::Layout
    }

    #[must_use]
    pub fn switch_to_workspace(&mut self, name: &str) -> bool {
        if let Some(index) = self
            .workspace_names
            .iter()
            .position(|workspace| workspace == name)
        {
            self.workspaces.go_to_tab(index);
            self.sync_persisted_layout();
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn active_workspace_contains_plugin(&self, plugin_type: &str) -> bool {
        find_pane_by_plugin_type(self.workspaces.active_runtime(), plugin_type).is_some()
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.workspaces.render(area, buf);
        if self.is_layout_mode() && !self.command_palette.is_open {
            self.render_layout_help(area, buf);
        }
        if self.command_palette.is_open {
            self.render_command_palette(area, buf);
        }
    }

    #[must_use]
    pub fn open_palette(&mut self) -> bool {
        self.command_palette = CommandPaletteState {
            is_open: true,
            ..CommandPaletteState::default()
        };
        true
    }

    #[must_use]
    pub fn handle_key(&mut self, key: GameKeyEvent) -> bool {
        if self.command_palette.is_open {
            return self.handle_command_palette_key(key);
        }

        let code = match key.code {
            GameKeyCode::Char(ch) => KeyCode::Char(ch),
            GameKeyCode::Enter => KeyCode::Enter,
            GameKeyCode::Esc => KeyCode::Escape,
            GameKeyCode::Tab => KeyCode::Tab,
            GameKeyCode::BackTab => KeyCode::BackTab,
            GameKeyCode::Backspace => KeyCode::Backspace,
            GameKeyCode::Delete => KeyCode::Delete,
            GameKeyCode::Up => KeyCode::Up,
            GameKeyCode::Down => KeyCode::Down,
            GameKeyCode::Left => KeyCode::Left,
            GameKeyCode::Right => KeyCode::Right,
        };

        let mut modifiers = Modifiers::NONE;
        if key.modifiers.shift {
            modifiers |= Modifiers::SHIFT;
        }
        if key.modifiers.ctrl {
            modifiers |= Modifiers::CTRL;
        }
        if key.modifiers.alt {
            modifiers |= Modifiers::ALT;
        }

        let consumed = self
            .workspaces
            .handle_event(HypertileEvent::Key(KeyChord::with_modifiers(
                code, modifiers,
            )))
            .is_consumed();
        if consumed {
            self.sync_persisted_layout();
        }
        consumed
    }

    fn handle_command_palette_key(&mut self, key: GameKeyEvent) -> bool {
        match key.code {
            GameKeyCode::Esc => {
                self.command_palette = CommandPaletteState::default();
                true
            }
            GameKeyCode::Up => {
                self.move_palette_selection(false);
                true
            }
            GameKeyCode::Down | GameKeyCode::Tab => {
                self.move_palette_selection(true);
                true
            }
            GameKeyCode::BackTab => {
                self.move_palette_selection(false);
                true
            }
            GameKeyCode::Backspace => {
                self.command_palette.filter.pop();
                self.clamp_palette_selection();
                true
            }
            GameKeyCode::Delete => {
                self.command_palette.filter.clear();
                self.command_palette.selected = 0;
                true
            }
            GameKeyCode::Enter => {
                let commands = self.filtered_palette_commands();
                let Some(&command) = commands.get(self.command_palette.selected) else {
                    return true;
                };
                let command = command.clone();
                self.command_palette = CommandPaletteState::default();
                self.execute_command(&command)
            }
            GameKeyCode::Char(ch) if !key.modifiers.ctrl && !key.modifiers.alt => {
                self.command_palette.filter.push(ch);
                self.command_palette.selected = 0;
                true
            }
            _ => true,
        }
    }

    #[must_use]
    pub fn execute_command_by_id(&mut self, id: &str) -> bool {
        let Some(command) = self.commands.get_by_id(id).cloned() else {
            return false;
        };
        self.execute_command(&command)
    }

    #[must_use]
    pub fn execute_command_by_label(&mut self, label: &str) -> bool {
        let Some(command) = self.commands.get_by_label(label).cloned() else {
            return false;
        };
        self.execute_command(&command)
    }

    fn execute_command(&mut self, command: &ShellCommand) -> bool {
        let executed = match &command.action {
            ShellCommandAction::SwitchWorkspace(name) => {
                let switched = self.switch_to_workspace(name);
                if switched {
                    self.sync_view_mode_for_workspace(name);
                }
                switched
            }
            ShellCommandAction::OpenPane(plugin_type) => self.focus_or_open_pane(plugin_type),
            ShellCommandAction::EnterLayoutMode => {
                self.workspaces
                    .active_runtime_mut()
                    .set_mode(InputMode::Layout);
                true
            }
            ShellCommandAction::ResetCurrentWorkspaceLayout => {
                self.reset_current_workspace_layout()
            }
            ShellCommandAction::PauseSimulation => {
                *self.world.borrow_mut().resource_mut::<GameState>() = GameState::Paused;
                true
            }
            ShellCommandAction::ResumeSimulation => {
                *self.world.borrow_mut().resource_mut::<GameState>() = GameState::Running;
                true
            }
            ShellCommandAction::SetSimulationSpeed(speed) => {
                let sim_speed = match speed {
                    1 => SimSpeed::Normal,
                    2 => SimSpeed::Fast,
                    3 => SimSpeed::Faster,
                    _ => return false,
                };
                self.world
                    .borrow_mut()
                    .resource_mut::<SimulationTime>()
                    .speed = sim_speed;
                true
            }
        };

        if executed {
            self.record_recent_command(command.label);
            self.sync_persisted_layout();
        }

        executed
    }

    fn sync_view_mode_for_workspace(&self, workspace_name: &str) {
        let mut world = self.world.borrow_mut();
        let mut view_mode = world.resource_mut::<ViewMode>();
        match workspace_name {
            SYSTEM_SURVEY_WORKSPACE => *view_mode = ViewMode::System,
            COLONY_OPS_WORKSPACE => *view_mode = ViewMode::Colony,
            DIRECTOR_WORKSPACE => {}
            _ => {}
        }
    }

    fn focus_or_open_pane(&mut self, plugin_type: &str) -> bool {
        let runtime = self.workspaces.active_runtime_mut();
        if let Some(existing_pane) = find_pane_by_plugin_type(runtime, plugin_type) {
            let focused = runtime.focus_pane(existing_pane).is_ok();
            runtime.set_mode(InputMode::PluginInput);
            return focused;
        }

        let placement = preferred_pane_placement(plugin_type);
        if let Some(anchor_pane) = find_anchor_pane(runtime, placement.anchor_plugin_types) {
            let _ = runtime.focus_pane(anchor_pane);
        }

        let opened = runtime
            .split_focused(placement.direction, plugin_type)
            .is_ok();
        runtime.set_mode(InputMode::PluginInput);
        opened
    }

    fn reset_current_workspace_layout(&mut self) -> bool {
        let workspace_name = self.active_workspace_name().to_string();
        apply_workspace_preset(self.workspaces.active_runtime_mut(), &workspace_name);
        self.workspaces
            .active_runtime_mut()
            .set_mode(InputMode::PluginInput);
        true
    }

    fn record_recent_command(&mut self, label: &str) {
        self.config.recent_commands.retain(|item| item != label);
        self.config.recent_commands.insert(0, label.to_string());
        self.config.recent_commands.truncate(16);
    }

    fn filtered_palette_commands(&self) -> Vec<&ShellCommand> {
        let filter_bytes = self.command_palette.filter.as_bytes();

        // ⚡ Bolt Optimization: Removed intermediate `.collect::<Vec<_>>()` and pre-allocate capacity
        let mut commands = Vec::with_capacity(self.commands.commands().len());
        commands.extend(self.commands.commands().iter().filter(|command| {
            if filter_bytes.is_empty() {
                return true;
            }
            let label_bytes = command.label.as_bytes();
            if filter_bytes.len() > label_bytes.len() {
                return false;
            }
            label_bytes
                .windows(filter_bytes.len())
                .any(|window| window.eq_ignore_ascii_case(filter_bytes))
        }));

        commands.sort_by_key(|command| {
            self.config
                .recent_commands
                .iter()
                .position(|recent| recent == command.label)
                .unwrap_or(usize::MAX)
        });
        commands
    }

    fn move_palette_selection(&mut self, forward: bool) {
        let command_count = self.filtered_palette_commands().len();
        if command_count == 0 {
            self.command_palette.selected = 0;
            return;
        }

        self.command_palette.selected = if forward {
            (self.command_palette.selected + 1) % command_count
        } else {
            (self.command_palette.selected + command_count - 1) % command_count
        };
    }

    fn clamp_palette_selection(&mut self) {
        let command_count = self.filtered_palette_commands().len();
        if command_count == 0 {
            self.command_palette.selected = 0;
        } else if self.command_palette.selected >= command_count {
            self.command_palette.selected = command_count - 1;
        }
    }

    fn build_palette_items<'a>(&self, commands: &[&'a ShellCommand]) -> Vec<ListItem<'a>> {
        if commands.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matching commands",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            commands
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    let domain = match command.domain {
                        ShellCommandDomain::Shell => "shell",
                        ShellCommandDomain::Gameplay => "game",
                    };
                    let prefix = if self.command_palette.selected == index {
                        ">"
                    } else {
                        " "
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{prefix} "), Style::default().fg(Color::Yellow)),
                        Span::raw(command.label),
                        Span::styled(
                            format!("  [{domain}]"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]))
                })
                .collect()
        }
    }

    fn render_command_palette(&self, area: Rect, buf: &mut Buffer) {
        let popup_area = centered_rect(70, 60, area);
        let commands = self.filtered_palette_commands();
        let title = if self.command_palette.filter.is_empty() {
            " Command Palette "
        } else {
            " Command Palette (Filtered) "
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Black));

        let items = self.build_palette_items(&commands);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(2),
            ])
            .split(block.inner(popup_area));

        Clear.render(popup_area, buf);
        block.render(popup_area, buf);
        Paragraph::new(self.command_palette.filter.as_str())
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Left)
            .render(layout[0], buf);
        List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .render(layout[1], buf);
        Paragraph::new("Enter runs command. Esc closes. Type to filter.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Left)
            .render(layout[2], buf);
    }

    fn render_layout_help(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let help_area = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(1),
            area.width,
            1,
        );
        let help = "LAYOUT: hjkl focus  Shift+HJKL move  [ ] resize  s/v split  d close  Enter interact  Esc play";
        Paragraph::new(help)
            .style(Style::default().bg(Color::DarkGray).fg(Color::Cyan))
            .alignment(Alignment::Left)
            .render(help_area, buf);
    }

    fn sync_persisted_layout(&mut self) {
        let active_workspace = self.active_workspace_name().to_string();
        self.config.startup_workspace = active_workspace;
        let layout = self.snapshot_persisted_layout();
        let _ = self.config.store_persisted_layout(&layout);
    }

    fn snapshot_persisted_layout(&mut self) -> PersistedShellLayout {
        let active_index = self.workspaces.active_tab_index();
        let mut workspaces = Vec::with_capacity(self.workspace_names.len());

        for index in 0..self.workspace_names.len() {
            self.workspaces.go_to_tab(index);
            let runtime = self.workspaces.active_runtime();
            workspaces.push(PersistedWorkspaceLayout {
                name: self.workspace_names[index].clone(),
                root: runtime.core().root().clone(),
                focused_pane: runtime.focused_pane(),
                panes: runtime
                    .core()
                    .state()
                    .pane_ids()
                    .into_iter()
                    .filter_map(|pane_id| {
                        runtime
                            .registry()
                            .plugin_type_for(pane_id)
                            .map(|plugin_type| PersistedPaneBinding {
                                pane_id,
                                plugin_type: plugin_type.to_string(),
                            })
                    })
                    .collect(),
            });
        }

        self.workspaces.go_to_tab(active_index);

        PersistedShellLayout {
            version: SHELL_LAYOUT_VERSION,
            active_workspace: self.workspace_names[active_index].clone(),
            workspaces,
        }
    }

    fn restore_from_persisted_layout(
        &mut self,
        layout: &PersistedShellLayout,
    ) -> Result<(), String> {
        for workspace in &layout.workspaces {
            let Some(index) = self
                .workspace_names
                .iter()
                .position(|name| name == &workspace.name)
            else {
                continue;
            };
            self.workspaces.go_to_tab(index);
            apply_workspace_layout(self.workspaces.active_runtime_mut(), workspace)?;
        }

        let target_index = self
            .workspace_names
            .iter()
            .position(|name| name == &layout.active_workspace)
            .ok_or_else(|| format!("unknown workspace {}", layout.active_workspace))?;
        self.workspaces.go_to_tab(target_index);
        self.workspaces
            .active_runtime_mut()
            .set_mode(InputMode::PluginInput);
        Ok(())
    }
}

fn initialize_curated_workspace(
    workspaces: &mut WorkspaceRuntime,
    world: &SharedWorld,
    workspace_name: &str,
) {
    register_default_plugins_with_runtime(workspaces.active_runtime_mut(), world.clone());
    workspaces.rename_tab(workspaces.active_tab_index(), workspace_name.to_string());
    apply_workspace_preset(workspaces.active_runtime_mut(), workspace_name);
    workspaces
        .active_runtime_mut()
        .set_mode(InputMode::PluginInput);
}

#[must_use]
pub fn build_default_shell(world: SharedWorld, config: ShellConfig) -> UiShell {
    let commands = build_default_command_registry();
    let had_persisted_layout = config.persisted_layout.is_some();
    let persisted_layout = config.decoded_persisted_layout();
    let workspace_names = vec![
        String::from(COLONY_OPS_WORKSPACE),
        String::from(SYSTEM_SURVEY_WORKSPACE),
        String::from(DIRECTOR_WORKSPACE),
        String::from(LOGS_WORKSPACE),
    ];

    let mut workspaces = WorkspaceRuntime::new(|| {
        HypertileRuntime::builder()
            .with_resize_step(0.08)
            .with_split_policy(SplitPolicy::Golden)
            .with_split_behavior(SplitBehavior::PromptPalette)
            .with_default_split_plugin(COLONY_MAP_PLUGIN_TYPE)
            .build()
    });

    initialize_curated_workspace(&mut workspaces, &world, COLONY_OPS_WORKSPACE);

    for workspace_name in [SYSTEM_SURVEY_WORKSPACE, DIRECTOR_WORKSPACE, LOGS_WORKSPACE] {
        workspaces.new_tab();
        initialize_curated_workspace(&mut workspaces, &world, workspace_name);
    }

    let mut shell = UiShell {
        world,
        workspaces,
        workspace_names,
        config,
        commands,
        command_palette: CommandPaletteState::default(),
    };

    let restored = persisted_layout
        .as_ref()
        .and_then(|layout| shell.restore_from_persisted_layout(layout).ok())
        .is_some();
    if !restored {
        let target_workspace = if had_persisted_layout {
            COLONY_OPS_WORKSPACE
        } else {
            shell.config.startup_workspace.as_str()
        };
        if let Some(index) = shell
            .workspace_names
            .iter()
            .position(|workspace| workspace == target_workspace)
        {
            shell.workspaces.go_to_tab(index);
        } else {
            shell.workspaces.go_to_tab(0);
        }
    }

    shell.sync_persisted_layout();
    shell
}

#[derive(Clone, Copy)]
struct PanePlacement {
    anchor_plugin_types: &'static [&'static str],
    direction: Direction,
}

fn preferred_pane_placement(plugin_type: &str) -> PanePlacement {
    match plugin_type {
        CHRONICLE_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                INSPECTOR_PLUGIN_TYPE,
                COLONY_MAP_PLUGIN_TYPE,
                SYSTEM_MAP_PLUGIN_TYPE,
            ],
            direction: Direction::Vertical,
        },
        TECH_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                CHRONICLE_PLUGIN_TYPE,
                INSPECTOR_PLUGIN_TYPE,
                COLONY_MAP_PLUGIN_TYPE,
            ],
            direction: Direction::Horizontal,
        },
        REIGN_SKY_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                CHRONICLE_PLUGIN_TYPE,
                TECH_PLUGIN_TYPE,
                INSPECTOR_PLUGIN_TYPE,
            ],
            direction: Direction::Vertical,
        },
        #[cfg(feature = "nova")]
        ORAL_TRADITION_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                INSPECTOR_PLUGIN_TYPE,
                COLONY_MAP_PLUGIN_TYPE,
                SYSTEM_MAP_PLUGIN_TYPE,
            ],
            direction: Direction::Vertical,
        },
        STATUS_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[COLONY_MAP_PLUGIN_TYPE, SYSTEM_MAP_PLUGIN_TYPE],
            direction: Direction::Vertical,
        },
        INSPECTOR_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                COLONY_MAP_PLUGIN_TYPE,
                SYSTEM_MAP_PLUGIN_TYPE,
                CHRONICLE_PLUGIN_TYPE,
            ],
            direction: Direction::Horizontal,
        },
        SYSTEM_MAP_PLUGIN_TYPE | COLONY_MAP_PLUGIN_TYPE => PanePlacement {
            anchor_plugin_types: &[
                INSPECTOR_PLUGIN_TYPE,
                CHRONICLE_PLUGIN_TYPE,
                TECH_PLUGIN_TYPE,
                #[cfg(feature = "nova")]
                ORAL_TRADITION_PLUGIN_TYPE,
            ],
            direction: Direction::Horizontal,
        },
        _ => PanePlacement {
            anchor_plugin_types: &[COLONY_MAP_PLUGIN_TYPE, SYSTEM_MAP_PLUGIN_TYPE],
            direction: Direction::Horizontal,
        },
    }
}

fn find_pane_by_plugin_type(runtime: &HypertileRuntime, plugin_type: &str) -> Option<PaneId> {
    runtime
        .core()
        .state()
        .pane_ids()
        .into_iter()
        .find(|pane_id| runtime.registry().plugin_type_for(*pane_id) == Some(plugin_type))
}

fn find_anchor_pane(runtime: &HypertileRuntime, anchor_plugin_types: &[&str]) -> Option<PaneId> {
    anchor_plugin_types
        .iter()
        .find_map(|plugin_type| find_pane_by_plugin_type(runtime, plugin_type))
        .or_else(|| runtime.focused_pane())
}

fn apply_workspace_layout(
    runtime: &mut HypertileRuntime,
    workspace: &PersistedWorkspaceLayout,
) -> Result<(), String> {
    runtime
        .set_root(workspace.root.clone())
        .map_err(|error| error.to_string())?;

    let pane_ids: Vec<_> = runtime.core().state().pane_ids().collect();
    for pane_id in pane_ids {
        let Some(binding) = workspace
            .panes
            .iter()
            .find(|binding| binding.pane_id == pane_id)
        else {
            return Err(format!(
                "workspace {} is missing a plugin binding for pane {}",
                workspace.name,
                pane_id.get()
            ));
        };
        if runtime
            .registry()
            .registered_types()
            .all(|plugin_type| plugin_type != binding.plugin_type)
        {
            return Err(format!(
                "workspace {} references unknown plugin type {}",
                workspace.name, binding.plugin_type
            ));
        }
        runtime
            .replace_pane_plugin(pane_id, &binding.plugin_type)
            .map_err(|error| error.to_string())?;
    }

    if let Some(focused_pane) = workspace.focused_pane {
        runtime
            .focus_pane(focused_pane)
            .map_err(|error| error.to_string())?;
    }
    runtime.set_mode(InputMode::PluginInput);
    Ok(())
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn apply_workspace_preset(runtime: &mut HypertileRuntime, workspace_name: &str) {
    let preset = match workspace_name {
        COLONY_OPS_WORKSPACE => colony_ops_layout(),
        SYSTEM_SURVEY_WORKSPACE => system_survey_layout(),
        DIRECTOR_WORKSPACE => director_layout(),
        _ => colony_ops_layout(),
    };

    apply_workspace_layout(runtime, &preset)
        .expect("workspace presets should always produce valid layouts");
}

fn colony_ops_layout() -> PersistedWorkspaceLayout {
    PersistedWorkspaceLayout {
        name: COLONY_OPS_WORKSPACE.to_string(),
        root: LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: 0.95,
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: 0.72,
                first: Box::new(LayoutNode::Pane(PaneId::ROOT)),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Vertical,
                    ratio: 0.62,
                    first: Box::new(LayoutNode::Pane(PaneId::new(1))),
                    second: Box::new(LayoutNode::Pane(PaneId::new(2))),
                }),
            }),
            second: Box::new(LayoutNode::Pane(PaneId::new(3))),
        },
        focused_pane: Some(PaneId::ROOT),
        panes: vec![
            PersistedPaneBinding {
                pane_id: PaneId::ROOT,
                plugin_type: COLONY_MAP_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(1),
                plugin_type: INSPECTOR_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(2),
                plugin_type: CHRONICLE_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(3),
                plugin_type: STATUS_PLUGIN_TYPE.to_string(),
            },
        ],
    }
}

fn system_survey_layout() -> PersistedWorkspaceLayout {
    PersistedWorkspaceLayout {
        name: SYSTEM_SURVEY_WORKSPACE.to_string(),
        root: LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: 0.95,
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: 0.74,
                first: Box::new(LayoutNode::Pane(PaneId::ROOT)),
                second: Box::new(LayoutNode::Pane(PaneId::new(1))),
            }),
            second: Box::new(LayoutNode::Pane(PaneId::new(2))),
        },
        focused_pane: Some(PaneId::ROOT),
        panes: vec![
            PersistedPaneBinding {
                pane_id: PaneId::ROOT,
                plugin_type: SYSTEM_MAP_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(1),
                plugin_type: INSPECTOR_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(2),
                plugin_type: STATUS_PLUGIN_TYPE.to_string(),
            },
        ],
    }
}

fn director_layout() -> PersistedWorkspaceLayout {
    PersistedWorkspaceLayout {
        name: DIRECTOR_WORKSPACE.to_string(),
        root: LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: 0.94,
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: 0.58,
                first: Box::new(LayoutNode::Pane(PaneId::ROOT)),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    first: Box::new(LayoutNode::Split {
                        direction: Direction::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutNode::Pane(PaneId::new(1))),
                        second: Box::new(LayoutNode::Pane(PaneId::new(2))),
                    }),
                    // Reign & Sky: constellation, god-mind, sovereign panels.
                    second: Box::new(LayoutNode::Pane(PaneId::new(4))),
                }),
            }),
            second: Box::new(LayoutNode::Pane(PaneId::new(3))),
        },
        focused_pane: Some(PaneId::ROOT),
        panes: vec![
            PersistedPaneBinding {
                pane_id: PaneId::ROOT,
                plugin_type: CHRONICLE_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(1),
                plugin_type: TECH_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(2),
                plugin_type: INSPECTOR_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(3),
                plugin_type: STATUS_PLUGIN_TYPE.to_string(),
            },
            PersistedPaneBinding {
                pane_id: PaneId::new(4),
                plugin_type: REIGN_SKY_PLUGIN_TYPE.to_string(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{setup_world_with_config, SetupConfig};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn default_shell_contains_curated_workspaces() {
        let shell = build_default_shell_for_test();

        assert!(shell.has_workspace(COLONY_OPS_WORKSPACE));
        assert!(shell.has_workspace(SYSTEM_SURVEY_WORKSPACE));
        assert!(shell.has_workspace(DIRECTOR_WORKSPACE));
    }

    #[test]
    fn director_workspace_contains_reign_sky_pane() {
        let mut shell = build_default_shell_for_test();
        let switched = shell.switch_to_workspace(DIRECTOR_WORKSPACE);

        assert!(switched);
        assert!(
            shell.active_workspace_contains_plugin(REIGN_SKY_PLUGIN_TYPE),
            "Director workspace should surface the Reign & Sky pane"
        );
    }

    #[test]
    fn command_registry_exposes_pause_and_open_chronicle() {
        let registry = build_command_registry_for_test();

        assert!(registry.contains("Pause Simulation"));
        assert!(registry.contains("Open Chronicle"));
        assert!(registry.contains("Reset Current Workspace Layout"));
    }

    #[test]
    fn open_chronicle_command_creates_or_focuses_chronicle_pane() {
        let mut shell = build_default_shell_for_test();
        let switched = shell.switch_to_workspace(SYSTEM_SURVEY_WORKSPACE);

        assert!(switched);
        assert!(!shell.active_workspace_contains_plugin(CHRONICLE_PLUGIN_TYPE));

        let executed = shell.execute_command_by_label("Open Chronicle");

        assert!(executed);
        assert!(shell.active_workspace_contains_plugin(CHRONICLE_PLUGIN_TYPE));
    }

    #[test]
    fn open_existing_chronicle_command_does_not_duplicate_panes() {
        let mut shell = build_default_shell_for_test();
        let before = count_plugin_instances(&shell, CHRONICLE_PLUGIN_TYPE);

        let executed = shell.execute_command_by_label("Open Chronicle");
        let after = count_plugin_instances(&shell, CHRONICLE_PLUGIN_TYPE);

        assert!(executed);
        assert_eq!(before, after);
    }

    #[test]
    fn command_palette_can_enter_layout_mode() {
        let mut shell = build_default_shell_for_test();

        assert!(shell.open_palette());
        for ch in ['l', 'a', 'y'] {
            assert!(shell.handle_key(GameKeyEvent::new(GameKeyCode::Char(ch))));
        }
        assert!(shell.handle_key(GameKeyEvent::new(GameKeyCode::Enter)));

        assert!(shell.is_layout_mode());
    }

    #[test]
    fn colony_ops_starts_with_compact_status_footer() {
        let mut shell = build_default_shell_for_test();

        let status_rect = pane_rect_for_plugin(&mut shell, STATUS_PLUGIN_TYPE);

        assert!(status_rect.height <= 4);
        assert_eq!(shell.focused_plugin_type(), Some(COLONY_MAP_PLUGIN_TYPE));
    }

    #[test]
    fn reset_current_workspace_layout_restores_compact_status_footer() {
        let mut shell = build_default_shell_for_test();
        let status_pane =
            find_pane_by_plugin_type(shell.workspaces().active_runtime(), STATUS_PLUGIN_TYPE)
                .expect("status pane should exist");
        shell
            .workspaces_mut()
            .active_runtime_mut()
            .focus_pane(status_pane)
            .expect("status pane should be focusable");
        shell
            .workspaces_mut()
            .active_runtime_mut()
            .set_focused_ratio(0.5)
            .expect("status footer ratio should be adjustable");

        let bloated = pane_rect_for_plugin(&mut shell, STATUS_PLUGIN_TYPE);
        assert!(bloated.height > 4);

        let executed = shell.execute_command_by_label("Reset Current Workspace Layout");
        let reset = pane_rect_for_plugin(&mut shell, STATUS_PLUGIN_TYPE);

        assert!(executed);
        assert!(reset.height <= 4);
        assert_eq!(shell.focused_plugin_type(), Some(COLONY_MAP_PLUGIN_TYPE));
    }

    #[test]
    fn layout_mode_renders_resize_help() {
        let mut shell = build_default_shell_for_test();
        let area = Rect::new(0, 0, 120, 40);
        let mut buffer = Buffer::empty(area);

        assert!(shell.execute_command_by_label("Enter Layout Mode"));
        shell.render(area, &mut buffer);

        let text = buffer_text(&buffer);
        assert!(text.contains("resize"));
        assert!(text.contains("LAYOUT:"));
    }

    fn build_default_shell_for_test() -> UiShell {
        let world = Rc::new(RefCell::new(setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        })));
        build_default_shell(world, ShellConfig::default())
    }

    fn build_command_registry_for_test() -> CommandRegistry {
        build_default_command_registry()
    }

    fn count_plugin_instances(shell: &UiShell, plugin_type: &str) -> usize {
        let runtime = shell.workspaces().active_runtime();
        runtime
            .core()
            .state()
            .pane_ids()
            .into_iter()
            .filter(|pane_id| runtime.registry().plugin_type_for(*pane_id) == Some(plugin_type))
            .count()
    }

    fn pane_rect_for_plugin(shell: &mut UiShell, plugin_type: &str) -> Rect {
        let area = Rect::new(0, 0, 120, 40);
        let mut buffer = Buffer::empty(area);
        shell.render(area, &mut buffer);

        let runtime = shell.workspaces().active_runtime();
        runtime
            .panes()
            .into_iter()
            .find(|pane| runtime.registry().plugin_type_for(pane.id) == Some(plugin_type))
            .map(|pane| pane.rect)
            .expect("plugin should have a rendered pane rect")
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
