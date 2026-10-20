# Spec 1387: The Grind — Structural Dissatisfaction (Cyberpunk Misery Engine)

## 1. Overview

Mark's steer (2026-10-20): *"Need the mechanics that make people miserable.
Cyberpunk dissatisfaction."*

This spec builds the **upstream engine** the colony's existing misery
outlets have always been missing: a per-pop **dissatisfaction ledger**
that integrates *structural* conditions — scrip poverty, rough sleeping,
rest denial, diet monotony, visible inequality — slowly, asymmetrically,
and with memory, and feeds the result into the existing `Morale` cache.
Graffiti, bulletin-board grievances, sabotage, mental breaks, stash
hoarding, and black-market demand then fire **as consequences**, through
their existing gates, unchanged.

It is explicitly **not** a retune of the Spec 357 vandalism lane. Mark's
call (2026-10-20): the unreachable vandalism lane is not worth tuning on
its own. This spec makes it reachable by fixing the *cause* upstream.

### 1.1 Why nobody stays miserable today (the forensic core)

Grounded in trunk `d849c80e`. Read these before building:

1. **Morale is a memoryless cache.** `Morale.value` is recomputed from
   scratch every tick by `update_morale_cache_system`
   (`src/layer1/social/morale.rs`) from `calculate_raw_morale`
   (`src/layer1/psychology/memory.rs:245`): base + memories + social +
   policy + trait + modifiers, clamped to `0.0..=1.0`.
2. **The base is instantaneous need satisfaction.** `calculate_raw_morale`
   starts from `Needs::morale()` (`src/layer1/psychology/needs.rs:162`) —
   the plain average of the four *current* needs. The utility AI
   self-services needs (eat/sleep/socialize refill in ~20 ticks), so the
   base snaps back the moment a pop eats or naps. Yesterday's conditions
   do not exist.
3. **The only persistent negative channel is capped and decays.**
   `Memories` (`MAX_MEMORIES_PER_POP = 12`, weakest evicted) carries
   impacts of −0.02..−0.35 that all decay to zero within 100–2000 ticks
   (room thoughts: 100 ticks). A pop in permanently awful conditions
   accrues the same bounded penalty as a pop who had one bad day.
   Nothing integrates.
4. **The one integrator is symmetric.** `StressTracker`
   (`src/layer1/psychology/stress.rs`) gains +1/tick below morale 0.15
   and *loses* −1/tick otherwise; breakdown needs 100 net ticks.
   `check_mental_break_system` (`src/layer1/social/unrest.rs`) likewise
   gates on instantaneous morale < 0.15. Recovery outruns accumulation
   by construction.
5. **Result** (Spec 357 audit, item 111): the vandalism lane of
   `graffiti_placement_system` (`Morale.value < 0.2`, 1%/tick roll) and
   the negative lane of `post_grievance_system`
   (`src/layer1/social/grievances.rs`, `needs.morale() < 0.2`) are
   effectively unreachable in organic play.

### 1.2 The design in one paragraph

Each pop carries a `Dissatisfaction` component: five source channels
that **accrue while their structural condition holds and decay ~20×
slower when it lifts** — so a grievance outlives its cause, and misery
can *compound* (one bad thing is life; three is a breaking pop). A
bridge system maintains exactly one persistent `MoodModifier`
("Ground Down") per pop in `Morale.modifiers`, valued at
`−0.7 × total`. Because `update_morale_cache_system` sums modifiers
*after* raw morale every tick, structural misery now depresses the
cached morale **regardless of how fed and rested the pop is** — which
is precisely the property §1.1 shows the sim lacks. No existing gate,
threshold, or recovery constant is touched.

### Decided (Mark, 2026-10-20 — not open for builder reinterpretation)

- Build the upstream dissatisfaction engine; do **not** retune the
  graffiti/vandalism lane, `StressTracker`, `calculate_raw_morale`, or
  needs decay. Misery wins against recovery *by construction* (a
  persistent modifier summed into the cache), not by weakening recovery.
- Difficulty law stands: **no global softening, no per-scenario
  dissatisfaction discounts.** Relief comes from player counterplay
  (§4.6) and Layer 2 Ready's existing material head start. Classic's
  harsher economy makes the channels bite harder *naturally*.
- Outlets are **consequences through existing systems** (§4.5). Do not
  build parallel graffiti/break/market systems.
