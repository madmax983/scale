---
name: mesh-coordinator
description: Plan and run multi-step work through the SCALE agent mesh. The main chat is the coordinator and sends tasks to worker subagents (designer, lore-master, architect, builder, integrator, codex, reviewer). Use when the user asks for work that several roles must do, or says "use the mesh".
---

# Mesh coordinator

You are the coordinator. You plan, assign, check, and close tasks.
You are the only writer of the ledger. Workers run only `fence`, `check-scope`, `show`, and `status`.
In this file, `mesh` means `<main checkout>/scripts/mesh` (Windows: `scripts/mesh.ps1`). Use the full path.

## 1. Start

1. Run `mesh doctor`. Fix each error before you continue.
2. Run `mesh status`. Run `mesh reap` to expire old leases.
3. If an old task will not finish, run `mesh cancel <task> --reason "<why>"`.

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

1. Run `mesh claim <task> --agent <role> --worker <label> --ttl <seconds>`. Set the ttl to the expected task time plus 50% (the default is 3 hours; the maximum is 86400). Keep the token.
2. Record the base: `git rev-parse HEAD`.
3. Start the worker with the Agent tool. Set `subagent_type` to the role. Send the brief below.
4. Send independent tasks in parallel. Use at most 3 builders at one time. Use `isolation: worktree` for parallel builders.
5. A worktree starts from the committed `.mesh/registry.toml`. The ledger stays in the main checkout. Do not commit live claims only for the worker.

## 4. Brief

Workers do not see this chat. Put all facts in the brief:

```text
Task: <id> — <title>
Token: <token>
Mesh: <main checkout>/scripts/mesh
Base: <commit from step 3.2>
Paths: <paths, or "role scope">
Detail: <detail from the plan>
Check: <command that proves the work>
Commit: add the trailers Mesh-Task: <id> and Mesh-Token: <token>
Do not: <limits>
Report: Result, Evidence, Changes (reviewer: Findings), Risks.
```

## 5. Verify and close

When a worker reports:

1. Read the report. Run the check command yourself. Do not trust the report alone.
2. Run `mesh check-scope --agent <role> --task <task> --base <base> --repo <worker work tree>`.
3. Run `mesh fence <task> --token <token>`. If it fails, run `mesh show <task>`. Do not merge the work. Claim the task again when it is ready.
4. If all checks pass, run `mesh complete <task> --token <token> --evidence "<proof>"`. Then merge the work.
5. If a check fails, run `mesh fail <task> --token <token> --reason "<why>"`. Then run `mesh retry <task>`, or change the plan.
6. If the worker asks a question, run `mesh ask <task> --token <token> --question "<q>"`. Get the answer, or ask the user. Run `mesh answer <task> --text "<a>"`. Then claim the task again and put the answer in the new brief.
7. For a background worker, run `mesh heartbeat <task> --token <token>` at each check-in.

## 6. Merge and conflicts

1. Run git commands that change `.mesh/` only when no mesh command runs.
2. After a git merge, run `mesh board`. Then run `mesh doctor`.
3. A `[ledger]` error names a rejected line and its fingerprint. Run `mesh log` to see the line (`!` marks it).
4. Fix the task state with mesh commands (for example, claim and complete again). Then run `mesh ack <fingerprint> --note "<what you did>"`.

## 7. Stop rules

1. Stop when each task is `completed` or `canceled`, or when you ask the user. Then run `mesh doctor --strict`.
2. A task that used its failure budget stays `failed`. Change the plan or ask the user.
3. Ask the user when a question needs a product decision.
4. Do not run more than 5 workers at one time.
5. Do not complete a task without evidence that you checked.

## 8. Commit

1. Commit `.mesh/ledger.jsonl` and `.mesh/BOARD.md` with the work.
2. Do not edit `.mesh/ledger.jsonl` or `.mesh/BOARD.md` by hand.
