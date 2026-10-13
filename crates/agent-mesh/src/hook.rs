//! The Claude Code `PreToolUse` hook. It blocks edits outside the role scope.
//!
//! Rules:
//! 1. The hook checks only edit tools. Other tools pass.
//! 2. The main session (no `agent_type`) passes. A session started with `--agent <role>` is that role.
//! 3. An agent that is not in the registry passes. The hook ignores a `plugin:` prefix and letter case.
//! 4. A mesh role passes only for paths in its `writes` globs.
//! 5. A mesh role never writes `.mesh/` or `.claude/` files. The coordinator owns them.
//! 6. Outside the repository, a mesh role passes, but not into a `.claude/` folder.
//! 7. The hook resolves symlinks first. The repository root is the outermost folder with
//!    `.mesh/registry.toml` above the session folder.
//! 8. A broken registry blocks only roles that have `.claude/agents/<role>.md`.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::ids::AgentId;
use crate::registry::Registry;
use crate::scope::{classify, PathClass};
use crate::store::{MESH_DIR, REGISTRY_FILE};

/// Tools that change files.
pub const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Tool input keys that name a file. The hook checks each key that is present.
const PATH_KEYS: [&str; 2] = ["file_path", "notebook_path"];

/// Folders that only the coordinator and humans change.
const PROTECTED: [&str; 2] = [".mesh", ".claude"];

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

impl Decision {
    /// The JSON that Claude Code reads from a `PreToolUse` hook.
    #[must_use]
    pub fn to_hook_json(&self) -> serde_json::Value {
        let (decision, reason) = match self {
            Self::Allow => ("allow", ""),
            Self::Deny(why) => ("deny", why.as_str()),
        };
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": decision,
                "permissionDecisionReason": reason,
            }
        })
    }
}

/// Finds the outermost folder at or above `path` that has `.mesh/registry.toml`.
/// A nested `.mesh/` folder cannot change the rules for its subtree.
#[must_use]
pub fn find_mesh_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .filter(|a| a.join(MESH_DIR).join(REGISTRY_FILE).is_file())
        .last()
        .map(Path::to_path_buf)
}

/// The role name of an agent type: the part after the last `:`, in lower case.
#[must_use]
pub fn normalize_agent(agent_type: &str) -> String {
    agent_type
        .rsplit(':')
        .next()
        .unwrap_or(agent_type)
        .trim()
        .to_ascii_lowercase()
}

/// Resolves symlinks in the part of `path` that exists. The rest stays as given.
fn resolve(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if let Ok(real) = base.canonicalize() {
            let rest = path.strip_prefix(base).unwrap_or_else(|_| Path::new(""));
            return real.join(rest);
        }
    }
    path.to_path_buf()
}

fn has_protected_part(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::Normal(n) if PROTECTED.iter().any(|p| n == *p)))
}

