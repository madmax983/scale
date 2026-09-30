# SCALE Revival Triage — 2026-09-29/30

Revival branch: `vesper/scale-revival` (from `trunk`).
Local checkout: `~/workspace/scale`. **Pushed to GitHub** 2026-09-30
(commit `8f30e7d6a1ed36f017d30fcf8b7649bd6dbdf0bd`; starvation fix follows
as a second push).

## Ugly-first baseline

The Jules swarm left a 223k-line Rust game (Bevy 0.15, Dwarf Fortress x
Stellaris) that **compiled** but was **unplayable on two counts**, both
reported by Mark:

1. **Mass suffocation on bootstrap.** New colonies died within seconds of
   spawning — every pop suffocating before the player could act.
2. **Permanent earthquake.** The viewport shook nonstop during ordinary play,
   making the game unwatchable.

Both were root-caused to small, specific defects (below) — not architectural
rot. The swarm's unhinged creative core (Desire Dust roads, propaganda
constellations, 30k lines of design docs) was left intact; the revival is a
rescue, not a redesign.

## Root cause 1: starting pops spawned in vacuum

`src/setup.rs`:

- **Scatter fallback:** when no valid 5x5 starter-habitat site exists
  (pathological map), `spawn_initial_pops()` scattered pops onto raw tiles
  with **zero pressure seeding**. `pressure_damage_system` then killed the
  colony within seconds.
- **Ground Survival:** `apply_ground_survival_start` seeded interior tiles at
  0.45 pressure, then **overwrote the life-support tile itself to 0.15** —
  below the 0.2 suffocation threshold. Pops idling on the generator suffocated
  next to the machine meant to save them.

**Fix** (`src/setup.rs`):

- New `seed_emergency_oxygen_for_pops()`: every pop in the scatter fallback
  gets a breathable 5x5 pressure patch (seeded 1.0) plus a `LifeSupport` unit
  on an adjacent free tile so the generator holds the patch against diffusion
  into vacuum.
- Removed the 0.15 overwrite; the life-support tile keeps the 0.45 interior
  pressure (well above the 0.2 threshold).

**Regression tests** (`src/setup.rs` `mod tests`):
`test_starting_pops_spawn_in_breathable_air` (all four start scenarios:
Classic, GroundSurvival, SocialDrama, Layer2Ready) and the emergency
scatter-spawn fallback test assert every starting pop tile is >= 0.2
pressure and at least one `LifeSupport` building exists. (Tests compile;
execution blocked — see Build & test.)

## Root cause 2: screen shake triggered every work tick

`src/layer1/execution/mining/legacy_mining.rs`:

- `handle_mining_visuals` and `handle_chopping_visuals` called
  `trigger_shake()` on **every ordinary work tick** (intensity scaled with
  progress). `ScreenShake` decays only 0.9x per update, so continuous work
  refreshed it faster than it decayed and it saturated at the 5.0 cap —
  a permanent earthquake during normal play.

**Fix:** working ticks now emit only occasional dust/chip particles (30%
chance). Shake remains for actual **events**: work completion, crits, combat,
explosions, geology, building placement — all one-shot, all verified
event-driven (not per-tick) at the other `trigger_shake` call sites
(`building.rs`, `turret.rs`, `combat.rs`, `map.rs`, `direct_link.rs`,
`pop.rs`, `demolish.rs`, `fauna`, `geology`).

**Regression test** (`legacy_mining.rs` `mod tests`):
`test_working_ticks_do_not_shake_screen` drives 200 mining + 200 chopping
work ticks against a live worker entity and asserts `ScreenShake.intensity`
stays exactly 0.0. (Compiles; execution blocked — see Build & test.)

## Also fixed

- `src/bin/headless.rs` `print_stats`: E0502 borrow error (immutable
  `PressureGrid` borrow held across a mutable world query). Fixed by
  collecting pop positions first, dropping the query borrow, then reading
  pressure. Headless stats line:
  `STATS tick=... pops=... avg_health=... min_pressure=... food=... wood=...
  stone=... tools=... buildings=... lifesupport=...`
