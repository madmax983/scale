//! The mesh folder: registry, ledger file, lock, and board.
//!
//! Every write takes an exclusive lock on `ledger.lock`, replays the ledger,
//! expires overdue leases, runs the command, appends the events, and
//! rewrites the board. Reads take a shared lock.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::board;
use crate::error::{MeshError, Result};
use crate::event::Envelope;
use crate::ledger::{Command, Ctx, Ledger};
use crate::registry::{Registry, DEFAULT_REGISTRY};

/// The name of the mesh folder.
pub const MESH_DIR: &str = ".mesh";
/// The registry file.
pub const REGISTRY_FILE: &str = "registry.toml";
/// The ledger file.
pub const LEDGER_FILE: &str = "ledger.jsonl";
/// The lock file. Git ignores it.
pub const LOCK_FILE: &str = "ledger.lock";
/// The board file.
pub const BOARD_FILE: &str = "BOARD.md";

/// A problem found during replay.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayIssue {
    /// The line is not a valid envelope.
    Corrupt {
        /// Line number (1-based).
        line: usize,
        /// The parse error.
        error: String,
    },
    /// The envelope is valid JSON but the ledger rejected it.
    Conflict {
        /// Line number (1-based).
        line: usize,
        /// The event sequence number.
        seq: u64,
        /// Why the ledger rejected it.
        error: String,
    },
}

/// The result of a replay.
#[derive(Debug, Clone)]
pub struct Replay {
    /// The rebuilt ledger.
    pub ledger: Ledger,
    /// All parsed envelopes, in file order. Conflicting envelopes are included.
    pub envelopes: Vec<Envelope>,
    /// Problems found.
    pub issues: Vec<ReplayIssue>,
}

/// Rebuilds the ledger from ledger text. Bad lines do not stop the replay.
#[must_use]
pub fn replay(text: &str) -> Replay {
    let mut ledger = Ledger::new();
    let mut envelopes = Vec::new();
    let mut issues = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        match Envelope::from_line(line) {
            Ok(env) => {
                if let Err(e) = ledger.apply(&env) {
                    ledger.observe(&env);
                    issues.push(ReplayIssue::Conflict {
                        line: line_no,
                        seq: env.seq,
                        error: e.to_string(),
                    });
                }
                envelopes.push(env);
            }
            Err(e) => issues.push(ReplayIssue::Corrupt {
                line: line_no,
                error: e.to_string(),
            }),
        }
    }
    Replay {
        ledger,
        envelopes,
        issues,
    }
}

fn read_or_empty(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(MeshError::io(path, e)),
    }
}

/// Writes a file only if it does not exist. Records new files in `created`.
fn write_new(path: PathBuf, content: &str, created: &mut Vec<PathBuf>) -> Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            f.write_all(content.as_bytes())
                .map_err(|e| MeshError::io(&path, e))?;
            created.push(path);
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(MeshError::io(&path, e)),
    }
}

/// Writes a temp file, then renames it over `path`.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| MeshError::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| MeshError::io(path, e))
}

/// What `init` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// The mesh folder.
    pub dir: PathBuf,
    /// Files made by this run.
    pub created: Vec<PathBuf>,
}

