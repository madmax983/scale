//! Checks the mesh files of this repository: registry, agent files, hook, skill.

use std::path::{Path, PathBuf};

use agent_mesh::doctor;
use agent_mesh::registry::{Registry, DEFAULT_REGISTRY};
use agent_mesh::store::Store;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn repo_registry_is_the_default_registry() {
    assert_eq!(read(".mesh/registry.toml"), DEFAULT_REGISTRY);
}

#[test]
fn repo_mesh_has_no_doctor_errors() {
    let store = Store::open(repo_root().join(".mesh")).unwrap_or_else(|e| panic!("{e}"));
    let report = doctor::run(&store, 0);
    assert_eq!(report.errors(), 0, "{:#?}", report.findings);
}

#[test]
fn each_role_has_a_claude_agent_file() {
    let r = Registry::parse(DEFAULT_REGISTRY).unwrap_or_else(|e| panic!("{e}"));
    for c in r.cards() {
        let text = read(&format!(".claude/agents/{}.md", c.id));
        assert_eq!(doctor::frontmatter_name(&text).as_deref(), Some(c.id.as_str()));
        assert!(text.contains("mesh"), "{} must describe the mesh protocol", c.id);
    }
    let reviewer = read(".claude/agents/reviewer.md");
    let tools = reviewer.lines().find(|l| l.starts_with("tools:")).unwrap_or_default();
    assert!(!tools.contains("Edit") && !tools.contains("Write"), "reviewer reads only");
}

#[test]
fn settings_wire_the_pre_tool_use_hook() {
    let v: serde_json::Value = serde_json::from_str(&read(".claude/settings.json")).unwrap_or_else(|e| panic!("{e}"));
    let entries = v["hooks"]["PreToolUse"].as_array().cloned().unwrap_or_default();
    let wired = entries.iter().any(|e| {
        e["matcher"].as_str().is_some_and(|m| m.contains("Edit") && m.contains("Write"))
            && e["hooks"].as_array().is_some_and(|h| h.iter().any(|x| x["command"].as_str().is_some_and(|c| c.contains("mesh-hook"))))
    });
    assert!(wired, "PreToolUse hook for Edit|Write must call scripts/mesh-hook.sh");
}

#[test]
fn coordinator_skill_and_scripts_exist() {
    let skill = read(".claude/skills/mesh-coordinator/SKILL.md");
    assert!(skill.starts_with("---\nname: mesh-coordinator\n"));
    for part in ["Brief", "Stop rules", "mesh plan", "mesh claim", "mesh complete", "check-scope"] {
        assert!(skill.contains(part), "skill must cover {part}");
    }
    #[cfg(unix)]
    for script in ["scripts/mesh", "scripts/mesh-hook.sh"] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(repo_root().join(script)).map(|m| m.permissions().mode()).unwrap_or(0);
        assert!(mode & 0o111 != 0, "{script} must be executable");
    }
}

#[test]
fn ledger_merges_by_union() {
    assert!(read(".gitattributes").lines().any(|l| l.trim() == ".mesh/ledger.jsonl merge=union"));
}
