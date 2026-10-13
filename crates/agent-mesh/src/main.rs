//! The `mesh` command-line tool. See README.md.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_mesh::board::{self, Group};
use agent_mesh::doctor;
use agent_mesh::event::{Envelope, Event, TaskSpec, DEFAULT_MAX_FAILURES};
use agent_mesh::hook::{self, Decision, HookInput};
use agent_mesh::ids::{AgentId, Skill, TaskId, Token};
use agent_mesh::ledger::{Command, Ledger, Task, DEFAULT_TTL};
use agent_mesh::plan;
use agent_mesh::scope;
use agent_mesh::store::{Store, MESH_DIR};
use agent_mesh::time::fmt_utc;
use agent_mesh::MeshError;
use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;

/// Prints one line to stdout. A closed pipe (for example `mesh status | head`) is not an error.
macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Agent mesh: task ledger, leases, and scope checks for AI agents.
#[derive(Parser)]
#[command(name = "mesh", version)]
struct Cli {
    /// The mesh folder. Default: `MESH_DIR`, else `.mesh` in this folder or a parent folder
    /// up to the repository root. `init` uses `./.mesh`.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    /// Print JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make `.mesh/` with the default registry. Changes nothing if it exists.
    Init,
    /// Claude Code `PreToolUse` hook. Reads JSON on stdin. Prints a JSON decision. Exit 2 blocks.
    Hook,
    #[command(flatten)]
    Mesh(MeshCmd),
}