- All names original; concept-only IP rule applies (§6 IP-guard test).

## 2. Dependencies

Real seams this spec builds on (all verified on trunk `d849c80e`):

- `Morale` / `MoodModifier` — `src/layer1/social/morale.rs`;
  `update_morale_cache_system` + `morale_decay_system` registered in
  `src/layer1/systems/economy.rs` (~line 161).
- `calculate_raw_morale`, `Memories`, `MemoryType` —
  `src/layer1/psychology/memory.rs` (`memory_weight()` exists).
- `Needs` — `src/layer1/psychology/needs.rs`.
- Pop spawn struct — `src/layer1/entities/pop.rs` (already carries
  `morale`, `memories`, `wallet`, `dietary_history`; new colonists start
  with `Wallet { credits: 50.0 }`).
- `Wallet`, `ColonyPrices { food_price, luxury_price }`,
  `get_wage_for_job` / `pay_wage` — `src/layer1/economy/`
  (`general_work.rs` pays wages through them; corruption skims them,
  Spec 348).
- `consume_food_system` — `src/layer1/agriculture/farm.rs`: already
  filters out hungry pops whose `Wallet.credits < food_price`
  (they silently do not eat), records `DietaryHistory`, picks meal
  flavour from active farms / tamed fauna.
- `DietaryHistory` + `calculate_palette_fatigue` —
  `src/layer1/palette_fatigue.rs`. The fatigue score exists and has
  **zero live callers** today; this spec wires it.
- `Housing { capacity, residents }` — `src/layer1/architecture/housing.rs`;
  room-quality thoughts (`SleptInAwfulRoom` etc.) —
  `src/layer1/architecture/room_quality.rs`.
- `MigrantFamilyInfo { remittance_target }` —
  `src/layer1/economy/remittances.rs` (flavour input to §4.2.1 only).
- Outlets: `graffiti_placement_system` (`src/layer1/graffiti.rs`),
  `post_grievance_system` (`src/layer1/social/grievances.rs`),
  `evaluate_sabotage` (`src/layer1/actions/sabotage.rs`, fed by
  `PopEvalData` assembled in `src/layer1/core/integration.rs`),
  `check_stress_breakdown_system` + `check_mental_break_system`
  (unchanged), `stash_creation_system`
  (`src/layer1/private_stash.rs`), `update_unmet_luxury_system`
  (`src/layer1/core/integration.rs:415`) + `black_market_spawn_system`
  (Spec 348).
- Modifier-refresh precedent: `bonding_mood_system` (Spec 353,
  `src/layer1/fauna/domestication.rs`) — "refreshed, never stacking",
  pinned by test. Copy that pattern exactly.
- Chronicle bridges: `AddChronicleEvent` / `EventImportance`
  (`src/layer1/core/chronicle.rs`); dwell-filtered warning precedent:
  item 99/104 LifeSupport display dwell (`src/layer1/physics/pressure.rs`,
  `src/ui/status.rs`).
- Inspector: pop rendering in `src/ui/inspector.rs`
  (`test_inspector_render_pop_morale` shows the pattern).
- Field Guide: `src/ui/help.rs` (item 103 law: every guide line wraps
  ~2.6× on the WASM viewport — keep additions *short*).
- Headless: `src/bin/headless.rs` (STATS line ~line 3271; panel
  precedent: the `blackmarket` panel ~line 5977).

## 3. RED Phase: Tests First

Write these failing first. Tests 1–3 are the design's contract; test 2
is the acceptance test for the entire spec.

