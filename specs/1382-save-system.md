# Spec 1382: Save System — Persistent Colonies (Native Files, Browser IndexedDB)

## 1. Overview
Scale has no save system: no save command, no world snapshot, nothing persists
a colony between sessions. This spec adds saving and loading everywhere the
game runs. One serialized colony snapshot behind a small storage trait —
native builds write files, the WASM build writes the same blob to the
browser's IndexedDB, and headless/tests use an in-memory store. A player can
close the browser tab (or the terminal) and come back to the same colony.
Mark approved this design 2026-10-12: IndexedDB for saves, localStorage only
for tiny settings crumbs, file export/import as the player's permanent backup.

## 2. Dependencies
- serde + serde_json (already dependencies) for the snapshot format.
- The deterministic simulation: same seed + same inputs ⇒ same state, which
  makes round-trip verification (save → run → load → run ⇒ identical STATS)
  a meaningful test.
- The `wasm` cargo feature / `cfg(target_arch = "wasm32")` dependency split
  already present in Cargo.toml.
- No dependency on any in-flight spec; this is infrastructure, not a mechanic.

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_load_round_trip_preserves_state() {
        // Arrange: live world, run 250 ticks, capture STATS digest.
        // Act: save to slot "test"; advance 100 ticks; load slot "test".
        // Assert: tick counter and STATS digest match the save point exactly,
        // and advancing 100 more ticks matches the uninterrupted run's digest.
    }

    #[test]
    fn test_save_carries_format_version() {
        // A snapshot whose format_version differs from CURRENT_SAVE_VERSION
        // must fail with a typed SaveError::VersionMismatch, never panic
        // and never silently load garbage.
    }

    #[test]
    fn test_save_slots_are_independent() {
        // Save colony A to slot 1, colony B (different seed) to slot 2;
        // loading slot 1 restores A, loading slot 2 restores B.
    }

    #[test]
    fn test_autosave_does_not_clobber_manual_slots() {
        // Autosave writes only the reserved "autosave" slot.
    }

    #[test]
    fn test_list_saves_returns_metadata() {
        // list_saves yields slot name, tick, colony name, format version,
        // and save timestamp for each stored save — without loading worlds.
    }
}
```

## 4. GREEN Phase: Minimal Implementation

### Snapshot
- `SaveSnapshot`: serde-serializable struct holding everything needed to
  resume deterministically — RNG seed + state, current tick, and the
  serialized form of every persistent component and resource. Builder
  surveys the codebase and enumerates what is persistent; transient
  render/UI state is explicitly out.
- `format_version: u32` (`CURRENT_SAVE_VERSION = 1`) as the first field.
- Serialization format: bincode if it is already in the tree, otherwise
  serde_json. (JSON is larger but debuggable; size is acceptable given
  IndexedDB quotas. Builder's call — record the choice in the report.)

### Storage trait
```rust
pub trait SaveStore {
    fn put(&mut self, slot: &str, data: &[u8]) -> Result<(), SaveError>;
    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, SaveError>;
    fn list(&self) -> Result<Vec<SaveMetadata>, SaveError>;
    fn delete(&mut self, slot: &str) -> Result<(), SaveError>;
}
```
- **Native backend:** files under a `saves/` directory (one file per slot,
  sanitized slot names — no path traversal).
- **Memory backend:** HashMap — used by headless and all unit tests.
- **WASM backend:** IndexedDB (database `scale-saves`, object store `saves`,
  key = slot name, value = blob + metadata). IndexedDB is async while the
  ECS is sync: the game fires a save/load *request event*; a bridge performs
  the IDB operation and delivers a *completion event* the sim polls. Use
  web-sys directly (already the wasm dep path) — do NOT pull a heavy new
  framework for this. Request `navigator.storage.persist()` once at startup
  (best-effort; failure is not fatal).

### Commands & UI surface
- Headless: `save <slot>`, `load <slot>`, `saves` (list), `delsave <slot>`;
  STATS gains `saves=<n>` (count of stored saves visible to the backend).
- In-game (native + WASM share the same menu path): Save / Load entries on
  the pause/main menu listing slots with their metadata.
- Autosave: every 1000 ticks to the reserved `autosave` slot, plus on the
  browser `visibilitychange` → hidden event in the WASM shell if cheap to
  wire (nice-to-have; note in report if deferred).

### Settings crumbs (localStorage, WASM only)
- Last scenario and last origin selection persist in localStorage under
  `scale.settings` and pre-select on the menu. Nothing larger ever goes to
  localStorage — it is capped ~5 MB, string-only, and synchronous.

### Export / Import (the permanent backup)
- WASM: "Export save" downloads the slot blob as a `.scale-save` file;
  "Import save" reads one back via file input into a chosen slot.
- Native: export/import are plain file copies (commands may simply document
  the `saves/` path; a headless `export <slot> <path>` is enough).

## 5. REFACTOR Phase: Quality & Design
- The persistent-component enumeration lives in ONE module
  (`persistence` / `save` module) so future specs register their state in
  one obvious place; note in the module docs that new persistent state must
  be added there.
- All storage errors are typed (`SaveError`) and surfaced to the player as a
  chronicle/menu message, never a panic.
- No changes to simulation determinism: loading reconstructs state, it does
  not re-simulate.

## 6. Acceptance Criteria
- All RED tests pass; `cargo check --all-targets` green (native AND
  `cargo check --target wasm32-unknown-unknown` under the wasm feature);
  clippy clean on touched files; full suite shows no new failures beyond
  the documented pre-existing/flaky set.
- Headless playtest: start a colony, run 500 ticks, `save a`, run 200,
  record STATS; `load a`, run 200, STATS identical to the first recording.
- WASM backend compiles and its IndexedDB bridge is exercised by a
  wasm-bindgen test or an in-browser smoke path the report describes
  honestly (if live-browser verification is not possible on the sprite,
  say so — the driver verifies in the real browser after deploy).
- Native backend verified by an on-disk round-trip test in a temp dir.

## 7. Technical Guidance
- Survey before writing: grep for existing `Save`/`snapshot`/`persist`
  fragments and any prior art in experimental modules; build on what
  exists instead of duplicating (the loop has hit dead-code duplicates
  before — specs 336/338).
- Entity identity across save/load: the snapshot owns entity remapping —
  either serialize entities in a stable order and respawn in that order,
  or store an explicit old→new `Entity` map for cross-references
  (relationships, assignments, chronicle actors). Cross-references are
  where save systems usually break; test them (the round-trip test must
  include a colony with assignments and relationships, not an empty map).
- Keep the snapshot boundary honest: if some subsystem's state genuinely
  cannot be serialized yet, exclude it explicitly in the spec report with
  the gameplay consequence named (e.g. "in-flight trade deals reset on
  load"), never silently.

## 8. Questions
- Compression (zstd/flate) for large late-game colonies? Defer unless the
  snapshot for a tick-5000 colony exceeds ~20 MB in playtest.
- Cloud saves / cross-device sync? Explicitly out of scope — export/import
  files are the sync mechanism for now.
