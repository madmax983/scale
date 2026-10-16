# Spec 1386: Fleet Orders — A Hand on the Helm

## 1. Overview

The Field Guide (landed `2335e2e6`) teaches the new keeper an honest
truth: *"the fleets fly themselves. No helm out there takes your
orders… The system map is not a helm. It is how you see it coming."*
That honesty was the right first step — Mark, playtesting on
2026-10-16, found his system map and his fleets and asked "how do?",
and before the guide there was no answer at all. This spec is the
sanctioned sequel Mark decided the same day ("guide now, fleet
orders specced next"): **the system map becomes a helm — for your
own fleets.**

The core verb: **open the system map, select one of your fleets,
click the world you want her to go to.** The fleet departs, crosses
at the speed the sim already computes, and the Chronicle names the
arrival. One maintenance verb rides along (hull scrape, §4.7),
because the sim already prices dirty hulls into every ETA and
already owns an unwired cleaning helper.

Two premise corrections must be stated before anything else,
because the natural first guess is wrong on both:

- **The order vehicle is the `FleetOrder` component, not the
  `FleetCommand` event.** `FleetCommand { fleet, target:
  GridPosition }` (`src/layer2/fleet.rs`) targets a *Layer 1 ground
  tile*, and its only reader in the codebase is
  `process_world_eater_system` (`src/layer2/ecophagy.rs`): if the
  named entity is a `WorldEater`, the targeted ground tile becomes
  Void and is harvested. It is a world-eater's dinner bell, not a
  movement order, and no part of this spec sends one. The fleet
  sim's real order channel is the `FleetOrder` component
  (`src/layer2/fleet.rs`): `MoveTo(Entity)` is consumed by
  `fleet_order_system`, `BuildStation` by `build_station_system`
  (`src/layer2/station.rs`), `Mine` by `fleet_mine_order_system`
  (`src/layer2/mining.rs`), `ClaimStation` by
  `claim_station_system` (`src/layer2/derelict_stations.rs`) — all
  four registered in the real simulation schedule
  (`src/simulation.rs`, ~:1173–1213). This spec wires the player
  to `FleetOrder::MoveTo` only (see §9 for why the other three
  verbs, despite having live consumers, are the follow-up spec).

- **"Your fleets" do not exist yet.** A fleet is the player's iff
  it carries `Fleet` + `FleetFaction::Player`
  (`src/layer2/fleet.rs`). In live play that set is empty, and
  every road to it is blocked:
  - No spawner creates a `FleetFaction::Player` fleet outside
    tests (mutiny / silent-mutiny / combat fixtures only).
  - The ship-construction chain is dormant at two links:
    `ShipConstruction` is inserted only in `station.rs` tests, so
    `ShipConstructionCompletedEvent` never fires in play; and if
    it did, `orbital_drydock_fleet_bridge_system`
    (`src/layer2/integration.rs`) would spawn a bare `(Fleet,
    InOrbit)` — no faction, no `FleetComposition`, no
    `OrbitalBody`, hence invisible, since `render_system_view`
    only draws entities carrying `OrbitalBody`.
  - The one designed acquisition path, hacking an orphan fleet,
    dead-ends: `HackOrphanFleetEvent` (`src/layer2/orphan_fleet.rs`)
    has no sender anywhere in the codebase.
  So this spec creates the first player fleet the same way
  Spec 1385 created the first drone fleet: **a Layer 2 Ready
  scenario grant** (§4.8). The order UI is written against *any*
  `FleetFaction::Player` fleet, so the follow-up specs that repair
  the drydock chain or wire the orphan hack inherit a working helm
  for free.

### Decided (Mark, 2026-10-16 — not open for builder reinterpretation)

- **Guide now, fleet orders next.** The guide's "fleets fly
  themselves" text gets amended by this feature (§4.10) — foreign
  fleets still fly themselves; *yours* take move orders.
- **The view gate is law.** The system view exists only behind a
  powered Command Center (`update_visibility_system`,
  `src/layer2/visibility.rs`). Orders are gated by the same rule
  (§4.5): lose the deck's power and you can neither watch nor
  command — but orders already given are carried out (§4.5, §7).
- **No silent no-ops.** Mark's awareness doctrine: every refused
  order says why, in the pane and in the log; every accepted order
  and every arrival is named in the Chronicle.
- **Classic gets no grant.** As with suits (1384) and drones
  (1385): the relief lane starts equipped, Classic starts with
  dirt and earns its sky later — via the acquisition follow-ups
  named in §9, not via a grant in this spec.

## 2. Dependencies

- Fleets: `Fleet`, `FleetFaction`, `FleetComposition`, `Ship`,
  `ShipType` (`base_speed` / `cargo_capacity`), `FleetHealth` +
  `ensure_fleet_health_system`, `InOrbit`, `InTransit`,
  `MovementSpeed`, `SpaceBarnacles` + `calculate_speed_modifier`
  (`MAX_BARNACLES`, `DRAG_PER_BARNACLE = 0.0005`, floor 0.1) —
  `src/layer2/fleet.rs`, `src/layer2/ship.rs`
- Orders in flight: `FleetOrder`, `fleet_order_system` (consumes
  `MoveTo` **only from `InOrbit`**; fixed `duration: 100.0` ticks,
  marked "Fixed duration for MVP" in the source),
  `fleet_movement_system` (progress += `(1.0 / duration) ×
  speed_mod`; arrival = remove `InTransit`, insert
  `InOrbit { destination }`, nothing else) — `src/layer2/fleet.rs`;
  registration in `src/simulation.rs` (~:1173–1213, with
  `build_station_system` / `claim_station_system`)
- View gate: `SystemVisibility { None, Full }`,
  `update_visibility_system`, `enforce_view_mode_system` (forces
  `ViewMode::Colony` when visibility is lost; carries a
  `// TODO: Add "SIGNAL LOST" notification` — §4.6 resolves that
  TODO for player fleets), `ViewMode` — `src/layer2/visibility.rs`,
  `src/shared/view_mode.rs`
- System generation: `generate_system` (star, 3–8 planets on
  `Orbit { parent: star, radius ≈ 20 + 15i ± 5, speed:
  1.0/√radius }`, moons) — `src/layer2/generation.rs`, registered
  in the startup schedule at `src/setup.rs:412`; bodies are
  `OrbitalBody { name, radius, color, char }` + `Orbit` —
  `src/layer2/system.rs` (polar model: position = parent position
  + `(radius·cos θ, radius·sin θ)`)
- Rendering: `render_system_view` (sun at pane center; bodies by
  polar projection with the terminal-aspect correction `radius_x =
  radius × 2.0`; fleets drawn at their `InOrbit` parent's cell or
  at the origin→destination lerp while `InTransit`;
  `UnidentifiedContact` rendered as grey `?`) —
  `src/layer2/render.rs`; pane wrapper `SystemMapPlugin` (render
  only, `_is_focused` ignored) — `src/ui/shell/plugins/system_map.rs`
- Sensors: `Sensors`, `SensorContact`, `UnidentifiedContact`,
  `resolve_sensors_system` (a fleet is identified iff it shares an
  `InOrbit` parent with a `Sensors`-equipped observer, for MVP),
  `assign_sensors_to_player_fleets_system` and
  `ensure_player_fleets_identified_system` (player fleets get
  `Sensors { range: 100.0 }` and are never unidentified) —
  `src/layer2/sensor_ambiguity.rs`, `src/layer2/integration.rs`
- Barnacles: `barnacle_accumulation_system` (InTransit only, 5% /
  tick, +1, cap 1000), `clean_barnacles(world, fleet)` (a `pub`
  helper with **zero production callers**), `ensure_barnacles_component_system`
  — `src/layer2/barnacles.rs`
- Cargo: `FleetCargo { contents, capacity }`, `CargoStack`,
  `current_load()` — `src/layer2/mining.rs`
- Input: `route_root_input` (palette → Esc interception for
  `context_has_own_escape` contexts → layout mode → shell → game),
  `route_root_mouse_input` (already admits
  `SYSTEM_MAP_PLUGIN_TYPE` focus; passes `map_inner: None` for it
  today — §4.4 changes what flows through), `route_mouse_input`,
  `InputContext` / `InputContextStack`, Normal-context Esc
  precedent (item 90: `handle_normal_mode` Esc clears the colony
  `Selection`, `src/ui/input.rs` ~:304, pinned by
  `test_input_router_normal_mode_esc_clears_selection`) —
  `src/ui/input.rs`; `UiShell::focused_plugin_type()` /
  `focused_pane_rect()` (shell runtime)
- Selection idiom: `Selection` / `SelectionTarget { None, Tile,
  Entity }` with `select_*` / `clear()` / `is_selected()` —
  `src/ui/selection.rs`. The system selection (§4.2) mirrors this
  API; it does not reuse the colony resource (the colony inspector
  and the off-by-one-fixed click math own that one).
- Mobile: pointer taps resolve to terminal cells and become
  `GameMouseEvent` (`cell_to_mouse`, `src/ui/mobile.rs`) and flow
  through `route_root_mouse_input` — the same river this spec
  fishes in; no hover exists anywhere in the design.
- Feedback channels: `AddChronicleEvent { text, importance }`
  (`EventImportance::{Minor, Standard, Major}`) —
  `src/layer1/core/chronicle.rs` (path as used by
  `src/layer2/orphan_fleet.rs`); `MessageLog`
  (`crate::shared::log::MessageLog`, the established "TUI
  documents the gap instead of silently eating the key" channel in
  `handle_direct_control_mode`).
- Scenarios: `src/shared/scenario.rs`; `apply_layer2_ready_start`
  in `src/setup.rs` (the Command Center / redundant LifeSupport /
  Generator / suit-floor / drone-hub grant site, Specs 1384/1385),
  the L2R starting-test that pins `SystemVisibility::Full` at
  tick 0 (`src/setup.rs` ~:2141), `add_start_scenario_intro_event`
  + `seed_field_guide_entries` (tick-0 orientation text).
- Observability pattern: headless STATS trailing fields and
  `suits` / `drones` panel commands — `src/bin/headless.rs`
  (drone fields at ~:2974, Spec 1385 §4.6 is the template).
- Persistence: `SaveSnapshot` (`src/persistence/mod.rs:398`) —
  resources + loose items + designations; **no Layer 2 entity is
  persisted today** (see §4.11).

## 3. RED Phase: Tests First

Write these failing first. Bridge and sim tests live beside the
code they pin (`src/layer2/` test modules or the fleet test module
— builder's choice, one home); UI route tests in
`src/ui/input.rs`'s test module in the item-75/90 style; the grant
tests in `src/setup.rs`'s test module, where the L2R tick-0 tests
already live.

```rust
// --- Command bridge (§4.5) ---

#[test]
fn test_issue_move_order_from_orbit_inserts_fleet_order() {
    // Player fleet InOrbit(A) (visibility Full). Issue MoveTo(B).
    // Assert: Ok; FleetOrder::MoveTo(B) on the fleet; an
    // AddChronicleEvent naming fleet and body was queued;
    // telemetry issued == 1.
}

#[test]
fn test_issue_move_order_refused_without_command_deck() {
    // SystemVisibility::None. Assert: Err; NO FleetOrder inserted;
    // SystemMapNotice set; telemetry refusals == 1.
}

#[test]
fn test_issue_move_order_refused_for_foreign_fleet() {
    // FleetFaction::Pirate fleet, otherwise valid. Assert: Err,
    // no order, notice names that she is not ours.
}

#[test]
fn test_issue_move_order_refused_for_current_orbit() {
    // Target == the fleet's InOrbit parent. Assert: Err
    // ("already in orbit"), no order inserted (in particular the
    // consumer is never handed a self-move).
}

#[test]
fn test_issue_move_order_refused_for_unidentified_contact() {
    // Target carries UnidentifiedContact. Assert: Err, no order.
}

#[test]
fn test_retask_in_transit_replaces_pending_order() {
    // Player fleet InTransit to C, holding FleetOrder::MoveTo(C2).
    // Issue MoveTo(B). Assert: Ok; order == MoveTo(B) (replaced,
    // not stacked); the confirmation text marks it as taking
    // effect on arrival (fleet_order_system acts only from
    // InOrbit — §7).
}

#[test]
fn test_order_executes_and_arrives_through_systems() {
    // fleet_order_system + fleet_movement_system composed (the
    // pairing simulation.rs registers). Fleet InOrbit(A) +
    // MoveTo(B). Run: order consumed, InTransit present; run
    // duration + margin ticks: InOrbit(B), no InTransit.
    // With SpaceBarnacles { count: 200 } the same trip takes
    // strictly longer (drag is in the ETA, not a surprise).
}

#[test]
fn test_player_fleet_arrival_is_chronicled() {
    // Schedule with the §4.6 arrival bridge registered: complete
    // a player fleet's transit. Assert: an AddChronicleEvent
    // naming fleet + destination body exists. A Pirate fleet
    // completing the same transit produces NO such event
    // (the bridge watches the player's fleets only).
}

// --- Hull scrape (§4.7) ---

#[test]
fn test_scrape_clears_barnacles_in_orbit_only() {
    // Player fleet InOrbit, SpaceBarnacles { 50 }: scrape ->
    // count == 0, chronicle queued. Same fleet InTransit:
    // refused, count unchanged.
}

// --- UI routing (§4.4) ---

#[test]
fn test_system_map_click_selects_player_fleet() {
    // route_root_mouse_input with the system map pane focused,
    // at the cell the renderer draws the charter fleet on
    // (computed through the §4.3 shared projection, not a
    // hardcoded guess). Assert: SystemMapSelection ==
    // Fleet(fleet). This test must FAIL against today's routing,
    // where the click is fed to colony selection through the
    // colony camera viewport.
}

#[test]
fn test_system_map_second_click_issues_move_order() {
    // Selection == Fleet(ours). Click body B's cell through the
    // same route. Assert: FleetOrder::MoveTo(B) on the fleet.
}

#[test]
fn test_system_map_click_foreign_fleet_selects_but_cannot_order() {
    // Click an identified Pirate fleet: selected (her card is
    // inspectable). Click body B: refusal notice, no FleetOrder.
}

#[test]
fn test_system_map_stacked_cell_prefers_own_fleet() {
    // Own fleet in orbit of body A renders on A's cell. With
    // nothing selected, a click there selects the FLEET, not A.
}

#[test]
fn test_system_map_esc_clears_system_selection_first() {
    // Route-level (route_root_input, item-75/90 style): Normal
    // context, system context active, SystemMapSelection set AND
    // colony Selection set. Bare Esc. Assert: system selection
    // cleared; colony Selection UNTOUCHED; shell not flipped.
    // Second Esc now behaves as item 90 (clears colony
    // selection).
}

#[test]
fn test_system_map_tap_equals_click() {
    // The §4.4 selection flow driven by a mobile
    // cell_to_mouse tap event lands identically.
}

#[test]
fn test_fleet_cycle_key_selects_next_player_fleet() {
    // Two player fleets, system context active: the cycle key
    // (§4.4) selects fleet 1, then fleet 2, then wraps.
}

// --- Grant & rendering (§4.8, §4.6) ---

#[test]
fn test_layer2_ready_starts_with_charter_fleet() {
    // setup_world_with_config(Layer2Ready): exactly 1 entity
    // with Fleet + FleetFaction::Player; composition is
    // [Scout, Transport]; OrbitalBody char == '▲'; InOrbit
    // parent is a Planet. Classic: 0 player fleets.
}

#[test]
fn test_charter_fleet_renders_on_system_map() {
    // render_system_view into a ratatui TestBackend buffer,
    // no selection: the '▲' glyph is present at the parent
    // body's cell, and the frame contains no selection card.
}

// --- Guide (§4.10) ---

#[test]
fn test_guide_fleet_text_matches_order_reality() {
    // The help.rs honesty pin, updated: guide states player
    // fleets take move orders AND foreign fleets remain
    // autonomous. (Replaces the 2335e2e6 pin verbatim.)
}
```

## 4. GREEN Phase: Minimal Implementation

### 4.1 The order vehicle

Issuing an order means exactly one thing: **insert (or replace)
`FleetOrder::MoveTo(target)` on the fleet entity.** The registered
`fleet_order_system` does the rest on its next run. No new event
type, no new order enum, and under no circumstances a
`FleetCommand` (see §1 — that event belongs to the world-eaters).

### 4.2 Player-fleet identity and the system selection state

- **Yours** := `Fleet` + `FleetFaction::Player`. Every part of
  this spec — selection, orders, arrival chronicle, STATS — keys
  off that pair and nothing else.
- New resource in `crate::ui` (beside `selection.rs`), mirroring
  the colony `Selection` API shape:

```rust
pub enum SystemSelectionTarget {
    None,
    Fleet(Entity),
    Body(Entity),
}

pub struct SystemMapSelection {
    target: SystemSelectionTarget,
    // + select_fleet / select_body / clear / is_selected /
    //   target, same shapes as Selection's
}
```

- New resource `SystemMapNotice { text: String, tick: u64 }` —
  the last refusal or status line, rendered by §4.6 and written by
  §4.5/§4.7. Refusals additionally go to `MessageLog`, so the
  headless console and tests can observe them without a pane.
- Neither resource is persisted (§4.11).

### 4.3 One projection for render and hit-test

`render_system_view`'s coordinate math is extracted into a shared
helper in `src/layer2/render.rs`, used by **both** the renderer
and the click hit-test, so the two can never drift (the colony map
learned this the hard way — item 89's off-by-one was two
conversions disagreeing):

```rust
/// The cell `render_system_view` draws `entity` on, inside
/// `inner` (the pane's inner area), or None if it is not drawn.
/// Bodies: polar projection about inner's center, radius_x =
/// radius * 2.0 (terminal aspect). No Orbit -> center (the star).
/// Fleets: InOrbit -> parent's cell; InTransit -> origin ->
/// destination lerp by progress.
pub fn system_entity_cell(world: &World, inner: Rect, entity: Entity)
    -> Option<(u16, u16)>;
```

`render_system_view` is refactored to draw via this helper with
byte-identical output when nothing is selected (§6 pins this).
Hit-testing inverts it: candidates are every entity the renderer
would draw (anything with `OrbitalBody`, fleets included); a click
matches a candidate on its exact cell, else the nearest candidate
within Chebyshev distance 1 (terminals are coarse; moons and
fleets stack on parent cells). **Stacking priority** at a shared
cell: player fleet → other identified fleet → station → body.
With nothing (or only a Body / foreign fleet) selected, clicking
an already-selected candidate's cell cycles to the next candidate
at that cell. With *your* fleet selected, a click is an order,
never a cycle (§4.4) — to inspect the body beneath your fleet,
Esc first.

### 4.4 Input routing — mouse, tap, keys, Esc

**System context** := the player is looking at the system map:
the system map pane is the shell's focused pane
(`focused_plugin_type() == SYSTEM_MAP_PLUGIN_TYPE`), or
`ViewMode::System` is up with the colony map pane focused.
Builders: verify which of these the shipping shell can actually
reach and pin a route test for each live path; do not build input
for a path nothing renders.

1. **Mouse/tap.** `route_root_mouse_input` already admits system
   map focus — today it then feeds the click to colony selection
   through the colony camera viewport, i.e. clicking the
   system map silently selects a colony tile under foreign
   math (§7).
   Change: when the focused pane is the system map, pass the
   focused pane rect (`focused_pane_rect()`) through
   `route_mouse_input` (an added `system_inner: Option<Rect>`
   alongside `map_inner`) to a new
   `handle_system_map_click(world, mouse, system_inner)`:
   - Nothing orderable selected → select per §4.3
     (fleet, body, or contact notice).
   - `SystemMapSelection::Fleet(f)` where `f` is yours → the
     click designates the target: resolve the body at the cell
     (§4.3) and call the §4.5 bridge. Clicking empty space keeps
     the selection and sets the notice ("no world there").
   - `Fleet(f)` where `f` is foreign → orders refuse via the
     bridge; selection clicks behave as inspection.
   - `Body(_)` selected → clicks re-select (cycle per §4.3);
     bodies take no orders.
   Taps arrive as `GameMouseEvent` at cell granularity
   (`mobile.rs::cell_to_mouse`) and take this exact path — no
   hover-dependent step exists in the flow: select is one tap,
   order is a second tap, state persists between them.
2. **Keys** (Normal context, system context only; outside system
   context these keys keep their current Normal meanings — the
   handler checks the same focus/view condition
   `route_root_mouse_input` uses, it does not steal keys from the
   colony view):
   - **Fleet cycle key**: selects the next player fleet (wrapping),
     or the first if none selected. Spec'd key: `n`. Builder:
     verify `n` is unbound in Normal-context handling; if it is
     taken, use the nearest free key and say so in the commit.
   - **`x`**: hull scrape for the selected fleet (§4.7).
3. **Esc layering** (requirement, coordinate with items 75/90):
   a bare Esc in Normal context with system context active is
   claimed by the system selection **before** the item-90 colony
   selection clearing and before any shell flip: if
   `SystemMapSelection` is non-empty, Esc clears it and is
   consumed (implemented where item 90's clearing lives,
   `src/ui/input.rs` ~:304, system check first). With no system
   selection, Esc falls through to today's exact behavior. In
   `ViewMode::System`, Esc does **not** leave the system view —
   Tab does, as the renderer's own instruction line already says.

### 4.5 The command bridge: validation, refusals, Chronicle

One bridge function in the UI layer (beside §4.4's handler), the
only code path that inserts a player-issued `FleetOrder`:

```rust
pub enum OrderRefusal {
    NoCommandDeck,        // SystemVisibility != Full
    NotYourFleet,         // no Fleet, or faction != Player
    UnidentifiedContact,  // target carries UnidentifiedContact
    NotAnOrbitParent,     // target is a fleet / has no OrbitalBody
    AlreadyInOrbit,       // target == fleet's InOrbit parent
}

pub fn issue_move_order(
    world: &mut World,
    fleet: Entity,
    target: Entity,
) -> Result<(), OrderRefusal>;
```

Validation, in order (each refusal sets `SystemMapNotice`, writes
a `MessageLog` line, and increments telemetry — §4.9):

1. **Gate**: `SystemVisibility::Full`, else `NoCommandDeck`. (The
   view itself is gated, so reaching the bridge without the deck
   should be impossible from the pane — validate anyway; tests
   and future callers use this bridge directly.)
2. **Ownership**: fleet is yours (§4.2), else `NotYourFleet`.
3. **Target legality**: target has `OrbitalBody` and is not a
   `Fleet`, else `NotAnOrbitParent`; target carries
   `UnidentifiedContact` → `UnidentifiedContact` ("close to
   identify her first" — identification is same-orbit work,
   `resolve_sensors_system`).
4. **Already there**: fleet `InOrbit` with `parent == target` →
   `AlreadyInOrbit`.
5. **Issue**:
   - `InOrbit` → insert `FleetOrder::MoveTo(target)`.
   - `InTransit` → **re-task**: insert `FleetOrder::MoveTo(target)`,
     replacing any pending order. The consumer acts only from
     `InOrbit`, so the new order takes effect on arrival at the
     current destination (§7) — the confirmation text must say
     exactly that ("re-tasked: departs for {target} on arrival
     at {destination}").
   - Neither (a fleet with no position state) → `NotYourFleet`-
     class refusal with notice "she is nowhere we can reach" —
     should be unreachable; refuse, never panic.

On success: `AddChronicleEvent` (Standard),
`"{fleet} ordered to {body}. Crossing will take about {eta}
ticks at current drag."` where eta = `ceil(100.0 / speed_mod)`
and speed_mod is `calculate_speed_modifier(barnacles)` × any
`MovementSpeed` current/base ratio — the same formula
`fleet_movement_system` will apply to the fixed 100-tick duration
(§7). Names come from `OrbitalBody.name` (fleets and bodies both
carry it once §4.8's bundle is in force).

### 4.6 Feedback in the pane; the arrival bridge

**Selection card.** `render_system_view` gains a card block at the
bottom of the inner area (above the instructions line) when
`SystemMapSelection` is non-empty. For a fleet:

```
▲ Charter Fleet — 2 ships (Scout, Transport)      [yours]
In orbit of Planet 2 · Hull {cur}/{max} · Cargo 0/1010
Barnacles: 37 (drag ×0.98) — [x] scrape while in orbit
```

```
▲ Charter Fleet — IN TRANSIT to Planet 4 · 43% · ≈57 ticks out
Orders on arrival: move to Planet 3
```

For a body: name, and what orbits there (fleet count, yours
marked). For a foreign fleet: name/faction if identified, and the
line "not yours — she takes no orders from this deck."
The instructions line becomes contextual: nothing selected →
"Click one of your fleets (▲) to select her · [Tab] colony view";
your fleet selected → "Click a world to order the {name} there ·
Esc clears". With nothing selected and no notice, renderer output
is byte-identical to today (§6).

**Notices.** `SystemMapNotice` renders under the card (or in its
place) until replaced — refusals are read where they were caused.

**Arrival bridge.** New system in `src/layer2/integration.rs`
(the bridge module — precedent:
`orbital_drydock_fleet_bridge_system` chronicling construction):
watch player fleets with `Added<InOrbit>` and send
`AddChronicleEvent` (Standard): `"{fleet} has arrived at
{body}."` Register it in `src/simulation.rs` beside the other
Layer 2 bridges and prove registration with the §3 schedule-level
test — registration by inspection is how this repo got the
unregistered-system bugs of items 77/78.

**Signal lost.** `enforce_view_mode_system` already snaps the
view home when the deck dies, next to a `TODO: Add "SIGNAL LOST"
notification`. Resolve it for the case that matters: if, at the
moment visibility drops, any player fleet is `InTransit` or holds
a pending `FleetOrder`, send one `AddChronicleEvent` (Major):
"Signal lost — the command deck is dark. Orders already given
will be carried out; no new ones can be sent." In-flight transits
and pending orders are unaffected (no Layer 2 consumer reads
`SystemVisibility` — §7); the fleet flies on, unwatched.

### 4.7 Hull scrape — the maintenance verb

Key `x` (§4.4) with your fleet selected:

- Fleet `InOrbit` and `SpaceBarnacles.count > 0` → call
  `clean_barnacles(world, fleet)` (`src/layer2/barnacles.rs` —
  this spec is its first production caller), Chronicle (Minor):
  `"{fleet}'s hull scraped clean."`
- Fleet `InTransit` → refusal notice: "Can't scrape underway —
  she has to be in orbit." (The helper's own test has always
  assumed orbit; v1 enforces it at the bridge.)
- Count == 0 → notice "Her hull is already clean." Not a
  Chronicle event; not a refusal for telemetry (it is a no-op the
  player asked about, answered in the pane).

Barnacles accumulate only in transit (5%/tick) and drag is
already priced into §4.5's ETA text and the card's transit line,
so the loop is legible end to end: fly → foul → arrive → scrape.

### 4.8 The Layer 2 Ready charter fleet grant

In `apply_layer2_ready_start` (`src/setup.rs`), beside the
Command Center / LifeSupport / Generator / suit / drone grants —
one fleet, the **Charter Fleet**, spawned with the full bundle
every consumer and the renderer actually require:

- `Fleet`, `FleetFaction::Player`
- `FleetComposition` — one `Ship::new(ShipType::Scout)` + one
  `Ship::new(ShipType::Transport)` (the composition
  `fleet.rs`'s own tests use: cargo 1010, slowest-ship speed 0.5)
- `FleetCargo { contents: vec![], capacity: 1010.0 }` (=
  `composition.total_cargo_capacity()`)
- `OrbitalBody { name: "Charter Fleet", radius: 0.0, color:
  <distinct>, char: '▲' }` — '▲' is the fleet glyph from
  `fleet.rs`'s spawn test; the color must read as *yours* at a
  glance: distinct from the sun's Yellow, from the grey `?`, and
  from station Cyan; builder picks from the ratatui palette and
  the §3 render test pins the glyph (color requirement is
  reviewed, not machine-pinned). `radius: 0.0` — fleets are
  points, per the same test's comment.
- `InOrbit { parent: <innermost planet> }` — the `Planet` entity
  with the smallest `Orbit.radius` in the generated system.
- `SpaceBarnacles::default()` (the ensure system would add it;
  explicit is kinder), and nothing else: `FleetHealth` arrives via
  `ensure_fleet_health_system`, `Sensors` and identification via
  the integration bridges — do not duplicate their work.

**Ordering checkpoint (builder must verify, test is the
arbiter):** `generate_system` runs in the startup schedule
(`src/setup.rs:412`); scenario application happens inside world
setup. If bodies do not exist yet when
`apply_layer2_ready_start` runs, spawn the fleet from a one-shot
setup step ordered after generation — do not invent a second
grant site in the sim schedule. §3's grant test is written against
the fully set-up world either way.

No other scenario changes: Classic / Ground Survival / Social
Drama worlds contain zero player fleets, byte-identical to today.

### 4.9 Observability

Following the append-only STATS pattern of Specs 1384/1385
(`src/bin/headless.rs`):

- STATS gains trailing fields: `player_fleets={n}
  fleet_orders={issued} fleet_order_refusals={refused}`.
  `player_fleets` is counted at print time; issued/refused come
  from a new `FleetOrderTelemetry` resource (plain counters,
  incremented only by §4.5's bridge — AI-issued orders, e.g. the
  mycelial network's, are not the player's and are not counted).
- Headless **`fleets`** panel command, mirroring `suits` /
  `drones`: header counts, then one line per player fleet —
  name, ships, location (`In orbit of {body}` / `In transit to
  {body}, {pct}%`), ETA ticks if transiting, barnacles, pending
  order if any. With no player fleets it prints the count and the
  reminder that Layer 2 Ready starts with a charter fleet.
- Telemetry is session state, not persisted (§4.11) — same rule
  as the other run counters.

### 4.10 Field Guide amendment (same feature, not a follow-up)

The guide landed at `2335e2e6` now lies about the helm, so the
implementing builder amends it in the same commit series:

- `src/ui/help.rs` (~:399–408): the passage "the fleets fly
  themselves… The system map is not a helm" is rewritten to the
  new truth: *your* fleets take move orders from the system map
  (select her, click the world); foreign fleets still fly
  themselves; the deck's power still gates everything; a fouled
  hull crosses slowly and can be scraped in orbit.
- The pinning test (~:669, "guide must be honest that fleets are
  not player-flown") is replaced by §3's
  `test_guide_fleet_text_matches_order_reality`.
- The Layer 2 Ready tick-0 orientation (the guide pass's
  kit list) names the Charter Fleet alongside the drone hub and
  the suits, so a new keeper learns she has a fleet from the
  Chronicle, not from a rumor.

### 4.11 What does NOT change

- `fleet_order_system`, `fleet_movement_system`, and the other
  three order consumers: untouched. Duration stays the fixed
  100.0 ticks; `FleetComposition::speed()` stays unused by
  movement (§7) — surfacing real composition speed is a sim
  change, not a UI one.
- `FleetCommand` and the world-eaters: untouched.
- Foreign fleets: no new AI, no new orders, no renderer changes
  for them (§9 notes their invisibility quirk and leaves it).
- Persistence: fleets and in-flight orders are **not** saved.
  `SaveSnapshot` persists no Layer 2 entity today — bodies,
  stations, and fleets alike — and v1 follows that rule: on load
  the system is regenerated and the L2R grant re-applies at world
  creation, exactly like every other scenario grant (Spec 1385
  §9's precedent). Persisting Layer 2 is a named follow-up (§9).
- The colony map, colony selection, and their click math:
  untouched. System clicks stop leaking into colony selection
  (§4.4) as a side effect of correct routing, not by editing the
  colony path.

## 5. REFACTOR Phase: Quality & Design

- Exactly one projection (`system_entity_cell`) serves renderer
  and hit-test; exactly one bridge (`issue_move_order`) inserts
  player orders; exactly one bridge system reports arrivals. Each
  singularity is the point: the bugs this feature replaces were
  all "two paths, one of them wrong."
- The selection resource mirrors `Selection`'s API so a builder
  who knows the colony idiom already knows this one; it stays a
  separate resource so neither domain's semantics bend.
- Validation lives at issue time, in the bridge — the sim
  consumers keep their current (sometimes silent) behavior for
  AI-issued orders, and the player's path never relies on it.

## 6. Acceptance Criteria

- [ ] All tests in the RED phase pass, including the route-level
      Esc test and both schedule-level sim tests
- [ ] `cargo check --all-targets` green; clippy clean on touched
      files
- [ ] `cargo test` failures no worse than the known pre-existing
      pool on clean trunk
- [ ] Layer 2 Ready world at setup: exactly one
      `FleetFaction::Player` fleet, composition [Scout,
      Transport], in orbit of the innermost planet; STATS shows
      `player_fleets=1`; Classic shows `player_fleets=0`
- [ ] Through the real input route on the focused system map:
      click own fleet → click Planet B → the fleet departs,
      arrives, and the Chronicle carries both the order line and
      the arrival line (route + schedule tests are the proof;
      headless `fleets` panel shows her mid-transit with an ETA)
- [ ] With the Command Center unpowered, the system view is gone
      (existing gate) and the bridge refuses with
      `NoCommandDeck`; a fleet already in transit completes her
      crossing and is chronicled on arrival when power returns
- [ ] Renderer output with nothing selected is byte-identical to
      trunk `2335e2e6`'s (existing render behavior is the control)
- [ ] Field Guide text and its pinning test describe the new
      reality (§4.10)

## 7. Technical Guidance

**Grounding (verified on trunk `2335e2e6`, 2026-10-16):**

- **`FleetOrder::MoveTo` only fires from `InOrbit`.**
  `fleet_order_system` queries `Option<&InOrbit>`; without it, the
  order is left in place, unconsumed. Two consequences this spec
  builds on: (a) an order inserted mid-transit is a *queued*
  order — it executes the tick the fleet next holds `InOrbit`;
  §4.5's re-task semantics are the consumer's real behavior, not
  an invention. (b) A `MoveTo` naming the fleet's current parent
  would depart and "arrive" at the same body 100 ticks later —
  §4.5 refuses it at issue instead.
- **The other consumers' traps are why they are not in v1.**
  `fleet_mine_order_system` *consumes* an invalid Mine order
  silently (wrong orbit or full cargo — the order vanishes);
  `claim_station_system` and `build_station_system` leave an
  unmeetable order (wrong orbit, unaffordable) inserted forever,
  retried every tick, saying nothing. A player UI may not issue
  into either failure mode; §9 scopes the follow-up that fixes
  the consumers or validates at issue for those verbs.
- **Duration is a constant wearing a speed costume.** Every
  transit is `duration: 100.0`; per tick, progress advances
  `(1/100) × speed_mod`, where speed_mod = barnacle modifier
  (`(1 − count × 0.0005)`, floored at 0.1) × `MovementSpeed`
  current/base when the component exists (most fleets never get
  one — it is `Option` in the movement query and no ensure-system
  adds it). Distance between bodies does not exist in the model:
  Planet 1 → Planet 2 costs the same 100 ticks as Planet 1 →
  Planet 8. ETA text (§4.5/§4.6) is computed from these facts and
  labelled "at current drag" because barnacles keep accruing in
  transit (5%/tick) — an ETA is a promise the hull keeps
  renegotiating.
- **Arrival is silent in the sim.** `fleet_movement_system`
  inserts `InOrbit` and nothing else — no event, no chronicle.
  §4.6's `Added<InOrbit>` bridge is the smallest honest seam, and
  it fires for *any* insertion of `InOrbit` on a player fleet,
  including the §4.8 grant: the grant runs at setup, before the
  sim schedule the bridge is registered in, so no spurious
  "arrived" line appears at tick 0 (builder: pin this in the
  arrival test by asserting the grant alone produces no event
  once the schedule runs — if `Added` semantics surprise you,
  gate the bridge on a prior `InTransit` having existed via a
  marker the movement system already removes… it removes
  `InTransit` itself; a fleet that has never transited is the
  only false positive, and the setup-ordering above is the fix —
  do not invent a tracking component without a failing test
  demanding one).
- **Clicks on the system map currently go to the colony.**
  `route_root_mouse_input` admits `SYSTEM_MAP_PLUGIN_TYPE` focus
  but passes `map_inner: None`, and `route_mouse_input` then runs
  `handle_selection_click` with the colony camera viewport —
  a keeper clicking her fleet is selecting a ground tile under
  colony math, in the lineage of the item-89/90 click bugs.
  §4.4's branch is the fix, and §3's first route test is written
  to fail against exactly this.
- **The renderer draws less than the sim contains.** Only
  `OrbitalBody` carriers render; several AI spawners omit it
  (e.g. `phantom_signal.rs` spawns `(Fleet, InOrbit)` bare), so
  some fleets are invisible on the map today. Out of scope (§9) —
  but it is why §4.8's bundle spells `OrbitalBody` out in full:
  a player fleet that does not render does not exist, whatever
  the ECS says.
- **Player fleets are sensor platforms.**
  `assign_sensors_to_player_fleets_system` gives every
  `FleetFaction::Player` fleet `Sensors { range: 100.0 }`, and
  `resolve_sensors_system` identifies any fleet sharing an
  `InOrbit` parent with a sensor platform. Moving your fleet
  therefore *identifies the traffic where she docks* — worth a
  line in the guide amendment (§4.10), and the reason §4.5 can
  promise identification is co-orbit work, not magic.
- **There is no home anchor.** `HomeColony` (`integration.rs:316`)
  is never inserted anywhere in live code, and Layer 1's colony
  has no Layer 2 body representing it. That is why the grant
  berths the Charter Fleet at the innermost planet (§4.8) and why
  "return home" is not a verb (§9): there is currently no *home*
  entity to return to. Naming one is a follow-up's first job.
- **Chronicle is the established bridge pattern.** Layer 2 systems
  speak to the player through `AddChronicleEvent` bridges in
  `integration.rs` (drydock completions, defection, tolls). §4.6
  follows that pattern rather than inventing a notification
  channel; pane-local, transient feedback goes through
  `SystemMapNotice` + `MessageLog` (§4.5), matching how
  `handle_direct_control_mode` documents its gap today.
- **STATS is append-only.** Every observability spec (1384 §, 1385
  §4.6) appends trailing fields to the one long `println!` in
  `print_stats` and adds panel commands beside the other
  `print_*` handlers in `src/bin/headless.rs`. §4.9 follows suit,
  literally.

## 8. Questions

*Builder: add questions here if spec is unclear.*

## 9. Out of Scope

- **Mine / ClaimStation / BuildStation player orders.** The
  consumers are live (`fleet_mine_order_system`,
  `claim_station_system`, `build_station_system`) and the verbs
  are the obvious follow-up — but each carries a silent-failure
  trap (§7) that a player-facing UI may not inherit. The
  follow-up spec either fixes the consumers to report failure or
  validates their preconditions (co-orbit, cargo, affordability)
  at issue through this spec's bridge. Wiring the drydock's
  `ShipConstruction` insertion belongs to that same follow-up.
- **Combat / attack orders.** `FleetOrder` has no attack verb and
  no consumer resolves one from an order (`combat.rs` resolves
  battles between compositions through its own entry points).
  An attack verb is a new sim system wearing a UI costume.
- **Trade-route orders.** Routes run on their own system
  (`src/layer2/trade/routes.rs`); no order consumer touches them.
- **Patrol, waypoint queues, mid-transit abort.** No consumer
  cycles waypoints, and `fleet_order_system` cannot act
  mid-transit; re-tasking (§4.5) is the v1 substitute for all
  three.
- **One-click "return home."** There is no home entity to return
  to (§7); `MoveTo` to the berth body covers the need until a
  follow-up names one.
- **Per-ship orders / fleet splitting.** Every consumer —
  health, cargo, speed, mining — treats `FleetComposition`
  atomically. See §10, fork 1.
- **Ship construction and fleet acquisition wiring** (drydock
  chain repair, a sender for `HackOrphanFleetEvent`). This spec's
  helm works for any `FleetFaction::Player` fleet those fixes
  later deliver; it delivers none itself beyond §4.8's grant.
- **Layer 2 persistence.** No body, station, or fleet is in
  `SaveSnapshot` today (§4.11); persisting the system layer is
  its own spec.
- **Invisible AI fleets.** Spawners that omit `OrbitalBody`
  (§7) leave sim fleets undrawn; a spawner/renderer audit is its
  own pass. The player's fleet always renders (§4.8).
- **AI faction reactions to player movement.** No perception
  consumer reads player fleet positions for faction decisions.
  (Pirates already prey on in-orbit fleets generically —
  `src/layer3/pirates.rs` queries `Fleet` + `InOrbit` without
  faction fear — so moving is already not free.) See §10, fork 2.
- **Composition-based travel time.** Making duration a function
  of `FleetComposition::speed()` and orbital distance is a sim
  rebalance with balance-loop implications, not a UI spec's
  side effect.

## 10. Open Questions for Mark

Two genuine taste forks. Each has a recommended default; if Mark
does not answer, builders proceed with the default and the
question is closed, not carried.

1. **Control granularity: per-fleet or per-ship?**
   *Recommended default: per-fleet.* Every consumer in the sim —
   health, cargo, mining, movement — treats the fleet as one
   atomic body, and one helm per fleet is the Stellaris-shaped
   verb the system map can honestly offer today. Per-ship orders
   would require composition splitting, per-ship state, and new
   consumers: a sim project, not a UI one. (This spec is written
   per-fleet throughout; choosing otherwise re-opens it.)
2. **Should AI factions visibly react to your fleets moving?**
   *Recommended default: not in v1.* Today nothing watches your
   movements — pirates and repo fleets interact with fleets they
   find in orbit, but no merchant reroutes around you and no
   rival shadows your crossing. Deliberate reaction (merchants
   avoiding a system you've entered in force, pirates trailing a
   fat transport) is a perception system of its own and a
   follow-up spec once the helm exists. The default keeps v1
   honest: the sky does not care that you moved — yet.

*Not a question, an architect's note:* the Charter Fleet's
composition (Scout + Transport) is borrowed from `fleet.rs`'s own
test fixture rather than tuned; like Spec 1385's hub tile and
repair rate, it is balance-loop territory after landing, along
with the 100-tick flat crossing time (§9) once keepers can
actually route around trouble.
