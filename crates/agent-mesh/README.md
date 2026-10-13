# agent-mesh

The agent mesh for the AI agents that build SCALE.
The main chat session is the **coordinator**. Worker subagents do the tasks.
The `mesh` CLI records each task, claim, and result in the repository.

Decision: [ADR 050](../../docs/adr/050-agent-mesh.md). Plan and acceptance criteria: [design](../../docs/plans/2026-10-13-agent-mesh-design.md).

## Files

| Path | Purpose |
|---|---|
| `.mesh/registry.toml` | Agent cards: role id, skills, write globs, WIP limit. |
| `.mesh/ledger.jsonl` | Append-only event log. Do not edit it by hand. |
| `.mesh/BOARD.md` | Generated board. Do not edit it by hand. |
| `.claude/agents/<role>.md` | One Claude Code worker per role. |
| `.claude/skills/mesh-coordinator/SKILL.md` | The coordinator playbook. |
| `.claude/settings.json` | The `PreToolUse` scope hook. |
| `scripts/mesh`, `scripts/mesh.ps1` | Run the CLI. They build it when needed. |

## Start

```bash
scripts/mesh doctor          # check the mesh
scripts/mesh agents          # list roles
scripts/mesh status          # show tasks by group
```

## Commands

| Command | Who | What |
|---|---|---|
| `init` | coordinator | Make `.mesh/`. Changes nothing if it exists. |
| `add`, `plan <file>` | coordinator | Add tasks. `plan` is atomic and rejects cycles. |
| `ready [--agent R]` | all | List ready tasks. |
| `claim <task>\|--next --agent R` | coordinator | Make a lease. Prints the token. |
| `heartbeat <task> --token N` | coordinator | Extend the lease. |
| `complete <task> --token N --evidence T` | coordinator | Close with proof. |
| `fail`, `ask`, `answer`, `retry`, `cancel` | coordinator | Change the task state. |
| `reap` | coordinator | Expire overdue leases. Each write does this first. |
| `fence <task> --token N` | all | Exit 0 only for the live token. Changes nothing. |
| `status`, `show`, `log` | all | Read the state. Add `--json` for JSON. |
| `board [--check]` | all | Write the board, or check it. |
| `check-scope --agent R [--task T] [files]` | all | Check changed files against the role scope. |
| `hook` | Claude Code | Read hook JSON on stdin. Exit 2 blocks the edit. |
| `doctor [--strict]` | all, CI | Check registry, role files, ledger, board, leases. |

Exit codes: `0` ok, `1` error or failed check, `2` usage error or hook block.
Environment: `MESH_DIR` sets the mesh folder. `MESH_NOW` sets the clock (Unix seconds) for tests.

## Task lifecycle

```mermaid
stateDiagram-v2
    [*] --> submitted
    submitted --> working: claim
    working --> completed: complete
    working --> failed: fail / expiry, no budget
    working --> input_required: ask
    working --> submitted: expiry, budget left
    input_required --> submitted: answer
    failed --> submitted: retry, budget left
    submitted --> canceled: cancel
    working --> canceled: cancel
    input_required --> canceled: cancel
    failed --> canceled: cancel
```

## Invariants

| Id | Invariant | Verus lemma | Test |
|---|---|---|---|
| I1 | Lease exists if and only if the state is `working`. | `lemma_step_preserves_inv` | `properties.rs` |
| I2 | `completed` and `canceled` accept no transition. | `lemma_terminal_absorbing` | `properties.rs`, `lifecycle.rs` |
| I3 | Each claim gets a larger token than all earlier claims. | `lemma_claim_token_fresh`, `lemma_later_claim_larger` | `properties.rs`, `concurrency.rs` |
| I4 | Worker writes need the live lease and its token. | `lemma_fencing`, `lemma_stale_token_rejected` | `properties.rs`, `ledger.rs` |
| I5 | `failures <= max_failures`. `failed` exits only by retry inside the budget, or by cancel. | `lemma_failed_exits`, `lemma_budget_exhausted` | `properties.rs` |
| I6 | A claim needs all dependencies `completed`. | — | `properties.rs` |
| I7 | The dependency graph has no cycle. | — | `properties.rs`, `plan.rs` |
| I8 | A `completed` task has evidence. | — | `properties.rs` |
| I9 | Working tasks per role do not exceed `max_active`. | — | `properties.rs` |

## Check

```bash
cd crates/agent-mesh
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::nursery
cargo test
cargo llvm-cov --fail-under-lines 85
verus verus/lifecycle.rs     # Verus model of the lifecycle
```

## Limits

- The coordinator is the only ledger writer. The CLI is still safe for many writers on one machine.
- A git merge with `merge=union` can add conflicting events. `doctor` reports them.
- The hook checks `Edit` and `Write` tools only. `check-scope` checks Bash changes.
