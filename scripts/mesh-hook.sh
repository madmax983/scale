#!/usr/bin/env bash
# Claude Code PreToolUse hook. Blocks edits outside the write scope of a mesh role.
# It allows the edit when the mesh binary is not built. `scripts/mesh doctor` builds it.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$root/crates/agent-mesh/target/release/mesh"
[[ -x "$bin" ]] || bin="$root/crates/agent-mesh/target/debug/mesh"
[[ -x "$bin" ]] || exit 0
exec "$bin" hook
