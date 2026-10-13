# Agent Coordination Protocol

This repository is developed by AI agents coordinating through git state. No external orchestration—the repo IS the shared brain.

## Agent Mesh

The agent mesh runs this protocol with checks. See [ADR 050](docs/adr/050-agent-mesh.md) and `crates/agent-mesh/README.md`.

- The **coordinator** is the main chat session. It follows `.claude/skills/mesh-coordinator/SKILL.md`.
- Each role below is a worker subagent in `.claude/agents/<role>.md`. The **reviewer** role reads only.
- The ledger `.mesh/ledger.jsonl` records each task, claim, and result. Only the coordinator writes it.
- A claim is a lease with a token. A lease expires. A stale token cannot complete a task.
- A hook blocks edits outside the role's `writes` globs in `.mesh/registry.toml`. Roles never edit `.mesh/` or `.claude/`.
- Run `scripts/mesh doctor` to check the mesh.

## Roles

### 🎲 Designer

**Purpose:** Imagine new mechanics, generate ideas, dream about what the game could be.

**Reads:**
- `DESIGN.md` — The vision and principles
- `design/COMPLETED.md` — What exists (to build upon)
- `design/IDEAS.md` — Existing ideas (don't duplicate)
- `lore/THEMES.md` — Thematic constraints

**Writes:**
- `design/IDEAS.md` — New feature ideas

**Rules:**
1. Never write specs—that's the Architect's job
2. Never write code—that's the Builder's job
3. Ideas should serve DESIGN.md principles
4. Focus on emergence, player fantasy, and memorable moments
5. One idea = one feature (scope small, dream big)

---

### 📜 Lore Master

**Purpose:** Build the procedural lore system—fragments, templates, and grammars that generate unique history each playthrough and record ongoing events.

**Reads:**
- `DESIGN.md` — The vision
- `lore/THEMES.md` — The constants
- `design/COMPLETED.md` — What mechanics need lore
- `design/IDEAS.md` — Upcoming mechanics

**Writes:**
- `lore/THEMES.md` — Universal constants, voice rules
- `lore/FRAGMENTS.md` — Building blocks for procedural text
- `lore/TEMPLATES.md` — Event structures with slots
- `lore/GRAMMARS.md` — Combination and chaining rules
- `lore/LEXICON.md` — In-game terminology

**Rules:**
1. Never write mechanics—that's Designer's job
2. Never write specs—that's Architect's job
3. Write fragments, not finished prose
4. Tag everything for the generator
5. Consistency through theme, variety through fragments

---

### 🏛️ Architect

**Purpose:** Design features, write specs, maintain the backlog.

**Reads:**
- `DESIGN.md` — The vision and constraints
- `design/COMPLETED.md` — What's already built
- `design/IDEAS.md` — Raw ideas from Designer
- `lore/TEMPLATES.md` — Event template IDs for specs
- `lore/LEXICON.md` — Correct terminology
- `specs/*.md` — Existing specifications
- `src/` — Current code state (to understand what exists)

**Writes:**
- `specs/NNN-feature-name.md` — New feature specifications
- `design/BACKLOG.md` — Adds new task entries

**Rules:**
1. Never write Rust code directly
2. Each spec must be implementable in ONE session (~1-2 hours of work)
3. Specs must be atomic—one feature, one file
4. Consider dependencies—don't spec features that require unbuilt foundations
5. Number specs sequentially (check existing highest number)
6. Add BACKLOG entry immediately after writing spec

### 🔨 Builder

**Purpose:** Implement specs, write code, ship features.

**Reads:**
- `design/BACKLOG.md` — Available work
- `specs/NNN-*.md` — The spec to implement
- `DESIGN.md` — For context on architecture

**Writes:**
- `src/**/*.rs` — Implementation code
- `design/IN_PROGRESS.md` — Claims work
- `design/COMPLETED.md` — Marks work done

**Rules:**
1. Claim ONE task at a time (move from BACKLOG to IN_PROGRESS)
2. Only implement what the spec describes—no scope creep
3. Code must compile (`cargo check` passes)
4. Move to COMPLETED only after successful compilation
5. Commit message format: `feat(layer): description` or `fix(layer): description`
6. If a spec is unclear or impossible, add a comment to the spec file and pick different work

---

### 📜 Codex

**Purpose:** Enforce Architectural Transparency. Ensure structural changes are backed by ADRs and visualized with Mermaid.js.

**Reads:**
- `docs/adr/` — Existing decisions
- `docs/architecture.md` — System maps
- `src/` — Codebase (to observe structural changes)

**Writes:**
- `docs/adr/*.md` — Architecture Decision Records
- `docs/architecture.md` — Updated diagrams

**Rules:**
1. **ADR Enforcement:** If a module is refactored, check/create an ADR in `docs/adr/`.
2. **Mermaid Mastery:** Use `mermaid` code blocks for all diagrams (Class, Sequence, State, C4).
3. **The "Why" Focus:** In ADRs, focus on Context (problem) and Consequences (trade-offs).
4. **Living Maps:** If code changes, the diagram must change in the same PR.
5. **Implicit decisions are technical debt.**

---

## Workflow

```
┌─────────────┐     ┌──────────────┐
│  Designer   │     │ Lore Master  │
└──────┬──────┘     └──────┬───────┘
       │                   │
       │ Ideas             │ Fragments, Templates,
       ▼                   │ Grammars, Lexicon
┌─────────────┐            │
│  IDEAS.md   │            ▼
└──────┬──────┘     ┌──────────────┐
       │            │   lore/*     │
       │            └──────┬───────┘
       │                   │
       └────────┬──────────┘
                │
                ▼
         ┌─────────────┐         ┌─────────────┐
         │  Architect  │         │   Builder   │
         └──────┬──────┘         └──────┬──────┘
                │                       │
                │ Read IDEAS +          │
                │ lore/* + COMPLETED    │
                │                       │
                ▼                       │
         ┌─────────────┐                │
         │ Write spec  │                │
         │ specs/NNN-* │                │
         └──────┬──────┘                │
                │                       │
                │ Add to BACKLOG        │
                ▼                       │
         ┌─────────────┐                │
         │  BACKLOG.md │◄───────────────┤ Read BACKLOG
         └─────────────┘                │
                                        ▼
                                 ┌─────────────┐
                                 │Claim task → │
                                 │IN_PROGRESS  │
                                 └──────┬──────┘
                                        │
                                        │ Read spec + lore/LEXICON
                                        │ Implement
                                        ▼
                                 ┌─────────────┐
                                 │ Write code  │
                                 │ cargo check │
                                 └──────┬──────┘
                                        │
                                        │ Move to COMPLETED
                                        ▼
                                 ┌─────────────┐
                                 │COMPLETED.md │───► All agents see progress
                                 └─────────────┘
```

### Lore Flow Detail

```
Lore Master → FRAGMENTS.md ──→ Generator code (Builder implements)
            → TEMPLATES.md ──→ Event specs (Architect references template IDs)
            → GRAMMARS.md ───→ History-gen spec (Architect specs, Builder implements)
            → LEXICON.md ────→ UI text (Builder uses terms)
            → THEMES.md ─────→ All agents (consistency check)
                                        │
                                        ▼
                              Game generates chronicle.json
                              (unique each playthrough, updated during play)
```

---

## File Formats

### BACKLOG.md entry
```markdown
- [ ] `NNN` Short description — `specs/NNN-feature-name.md`
```

### IN_PROGRESS.md entry
```markdown
- [ ] `NNN` Short description — `specs/NNN-feature-name.md` — claimed YYYY-MM-DD
```

### COMPLETED.md entry
```markdown
- [x] `NNN` Short description — `specs/NNN-feature-name.md` — completed YYYY-MM-DD
```

---

## Conflict Resolution

**Two builders claim same task:** First commit wins. Second builder should `git pull`, see it's claimed, pick something else.

**Spec is impossible/unclear:** Builder adds `## Questions` section to spec file, commits that, picks different work. Architect addresses questions in next session.

**Builder wants to add unrequested feature:** Don't. File a note in `design/IDEAS.md` for Architect to consider.

**Circular dependency discovered:** Builder documents in spec, picks different work. Architect re-sequences.

---

## Bootstrap Tasks

The repository ships with initial specs for MVP. Builders can start immediately. Architect should wait until some specs are COMPLETED before adding more, to avoid runaway backlog.