/// A mesh folder.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Uses the mesh folder at `dir`. It must contain `registry.toml`.
    ///
    /// # Errors
    /// [`MeshError::NotFound`] if `dir/registry.toml` does not exist.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        if dir.join(REGISTRY_FILE).is_file() {
            Ok(Self { dir })
        } else {
            Err(MeshError::NotFound)
        }
    }

    /// Finds `.mesh/registry.toml` in `start` or a parent folder.
    ///
    /// # Errors
    /// [`MeshError::NotFound`] if no folder has it.
    pub fn discover(start: &Path) -> Result<Self> {
        start
            .ancestors()
            .map(|a| a.join(MESH_DIR))
            .find(|d| d.join(REGISTRY_FILE).is_file())
            .map(|dir| Self { dir })
            .ok_or(MeshError::NotFound)
    }

    /// Makes the mesh folder with the default registry, an empty ledger, a board,
    /// and a `.gitignore` for the lock file. It does not change files that exist.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file write fails.
    pub fn init(dir: &Path) -> Result<InitReport> {
        fs::create_dir_all(dir).map_err(|e| MeshError::io(dir, e))?;
        let mut created = Vec::new();
        write_new(dir.join(REGISTRY_FILE), DEFAULT_REGISTRY, &mut created)?;
        write_new(dir.join(LEDGER_FILE), "", &mut created)?;
        write_new(dir.join(".gitignore"), "ledger.lock\n*.tmp\n", &mut created)?;
        let ledger = replay(&read_or_empty(&dir.join(LEDGER_FILE))?).ledger;
        write_new(dir.join(BOARD_FILE), &board::render(&ledger), &mut created)?;
        Ok(InitReport {
            dir: dir.to_path_buf(),
            created,
        })
    }

    /// Opens the lock file and takes the lock. The lock ends when the file closes.
    fn lock(&self, exclusive: bool) -> Result<File> {
        let path = self.dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| MeshError::io(&path, e))?;
        let locked = if exclusive {
            file.lock()
        } else {
            file.lock_shared()
        };
        locked.map_err(|e| MeshError::io(&path, e))?;
        Ok(file)
    }

    fn ledger_path(&self) -> PathBuf {
        self.dir.join(LEDGER_FILE)
    }

    /// The mesh folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The repository root: the parent of the mesh folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.dir.parent().unwrap_or(&self.dir)
    }

    /// Reads and checks the registry.
    ///
    /// # Errors
    /// [`MeshError::Io`] or a registry error.
    pub fn registry(&self) -> Result<Registry> {
        let path = self.dir.join(REGISTRY_FILE);
        let text = fs::read_to_string(&path).map_err(|e| MeshError::io(&path, e))?;
        Registry::parse(&text)
    }

    /// Replays the ledger under a shared lock.
    ///
    /// # Errors
    /// [`MeshError::Io`] if the files cannot be read.
    pub fn load(&self) -> Result<Replay> {
        // A read-only checkout cannot make the lock file. Read without the lock then.
        let _guard = self.lock(false).ok();
        Ok(replay(&read_or_empty(&self.ledger_path())?))
    }

    /// Runs one command as one atomic write. Overdue leases expire first.
    /// Returns all new events, lease expiries first.
    ///
    /// # Errors
    /// The command error, or [`MeshError::Io`]. On error nothing is written.
    pub fn execute(&self, now: u64, cmd: Command) -> Result<Vec<Envelope>> {
        let registry = self.registry()?;
        let _guard = self.lock(true)?;
        let path = self.ledger_path();
        let text = read_or_empty(&path)?;
        let mut ledger = replay(&text).ledger;
        let ctx = Ctx {
            now,
            registry: &registry,
        };
        let mut events = ledger.execute(Command::Reap, &ctx)?;
        events.extend(ledger.execute(cmd, &ctx)?);
        if events.is_empty() {
            return Ok(events);
        }
        let mut buf = String::new();
        if !text.is_empty() && !text.ends_with('\n') {
            buf.push('\n');
        }
        for env in &events {
            buf.push_str(&env.to_line()?);
            buf.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| MeshError::io(&path, e))?;
        file.write_all(buf.as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|e| MeshError::io(&path, e))?;
        write_atomic(&self.dir.join(BOARD_FILE), &board::render(&ledger))?;
        Ok(events)
    }

    /// Rewrites the board from the ledger.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file operation fails.
    pub fn write_board(&self) -> Result<()> {
        let _guard = self.lock(true)?;
        let ledger = replay(&read_or_empty(&self.ledger_path())?).ledger;
        write_atomic(&self.dir.join(BOARD_FILE), &board::render(&ledger))
    }

    /// `true` if the board file matches the ledger.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file cannot be read.
    pub fn board_is_current(&self) -> Result<bool> {
        let ledger = self.load()?.ledger;
        let board = read_or_empty(&self.dir.join(BOARD_FILE))?;
        Ok(board == board::render(&ledger))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, TaskSpec};
    use crate::ids::{AgentId, Skill, TaskId};
    use crate::lifecycle::State;

    fn tid(s: &str) -> TaskId {
        TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
    }

    fn add(id: &str) -> Command {
        Command::Add(TaskSpec::new(
            tid(id),
            "T",
            Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
        ))
    }

    fn claim(id: &str, ttl: u64) -> Command {
        Command::Claim {
            task: Some(tid(id)),
            agent: AgentId::new("builder").unwrap_or_else(|e| panic!("{e}")),
            worker: None,
            ttl,
        }
    }

    fn mesh() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let dir = tmp.path().join(MESH_DIR);
        Store::init(&dir).unwrap_or_else(|e| panic!("{e}"));
        let store = Store::open(&dir).unwrap_or_else(|e| panic!("{e}"));
        (tmp, store)
    }

    #[test]
    fn init_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let dir = tmp.path().join(MESH_DIR);
        let first = Store::init(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.created.len(), 4);
        std::fs::write(
            dir.join(REGISTRY_FILE),
            crate::registry::DEFAULT_REGISTRY.replace("max_active = 3", "max_active = 2"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let second = Store::init(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(second.created.is_empty());
        let text = std::fs::read_to_string(dir.join(REGISTRY_FILE)).unwrap_or_default();
        assert!(text.contains("max_active = 2"));
    }

    #[test]
    fn open_and_discover() {
        let (tmp, store) = mesh();
        assert_eq!(store.root(), tmp.path());
        let deep = tmp.path().join("a/b");
        std::fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("{e}"));
        let found = Store::discover(&deep).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(found.dir(), store.dir());
        let other = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            Store::open(other.path()),
            Err(MeshError::NotFound)
        ));
    }

    #[test]
    fn execute_appends_lines_and_updates_the_board() {
        let (_tmp, store) = mesh();
        assert!(store.board_is_current().unwrap_or(false));
        store
            .execute(100, add("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(101, claim("a", 50))
            .unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(store.dir().join(LEDGER_FILE)).unwrap_or_default();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
        assert!(store.board_is_current().unwrap_or(false));
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert!(r.issues.is_empty());
        assert_eq!(r.envelopes.len(), 2);
    }

    #[test]
    fn failed_command_writes_nothing() {
        let (_tmp, store) = mesh();
        store
            .execute(100, add("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(store.execute(100, add("a")).is_err());
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.envelopes.len(), 1);
    }

    #[test]
    fn writes_expire_overdue_leases_first() {
        let (_tmp, store) = mesh();
        store
            .execute(100, add("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .execute(100, claim("a", 10))
            .unwrap_or_else(|e| panic!("{e}"));
        let evs = store
            .execute(200, claim("a", 10))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(evs.len(), 2);
        assert!(matches!(evs[0].event, Event::LeaseExpired { .. }));
        assert!(matches!(evs[1].event, Event::Claimed { .. }));
    }

    #[test]
    fn board_check_finds_manual_edits() {
        let (_tmp, store) = mesh();
        store
            .execute(100, add("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(store.dir().join(BOARD_FILE), "edited\n").unwrap_or_else(|e| panic!("{e}"));
        assert!(!store.board_is_current().unwrap_or(true));
        store.write_board().unwrap_or_else(|e| panic!("{e}"));
        assert!(store.board_is_current().unwrap_or(false));
    }

    #[test]
    fn replay_reports_corrupt_lines_and_conflicts() {
        let (_tmp, store) = mesh();
        store
            .execute(100, add("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        let path = store.dir().join(LEDGER_FILE);
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        let dup = text.clone();
        text.push_str(&dup);
        text.push_str("\n{\"seq\":");
        std::fs::write(&path, &text).unwrap_or_else(|e| panic!("{e}"));
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.ledger.len(), 1);
        assert_eq!(r.issues.len(), 2);
        assert!(matches!(r.issues[0], ReplayIssue::Conflict { line: 2, .. }));
        assert!(matches!(r.issues[1], ReplayIssue::Corrupt { line: 4, .. }));
        // A new write starts on a new line and gets a fresh sequence number.
        let evs = store
            .execute(101, add("b"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(evs[0].seq, 2);
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.ledger.len(), 2);
        assert_eq!(
            r.ledger.task(&tid("b")).map(|t| t.life.state()),
            Some(State::Submitted)
        );
    }

    #[test]
    fn missing_ledger_file_is_an_empty_ledger() {
        let (_tmp, store) = mesh();
        std::fs::remove_file(store.dir().join(LEDGER_FILE)).unwrap_or_else(|e| panic!("{e}"));
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert!(r.ledger.is_empty());
        store.execute(1, add("a")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.load().map(|r| r.ledger.len()).unwrap_or(0), 1);
    }
}