/// Decides whether the tool call can run.
#[must_use]
pub fn decide(input: &HookInput) -> Decision {
    if !EDIT_TOOLS.contains(&input.tool_name.as_str()) {
        return Decision::Allow;
    }
    let Some(agent) = input.agent_type.as_deref().map(normalize_agent) else {
        return Decision::Allow;
    };
    let base = input.cwd.as_deref().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        PathBuf::from,
    );
    let files: Vec<PathBuf> = PATH_KEYS
        .iter()
        .filter_map(|k| input.tool_input.get(*k).and_then(serde_json::Value::as_str))
        .filter(|f| !f.is_empty())
        .map(|f| resolve(&base.join(f)))
        .collect();
    let base = resolve(&base);
    let root = find_mesh_root(&base).or_else(|| files.iter().find_map(|f| find_mesh_root(f)));
    let Some(root) = root else {
        return Decision::Allow;
    };
    let registry = std::fs::read_to_string(root.join(MESH_DIR).join(REGISTRY_FILE))
        .map_err(|e| e.to_string())
        .and_then(|text| Registry::parse(&text).map_err(|e| e.to_string()));
    let registry = match registry {
        Ok(r) => r,
        Err(_) if !root.join(format!(".claude/agents/{agent}.md")).is_file() => {
            return Decision::Allow
        }
        Err(e) => {
            return Decision::Deny(format!(
                "mesh: the registry is broken ({e}); run `scripts/mesh doctor` and fix it"
            ))
        }
    };
    let Some((id, scope)) = AgentId::new(agent)
        .ok()
        .and_then(|id| registry.scope(&id).cloned().map(|s| (id, s)))
    else {
        return Decision::Allow;
    };
    for file in &files {
        let deny = |why: String| {
            Decision::Deny(format!(
                "mesh: role `{id}` may not write `{}`: {why}",
                file.display()
            ))
        };
        let decision = match classify(&file.to_string_lossy(), &root) {
            PathClass::Inside(rel) if has_protected_part(Path::new(&rel)) => deny(
                "the coordinator owns `.mesh/` and `.claude/`. Report this to the coordinator."
                    .into(),
            ),
            PathClass::Inside(rel) if scope.is_match(&rel) => Decision::Allow,
            PathClass::Inside(_) => deny(format!(
                "allowed paths are {}. Report this to the coordinator.",
                if scope.patterns().is_empty() {
                    "none (read only)".to_string()
                } else {
                    scope.patterns().join(", ")
                }
            )),
            PathClass::Outside if has_protected_part(file) => {
                deny("roles do not change Claude Code settings.".into())
            }
            PathClass::Outside => Decision::Allow,
            PathClass::Invalid(why) => deny(why.to_string()),
        };
        if decision != Decision::Allow {
            return decision;
        }
    }
    Decision::Allow
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
        assert_eq!(
            decide(&input("Edit", None, "specs/1.md", root)),
            Decision::Allow
        );
        assert_eq!(
            decide(&input("Read", Some("builder"), "specs/1.md", root)),
            Decision::Allow
        );
        assert_eq!(
            decide(&input("Edit", Some("general-purpose"), "specs/1.md", root)),
            Decision::Allow
        );
    }

    #[test]
    fn role_inside_scope_passes() {
        let tmp = repo();
        let root = tmp.path();
        let abs = root.join("src/lib.rs").display().to_string();
        assert_eq!(
            decide(&input("Write", Some("builder"), &abs, root)),
            Decision::Allow
        );
        assert_eq!(
            decide(&input("Edit", Some("builder"), "src/lib.rs", root)),
            Decision::Allow
        );
        assert!(matches!(
            decide(&input("Edit", Some("reviewer"), ".mesh/BOARD.md", root)),
            Decision::Deny(_)
        ));
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
        assert!(matches!(
            decide(&input("Write", Some("reviewer"), "src/lib.rs", root)),
            Decision::Deny(_)
        ));
        assert!(matches!(
            decide(&input("Edit", Some("builder"), "src/../specs/x.md", root)),
            Decision::Deny(_)
        ));
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
        assert_eq!(
            decide(&input("Edit", Some("builder"), "specs/x.md", tmp.path())),
            Decision::Allow
        );
        assert_eq!(decide(&HookInput::default()), Decision::Allow);
    }

    #[test]
    fn broken_registry_blocks_roles() {
        let tmp = repo();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".claude/agents")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            root.join(".claude/agents/builder.md"),
            "---\nname: builder\n---\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(root.join(".mesh/registry.toml"), "broken [")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            decide(&input("Edit", Some("builder"), "src/lib.rs", root)),
            Decision::Deny(_)
        ));
        assert_eq!(
            decide(&input("Edit", None, "src/lib.rs", root)),
            Decision::Allow
        );
    }

    #[test]
    fn find_root_walks_up() {
        let tmp = repo();
        let deep = tmp.path().join("a/b/c.rs");
        assert_eq!(find_mesh_root(&deep).as_deref(), Some(tmp.path()));
        assert_eq!(find_mesh_root(Path::new("/")), None);
    }

    #[test]
    fn multiedit_is_checked() {
        let tmp = repo();
        let root = tmp.path();
        assert!(matches!(
            decide(&input("MultiEdit", Some("builder"), "specs/x.md", root)),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn deny_json_has_the_claude_code_shape() {
        let v = Decision::Deny("why".into()).to_hook_json();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(v["hookSpecificOutput"]["permissionDecisionReason"], "why");
        assert_eq!(
            Decision::Allow.to_hook_json()["hookSpecificOutput"]["permissionDecision"],
            "allow"
        );
    }
}
