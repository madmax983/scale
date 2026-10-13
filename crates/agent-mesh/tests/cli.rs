//! End-to-end tests of the `mesh` binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use agent_mesh::registry::{Registry, DEFAULT_REGISTRY};

struct Repo {
    tmp: tempfile::TempDir,
}

impl Repo {
    /// A temp repository with `mesh init` done.
    fn new() -> Self {
        let repo = Self::empty();
        let out = repo.mesh(1, &["init"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        repo
    }

    fn empty() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap_or_else(|e| panic!("{e}")),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn cmd(&self, now: u64, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_mesh"));
        c.current_dir(self.root())
            .env("MESH_NOW", now.to_string())
            .env_remove("MESH_DIR")
            .args(args);
        c
    }

    fn mesh(&self, now: u64, args: &[&str]) -> Output {
        self.cmd(now, args)
            .output()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn ok(&self, now: u64, args: &[&str]) -> String {
        let out = self.mesh(now, args);
        assert!(
            out.status.success(),
            "mesh {args:?} failed:\n{}",
            text(&out.stderr)
        );
        text(&out.stdout)
    }

    fn fail(&self, now: u64, args: &[&str]) -> String {
        let out = self.mesh(now, args);
        assert_eq!(out.status.code(), Some(1), "mesh {args:?} must fail");
        text(&out.stderr)
    }

    fn json(&self, now: u64, args: &[&str]) -> serde_json::Value {
        let mut all = args.to_vec();
        all.push("--json");
        serde_json::from_str(&self.ok(now, &all)).unwrap_or_else(|e| panic!("{e}"))
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.root().join(rel);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(&p, body).unwrap_or_else(|e| panic!("{e}"));
        p
    }

    fn agent_files(&self) {
        let r = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
        for c in r.cards() {
            self.write(
                &format!(".claude/agents/{}.md", c.id),
                &format!("---\nname: {}\ndescription: d\n---\n", c.id),
            );
        }
        self.write(".gitattributes", ".mesh/ledger.jsonl merge=union\n");
    }

    fn git(&self, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(self.root())
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(st.status.success(), "git {args:?}: {}", text(&st.stderr));
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn token_of(claim_out: &str) -> String {
    claim_out
        .lines()
        .find_map(|l| l.trim().strip_prefix("token: "))
        .unwrap_or_else(|| panic!("no token in:\n{claim_out}"))
        .to_string()
}

#[test]
fn init_creates_the_mesh_once() {
    let repo = Repo::empty();
    let first = repo.ok(1, &["init"]);
    assert!(first.contains("registry.toml"));
    for f in ["registry.toml", "ledger.jsonl", "BOARD.md", ".gitignore"] {
        assert!(repo.root().join(".mesh").join(f).exists(), "{f}");
    }
    let second = repo.ok(1, &["init"]);
    assert!(second.contains("exists"), "{second}");
}

#[test]
fn no_mesh_gives_a_clear_error() {
    let repo = Repo::empty();
    let err = repo.fail(1, &["status"]);
    assert!(err.contains("mesh init"), "{err}");
}

#[test]
fn agents_lists_the_roles() {
    let repo = Repo::new();
    let out = repo.ok(1, &["agents"]);
    for id in [
        "designer",
        "lore-master",
        "architect",
        "builder",
        "integrator",
        "codex",
        "reviewer",
    ] {
        assert!(out.contains(id), "{id}");
    }
    let v = repo.json(1, &["agents"]);
    assert_eq!(v.as_array().map(Vec::len), Some(7));
}

#[test]
fn full_task_flow() {
    let repo = Repo::new();
    repo.ok(
        10,
        &[
            "add", "--id", "a", "--title", "Build A", "--skill", "build", "--path", "src/**",
        ],
    );
    repo.ok(
        10,
        &[
            "add", "--id", "b", "--title", "Review A", "--skill", "review", "--dep", "a",
        ],
    );
    let err = repo.fail(
        10,
        &["add", "--id", "a", "--title", "Again", "--skill", "build"],
    );
    assert!(err.contains("already exists"), "{err}");

    let ready = repo.ok(11, &["ready"]);
    assert!(
        ready.contains('a') && !ready.contains("Review A"),
        "{ready}"
    );

    let claim = repo.ok(
        12,
        &["claim", "a", "--agent", "builder", "--worker", "sub-1"],
    );
    let token = token_of(&claim);
    assert!(claim.contains("Mesh-Task: a"), "{claim}");
    assert!(claim.contains(&format!("Mesh-Token: {token}")), "{claim}");

    let err = repo.fail(13, &["claim", "a", "--agent", "builder"]);
    assert!(err.contains("cannot claim"), "{err}");
    let err = repo.fail(13, &["complete", "a", "--token", "99", "--evidence", "x"]);
    assert!(err.contains("stale token"), "{err}");

    repo.ok(14, &["heartbeat", "a", "--token", &token, "--ttl", "600"]);
    repo.ok(
        15,
        &[
            "complete",
            "a",
            "--token",
            &token,
            "--evidence",
            "cargo test: 12 passed",
        ],
    );

    let ready = repo.ok(16, &["ready", "--agent", "reviewer"]);
    assert!(ready.contains("Review A"), "{ready}");
    let status = repo.ok(16, &["status"]);
    assert!(status.contains("completed"), "{status}");
    let show = repo.ok(16, &["show", "a"]);
    assert!(show.contains("cargo test: 12 passed"), "{show}");
    let log = repo.ok(16, &["log"]);
    assert_eq!(
        log.lines().filter(|l| l.starts_with('#')).count(),
        5,
        "{log}"
    );
    let log_a = repo.ok(16, &["log", "--task", "a", "--limit", "2"]);
    assert_eq!(
        log_a.lines().filter(|l| l.starts_with('#')).count(),
        2,
        "{log_a}"
    );
}

#[test]
fn json_outputs() {
    let repo = Repo::new();
    let added = repo.json(
        10,
        &["add", "--id", "a", "--title", "A", "--skill", "build"],
    );
    assert_eq!(added[0]["event"]["type"], "task_added");
    let claimed = repo.json(11, &["claim", "--next", "--agent", "builder"]);
    assert_eq!(claimed[0]["event"]["token"], 1);
    let status = repo.json(12, &["status"]);
    assert_eq!(status["counts"]["working"], 1);
    assert_eq!(status["tasks"][0]["id"], "a");
    let show = repo.json(12, &["show", "a"]);
    assert_eq!(show["task"]["life"]["state"], "working");
    let ready = repo.json(12, &["ready"]);
    assert_eq!(ready.as_array().map(Vec::len), Some(0));
    let log = repo.json(12, &["log"]);
    assert_eq!(log.as_array().map(Vec::len), Some(2));
}

#[test]
fn leases_expire_and_fence_checks_the_token() {
    let repo = Repo::new();
    repo.ok(
        100,
        &[
            "add",
            "--id",
            "a",
            "--title",
            "A",
            "--skill",
            "build",
            "--max-failures",
            "2",
        ],
    );
    let token = token_of(&repo.ok(100, &["claim", "a", "--agent", "builder", "--ttl", "10"]));
    repo.ok(105, &["fence", "a", "--token", &token]);
    let err = repo.fail(105, &["fence", "a", "--token", "42"]);
    assert!(err.contains("stale token"), "{err}");
    let status = repo.ok(200, &["status"]);
    assert!(status.contains("overdue"), "{status}");
    let err = repo.fail(200, &["fence", "a", "--token", &token]);
    assert!(err.contains("expired"), "{err}");
    let reap = repo.ok(200, &["reap"]);
    assert!(reap.contains("expired"), "{reap}");
    assert!(repo.ok(201, &["ready"]).contains("A"));
    // The second expiry uses the budget: the task fails.
    repo.ok(300, &["claim", "a", "--agent", "builder", "--ttl", "10"]);
    let out = repo.ok(
        400,
        &["add", "--id", "b", "--title", "B", "--skill", "build"],
    );
    assert!(out.contains("lease of a expired"), "{out}");
    assert!(repo.ok(401, &["show", "a"]).contains("failed"));
}

#[test]
fn ask_answer_fail_retry_cancel() {
    let repo = Repo::new();
    repo.ok(1, &["add", "--id", "a", "--title", "A", "--skill", "build"]);
    let t = token_of(&repo.ok(2, &["claim", "a", "--agent", "builder"]));
    repo.ok(
        3,
        &["ask", "a", "--token", &t, "--question", "Which module?"],
    );
    assert!(repo.ok(3, &["status"]).contains("Which module?"));
    repo.ok(4, &["answer", "a", "--text", "layer1"]);
    let t = token_of(&repo.ok(5, &["claim", "a", "--agent", "builder"]));
    let out = repo.ok(6, &["fail", "a", "--token", &t, "--reason", "tests fail"]);
    assert!(out.contains("1/3"), "{out}");
    repo.ok(7, &["retry", "a"]);
    repo.ok(8, &["cancel", "a", "--reason", "obsolete"]);
    let err = repo.fail(9, &["retry", "a"]);
    assert!(err.contains("canceled"), "{err}");
}

#[test]
fn plan_files_are_checked_and_atomic() {
    let repo = Repo::new();
    let cyclic = repo.write(
        "cyclic.toml",
        "[[task]]\nid = \"a\"\ntitle = \"A\"\nskill = \"build\"\ndeps = [\"b\"]\n\n[[task]]\nid = \"b\"\ntitle = \"B\"\nskill = \"build\"\ndeps = [\"a\"]\n",
    );
    let err = repo.fail(1, &["plan", cyclic.to_str().unwrap_or_default()]);
    assert!(err.contains("dependency cycle"), "{err}");

    let good = repo.write(
        "good.toml",
        "[[task]]\nid = \"r\"\ntitle = \"R\"\nskill = \"review\"\ndeps = [\"s\"]\n\n[[task]]\nid = \"s\"\ntitle = \"S\"\nskill = \"spec\"\n",
    );
    let dry = repo.ok(1, &["plan", good.to_str().unwrap_or_default(), "--dry-run"]);
    assert!(dry.contains("s, r"), "{dry}");
    assert!(repo.ok(1, &["status"]).contains("0 tasks"));
    let applied = repo.ok(1, &["plan", good.to_str().unwrap_or_default()]);
    assert!(applied.contains("added 2 tasks"), "{applied}");
    let err = repo.fail(1, &["plan", "missing.toml"]);
    assert!(err.contains("missing.toml"), "{err}");
}

#[test]
fn board_check_detects_drift() {
    let repo = Repo::new();
    repo.ok(1, &["add", "--id", "a", "--title", "A", "--skill", "build"]);
    repo.ok(1, &["board", "--check"]);
    repo.write(".mesh/BOARD.md", "hand edit\n");
    let err = repo.fail(1, &["board", "--check"]);
    assert!(err.contains("stale"), "{err}");
    repo.ok(1, &["board"]);
    repo.ok(1, &["board", "--check"]);
}

#[test]
fn check_scope_with_files() {
    let repo = Repo::new();
    repo.ok(
        1,
        &[
            "check-scope",
            "--agent",
            "builder",
            "src/lib.rs",
            "tests/a.rs",
        ],
    );
    let out = repo.mesh(
        1,
        &[
            "check-scope",
            "--agent",
            "builder",
            "src/lib.rs",
            "specs/1.md",
            "src/../specs/2.md",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("specs/1.md") && stdout.contains("src/../specs/2.md"),
        "{stdout}"
    );
    repo.ok(
        1,
        &[
            "add",
            "--id",
            "a",
            "--title",
            "A",
            "--skill",
            "build",
            "--path",
            "src/ui/**",
        ],
    );
    let out = repo.mesh(
        1,
        &[
            "check-scope",
            "--agent",
            "builder",
            "--task",
            "a",
            "src/ui/x.rs",
            "src/layer1/y.rs",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("src/layer1/y.rs"));
    let v = repo.json(1, &["check-scope", "--agent", "reviewer", ".mesh/BOARD.md"]);
    assert_eq!(v["violations"].as_array().map(Vec::len), Some(0));
}

#[test]
fn check_scope_reads_git_changes() {
    let repo = Repo::new();
    repo.git(&["init", "-q"]);
    repo.write("src/lib.rs", "// a\n");
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "init"]);
    repo.write("src/lib.rs", "// b\n");
    repo.write("specs/new.md", "x\n");
    let out = repo.mesh(1, &["check-scope", "--agent", "architect"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("src/lib.rs"), "{stdout}");
    assert!(!stdout.contains("specs/new.md"), "{stdout}");
    let err = repo.fail(
        1,
        &["check-scope", "--agent", "architect", "--base", "--evil"],
    );
    assert!(err.contains("git"), "{err}");
}

#[test]
fn hook_blocks_out_of_scope_edits() {
    let repo = Repo::new();
    let run = |input: serde_json::Value| {
        let mut child = repo
            .cmd(1, &["hook"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("{e}"));
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.to_string().as_bytes())
                .unwrap_or_else(|e| panic!("{e}"));
        }
        child.wait_with_output().unwrap_or_else(|e| panic!("{e}"))
    };
    let cwd = repo.root().display().to_string();
    let deny = run(serde_json::json!({
        "hook_event_name": "PreToolUse", "tool_name": "Write", "agent_type": "builder",
        "cwd": cwd, "tool_input": { "file_path": "specs/9.md" }
    }));
    assert_eq!(deny.status.code(), Some(2));
    assert!(text(&deny.stderr).contains("builder"));
    let allow = run(serde_json::json!({
        "tool_name": "Write", "agent_type": "builder", "cwd": cwd,
        "tool_input": { "file_path": "src/x.rs" }
    }));
    assert_eq!(allow.status.code(), Some(0));
    let main_session = run(serde_json::json!({
        "tool_name": "Write", "cwd": cwd, "tool_input": { "file_path": "specs/9.md" }
    }));
    assert_eq!(main_session.status.code(), Some(0));
    let garbage = repo
        .cmd(1, &["hook"])
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(garbage.status.code(), Some(0));
}

#[test]
fn doctor_reports_and_passes() {
    let repo = Repo::new();
    let out = repo.mesh(1, &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("agent-files"));
    repo.agent_files();
    let ok = repo.ok(1, &["doctor", "--strict"]);
    assert!(ok.contains("0 errors"), "{ok}");
    let v = repo.json(1, &["doctor"]);
    assert_eq!(v["findings"].as_array().map(Vec::len), Some(0));
}

#[test]
fn dir_flag_and_env_select_the_mesh() {
    let repo = Repo::new();
    let other = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let dir = repo.root().join(".mesh").display().to_string();
    let out = Command::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(other.path())
        .args(["--dir", &dir, "status"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = Command::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(other.path())
        .env("MESH_DIR", &dir)
        .args(["agents"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(out.status.success(), "{}", text(&out.stderr));
}

#[test]
fn bad_input_is_rejected() {
    let repo = Repo::new();
    let out = repo.mesh(
        1,
        &["add", "--id", "Bad Id", "--title", "x", "--skill", "build"],
    );
    assert_eq!(out.status.code(), Some(2), "clap rejects invalid ids");
    let out = repo.mesh(1, &["claim", "--agent", "builder"]);
    assert_eq!(out.status.code(), Some(2), "claim needs a task or --next");
    let out = repo
        .cmd(1, &["status"])
        .env("MESH_NOW", "soon")
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("MESH_NOW"));
}
