//! Regression tests for the multi-angle review findings.
//! Each test names the finding: S = security, C = correctness, D = docs/UX.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command as Process, Output, Stdio};

use agent_mesh::event::{Envelope, Event, TaskSpec};
use agent_mesh::hook::{decide, Decision, HookInput};
use agent_mesh::ids::{AgentId, Skill, TaskId, Token};
use agent_mesh::ledger::{Command, Ctx, Ledger, Readiness};
use agent_mesh::registry::{Registry, DEFAULT_REGISTRY};
use agent_mesh::store::{fingerprint, replay, ReplayIssue, Store, MESH_DIR};
use agent_mesh::{scope, MeshError, State};

fn tid(s: &str) -> TaskId {
    TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
}

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap_or_else(|e| panic!("{e}"))
}

fn reg() -> Registry {
    Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"))
}

fn spec(id: &str, skill: &str, deps: &[&str]) -> TaskSpec {
    let mut s = TaskSpec::new(
        tid(id),
        "T",
        Skill::new(skill).unwrap_or_else(|e| panic!("{e}")),
    );
    s.deps = deps.iter().map(|d| tid(d)).collect();
    s
}

fn env(seq: u64, ts: u64, actor: &str, event: Event) -> Envelope {
    Envelope {
        seq,
        ts,
        actor: actor.into(),
        event,
    }
}

fn line(e: &Envelope) -> String {
    e.to_line().unwrap_or_else(|err| panic!("{err}"))
}

fn claimed(seq: u64, task: &str, worker: &str, token: Token) -> Envelope {
    env(
        seq,
        10,
        "builder",
        Event::Claimed {
            task: tid(task),
            agent: aid("builder"),
            worker: worker.into(),
            token,
            expires_at: 1_000,
        },
    )
}

fn completed(seq: u64, task: &str, token: Token, evidence: &str) -> Envelope {
    env(
        seq,
        20,
        "builder",
        Event::Completed {
            task: tid(task),
            token,
            evidence: evidence.into(),
        },
    )
}

fn added(seq: u64, task: &str) -> Envelope {
    env(
        seq,
        1,
        "coordinator",
        Event::TaskAdded {
            task: spec(task, "build", &[]),
        },
    )
}

/// C1 + concurrency 2: two branches claim the same task. Tokens have a nonce, so after a
/// union merge the losing branch cannot complete the winning branch's lease.
#[test]
fn c1_union_merge_does_not_let_a_rejected_claim_complete() {
    let ta = Token::new(1, 0xaaaa);
    let tb = Token::new(1, 0xbbbb);
    let a = [added(1, "x"), claimed(2, "x", "wa", ta)];
    let b = [claimed(2, "x", "wb", tb), completed(3, "x", tb, "B work")];
    let text: String = a.iter().chain(b.iter()).map(|e| line(e) + "\n").collect();
    let r = replay(&text);
    let x = r.ledger.task(&tid("x")).unwrap_or_else(|| panic!("x"));
    assert_eq!(
        x.life.state(),
        State::Working,
        "B's completion must be rejected"
    );
    assert_eq!(r.issues.len(), 2, "{:?}", r.issues);
    assert!(
        r.ledger.check_fence(&tid("x"), ta, 50).is_ok(),
        "A keeps its lease"
    );
    assert!(matches!(
        r.ledger.check_fence(&tid("x"), tb, 50),
        Err(MeshError::StaleToken { .. })
    ));
}

/// C1: an identical duplicate claim line (same content) does not contest the lease.
#[test]
fn c1_identical_duplicate_claim_does_not_contest() {
    let t = Token::new(1, 7);
    let c = claimed(2, "x", "wa", t);
    let text = [added(1, "x"), c.clone(), c, completed(3, "x", t, "ok")]
        .iter()
        .map(|e| line(e) + "\n")
        .collect::<String>();
    let r = replay(&text);
    assert_eq!(
        r.ledger.task(&tid("x")).map(|t| t.life.state()),
        Some(State::Completed)
    );
}

