#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellCommandDomain {
    Shell,
    Gameplay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellCommandAction {
    SwitchWorkspace(&'static str),
    OpenPane(&'static str),
    EnterLayoutMode,
    ResetCurrentWorkspaceLayout,
    PauseSimulation,
    ResumeSimulation,
    SetSimulationSpeed(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub domain: ShellCommandDomain,
    pub action: ShellCommandAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandRegistry {
    commands: Vec<ShellCommand>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new(commands: Vec<ShellCommand>) -> Self {
        Self { commands }
    }

    #[must_use]
    pub fn commands(&self) -> &[ShellCommand] {
        &self.commands
    }

    #[must_use]
    pub fn contains(&self, label: &str) -> bool {
        self.commands.iter().any(|command| command.label == label)
    }

    #[must_use]
    pub fn get_by_id(&self, id: &str) -> Option<&ShellCommand> {
        self.commands.iter().find(|command| command.id == id)
    }

    #[must_use]
    pub fn get_by_label(&self, label: &str) -> Option<&ShellCommand> {
        self.commands.iter().find(|command| command.label == label)
    }
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build_default_command_registry() -> CommandRegistry {
    CommandRegistry::new(vec![
        ShellCommand {
            id: "workspace.colony_ops",
            label: "Switch to Colony Ops",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::SwitchWorkspace("Colony Ops"),
        },
        ShellCommand {
            id: "workspace.system_survey",
            label: "Switch to System Survey",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::SwitchWorkspace("System Survey"),
        },
        ShellCommand {
            id: "workspace.director",
            label: "Switch to Director",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::SwitchWorkspace("Director"),
        },
        ShellCommand {
            id: "pane.colony_map",
            label: "Open Colony Map",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::OpenPane("colony-map"),
        },
        ShellCommand {
            id: "pane.system_map",
            label: "Open System Map",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::OpenPane("system-map"),
        },
        ShellCommand {
            id: "pane.inspector",
            label: "Open Inspector",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::OpenPane("inspector"),
        },
        ShellCommand {
            id: "pane.status",
            label: "Open Status",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::OpenPane("status"),
        },
        ShellCommand {
            id: "pane.chronicle",
            label: "Open Chronicle",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::OpenPane("chronicle"),
        },
        ShellCommand {
            id: "pane.field_guide",
            label: "Open Field Guide",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::OpenPane("field-guide"),
        },
        ShellCommand {
            id: "pane.tech",
            label: "Open Tech Tree",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::OpenPane("tech"),
        },
        #[cfg(feature = "nova")]
        ShellCommand {
            id: "pane.oral_tradition",
            label: "Open Oral Tradition",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::OpenPane("oral-tradition"),
        },
        ShellCommand {
            id: "shell.layout_mode",
            label: "Enter Layout Mode",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::EnterLayoutMode,
        },
        ShellCommand {
            id: "shell.reset_current_workspace",
            label: "Reset Current Workspace Layout",
            domain: ShellCommandDomain::Shell,
            action: ShellCommandAction::ResetCurrentWorkspaceLayout,
        },
        ShellCommand {
            id: "sim.pause",
            label: "Pause Simulation",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::PauseSimulation,
        },
        ShellCommand {
            id: "sim.resume",
            label: "Resume Simulation",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::ResumeSimulation,
        },
        ShellCommand {
            id: "sim.speed_1",
            label: "Set Simulation Speed 1x",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::SetSimulationSpeed(1),
        },
        ShellCommand {
            id: "sim.speed_2",
            label: "Set Simulation Speed 2x",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::SetSimulationSpeed(2),
        },
        ShellCommand {
            id: "sim.speed_3",
            label: "Set Simulation Speed 3x",
            domain: ShellCommandDomain::Gameplay,
            action: ShellCommandAction::SetSimulationSpeed(3),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_contains_open_field_guide() {
        let registry = build_default_command_registry();
        let command = registry
            .get_by_label("Open Field Guide")
            .expect("field guide command should be registered");
        assert_eq!(command.id, "pane.field_guide");
        assert_eq!(command.action, ShellCommandAction::OpenPane("field-guide"));
        assert_eq!(command.domain, ShellCommandDomain::Gameplay);
    }
}