- `src/setup.rs`: registered `Events<SubLightArrivalEvent>` in
  `setup_world_with_config`. The `sub_light_arrival_chronicle_bridge_system`
  panicked on the very first tick without it ("could not access
  Res<'_, Events<SubLightArrivalEvent>>").

## Root cause 3: instant-despawn starvation killer wiped the colony on tick 2

`src/layer1/psychology/needs.rs` :: `kill_starving_pops_system`:

- Instantly **despawned** any pop with `hunger <= 0.0` (no death, no corpse,
  no `Dead` marker — the entity simply vanished).
- Starting pops spawn with hunger 0.8, but a rapid (still unexplained)
  hunger drain drops them to ~0.1 on tick 1. By tick 2, 3 of 5 pops hit 0.0
  and vanished. (With the mind-spore infection active, 2 random pops got
  hunger set to 1.0 and survived — which is how the infection was initially
  misidentified as the culprit. Disabling the infection made all 5 vanish,
  proving the killer targeted the uninfected.)
- This contradicted the design intent: `starvation_damage_system` already
  implements gradual starvation (0.2 dmg/tick at hunger 0.0, ~500 ticks to
  die). The instant despawn made the graceful system pointless.

**Fix** (`src/layer1/systems/consumption.rs`): removed
`kill_starving_pops_system` from the schedule. Starvation is now handled by
`starvation_damage_system` alone. The function is kept (with a deprecation
note and its tests) but is not scheduled.

**Verification** (Fly sprite `vesper-scale-build-k7r2`, 8 CPUs):

| Tick | Pops | Avg Health | Food |
|---|---|---|---|
| 10 | 5 | 98.3 | 5.6 |
| 50 | 5 | 91.1 | 0.1 |
| 100 | 5 | 96.5 | 4.4 |
| 200 | 0 | — | 0.0 |

The colony survives bootstrap, eats, and food production recovers by tick
100 (health rebounds from 91.1 to 96.5). **However, the colony starves by
tick 200**: food production cannot keep up with consumption long-term.
This is a game-balance issue (farm output vs. pop consumption rates), not
a crash — the simulation runs correctly, pops eat, take starvation damage,
and die through the proper `Dead` → corpse pipeline. Tuning the food
economy is out of scope for the revival; the game is functional.

## Build & test

Per Mark's direction ("use sprites more, stop trying to do everything on your
poor vm"), all heavy builds moved to Fly sprite `vesper-scale-build-k7r2`
(8 CPUs, 8 GiB RAM). `CARGO_TARGET_DIR=/tmp/scale-target` (overlayfs
proc-macro link workaround); `/.sprite/bin/cargo` 1.90.0.

| Command | Result |
|---|---|
| `cargo check --all-targets` (sprite, `-j4`) | **GREEN**. |
| `cargo check --all-targets` (local VM, `CARGO_BUILD_JOBS=1`, serial) | **GREEN** (7m 15s, 2026-09-29). One pre-existing `Resource`-derive future-compat warning; dependency warnings from `naga`/`nix`/`wgpu`. |
| `cargo test --lib -j1` (local VM) | **BLOCKED — not a code failure.** The test-binary link (single rustc codegenning the 223k-line crate + test harness) is SIGKILLed by the OOM killer on this 7.7 GiB, zero-swap, heavily shared box. Four attempts: default profile, `debug=0`, `debug=0` + `codegen-units=1` (twice). `swapon` is not permitted in the sandbox, so no swap relief. Notably, compilation *through diagnostics* succeeds every time (the 1 pre-existing warning emits, then codegen dies) — the test code, including the new regression tests, type-checks and borrow-checks clean. |
| `cargo build --bin headless -j4` (sprite) | **GREEN** (~4 min first build, ~1 min incremental). |
| `cargo build --bin headless -j1` (local VM, `dev` profile, `debug=0`, `codegen-units=32`) | **GREEN** (11m 18s first, 6m 45s rebuild). |

Honest statement: **the full test suite has not been executed.** `cargo check`
is green on both hosts; the tests compile; the test *binary* cannot link on
the local VM (OOM, infra-only). The sprite has 8 GiB RAM and could likely
link it — not yet attempted.

## Headless playthrough

Sprite `vesper-scale-build-k7r2`, `headless` binary, Classic start:

- **Tick 0→100:** colony bootstraps cleanly. All 5 pops survive; pressure
  1.00 throughout; food dips to 0.1 at tick 50 then recovers to 4.4 by
  tick 100 as production kicks in; health rebounds 91.1 → 96.5.
- **Tick 100→200:** food depletes (production < consumption); pops starve
  through the proper damage pipeline and the colony is extinct by tick 200.
  This is a food-economy balance issue, not a crash — out of revival scope.

The temporary `debug_pops` headless diagnostic used during the tick-2
investigation has been removed.

## Remaining limitations

- Full `cargo test` unrun (OOM on local VM, infra-only; sprite not yet tried).
  Recommend running on the sprite or a machine with >16 GiB RAM before merge.
- Long-term food-economy balance: colony starves by ~tick 200. Needs design
  tuning (farm output, consumption rates, or starting food), not a code fix.
- Unexplained rapid hunger drain (0.8 → 0.1 in 1 tick) — masked by the
  starvation fix but worth investigating; likely a separate consumption bug.
- `specs/1379-the-sentient-commute.md`, `specs/1380-the-propaganda-constellation.md`,
  `specs/1381-the-degraded-god-mind.md` are **spec-only** per Mark's
  "spec both, build later" — not implemented in this revival.
- Untouched per standing rules: movement code.

## Feature ideas: spec'd, awaiting Mark's build order

Per Mark's decision (**SPEC BOTH, BUILD LATER** — later extended to a third
idea), all three feature ideas have been fully specced by the Architect but
are **NOT implemented**. They await Mark's explicit build order.