```rust
// src/layer1/social/dissatisfaction.rs (tests in-module, house style)

#[test]
fn test_scrip_squeeze_accrues_while_broke()
    // Live-schedule wiring (item 78 lesson): build the App with the
    // SAME registration function the game uses — not a hand-rolled
    // schedule with only the new system. Pop with Wallet { credits: 0.0 },
    // ColonyPrices { food_price: 1.0, .. }. Run 600 ticks.
    // ScripSqueeze channel > 0.9 (saturated); Morale.value depressed
    // well below the pop's needs-only prediction.

#[test]
fn test_misery_wins_against_recovery()
    // THE test. Pop pinned fed + rested (needs forced to 1.0 every
    // tick by the harness) with three source conditions forced on
    // (broke wallet, no housing, monoculture DietaryHistory).
    // After saturation: Morale.value < 0.35 despite perfect needs.
    // On pristine trunk this is impossible by construction (§1.1).

#[test]
fn test_relief_decays_slowly()
    // Saturate a channel, remove the condition, run 100 ticks:
    // channel retains > 80% of peak. Run 5000 more: eventually < 0.1.
    // Grievances linger; they are not life sentences.

#[test]
fn test_bridge_modifier_never_stacks()
    // Run the bridge 100 ticks: Morale.modifiers contains exactly
    // ONE modifier labelled "Ground Down", duration refreshed.

#[test]
fn test_vandalism_lane_reachable_under_grind()
    // Live schedule incl. graffiti_placement_system + OccupiedTiles.
    // Saturated pop adjacent to a building; iterate like the existing
    // graffiti tests (1000 placement opportunities): at least one
    // GraffitiType::Vandalism marking appears. (Organic baseline
    // from the Spec 357 audit: ~never.)

#[test]
fn test_sabotage_gate_opens_at_bitter()
    // PopEvalData with dissatisfaction_total >= BITTER and
    // is_nostalgic == false, advanced-tech structure in buffer:
    // evaluate_sabotage returns ActionType::Sabotage.
    // Same data below BITTER: returns None.

#[test]
fn test_bitter_pop_counts_as_unmet_luxury()
    // Integration-level: pop with leisure 0.9 and total >= BITTER is
    // counted by update_unmet_luxury_system (drives smuggler spawns).

#[test]
fn test_grievance_board_uses_effective_morale()
    // Pop with needs.morale() == 0.9 but Morale.value depressed by
    // the grind posts a negative bulletin note (post_grievance_system
    // with the §4.5 re-point); a genuinely content pop does not.

#[test]
fn test_fresh_colonist_carries_dissatisfaction()
    // Spawn through the real pop-spawn path (entities/pop.rs):
    // entity has Dissatisfaction::default() (all channels 0.0).
```

Plus the house IP-guard test: no user-facing string in the new module
or UI text names any real-world brand, franchise, or distinctive IP
entity (pattern per recent specs).

## 4. GREEN Phase: Minimal Implementation

### 4.1 The ledger — new module `src/layer1/social/dissatisfaction.rs`

```rust
/// Structural misery sources. Each is a *condition of the colony*,
/// not an event: it accrues while it holds and forgives slowly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrindSource {
    /// Wages don't reach the food price (Wallet vs ColonyPrices).
    ScripSqueeze,
    /// No quarters, or quarters over capacity / awful.
    RoughSleeping,
    /// Rest held low by the work treadmill.
    RestDenial,
    /// Palette fatigue: the same meal forever.
    Monotony,
    /// Watching richer colonists live better.
    RelativeDeprivation,
}

/// Per-pop dissatisfaction ledger (Spec 1387).
#[derive(Component, Debug, Clone, Default)]
pub struct Dissatisfaction {
    /// One channel per GrindSource, 0.0..=CHANNEL_CAP, index by source.
    pub channels: [f32; 5],
    /// Ticks since the last expensive-channel evaluation (§4.2).
    pub eval_cooldown: u32,
}

impl Dissatisfaction {
    pub fn channel(&self, source: GrindSource) -> f32;
    pub fn set_channel(&mut self, source: GrindSource, value: f32);
    /// Weighted colony-visible misery, 0.0..=1.0.
    /// total = clamp01(sum(channels) * TOTAL_WEIGHT)
    pub fn total(&self) -> f32;
    /// Display/Threshold band of the current total.
    pub fn band(&self) -> GrindBand;
    /// The two strongest channels, strongest first (inspector copy).
    pub fn top_sources(&self) -> [Option<(GrindSource, f32)>; 2];
}
```

Constants (starting values — the balance loop owns tuning from here):

| Constant | Value | Meaning |
|---|---|---|
| `CHANNEL_CAP` | 1.0 | per-source saturation |
| `ACCRUE_PER_TICK` | 0.002 | ~500 ticks of a held condition saturates a channel |
| `ACCRUE_DOUBLE` | 2.0× | multiplier for the severe form of a condition |
| `DECAY_PER_TICK` | 0.0001 | 20× slower than accrual — forgiveness takes ~27× longer than the offence |
| `TOTAL_WEIGHT` | 0.34 | one saturated channel ≈ 0.34 total; three ≈ 1.0 |
| `BAND_SIMMERING` | 0.25 | display + status-bar threshold |
| `BAND_BITTER` | 0.55 | outlet threshold (§4.5) |
| `BAND_BREAKING` | 0.85 | display only in v1 (breaks fire via existing stress gates) |
| `MORALE_BRIDGE_COEF` | −0.7 | modifier value = coef × total |
| `EVAL_EPOCH_TICKS` | 20 | cadence for expensive channels (§4.2) |

