//! Ledger events. One event is one JSON line in `.mesh/ledger.jsonl`.

use serde::{Deserialize, Serialize};

use crate::error::{MeshError, Result};
use crate::ids::{AgentId, Skill, TaskId, Token};
use crate::scope::validate_pattern;

/// Default failure budget of a task.
pub const DEFAULT_MAX_FAILURES: u32 = 3;
/// Largest failure budget of a task.
pub const MAX_MAX_FAILURES: u32 = 10;
/// Longest title.
pub const MAX_TITLE: usize = 200;
/// Longest free text (detail, evidence, reason, question, answer).
pub const MAX_TEXT: usize = 4000;
/// Longest worker label.
pub const MAX_WORKER: usize = 64;
/// Most path patterns on one task.
pub const MAX_PATHS: usize = 64;

const fn default_max_failures() -> u32 {
    DEFAULT_MAX_FAILURES
}

/// Checks a text field.
///
/// # Errors
/// [`MeshError::EmptyText`] if `required` and the text is blank,
/// [`MeshError::TextTooLong`] if it has more than `max` characters, and
/// [`MeshError::ControlChars`] if `one_line` and it has a control character.
pub fn validate_text(
    field: &'static str,
    text: &str,
    max: usize,
    required: bool,
    one_line: bool,
) -> Result<()> {
    if required && text.trim().is_empty() {
        return Err(MeshError::EmptyText(field));
    }
    let len = text.chars().count();
    if len > max {
        return Err(MeshError::TextTooLong { field, len, max });
    }
    let bad = |c: char| is_bidi(c) || (c.is_control() && (one_line || !matches!(c, '\n' | '\t')));
    if text.chars().any(bad) {
        return Err(MeshError::ControlChars(field));
    }
    Ok(())
}

/// `true` for characters that change the text direction. They can hide text in a terminal.
#[must_use]
pub const fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// The definition of a task. Plan files use the same fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    /// Unique id.
    pub id: TaskId,
    /// One-line title.
    pub title: String,
    /// The skill that a worker needs.
    pub skill: Skill,
    /// Tasks that must be `completed` first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<TaskId>,
    /// Path globs that the worker may change. Empty means the role scope only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Free text for the worker brief: spec path, check command, limits.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Failure budget.
    #[serde(default = "default_max_failures")]
    pub max_failures: u32,
}

impl TaskSpec {
    /// Makes a spec with default fields.
    #[must_use]
    pub fn new(id: TaskId, title: impl Into<String>, skill: Skill) -> Self {
        Self {
            id,
            title: title.into(),
            skill,
            deps: Vec::new(),
            paths: Vec::new(),
            detail: String::new(),
            max_failures: DEFAULT_MAX_FAILURES,
        }
    }

    /// Checks the text limits, the budget, and the path patterns.
    ///
    /// # Errors
    /// The first [`MeshError`] found.
    pub fn validate(&self) -> Result<()> {
        validate_text("title", &self.title, MAX_TITLE, true, true)?;
        validate_text("detail", &self.detail, MAX_TEXT, false, false)?;
        if !(1..=MAX_MAX_FAILURES).contains(&self.max_failures) {
            return Err(MeshError::InvalidBudget {
                got: self.max_failures,
                max: MAX_MAX_FAILURES,
            });
        }
        if self.paths.len() > MAX_PATHS {
            return Err(MeshError::InvalidPattern {
                pattern: format!("{} patterns", self.paths.len()),
                reason: format!("use at most {MAX_PATHS} patterns"),
            });
        }
        self.paths.iter().try_for_each(|p| validate_pattern(p))
    }
}

/// A change to the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A new task.
    TaskAdded {
        /// The task definition.
        task: TaskSpec,
    },
    /// A claim with a new lease.
    Claimed {
        /// The task.
        task: TaskId,
        /// The role.
        agent: AgentId,
        /// The worker label.
        worker: String,
        /// The new fencing token.
        token: Token,
        /// Lease expiry (Unix seconds).
        expires_at: u64,
    },
    /// A lease extension.
    Heartbeat {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// New expiry (Unix seconds).
        expires_at: u64,
    },
    /// Work done.
    Completed {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// Proof: commands run, results, commit.
        evidence: String,
    },
    /// Work failed.
    Failed {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// Why it failed.
        reason: String,
    },
    /// The worker needs an answer.
    InputRequested {
        /// The task.
        task: TaskId,
        /// The lease token.
        token: Token,
        /// The question.
        question: String,
    },
    /// The coordinator answered.
    Answered {
        /// The task.
        task: TaskId,
        /// The answer.
        answer: String,
    },
    /// The coordinator retried a failed task.
    Retried {
        /// The task.
        task: TaskId,
    },
    /// The coordinator canceled the task.
    Canceled {
        /// The task.
        task: TaskId,
        /// Why.
        reason: String,
    },
    /// The coordinator checked a rejected or broken ledger line. Doctor stops reporting it.
    Acknowledged {
        /// The fingerprint of the line (see `store::fingerprint`).
        fingerprint: String,
        /// What the coordinator did about it.
        note: String,
    },
    /// An overdue lease ended.
    LeaseExpired {
        /// The task.
        task: TaskId,
        /// The token of the ended lease.
        token: Token,
    },
}

