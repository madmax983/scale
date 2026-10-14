# Spec 1384: EVA Suits — Timed Air for the World Outside the Hull

## 1. Overview

Colonies do not die inside the hull. They die outside it.

The Atmosphere Forensics investigation (Item 77 Phase 1,
`hidden_files/atmosphere-forensics-report.md`) proved the dominant death
chain: every tile beyond the starter hull is hard vacuum (0.0 atm), pops
take 1 HP/tick below 0.2 atm against a 100 HP budget, and ordinary colony
life — excursions, salvage, exterior work, and above all panic cascades —
keeps pops outside for **80–200 ticks** at a time. Phase 2
(`hidden_files/atmosphere-phase2-report.md`) added the telegraphing
(suffocation alert, colony pressure readout, Field Guide) and the
shelter/recall order, and its counterplay audit recorded the missing
verb verbatim: **"EVA suits / oxygen equipment — Absent. No vacuum
protection exists."** This spec is that verb.

An EVA suit is equippable body-slot gear with a **timed air supply**.
While a worn suit has air, vacuum exposure drains the suit's air instead
of the pop's HP. When the bottle runs dry, the existing suffocation rules
resume, unchanged. Air refills in pressurized space. Suits are physical,
scarce, fabricable, and upgradeable through the tech tree. Nothing about
Classic's vacuum gets softer: 1 HP/tick, the 0.2 atm threshold, and
random-direction panic flight all stay exactly as decided. A suit
converts an invisible death sentence into a **planned round trip with a
visible fuel gauge** — counterplay, not mercy, per Mark's doctrine that
fairness work goes to telegraphing and counterplay.

### Decided (Mark, 2026-10-14 — not open for builder reinterpretation)

- **Timed air is the mechanic.** One tick = one day (status bar renders
  `Day {tick}`; `src/ui/status.rs`). Air is measured in days.
- **The ladder is 50 / 100 / 200 days.** Base suit: **50 days** of air.
  Two tech-tree research nodes raise newly issued suits to **100 days**
  (Mk II) and **200 days** (expedition class).
- **Correction on record:** Mark's first numbers (base 5 days, max 20)
  were **rejected by Mark himself the same day** as too small at play
  speed: Normal speed runs ~10 ticks per real second
  (`src/shared/time.rs`, SimSpeed::Normal ≈ 10 tps), so 5 days of air is
  half a second of gameplay, and the fatal exposures measured by
  forensics ran 80–200 ticks. The 50/100/200 ladder is sized against
  that evidence: **base = errands and emergency dashes** (a 50-day
  bottle covers a short trip out and back, not a work shift);
  **exterior designation work wants upgraded bottles** (100 days covers
  a work site with margin); **expedition = real excursions** (200 days
  covers the longest measured exposure spans).
- **Migrants arrive wearing expedition-class suits**, bottles
  **partially drained by their journey** — remaining air reflects the
  trip. Their suits join the colony pool when doffed (see §4.6).
- **Scenario dial (difficulty law):** Classic gets **few suits, base
  class**. Layer 2 Ready gets a **full rack of long bottles** — the
  sanctioned relief lane stays a *maintenance* advantage, not a
  softening of the sim.
- **Telegraphing doctrine applies to air itself:** the chronicle warns
  when a suited pop's air runs low and again when it runs dry, before
  the existing suffocation alert ever needs to fire.

## 2. Dependencies

- Equipment & clothing (Spec-era core): `Equipment` body slot,
  `Clothing`, `clothing_wear_system` — `src/layer1/economy/items.rs`,
  `src/layer1/clothing.rs`
- Fetch-clothing flow (the pattern this spec mirrors end-to-end):
  `evaluate_fetch_clothing` (`src/layer1/actions/clothing.rs`),
  `handle_fetch_clothing` (`src/layer1/execution/arrival.rs`),
  Tailor refining (`src/layer1/economy/refining.rs`)
- Pressure & suffocation: `pressure_damage_system`,
  `SUFFOCATION_PRESSURE_THRESHOLD` (`src/layer1/physics/pressure.rs`)
- Tech tree: `Tech` enum + `TechState` (`src/layer1/tech/mod.rs`)
- Start scenarios: `src/shared/scenario.rs`, grants applied in
  `src/setup.rs` (`apply_layer2_ready_start` etc.)