Band labels for UI: below SIMMERING "Content", then "Simmering",
"Bitter", "Breaking" (`pub enum GrindBand`).

Attach `Dissatisfaction::default()` in the pop spawn struct in
`src/layer1/entities/pop.rs`, alongside `morale` / `wallet` /
`dietary_history` — and at every other pop-spawn site the builder finds
by grepping for the spawn struct's use. Do **not** imitate
`StressTracker`'s ad-hoc insertion (e.g. `social/rumor.rs` inserts it
opportunistically; lanes keyed on it silently skip pops that lack it).
Total coverage is the point; test 9 pins it.

### 4.2 Source channels — `dissatisfaction_accrual_system`

Per-tick, cheap channels (read only the pop's own components):

1. **ScripSqueeze** — needs `&Wallet` + `Res<ColonyPrices>`.
   Accrue at `ACCRUE_PER_TICK` while `credits < 2.0 * food_price`
   ("can't cover the next two meals"); at `ACCRUE_DOUBLE` while
   `credits < 1.0 * food_price`. This is the same affordability line
   `consume_food_system` already enforces silently — a broke pop is
   *already* being skipped at the kitchen; the channel makes the
   squeeze *felt*. (A pop with `MigrantFamilyInfo.remittance_target`
   unmet counts its target as spoken-for: effective credits =
   `credits − remittance_target`, floored at 0, for this check only.
   Sending everything home is a grind, not a savings plan.)
2. **RestDenial** — needs `&Needs`. Accrue while `rest < 0.30`;
   double while `rest < 0.15`. The need refills with one nap; the
   channel remembers the month of nights that led there. (Composes
   with Spec 359 Shift Work when it lands — night-shift assignment
   becomes an accrual input then. 359 is unbuilt; do not depend on it.)
3. **Monotony** — needs `&DietaryHistory`. Accrue while
   `calculate_palette_fatigue(history) > 0.6`. First live caller of
   palette fatigue. Player counterplay already exists: meal flavour
   is drawn from active farms and tamed fauna, so a second crop or a
   tamed herd literally dilutes the channel.

Epoch channels (evaluate every `EVAL_EPOCH_TICKS`, apply ×epoch
accrual/decay — keeps the O(pops × housings) scans off the hot path):

4. **RoughSleeping** — needs housing scan: pop is in no
   `Housing.residents` list (sleeps rough), **or** their housing's
   `residents.len() > capacity`, **or**
   `memories.memory_weight(SleptInAwfulRoom) + memory_weight(SleptInDullRoom) >= 0.5`.
   Any one condition accrues the channel (double if two or more).
5. **RelativeDeprivation** — needs colony wallet median: sample all
   pops' `Wallet.credits` (+ own-stash value via `PrivateStash` where
   carried). Accrue while `own < 0.5 * median` **and** `median > 0`
   (a colony where everyone is broke is not envious; it is
   ScripSqueezed — different channel, different fix). Decay normally
   otherwise. This channel is *perception*, so it must sample, not
   stream: median computed once per epoch, stored in a small
   `ColonyWalletMedian` resource the system maintains.

When a channel's condition does **not** hold, the channel decays at
`DECAY_PER_TICK` (×epoch for epoch channels). Channels never go
negative, never exceed `CHANNEL_CAP`.

### 4.3 The morale bridge — `dissatisfaction_bridge_system`

For each pop with `(&Dissatisfaction, &mut Morale)`:

- Find the modifier labelled `"Ground Down"` in `morale.modifiers`.
- If `total > 0.01`: insert-or-refresh it in place — `value =
  MORALE_BRIDGE_COEF * total`, `duration = 100` (refreshed every tick,
  so it never lapses while the ledger says miserable; the exact
  refresh-never-stack pattern of Spec 353's `bonding_mood_system`).
- If `total <= 0.01`: remove it if present.

Registration: add both new systems in `src/layer1/systems/economy.rs`
beside `update_morale_cache_system`, with accrual **before** the cache
update and the bridge **before** the cache update and **after**
accrual (`.before(update_morale_cache_system)`), so a condition that
starts this tick already casts its shadow this tick. No changes to
`update_morale_cache_system`, `morale_decay_system`, or
`calculate_raw_morale`.

Also write real `Memories` at band crossings (once per crossing,
re-armed on dropping a full band below): crossing into Bitter adds
`MemoryType::GroundDown` (new variant, `base_mood_impact` −0.1,
slow decay 0.0005 — it is a summary of the ledger, deliberately small:
the bridge does the heavy lifting; the memory makes the misery
*legible* in existing memory views and outlives the ledger's decay
tail). Add the variant at the end of the enum; do not renumber
existing variants.

### 4.4 Thresholds are read, not re-implemented

`Dissatisfaction::total()` and `band()` are the only new public
surface the rest of the game reads. Existing thresholds
(morale < 0.2 vandalism, < 0.15 stress/break) stay exactly where they
are — §4.3's bridge moves pops across them.

### 4.5 Outlet wiring (consequences — small, named diffs only)

1. **Graffiti / vandalism: NO CHANGE.** Reachability is restored by
   §4.3 alone (test 5 proves it). If the builder is tempted to touch
   `graffiti.rs`, stop — that is the retune Mark declined.
2. **Bulletin grievances: one re-point.**
   `post_grievance_system` (`src/layer1/social/grievances.rs`)
   currently gates on `needs.morale()`. Add `Option<&Morale>` to its
   query and use `morale.value` when present, falling back to
   `needs.morale()` — the exact fallback pattern
   `check_stress_breakdown_system` already uses. A ground-down pop
   now complains on the board even on a full stomach.
3. **Sabotage: second gate on the existing evaluator.**
   `evaluate_sabotage` (`src/layer1/actions/sabotage.rs`) today fires
   only for `is_nostalgic` pops. Add `dissatisfaction_total: f32` to
   `PopEvalData` (`src/layer1/mind/utility_eval_types.rs`, default
   0.0 in `test_instance()`), populate it where eval data is
   assembled (`src/layer1/core/integration.rs`), and extend the
   evaluator: a pop with `total >= BAND_BITTER` evaluates Sabotage
   against the nearest advanced-tech structure with utility scaled
   `60.0 + 40.0 * total` (below the 100.0 of an active breakdown —
   sabotage is cold, breaks are hot). Nostalgia lane unchanged.
4. **Black-market demand: misery counts as unmet.**
   `update_unmet_luxury_system` (`src/layer1/core/integration.rs:415`)
   counts pops with leisure < 0.3. Add: **or** `Dissatisfaction`
   total ≥ `BAND_BITTER`. Misery seeks relief; the smuggler
   (Spec 348) is the relief that bills you in corruption. No change
   to spawn caps, cooldowns, or trade.
5. **Stash hoarding: insecurity path.**
   `stash_creation_system` (`src/layer1/private_stash.rs`) keeps its
   shortage path untouched. Add a parallel trigger: pop total ≥
   `BAND_BITTER`, colony food ≥ 1.0, throttle
   `STASH_MISERY_CHANCE = 0.005`/tick, no trait requirement (the
   existing path keeps its Anxious/Hoarder gate). A bitter pop
   quietly starts keeping food back *before* the shortage — which
   then helps cause one. That feedback loop is the design working.
6. **Mental breaks / stress: NO CHANGE.** With morale durably
   depressed, `StressTracker` finally integrates (its −1/tick drain
   only wins when morale actually recovers — which now requires
   fixing a *condition*, not a nap). State this in the module docs.

### 4.6 Counterplay (existing levers only — name them in the guide)

- **ScripSqueeze:** keep food stocked so wages stretch; reduce
  corruption (it skims every wage — Spec 348); research/industry that
  raises job wages is downstream of existing tech. (Direct wage
  policy is *not* a v1 lever — see §9.)
- **RoughSleeping:** build housing; keep occupancy ≤ capacity; raise
  room quality (beauty/furnishings via `room_quality.rs` inputs).
- **RestDenial:** more pops per job, leisure venues (taverns/libraries
  already in the utility buffer), not running every pop at the
  treadmill's edge.
- **Monotony:** second crop type, tamed-fauna meat (Spec 353),
  trade imports — anything that varies `available_items` at the
  kitchen.
- **RelativeDeprivation:** broad wage base; room searches recover
  stashes but at the existing colony-wide morale penalty
  (`SEARCH_ROOMS_MORALE_PENALTY`) — a real, nasty trade-off that
  already exists. Cite it; don't soften it.

### 4.7 Telegraphing (legibility over softening)

- **Pop inspector** (`src/ui/inspector.rs`): when a pop has a
  `Dissatisfaction` with total ≥ 0.05, render a block under the
  morale readout: band label + the top-2 channels in plain language:
  - ScripSqueeze: `Can't cover the next meals`
  - RoughSleeping: `Sleeps rough` (or `Barracks over capacity`)
  - RestDenial: `Running on empty`
  - Monotony: `Paste again. Paste forever.`
  - RelativeDeprivation: `Watches richer colonists eat`
- **Status bar:** a dwell-filtered segment, following the item 104
  discipline exactly (ON-dwell 20 ticks, linger 30 — reuse the
  pattern, do not invent an ad-hoc flicker): `⚠ {n} colonists
  simmering` appears only while ≥ 20% of pops are ≥ SIMMERING, and
  owns its tail slot while visible (item 104's deterministic-tail
  rule).
- **Chronicle:** first pop crossing into Bitter → Standard entry
  (`THE GRIND: {name} is coming apart quietly — {top source line}.`);
  ≥ 25% of pops Bitter-or-worse → Major entry, once per crossing,
  re-armed when the share falls below 10% (item 99 escalation
  pattern).
- **Field Guide** (`src/ui/help.rs`): one SHORT section,
  "THE GRIND", ≤ 6 lines of copy: misery is structural, it lingers,
  the inspector names each pop's top grievances, the counterplays
  are housing / variety / wages-that-reach. (Item 103: lines wrap
  ~2.6× on WASM. Brevity is a correctness property here.)

### 4.8 Instrumentation (the balance loop must be able to see it)

- **STATS** (`src/bin/headless.rs`): append
  `dissat={avg:.2}/{max:.2} miserable={} bitter={}` — colony mean
  total, max total, pops ≥ SIMMERING, pops ≥ BITTER.
- **Headless panel `misery`:** per-source colony mean channel values,
  band counts, and the top-5 most miserable pops (id, total, top
  source label). Follow the `blackmarket` panel's shape.
- **Headless levers for bots** (add if absent — check the command
  dispatch first): `setprice food <f32>` writes
  `ColonyPrices.food_price`; `drainwallets` zeroes every
  `Wallet.credits`. With these two, a balance script can manufacture
  a squeeze deterministically (start L2R, `drainwallets`, `tick
  1000`, read `miserable=` / `bitter=` / `graffiti=` / `sabotaged=`)
  instead of hoping organic economies wander there.
- Existing counters (`graffiti=`, `sabotaged=`, `smugglers=`,
  `stashes=`) are the outlet evidence; do not duplicate them.

### 4.9 Scenario notes

No per-scenario constants. Expect, and say in the module docs:
Classic colonies (thin wages, rough starts) will simmer earlier;
Layer 2 Ready's material head start (housing, stocked food, drones
freeing pop labour) is the sanctioned relief — the *conditions*
differ, the engine doesn't.

