# Spec 1388: The Director — A Threat Budget That Paces the Colony's Bad Days

## 1. Overview

Scale has no storyteller. The balance loop's pacing cycle (2026-10-26, 40 colonies @ 2000t, trunk `275b4934`, `balance/research/2026-10-26-storyteller-pacing-event-deck.md`) measured the consequence: crisis pressure is a flat deck firing from tick 0 — first crisis at median tick 16–28 in every era, 40/40 colonies raided in season 1 (first-raid medians: Classic 46, Layer 2 Ready 92, Ground Survival 31, Social Drama 48) — while lethality is back-loaded *by accident* (4 deaths in the first 500t across 40 colonies; mass deaths at 1250t+ from winter and fire cascades the deck had nothing to do with). Menace is immediate, execution is late, and none of it is intended. There is no grace period, no rest cycle, no escalation arc, and no era has a pacing identity of its own: the same rolls fire in Layer 2 Ready as in Classic.

This spec builds **the Director**: a single pacing authority whose central mechanism is a **threat budget**. The budget accrues over time, scaled by era temperament, colony size, wealth, season, and colony age. Every hostile event kind carries a first-class **threat level**, and the threat level *is* the price: the Director spends budget at exactly the event kind's threat level to fire it through the existing event seams. After a major spend the Director enters a rest window — budget-negative pressure on the colony — during which it cannot spend again. Pressure is legible throughout: the colony can *see* the budget filling, in Chronicle omens written in threat vocabulary, before anything is spent.

The design goal, stated plainly: **pacing is tunable purely by turning budget knobs.** Accrual rates, caps, cost multipliers, grace lengths, and rest lengths are data in one config resource. The balance loop should never need a code change to A/B a pacing curve — it turns knobs, reads the STATS ledger, and measures.

### 1.1 How events fire today (the forensic core)

Verified in code at trunk `275b4934`:

- **The beacon deck** (`src/layer1/economy/beacon.rs`). `ColonyBeacon` toggles `is_active` every 100 ticks (≈50% duty cycle; both systems live at `src/layer1/systems/economy.rs:294-295`). While active, `process_colony_beacon_system` rolls per tick: trade ship 5%/tick (`TradeShipArrivalEvent`, cargo_value 5000, "Independent Merchants"), migrants 1%/tick (`MigrantArrivalEvent`, count 1–3, criminal_chance 0.3, low_skill_chance 0.5), pirate raid 1%/tick (`PirateRaidEvent`). Effective raid rate ≈0.5%/tick from tick 0 — matching the measured raid inter-arrival median of 123–182t. No state, no memory, no era input.
- **Raid consequence** (`beacon_pirate_raid_bridge`, `src/layer1/core/integration.rs:1879`, registered `systems/economy.rs:299`): steals 25% of food stockpile capped at 10, metal −20, all pops take a Morale −0.2 modifier for 500 ticks, Major Chronicle "Pirates have raided the colony!". Magnitude is fixed at every colony age.
- **A second raid source** (`src/layer1/void_weed.rs`): `evaluate_smuggling_heat_system` sends `PirateRaidEvent` when `SmugglingHeat ≥ 100`. This is a *player-caused consequence*, not deck pressure — §9 keeps it out of the Director's spend authority.
- **Organic migrants** (`process_remittances_system`, `src/layer1/economy/remittances.rs`): per 100 credits remitted per home faction, one `MigrantArrivalEvent` (count 1, criminal/low-skill chances 0). Player-economy-driven; not deck.
- **Outbreaks** — Spec 326 Invasive Biology (`src/layer1/biology/invasive.rs`; systems live at `src/simulation.rs:940-941`). See §1.2 — this mechanism is the reason the spore event class silently dies, and the Director design must subsume it knowingly.
- **Orbital crossfire** (Spec 356, `src/layer1/environment/orbital_crossfire.rs`; registered `systems/environment.rs:92`): the closest existing pattern and this spec's structural model. A `CrossfireDirector` Resource holds tuning knobs `{enabled, check_interval_ticks 200, base_chance 0.06, debris_chance_per_unit 0.20, max_chance 0.35, damage 150, heat 250, scatter_radius 12}`; a pure fn `crossfire_chance(director, debris)`; a `CrossfireStats` ledger Resource; an exclusive-`World` director system firing on `tick % interval == 0`; and a shared `trigger_crossfire()` used by both the director and the headless debug command. Crossfire pressure is *earned by the player's own debris ledger* — it is an environmental state, not scheduled menace. §4.7 decides coexistence: **no merger**.
- **Weather/seasons** (`src/layer1/nature/weather.rs`): seasonal table `pick_weather_for_season` never returns SporeStorm/MutagenicRain/BlissStorm (the variants exist; they are unreachable in live play). Weather durations 50–200t. `TICKS_PER_YEAR = 1000` (`src/layer1/balance.rs:4`), 250t/season, current season readable via `SeasonState`. The calendar is deterministic and stays out of the Director in v1 — season is an *input weight* on accrual, never a spend.
- **Eras** (`src/shared/scenario.rs`): `StartScenarioId {Classic, GroundSurvival, SocialDrama, Layer2Ready}`; `start_scenario_definition(id)` (const fn) → Classic = Standard, GroundSurvival = Hard, SocialDrama = Medium, Layer2Ready = Easy. `ActiveStartScenario` is a Resource in the world — the Director reads its temperament from it at setup.