- Migrant bridge INT-762: `beacon_migrant_arrival_bridge`
  (`src/layer1/core/integration.rs`)
- Private Stashes (Spec 350): interplay only, see §7
- Save System (Spec 1382): `src/persistence/mod.rs` conventions

## 3. RED Phase: Tests First

Write these failing first. Unit tests call systems directly; the two
schedule-level tests at the end exist because Item 78 taught this repo
that direct-call tests can hide an unregistered system for years — the
air systems must be proven in the **real** schedule.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    // --- Air drain / protection ---

    #[test]
    fn test_suited_pop_in_vacuum_drains_air_not_hp() {
        // Arrange: pop in vacuum (pressure grid all 0.0) wearing a
        // base suit with 50.0 air; Health 100.
        // Act: run pressure_damage_system once.
        // Assert: suit air == 49.0; health == 100.0.
    }

    #[test]
    fn test_dry_suit_resumes_suffocation() {
        // Arrange: suited pop in vacuum, suit air == 0.0.
        // Act: run pressure_damage_system once.
        // Assert: health == 99.0 (existing 1 HP/tick path untouched)
        // and the pop carries SuffocationAlerted after the tick.
    }

    #[test]
    fn test_air_refills_in_pressurized_space_and_caps() {
        // Arrange: suited pop (air 48.0 / capacity 50.0) on a 1.0 atm tile.
        // Act: run suit air system for 10 ticks.
        // Assert: air == 50.0 exactly (refill never exceeds capacity).
    }

    #[test]
    fn test_air_low_and_dry_chronicle_fire_once_per_excursion() {
        // Arrange: suited pop in vacuum, air at 26% of capacity.
        // Act: run until air crosses 25%, then to 0, then 5 more ticks.
        // Assert: exactly one "suit air is low" Major event and exactly
        // one "suit has run dry" Major event in the chronicle queue;
        // no repeats while the excursion continues.
    }

    // --- Tiers & tech ---

    #[test]
    fn test_suit_capacity_follows_issue_tier() {
        // Base (no tech): 50. ClosedCycleBreathing unlocked: 100.
        // ExpeditionSystems unlocked: 200. (SuitClass::capacity via
        // the colony's current issue tier.)
        assert_eq!(issue_capacity(&TechState::default()), 50.0);
    }

    #[test]
    fn test_research_does_not_retrofit_worn_suits() {
        // A suit issued at base tier keeps capacity 50 after
        // ClosedCycleBreathing is researched. Bottles are physical.
    }

    // --- Don / doff ---

    #[test]
    fn test_fetch_suit_equips_from_pool_and_decrements() {
        // Arrange: ColonyResources.eva_suits == 2; unsuited pop arrives
        // at a stockpile on a FetchSuit action.
        // Act: run arrival_handler_system.
        // Assert: pool == 1; pop Equipment.body is an EvaSuit entity
        // with full air at the colony's issue tier; any prior ordinary
        // clothing in the body slot was despawned (FetchClothing
        // upgrade semantics).
    }

    #[test]
    fn test_doff_on_interior_arrival_returns_colony_suit_to_pool() {
        // Arrange: pop wearing a colony-issued suit arrives at housing
        // (pressurized tile) completing SatisfyRest.
        // Act: run arrival_handler_system.
        // Assert: pool incremented by 1; Equipment.body cleared.
    }

    #[test]
    fn test_fetch_suit_scored_only_when_exterior_work_pends() {
        // evaluate_fetch_suit returns Some(score >= 0.9) when a Work or
        // Repair designation in the evaluation buffers targets a tile
        // below SUFFOCATION_PRESSURE_THRESHOLD and the pool/loose suits
        // exist; None when all pending designations are interior, when
        // the pop already wears a suit, or when no suit exists anywhere.
    }

    #[test]
    fn test_panicked_pop_is_never_assigned_fetch_suit() {
        // A pop with the Panic component keeps panic movement; the
        // utility AI does not pick it up (panic_spirals contract).
    }

    // --- Migrants, scenarios, death ---

    #[test]
    fn test_migrant_arrives_in_partially_drained_expedition_suit() {
        // Drive beacon_migrant_arrival_bridge with one MigrantArrivalEvent.
        // Assert: the spawned pop wears an Expedition EvaSuit with
        // 200 - 60 <= air <= 200 - 30 (transit drain band, §4.6).
    }

    #[test]
    fn test_scenario_suit_grants() {
        // Classic: pool == 3. Ground Survival: pool == 2.
        // Social Drama: pool == 3. Layer 2 Ready: pool == 6 and the
        // scenario bottle floor is Mk II (issue tier >= 100 in L2R
        // before any research).
    }

    #[test]
    fn test_death_does_not_delete_suit() {
        // Colony suit: wearer's death returns +1 to the pool.
        // Expedition suit: wearer's death leaves a loose EvaSuit entity
        // at the death tile with its air preserved.
    }

    // --- Persistence ---

    #[test]
    fn test_save_load_round_trips_pool_worn_and_loose_suits() {
        // Snapshot a world with pool == 2, one pop wearing a Mk II
        // suit at 63.0 air, one loose expedition suit at 141.0 air.
        // Capture -> apply -> capture must be identical, and the
        // restored pop still wears a 63.0-air Mk II suit.
    }

    // --- Schedule-level (real Layer1 schedule, no direct calls) ---

    #[test]
    fn test_schedule_suited_pop_survives_30_ticks_of_vacuum() {
        // Build the world through the real schedule setup (pattern:
        // src/layer1/execution/tests/schedule_work_tests.rs). Suited
        // pop stands in vacuum 30 ticks: air 50 -> 20, HP 100 -> 100.
        // Unsuited control pop: HP 100 -> 70.
    }
}
```

## 4. GREEN Phase: Minimal Implementation

### 4.1 Components & constants (new module `src/layer1/eva_suit.rs`)

```rust
/// Class of an EVA suit. Capacity is stamped at issue; research never
/// retrofits an existing bottle (see test_research_does_not_retrofit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuitClass { Base, Mk2, Expedition }