impl Event {
    /// The task of the event. `Acknowledged` has no task.
    #[must_use]
    pub const fn task(&self) -> Option<&TaskId> {
        Some(match self {
            Self::Acknowledged { .. } => return None,
            Self::TaskAdded { task } => &task.id,
            Self::Claimed { task, .. }
            | Self::Heartbeat { task, .. }
            | Self::Completed { task, .. }
            | Self::Failed { task, .. }
            | Self::InputRequested { task, .. }
            | Self::Answered { task, .. }
            | Self::Retried { task }
            | Self::Canceled { task, .. }
            | Self::LeaseExpired { task, .. } => task,
        })
    }

    /// The short name of the event.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::TaskAdded { .. } => "task_added",
            Self::Claimed { .. } => "claimed",
            Self::Heartbeat { .. } => "heartbeat",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::InputRequested { .. } => "input_requested",
            Self::Answered { .. } => "answered",
            Self::Retried { .. } => "retried",
            Self::Canceled { .. } => "canceled",
            Self::LeaseExpired { .. } => "lease_expired",
            Self::Acknowledged { .. } => "acknowledged",
        }
    }
}

/// An event with its sequence number, time, and actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Sequence number. Each writer makes it larger than all earlier numbers.
    /// A git merge of two ledgers can repeat numbers; `doctor` reports that.
    pub seq: u64,
    /// Time (Unix seconds).
    pub ts: u64,
    /// Who wrote the event: a role id, `coordinator`, or `mesh`.
    pub actor: String,
    /// The event.
    pub event: Event,
}

impl Envelope {
    /// Encodes the envelope as one JSON line without the newline.
    ///
    /// # Errors
    /// [`MeshError::Ledger`] if encoding fails.
    pub fn to_line(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| MeshError::Ledger(e.to_string()))
    }

    /// Decodes one JSON line.
    ///
    /// # Errors
    /// [`MeshError::Ledger`] if the line is not a valid envelope.
    pub fn from_line(line: &str) -> Result<Self> {
        serde_json::from_str(line).map_err(|e| MeshError::Ledger(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(s: &str) -> TaskId {
        TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    fn skill() -> Skill {
        Skill::new("build").unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn validate_text_rules() {
        assert!(validate_text("title", "ok", 10, true, true).is_ok());
        assert!(validate_text("detail", "", 10, false, false).is_ok());
        assert!(matches!(
            validate_text("title", "  ", 10, true, true),
            Err(MeshError::EmptyText("title"))
        ));
        assert!(matches!(
            validate_text("title", "abcdef", 5, true, true),
            Err(MeshError::TextTooLong { len: 6, max: 5, .. })
        ));
        assert!(matches!(
            validate_text("title", "a\nb", 10, true, true),
            Err(MeshError::ControlChars("title"))
        ));
        assert!(validate_text("evidence", "line 1\nline 2", 100, true, false).is_ok());
        assert!(validate_text("title", "ééé", 3, true, true).is_ok());
    }

    #[test]
    fn spec_validation() {
        let ok = TaskSpec::new(tid("t1"), "Title", skill());
        assert!(ok.validate().is_ok());

        let mut bad = ok.clone();
        bad.title = String::new();
        assert!(bad.validate().is_err());

        let mut bad = ok.clone();
        bad.max_failures = 0;
        assert!(matches!(
            bad.validate(),
            Err(MeshError::InvalidBudget { .. })
        ));
        bad.max_failures = MAX_MAX_FAILURES + 1;
        assert!(matches!(
            bad.validate(),
            Err(MeshError::InvalidBudget { .. })
        ));

        let mut bad = ok.clone();
        bad.paths = vec!["src/../specs/**".into()];
        assert!(matches!(
            bad.validate(),
            Err(MeshError::InvalidPattern { .. })
        ));

        let mut bad = ok;
        bad.paths = vec!["x".into(); MAX_PATHS + 1];
        assert!(bad.validate().is_err());
    }

    #[test]
    fn envelope_round_trips_as_one_line() {
        let env = Envelope {
            seq: 3,
            ts: 100,
            actor: "builder".into(),
            event: Event::Completed {
                task: tid("t1"),
                token: Token::new(2, 0),
                evidence: "cargo test: 10 passed\ncommit abc".into(),
            },
        };
        let line = env.to_line().unwrap_or_else(|e| panic!("{e}"));
        assert!(!line.contains('\n'));
        assert!(line.contains("\"type\":\"completed\""));
        let back = Envelope::from_line(&line).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, env);
        assert_eq!(back.event.task().map(TaskId::as_str), Some("t1"));
        assert_eq!(back.event.name(), "completed");
    }

    #[test]
    fn from_line_rejects_garbage_and_bad_ids() {
        assert!(matches!(
            Envelope::from_line("{"),
            Err(MeshError::Ledger(_))
        ));
        let bad_id = r#"{"seq":1,"ts":1,"actor":"x","event":{"type":"retried","task":"../x"}}"#;
        assert!(Envelope::from_line(bad_id).is_err());
    }

    #[test]
    fn spec_rejects_unknown_fields() {
        let r: std::result::Result<TaskSpec, _> =
            serde_json::from_str(r#"{"id":"a","title":"t","skill":"build","dep":["b"]}"#);
        assert!(r.is_err());
    }
}
