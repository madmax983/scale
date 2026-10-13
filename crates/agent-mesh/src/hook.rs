//! The Claude Code `PreToolUse` hook. It blocks edits outside the role scope.
//!
//! Rules:
//! 1. Only edit tools are checked. Other tools pass.
//! 2. The main session (no `agent_type`) passes.
//! 3. An agent that is not in the registry passes.
//! 4. A file outside the repository passes.
//! 5. A mesh role passes only for paths in its `writes` globs or mesh-managed files.
//! 6. A broken registry blocks mesh roles. Run `mesh doctor` to find the cause.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Tools that change files.
pub const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// The part of the hook input that the mesh reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookInput {
    /// The tool name.
    #[serde(default)]
    pub tool_name: String,
    /// The tool arguments.
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// The subagent type. Absent for the main session.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// The working folder of the session.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// The hook result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Let the tool run.
    Allow,
    /// Block the tool. The text tells the agent why.
    Deny(String),
}

/// Finds the nearest folder at or above `path` that has `.mesh/registry.toml`.
#[must_use]
pub fn find_mesh_root(path: &Path) -> Option<PathBuf> {
    let _ = path;
    todo!("RED")
}

/// Decides whether the tool call can run.
#[must_use]
pub fn decide(input: &HookInput) -> Decision {
    let _ = input;
    todo!("RED")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Store, MESH_DIR};

    fn repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        Store::init(&tmp.path().join(MESH_DIR)).unwrap_or_else(|e| panic!("{e}"));
        tmp
    }

    fn input(tool: &str, agent: Option<&str>, file: &str, cwd: &Path) -> HookInput {
        HookInput {
            tool_name: tool.into(),
            tool_input: serde_json::json!({ "file_path": file }),
            agent_type: agent.map(str::to_string),
            cwd: Some(cwd.display().to_string()),
        }
    }

    #[test]
    fn main_session_and_other_tools_pass() {
        let tmp = repo();
        let root = tmp.path();
        assert_eq!(decide(&input("Edit", None, "specs/1.md", root)), Decision::Allow);
        assert_eq!(decide(&input("Read", Some("builder"), "specs/1.md", root)), Decision::Allow);
        assert_eq!(decide(&input("Edit", Some("general-purpose"), "specs/1.md", root)), Decision::Allow);
    }

    #[test]
    fn role_inside_scope_passes() {
        let tmp = repo();
        let root = tmp.path();
        let abs = root.join("src/lib.rs").display().to_string();
        assert_eq!(decide(&input("Write", Some("builder"), &abs, root)), Decision::Allow);
        assert_eq!(decide(&input("Edit", Some("builder"), "src/lib.rs", root)), Decision::Allow);
        assert_eq!(decide(&input("Edit", Some("reviewer"), ".mesh/BOARD.md", root)), Decision::Allow);
    }

    #[test]
    fn role_outside_scope_is_denied_with_a_reason() {
        let tmp = repo();
        let root = tmp.path();
        match decide(&input("Edit", Some("builder"), "specs/1384-x.md", root)) {
            Decision::Deny(why) => {
                assert!(why.contains("builder"));
                assert!(why.contains("specs/1384-x.md"));
            }
            Decision::Allow => panic!("builder must not write specs"),
        }
        assert!(matches!(decide(&input("Write", Some("reviewer"), "src/lib.rs", root)), Decision::Deny(_)));
        assert!(matches!(decide(&input("Edit", Some("builder"), "src/../specs/x.md", root)), Decision::Deny(_)));
    }

    #[test]
    fn notebook_path_is_checked() {
        let tmp = repo();
        let root = tmp.path();
        let mut i = input("NotebookEdit", Some("architect"), "", root);
        i.tool_input = serde_json::json!({ "notebook_path": "src/x.ipynb" });
        assert!(matches!(decide(&i), Decision::Deny(_)));
    }

    #[test]
    fn outside_a_mesh_repo_passes() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(decide(&input("Edit", Some("builder"), "specs/x.md", tmp.path())), Decision::Allow);
        assert_eq!(decide(&HookInput::default()), Decision::Allow);
    }

    #[test]
    fn broken_registry_blocks_roles() {
        let tmp = repo();
        let root = tmp.path();
        std::fs::write(root.join(".mesh/registry.toml"), "broken [").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(decide(&input("Edit", Some("builder"), "src/lib.rs", root)), Decision::Deny(_)));
        assert_eq!(decide(&input("Edit", None, "src/lib.rs", root)), Decision::Allow);
    }

    #[test]
    fn find_root_walks_up() {
        let tmp = repo();
        let deep = tmp.path().join("a/b/c.rs");
        assert_eq!(find_mesh_root(&deep).as_deref(), Some(tmp.path()));
        assert_eq!(find_mesh_root(Path::new("/")), None);
    }
}