impl SuitClass {
    /// Air capacity in days (1 tick = 1 day). Mark's ladder, 2026-10-14.
    pub const fn capacity(self) -> f32 {
        match self { Self::Base => 50.0, Self::Mk2 => 100.0, Self::Expedition => 200.0 }
    }
}

/// A worn or loose EVA suit (an item entity, like Clothing).
#[derive(Component, Debug, Clone)]
pub struct EvaSuit {
    pub class: SuitClass,
    /// Days of air remaining. Drains in vacuum, refills in air.
    pub air: f32,
    /// Follows clothing wear (clothing_wear_system rates).
    pub durability: f32,
    pub max_durability: f32,
}

pub const EVA_AIR_DRAIN_PER_TICK: f32 = 1.0;   // mirrors 1 HP/tick: a day out costs a day of air
pub const EVA_AIR_LOW_FRACTION: f32 = 0.25;    // chronicle warning threshold
pub const EVA_REFILL_FRACTION_PER_TICK: f32 = 0.1; // full refill in ~10 ticks indoors
```

A suit occupies the **`body` slot** of `Equipment`
(`src/layer1/economy/items.rs`). It is mutually exclusive with ordinary
`Clothing` and with the `BioSuit` (`src/layer1/economy/bio_loom.rs`),
exactly as those two already are with each other. Colony-issued suits
are tracked as an abstract pool — `ColonyResources.eva_suits` /
`max_eva_suits`, mirroring `clothing` / `max_clothing` field-for-field —
and materialize into `EvaSuit` entities on fetch, mirroring how
`tools` materialize via `handle_fetch_tool` and `clothing` via
`handle_fetch_clothing`. Loose physical suits (migrant gear, death
drops) are `EvaSuit` entities in the world and are never pooled (§4.6).

### 4.2 Air system — the hook into suffocation

One new system, `suit_air_system`, registered in the real Layer 1
schedule **before** `pressure_damage_system`
(`src/layer1/physics/pressure.rs`), plus a narrow change inside
`pressure_damage_system` itself:

- For each pop on a tile below `SUFFOCATION_PRESSURE_THRESHOLD` (0.2)
  wearing a suit with `air > 0.0`: drain `EVA_AIR_DRAIN_PER_TICK`. The
  pop takes **no** pressure damage this tick and does **not** gain
  `SuffocationAlerted`.
- Implementation shape: `pressure_damage_system`'s query gains the
  pop's `Equipment` and access to `EvaSuit`; when a live bottle covers
  the tick, the damage branch is skipped. The drain lives in the same
  branch decision (one system, one source of truth for "was this tick
  covered") — do not build a parallel marker pipeline that can drift
  from the damage path.
- Refill: a suited pop on a tile at or above the threshold regains
  `capacity * EVA_REFILL_FRACTION_PER_TICK` per tick, capped at
  capacity. Loose (unworn) suits do not drain or refill in v1.
- Wear: extend the body-slot wear path (`clothing_wear_system`,
  `src/layer1/clothing.rs`, 0.05 durability/tick) to `EvaSuit`
  durability. A suit that reaches 0 durability breaks with the existing
  semantics (entity despawned, slot cleared); if the break happens
  below the pressure threshold, chronicle it Major:
  `"{name}'s suit has torn beyond use."` A pop who lives in their suit
  wears it out in ~2000 ticks, same as clothing — doffing (§4.4) is the
  counterplay.

### 4.3 Chronicle telegraphs (awareness doctrine)

Mirror the `SuffocationAlerted` pattern — a `SuitAirAlerted` marker,
once per pop per excursion, re-armed when the suit refills above the
low threshold:

- Air crosses **below 25%** of capacity while in vacuum (Major):
  `"{name}'s suit air is low — {days} days of air left."`
  (`{days}` = remaining air, rounded down.)
- Air reaches **0** while in vacuum (Major):
  `"{name}'s suit has run dry."` — after which the existing
  suffocation alert and damage flow take over naturally.

### 4.4 Don & doff — the utility hook (smallest honest seam)

Suits are donned **on purpose, before exposure**, by reusing the
fetch-clothing shape end-to-end:

- New `ActionType::FetchSuit` (`src/layer1/mind/utility_types.rs`).
- New evaluator `evaluate_fetch_suit` beside `evaluate_fetch_clothing`
  (`src/layer1/actions/clothing.rs`), considered in
  `evaluate_group_logistics` (`src/layer1/mind/utility_ai.rs`, next to
  the FetchClothing evaluation at ~:690). It scores **0.97** (above
  Work's 0.5, in the FetchClothing 0.95 family) when **all** hold:
  the pop wears no `EvaSuit`; a suit is obtainable (pool > 0, or a
  loose suit entity exists — loose expedition suits are targeted
  first, they are strictly better and already physical); and an
  exterior commitment pends for this pop: a Work or Repair designation
  in the evaluation buffers (`buffer.work_designations`,
  `buffer.repair_designations`) whose target tile is below the
  pressure threshold, or an Explore candidate below threshold, **or**
  the pop is already standing below threshold unsuited (emergency
  dash home via the nearest suit — the evaluator targets the suit,
  the pop still has to survive the walk to it; that is intended).
- Execution mirrors `handle_fetch_clothing`
  (`src/layer1/execution/arrival.rs`, `ActionType::FetchSuit` arm in
  `arrival_handler_system`): on arrival at the stockpile, consume 1
  from the pool and spawn a full `EvaSuit` at the colony's **issue
  tier** (§4.5); on arrival at a loose suit, equip that entity. Any
  ordinary clothing in the body slot is despawned (the existing
  upgrade semantics); the `BioSuit` fused-host check applies verbatim
  (attachment > 75 → `UnequipFailedEvent`, don fails — a fused BioSuit
  wearer cannot EVA in v1).
- **Doff** is the symmetric arrival rule in `arrival_handler_system`:
  when a suited pop completes **any** action on a tile at or above the
  pressure threshold, the suit comes off — colony-issued suits return
  to the pool (+1), expedition suits drop as loose entities at the
  arrival tile. A suited pop whose action completes outdoors keeps the
  suit. Pops therefore dress to go out and undress when they come home,
  with no new AI state machine.
- **Panic:** panicked pops are never assigned `FetchSuit` — the
  `Panic` component (`src/layer1/psychology/panic_spirals.rs`) already
  removes pops from utility evaluation ("Panicked pops abandon work:
  the utility AI never picks them up again"). **A panicking pop will
  not stop to dress. That is intended cruelty, not a bug.** A pop who
  is already suited when panic takes them keeps the suit, and its air
  may save them; the doff-on-arrival rule does not apply to panic
  movement (panic is not action arrival), and the Phase 2 shelter
  order already excludes panicking pops for the same reason.
- **Direct control:** a possessed pop (`DirectControlState`,
  `src/layer1/direct_link.rs`) gets the physics and nothing else —
  air drains, refills, warns, and runs dry by the same rules; no
  auto-don, no auto-doff (the Phase 2 shelter order sets the
  precedent: the possessed pop is the player's own hands). There is
  no manual don/doff verb in v1 (see Out of Scope).

### 4.5 Tech tree — two new nodes

The tree is a **flat enum, tiered by Knowledge cost, with no hard
prerequisites** (`src/layer1/tech/mod.rs`: "Technologies are flat (no
hard dependencies) but tiered by cost"). The ladder honors that: the
second node does not hard-require the first; its cost is the gate.
Effects are read via `TechState::is_unlocked(Tech::…)` at the point of
use, per house convention (cf. `Tech::Astronomy` gating in
`src/layer1/disasters/deorbit.rs`).

| Variant | Label | Cost (Knowledge) | Storage | Effect |
|---|---|---|---|---|
| `ClosedCycleBreathing` | "Closed-Cycle Breathing" | 40.0 (Medical tier) | 20 TB | Newly issued suits are **Mk II — 100 days** |
| `ExpeditionSystems` | "Expedition Systems" | 75.0 (Terraforming tier) | 30 TB | Newly issued suits are **Expedition — 200 days** |

Descriptions in house voice: Closed-Cycle Breathing — *"Recyclers,
scrubbers, and fat bottles: newly issued EVA suits carry 100 days of
air."* Expedition Systems — *"Full expedition rigs built for the long
dark: newly issued EVA suits carry 200 days of air."*

The **issue tier** is a pure function of colony state: the highest of
(the scenario bottle floor, §4.7) and (Expedition if
`ExpeditionSystems` unlocked, else Mk II if `ClosedCycleBreathing`
unlocked, else Base). It is consulted at fetch time (pool suits are
unstamped shells; bottles are fitted at issue) and at fabrication
time is irrelevant (the pool is classless). Research never changes a
suit already worn or loose.

### 4.6 Fabrication & migrants

- **Fabrication:** the **Tailor** gains a second refining recipe in
  the refining table (`src/layer1/economy/refining.rs`, beside the
  existing Tailor cloth→clothing entry): **2 Cloth + 1 Metal → 1 EVA
  suit** into `ColonyResources.eva_suits`, capped by `max_eva_suits`.
  Recipe selection: when metal ≥ 1 and the pool is below cap, the
  Tailor produces suits; otherwise it falls back to clothing. (If the
  single-entry-per-building table cannot express this, the builder
  adds the alternate-output branch in the Tailor arm — the behavior
  above is the spec, the table shape is an implementation detail.)
  Suits are deliberately *not* cheap: metal is the same resource a
  spare LifeSupport wants, and that tension is the game.
- **Migrants:** in `beacon_migrant_arrival_bridge`
  (`src/layer1/core/integration.rs`, INT-762), each spawned migrant
  additionally receives an `Expedition` `EvaSuit` in the body slot
  (replacing any bundle clothing, per §4.4 equip semantics) with
  `air = 200 − transit`, where `transit` is drawn per migrant from
  **30..=60 days**. Grounding honesty: on current trunk, beacon
  migrants spawn on a colony tile inside the habitat (the bridge's
  own fix for vacuum spawn deaths), so no literal walk-in exists to
  drain the bottle today; the transit drain models the off-screen
  approach the fiction requires (forensics measured rim-corridor
  exposure clusters at Chebyshev distance 20–24 from the habitat on
  the 80×50 map, at ~1 tile/tick movement). If a future spec restores
  rim spawning for migrants, the same suit drains naturally en route
  and the modeled drain should be removed. Rim **Visitors**
  (`spawn_visitor_system`, `src/layer1/entities/visitor.rs`) are not
  pops — no Health, cannot suffocate — and receive no suits in v1.
  A migrant's suit joins colony circulation on doff (§4.4): it drops
  as a loose expedition suit, fetchable by anyone, forever physical.

### 4.7 Scenario grants (in `src/setup.rs` scenario application)

Pool counts at world creation, beside the existing resource grants:

| Scenario | Suits (pool) | Bottle floor |
|---|---|---|
| Classic (Standard) | **3** | Base (50) |
| Ground Survival (Hard) | **2** | Base (50) |
| Social Drama (Medium) | **3** | Base (50) |
| Layer 2 Ready (Easy) | **6** | **Mk II (100)** |

Layer 2 Ready's floor is implemented as a scenario-set issue-tier
floor consulted by §4.5's pure function, set inside
`apply_layer2_ready_start` alongside its existing maintenance-lane
grants (redundant LifeSupport, Generator, metal earmark). Classic,
Ground Survival, and Social Drama numbers elsewhere are untouched.

### 4.8 Observability (Phase 2 conventions)

- New `ColonySuitStatus` resource refreshed per tick beside
  `update_colony_atmosphere_system`: pool count, worn count, and the
  lowest air among suited pops currently below the pressure
  threshold.
- Headless **STATS** gains trailing fields (the Phase 2 pattern):
  `suits={pool} worn={worn} suit_air={lowest-air-in-vacuum or "none"}`.
- Headless **`suits`** panel command, mirroring the `stashes` panel
  from Spec 350: pool count, issue tier, and one line per worn/loose
  suit (wearer or position, class, air remaining).
- Headless **`probe`** state rows gain the same suit fields.
- Shell status bar: extend the Phase 2 atmosphere segment
  (`build_atmosphere_spans`) — when a suited pop in vacuum is below
  the low-air fraction, render `⚠ suit air {days}d`; silent otherwise,
  per the Phase 2 discipline (the bar speaks only when someone is in
  danger).
- Field Guide: extend the existing **"THE AIR IS THE HULL"** section
  (`src/ui/help.rs`, added Phase 2) with a suits paragraph: pops dress
  automatically before exterior work when a suit is available; bottle
  days by tier (50/100/200) and the two research nodes by label; a
  suit is a round trip, not a home.

### 4.9 Death & miscellany

- **Death of a wearer:** a suit is never silently deleted by its
  wearer's death. Colony-issued suit → pool +1 (recovered with the
  body; `BuryCorpse` is unaffected). Expedition suit → loose entity
  at the death tile, air preserved — a rescuer can wear a dead pop's
  bottle, and in vacuum that pickup is itself an expedition.
- **Suit at 0 air indoors:** nothing happens; it refills (§4.2). A
  0-air suit outdoors is a costume: protection is exactly `air > 0`.
- **Corpse wearing:** corpses do not wear suits after the death rule
  above runs; no suit is ever double-counted (pool + loose + worn is
  conserved — assert it in the round-trip test).

## 5. REFACTOR Phase: Quality & Design

- Keep the drain decision inside `pressure_damage_system`'s branch so
  "covered by bottle" and "takes damage" can never disagree; the
  alert markers (`SuitAirAlerted` / `SuffocationAlerted`) re-arm on
  the same threshold crossing.
- The issue-tier function is pure and unit-tested against all
  combinations of scenario floor × tech state.
- Chronicle strings name the pop (via `PopName`, as the suffocation
  alert does) and use days, the player's unit.
- Register `suit_air_system` and the status refresh in the same
  schedule sets Phase 2 used for atmosphere; verify with the
  schedule-level test in §3, not by inspection.

## 6. Acceptance Criteria

- [ ] All tests in RED phase pass, including both schedule-level tests
- [ ] `cargo test` returns 0 failures beyond the known pre-existing pool
- [ ] `cargo clippy -- -D warnings` passes on touched modules
- [ ] Coverage >= 85% for new code
- [ ] A headless Classic colony shows `suits=3` in STATS at tick 0;
      a suited pop teleported into vacuum (`relocate`) loses air, not
      HP, and the chronicle carries the low-air warning before any
      suffocation alert
- [ ] Layer 2 Ready STATS shows `suits=6` and a fetched suit there
      carries 100 days before any research

## 7. Technical Guidance

**Grounding (verified on trunk `80ff78a4`, 2026-10-14):**

- `Equipment { tool, weapon, body, head, totem, mask, has_symbiote }`
  and `Clothing { clothing_type: Tunic|Parka, insulation, durability,
  max_durability }` — `src/layer1/economy/items.rs`. Items are
  entities carrying the `Item` marker; a pop's slots hold `Entity`
  references.
- Clothing lifecycle to mirror: Tailor refining 1 Cloth → 1
  `clothing` resource (`src/layer1/economy/refining.rs` ~:174);
  `evaluate_fetch_clothing` scores 0.95 when unclothed
  (`src/layer1/actions/clothing.rs`); `handle_fetch_clothing`
  (`src/layer1/execution/arrival.rs` :338) consumes the resource,
  spawns the entity, sets `eq.body`, despawns the replaced garment,
  and honors the BioSuit fused check via `UnequipFailedEvent`.
- `clothing_wear_system` (`src/layer1/clothing.rs`) degrades the body
  slot at 0.05/tick and despawns at 0 — extend, don't duplicate.
- `BioSuit` (`src/layer1/economy/bio_loom.rs`) is a **parasitic armor
  organism**, not protective gear; the name is taken and the body slot
  is shared. Do not reuse the component.
- Suffocation: `pressure_damage_system` (`src/layer1/physics/pressure.rs`
  :~415), `SUFFOCATION_PRESSURE_THRESHOLD = 0.2` (:398), 1 HP/tick,
  `DeathCause("Atmospheric breach")`, Phase 2's `SuffocationAlerted`
  + `ColonyAtmosphere` live in the same file — work with them.
- Tech: flat `Tech` enum with `cost()` / `label()` / `description()` /
  `storage_cost()` const fns (`src/layer1/tech/mod.rs`); new variants
  must be added to **all four** match arms. Costs in the tree run
  10–100 Knowledge, storage 5–50 TB.
- Utility AI: evaluation groups live in `src/layer1/mind/utility_ai.rs`
  (`evaluate_group_logistics` ~:660); candidate buffers
  (`work_designations`, `repair_designations`, stockpiles, item
  entities) are already plumbed into the evaluator context.
- Persistence (`src/persistence/mod.rs`): `ColonyResourcesSave`
  mirrors every `ColonyResources` field 1:1 (the pool persists free);
  pop **equipment is explicitly NOT captured in v1** — this spec
  requires a deliberate minimal exception: `PopSave.worn_suit:
  Option<SuitSave { class: String, air: f32, durability: f32 }>`
  (a mid-excursion save must not strip the pop's bottle), and
  `ItemSave` gains optional suit payload fields following the
  `Corpse`/`Curio` payload precedent (:751). Bump `format_version`
  per the module's convention and document the exception in the
  module docs where the v1 exclusion is stated.
- **Private Stash interplay (Spec 350):** `PrivateStash` holds
  `HashMap<ResourceType, f32>` — resource amounts, not item entities
  (`src/layer1/private_stash.rs` :44). Suits are **not stashable** in
  v1. The honest emergent edge: a suit *being worn* is unavailable to
  everyone else until doffed, and Search Rooms does not strip worn
  gear. A Hoarder who refuses to come inside is a story, not a bug.

## 8. Questions

*Builder: add questions here if spec is unclear.*

## 9. Out of Scope

- A suit-rack / locker building or designated storage furniture
  (loose suits lie where they drop; the pool is abstract).
- Suit repair, suit quality grades from the crafting `Quality` enum,
  and per-pop suit ownership or theft.
- An oxygen resource economy (no O₂ stockpile; air is per-bottle time).
- Manual don/doff verbs for possessed pops, and any TUI suit panel
  beyond the status-bar segment in §4.8.
- Suits for Visitors, drones, or fauna; protection against anything
  other than pressure (smog, temperature, radiation are other systems'
  problems — the existing smog mask stays smog-only).
- Restoring rim spawn for beacon migrants (the transit drain is
  modeled until that separate decision is made).
- Retroactive bottle upgrades for existing suits (physical bottles;
  research improves *new issues* only).

## 10. Open Questions for Mark

None. The mechanic, the ladder (50/100/200), the two-node tech path,
migrant gear, and the scenario dial were all decided by Mark on
2026-10-14 (including his same-day correction of his own first
numbers). Calibration constants (drain 1.0/tick, low-air 25%, refill
10%/tick, transit drain 30–60) are architect's choices inside those
decisions and are balance-loop territory after landing.
