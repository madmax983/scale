//! Write scope: which paths a role may change.

use std::path::Path;
use std::process::Command;

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
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
        let mut builder = GlobSetBuilder::new();
        for p in patterns {
            validate_pattern(p)?;
            builder.add(glob(p)?);
        }
        let set = builder.build().map_err(|e| MeshError::InvalidPattern {
            pattern: patterns.join(", "),
            reason: e.to_string(),
        })?;
        Ok(Self {
            patterns: patterns.to_vec(),
            set,
        })
    }

    /// `true` if the repository-relative path matches a pattern.
    #[must_use]
    pub fn is_match(&self, rel: &str) -> bool {
        self.set.is_match(rel)
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
    let reason = if pattern.is_empty() {
        Some("the pattern is empty")
    } else if pattern.len() > MAX_PATTERN {
        Some("the pattern is too long")
    } else if pattern.starts_with('/') || pattern.contains('\\') {
        Some("use a path relative to the repository root, with '/'")
    } else if pattern.split('/').any(|part| part == "..") {
        Some("do not use `..`")
    } else {
        None
    };
    reason.map_or_else(
        || glob(pattern).map(|_| ()),
        |reason| {
            Err(MeshError::InvalidPattern {
                pattern: pattern.to_string(),
                reason: reason.to_string(),
            })
        },
    )
}

fn glob(pattern: &str) -> Result<Glob> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| MeshError::InvalidPattern {
            pattern: pattern.to_string(),
            reason: e.kind().to_string(),
        })
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
    let norm = path.replace('\\', "/");
    if norm.is_empty() {
        return PathClass::Invalid("the path is empty");
    }
    let p = Path::new(&norm);
    let rel = if p.is_absolute() {
        match p.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().into_owned(),
            Err(_) => return PathClass::Outside,
        }
    } else {
        norm.clone()
    };
    let mut parts = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => return PathClass::Invalid("the path uses `..`"),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return PathClass::Invalid("the path is the repository root");
    }
    PathClass::Inside(parts.join("/"))
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
    let scope = registry
        .scope(agent)
        .ok_or_else(|| MeshError::UnknownAgent(agent.clone()))?;
    let task_scope = if task_paths.is_empty() {
        None
    } else {
        Some(Scope::new(task_paths)?)
    };
    let mut sorted: Vec<&String> = files.iter().collect();
    sorted.sort();
    sorted.dedup();
    let mut out = Vec::new();
    for file in sorted {
        let reason = match classify(file, root) {
            PathClass::Inside(rel) if MESH_MANAGED.contains(&rel.as_str()) => None,
            PathClass::Inside(rel) if !scope.is_match(&rel) => {
                Some(format!("role `{agent}` may not write `{rel}`"))
            }
            PathClass::Inside(rel) if task_scope.as_ref().is_some_and(|t| !t.is_match(&rel)) => {
                Some(format!("`{rel}` is outside the task paths"))
            }
            PathClass::Inside(_) => None,
            PathClass::Outside => Some("the path is outside the repository".to_string()),
            PathClass::Invalid(why) => Some(why.to_string()),
        };
        if let Some(reason) = reason {
            out.push(Violation {
                path: file.clone(),
                reason,
            });
        }
    }
    Ok(out)
}

/// Lists files changed since `base` (tracked changes and untracked files).
///
/// # Errors
/// [`MeshError::Git`] if `base` starts with `-` or git fails.
pub fn git_changed(root: &Path, base: &str) -> Result<Vec<String>> {
    if base.is_empty() || base.starts_with('-') {
        return Err(MeshError::Git(format!(
            "invalid base `{base}`; use a branch, tag, or commit"
        )));
    }
    let diff = run_git(root, &["diff", "--name-only", "-z", base, "--"])?;
    let untracked = run_git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut files: Vec<String> = diff
        .split('\0')
        .chain(untracked.split('\0'))
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// The top folder of the git work tree that contains `start`.
///
/// # Errors
/// [`MeshError::Git`] if `start` is not in a git work tree.
pub fn git_root(start: &Path) -> Result<std::path::PathBuf> {
    let top = run_git(start, &["rev-parse", "--show-toplevel"])?;
    Ok(std::path::PathBuf::from(top.trim_end_matches(['\n', '\r'])))
}

fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| MeshError::Git(format!("cannot run git: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(MeshError::Git(err.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
        for bad in [
            "",
            "/etc/**",
            "src/../x",
            "..",
            "a/[",
            &"a".repeat(MAX_PATTERN + 1),
        ] {
            assert!(validate_pattern(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn classify_paths() {
        let root = Path::new("/repo");
        assert_eq!(
            classify("src/lib.rs", root),
            PathClass::Inside("src/lib.rs".into())
        );
        assert_eq!(
            classify("./src//lib.rs", root),
            PathClass::Inside("src/lib.rs".into())
        );
        assert_eq!(
            classify("src\\ui\\a.rs", root),
            PathClass::Inside("src/ui/a.rs".into())
        );
        assert_eq!(
            classify("/repo/specs/1.md", root),
            PathClass::Inside("specs/1.md".into())
        );
        assert_eq!(classify("/tmp/x", root), PathClass::Outside);
        assert!(matches!(
            classify("src/../specs/x", root),
            PathClass::Invalid(_)
        ));
        assert!(matches!(classify("", root), PathClass::Invalid(_)));
        assert!(matches!(classify("/repo", root), PathClass::Invalid(_)));
    }
}
