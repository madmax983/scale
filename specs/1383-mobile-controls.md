# Spec 1383: Mobile Controls — Touch Play for the WASM Build

## 1. Overview
The WASM build boots and renders in a phone browser (verified live at
scale-wasm.fly.dev, 2026-10-12), but the game is keyboard-driven — arrows,
Tab, Enter, Esc, letter keys — and a phone has no keyboard. This spec makes
Scale fully playable on a touchscreen: tap-to-select on the terminal grid,
an on-screen control cluster for the keys the game actually uses, and
viewport scaling so the colony fits a phone screen. It is an input
*translation layer* only: no game logic changes; every touch becomes the
same key/mouse events the game already handles. Mark's request, 2026-10-12:
"How fun would it be to colony sim from my phone."

## 2. Dependencies
- The live WASM shell (Trunk index + Ratzilla web backend) — exists.
- Independent of spec 1382 (Save System), but queued after it: a phone
  session without saves is a demo, not a colony.
- Existing input paths: menu navigation (↑/↓ select, ←/→ change, Tab
  switch row, Enter confirm, Esc back) and the in-game key bindings as
  they exist on trunk at build time — builder inventories them first
  (including any keybinding work Mark lands in the meantime).

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tap_maps_to_correct_cell() {
        // Given a terminal grid of known cols/rows rendered into a known
        // pixel rect, a tap at pixel (x, y) resolves to the expected
        // (col, row) cell — including letterboxed/offset rects.
    }

    #[test]
    fn test_tap_on_menu_item_selects_and_confirms() {
        // A tap on the "Start Game" row produces the same input event
        // sequence the game handles for keyboard selection of that row.
    }

    #[test]
    fn test_control_cluster_emits_real_keys() {
        // Each on-screen button (Up/Down/Left/Right/Enter/Esc/Tab) emits
        // exactly the key event its physical counterpart emits — asserted
        // against the game's own key representation, not a parallel one.
    }

    #[test]
    fn test_grid_scales_to_phone_viewport() {
        // For a 390x844 CSS-pixel portrait viewport, the computed grid
        // scale/cols/rows keep every cell >= 12px wide (tappable) and the
        // full UI visible without horizontal scrolling.
    }
}
```

Where a behavior lives in the JS shell instead of Rust, the same cases are
covered by the shell's test path or by the mapping logic extracted into
Rust; the report states honestly which tests are Rust, which are JS, and
which are manual-device checks.

## 4. GREEN Phase: Minimal Implementation

### Tap layer
- Pointer events on the terminal canvas resolve to grid cells
  (cell size from the rendered font metrics / canvas rect).
- Tap behavior mirrors the game's existing pointer support if any exists;
  otherwise tap = move selection/cursor to that cell, and tap on the
  already-selected menu row = Enter (confirm). In map views, tap sets the
  cursor position the keyboard would have moved to.

### On-screen control cluster (DOM, in the Trunk index shell)
- A fixed cluster shown on touch devices (pointer: coarse media query, or
  a manual toggle): D-pad (↑ ↓ ← →), Enter (⏎), Esc, Tab.
- Buttons dispatch the identical keyboard events the game consumes —
  implemented via the same channel Ratzilla/the shell already uses for
  physical keys (builder finds it; no parallel input system).
- Cluster must not cover the terminal: shell layout reserves space
  (terminal area shrinks, cluster sits below/beside), works portrait and
  landscape, and the cluster collapses to a single reopen button.

### Viewport scaling
- The shell scales the terminal to fit the device viewport: dynamic
  cols/rows or CSS transform scale — builder's choice, with the test
  above as the bar (no clipped UI, tappable cells, no horizontal scroll).
- Handle rotation (portrait ↔ landscape) without reload.

### Text input bridge
- Screens that accept typed text (console commands, naming) get a
  "keyboard" button that focuses a real (visually unobtrusive) input so
  the phone's virtual keyboard opens, and typed characters flow into the
  game as key events. If full bridging is disproportionate, the minimal
  viable version is: virtual keyboard opens, Enter submits the line.
  Report honestly which level shipped.

## 5. REFACTOR Phase: Quality & Design
- All touch logic lives in the web shell / wasm input layer — zero changes
  to simulation code, and desktop keyboard/mouse behavior byte-identical.
- Buttons are real DOM buttons (accessible labels), not canvas drawings.

## 6. Acceptance Criteria
- All RED tests pass on their respective sides (Rust/JS); `cargo check
  --all-targets` green incl. wasm32 target; no new full-suite failures.
- Manual device checklist (report records results per item) on a phone
  browser against the deployed build:
  - [ ] Menu: change scenario and origin by tapping / D-pad only
  - [ ] Start a game with no physical keyboard attached
  - [ ] In game: move the cursor, open and close panels (Esc), confirm
        actions (Enter), switch rows/tabs (Tab)
  - [ ] Rotate the phone mid-game; UI remains fully usable
  - [ ] Open the console/text entry and type a command via virtual keyboard
- Desktop regression: keyboard play unchanged (quick manual pass).

## 7. Technical Guidance
- Inventory first: list every key the current build actually handles
  (menus + in-game) and make the cluster cover exactly that set — Mark is
  actively fixing WASM keybindings (2026-10-12), so build against trunk at
  start-of-work, not against assumptions.
- Prefer `pointerdown` over `click` for responsiveness; add
  `touch-action: none` on the canvas so taps don't scroll/zoom the page.
- Keep the shell changes in the Trunk template/assets so Mark's build
  pipeline (trunk push → build) picks them up with no extra steps.

## 8. Questions
- Pinch-to-zoom / drag-pan for large map views? Only if the game gains a
  scrollable map viewport that needs it; out of scope until then.
- Haptics on confirm? Skip — browser vibration APIs are inconsistent and
  it is not needed for playability.