/// Concurrency 2: a rolled-back ledger gives the same token number again, but not the same token.
#[test]
fn tokens_stay_unique_after_a_rollback() {
    let r = reg();
    let ctx = Ctx {
        now: 1,
        registry: &r,
    };
    let mut base = Ledger::new();
    base.execute(Command::Add(spec("a", "build", &[])), &ctx)
        .unwrap_or_else(|e| panic!("{e}"));
    let claim = || Command::Claim {
        task: Some(tid("a")),
        agent: aid("builder"),
        worker: None,
        ttl: 10,
    };
    let token_of = |evs: Vec<Envelope>| match &evs[0].event {
        Event::Claimed { token, .. } => *token,
        e => panic!("{e:?}"),
    };
    let mut first = base.clone();
    let t1 = token_of(
        first
            .execute(claim(), &ctx)
            .unwrap_or_else(|e| panic!("{e}")),
    );
    let mut second = base;
    let t2 = token_of(
        second
            .execute(claim(), &ctx)
            .unwrap_or_else(|e| panic!("{e}")),
    );
    assert_eq!(t1.n, t2.n);
    assert_ne!(t1, t2, "the nonce makes each claim unique");
    assert!(matches!(
        second.check_fence(&tid("a"), t1, 2),
        Err(MeshError::StaleToken { .. })
    ));
    assert_eq!(t1.to_string().parse::<Token>().ok(), Some(t1));
    assert_eq!(
        "5".parse::<Token>().ok(),
        Some(Token::new(5, 0)),
        "a bare number has nonce 0"
    );
}

/// C2: the token counter never wraps or repeats at `u64::MAX`.
#[test]
fn c2_token_space_exhaustion_is_an_error() {
    let text = [
        added(1, "a"),
        added(2, "b"),
        claimed(3, "b", "w", Token::new(u64::MAX, 1)),
    ]
    .iter()
    .map(|e| line(e) + "\n")
    .collect::<String>();
    let mut r = replay(&text);
    assert_eq!(r.issues.len(), 1, "a claim with token u64::MAX is rejected");
    let ctx = Ctx {
        now: 50,
        registry: &reg(),
    };
    let claim = Command::Claim {
        task: Some(tid("a")),
        agent: aid("builder"),
        worker: None,
        ttl: 10,
    };
    assert!(r.ledger.execute(claim, &ctx).is_err(), "no token is left");
}

