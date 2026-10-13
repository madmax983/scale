//! Write scope: which paths a role may change.

use std::path::Path;
use std::process::Command;

use globset::GlobSet;
use serde::Serialize;

use crate::error::{MeshError, Result};
use crate::ids::AgentId;
use crate::registry::Registry;

/// Files that the mesh CLI writes. Every role may change them.
pub const MESH_MANAGED: [&str; 2] = [".mesh/ledger.jsonl", ".mesh/BOARD.md"];

/// Longest path pattern.
pub const MAX_PATTERN: usize = 256;

/// A compiled set of path globs. `*` stays in one folder. `**` crosses folders.
#[derive(Debug, Clone)]
pub struct Scope {
    patterns: Vec<String>,
    set: GlobSet,
}

impl Scope {
    /// Compiles the patterns.
    ///
    /// # Errors
    /// [`MeshError::InvalidPattern`] for the first pattern that fails [`validate_pattern`].
    pub fn new(patterns: &[String]) -> Result<Self> {
        let _ = patterns;
        todo!("RED")
    }

    /// `true` if the repository-relative path matches a pattern.
    #[must_use]
    pub fn is_match(&self, rel: &str) -> bool {
        let _ = rel;
        todo!("RED")
    }

    /// The source patterns.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

/// Checks one path pattern: not empty, relative, no `..`, and a valid glob.
///
/// # Errors
/// [`MeshError::InvalidPattern`] with the reason.
pub fn validate_pattern(pattern: &str) -> Result<()> {
    let _ = pattern;
    todo!("RED")
}

/// Where a path is, relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathClass {
    /// Inside the repository. The value uses `/` and has no `.` parts.
    Inside(String),
    /// Outside the repository.
    Outside,
    /// Not a valid path. The value is the reason.
    Invalid(&'static str),
}

/// Classifies `path` (absolute, or relative to `root`).
#[must_use]
pub fn classify(path: &str, root: &Path) -> PathClass {
    let _ = (path, root);
    todo!("RED")
}

/// One file outside the scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// The path as given.
    pub path: String,
    /// Why it is outside the scope.
    pub reason: String,
}

/// Checks files against the role scope and, if not empty, the task paths.
///
/// # Errors
/// [`MeshError::UnknownAgent`] or [`MeshError::InvalidPattern`].
pub fn check(
    registry: &Registry,
    agent: &AgentId,
    task_paths: &[String],
    files: &[String],
    root: &Path,
) -> Result<Vec<Violation>> {
    let _ = (registry, agent, task_paths, files, root);
    todo!("RED")
}

/// Lists files changed since `base` (tracked changes and untracked files).
///
/// # Errors
/// [`MeshError::Git`] if `base` starts with `-` or git fails.
pub fn git_changed(root: &Path, base: &str) -> Result<Vec<String>> {
    let _ = (root, base, Command::new("git"));
    todo!("RED")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn star_stays_in_one_folder_and_double_star_crosses() {
        let scope = Scope::new(&s(&["src/*.rs", "docs/**"])).unwrap_or_else(|e| panic!("{e}"));
        assert!(scope.is_match("src/lib.rs"));
        assert!(!scope.is_match("src/a/lib.rs"));
        assert!(scope.is_match("docs/adr/x.md"));
        assert!(scope.is_match("docs/x.md"));
        assert!(!scope.is_match("specs/x.md"));
        assert_eq!(scope.patterns().len(), 2);
    }

    #[test]
    fn empty_scope_matches_nothing() {
        let scope = Scope::new(&[]).unwrap_or_else(|e| panic!("{e}"));
        assert!(!scope.is_match("src/lib.rs"));
    }

    #[test]
    fn pattern_validation() {
        assert!(validate_pattern("src/**").is_ok());
        for bad in ["", "/etc/**", "src/../x", "..", "a/[", &"a".repeat(MAX_PATTERN + 1)] {
            assert!(validate_pattern(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn classify_paths() {
        let root = Path::new("/repo");
        assert_eq!(classify("src/lib.rs", root), PathClass::Inside("src/lib.rs".into()));
        assert_eq!(classify("./src//lib.rs", root), PathClass::Inside("src/lib.rs".into()));
        assert_eq!(classify("src\\ui\\a.rs", root), PathClass::Inside("src/ui/a.rs".into()));
        assert_eq!(classify("/repo/specs/1.md", root), PathClass::Inside("specs/1.md".into()));
        assert_eq!(classify("/tmp/x", root), PathClass::Outside);
        assert!(matches!(classify("src/../specs/x", root), PathClass::Invalid(_)));
        assert!(matches!(classify("", root), PathClass::Invalid(_)));
        assert!(matches!(classify("/repo", root), PathClass::Invalid(_)));
    }
}
