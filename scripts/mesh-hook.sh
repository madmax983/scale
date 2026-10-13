#!/usr/bin/env bash
# Claude Code PreToolUse hook. Blocks edits outside the write scope of a mesh role.
# The binary prints a JSON decision. The settings command ends with `; exit 0`,
# so a broken script never blocks an edit.
# scripts/mesh builds the binary into crates/agent-mesh/target (it sets CARGO_TARGET_DIR).
# MESH_BIN overrides the binary (tests use it).
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="${MESH_BIN:-$here/crates/agent-mesh/target/release/mesh}"
if [[ ! -x "$bin" ]]; then
  # In a git worktree, use the binary of the main checkout.
  common="$(git -C "$here" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  bin="$(dirname "$common")/crates/agent-mesh/target/release/mesh"
fi
[[ -x "$bin" ]] || exit 0
exec env -u MESH_DIR "$bin" hook
