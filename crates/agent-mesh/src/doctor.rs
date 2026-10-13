//! Health checks for a mesh. CI runs `mesh doctor`. Errors fail; warnings need attention.

use std::fmt;
use std::path::Path;

use serde::Serialize;

use crate::ledger::Readiness;
use crate::registry::Registry;
use crate::store::{Replay, ReplayIssue, Store};
use crate::time::fmt_utc;

/// The `.gitattributes` line that makes git merge ledgers by union with LF line endings.
pub const UNION_RULE: &str = ".mesh/ledger.jsonl text eol=lf merge=union";

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The mesh is not consistent. Fix it.
    Error,
    /// The mesh needs attention.
    Warning,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// Check name.
    pub check: &'static str,
    /// What is wrong and what to do.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}: [{}] {}", self.check, self.message)
    }
}

/// The doctor result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// All findings.
    pub findings: Vec<Finding>,
}

impl Report {
    /// The number of errors.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// The number of warnings.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn push(&mut self, severity: Severity, check: &'static str, message: String) {
        self.findings.push(Finding {
            severity,
            check,
            message,
        });
    }

    fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == s).count()
    }

    /// `true` if the report passes. With `strict`, warnings fail too.
    #[must_use]
    pub fn passes(&self, strict: bool) -> bool {
        self.errors() == 0 && (!strict || self.warnings() == 0)
    }
}

