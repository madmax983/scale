//! The mesh folder: registry, ledger file, lock, and board.
//!
//! Every write takes an exclusive lock on `ledger.lock`, replays the ledger,
//! expires overdue leases, runs the command, appends the events, and
//! rewrites the board. Reads take a shared lock.

use std::path::{Path, PathBuf};

use crate::error::{MeshError, Result};
use crate::event::Envelope;
use crate::ledger::{Command, Ledger};
use crate::registry::Registry;

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
    let _ = text;
    todo!("RED")
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
        let _ = dir.into();
        todo!("RED")
    }

    /// Finds `.mesh/registry.toml` in `start` or a parent folder.
    ///
    /// # Errors
    /// [`MeshError::NotFound`] if no folder has it.
    pub fn discover(start: &Path) -> Result<Self> {
        let _ = start;
        todo!("RED")
    }

    /// Makes the mesh folder with the default registry, an empty ledger, a board,
    /// and a `.gitignore` for the lock file. It does not change files that exist.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file write fails.
    pub fn init(dir: &Path) -> Result<InitReport> {
        let _ = dir;
        todo!("RED")
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
        todo!("RED")
    }

    /// Replays the ledger under a shared lock.
    ///
    /// # Errors
    /// [`MeshError::Io`] if the files cannot be read.
    pub fn load(&self) -> Result<Replay> {
        todo!("RED")
    }

    /// Runs one command as one atomic write. Overdue leases expire first.
    /// Returns all new events, lease expiries first.
    ///
    /// # Errors
    /// The command error, or [`MeshError::Io`]. On error nothing is written.
    pub fn execute(&self, now: u64, cmd: Command) -> Result<Vec<Envelope>> {
        let _ = (now, cmd);
        todo!("RED")
    }

    /// Rewrites the board from the ledger.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file operation fails.
    pub fn write_board(&self) -> Result<()> {
        todo!("RED")
    }

    /// `true` if the board file matches the ledger.
    ///
    /// # Errors
    /// [`MeshError::Io`] if a file cannot be read.
    pub fn board_is_current(&self) -> Result<bool> {
        todo!("RED")
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
        Command::Add(TaskSpec::new(tid(id), "T", Skill::new("build").unwrap_or_else(|e| panic!("{e}"))))
    }

    fn claim(id: &str, ttl: u64) -> Command {
        Command::Claim { task: Some(tid(id)), agent: AgentId::new("builder").unwrap_or_else(|e| panic!("{e}")), worker: None, ttl }
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
        std::fs::write(dir.join(REGISTRY_FILE), crate::registry::DEFAULT_REGISTRY.replace("max_active = 3", "max_active = 2"))
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
        assert!(matches!(Store::open(other.path()), Err(MeshError::NotFound)));
    }

    #[test]
    fn execute_appends_lines_and_updates_the_board() {
        let (_tmp, store) = mesh();
        assert!(store.board_is_current().unwrap_or(false));
        store.execute(100, add("a")).unwrap_or_else(|e| panic!("{e}"));
        store.execute(101, claim("a", 50)).unwrap_or_else(|e| panic!("{e}"));
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
        store.execute(100, add("a")).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.execute(100, add("a")).is_err());
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.envelopes.len(), 1);
    }

    #[test]
    fn writes_expire_overdue_leases_first() {
        let (_tmp, store) = mesh();
        store.execute(100, add("a")).unwrap_or_else(|e| panic!("{e}"));
        store.execute(100, claim("a", 10)).unwrap_or_else(|e| panic!("{e}"));
        let evs = store.execute(200, claim("a", 10)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(evs.len(), 2);
        assert!(matches!(evs[0].event, Event::LeaseExpired { .. }));
        assert!(matches!(evs[1].event, Event::Claimed { .. }));
    }

    #[test]
    fn board_check_finds_manual_edits() {
        let (_tmp, store) = mesh();
        store.execute(100, add("a")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(store.dir().join(BOARD_FILE), "edited\n").unwrap_or_else(|e| panic!("{e}"));
        assert!(!store.board_is_current().unwrap_or(true));
        store.write_board().unwrap_or_else(|e| panic!("{e}"));
        assert!(store.board_is_current().unwrap_or(false));
    }

    #[test]
    fn replay_reports_corrupt_lines_and_conflicts() {
        let (_tmp, store) = mesh();
        store.execute(100, add("a")).unwrap_or_else(|e| panic!("{e}"));
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
        let evs = store.execute(101, add("b")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(evs[0].seq, 2);
        let r = store.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.ledger.len(), 2);
        assert_eq!(r.ledger.task(&tid("b")).map(|t| t.life.state()), Some(State::Submitted));
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
