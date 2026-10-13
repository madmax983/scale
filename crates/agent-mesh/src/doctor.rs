//! Health checks for a mesh. CI runs `mesh doctor --strict`.

use std::fmt;
use std::path::Path;

use serde::Serialize;

use crate::board;
use crate::ledger::Readiness;
use crate::registry::Registry;
use crate::store::{Replay, ReplayIssue, Store, BOARD_FILE};
use crate::time::fmt_utc;

/// The `.gitattributes` line that makes git merge ledgers by union.
pub const UNION_RULE: &str = ".mesh/ledger.jsonl merge=union";

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
            r.push(
                Severity::Error,
                "registry",
                format!("{e}; fix .mesh/registry.toml"),
            );
            None
        }
    };
    if let Some(reg) = &registry {
        check_agent_files(store.root(), reg, &mut r);
    }
    match store.load() {
        Ok(replay) => check_ledger(store, &replay, registry.as_ref(), now, &mut r),
        Err(e) => r.push(Severity::Error, "ledger", e.to_string()),
    }
    let attrs = std::fs::read_to_string(store.root().join(".gitattributes")).unwrap_or_default();
    if !attrs
        .lines()
        .any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == UNION_RULE)
    {
        r.push(
            Severity::Warning,
            "gitattributes",
            format!("add `{UNION_RULE}` to .gitattributes"),
        );
    }
    r
}

fn check_ledger(
    store: &Store,
    replay: &Replay,
    registry: Option<&Registry>,
    now: u64,
    r: &mut Report,
) {
    for issue in &replay.issues {
        let message = match issue {
            ReplayIssue::Corrupt { line, error } => {
                format!("line {line} is not a valid event ({error}); repair or remove the line")
            }
            ReplayIssue::Conflict { line, seq, error } => format!(
                "line {line} (seq {seq}) conflicts with earlier events ({error}); a merge mixed two ledgers"
            ),
        };
        r.push(Severity::Error, "ledger", message);
    }
    let ledger = &replay.ledger;
    for t in ledger.tasks() {
        let id = &t.spec.id;
        if !t.life.invariant_holds() {
            r.push(
                Severity::Error,
                "invariants",
                format!("task `{id}` breaks the lifecycle invariants"),
            );
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
    let current = std::fs::read_to_string(store.dir().join(BOARD_FILE))
        .is_ok_and(|b| b == board::render(ledger));
    if !current {
        let message = "BOARD.md does not match the ledger; run `mesh board`".to_string();
        r.push(Severity::Error, "board", message);
    }
}

fn check_agent_files(root: &Path, registry: &Registry, r: &mut Report) {
    for card in registry.cards() {
        let rel = format!(".claude/agents/{}.md", card.id);
        match std::fs::read_to_string(root.join(&rel)) {
            Err(_) => r.push(
                Severity::Error,
                "agent-files",
                format!("{rel} is missing; add a subagent file for the role"),
            ),
            Ok(text) => match frontmatter_name(&text) {
                Some(name) if name == card.id.as_str() => {}
                found => r.push(
                    Severity::Error,
                    "agent-files",
                    format!(
                        "{rel} has name `{}`; use `{}`",
                        found.unwrap_or_default(),
                        card.id
                    ),
                ),
            },
        }
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
}
