# Vendored: ratatui-hypertile-extras 0.4.2

Copy of the crates.io release (MIT, https://github.com/nikolic-milos/ratatui-hypertile),
wired in through `[patch.crates-io]` in the root `Cargo.toml`.

## Why

Upstream calls `std::time::Instant::now()` on every `HypertileRuntime::render`.
That panics on `wasm32-unknown-unknown`, which blanked the browser build as soon
as the pane shell drew its first frame.

## Local changes

- `src/runtime/{render,mouse,mod,animation}.rs`: `std::time::Instant` -> `web_time::Instant`
  (a re-export of `std::time::Instant` on native, `performance.now()` on wasm).
- `Cargo.toml`: added the `web-time` dependency.

Drop this directory and the `[patch.crates-io]` entry once upstream supports wasm.
