# Spec 1385: Drone Exterior Work — Repair First, and a Fleet for Layer 2 Ready

## 1. Overview

Drones are the colony's second vacuum counterplay, beside EVA suits
(Spec 1384, landed `ae77087b`). The reasoning is the same one that made
suits necessary — the Atmosphere Forensics death chain: everything
beyond the starter hull is hard vacuum, pops take 1 HP/tick below
0.2 atm, and the buildings that keep everyone alive (above all
LifeSupport) sit exposed to sabotage, fire, and decay. Suits let a
*pop* survive the trip out. Drones skip the problem entirely:
**machines don't breathe.** A drone has no `Health`, no `Needs`, and
`pressure_damage_system` never sees it — it can work in vacuum all
day on battery alone.

Today that advantage is wasted. Drones (Spec 116,
`src/layer1/entities/drone.rs`) exist as battery-powered haulers:
they charge at a powered `DroneHub` below 20% battery
(`evaluate_drone_actions_system`, `process_charge_system`), they haul
(`drone_work_bridge_system` flips Idle → `ActionType::Haul`), and
they go Feral after 5000 ticks disconnected from their command
anchor (`check_drone_connection`). Their only work verb is Haul.
Every repair — including the sabotage-chewed exterior LifeSupport
that the sabotage/repair counterplay pass (2026-10-03) proved is the
colony's mortal wound — is pops-only work, done in vacuum, at
1 HP/tick.

This spec extends drone duties to **repair**:

- An idle (or merely hauling) drone with charge is dispatched to the
  colony's most urgent damaged building, **exterior (vacuum-exposed)
  buildings first** — a damaged LifeSupport on the hull outranks a
  scuffed interior floor — then by damage fraction, then by distance.
- Repair uses the colony's existing repair pipeline
  (`structure::process_repair`, 10 HP/tick — the same rate a pop
  repair crew delivers), so quirks reveal, ancient-structure refusal,
  and designation cleanup behave identically whoever holds the welder.
- Battery and Feral law are untouched and take precedence: below 20%
  battery the drone charges instead of repairing; a Feral drone
  repairs nothing; a disconnected drone holds position rather than
  freelancing.

And it gives the relief lane its fleet: **Layer 2 Ready starts with
a DroneHub and a fleet of 3 drones** — exactly one hub's full
complement under the spawner's existing 3-per-hub cap — placed on
the one free interior tile, cardinal-adjacent to the Lander, so the
hub is on the power grid from tick 0. Same established pattern as
Layer 2 Ready's Command Center, redundant LifeSupport, Generator,
and Spec 1384's rack of Mk II suits: preparation, not softening.
Classic, Ground Survival, and Social Drama start with no hub and no
drones, exactly as today; a Classic colony that builds a DroneHub
(30 metal + 10 stone) gets repair drones the same day, because the
dispatch is scenario-agnostic.

### Decided (Mark, 2026-10-14/15 — not open for builder reinterpretation)

- **Drones are the second vacuum counterplay beside EVA suits.**
  Machines don't breathe; exterior work is their niche.
- **Repair first.** Damaged exterior buildings — LifeSupport above
  all — are the priority use case. Mining and chopping are *not* in
  this spec (see §9): the pop Work pipeline routes through skills,
  tools, wages, and morale that drones do not have, and bolting it
  on speculatively would break the tight scope. They are named
  follow-ups if repair lands cleanly.
- **Layer 2 Ready "just already has" them**: a DroneHub + small
  drone fleet in its starting grant, mirroring its Command Center
  and redundant LifeSupport grants.
- **Machine Awakening stays untouched.** The tech tree's Bots /
  Awakening lane is a different feature; nothing here implements,
  hooks, or prefigures awakening. Feral (disconnection) is the only
  drone autonomy failure mode, exactly as before.

## 2. Dependencies

- Drones (Spec 116): `Drone`, `DroneState`, `DroneBattery`,
  `DroneHub`, `GridConnection`, `ConnectedTo`, `FeralDrone`,
  `evaluate_drone_actions_system`, `process_charge_system`,
  `drone_battery_system`, `check_drone_connection`,
  `process_feral_drones`, `drone_spawner_bridge_system` (cap:
  3 drones per hub), `drone_work_bridge_system` —
  `src/layer1/entities/drone.rs`, tests in
  `src/layer1/entities/drone_tests.rs`