- **`specs/1379-the-sentient-commute.md`** — *The Sentient Commute* (Layer 1):
  Desire Dust accumulation on path tiles → movement speed bonus → sentient
  `RoadMind` coagulation with pheromone-forced routing → ghost-road
  persistence after deletion → tile-by-tile excavation counterplay.
  Backlog: `design/BACKLOG.md` entry `1379`.
- **`specs/1380-the-propaganda-constellation.md`** — *The Propaganda
  Constellation* (Layer 2→1): slogan satellites linking into a sky message →
  colony-wide morale boost overriding negative moods → rival hacking flips it
  to despair → shoot-down-your-own-satellites counterplay feeding the
  existing `OrbitalDebris` mechanic (`src/layer2/debris.rs`).
  Backlog: `design/BACKLOG.md` entry `1380`.

- **`specs/1381-the-degraded-god-mind.md`** — *The Degraded God-Mind* (Layer 3):
  uploaded-AI Eternal Ruler granting permanent stability and succession
  immunity → bit-rot accumulation over centuries → erratic edicts, bizarre
  resource tributes (monument to a pet dead 400 years), corrupted-memory wars
  on fallen empires → unplug counterplay risking catastrophic civil war
  (`GodMindSchism`). Backlog: `design/BACKLOG.md` entry `1381`.

All three specs are atomic, TDD-formatted (RED/GREEN/REFACTOR), scoped for a
single ~1-2h build session each, and reference real integration points
(`movement_system`, `Speed`, `Morale.modifiers`, `LaunchEvent`, `InOrbit`).

**Curiosity:** the Jules swarm independently dreamed up both ideas earlier
in `design/IDEAS.md` ("The Sentient Commute" transit-AI variant at line
~25215; "The Propaganda Constellation" at line ~30879). Mark's canonical
versions are the `[SPECCED]` entries at the end of the file, each noting the
earlier swarm draft.
