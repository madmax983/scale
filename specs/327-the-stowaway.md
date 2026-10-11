# 327: The Stowaway

## 1. Overview

Every trade ship that docks might be carrying one passenger too many. On
each `TradeShipArrivalEvent` there is a small chance a stowaway slips off
the ship and melts into the colony: a real, full-bundle pop (name, health,
needs, hunger — they eat at the colony's table) carrying a `Stowaway`
marker with a hidden agenda. The colony census never counts them — STATS
`pops=`, the `pops` roster, and average health/morale skip them, and the
chore wheel never assigns them work (they are hiding, not laboring). Their
food consumption is physical, not bureaucratic: the stockpile feeds one
more mouth than the census admits.

This is the ghost-pop audit pattern from the bcb1df63 fix turned
deliberate: the `ghosts` command still counts every `Pop` entity, so a
careful overseer notices STATS says N while the audit counts N+1. Further
tells: occasional Minor chronicle signs (mismatched stores, strange
malfunctions, encrypted static), a `stowaway` investigation command, and a
STATS `stowaway=` hint field (`clear` / `signs` / `exposed:N`).

Agendas (rolled at spawn): **Saboteur** (periodically damages a random
building through the real `ExplosionEvent` pipeline), **Spy** (sends intel
packets; enough packets trigger a real `PirateRaidEvent` — the raiders
came too prepared), **Sympathizer** (no malice; reveals themselves after
enough ticks and asks to belong).

Counterplay is the `muster` command: a colony-wide census sweep. Each
hidden stowaway rolls detection against their elusiveness; the exposed are
dragged into the light (Major chronicle, unrest shock). Then the reveal
fork: `pardon <id>` (they join the colony as a registered pop; a pardoned
spy defects, improving relations with their home faction) or `expel <id>`
(they are put off the colony; relations with the home faction take a hit
scaled by agenda, the colony is unsettled).

## 2. Dependencies

- `TradeShipArrivalEvent` (`src/layer2/trade/blockade.rs`) — arrival hook
  (spec 326 invasive-biology precedent).
- `PopBundle::random` (`src/layer1/entities/pop.rs`) — full-bundle spawn,
  so the stowaway is a complete colonist, never an accidental ghost.
- `ExplosionEvent` (`src/layer1/environment/volatile.rs`, NOT the
  ignition.rs one) — saboteur damage pipeline (spec 277 deorbit precedent).
- `PirateRaidEvent` (`src/layer1/void_weed.rs`) — spy intel payoff; the
  beacon already sends it at 1%/tick.
- `FactionRelations` (`src/layer3/diplomacy/system_sovereignty.rs`) —
  reveal-fork faction consequences (spec 272 martyrdom precedent).
  Faction id is a deterministic FNV-1a hash of the trade faction name.
- `Unrest` / `UnrestModifier` (`src/layer1/social/unrest.rs`) — exposure
  and expulsion shocks.
- Headless console + STATS (`src/bin/headless.rs`) — `stowaway`, `muster`,
  `pardon`, `expel` commands; `stowaway=` STATS field.

## 3. Design

### 3.1 Arrival (`stowaway_arrival_system`)

Reads `TradeShipArrivalEvent`. Per arrival, rolls
`StowawayConfig::arrival_chance` (0.02; trade ships arrive ~5%/tick, so
roughly one stowaway per 1000 ticks). Skips if `max_hidden` (3) hidden
stowaways already exist. Spawns `PopBundle::random` at a random living
pop's tile (they blend into the crowd; skipped if the colony is empty)
plus a `Stowaway` component: random agenda, `home_faction` from the
event, `elusiveness` 0.0–0.5, `agenda_timer` seeded to the agenda's first
interval. Writes a Minor chronicle hint: "A dockworker swears someone
slipped off the {faction} trade ship without registering."

### 3.2 The hidden pop (`stowaway_agenda_system` + signs)

Every tick, for each stowaway with `!exposed`: `ticks_hidden += 1`,
`agenda_timer` counts down; on zero the agenda acts and the timer resets:

- **Saboteur**: picks a random building, sends `ExplosionEvent`
  (damage 10, radius 1) at its tile, Minor chronicle: "A strange
  malfunction rattles the {building:?}." Interval 400 ticks.
- **Spy**: `intel_packets += 1`, Minor chronicle: "Encrypted static
  bursts from the comms array — gone before it can be traced." At
  `intel_raid_threshold` (3) packets: sends `PirateRaidEvent`, resets
  packets, Minor chronicle: "The raiders came too prepared — someone
  told them exactly where the stores are." Interval 300 ticks.
- **Sympathizer**: no timer action. At `sympathizer_reveal_ticks` (1500)
  ticks hidden: `exposed = true`, Major chronicle: "A gaunt stranger
  steps forward: 'I hid aboard the {faction} ship. I only want to
  belong.'"

Signs: while any hidden stowaway exists, each tick rolls
`signs_chance` (0.002) for one Minor chronicle from a flavor pool
("Someone's been in the stores — the count is off again.",
"A bunk that was empty shows signs of use.",
"The night watch reports footsteps with no one attached.",
"Two crew swear they served an extra mouth at mess.").

Stowaways eat through the normal `consume_food_system` (they hold
`Pop` + `Needs`) — the drain is emergent, not scripted. If the stockpile
runs dry they starve like anyone else.

### 3.3 Census invisibility (deliberate, detectable)

- `collect_pop_data` (utility AI): `Without<Stowaway>` — they never work.
- `print_stats` pops/avg_health query: `Without<Stowaway>`.
- `print_pops` roster: `Without<Stowaway>`.
- `calculate_avg_morale`: `Without<Stowaway>`.
- `ghosts` audit: UNCHANGED — counts all `Pop` entities, so the audit
  total exceeding STATS `pops=` is the deliberate tell.

### 3.4 Muster (counterplay)

`muster` console command → `muster_sweep(world)`: for each hidden
stowaway, detection roll `rng.gen::<f32>() <
muster_base_detection * (1.0 - elusiveness)` (base 0.60). On success:
`exposed = true`, Major chronicle "The muster drags {name} into the
light — a stowaway!", timed `UnrestModifier` +0.15 / 500 ticks
("Stowaway exposed"). Returns the exposed names for the console panel.

### 3.5 Reveal fork (`pardon` / `expel`)

Both require the target to be exposed and resolve immediately:

- `pardon <id>`: removes the `Stowaway` component — they become a
  registered colonist (census counts them, chore wheel takes them).
  Major chronicle: "The colony takes in {name}, late of {faction}."
  A pardoned **spy** defects: `FactionRelations[faction_id] += 25`
  ("their agent came over with the cipher book").
- `expel <id>`: despawns the pop. `FactionRelations[faction_id]` takes
  an agenda-scaled hit: Saboteur −50, Spy −60, Sympathizer −40 ("the
  faction hears you spaced their runaway"). Timed `UnrestModifier`
  +0.10 / 300 ticks ("Stowaway expelled"). Major chronicle:
  "The stowaway {name} is put off the colony — {faction} will hear of
  this."

### 3.6 STATS + console

- STATS gains `stowaway=`: `clear` (none), `signs` (hidden present),
  `exposed:N` (N awaiting resolution).
- `stowaway`: investigation panel — signs assessment plus, for each
  known stowaway, entity id, agenda, ticks hidden, home faction.
  `stowaway spawn [saboteur|spy|sympathizer]` is the debug playtest
  hook (bypasses the trade-ship roll; every other spec ships one).
- `muster`: runs the census sweep.
- `pardon [<id>]` / `expel [<id>]`: resolve exposed stowaways (id from the
  `stowaway` panel; with exactly one exposed stowaway the id may be
  omitted).

## 4. RED Phase: Tests First

Atomic TDD in `src/layer1/social/ship_stowaway.rs`:

1. `test_arrival_spawns_hidden_stowaway` — send `TradeShipArrivalEvent`
   with config `arrival_chance = 1.0`; assert one `Stowaway` pop spawned
   with a full bundle (has `PopName`, `Health`, `Needs`).
2. `test_stowaway_excluded_from_census` — spawn stowaway + normal pop;
   assert the census query (`With<Pop>, Without<Stowaway>`) counts 1.
3. `test_muster_exposes_stowaway` — config `muster_base_detection = 1.0`;
   `muster_sweep` returns the name; `exposed == true`; unrest modifier
   pushed.
4. `test_muster_can_miss` — detection 0.0; stowaway stays hidden.
5. `test_saboteur_damages_building` — agenda timer 1, building present;
   after update, an `ExplosionEvent` was sent.
6. `test_spy_triggers_raid_at_threshold` — intel at threshold − 1, timer
   1; after update, `PirateRaidEvent` sent and packets reset.
7. `test_sympathizer_self_reveals` — ticks_hidden at threshold; after
   update, exposed with Major chronicle.
8. `test_pardon_registers_pop` — exposed stowaway pardoned: `Stowaway`
   component gone, still a `Pop`.
9. `test_expel_hits_faction_relations` — exposed spy expelled: entity
   gone, `FactionRelations[faction_id] == -60`.
10. `test_signs_hint_chronicle` — signs chance 1.0 with hidden stowaway;
    a Minor `AddChronicleEvent` fires.
11. `test_stowaway_status_label` — clear / signs / exposed:N transitions.
12. `test_ip_guard_no_banned_terms` — public strings contain no
    Hitchhiker's-contamination terms.
13. `test_debug_spawn_creates_stowaway` — debug spawn bypasses the roll,
    agenda override honored, starts hidden.
14. `test_debug_spawn_empty_colony` — no living pops, no spawn.
15. `test_agenda_parse` — console agenda arguments parse (case-insensitive);
    unknown words rejected.

## 5. GREEN notes (adaptations)

- No new `Events<...>` type: the module reuses `AddChronicleEvent`,
  `ExplosionEvent`, `PirateRaidEvent`, all already initialized in
  `simulation.rs` (fresh-world panic class avoided by construction).
- Module lives in `src/layer1/social/ship_stowaway.rs` (NOT `stowaway.rs`:
  the legacy `crate::layer1::stowaway` module — an old, unscheduled
  visitor-infiltration mechanic — owns the `stowaway` module name at the
  `layer1` level through `pub use social::*`, so `social::stowaway` would
  be a duplicate definition. The `Stowaway` component is likewise only
  reachable via its full path, never glob re-exported, to avoid an
  ambiguous-glob collision with the legacy `crate::layer1::Stowaway`
  type — the physics::harpoon precedent).
- Arrival and agenda systems chained (arrival before agenda); a
  same-tick spawn is simply not seen by the agenda until the next tick —
  harmless.
- Console `muster`/`pardon`/`expel` are pure `&mut World` functions in
  the module, called from headless.rs (scion-command precedent).
- Faction id: deterministic FNV-1a-32 of the trade faction name —
  "the colony's informal faction registry".

## 6. IP guard

All names original and generic: stowaway, muster, saboteur, spy,
sympathizer, "Independent Merchants". No named characters, places, or
distinctive IP. Banned-term test guards against Hitchhiker's drift
(Vogon, Heart of Gold, Ford Prefect, Zaphod, Babel fish, Pan Galactic).

## 7. Out of scope

- No new `BuildingType` (187 files match on it) — census counterplay is
  the `muster` command, not a building.
- No TUI panel (headless-console first, per loop convention).
- Stowaways never appear in the `pops` roster the adventurer spike lists
  while hidden (they are excluded from `print_pops`).
- Long-run balance tuning (arrival rate, intervals) left for playtest.