/// Reads the `name:` value from the frontmatter of a Claude Code agent file.
#[must_use]
pub fn frontmatter_name(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            return None;
        }
        if let Some(value) = line.strip_prefix("name:") {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

/// Runs all checks.
#[must_use]
pub fn run(store: &Store, now: u64) -> Report {
    let mut r = Report::default();
    let registry = match store.registry() {
        Ok(reg) => Some(reg),
        Err(e) => {
            let message = format!("{e}; fix .mesh/registry.toml");
            r.push(Severity::Error, "registry", message);
            None
        }
    };
    if let Some(reg) = &registry {
        check_agent_files(store.root(), reg, &mut r);
    }
    match store.load() {
        Ok(replay) => {
            check_replay(&replay, registry.as_ref(), &mut r);
            check_tasks(store, &replay, registry.as_ref(), now, &mut r);
        }
        Err(e) => r.push(Severity::Error, "ledger", e.to_string()),
    }
    let attrs = std::fs::read_to_string(store.root().join(".gitattributes")).unwrap_or_default();
    let union = attrs.lines().any(|l| {
        let mut parts = l.split_whitespace();
        parts.next() == Some(".mesh/ledger.jsonl") && parts.any(|p| p == "merge=union")
    });
    if !union {
        let message = format!("add `{UNION_RULE}` to .gitattributes");
        r.push(Severity::Error, "gitattributes", message);
    }
    r
}

/// Ledger lines, merges, and WIP limits.
fn check_replay(replay: &Replay, registry: Option<&Registry>, r: &mut Report) {
    for issue in &replay.issues {
        let what = match issue {
            ReplayIssue::Corrupt { line, error, .. } => {
                format!("line {line} is not a valid event ({error})")
            }
            ReplayIssue::Conflict {
                line, seq, error, ..
            } => format!("line {line} (seq {seq}) conflicts with earlier events ({error})"),
        };
        let message = format!(
            "{what}; fix the task state with mesh commands, then run `mesh ack {} --note \"<what you did>\"`",
            issue.fingerprint()
        );
        r.push(Severity::Error, "ledger", message);
    }
    if let Some(first) = replay.merged_lines.first() {
        let message = format!(
            "{} lines (first: line {first}) have sequence numbers that repeat or go down; a git merge joined two ledgers; check `mesh log` and `mesh status`",
            replay.merged_lines.len()
        );
        r.push(Severity::Warning, "merge", message);
    }
    for card in registry.map_or(&[][..], Registry::cards) {
        let active = replay.ledger.active_count(&card.id);
        if active > card.max_active as usize {
            let message = format!(
                "agent `{}` holds {active} working tasks; the limit is {}; fail or cancel the extra tasks",
                card.id, card.max_active
            );
            r.push(Severity::Warning, "wip", message);
        }
    }
}

/// Task skills, blocked tasks, overdue leases, and the board.
fn check_tasks(
    store: &Store,
    replay: &Replay,
    registry: Option<&Registry>,
    now: u64,
    r: &mut Report,
) {
    let ledger = &replay.ledger;
    for t in ledger.tasks() {
        let id = &t.spec.id;
        if !t.life.invariant_holds() {
            let message = format!("task `{id}` breaks the lifecycle invariants");
            r.push(Severity::Error, "invariants", message);
        }
        if registry.is_some_and(|reg| !reg.serves(&t.spec.skill)) && !t.life.state().is_terminal() {
            let message = format!(
                "task `{id}` needs skill `{}`; no agent has it",
                t.spec.skill
            );
            r.push(Severity::Error, "skills", message);
        }
        if let Readiness::Blocked(deps) = ledger.readiness(id) {
            let deps: Vec<String> = deps.iter().map(ToString::to_string).collect();
            let message = format!(
                "task `{id}` is blocked by {}; retry them or cancel this task",
                deps.join(", ")
            );
            r.push(Severity::Warning, "blocked", message);
        }
    }
    for t in ledger.overdue(now) {
        if let Some(l) = t.life.lease() {
            let message = format!(
                "the lease of `{}` (token {}) expired at {}; run `mesh reap`",
                t.spec.id,
                l.token,
                fmt_utc(l.expires_at)
            );
            r.push(Severity::Warning, "leases", message);
        }
    }
    if !store.board_is_current().unwrap_or(false) {
        let message = "BOARD.md does not match the ledger; run `mesh board`".to_string();
        r.push(Severity::Error, "board", message);
    }
}

fn check_agent_files(root: &Path, registry: &Registry, r: &mut Report) {
    for card in registry.cards() {
        let rel = format!(".claude/agents/{}.md", card.id);
        let message = match std::fs::read_to_string(root.join(&rel)) {
            Err(_) => format!("{rel} is missing; add a subagent file for the role"),
            Ok(text) => match frontmatter_name(&text) {
                Some(name) if name == card.id.as_str() => continue,
                Some(name) => format!("{rel} has name `{name}`; use `{}`", card.id),
                None => format!(
                    "{rel} has no `name` in its frontmatter; add `name: {}`",
                    card.id
                ),
            },
        };
        r.push(Severity::Error, "agent-files", message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TaskSpec;
    use crate::ids::{AgentId, Skill, TaskId};
    use crate::ledger::Command;
    use crate::registry::{Registry, DEFAULT_REGISTRY};
    use crate::store::MESH_DIR;

    fn healthy() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let root = tmp.path();
        Store::init(&root.join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
        let agents = root.join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap_or_else(|e| panic!("{e}"));
        let r = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
        for c in r.cards() {
            let text = format!("---\nname: {}\ndescription: x\n---\nBody\n", c.id);
            std::fs::write(agents.join(format!("{}.md", c.id)), text)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(
            root.join(".gitattributes"),
            ".mesh/ledger.jsonl merge=union\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let store = Store::open(root.join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
        (tmp, store)
    }

    fn checks(r: &Report) -> Vec<&'static str> {
        r.findings.iter().map(|f| f.check).collect()
    }

    #[test]
    fn frontmatter_name_reads_the_name() {
        assert_eq!(
            frontmatter_name("---\nname: builder\n---\n").as_deref(),
            Some("builder")
        );
        assert_eq!(
            frontmatter_name("---\ndescription: x\nname:  \"lore-master\" \n---\n").as_deref(),
            Some("lore-master")
        );
        assert_eq!(frontmatter_name("name: builder\n"), None);
        assert_eq!(
            frontmatter_name("---\ndescription: x\n---\nname: no\n"),
            None
        );
    }

    #[test]
    fn healthy_mesh_passes_strict() {
        let (_tmp, store) = healthy();
        let r = run(&store, 1);
        assert!(r.passes(true), "{:?}", r.findings);
    }

    #[test]
    fn missing_or_wrong_agent_file_is_an_error() {
        let (tmp, store) = healthy();
        std::fs::remove_file(tmp.path().join(".claude/agents/codex.md"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            tmp.path().join(".claude/agents/builder.md"),
            "---\nname: build\n---\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let r = run(&store, 1);
        assert_eq!(r.errors(), 2);
        assert!(checks(&r).iter().all(|c| *c == "agent-files"));
        assert!(!r.passes(false));
    }

    #[test]
    fn stale_board_corrupt_ledger_and_attributes() {
        let (tmp, store) = healthy();
        std::fs::write(store.dir().join("BOARD.md"), "x").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(store.dir().join("ledger.jsonl"), "garbage\n")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::remove_file(tmp.path().join(".gitattributes")).unwrap_or_else(|e| panic!("{e}"));
        let r = run(&store, 1);
        let c = checks(&r);
        assert!(c.contains(&"ledger"));
        assert!(c.contains(&"board"));
        assert!(c.contains(&"gitattributes"));
        assert_eq!(r.passes(false), r.errors() == 0);
    }

    #[test]
    fn overdue_leases_and_blocked_tasks_are_warnings() {
        let (_tmp, store) = healthy();
        let tid = |s: &str| TaskId::new(s).unwrap_or_else(|e| panic!("{e}"));
        let mut b = TaskSpec::new(
            tid("b"),
            "B",
            Skill::new("review").unwrap_or_else(|e| panic!("{e}")),
        );
        b.deps = vec![tid("a")];
        let a = TaskSpec::new(
            tid("a"),
            "A",
            Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
        );
        let c = TaskSpec::new(
            tid("c"),
            "C",
            Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
        );
        store
            .execute(10, Command::Plan(vec![a, b, c]))
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(
                10,
                Command::Cancel {
                    task: tid("a"),
                    reason: "x".into(),
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(
                10,
                Command::Claim {
                    task: Some(tid("c")),
                    agent: AgentId::new("builder").unwrap_or_else(|e| panic!("{e}")),
                    worker: None,
                    ttl: 5,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let r = run(&store, 100);
        assert_eq!(r.errors(), 0, "{:?}", r.findings);
        assert_eq!(r.warnings(), 2);
        assert!(r.passes(false));
        assert!(!r.passes(true));
        assert!(r
            .findings
            .iter()
            .any(|f| f.to_string().starts_with("warning: [leases]")));
    }

    #[test]
    fn unserved_skill_and_bad_registry_are_errors() {
        let (_tmp, store) = healthy();
        let tid = TaskId::new("a").unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(
                1,
                Command::Add(TaskSpec::new(
                    tid,
                    "A",
                    Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
                )),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let without_build =
            DEFAULT_REGISTRY.replace("skills = [\"build\", \"fix\"]", "skills = [\"fix\"]");
        std::fs::write(store.dir().join("registry.toml"), without_build)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(checks(&run(&store, 1)).contains(&"skills"));
        std::fs::write(store.dir().join("registry.toml"), "bad [")
            .unwrap_or_else(|e| panic!("{e}"));
        let r = run(&store, 1);
        assert!(checks(&r).contains(&"registry"));
        assert!(!r.passes(false));
    }

    #[test]
    fn corrupt_ledger_is_an_error() {
        let (_tmp, store) = healthy();
        std::fs::write(store.dir().join("ledger.jsonl"), "garbage\n")
            .unwrap_or_else(|e| panic!("{e}"));
        let r = run(&store, 1);
        let ledger = r.findings.iter().find(|f| f.check == "ledger");
        assert_eq!(ledger.map(|f| f.severity), Some(Severity::Error));
        assert!(ledger.is_some_and(|f| f.message.contains("mesh ack")));
    }

    #[test]
    fn terminal_tasks_need_no_skill() {
        let (_tmp, store) = healthy();
        let tid = TaskId::new("a").unwrap_or_else(|e| panic!("{e}"));
        let spec = TaskSpec::new(
            tid.clone(),
            "A",
            Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
        );
        store
            .execute(1, Command::Add(spec))
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(
                1,
                Command::Cancel {
                    task: tid,
                    reason: "done".into(),
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let without_build =
            DEFAULT_REGISTRY.replace("skills = [\"build\", \"fix\"]", "skills = [\"fix\"]");
        std::fs::write(store.dir().join("registry.toml"), without_build)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!checks(&run(&store, 1)).contains(&"skills"));
    }
}
