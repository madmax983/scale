//! Errors of the agent mesh.

use std::path::PathBuf;

use crate::ids::{AgentId, Skill, TaskId, Token};
use crate::lifecycle::State;

/// An error from a mesh operation. Each message tells the reader what to do.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    /// The text is not a valid id.
    #[error(
        "invalid id {0:?}: use 1-64 characters a-z, 0-9, '.', '_', '-'; start with a-z or 0-9"
    )]
    InvalidId(String),
    /// The text is not a valid token.
    #[error(
        "invalid token {0:?}: use the token that `mesh claim` printed, for example 7-9f3a2b1c"
    )]
    InvalidToken(String),
    /// A required text field is empty.
    #[error("{0} must not be empty")]
    EmptyText(&'static str),
    /// A text field is too long.
    #[error("{field} is too long: {len} characters (limit {max})")]
    TextTooLong {
        /// Field name.
        field: &'static str,
        /// Actual length.
        len: usize,
        /// Limit.
        max: usize,
    },
    /// A single-line field contains a control character.
    #[error("{0} must be one line without control characters")]
    ControlChars(&'static str),
    /// The lease time is out of range.
    #[error("ttl must be 1..={max} seconds, got {ttl}")]
    InvalidTtl {
        /// Given value.
        ttl: u64,
        /// Limit.
        max: u64,
    },
    /// The failure budget is out of range.
    #[error("max_failures must be 1..={max}, got {got}")]
    InvalidBudget {
        /// Given value.
        got: u32,
        /// Limit.
        max: u32,
    },
    /// A path glob is not valid.
    #[error("invalid path pattern `{pattern}`: {reason}")]
    InvalidPattern {
        /// The pattern.
        pattern: String,
        /// Why it is not valid.
        reason: String,
    },
    /// The task does not exist.
    #[error("task `{0}` does not exist; run `mesh status`")]
    UnknownTask(TaskId),
    /// The task id is already used.
    #[error("task `{0}` already exists; use a new id")]
    DuplicateTask(TaskId),
    /// A dependency does not exist.
    #[error("task `{task}` depends on unknown task `{dep}`")]
    UnknownDependency {
        /// The task.
        task: TaskId,
        /// The missing dependency.
        dep: TaskId,
    },
    /// The plan has a dependency cycle.
    #[error("dependency cycle: {}", join(.0))]
    Cycle(Vec<TaskId>),
    /// The agent is not in the registry.
    #[error("agent `{0}` is not in the registry; run `mesh agents`")]
    UnknownAgent(AgentId),
    /// No agent has the skill of the task.
    #[error("no agent has skill `{skill}` (task `{task}`); add the skill to an agent card")]
    UnservedSkill {
        /// The task.
        task: TaskId,
        /// The skill.
        skill: Skill,
    },
    /// The agent does not have the skill of the task.
    #[error("agent `{agent}` does not have skill `{skill}` for task `{task}`; run `mesh agents`")]
    SkillMismatch {
        /// The task.
        task: TaskId,
        /// The agent.
        agent: AgentId,
        /// The skill.
        skill: Skill,
    },
    /// The agent has too many working tasks.
    #[error("agent `{agent}` has {active} working tasks; the limit is {limit}; complete or fail one first")]
    WipLimit {
        /// The agent.
        agent: AgentId,
        /// Working tasks now.
        active: usize,
        /// Limit.
        limit: u32,
    },
    /// No task is ready for the agent.
    #[error("no ready task for agent `{0}`; run `mesh status`")]
    NothingReady(AgentId),
    /// The operation is not valid in this state.
    #[error("cannot {op} task `{task}` in state `{state}`")]
    InvalidTransition {
        /// The task.
        task: TaskId,
        /// The operation.
        op: &'static str,
        /// The current state.
        state: State,
    },
    /// The task waits for dependencies.
    #[error("task `{task}` is not ready; it waits for: {}", join(.waiting))]
    NotReady {
        /// The task.
        task: TaskId,
        /// Dependencies that are not completed.
        waiting: Vec<TaskId>,
    },
    /// A dependency failed or was canceled, so the task cannot start.
    #[error("task `{task}` is blocked by: {}; retry them, or cancel this task", join(.by))]
    Blocked {
        /// The task.
        task: TaskId,
        /// The dependencies that block it.
        by: Vec<TaskId>,
    },
    /// The token is not the token of the current lease.
    #[error("stale token {token} for task `{task}`; the lease is lost; stop work on this task")]
    StaleToken {
        /// The task.
        task: TaskId,
        /// The given token.
        token: Token,
    },
    /// The token is not larger than all earlier tokens.
    #[error("token {token} for task `{task}` is not fresh; a merge or a copy reused it")]
    TokenReused {
        /// The task.
        task: TaskId,
        /// The token.
        token: Token,
    },
    /// A counter reached its largest value.
    #[error("the {0} counter is at its limit; start a new ledger")]
    Exhausted(&'static str),
    /// The lease time is over.
    #[error("the lease of task `{0}` expired; stop work on this task")]
    LeaseExpired(TaskId),
    /// The task used all of its failure budget.
    #[error("task `{task}` used its failure budget ({max}); cancel it or add a new task")]
    BudgetExhausted {
        /// The task.
        task: TaskId,
        /// The budget.
        max: u32,
    },
    /// The registry file is not valid.
    #[error("registry: {0}")]
    Registry(String),
    /// The plan file is not valid.
    #[error("plan: {0}")]
    Plan(String),
    /// A ledger line is not valid.
    #[error("ledger: {0}")]
    Ledger(String),
    /// No mesh folder was found.
    #[error("no mesh found; run `mesh init` in the repository root or set MESH_DIR")]
    NotFound,
    /// A file operation failed.
    #[error("i/o error at {}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A git command failed.
    #[error("git: {0}")]
    Git(String),
}

impl MeshError {
    /// Wraps an I/O error with its path.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

fn join(ids: &[TaskId]) -> String {
    ids.iter()
        .map(TaskId::as_str)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// Result type of the mesh.
pub type Result<T, E = MeshError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> TaskId {
        TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn cycle_message_shows_the_path() {
        let e = MeshError::Cycle(vec![id("a"), id("b"), id("a")]);
        assert_eq!(e.to_string(), "dependency cycle: a -> b -> a");
    }

    #[test]
    fn stale_token_message_tells_the_worker_to_stop() {
        let e = MeshError::StaleToken {
            task: id("t1"),
            token: Token::new(4, 0),
        };
        assert!(e.to_string().contains("stop work"));
    }
}