### 1.2 Why spore outbreaks cease after ~tick 1000 (answered explicitly)

Balance measurement: spore outbreaks run 1.58–1.62 per run in bin 0–249 in *every* era, then fall to **0.00 in every bin ≥1000, 40/40 runs**. The cause is not deck exhaustion, not a probability gate closing, and not a consumed resource. It is a **saturated hard cap on a monotonically growing, never-culled population**:

- `InvasiveConfig::default()` sets `max_spores = 200`. `spore_hitchhiker_system` checks `spores.iter().count() >= config.max_spores` **first** on each `TradeShipArrivalEvent` and `continue`s — at cap, the seeding roll (arrival_chance 0.25, exotic 0.6) is never even reached. On success it spawns a `Spore`, sends `SporeBloomEvent`, and writes the Major Chronicle beginning `[SPORE_OUTBREAK]` (the exact string the balance analyzer keys on).
- `spore_spread_system` grows each Spore across infectable neighbor tiles (Grass/Dirt/Shrub/Sapling/Path only) every `spread_interval` (Xenofungus 40t, TerranKudzu 25t), stopping at the same 200 cap.
- **Nothing in bot or ambient play ever removes a Spore.** The only removal path, `quarantine_burn_system`, is player-designation-driven. The live population is therefore a one-way ratchet to exactly 200 — and ~tick 1000 is simply the measured saturation time. Trade ships keep arriving afterward (observed to tick 1408); their seeding is cap-gated before the roll. Frontier exhaustion (converted SporeBloom tiles are not infectable) contributes at the margin.

