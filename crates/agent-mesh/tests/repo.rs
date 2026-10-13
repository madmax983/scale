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
        assert_eq!(
            doctor::frontmatter_name(&text).as_deref(),
            Some(c.id.as_str())
        );
        assert!(
            text.contains("mesh"),
            "{} must describe the mesh protocol",
            c.id
        );
    }
    let reviewer = read(".claude/agents/reviewer.md");
    let tools = reviewer
        .lines()
        .find(|l| l.starts_with("tools:"))
        .unwrap_or_default();
    assert!(
        !tools.contains("Edit") && !tools.contains("Write"),
        "reviewer reads only"
    );
}

#[test]
fn settings_wire_the_pre_tool_use_hook() {
    let v: serde_json::Value =
        serde_json::from_str(&read(".claude/settings.json")).unwrap_or_else(|e| panic!("{e}"));
    let entries = v["hooks"]["PreToolUse"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let wired = entries.iter().any(|e| {
        e["matcher"]
            .as_str()
            .is_some_and(|m| m.contains("Edit") && m.contains("Write"))
            && e["hooks"].as_array().is_some_and(|h| {
                h.iter().any(|x| {
                    x["command"]
                        .as_str()
                        .is_some_and(|c| c.contains("mesh-hook"))
                })
            })
    });
    assert!(
        wired,
        "PreToolUse hook for Edit|Write must call scripts/mesh-hook.sh"
    );
    let command = entries
        .iter()
        .flat_map(|e| e["hooks"].as_array().cloned().unwrap_or_default())
        .filter_map(|h| h["command"].as_str().map(str::to_string))
        .find(|c| c.contains("mesh-hook"))
        .unwrap_or_default();
    assert!(
        command.trim_end().ends_with("; exit 0"),
        "a broken script must not block edits: {command}"
    );
}

#[test]
fn coordinator_skill_and_scripts_exist() {
    let skill = read(".claude/skills/mesh-coordinator/SKILL.md");
    assert!(skill.starts_with("---\nname: mesh-coordinator\n"));
    for part in [
        "Brief",
        "Stop rules",
        "mesh plan",
        "mesh claim",
        "mesh complete",
        "check-scope",
    ] {
        assert!(skill.contains(part), "skill must cover {part}");
    }
    #[cfg(unix)]
    for script in ["scripts/mesh", "scripts/mesh-hook.sh"] {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            std::fs::metadata(repo_root().join(script)).map_or(0, |m| m.permissions().mode());
        assert!(mode & 0o111 != 0, "{script} must be executable");
    }
}

#[test]
fn ledger_merges_by_union() {
    let attrs = read(".gitattributes");
    assert!(attrs
        .lines()
        .any(|l| l.trim() == agent_mesh::doctor::UNION_RULE));
}

#[test]
fn scripts_and_mesh_files_keep_lf_line_endings() {
    let attrs = read(".gitattributes");
    for rule in [
        "scripts/mesh text eol=lf",
        "*.sh text eol=lf",
        ".mesh/ledger.jsonl text eol=lf merge=union",
        ".mesh/BOARD.md text eol=lf merge=union",
    ] {
        assert!(attrs.lines().any(|l| l.trim() == rule), "missing: {rule}");
    }
}

#[test]
fn scripts_pin_the_target_dir() {
    for script in ["scripts/mesh", "scripts/mesh-hook.sh", "scripts/mesh.ps1"] {
        assert!(read(script).contains("CARGO_TARGET_DIR"), "{script}");
    }
}

/// The settings command blocks through JSON, and a broken script never blocks.
#[cfg(unix)]
#[test]
fn settings_hook_command_denies_with_json_and_never_blocks_by_accident() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let v: serde_json::Value =
        serde_json::from_str(&read(".claude/settings.json")).unwrap_or_else(|e| panic!("{e}"));
    let command = v["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    Store::init(&tmp.path().join(".mesh")).unwrap_or_else(|e| panic!("{e}"));
    let input = serde_json::json!({
        "tool_name": "Write", "agent_type": "builder", "cwd": tmp.path(),
        "tool_input": { "file_path": "specs/x.md" }
    });
    let run = |project: &Path| {
        let mut child = Command::new("bash")
            .arg("-c")
            .arg(&command)
            .env("CLAUDE_PROJECT_DIR", project)
            .env("MESH_BIN", env!("CARGO_BIN_EXE_mesh"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{e}"));
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.to_string().as_bytes())
                .unwrap_or_else(|e| panic!("{e}"));
        }
        child.wait_with_output().unwrap_or_else(|e| panic!("{e}"))
    };
    let out = run(&repo_root());
    assert_eq!(out.status.code(), Some(0));
    let decision: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    assert_eq!(decision["hookSpecificOutput"]["permissionDecision"], "deny");

    // A CRLF copy of the script (a Windows checkout without eol=lf) must not block.
    let broken = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir_all(broken.path().join("scripts")).unwrap_or_else(|e| panic!("{e}"));
    let crlf = read("scripts/mesh-hook.sh").replace('\n', "\r\n");
    std::fs::write(broken.path().join("scripts/mesh-hook.sh"), crlf)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run(broken.path()).status.code(), Some(0));
}
