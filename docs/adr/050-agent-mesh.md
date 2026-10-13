# 050. Adopt an Agent Mesh for Agent Coordination

## Status

Accepted

## Context

AI agents build SCALE. They coordinate with markdown boards (`BACKLOG.md`, `IN_PROGRESS.md`, `COMPLETED.md`) and the rule "first commit wins".
Nothing enforces these rules. The boards drift:

- Task 1381 is on all three boards at the same time.
- 15 completed specs are still open in `BACKLOG.md`. Agents can claim them again.
- A claim never expires. A crashed agent holds its task forever.
- The "Writes" lists in `AGENTS.md` are advice only.

Research (A2A 1.0, MAST, Anthropic multi-agent lessons, lease and fencing practice) shows that state hygiene and enforcement cause most failures, not transport.

## Decision

Add an agent mesh. The main chat session is the coordinator. Worker subagents do one task each.

- `crates/agent-mesh`: a standalone Rust crate with the `mesh` CLI. The game build does not include it.
- `.mesh/ledger.jsonl`: an append-only event log. The task state is a replay of the log.
- Task states follow A2A: `submitted`, `working`, `input-required`, `completed`, `failed`, `canceled`.
- A claim is a lease with a fencing token. Each write takes a file lock. Overdue leases expire before each write.
- `.mesh/registry.toml`: agent cards with skills, write globs, and WIP limits.
- `.claude/agents/*.md`: one Claude Code subagent per role. `mesh doctor` checks that they match the registry.
- A `PreToolUse` hook blocks edits outside the role scope.
- `.claude/skills/mesh-coordinator`: the playbook for the coordinator.
- A Verus model proves the lifecycle invariants I1–I5. Property tests check I1–I9.

```mermaid
flowchart LR
    C[Coordinator] -- mesh CLI --> L[(ledger.jsonl)]
    C -- brief --> W[Worker subagent]
    W -- report + evidence --> C
    W -. Edit/Write .-> H[PreToolUse hook]
    H --> R[registry.toml]
    L --> B[BOARD.md]
```

## Consequences

- Positive: claims expire; stale workers cannot complete tasks; completion needs evidence; role scopes are enforced; the ledger is an audit trail.
- Positive: the boards are generated, so they cannot drift without `doctor` failing.
- Negative: the coordinator is the single writer. This keeps races out but serializes ledger writes.
- Negative: `merge=union` can join two ledgers with conflicting events. Replay reports them; a human or the coordinator must resolve them.
- Negative: Bash commands can still write outside the scope. `check-scope` and the coordinator review are the backstop.
- Deferred: generating the old markdown boards from the ledger, OpenTelemetry spans, and file reservations between workers.