- Repair pipeline: `Structure { current_hp, max_hp }` (every building
  spawned via `spawn_building` carries one),
  `structure::process_repair` (10 HP/tick pop rate; refuses
  `AncientStructure`; despawns a completed Repair *designation* but
  never the structure itself), `DeferMaintenance` opt-out —
  `src/layer1/architecture/structure.rs`
- Pop repair precedent: utility repair evaluation with
  `DeferMaintenance` filtering and damage-fraction urgency —
  `src/layer1/mind/utility_ai_population.rs` (~:525–560);
  `work_execution_system` executing `DesignationType::Repair` via
  `process_repair` — `src/layer1/execution/general_work.rs`
- Movement pipeline (shared, already drone-compatible):
  `StartPlan` → `process_start_plan_system` → `MovementTarget` →
  `movement_system` (any non-Building entity with `Speed`; work
  actions arrive *adjacent* via `check_work_adjacency`) →
  `AtTarget` — `src/layer1/execution/movement.rs`
- Pressure: `PressureGrid`, `SUFFOCATION_PRESSURE_THRESHOLD = 0.2` —
  `src/layer1/physics/pressure.rs`. "Exterior" in this spec means:
  the building's tile reads below that threshold.
- Power: `PowerConsumer { demand, active }`, `power_grid_system`
  (grid membership is cardinal tile adjacency between power
  components) — `src/layer1/energy/`; DroneHub costs 30 metal +
  10 stone, draws 10 power — `src/layer1/architecture/building.rs`
- Start scenarios: `src/shared/scenario.rs`,
  `src/setup.rs` (`apply_layer2_ready_start`; starter layout
  constants `STARTER_*_OFFSET`; interior is the 3×3 around the
  center, ringed by walls at ±2)
- EVA suits (Spec 1384): interplay only, see §7

## 3. RED Phase: Tests First

