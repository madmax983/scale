---
name: mesh-coordinator
description: Plan and run multi-step work through the SCALE agent mesh. The main chat is the coordinator and sends tasks to worker subagents (designer, lore-master, architect, builder, integrator, codex, reviewer). Use when the user asks for work that several roles must do, or says "use the mesh".
---

# Mesh coordinator

You are the coordinator. You plan, assign, check, and close tasks.
You are the only writer of the ledger. Workers never run mesh write commands.
In this file, `mesh` means `scripts/mesh` (Windows: `scripts/mesh.ps1`).

## 1. Start

1. Run `mesh doctor`. Fix each error before you continue.
2. Run `mesh status`. Close or cancel old tasks first.
3. Run `mesh reap` to expire old leases.

## 2. Plan

1. Split the goal into tasks. One task has one skill and one role.
2. Make each task small: one worker, one session, one check command.
3. Add a `review` task after each `build` or `fix` task. The review depends on it.
4. Give parallel tasks `paths` that do not overlap.
5. Write the plan to `.mesh/plans/<date>-<slug>.toml`:

```toml
[[task]]
id = "spec-1370-build"
title = "Implement spec 1370"
skill = "build"
deps = []
paths = ["src/layer1/**", "tests/**"]
detail = "Read specs/1370-civilizational-compost.md. Check: cargo test."
max_failures = 3
```

6. Run `mesh plan <file> --dry-run`. Then run `mesh plan <file>`.

## 3. Dispatch

For each task in `mesh ready`:

1. Run `mesh claim <task> --agent <role> --worker <label>`. Keep the token.
2. Start the worker with the Agent tool. Set `subagent_type` to the role. Send the brief below.
3. Send independent tasks in parallel. Use at most 3 builders at one time.
4. Use `isolation: worktree` for parallel builders. Commit `.mesh/` first, because a worktree starts from the committed state.

## 4. Brief

Workers do not see this chat. Put all facts in the brief:

```text
Task: <id> — <title>
Token: <token>
Mesh: <absolute path>/scripts/mesh
Detail: <detail from the plan>
Paths: <paths, or "role scope">
Check: <command that proves the work>
Do not: <limits>
Report: Result, Evidence, Changes, Risks.
```

## 5. Verify and close

When a worker reports:

1. Read the report. Run the check command yourself. Do not trust the report alone.
2. Run `mesh check-scope --agent <role> --task <task>` on the changes.
3. Run `mesh fence <task> --token <token>`. If it fails, do not merge the work.
4. If all checks pass, run `mesh complete <task> --token <token> --evidence "<proof>"`. Then merge.
5. If a check fails, run `mesh fail <task> --token <token> --reason "<why>"`. Then `mesh retry <task>` or change the plan.
6. If the worker asks a question, run `mesh ask <task> --token <token> --question "<q>"`. Answer with `mesh answer <task> --text "<a>"`, or ask the user.
7. For long tasks, run `mesh heartbeat <task> --token <token>` at each check-in.

## 6. Stop rules

1. Stop when all tasks are terminal. Then run `mesh doctor --strict`.
2. A task that used its failure budget stays `failed`. Change the plan or ask the user.
3. Ask the user when a question needs a product decision.
4. Do not run more than 5 workers at one time.
5. Do not complete a task without evidence that you checked.

## 7. Commit

1. Add trailers to each work commit: `Mesh-Task: <task>` and `Mesh-Token: <token>`.
2. Commit `.mesh/ledger.jsonl` and `.mesh/BOARD.md` with the work.
3. Do not edit `.mesh/ledger.jsonl` or `.mesh/BOARD.md` by hand.