## 5. REFACTOR Phase: Quality & Design

- Module docs in `dissatisfaction.rs` must carry §1.1's mechanism in
  brief (why the ledger exists) and §4.5.6's note (breaks unchanged),
  so the next agent doesn't "helpfully" retune recovery.
- All thresholds/rates are named constants in the one module —
  no literals in systems/economy.rs, integration.rs, or the UI.
- `GrindSource` gets a `label()` for inspector/panel copy; UI never
  matches on the enum for text.
- Perf: per-tick systems touch only the pop's own components;
  housing/median scans run on the epoch. State the epoch in docs.

## 6. Acceptance Criteria

- [ ] All §3 tests pass, including test 2 (misery beats recovery)
      and test 5 (vandalism reachable in the live schedule).
- [ ] Full-schedule proof: tests register systems through the game's
      real registration path (item 78 lesson — direct-call-only
      tests do not count for tests 1, 2, 5).
- [ ] Headless evidence in the builder report: a scripted squeeze
      (`drainwallets` + ticks on a starting colony) shows
      `miserable=` > 0 and, over enough ticks/seeds, `graffiti=` or
      `sabotaged=` movement vs the ~zero organic baseline.
- [ ] `cargo check --all-targets` green; clippy clean in changed
      code; full lib suite vs the known pre-existing failure pool.