/// Commands that need an existing mesh.
#[derive(Subcommand)]
enum MeshCmd {
    /// List the agent cards.
    Agents,
    /// Add one task.
    Add {
        /// Task id.
        #[arg(long)]
        id: TaskId,
        /// One-line title.
        #[arg(long)]
        title: String,
        /// The skill that a worker needs.
        #[arg(long)]
        skill: Skill,
        /// A task that must be completed first. Repeat for more.
        #[arg(long = "dep")]
        deps: Vec<TaskId>,
        /// A path glob that the worker may change. Repeat for more.
        #[arg(long = "path")]
        paths: Vec<String>,
        /// Text for the worker brief.
        #[arg(long, default_value = "")]
        detail: String,
        /// Failure budget.
        #[arg(long, default_value_t = DEFAULT_MAX_FAILURES)]
        max_failures: u32,
    },
    /// Add all tasks of a plan file (TOML) in one atomic step.
    Plan {
        /// The plan file.
        file: PathBuf,
        /// Check the plan only.
        #[arg(long)]
        dry_run: bool,
    },
    /// List ready tasks.
    Ready {
        /// Only tasks that this role can do.
        #[arg(long)]
        agent: Option<AgentId>,
    },
    /// Claim a task. Prints the fencing token.
    Claim {
        /// The task.
        #[arg(required_unless_present = "next", conflicts_with = "next")]
        task: Option<TaskId>,
        /// Claim the first ready task for the role.
        #[arg(long)]
        next: bool,
        /// The role.
        #[arg(long)]
        agent: AgentId,
        /// A label for the worker instance.
        #[arg(long)]
        worker: Option<String>,
        /// Lease time in seconds. Use the expected task time plus 50%.
        #[arg(long, default_value_t = DEFAULT_TTL)]
        ttl: u64,
    },
    /// Extend a lease.
    Heartbeat {
        /// The task.
        task: TaskId,
        /// The lease token.
        #[arg(long)]
        token: Token,
        /// New lease time in seconds, from now.
        #[arg(long, default_value_t = DEFAULT_TTL)]
        ttl: u64,
    },
    /// Complete a task. Evidence is required.
    Complete {
        /// The task.
        task: TaskId,
        /// The lease token.
        #[arg(long)]
        token: Token,
        /// Proof: commands, results, commit.
        #[arg(long)]
        evidence: String,
    },
    /// Fail a task.
    Fail {
        /// The task.
        task: TaskId,
        /// The lease token.
        #[arg(long)]
        token: Token,
        /// Why it failed.
        #[arg(long)]
        reason: String,
    },
    /// Ask the coordinator a question. Releases the lease.
    Ask {
        /// The task.
        task: TaskId,
        /// The lease token.
        #[arg(long)]
        token: Token,
        /// The question.
        #[arg(long)]
        question: String,
    },
    /// Answer a question. The task becomes submitted again.
    Answer {
        /// The task.
        task: TaskId,
        /// The answer.
        #[arg(long)]
        text: String,
    },
    /// Retry a failed task inside its budget.
    Retry {
        /// The task.
        task: TaskId,
    },
    /// Cancel a task.
    Cancel {
        /// The task.
        task: TaskId,
        /// Why.
        #[arg(long)]
        reason: String,
    },
    /// Expire overdue leases.
    Reap,
    /// Mark a rejected or broken ledger line as checked. `doctor` prints the fingerprint.
    Ack {
        /// The line fingerprint (16 hex digits).
        fingerprint: String,
        /// What you did about the line.
        #[arg(long)]
        note: String,
    },
    /// Exit 0 only if the token holds the live lease. Changes nothing.
    Fence {
        /// The task.
        task: TaskId,
        /// The lease token.
        #[arg(long)]
        token: Token,
    },
    /// Show task counts and tasks by group.
    Status,
    /// Show one task.
    Show {
        /// The task.
        task: TaskId,
    },
    /// Show events. `!` marks an event that the ledger rejected.
    Log {
        /// Only events of this task.
        #[arg(long)]
        task: Option<TaskId>,
        /// Only the last N events.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Write `.mesh/BOARD.md`. With `--check`, fail if it is stale.
    Board {
        /// Check only.
        #[arg(long)]
        check: bool,
    },
    /// Check changed files against the write scope of a role.
    CheckScope {
        /// The role.
        #[arg(long)]
        agent: AgentId,
        /// Also check the paths of this task.
        #[arg(long)]
        task: Option<TaskId>,
        /// The commit that the work started from. Needed when you give no files.
        #[arg(long, allow_hyphen_values = true)]
        base: Option<String>,
        /// The git work tree to check. Default: the work tree of this folder.
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Files to check. Default: git changes since `--base`.
        files: Vec<String>,
    },
    /// Check the health of the mesh.
    Doctor {
        /// Fail on warnings too.
        #[arg(long)]
        strict: bool,
    },
}

fn main() -> ExitCode {
    // The hook must never fail on arguments or environment: exit 2 would block the tool.
    if std::env::args().nth(1).as_deref() == Some("hook") {
        return hook_cmd();
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// The current time. `MESH_NOW` (Unix seconds) sets it for tests.
fn now() -> Result<u64> {
    if let Ok(v) = std::env::var("MESH_NOW") {
        return v
            .parse()
            .map_err(|_| anyhow!("MESH_NOW must be Unix seconds, got `{v}`"));
    }
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

/// `--dir`, else a non-empty `MESH_DIR`.
fn mesh_dir(flag: Option<PathBuf>) -> Option<PathBuf> {
    flag.or_else(|| {
        std::env::var_os("MESH_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}

fn open(dir: Option<&Path>) -> Result<Store> {
    Ok(match dir {
        Some(d) => Store::open(d)?,
        None => Store::discover(&std::env::current_dir()?)?,
    })
}

fn run(cli: Cli) -> Result<ExitCode> {
    let json = cli.json;
    let dir = mesh_dir(cli.dir);
    match cli.cmd {
        Cmd::Init => init(dir.as_deref(), json),
        Cmd::Hook => Ok(hook_cmd()),
        Cmd::Mesh(cmd) => dispatch(&open(dir.as_deref())?, cmd, json),
    }
}

fn dispatch(store: &Store, cmd: MeshCmd, json: bool) -> Result<ExitCode> {
    let write = |cmd: Command| write(store, cmd, json);
    match cmd {
        MeshCmd::Agents => agents(store, json),
        MeshCmd::Add {
            id,
            title,
            skill,
            deps,
            paths,
            detail,
            max_failures,
        } => write(Command::Add(TaskSpec {
            id,
            title,
            skill,
            deps,
            paths,
            detail,
            max_failures,
        })),
        MeshCmd::Plan { file, dry_run } => plan_cmd(store, &file, dry_run, json),
        MeshCmd::Ready { agent } => ready(store, agent.as_ref(), json),
        MeshCmd::Claim {
            task,
            next: _,
            agent,
            worker,
            ttl,
        } => write(Command::Claim {
            task,
            agent,
            worker,
            ttl,
        }),
        MeshCmd::Heartbeat { task, token, ttl } => write(Command::Heartbeat { task, token, ttl }),
        MeshCmd::Complete {
            task,
            token,
            evidence,
        } => write(Command::Complete {
            task,
            token,
            evidence,
        }),
        MeshCmd::Fail {
            task,
            token,
            reason,
        } => write(Command::Fail {
            task,
            token,
            reason,
        }),
        MeshCmd::Ask {
            task,
            token,
            question,
        } => write(Command::Ask {
            task,
            token,
            question,
        }),
        MeshCmd::Answer { task, text } => write(Command::Answer { task, text }),
        MeshCmd::Retry { task } => write(Command::Retry { task }),
        MeshCmd::Cancel { task, reason } => write(Command::Cancel { task, reason }),
        MeshCmd::Reap => write(Command::Reap),
        MeshCmd::Ack { fingerprint, note } => write(Command::Ack { fingerprint, note }),
        MeshCmd::Fence { task, token } => fence(store, &task, token, json),
        MeshCmd::Status => status(store, json),
        MeshCmd::Show { task } => show(store, &task, json),
        MeshCmd::Log { task, limit } => log(store, task.as_ref(), limit, json),
        MeshCmd::Board { check } => board_cmd(store, check),
        MeshCmd::CheckScope {
            agent,
            task,
            base,
            repo,
            files,
        } => check_scope(
            store,
            &agent,
            task.as_ref(),
            base.as_deref(),
            repo.as_deref(),
            files,
            json,
        ),
        MeshCmd::Doctor { strict } => doctor_cmd(store, strict, json),
    }
}

fn print_json(v: &impl serde::Serialize) -> Result<()> {
    say!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn init(dir: Option<&Path>, json: bool) -> Result<ExitCode> {
    let dir = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir()?.join(MESH_DIR),
    };
    let report = Store::init(&dir)?;
    if json {
        print_json(&json!({ "dir": report.dir, "created": report.created }))?;
    } else if report.created.is_empty() {
        say!("mesh exists at {}; nothing to do", report.dir.display());
    } else {
        say!("created:");
        for p in &report.created {
            say!("  {}", p.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads hook JSON on stdin. Bad input cannot name an agent, so the tool runs.
fn hook_cmd() -> ExitCode {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        return ExitCode::SUCCESS;
    }
    let Ok(input) = serde_json::from_str::<HookInput>(&text) else {
        return ExitCode::SUCCESS;
    };
    let decision = hook::decide(&input);
    match &decision {
        Decision::Allow => ExitCode::SUCCESS,
        Decision::Deny(why) => {
            say!("{}", decision.to_hook_json());
            eprintln!("{why}");
            ExitCode::from(2)
        }
    }
}

fn write(store: &Store, cmd: Command, json: bool) -> Result<ExitCode> {
    let is_plan = matches!(cmd, Command::Plan(_));
    let is_reap = matches!(cmd, Command::Reap);
    let (events, ledger) = store.execute_report(now()?, cmd)?;
    if !events.is_empty() && !store.board_is_current().unwrap_or(false) {
        eprintln!("warning: the events are saved, but BOARD.md is stale; run `mesh board`");
    }
    if json {
        print_json(&events)?;
        return Ok(ExitCode::SUCCESS);
    }
    if is_plan {
        let added: Vec<&str> = events
            .iter()
            .filter_map(|e| match &e.event {
                Event::TaskAdded { task } => Some(task.id.as_str()),
                _ => None,
            })
            .collect();
        say!("added {} tasks: {}", added.len(), added.join(", "));
    } else {
        for env in &events {
            say!("{}", describe(&ledger, env));
        }
    }
    if is_reap && events.is_empty() {
        say!("no overdue leases");
    }
    Ok(ExitCode::SUCCESS)
}

fn budget(ledger: &Ledger, task: &TaskId) -> String {
    ledger.task(task).map_or_else(String::new, |t| {
        format!("failures {}/{}", t.life.failures(), t.life.max_failures())
    })
}

fn describe(ledger: &Ledger, env: &Envelope) -> String {
    match &env.event {
        Event::TaskAdded { task } => format!("added {} [{}] {}", task.id, task.skill, task.title),
        Event::Claimed {
            task,
            agent,
            worker,
            token,
            expires_at,
        } => format!(
            "claimed {task} for {agent} (worker {worker})\ntoken: {token}\nexpires: {}\ntrailers:\n  Mesh-Task: {task}\n  Mesh-Token: {token}",
            fmt_utc(*expires_at)
        ),
        Event::Heartbeat {
            task, expires_at, ..
        } => format!("lease of {task} extended to {}", fmt_utc(*expires_at)),
        Event::Completed { task, .. } => format!("completed {task}"),
        Event::Failed { task, .. } => format!("failed {task} ({})", budget(ledger, task)),
        Event::InputRequested { task, question, .. } => format!("{task} needs input: {question}"),
        Event::Answered { task, .. } => format!("answered {task}; it is submitted again"),
        Event::Retried { task } => {
            format!("{task} is submitted again ({})", budget(ledger, task))
        }
        Event::Canceled { task, .. } => format!("canceled {task}"),
        Event::Acknowledged { fingerprint, .. } => format!("acknowledged line {fingerprint}"),
        Event::LeaseExpired { task, token } => {
            format!("note: lease of {task} expired (token {token})")
        }
    }
}

fn agents(store: &Store, json: bool) -> Result<ExitCode> {
    let registry = store.registry()?;
    if json {
        print_json(&registry.cards())?;
        return Ok(ExitCode::SUCCESS);
    }
    for c in registry.cards() {
        let skills: Vec<&str> = c.skills.iter().map(Skill::as_str).collect();
        say!(
            "{:<12} skills: {:<16} max_active: {}  writes: {}",
            c.id,
            skills.join(", "),
            c.max_active,
            if c.writes.is_empty() {
                "(read only)".to_string()
            } else {
                c.writes.join(", ")
            }
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn plan_cmd(store: &Store, file: &Path, dry_run: bool, json: bool) -> Result<ExitCode> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("cannot read plan file {}", file.display()))?;
    let specs = plan::parse(&text)?;
    if !dry_run {
        return write(store, Command::Plan(specs), json);
    }
    let ledger = store.load()?.ledger;
    let registry = store.registry()?;
    let ordered = plan::order(&ledger, specs)?;
    for s in &ordered {
        s.validate()?;
        if !registry.serves(&s.skill) {
            return Err(MeshError::UnservedSkill {
                task: s.id.clone(),
                skill: s.skill.clone(),
            }
            .into());
        }
    }
    let ids: Vec<&str> = ordered.iter().map(|s| s.id.as_str()).collect();
    if json {
        print_json(&ids)?;
    } else {
        say!("plan ok: {} tasks in order: {}", ids.len(), ids.join(", "));
    }
    Ok(ExitCode::SUCCESS)
}

fn line(task: &Task) -> String {
    format!(
        "  {:<20} [{}] {}",
        task.spec.id, task.spec.skill, task.spec.title
    )
}

fn ready(store: &Store, agent: Option<&AgentId>, json: bool) -> Result<ExitCode> {
    let registry = store.registry()?;
    if let Some(a) = agent.filter(|a| registry.get(a).is_none()) {
        return Err(MeshError::UnknownAgent(a.clone()).into());
    }
    let ledger = store.load()?.ledger;
    let tasks = ledger.ready_for(&registry, agent);
    if json {
        let rows: Vec<_> = tasks
            .iter()
            .map(|t| json!({ "id": t.spec.id, "title": t.spec.title, "skill": t.spec.skill }))
            .collect();
        print_json(&rows)?;
    } else if tasks.is_empty() {
        say!("no ready tasks");
    } else {
        for t in tasks {
            say!("{}", line(t));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn fence(store: &Store, task: &TaskId, token: Token, json: bool) -> Result<ExitCode> {
    let ledger = store.load()?.ledger;
    let lease = ledger.check_fence(task, token, now()?)?;
    if json {
        print_json(lease)?;
    } else {
        say!(
            "token {token} holds {task} until {}",
            fmt_utc(lease.expires_at)
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn status(store: &Store, json: bool) -> Result<ExitCode> {
    let ledger = store.load()?.ledger;
    let now = now()?;
    let counts = board::counts(&ledger);
    if json {
        let counts: serde_json::Map<String, serde_json::Value> = counts
            .iter()
            .map(|(g, n)| (g.title().to_string(), json!(n)))
            .collect();
        let tasks: Vec<_> = ledger
            .tasks()
            .map(|t| {
                json!({
                    "id": t.spec.id,
                    "title": t.spec.title,
                    "skill": t.spec.skill,
                    "state": t.life.state(),
                    "group": board::group_of(&ledger, t),
                    "lease": t.life.lease(),
                    "overdue": t.life.lease().is_some_and(|l| l.expires_at <= now),
                })
            })
            .collect();
        print_json(&json!({ "counts": counts, "tasks": tasks }))?;
        return Ok(ExitCode::SUCCESS);
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(g, n)| format!("{} {n}", g.title()))
        .collect();
    say!("{} tasks: {}", ledger.len(), summary.join(", "));
    for group in Group::ALL {
        let tasks: Vec<&Task> = ledger
            .tasks()
            .filter(|t| board::group_of(&ledger, t) == group)
            .collect();
        if tasks.is_empty() {
            continue;
        }
        say!("{}:", group.title());
        for t in tasks {
            let overdue = t.life.lease().is_some_and(|l| l.expires_at <= now);
            let info = board::info(&ledger, t);
            say!(
                "{}{}{}",
                line(t),
                if info.is_empty() {
                    String::new()
                } else {
                    format!(" — {info}")
                },
                if overdue {
                    " [overdue: run `mesh reap`]"
                } else {
                    ""
                }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn show(store: &Store, id: &TaskId, json: bool) -> Result<ExitCode> {
    let ledger = store.load()?.ledger;
    let task = ledger
        .task(id)
        .ok_or_else(|| MeshError::UnknownTask(id.clone()))?;
    if json {
        print_json(&json!({
            "task": task,
            "readiness": ledger.readiness(id),
            "group": board::group_of(&ledger, task),
        }))?;
        return Ok(ExitCode::SUCCESS);
    }
    let list = |v: &[String]| {
        if v.is_empty() {
            "-".to_string()
        } else {
            v.join(", ")
        }
    };
    let deps: Vec<String> = task.spec.deps.iter().map(ToString::to_string).collect();
    say!("task {}: {}", task.spec.id, task.spec.title);
    say!(
        "skill: {}  state: {}  group: {}",
        task.spec.skill,
        task.life.state(),
        board::group_of(&ledger, task).title()
    );
    say!("deps: {}  paths: {}", list(&deps), list(&task.spec.paths));
    say!(
        "attempts: {}  failures: {}/{}",
        task.life.attempts(),
        task.life.failures(),
        task.life.max_failures()
    );
    match task.life.lease() {
        Some(l) => say!(
            "lease: {} ({}), token {}, until {}",
            l.agent,
            l.worker,
            l.token,
            fmt_utc(l.expires_at)
        ),
        None => say!("lease: -"),
    }
    if !task.spec.detail.is_empty() {
        say!("detail:\n{}", task.spec.detail);
    }
    if !task.notes.is_empty() {
        say!("notes:");
        for n in &task.notes {
            let kind = serde_json::to_value(n.kind)?;
            say!(
                "  #{} {} {} {}: {}",
                n.seq,
                fmt_utc(n.ts),
                n.actor,
                kind.as_str().unwrap_or_default(),
                n.text
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// One `log` row. `rejected` marks an event that the ledger did not apply.
#[derive(serde::Serialize)]
struct LogRow<'a> {
    #[serde(flatten)]
    env: &'a Envelope,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    rejected: bool,
}

fn log(store: &Store, task: Option<&TaskId>, limit: Option<usize>, json: bool) -> Result<ExitCode> {
    let replay = store.load()?;
    let rows: Vec<LogRow<'_>> = replay
        .envelopes
        .iter()
        .enumerate()
        .filter(|(_, e)| task.is_none_or(|t| e.event.task() == Some(t)))
        .map(|(i, env)| LogRow {
            env,
            rejected: replay.rejected.contains(&i),
        })
        .collect();
    let skip = limit.map_or(0, |n| rows.len().saturating_sub(n));
    let rows = &rows[skip..];
    if json {
        print_json(&rows)?;
        return Ok(ExitCode::SUCCESS);
    }
    for r in rows {
        say!(
            "{}#{} {} {:<12} {:<15} {}",
            if r.rejected { "!" } else { "" },
            r.env.seq,
            fmt_utc(r.env.ts),
            r.env.actor,
            r.env.event.name(),
            r.env.event.task().map_or("-", TaskId::as_str)
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn board_cmd(store: &Store, check: bool) -> Result<ExitCode> {
    if !check {
        store.write_board()?;
        say!("wrote {}", store.dir().join("BOARD.md").display());
        return Ok(ExitCode::SUCCESS);
    }
    if store.board_is_current()? {
        say!("board is current");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("error: the board is stale; run `mesh board`");
        Ok(ExitCode::FAILURE)
    }
}

fn check_scope(
    store: &Store,
    agent: &AgentId,
    task: Option<&TaskId>,
    base: Option<&str>,
    repo: Option<&Path>,
    files: Vec<String>,
    json: bool,
) -> Result<ExitCode> {
    let registry = store.registry()?;
    let paths = match task {
        Some(id) => {
            let ledger = store.load()?.ledger;
            let t = ledger
                .task(id)
                .ok_or_else(|| MeshError::UnknownTask(id.clone()))?;
            t.spec.paths.clone()
        }
        None => Vec::new(),
    };
    // A worker in a git worktree checks its own work tree, not the main checkout.
    let start = match repo {
        Some(r) => r.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let root = scope::git_root(&start).unwrap_or_else(|_| store.root().to_path_buf());
    let files = match (files.is_empty(), base) {
        (false, _) => files,
        (true, Some(base)) => scope::git_changed(&root, base)?,
        (true, None) => {
            return Err(anyhow!(
                "give --base <commit the work started from>, or the files to check"
            ))
        }
    };
    let violations = scope::check(&registry, agent, &paths, &files, &root)?;
    if json {
        print_json(&json!({ "agent": agent, "files": files.len(), "violations": violations }))?;
    } else if violations.is_empty() {
        say!(
            "ok: {} in scope for {agent}",
            plural(files.len(), "file", "files")
        );
    } else {
        say!(
            "{} outside the scope of {agent}:",
            plural(violations.len(), "file", "files")
        );
        for v in &violations {
            say!("  {}: {}", v.path, v.reason);
        }
    }
    Ok(if violations.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn doctor_cmd(store: &Store, strict: bool, json: bool) -> Result<ExitCode> {
    let report = doctor::run(store, now()?);
    if json {
        print_json(&report)?;
    } else {
        for f in &report.findings {
            say!("{f}");
        }
        say!(
            "doctor: {}, {}",
            plural(report.errors(), "error", "errors"),
            plural(report.warnings(), "warning", "warnings")
        );
    }
    Ok(if report.passes(strict) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