So the outbreak class does not end — it *fills up and can never fire again*. Any Director that merely reschedules outbreak attempts against this mechanism inherits a class that dies at saturation. §4.6 therefore folds a fix into v1: **spore senescence** — a Spore that reaches its lifespan despawns, the live count falls below cap, and the outbreak class becomes renewable. (Not to be conflated with BloomTouched's separate `BloomZone`, adventurer origin, `BLOOM_MAX_TILES = 400`: that is not the outbreak source.)

### 1.3 The event-kind census — every kind carries a threat level

Per Mark's steer, a threat level is a **property of the event kind itself**, declared in the event-kind registry data (§4.1) — never an ad-hoc number inside Director logic. The census below covers every event kind that can arrive at or strike the colony without a direct player order in the current live build, plus the benign deck arrivals the ledger must account for. Kinds are classified by spend authority:

- **Priced** — the Director may spend budget to fire it (v1: two kinds).
- **Rated, not spent** — threat level assigned now (it feeds the crisis-recency ledger and telegraph vocabulary); spend authority deferred or permanently elsewhere.
- **Benign (threat 0)** — arrivals and calendar beats; never bought with threat budget.

Threat tiers for player-facing language: **0 Benign · 1–24 Minor · 25–49 Standard · 50–74 Major · 75–100 Severe**. No live kind is rated Severe in v1 — the band is headroom, deliberately unused, so future kinds have somewhere scarier to live.

| # | Event kind | Code seam | Threat level | Class |
|---|-----------|-----------|:---:|-------|
| 1 | Pirate raid (`PirateRaidEvent`, deck lane) | beacon deck → `beacon_pirate_raid_bridge` | **60** (Major) | **Priced** |
| 2 | Spore outbreak seeding (`SporeBloomEvent` via hitchhiker) | Spec 326 `spore_hitchhiker_system` | **40** (Standard) | **Priced** (token-gated, §4.6) |
| 3 | Orbital crossfire strike | Spec 356 `crossfire_director_system` | **45** (Standard) | Rated, not spent — debris ledger keeps spend authority (§4.7) |
| 4 | Pirate raid, smuggling-heat lane (same kind as #1) | `evaluate_smuggling_heat_system`, heat ≥ 100 | **60** (same kind, same rating) | Not spent — player-caused trigger; the *kind* is rated once, the *lane* is not the Director's |
| 5 | Orphaned swarm arrival (`SwarmArrivalEvent`) | `src/layer1/orphaned_swarm.rs` | **35** (Standard) | Rated, spend deferred (§9) |
| 6 | Symbiont sabotage crisis | infected-pop lane (damage `12 + 3×members`, cap 30) | **25** (Standard) | Rated, endogenous — recency ledger only |
| 7 | Building-destroying fire / malfunction crisis | grid/malfunction lane (balance taxonomy: `Fire!`, building destroyed) | **30** (Standard) | Rated, endogenous — recency input deferred (§9) |
| 8 | Refugee arrival (`RefugeeArrivalEvent`, layer 2) | `src/layer2/refugees.rs` | **10** (Minor) | Rated, benign-pressure — mouths and labor, not hostility; not spent in v1 |
| 9 | Trade ship arrival (`TradeShipArrivalEvent`) | beacon deck | **0** (Benign) | Benign — Director does not schedule in v1 (§9) |
| 10 | Migrant arrival (`MigrantArrivalEvent`, both lanes) | beacon deck; remittances | **0** (Benign) | Benign — see reserved kind below |
| — | Criminal-migrant surge *(reserved — not a live kind today; a future social spend class would be its own kind, not a repricing of #10)* | — | **20** (Minor, reserved) | Deferred (§9) |
| — | Season transition / weather shift | calendar, `SeasonState` | **0** | Not a purchasable kind — deterministic calendar; season is an accrual weight only |

The long tail of internal `*Event` types in the codebase (system messages between mechanics — damage, kills, designations, diplomacy pings) are **not deck kinds** and are deliberately unrated. The rule going forward, which this spec establishes: **any future exogenous event kind must declare a threat level in the registry before any system may schedule it.** A kind without a rating cannot be spent on, full stop.

### 1.4 Decided: threat level is static per kind

**Decision: the threat level of a kind is a single static number, fixed in the registry. The Director's price for firing a kind is that number, adjusted only by the era's `cost_mult` (§2) — a temperament parameter, printed in STATS, never hidden. What scales with colony size is the event's internal magnitude at fire time, never its price.**

Defense:

1. **Legibility.** A static rating is vocabulary. The player learns "a raid is a Major threat" the way they learn any other rating in the game, and the Chronicle can speak it honestly (§4.5): an omen that says a *Major* threat gathers means the same thing in hour one and hour ten. A price recomputed at spend time from colony state is a number the player can never learn, verify, or anticipate — telegraphing built on it would be lying with extra steps.
2. **Accounting auditability.** With static prices, any run's entire Director ledger reconstructs from STATS alone: spends × registry prices × the era's one multiplier. The balance loop can diff two pacing curves and attribute every difference to a knob. Computed-at-spend pricing smears attribution across the colony's whole state history.
3. **The no-rubber-band law.** An "effective threat" computed at spend time from colony size/wealth/distress is exactly where rescue logic hides: price the raid cheaper because the colony is struggling and you have built a rubber band with plausible deniability. Static prices cannot do that. Colony state enters through **accrual scalers** (§4.2) — which are era-declared, STATS-visible, and by law (§Decided) never distress-sensitive — and through **magnitude at fire time** (the raid bridge already steals a *percentage* of stockpile; the spore already spreads tile by tile). The size of the colony changes what a raid *does*, not what it *costs to send*.
4. **Determinism.** Static price + declared scalers keeps the Director a pure function of (tick, config, ledger, colony aggregates) — replayable and testable in the live schedule (§3).

### Decided (Mark, 2026-10-26 — not open for builder reinterpretation)

> - **"Yes, we need a director."** One pacing authority for intentional hostile pressure. (2026-10-26)
> - **The threat budget is the central mechanism** — "a really nice mathematical balance knob you can tweak. This seems like perfect usecase for them." Accrual, prices, cap/floor, spend rules, and rest are budget mechanics; era temperament is expressed *as* budget parameters in data. (2026-10-26)
> - **Every event kind carries a threat level, and the threat level is the price** — "every kind of event needs to have a threat level, and the director gets a threat budget it can account for." One number, two jobs: danger rating and budget cost. Ratings live in the event-kind data, not in Director logic. (2026-10-26)
> - **Difficulty lives in start selection** (Pirates!-style eras; Classic is the brutal default, decided 2026-10-13). The Director shapes WHEN events fire, never WHETHER catastrophe is possible. Era differences are temperament data (§2), not separate code paths.
> - **NO rubber-banding.** Accrual is never distress-sensitive: low morale, deaths, and damage never reduce accrual, and deaths never refund budget. A doomed colony is paced, not rescued.
> - **Fairness = telegraphing and counterplay.** Pressure must be legible before it lands (Chronicle foreshadowing in threat vocabulary, ambient tension signal). Recovery windows are pacing, not mercy: the budget keeps filling during rest — deferral, never forgiveness.

## 2. Era temperaments — expressed as budget parameters

Temperaments live in a Director-side `temperament_for(id)` const fn mirroring `start_scenario_definition` (the Director reads `ActiveStartScenario`; shared scenario metadata is not touched). Every field below is a `DirectorConfig` knob the balance loop can override per-run — the table is the *shipped default data*, not code.

Base constants (era-independent defaults): `base_accrual 0.5/tick`, `check_interval 25t`, `budget_floor 0`, raid threat 60, outbreak threat 40 (registry, §1.3).

| Parameter | Classic (Standard) | Ground Survival (Hard) | Social Drama (Medium) | Layer 2 Ready (Easy) |
|---|---|---|---|---|
| `grace_ticks` (no spends before) | 100 | 150 | 125 | **400** |
| `accrual_mult` | 1.0 | **0.6** | 0.7 | 0.7 |
| `budget_max` (cap) | 120 | 100 | 110 | 90 |
| `cost_mult` | 1.0 | 1.0 | 1.0 | 1.1 |
| `rest_ticks` after a hostile spend | 150 | 200 | 250 | **400** |
| `year_escalation` (accrual growth/yr) | +15% | +10% | +10% | +5% |
| Season accrual weight of note | — | **winter ×1.5** (the calendar is this era's villain; the budget leans into winter rather than competing with it) | — | — |
| Class weights (raid : outbreak) | 3 : 2 | 1 : 1 | 1 : 1 | 1 : 1 |
| Foreshadow threshold (budget ≥ % of cheapest affordable class) | 75% | 75% | **60%** (this era hears about it earlier) | **60%**, omens at every band |
| **First-raid target median (acceptance, §6)** | **150–300t** | **300–600t** | **250–500t** | **500–900t** |

Temperament rationale in one line each: **Classic** is the brutal default — short grace, short rests, the steepest escalation; pressure arrives early because Classic promises it will. **Ground Survival** spends *less* on raids because winter is already its executioner; its budget leans seasonal. **Social Drama** is forewarned — longer rests, earlier omens; its catastrophes are social, and the Director's job is dread, not frequency. **Layer 2 Ready** is "much easier to maintain" (Mark's phrase for the era) expressed honestly: a real grace period and long recovery windows — the same kinds at the same ratings, arriving later and resting longer. Nothing is removed from its deck; catastrophe remains possible, merely paced.

## 3. RED Phase: Tests First

All behavioral proof runs through the **live schedule** (systems registered in the real app, advanced by ticks — the item-78 lesson: direct-call tests of an unwired system prove nothing). Test sketches:

```rust
#[test] // temperament data is ordered the way §2 promises
fn temperament_ordering_matches_era_table() {
    // L2R grace > Classic grace; L2R rest > Classic rest;
    // Ground hostile accrual_mult is the lowest; Classic escalation steepest.
}

#[test] // grace is absolute: even a forced-full budget cannot spend early
fn no_spend_before_grace_under_forced_budget() { /* live schedule, Classic, budget=cap at t0 → zero hostile events before tick 100 */ }

#[test] // accrual continues during rest (deferral), spends are blocked during rest (pacing)
fn rest_blocks_spends_but_not_accrual() {}

#[test] // after rest expires, an affordable class spends and the ledger records price == threat_level × cost_mult
fn spend_after_rest_deducts_threat_level() {}

#[test] // the beacon is no longer a raid authority
fn beacon_fires_no_pirate_raids_over_long_run() { /* trade + migrants still fire; PirateRaidEvent count == 0 from the deck lane */ }

#[test] // a Director raid spend resolves through the existing bridge unchanged
fn director_raid_spend_resolves_via_bridge() { /* food/metal theft + Morale modifier + Major Chronicle all present */ }

#[test] // outbreak token gating: no token → arrivals never seed; token → first qualifying arrival seeds, token consumed
fn outbreak_requires_director_token() {}

#[test] // senescence: an aged spore despawns, live count falls below the 200 cap,
        // and a later outbreak fires after prior saturation (the 40/40 regression)
fn spores_senesce_and_outbreaks_recur() {}

#[test] // a crossfire strike imposes a half-length rest on the Director
fn crossfire_strike_imposes_half_rest() {}

#[test] // omens: exactly one omen Chronicle per upward tension-band crossing per rest cycle — never per tick
fn omen_fires_once_per_band_crossing() {}

#[test] // the anti-rubber-band clause: killing pops / tanking morale does not change accrual
fn accrual_is_distress_insensitive() {}

#[test] // STATS carries the ledger
fn stats_line_contains_director_fields() { /* director= field present with budget/spends/rest */ }
```

## 4. GREEN Phase: Minimal Implementation

### 4.1 The registry and the ledger — new module `src/layer1/director.rs`

Follow the crossfire pattern (Spec 356) exactly: knob Resource + pure functions + stats ledger Resource + one director system.

```rust
pub enum DirectorEventKind { PirateRaid, SporeOutbreak, CrossfireStrike, OrphanedSwarm,
                             SymbiontSabotage, BuildingFire, RefugeeArrival,
                             TradeShip, MigrantArrival }

impl DirectorEventKind {
    /// THE threat level: danger rating and budget price in one number.
    /// Static per kind (§1.4). New exogenous kinds declare theirs here or
    /// cannot be scheduled.
    pub const fn threat_level(self) -> u32 { match self {
        Self::PirateRaid => 60, Self::SporeOutbreak => 40,
        Self::CrossfireStrike => 45, Self::OrphanedSwarm => 35,
        Self::SymbiontSabotage => 25, Self::BuildingFire => 30,
        Self::RefugeeArrival => 10, Self::TradeShip => 0, Self::MigrantArrival => 0 } }
    pub const fn tier(self) -> ThreatTier { /* 0 Benign, 1–24 Minor, 25–49 Standard, 50–74 Major, 75+ Severe */ }
}

#[derive(Resource)] pub struct DirectorConfig { /* every knob in §2 + base constants;
    resolved from ActiveStartScenario at setup; overridable wholesale by headless/bot levers */ }
#[derive(Resource)] pub struct DirectorState { pub budget: f32, pub rest_until: u64,
    pub last_spend_tick: u64, pub last_spend_kind: Option<DirectorEventKind>,
    pub spends_total: u32, pub saving_for_major: bool, pub outbreak_token: bool,
    pub tension_band: TensionBand, pub temperament: DirectorConfig /* resolved copy */ }
```

### 4.2 Accrual — the budget fills

Per tick (cheap; the spend check itself runs on `check_interval`):

```
accrual = base_accrual × era.accrual_mult
        × pop_scaler      // 1.0 + 0.02 × max(0, pop_count − 5), capped ×1.5
        × wealth_scaler   // stockpiled food+metal vs era-neutral thresholds, capped ×1.25
        × season_weight   // era table (§2); default 1.0
        × (1 + year_escalation × years_elapsed)
budget = min(budget + accrual, budget_max)   // floor 0; never negative
```

Scalers are computed from colony aggregates only (pop count via `Pop` query — the colony-heart pattern in crossfire; wealth via `ColonyResources`). **By law, no scaler reads morale, health, deaths, or damage.** During rest, accrual continues at the same rate: rest is budget-negative *pressure on the colony* (nothing can be spent at it) while the budget silently refills — which is precisely what makes the end of a rest window legible and tense rather than safe.

**Cap behavior (use-it-or-lose-it):** at cap, accrual is lost. If the gates (§4.3) pass while at cap, the Director *must* spend at that check — hoarding past the cap is waste, and the colony should be able to learn that a full, held budget is a promise, not a bluff.

### 4.3 Spend rules — the budget is spent

On each check (`tick % check_interval == 0`), in order:

1. **Grace gate:** `tick < scenario_start + grace_ticks` → no spend, ever (bot levers excepted, §4.8).
2. **Rest gate:** `tick < rest_until` → no spend.
3. **Affordability:** candidate classes = priced kinds whose effective price (`threat_level × cost_mult`) ≤ budget. None → no spend (accrual continues).
4. **Save-for-a-major:** if `budget ≥ 0.8 × raid_effective_price` and the raid is not yet affordable, set `saving_for_major` and skip cheaper spends this check — the Director is visibly gathering itself for a Major arc. Exception: at cap, spend the cheapest affordable class rather than waste accrual (cap rule beats saving rule).
5. **Class choice:** weighted pick among affordable classes by era class weights. **Partial spends** mean exactly this: buying the cheaper affordable class instead of the unaffordable preferred one. A kind is never split, discounted, or fired at partial magnitude for a partial price — one kind, one price, whole event.
6. **On spend:** deduct the effective price; `rest_until = tick + rest_ticks`; record `last_spend_*` and `spends_total`; fire through the kind's seam (§4.4/§4.6); clear `saving_for_major`.

### 4.4 Spend class 1 — Raid (Director becomes sole deck raid authority)

The Director sends `PirateRaidEvent` directly; `beacon_pirate_raid_bridge` resolves it unchanged (same theft, same morale hit, same Major Chronicle). **The beacon's 1%/tick pirate roll is removed** — the beacon keeps its trade (5%) and migrant (1%) rolls and its 100t toggle. The existing test `test_beacon_spawns_criminals_and_pirates` is updated to assert the new law: the deck lane sends zero raids; raids arrive only via Director spend or the smuggling-heat lane. Raid magnitude stays the bridge's fixed/percentage behavior in v1 (§9 defers magnitude scaling).

### 4.5 Telegraphing — pressure in threat vocabulary

- **Tension bands** derive from the budget itself: `Calm` (budget < 40% of cheapest priced class), `Rising` (40–75%), `High` (≥ foreshadow threshold), `Critical` (a class is affordable and all gates are open — a spend is *possible at the next check*).
- **Omens:** on each *upward* band crossing (once per crossing per rest cycle — dwell, never strobe; the item-104 lesson), a Minor Chronicle in threat terms, e.g. "Scouts report raider sign gathering beyond the beacon's edge — a **Major** threat gathers." / "Traders speak of contaminated holds — a **Standard** threat rides the next hulls." The tier word is the registry's word (§1.3): the vocabulary the player learns from omens is the same vocabulary that prices the event when it lands.
- **Foreshadow:** at the era's foreshadow threshold, a Standard Chronicle names the gathering class when the class choice is already determined by affordability (only one affordable class), else names the tier only. No exact countdowns in v1 (§9) — dread, not a timer.
- **Status bar:** a tension segment appears only at High/Critical (dwell-filtered). Calm colonies show nothing — silence is the signal for safety.

### 4.6 Spend class 2 — Outbreak (token-gated) + the senescence fix

- A Director outbreak spend does not spawn anything directly. It sets `outbreak_token = true` and deducts 40 × cost_mult. The **next qualifying trade arrival** runs its hitchhiker roll under the token (existing arrival/exotic chances); on the roll resolving — success *or* failure — the token is consumed. No token → `spore_hitchhiker_system` never seeds, however many ships arrive. This keeps outbreaks riding the trade fiction (contaminated hulls arrive on ships) while making their *cadence* the Director's accounting decision.
- **Senescence (the §1.2 fix):** `Spore` gains an age; at `spore_lifespan` (default 1500t, a config knob on `InvasiveConfig`) the entity despawns. Terrain stays SporeBloom (the scar remains; the living pressure does not). The live count can now fall below the 200 cap, the seeding gate reopens, and the outbreak class is renewable across a 4000t run instead of dying at saturation.

### 4.7 Coexistence with the crossfire director — decided: no merger

Crossfire keeps its own director, its own ledger, and its spend authority, because its pressure is *bought by the player's own debris* — a managed sky state, not scheduled menace. Merging would force era temperaments to fight the debris ledger for the same budget and would make the player's own orbital housekeeping a pacing input it was never designed to be. The two systems share exactly one signal: **a crossfire strike imposes a half-length rest on the Director** (observed via `CrossfireStats.last_tick` changing) — the sky falling on the colony buys it a shorter, not full, breather from scheduled menace, and the strike's threat level (45) is recorded in the Director's recency ledger so STATS tells one coherent crisis story. Neither director spends the other's budget; neither can fire the other's kinds.

### 4.8 Instrumentation — the balance loop's control panel (mandatory)

- **STATS:** append a `director=` field to the headless STATS line (`src/bin/headless.rs`, in the established appended-field style of `crossfire=`, `dissat=`): `director={band}:{budget:.0}/{cap} spends={n} last={kind}@{tick} rest_until={tick} accrual={rate:.2} cost_mult={m}`. Any run's ledger must reconstruct from STATS alone (§1.4).
- **Headless panel:** a `director` panel (crossfire/choir panel precedent) printing resolved temperament (every §2 knob), current state, last spend, and next-window estimate (earliest tick a spend becomes possible = max(grace end, rest end, projected affordability at current accrual)).
- **Bot levers:** `director force raid`, `director force outbreak` (fire the seam now, ledger-recorded as forced, bypassing gates — for tests and balance probes), `director budget <n>` (set budget directly), `director show`. Forced spends are marked forced in the ledger so A/B data never confuses them with organic pacing.
- **Tuning contract:** every number in §2 and §4.2 lives in `DirectorConfig`. A pacing experiment = a config diff. If a balance cycle ever needs a code change to move a first-raid median, that is a spec bug, and it should be filed as one.

### 4.9 Scenario notes

Era identity is data (§2); there are no per-era systems. One note: Ground Survival's winter weight means its Director pressure and its calendar pressure crest together by design — that era's fantasy is the year itself closing in, and the telemetry (§6) must be read with the season in view.

## 5. REFACTOR Phase: Quality & Design

- `crossfire_chance`-style purity: accrual and spend-decision logic as pure functions over `(config, state, aggregates)`; the system is a thin exclusive-`World` shell on the check interval, mirroring `crossfire_director_system`.
- The beacon diff is deletion-first: the pirate roll and its test expectations leave; nothing about trade/migrant cadence changes.
- Registry (§4.1) is the single home for threat levels — grep for threat numbers elsewhere in Director code is a review failure.

## 6. Acceptance Criteria

Measured by the balance loop on the live build, 40-colony pacing corpus (12/12/8/8 era split, 2000t; outbreak recurrence read on an extended 4000t Classic subset):

1. **Grace holds:** zero Director hostile spends before era grace in ≥95% of runs (target: 100%; any breach is a gate bug, not tuning).
2. **First-raid medians land in the §2 bands:** Classic 150–300t, Ground Survival 300–600t, Social Drama 250–500t, Layer 2 Ready 500–900t (vs today's 46/31/48/92).
3. **Outbreaks recur:** `[SPORE_OUTBREAK]` Chronicles appear in bins ≥1000 in ≥50% of extended runs (vs 0.00 in 40/40 today), and the live spore count is observed below cap after prior saturation in those runs.
4. **Rest is real:** zero second hostile spends inside a rest window, instrumented via the ledger — no exceptions, including the check immediately after a crossfire half-rest expires.
5. **Compounding falls:** crisis-overlap pairs (≤100t) per 2000t run drop below the flat-deck baseline (Classic 8.8, L2R 13.1, Ground 9.1, Social 9.0) in every era.
6. **Deferral, not deletion:** total Director hostile events @4000t within ±25% of flat-deck expectation at matched era — the Director reschedules menace; it does not quietly remove it.
7. **No rescue:** accrual under induced colony distress (deaths, morale collapse) matches accrual in a matched calm run within noise — the §3 distress-insensitivity test at corpus scale.
8. **Suite gates:** full lib suite green against the known pre-existing failure pool only; live-schedule proof for every §3 test (no direct-call substitutes).

## 7. Technical Guidance

- Rust edition 2021 (repo standard — do not migrate). Bevy ECS idioms as in `orbital_crossfire.rs`; exclusive `World` system for the spend check, normal system for accrual if profiling ever asks (it will not at 0.5/tick arithmetic).
- `DirectorState`/`DirectorConfig` are Resources inserted at setup beside `CrossfireDirector`; temperament resolved once from `ActiveStartScenario`.
- The outbreak token lives in `DirectorState`, read by `spore_hitchhiker_system` — one new resource read in the invasive system, no event-type changes. Senescence is an age field on `Spore` plus a despawn check in the existing spread system.
- Determinism: class-choice weighted pick uses the run's seeded RNG stream, never wall-clock or hash iteration order.

## 8. Questions

- Should the status-bar tension segment reuse the LifeSupport segment slot pattern or take its own? (Builder's call — dwell behavior is the requirement, the slot is not.)
- Is `check_interval 25t` fine enough that a spend can land within a tick-band of gates opening for the §6 median measurements? (Default: yes; if medians smear, drop to 10t — a knob, not a redesign.)

## 9. Out of Scope (deferred, named so nobody reinvents them)

- **Benign-deck pacing** — Director scheduling of trade/migrant arrivals. The deck's benign cadence is untouched in v1.
- **Social spend classes** — criminal-migrant surges (reserved kind, threat 20, §1.3), sabotage waves, grievance events. Rated where they exist; spend authority is a follow-up spec.
- **Swarm spend authority** — `OrphanedSwarm` is rated (35) but not priced into v1's class table.
- **Raid magnitude scaling** — v1 raids resolve through the existing fixed bridge. Scaling theft/morale by colony size at fire time is a follow-up that must not reprice the kind (§1.4).
- **Fire/panic crisis-recency inputs** — endogenous crises (rated 25/30) inform the ledger only via crossfire in v1; feeding fire/panic recency into rest is deferred (the panic lane has its own recent fix history — item 133 — and earns a separate look).
- **Explicit countdown warnings** — omens speak in tiers and bands, never "raid in 50 ticks."
- **Crossfire merger** — decided against (§4.7), not deferred: do not reopen without new evidence.
- **Director UI** beyond the status-bar tension segment and the headless panel.
- **Smuggling-heat lane changes** — player-caused raids stay player-caused, unpriced and ungated by the Director.

## 10. Open Questions for Mark

1. **Severe band headroom (75–100) sits empty in v1.** Default: keep it empty — a scarier-than-raid kind should be a deliberate future design, and the empty band makes omens honest. Alternative: rate a future combined arc (raid + outbreak same window) into it now, unspendable in v1.
2. **Spore lifespan default 1500t.** Default: ship it — saturation at ~1000t plus a 1500t lifespan yields recurrence on the 4000t horizon the acceptance matrix reads. The balance loop tunes it as an `InvasiveConfig` knob thereafter.
3. **Layer 2 Ready `cost_mult 1.1`** is the only era that reprices kinds. Default: keep — it is declared, STATS-visible, and expresses "the same world, slightly dearer menace" without touching ratings. Alternative: 1.0 everywhere and let grace/rest carry the whole era difference.

*None of these block a builder: each carries its decided default.*