Write these failing first, in `src/layer1/entities/drone_tests.rs`
(setup grant tests in `src/setup.rs`'s test module). Unit tests call
systems directly; the last drone test and the last setup test are
schedule-level, because this repo has repeatedly learned that
direct-call tests can hide an unregistered system.

```rust
// --- Target selection (pure helper, §4.2) ---

#[test]
fn test_pick_repair_target_prefers_exterior_over_worse_interior_damage() {
    // Exterior candidate at 50% damage vs interior at 90% damage:
    // the exterior one wins — vacuum exposure is the priority axis.
}

#[test]
fn test_pick_repair_target_picks_most_damaged_exterior_then_nearest() {
    // Two exterior candidates: higher damage fraction wins;
    // equal damage: the nearer (Manhattan) wins.
}

#[test]
fn test_pick_repair_target_none_when_nothing_damaged() {
    // All candidates at full HP (or empty list) -> None.
}

// --- Dispatch (evaluate_drone_actions_system) ---

#[test]
fn test_drone_assigned_repair_for_damaged_building() {
    // Drone (battery 100) + Building with Structure at 50/100 HP.
    // Act: evaluate once.
    // Assert: PopAction.current == ActionType::Repair, a StartPlan
    // targeting the building exists, Drone.state == Repairing.
}

#[test]
fn test_drone_repair_prefers_exterior_lifesupport_over_interior() {
    // PressureGrid all 0.0 except the interior building's tile (1.0).
    // Interior building at 10/100, LifeSupport-type exterior building
    // at 50/100. Assert the StartPlan targets the exterior building.
}

#[test]
fn test_low_battery_drone_charges_instead_of_repairing() {
    // Battery 10/100, damaged building AND powered hub present.
    // Assert: action == Charge (battery law outranks repair).
}

#[test]
fn test_feral_drone_is_never_assigned_repair() {
    // Drone with FeralDrone / state Feral + damaged building.
    // Assert: action stays not-Repair, no StartPlan inserted.
}

#[test]
fn test_disconnected_drone_holds_instead_of_repairing() {
    // Drone with GridConnection { is_connected: false, .. } +
    // damaged building. Assert: no Repair assignment.
    // (A drone with NO GridConnection — the unit-test default and
    // pre-116 saves — is treated as connected; see §4.3.)
}

#[test]
fn test_drone_skips_deferred_and_ancient_structures() {
    // Only damaged candidates carry DeferMaintenance /
    // AncientStructure. Assert: no Repair assignment.
}

// --- Execution (drone_repair_system) ---

#[test]
fn test_drone_repair_restores_hp_and_completes_to_idle() {
    // Drone AtTarget on a Repair MovementTarget for a building at
    // 50/100 HP. Run drone_repair_system 5 times.
    // Assert: HP == 100 (5 ticks x DRONE_REPAIR_PER_TICK), action
    // back to Idle, state back to Idle, MovementTarget/AtTarget
    // removed.
}

#[test]
fn test_schedule_drone_travels_and_repairs() {
    // Schedule-level: evaluate + process_start_plan + movement +
    // drone_repair in one Schedule; drone starts 3 tiles from a
    // building at 50/100. Run up to 30 ticks.
    // Assert: HP > 50 (repair actually happened through the real
    // movement pipeline, not a direct call).
}

// --- Layer 2 Ready grant (src/setup.rs tests) ---

#[test]
fn test_layer2_ready_starts_with_drone_hub_and_fleet() {
    // setup_world_with_config(Layer2Ready): exactly 1 DroneHub
    // building, exactly 3 Drone entities at the hub tile, each
    // ConnectedTo the hub with a full battery. Classic: 0 hubs,
    // 0 drones.
}

#[test]
fn test_layer2_ready_drone_repairs_damaged_lifesupport_in_real_schedule() {
    // L2R world; set the redundant LifeSupport's Structure to
    // 50% HP; run the real simulation up to 300 ticks, sampling
    // drone PopActions.
    // Assert: some drone held ActionType::Repair during the run,
    // and the LifeSupport ends above its damaged HP.
}
```

## 4. GREEN Phase: Minimal Implementation

### 4.1 State & constants (`src/layer1/entities/drone.rs`)

- `DroneState` gains **`Repairing`**. (Hauling/Charge keep their
  existing representation via `PopAction`; `state` mirrors the
  drone's headline activity, as it already does for Feral.)
- `pub const DRONE_REPAIR_PER_TICK: f32 = 10.0;` — deliberately the
  pop repair rate (`process_repair` callers pass 10 HP/tick of work;
  the sabotage pass measured pop repair at 10 HP/tick). A drone is
  a tireless pop-rate welder, not a better one; its advantage is
  that it can stand in vacuum forever, not that it welds faster.

### 4.2 Target selection (pure, unit-tested)

```rust
/// One damaged-building candidate for drone repair dispatch.
pub struct RepairCandidate {
    pub entity: Entity,
    pub pos: GridPosition,
    /// 1.0 - current_hp / max_hp, in (0.0, 1.0].
    pub damage_fraction: f32,
    /// Tile pressure below SUFFOCATION_PRESSURE_THRESHOLD.
    pub exterior: bool,
}

pub fn pick_repair_target(
    candidates: &[RepairCandidate],
    from: GridPosition,
) -> Option<Entity>
```

Ordering, lexicographic: **exterior first**, then **damage
fraction descending**, then **Manhattan distance ascending**.
Deterministic on ties (first candidate wins). This mirrors the pop
lane's damage-fraction urgency (`utility_ai_population.rs`) with
one drone-specific axis on top: exposure.

### 4.3 Dispatch — extend `evaluate_drone_actions_system`

The existing evaluation order is preserved; repair slots in at
priority 2, between charging and hauling:

1. **Charge** (unchanged): battery < 20% → nearest active hub,
   `continue`. A repairing drone whose battery crosses 20% is
   recalled by this same branch — it abandons the repair target
   and goes home to charge. Battery law is absolute.
2. **Charge completion** (unchanged): at ≥ 99% while charging,
   return to Idle.
3. **Repair** (new): if the drone's action is `Idle`, `Haul`, or a
   stale `Repair` (its `MovementTarget` names a different building
   than the current pick, or it has no `MovementTarget`), and
   `pick_repair_target` over the colony's damaged buildings returns
   a target, then: `action.current = Repair`,
   `action.current_utility = 0.9` (above Haul's 0.8, below Charge's
   1.0 — repair outranks hauling, never outranks charging),
   `state = Repairing`, insert `StartPlan { Repair, target }`.
   If the drone already holds a `Repair` action whose
   `MovementTarget` names the current pick, do nothing (no
   StartPlan churn). If no damaged building exists and the drone
   holds `Repair`, release it to `Idle`.
   - **Candidate set**: every entity with `Building` + `Structure`
     where `current_hp < max_hp`, excluding `DeferMaintenance`
     (the player's "not automatically" marker binds drones exactly
     as it binds pop utility AI) and excluding `AncientStructure`
     (`process_repair` refuses ancients; dispatching one would
     park the drone forever).
   - **Exterior flag**: `PressureGrid` lookup at the building's
     tile; below 0.2 → exterior. No grid (unit worlds) → all
     candidates interior, ordering falls through to damage.
   - **Feral**: drones carrying `FeralDrone` (or `state == Feral`)
     are skipped entirely — the feral systems own them.
   - **Connection**: a drone carrying `GridConnection` with
     `is_connected == false` holds (no assignment). A drone with
     no `GridConnection` component is treated as connected — that
     is the pre-existing unit-spawn shape, and
     `check_drone_connection` only ever *removes* connection by
     going Feral at 5000 ticks, which the Feral skip already covers.
4. **Haul** (unchanged, downstream): `drone_work_bridge_system`
   still flips `Idle` → `Haul` in the Execution set. Because repair
   assignment runs in the AI decision chain *before* Execution each
   tick, a drone the bridge sent hauling is re-tasked to Repair on
   the next tick whenever damage exists. Haul remains the default
   filler work; repair preempts it.

Battery drain needs no change: `drone_battery_system` already
drains 0.1/tick for any non-Idle, non-Charge action — Repair is
work, and pays the work rate.

### 4.4 Execution — new `drone_repair_system`

An exclusive (`&mut World`) system in the Execution set, registered
`.after(work_execution_system)` beside the other drone systems in
`src/layer1/systems/execution.rs` — the same shape as
`work_execution_system` itself, because it calls the same
world-shaped API:

- For each drone with `PopAction.current == Repair`, an `AtTarget`
  marker, and a `MovementTarget` whose `for_action == Repair`:
  call `structure::process_repair(world, target_entity,
  DRONE_REPAIR_PER_TICK)`.
- On completion (`process_repair` returns true) or a vanished
  target: clear the drone's `MovementTarget` + `AtTarget`, set
  `action.current = Idle`, `state = Idle` — the same cleanup shape
  as pop work completion (`cleanup_pop_work_state`).
- Drones repair the *building entity directly* (their StartPlan
  target is the building, not a designation), so no Repair
  designation is created or consumed by drone work. If a pop and a
  drone repair the same building, both contributions land in the
  same `Structure` — `process_repair` is additive and idempotent
  at full HP.

Travel needs no new code: `process_start_plan_system` and
`movement_system` already move drones (the Haul lane proves it),
and work actions arrive adjacent to obstacle buildings via
`check_work_adjacency`, so a drone can service a building whose
tile it cannot stand on — including every starter-hull building.

### 4.5 Layer 2 Ready grant (`apply_layer2_ready_start` in `src/setup.rs`)

Beside the existing Command Center / redundant LifeSupport /
Generator grants:

- **Position**: the hub goes on the starter layout's one free
  interior tile, offset **(+1, −1)** from the center. Every other
  interior tile is already spoken for (Command Center (0,0),
  LifeSupport (−1,0), Lander (+1,0), redundant LifeSupport (0,+1),
  Generator (0,−1), Farms (+1,+1)/(−1,−1), Stockpile (−1,+1)).
  (+1,−1) is cardinal-adjacent to the Lander (+1,0) — a power
  source — and to the Generator (0,−1), so the hub joins the grid
  the same way the LifeSupport does, through the center cross.
- Spawn via `spawn_building(world, …, BuildingType::DroneHub,
  MaterialType::default())` (free grant, like the other L2R
  buildings — no resource deduction), record the tile in
  `OccupiedTiles`, guarded by the same "not already present"
  check the other grants use.
- After the existing `power_grid_system(world)` call, force the
  hub's `PowerConsumer.active = true` alongside the Command
  Center's — the same belt-and-braces the Command Center already
  gets, so the fleet's charge point is live at tick 0 regardless
  of grid recomputation order.
- **Fleet: 3 drones**, spawned at the hub tile with the
  `drone_spawner_bridge_system` bundle verbatim (`Drone { Idle }`,
  `ConnectedTo(hub)`, `GridConnection { is_connected: true,
  time_disconnected: 0 }`, `PopAction::default()`,
  `DroneBattery { 100 / 100 }`, `Speed { 1.0 }`,
  `UtilityWeights::default()`). Three is one hub's full complement:
  the spawner's cap is `3 × hubs`, so the grant fills the fleet to
  its natural ceiling at tick 0 and the spawner will never add a
  fourth — and if a drone is lost, the spawner replaces it through
  the ordinary lane. One drone repairs while one hauls and one
  cycles to the charger; that rotation is the maintenance lane
  working as designed.
- No other scenario changes. Classic / Ground Survival / Social
  Drama grants are byte-identical to before.

### 4.6 Observability

- Headless **STATS** gains trailing fields (the established
  append-only pattern): `drones={total} drone_repairing={count}`,
  where the second counts drones whose `PopAction` is `Repair`.
- Headless **`drones`** panel command, mirroring the `suits` /
  `tethers` panels: one line per drone — index, position, state,
  battery %, current action — plus a header with the hub count.
  This is the playtest seam: `sabotage`-damaged exterior buildings
  in a Layer 2 Ready colony (TUI scenario) can be watched being
  repaired via `drones` + STATS without new debug verbs.

### 4.7 What does NOT change

- Charge behavior, thresholds (20% / 99%), and hub selection.
- Feral threshold (5000 ticks), feral hoarding/attack/reproduction.
- The spawner cap and its 1-per-tick trickle (non-L2R colonies).
- Pop repair: designations, utility urgency, wages, skills — the
  pop lane is untouched; drones are an additional crew, not a
  replacement. A colony with no hub gets no drone help.
- Pressure, suffocation, and EVA rules: drones were never subject
  to them and still aren't. Nothing about vacuum gets softer for
  pops.

## 5. REFACTOR Phase: Quality & Design

- Target selection is a pure function over plain candidate data —
  no world access — so the priority law (exterior → damage →
  distance) is unit-tested without a schedule.
- Dispatch lives inside the existing drone evaluation, keeping one
  system as the single source of truth for "what is this drone
  doing" (charge > repair > haul), rather than a second evaluator
  that can fight the bridge.
- Execution reuses `process_repair` verbatim; there is exactly one
  place in the codebase that knows how a building regains HP.
- Register `drone_repair_system` in the real schedule and prove it
  with the schedule-level tests in §3, not by inspection.

## 6. Acceptance Criteria

- [ ] All tests in the RED phase pass, including both
      schedule-level tests
- [ ] `cargo check --all-targets` green; clippy clean on touched
      files
- [ ] `cargo test` failures no worse than the known pre-existing
      pool on clean trunk
- [ ] Layer 2 Ready world at setup: `buildings` includes exactly
      one DroneHub and 3 drones stand at its tile with full
      batteries (setup test + STATS `drones=3` shape)
- [ ] A damaged exterior LifeSupport in a Layer 2 Ready world is
      repaired by a drone through the real simulation schedule
      (setup schedule test), with STATS showing
      `drone_repairing >= 1` during the repair window
- [ ] Classic world at tick 0: `drones=0` — no scenario leakage

## 7. Technical Guidance

**Grounding (verified on trunk `927c0619`, 2026-10-15):**

- `evaluate_drone_actions_system` today: charge branch
  (`battery_pct < 0.2`, nearest *active* hub, `StartPlan` to the
  hub, utility 1.0), charge-completion branch (≥ 0.99 → Idle),
  then a literal `// 3. TODO: Haul Logic` placeholder — the Haul
  behavior actually lives in `drone_work_bridge_system`
  (Idle → Haul, utility 0.8), registered in the Execution set
  after `process_start_plan_system`. Repair dispatch belongs in
  the evaluator (AI decision chain, `src/simulation.rs`, after
  `update_building_map_system`), which runs before Execution —
  that ordering is what lets repair preempt the bridge's Haul.
- Drones are spawned (spawner + this spec's grant) with:
  `Drone`, `ConnectedTo(hub_entity)`, `GridConnection::default()`,
  `GridPosition`, `PopAction::default()`, `DroneBattery`,
  `Speed`, `UtilityWeights`. Note the spawner connects drones to
  the *hub* entity, and `check_drone_connection`'s "command
  centers" query is any `Building` with a `PowerConsumer` — a hub
  qualifies, so connection == "my hub is powered". Do not
  "fix" this to CommandCenter; it is the live behavior and the
  Feral tests depend on it.
- `GridConnection::default()` is `is_connected: false` — spawner
  drones read disconnected until `check_drone_connection` first
  runs. The §4.3 connection gate therefore delays a fresh
  spawner drone's first repair assignment by a tick, not forever;
  the L2R grant spawns with `is_connected: true` explicitly.
- Every `spawn_building` building carries `Structure` (base
  50 HP × material modifier) from
  `insert_base_building_components` — there is no separate
  "repairable" registry to join.
- `process_repair(world, entity, amount)`: resolves a designation
  *or* a direct structure entity; refuses `AncientStructure`
  (returns false without repairing); reveals `HiddenQuirk` on
  first HP gain; despawns the target only when it is a completed
  designation. Passing the building entity is the safe call.
- Pop repair urgency precedent: `utility_ai_population.rs`
  computes a damage-fraction-scaled bonus for LifeSupport and
  skips `DeferMaintenance` structures in automatic candidacy —
  §4.2/§4.3 mirror both choices for drones.
- Movement: `movement_system`'s query is
  `(Without<AtTarget>, Without<Building>)` over anything with a
  `MovementTarget` — drones qualify today (Haul proves the path).
  `check_work_adjacency` treats `ActionType::Repair` as a work
  action (arrival at Chebyshev adjacency), which is what makes
  obstacle-tile buildings (Lander, LifeSupport) serviceable.
- Power grid: membership flows through cardinal-adjacent tiles
  carrying power components (`power_grid_system`); the starter
  LifeSupport is powered exactly this way via the center cross.
  The L2R hub at (+1,−1) touches the Lander (+1,0) directly.
- STATS is one long `println!` in `print_stats`
  (`src/bin/headless.rs`); every spec appends trailing fields —
  follow suit, and add the panel command beside the other
  `print_*` panel handlers in the dispatch match.
- **EVA interplay (Spec 1384):** none mechanically. Suits protect
  pops; drones need no protection. They compose: a suited pop and
  a drone can repair the same LifeSupport simultaneously, and a
  colony can lean on drones for the hull while suits cover
  expeditions. No shared state, no hooks either way.

## 8. Questions

*Builder: add questions here if spec is unclear.*

## 9. Out of Scope

- Drone mining / chopping / general designation work. The pop
  Work pipeline (skills, tool efficiency, wages, workplace
  hazards, morale) has no drone-shaped hole in it; if repair
  lands cleanly, a follow-up spec can evaluate reusing the
  dispatch (not the pipeline) for Mine/Chop designations.
- Drone construction, demolition, hauling changes, or cargo
  capacity changes.
- Machine Awakening, Bot tech-tree effects, drone sapience —
  explicitly left alone per Mark's direction.
- Player-issued drone orders, drone TUI panels beyond the
  headless observability in §4.6, and drone naming.
- Repair of non-building `Structure`s (walls *are* buildings and
  are in scope via the `Building` filter; any future non-building
  structure is a follow-up).
- Changes to the Feral threshold, spawner cap, battery size, or
  charge rate — balance-loop territory if playtesting asks.
- Save/load changes: drones and hubs persist (or not) exactly as
  they did before this spec; the L2R grant re-applies at world
  creation only, like every other scenario grant.

## 10. Open Questions for Mark

None. The direction was decided by Mark on 2026-10-14/15 (drones
as vacuum counterplay, repair first, Layer 2 Ready starts with a
hub + fleet, Awakening lane untouched). The fleet size (3 = one
hub's full complement), the hub tile (the one free interior tile),
and the repair rate (pop parity, 10 HP/tick) are architect's
choices inside those decisions and are balance-loop territory
after landing.