/// C8: blocking is transitive, and a blocked claim says "blocked".
#[test]
fn c8_blocking_is_transitive() {
    let r = reg();
    let mut l = Ledger::new();
    let ctx = Ctx {
        now: 1,
        registry: &r,
    };
    l.execute(
        Command::Plan(vec![
            spec("a", "build", &[]),
            spec("b", "build", &["a"]),
            spec("c", "build", &["b"]),
        ]),
        &ctx,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    l.execute(
        Command::Cancel {
            task: tid("a"),
            reason: "x".into(),
        },
        &ctx,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(l.readiness(&tid("c")), Readiness::Blocked(vec![tid("b")]));
    let claim = Command::Claim {
        task: Some(tid("b")),
        agent: aid("builder"),
        worker: None,
        ttl: 10,
    };
    assert!(matches!(
        l.execute(claim, &ctx),
        Err(MeshError::Blocked { .. })
    ));
}

/// S7: the actor must be a valid id (no control characters).
#[test]
fn s7_actor_must_be_a_valid_id() {
    let mut l = Ledger::new();
    let bad = env(
        1,
        1,
        "evil\u{1b}]0;x\u{7}",
        Event::TaskAdded {
            task: spec("a", "build", &[]),
        },
    );
    assert!(l.apply(&bad).is_err());
}

/// S7: patterns and text reject control and bidi characters.
#[test]
fn s7_patterns_and_text_reject_control_and_bidi_characters() {
    assert!(scope::validate_pattern("src/\u{1b}]0;x\u{7}/**").is_err());
    assert!(
        agent_mesh::event::validate_text("evidence", "ok \u{202e}gnp.exe", 100, true, false)
            .is_err()
    );
    assert!(MeshError::InvalidId("a\u{1b}b".into())
        .to_string()
        .contains("\\u{1b}"));
}

/// D11: a reviewed conflict can be acknowledged. Then doctor stops reporting it.
#[test]
fn d11_ack_hides_a_reviewed_conflict() {
    let dup = added(1, "x");
    let mut text: String = [dup.clone(), dup].iter().map(|e| line(e) + "\n").collect();
    let r = replay(&text);
    let fp = match &r.issues[..] {
        [ReplayIssue::Conflict { fingerprint, .. }] => fingerprint.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(fp, fingerprint(&line(&added(1, "x"))));
    let ack = env(
        2,
        5,
        "coordinator",
        Event::Acknowledged {
            fingerprint: fp,
            note: "duplicate add".into(),
        },
    );
    text.push_str(&(line(&ack) + "\n"));
    let r = replay(&text);
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.rejected.len(), 1, "the line stays marked as rejected");
}

fn store() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    Store::init(&tmp.path().join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
    let s = Store::open(tmp.path().join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
    (tmp, s)
}

/// C6/D9: a worker write after the lease expired says "expired" and keeps the expiry.
#[test]
fn c6_expired_worker_write_reports_expiry_and_keeps_the_reap() {
    let (_tmp, s) = store();
    s.execute(100, Command::Add(spec("a", "build", &[])))
        .unwrap_or_else(|e| panic!("{e}"));
    s.execute(
        100,
        Command::Claim {
            task: Some(tid("a")),
            agent: aid("builder"),
            worker: None,
            ttl: 10,
        },
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let token = s.load().ok().and_then(|r| {
        r.ledger
            .task(&tid("a"))
            .and_then(|t| t.life.lease().map(|l| l.token))
    });
    let token = token.unwrap_or_else(|| panic!("lease"));
    let err = s.execute(
        200,
        Command::Complete {
            task: tid("a"),
            token,
            evidence: "x".into(),
        },
    );
    assert!(matches!(err, Err(MeshError::LeaseExpired(_))), "{err:?}");
    let r = s.load().unwrap_or_else(|e| panic!("{e}"));
    assert!(r
        .envelopes
        .iter()
        .any(|e| matches!(e.event, Event::LeaseExpired { .. })));
}

/// C4: a CRLF checkout of the board is still current.
#[test]
fn c4_board_check_ignores_crlf() {
    let (_tmp, s) = store();
    s.execute(1, Command::Add(spec("a", "build", &[])))
        .unwrap_or_else(|e| panic!("{e}"));
    let p = s.dir().join("BOARD.md");
    let text = std::fs::read_to_string(&p)
        .unwrap_or_default()
        .replace('\n', "\r\n");
    std::fs::write(&p, text).unwrap_or_else(|e| panic!("{e}"));
    assert!(s.board_is_current().unwrap_or(false));
}

/// C5: a board write failure after the append is not a command failure.
#[test]
fn c5_board_write_failure_does_not_fail_the_command() {
    let (_tmp, s) = store();
    std::fs::create_dir_all(s.dir().join("BOARD.tmp")).unwrap_or_else(|e| panic!("{e}"));
    let events = s.execute(1, Command::Add(spec("a", "build", &[])));
    assert!(events.is_ok(), "{events:?}");
    assert_eq!(s.load().map_or(0, |r| r.ledger.len()), 1);
}

fn git(root: &Path, args: &[&str]) {
    let out = Process::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap_or(root)).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(p, body).unwrap_or_else(|e| panic!("{e}"));
}

/// S1/D1: committed changes since the base are checked. Renames show both paths.
#[test]
fn s1_git_changed_sees_commits_since_base_and_both_rename_paths() {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = tmp.path();
    git(root, &["init", "-q"]);
    write(root, "specs/a.md", "a\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["tag", "base"]);
    write(root, "specs/b.md", "b\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "work"]);
    std::fs::create_dir_all(root.join("src")).unwrap_or_else(|e| panic!("{e}"));
    git(root, &["mv", "specs/a.md", "src/a.md"]);
    let files = scope::git_changed(root, "base").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(files, ["specs/a.md", "specs/b.md", "src/a.md"]);
}

fn hook_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    Store::init(&tmp.path().join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
    write(
        tmp.path(),
        ".claude/agents/builder.md",
        "---\nname: builder\n---\n",
    );
    tmp
}

fn hook(tool: &str, agent: Option<&str>, input: serde_json::Value, cwd: &Path) -> Decision {
    decide(&HookInput {
        tool_name: tool.into(),
        tool_input: input,
        agent_type: agent.map(str::to_string),
        cwd: Some(cwd.display().to_string()),
    })
}

fn file(p: &str) -> serde_json::Value {
    serde_json::json!({ "file_path": p })
}

/// S3: roles may not edit mesh files by hand.
#[test]
fn s3_roles_cannot_edit_mesh_files() {
    let tmp = hook_repo();
    for p in [
        ".mesh/ledger.jsonl",
        ".mesh/BOARD.md",
        ".mesh/registry.toml",
        "src/.mesh/registry.toml",
    ] {
        assert!(
            matches!(
                hook("Write", Some("builder"), file(p), tmp.path()),
                Decision::Deny(_)
            ),
            "{p}"
        );
    }
}

/// S4: a broken registry blocks mesh roles only.
#[test]
fn s4_broken_registry_blocks_only_mesh_roles() {
    let tmp = hook_repo();
    std::fs::write(tmp.path().join(".mesh/registry.toml"), "broken [")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        hook(
            "Edit",
            Some("general-purpose"),
            file("src/x.rs"),
            tmp.path()
        ),
        Decision::Allow
    );
    assert!(matches!(
        hook("Edit", Some("builder"), file("src/x.rs"), tmp.path()),
        Decision::Deny(_)
    ));
}

/// S5: a planted nested registry does not change the rules.
#[test]
fn s5_nested_registry_does_not_reroot() {
    let tmp = hook_repo();
    write(tmp.path(), "src/.mesh/registry.toml", "version = 1\n[[agent]]\nid = \"designer\"\ndescription = \"d\"\nskills = [\"x\"]\nwrites = [\"**\"]\n");
    assert!(matches!(
        hook("Write", Some("designer"), file("src/x.rs"), tmp.path()),
        Decision::Deny(_)
    ));
}

/// S9: plugin-namespaced and upper-case agent types map to the role.
#[test]
fn s9_agent_type_is_normalized() {
    let tmp = hook_repo();
    for agent in ["mesh:builder", "Builder"] {
        assert!(
            matches!(
                hook("Write", Some(agent), file("specs/x.md"), tmp.path()),
                Decision::Deny(_)
            ),
            "{agent}"
        );
    }
}

/// S10: a symlinked folder cannot carry a role outside its scope.
#[cfg(unix)]
#[test]
fn s10_symlinks_are_resolved() {
    let tmp = hook_repo();
    std::fs::create_dir_all(tmp.path().join("specs")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir_all(tmp.path().join("src")).unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink("../specs", tmp.path().join("src/link"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        hook("Write", Some("builder"), file("src/link/x.md"), tmp.path()),
        Decision::Deny(_)
    ));
}

/// S11: every path key of the tool input is checked.
#[test]
fn s11_all_path_keys_are_checked() {
    let tmp = hook_repo();
    let input =
        serde_json::json!({ "file_path": "src/ok.ipynb", "notebook_path": "specs/x.ipynb" });
    assert!(matches!(
        hook("NotebookEdit", Some("builder"), input, tmp.path()),
        Decision::Deny(_)
    ));
}

/// S12: roles may not change Claude Code settings outside the repository.
#[test]
fn s12_roles_cannot_write_claude_settings_outside_the_repo() {
    let tmp = hook_repo();
    let other = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let settings = other
        .path()
        .join(".claude/settings.json")
        .display()
        .to_string();
    assert!(matches!(
        hook("Write", Some("builder"), file(&settings), tmp.path()),
        Decision::Deny(_)
    ));
    let notes = other.path().join("notes.txt").display().to_string();
    assert_eq!(
        hook("Write", Some("builder"), file(&notes), tmp.path()),
        Decision::Allow
    );
}

fn mesh(root: &Path, now: u64, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(root)
        .env("MESH_NOW", now.to_string())
        .env_remove("MESH_DIR")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{e}"))
}

fn text_of(b: &[u8]) -> String {
    text(b)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// S2: an empty `MESH_DIR` or odd arguments never make the hook block the main session.
#[test]
fn s2_hook_ignores_empty_mesh_dir_and_extra_arguments() {
    let tmp = hook_repo();
    let input = serde_json::json!({ "tool_name": "Edit", "cwd": tmp.path(), "tool_input": { "file_path": "README.md" } });
    for args in [vec!["hook"], vec!["hook", "--unknown"]] {
        let mut child = Process::new(env!("CARGO_BIN_EXE_mesh"))
            .current_dir(tmp.path())
            .env("MESH_DIR", "")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{e}"));
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.to_string().as_bytes())
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let out = child.wait_with_output().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out.status.code(), Some(0), "{args:?}");
    }
    let out = Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(tmp.path())
        .env("MESH_DIR", "")
        .args(["agents"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "empty MESH_DIR means unset: {}",
        text(&out.stderr)
    );
}

/// S1/D1: check-scope needs files or a base, and checks committed work.
#[test]
fn s1_check_scope_needs_a_base_and_checks_commits() {
    let tmp = hook_repo();
    let root = tmp.path();
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    let base = text(
        &Process::new("git")
            .current_dir(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap_or_else(|e| panic!("{e}"))
            .stdout,
    );
    write(root, "specs/x.md", "x\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "work"]);
    let out = mesh(root, 1, &["check-scope", "--agent", "builder"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("--base"),
        "{}",
        text(&out.stderr)
    );
    let out = mesh(
        root,
        1,
        &["check-scope", "--agent", "builder", "--base", base.trim()],
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("specs/x.md"));
    let other = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = root.display().to_string();
    let out = Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(other.path())
        .args([
            "--dir",
            &root.join(".mesh").display().to_string(),
            "check-scope",
            "--agent",
            "builder",
            "--base",
            base.trim(),
            "--repo",
            &repo,
        ])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        text(&out.stdout).contains("specs/x.md"),
        "--repo checks another work tree"
    );
}

/// D2: the default lease is long enough for a 1-2 hour task.
#[test]
fn d2_default_lease_is_three_hours() {
    let tmp = hook_repo();
    let root = tmp.path();
    assert!(mesh(
        root,
        0,
        &["add", "--id", "a", "--title", "A", "--skill", "build"]
    )
    .status
    .success());
    let out = text(&mesh(root, 0, &["claim", "a", "--agent", "builder"]).stdout);
    assert!(out.contains("expires: 1970-01-01T03:00:00Z"), "{out}");
}

/// C7: the expiry note does not print a state from after the whole write.
#[test]
fn c7_expiry_note_has_no_later_state() {
    let tmp = hook_repo();
    let root = tmp.path();
    assert!(mesh(
        root,
        0,
        &["add", "--id", "a", "--title", "A", "--skill", "build"]
    )
    .status
    .success());
    assert!(
        mesh(root, 0, &["claim", "a", "--agent", "builder", "--ttl", "5"])
            .status
            .success()
    );
    let out = text(&mesh(root, 10, &["claim", "a", "--agent", "builder"]).stdout);
    assert!(out.contains("lease of a expired (token 1-"), "{out}");
    assert!(!out.contains("state working"), "{out}");
}

/// C9: `ready --agent` with an unknown role is an error.
#[test]
fn c9_ready_rejects_unknown_agent() {
    let tmp = hook_repo();
    let out = mesh(tmp.path(), 1, &["ready", "--agent", "ghost"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("ghost"));
}

/// C10 + D11: `log` marks rejected events; `ack` clears the doctor error.
#[test]
fn c10_log_marks_rejected_events_and_ack_clears_doctor() {
    let tmp = hook_repo();
    let root = tmp.path();
    let dup = line(&added(1, "x")) + "\n";
    std::fs::write(root.join(".mesh/ledger.jsonl"), format!("{dup}{dup}"))
        .unwrap_or_else(|e| panic!("{e}"));
    let log = text(&mesh(root, 1, &["log"]).stdout);
    assert_eq!(
        log.lines().filter(|l| l.starts_with('!')).count(),
        1,
        "{log}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&mesh(root, 1, &["log", "--json"]).stdout).unwrap_or_default();
    assert_eq!(json[1]["rejected"], true);
    let doctor = text(&mesh(root, 1, &["doctor"]).stdout);
    let fp = fingerprint(dup.trim_end());
    assert!(
        doctor.contains(&fp) && doctor.contains("mesh ack"),
        "{doctor}"
    );
    assert!(mesh(root, 2, &["ack", &fp, "--note", "same task twice"])
        .status
        .success());
    let doctor = text(&mesh(root, 3, &["doctor"]).stdout);
    assert!(!doctor.contains("[ledger]"), "{doctor}");
}

/// D19: discovery stops at the git repository boundary.
#[test]
fn d19_discovery_stops_at_the_repo_boundary() {
    let tmp = hook_repo();
    let inner = tmp.path().join("other");
    std::fs::create_dir_all(&inner).unwrap_or_else(|e| panic!("{e}"));
    git(&inner, &["init", "-q"]);
    let out = mesh(&inner, 1, &["status"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("mesh init"));
}

/// D18: doctor counts use the right plural.
#[test]
fn d18_doctor_plural() {
    let tmp = hook_repo();
    let out = text(&mesh(tmp.path(), 1, &["doctor"]).stdout);
    assert!(
        out.contains("1 warning\n") || out.contains("1 warning,") || !out.contains("1 warnings"),
        "{out}"
    );
    assert!(
        !out.contains("1 errors") && !out.contains("1 warnings"),
        "{out}"
    );
}

/// Concurrency 3: doctor warns when a merge joined two ledgers (sequence numbers repeat).
#[test]
fn doctor_warns_on_merged_sequence_numbers() {
    let tmp = hook_repo();
    let root = tmp.path();
    let text: String = [added(1, "a"), added(1, "b")]
        .iter()
        .map(|e| line(e) + "\n")
        .collect();
    std::fs::write(root.join(".mesh/ledger.jsonl"), text).unwrap_or_else(|e| panic!("{e}"));
    let out = text_of(&mesh(root, 1, &["doctor"]).stdout);
    assert!(out.contains("[merge]"), "{out}");
}

/// Correctness 1: doctor warns when a role holds more working tasks than its limit.
#[test]
fn doctor_warns_on_wip_over_the_limit() {
    let tmp = hook_repo();
    let root = tmp.path();
    let lines: String = [
        env(
            1,
            1,
            "coordinator",
            Event::TaskAdded {
                task: spec("a", "spec", &[]),
            },
        ),
        env(
            2,
            1,
            "coordinator",
            Event::TaskAdded {
                task: spec("b", "spec", &[]),
            },
        ),
        env(
            3,
            2,
            "architect",
            Event::Claimed {
                task: tid("a"),
                agent: aid("architect"),
                worker: "w".into(),
                token: Token::new(1, 1),
                expires_at: 1_000,
            },
        ),
        env(
            4,
            2,
            "architect",
            Event::Claimed {
                task: tid("b"),
                agent: aid("architect"),
                worker: "w".into(),
                token: Token::new(2, 2),
                expires_at: 1_000,
            },
        ),
    ]
    .iter()
    .map(|e| line(e) + "\n")
    .collect();
    std::fs::write(root.join(".mesh/ledger.jsonl"), lines).unwrap_or_else(|e| panic!("{e}"));
    let out = text_of(&mesh(root, 5, &["doctor"]).stdout);
    assert!(out.contains("[wip]"), "{out}");
}

/// Concurrency 1: the hook prints a JSON decision, so the settings command can never block by accident.
#[test]
fn hook_deny_prints_a_json_decision() {
    let tmp = hook_repo();
    let input = serde_json::json!({ "tool_name": "Write", "agent_type": "builder", "cwd": tmp.path(), "tool_input": { "file_path": "specs/x.md" } });
    let mut child = Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(tmp.path())
        .args(["hook"])
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
    let out = child.wait_with_output().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
}

/// Out-of-scope note from the concurrency review: a closed pipe must not panic the CLI.
#[cfg(unix)]
#[test]
fn closed_stdout_does_not_panic() {
    let tmp = hook_repo();
    let mut child = Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(tmp.path())
        .args(["agents"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    drop(child.stdout.take());
    let status = child.wait().unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(
        status.code(),
        Some(101),
        "the CLI panicked on a closed pipe"
    );
}