- [ ] No changes to: `calculate_raw_morale`, needs decay constants,
      `StressTracker` rates/thresholds, graffiti placement chances,
      mental-break thresholds, black-market caps/cooldowns.
- [ ] IP-guard test passes; all player-facing copy is original.

## 7. Technical Guidance

- `Morale.modifiers` is a `Vec<MoodModifier>`; find-by-label and
  mutate in place. `morale_decay_system` prunes `duration == 0` —
  refreshing duration to 100 every tick keeps the bridge modifier
  alive without touching the decay system.
- Beware the query in `update_morale_cache_system`: it reads
  `&Morale` modifiers through `calculate_raw_morale(..., None)` for
  the morale argument but sums `morale.modifiers` separately in the
  system body — the bridge modifier flows through that sum. Verify
  with test 1 rather than assuming.
- `Option<&Morale>` fallback pattern for §4.5.2 already exists in
  `check_stress_breakdown_system` — copy its shape.
- Median: collect wallet values into a `Vec`, sort, take the middle;
  colonies are small — do not build a streaming estimator.
- RNG: the misery paths in §4.5.5 use `rand::thread_rng()` per the
  codebase's existing convention (stash/graffiti systems); the
  ledger itself is deterministic — no RNG in accrual/decay/bridge.
- Simulation determinism for tests: drive ticks through the schedule,
  not wall-clock helpers.

## 8. Questions

- Should `RelativeDeprivation` include carried-stash *food* value in
  the wealth comparison, or credits only? Spec defaults to credits +
  stash total value where a `PrivateStash` component is carried on
  the pop; standalone stash entities are excluded in v1 (ownership
  resolution is an extra join). Revisit if the channel under-fires.
- `MemoryType::GroundDown` impact −0.1 is deliberately small next to
  the bridge's −0.7×total. If playtesting shows double-counting
  harshness, the memory — not the bridge — is the knob.

## 9. Out of Scope (deferred, named so nobody reinvents them)

- **Permit/bureaucratic friction as a source.** Permits today gate
  *buildings* (`PermitRequired`, `src/layer1/administration/permit.rs`),
  not pops. A pop-facing friction channel needs denial events that
  don't exist yet — its own spec.
- **Unkept colony promises.** There is no promise/commitment ledger;
  `ColonyPolicies` are static toggles. Promises ("housing by winter")
  need a commitments system first — its own spec.
- **Surveillance pressure.** No pop-level surveillance signal exists
  to integrate.
- **Shift-grind proper.** Awaits Spec 359 (Shift Work); §4.2.2 leaves
  the seam (RestDenial accrual input) and nothing more.
- **Wage policy / debt relief UI.** Wages are code-level
  (`get_wage_for_job`); a player-facing wage or scrip-relief edict is
  an economy-UI spec, not part of this engine.
- **Retuning recovery.** `StressTracker` symmetry, memory decay, and
  needs rates are untouched by design (§1.2 Decided). If the balance
  loop later wants recovery itself revisited, that is a separate,
  evidence-led proposal — with a balance-matrix re-run first, per
  the standing sequencing rule for morale-adjacent constants.
- **Desire dust.** It is road-wear (Spec 1379), not a coping
  commodity; the misery-economy outlets are the black market,
  stashes, and sabotage above.

## 10. Open Questions for Mark

Defaults are decided below so the builder is never blocked; veto any
of them and the constants move, the architecture doesn't.

1. **Sabotage at Bitter (§4.5.3)** — default: ON. A bitter pop
   cold-sabotaging advanced tech is the cyberpunk heart of this
   spec; the utility (60 + 40×total) keeps it rarer than breakdowns.
2. **Status-bar simmering segment (§4.7)** — default: ON at ≥20%
   simmering share, dwell-filtered. It is a warning you act on
   (housing/food/variety), not ambient noise.
3. **Forgiveness speed** — default: decay at 1/20th of accrual.
   Grievances should outlive a fixed condition by days, not
   seasons; the balance loop can argue with data either way.
