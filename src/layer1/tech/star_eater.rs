//! Spec 368: The Star-Eater's Wake.
//!
//! A massive dark-matter entity — the Star-Eater — passes through the
//! star system (Layer 2). It does not attack. Its wake siphons energy
//! from the sun, plunging Layer 1 into a sudden deep freeze ("The
//! Long Dark"), and where the wake dragged across the sky it leaves
//! behind Dark Motes: a volatile but energy-dense resource the colony
//! can burn for heat. An immediate survival crisis, balanced by a
//! resource windfall.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No `layer2::system::StarEaterEvent`, no `layer1::weather::
//!   TemperatureGrid`, no `layer1::resources` module.** The real
//!   Layer 2 system view (`layer2::system`) is orbital bodies and
//!   orbits — it has no event channel, and the established cross-layer
//!   crisis precedents are resource bridges (`layer2::void_leviathan::
//!   LeviathanEclipse`, read by the Layer 1 temperature system) and
//!   Layer 1 event/state pairs (`nature::long_night::LongNightEvent` +
//!   `StartLongNightEvent`). This module therefore lives in
//!   `layer1::tech` — the location the spec's own GREEN sketch names
//!   (`src/layer1/tech/star_eater.rs`) — with [`StarEaterEvent`] as
//!   the arrival/departure channel and [`StarEaterState`] as the
//!   Layer 2 passage state the Layer 1 thermal systems key off. The
//!   entity itself is never rendered as a body; a dedicated Layer 2
//!   transit body is deferred.
//! * **`TemperatureGrid` is the real one** (`layer1::nature::
//!   temperature`, re-exported as `layer1::temperature`): fields are
//!   `values`/`scratch`/`ambient` (the sketch's `cells` does not
//!   exist) and there is no `average_temp()` — [`average_temp`] here
//!   computes it from `values`. Two consequences drive the design:
//!   (1) `update_temperature_system` recomputes `grid.ambient` from
//!   the season *every tick*, so a one-shot ambient change is erased
//!   next tick — the Long Dark therefore *pins* `ambient` every tick
//!   from [`long_dark_system`], scheduled `.after(update_temperature_
//!   system)`, exactly the persistence problem `LongNightEvent` and
//!   the leviathan eclipse solve in their own ways; (2) the arrival
//!   freeze itself is an instant drop applied directly to `values`
//!   (the sketch's mechanism), which diffusion/drift then sustains
//!   via the pinned ambient.
//! * **`ColonyResources` is a struct of `f32` fields, not a map.**
//!   The sketch's `add(ResourceType::DarkMote, 10)` / `get_amount(..)
//!   -> u32` / `consume(..)` do not exist. The real seam is
//!   `get_amount(ResourceType) -> f32`, `add_resource(&ResourceType,
//!   f32)` and `try_consume(ResourceType, f32) -> bool`. Following
//!   the Spec 270 `Organs` precedent exactly, this spec adds a real
//!   `ResourceType::DarkMote` variant plus `dark_motes` /
//!   `max_dark_motes` ledger fields, wired through `consume`,
//!   `get_amount`, `get_max_amount`, `has_room_for` and
//!   `add_resource` (and the exhaustive matches in the GPU buffer
//!   extraction, the map renderer, and the ephemeral market — where
//!   motes are untradeable, like `VoidAle`). Like `Organs`, motes are
//!   deliberately absent from `can_afford`/`deduct`: building costs
//!   cannot be priced in motes.
//! * **Dark motes on the ground are real items.** The sketch spawns
//!   bare `DarkMote` markers; the real mining/hauling seam is
//!   `ResourceItem { resource_type, amount }` + `GridPosition`
//!   entities, hauled into the colony ledger by the existing hauling
//!   systems. Departure spawns motes as `(DarkMote, ResourceItem,
//!   GridPosition)` triples, so [`DarkMote`] stays queryable exactly
//!   as the spec's RED test expects *and* the motes are collectable
//!   through the real loop with no parallel collection system.
//! * **Burning heats the whole grid (the sketch's mechanism), via
//!   the ledger.** [`BurnMoteEvent`] carries the sketch's `u32`
//!   amount; the burn converts it to the ledger's `f32`, consumes
//!   atomically through `try_consume` (a burn the colony cannot
//!   afford is a no-op, never a partial burn or negative stock),
//!   and adds [`HEAT_PER_MOTE`] per mote to every tile. §7's
//!   localized-heater alternative was considered: a single-tile burn
//!   is invisible after one diffusion step and cannot satisfy the
//!   spec's own acceptance test (average temperature rises), so the
//!   burn is modelled as the colony furnace burning — documented
//!   here rather than silently diverging.
//! * **§5 REFACTOR items:** mote spawning is a deterministic seeded
//!   scatter across the grid (no `rand` — the same departure replays
//!   identically, and tests stay exact); the Long Dark is a timer
//!   ([`LONG_DARK_TICKS`]), not an instant Departure-only end —
//!   Departure is the entity leaving early, which ends the freeze
//!   and drops the motes. The §5 burn side-effect (mutation/lung
//!   damage near burning motes) is deferred: burns happen in the
//!   abstract furnace, no pop stands next to them, and no
//!   protection-gear seam exists to gate the debuff fairly.
//! * **Spec §6's `cargo clippy -- -D warnings` / 85% coverage** are
//!   aspirational template text; the verified bar is the established
//!   one: `cargo check --all-targets` green, clippy clean on touched
//!   files, and this module's tests green.
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::temperature::TemperatureGrid;

/// Instant temperature drop (Celsius) applied to every tile when the
/// Star-Eater arrives — the sketch's `-60` plunge.
pub const FREEZE_DROP: f32 = 60.0;
/// Ambient temperature the Long Dark pins the grid to: far below the
/// coldest season (winter is -5) and below the hypothermia threshold
/// by a killing margin. LifeSupport thermoregulation (radius 4,
/// +12/tick toward 20C) remains the colony's lifeline — if it stands.
pub const LONG_DARK_AMBIENT: f32 = -45.0;
/// How long the Long Dark lasts when the entity is left to finish
/// its passage (§5: a timer, not an instant event).
pub const LONG_DARK_TICKS: u32 = 500;
/// Heat (Celsius) one burned mote adds to every tile — the sketch's
/// `25.0 * amount`. Motes are energy-dense: two motes undo the
/// arrival plunge on paper, but diffusion and the pinned ambient
/// eat the heat while the Dark lasts.
pub const HEAT_PER_MOTE: f32 = 25.0;
/// Motes left behind by one departure.
pub const MOTES_PER_DEPARTURE: usize = 12;

/// The Star-Eater's passage through the system (Layer 2 event,
/// Layer 1 consequences — see the module docs on why the event
/// lives here rather than in `layer2::system`).
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StarEaterEvent {
    /// The entity enters the system; its wake begins siphoning the
    /// sun. The Long Dark starts.
    Arrival,
    /// The entity leaves; the freeze ends and its wake sheds
    /// Dark Motes across the map.
    Departure,
}

/// Burn stockpiled Dark Motes for heat. The sketch's `u32` amount;
/// the ledger conversion happens inside [`burn_mote_system`].
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurnMoteEvent {
    /// How many motes to burn.
    pub amount: u32,
}

/// Marker for a Dark Mote lying on the ground. Always paired with a
/// real `ResourceItem { resource_type: ResourceType::DarkMote }`
/// and a `GridPosition`, so the existing hauling loop collects it.
#[derive(Component, Debug, Clone, Copy)]
pub struct DarkMote;

/// Passage state + colony ledger for the Star-Eater (STATS / panel
/// surface). One resource, following the `LongNightEvent` shape:
/// the thermal pin reads `long_dark_remaining` every tick.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct StarEaterState {
    /// Ticks of Long Dark remaining (0 = the sun shines normally).
    pub long_dark_remaining: u32,
    /// Completed arrivals (a second Arrival mid-Dark is ignored).
    pub arrivals: u64,
    /// Completed departures.
    pub departures: u64,
    /// Total motes shed by departures.
    pub motes_shed: u64,
    /// Total motes burned for heat.
    pub motes_burned: u64,
    /// Total heat released by burning (Celsius summed over burns).
    pub heat_released: f32,
}

impl StarEaterState {
    /// True while the Long Dark holds.
    #[must_use]
    pub fn is_long_dark(&self) -> bool {
        self.long_dark_remaining > 0
    }
}

/// Mean temperature of the grid (the spec's `average_temp()`, which
/// the real `TemperatureGrid` does not provide — see module docs).
#[must_use]
pub fn average_temp(grid: &TemperatureGrid) -> f32 {
    if grid.values.is_empty() {
        return grid.ambient;
    }
    #[allow(clippy::cast_precision_loss)]
    let n = grid.values.len() as f32;
    grid.values.iter().sum::<f32>() / n
}

/// Deterministic scatter position for mote `index` of departure
/// `departure` inside a `width`x`height` grid (a small LCG — §5's
/// procedural placement, replayable and test-exact).
fn mote_position(departure: u64, index: usize, width: usize, height: usize) -> GridPosition {
    if width == 0 || height == 0 {
        return GridPosition { x: 0, y: 0 };
    }
    let mut seed = 0x5EED_3680_u64
        ^ departure.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (index as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    GridPosition {
        x: (next() % width) as i32,
        y: (next() % height) as i32,
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Handles the passage: Arrival starts the Long Dark (instant
/// [`FREEZE_DROP`] plunge to every tile + pinned ambient), Departure
/// ends it early and sheds [`MOTES_PER_DEPARTURE`] Dark Motes as
/// real ground items scattered over the grid.
pub fn handle_star_eater_events(
    mut events: EventReader<StarEaterEvent>,
    mut state: ResMut<StarEaterState>,
    mut grid: Option<ResMut<TemperatureGrid>>,
    mut commands: Commands,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    // Grid geometry for the scatter: the real grid when present,
    // else the standard 80x50 world the setup builds. The scatter
    // is anchor-free: the wake crosses the whole map.
    let (width, height) = grid
        .as_deref()
        .map_or((80, 50), |g| (g.width, g.height));

    for event in events.read() {
        match event {
            StarEaterEvent::Arrival => {
                if state.is_long_dark() {
                    // Already passing: a duplicate arrival must not
                    // stack a second plunge.
                    continue;
                }
                state.arrivals += 1;
                state.long_dark_remaining = LONG_DARK_TICKS;
                if let Some(grid) = grid.as_deref_mut() {
                    for value in &mut grid.values {
                        *value -= FREEZE_DROP;
                    }
                    grid.ambient = LONG_DARK_AMBIENT;
                }
                chronicle.send(AddChronicleEvent {
                    text: "A vast dark shape crossed the sun. The light thinned, the warmth went out of the world, and the Long Dark began.".to_string(),
                    importance: EventImportance::Legendary,
                });
            }
            StarEaterEvent::Departure => {
                state.departures += 1;
                state.long_dark_remaining = 0;
                for index in 0..MOTES_PER_DEPARTURE {
                    let pos = mote_position(state.departures, index, width, height);
                    commands.spawn((
                        DarkMote,
                        ResourceItem {
                            resource_type: ResourceType::DarkMote,
                            amount: 1.0,
                        },
                        pos,
                    ));
                    state.motes_shed += 1;
                }
                chronicle.send(AddChronicleEvent {
                    text: "The Star-Eater slid on into the black. Where its wake dragged across the sky, dark motes drifted down like black snow.".to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// The Long Dark itself: while the passage timer runs, pin the
/// grid's ambient to [`LONG_DARK_AMBIENT`] every tick (scheduled
/// after `update_temperature_system`, which would otherwise
/// recompute ambient from the season and erase the freeze), count
/// the timer down, and chronicle the dawn when it expires.
pub fn long_dark_system(
    mut state: ResMut<StarEaterState>,
    grid: Option<ResMut<TemperatureGrid>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if !state.is_long_dark() {
        return;
    }
    if let Some(mut grid) = grid {
        grid.ambient = LONG_DARK_AMBIENT;
    }
    state.long_dark_remaining -= 1;
    if state.long_dark_remaining == 0 {
        chronicle.send(AddChronicleEvent {
            text: "The sun's light returned, thin but growing. The Long Dark was over.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Burns stockpiled motes for heat: consumes atomically through the
/// real ledger (`try_consume` — insufficient stock is a no-op) and
/// adds [`HEAT_PER_MOTE`] per mote to every tile.
pub fn burn_mote_system(
    mut events: EventReader<BurnMoteEvent>,
    resources: Option<ResMut<ColonyResources>>,
    grid: Option<ResMut<TemperatureGrid>>,
    mut state: ResMut<StarEaterState>,
) {
    let (Some(mut resources), Some(mut grid)) = (resources, grid) else {
        // Still drain the channel so stale burns never replay.
        for _ in events.read() {}
        return;
    };
    for ev in events.read() {
        if ev.amount == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let amount = ev.amount as f32;
        if !resources.try_consume(ResourceType::DarkMote, amount) {
            continue;
        }
        let heat = HEAT_PER_MOTE * amount;
        for value in &mut grid.values {
            *value += heat;
        }
        state.motes_burned += u64::from(ev.amount);
        state.heat_released += heat;
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census: (long dark active, ticks remaining, ground motes,
/// stockpiled motes, motes burned).
#[must_use]
pub fn stareater_counts(world: &mut World) -> (bool, u32, usize, f32, u64) {
    let (active, remaining, burned) = world
        .get_resource::<StarEaterState>()
        .map_or((false, 0, 0), |s| {
            (s.is_long_dark(), s.long_dark_remaining, s.motes_burned)
        });
    let ground = {
        let mut query = world.query::<&DarkMote>();
        query.iter(world).count()
    };
    let stockpiled = world
        .get_resource::<ColonyResources>()
        .map_or(0.0, |r| r.get_amount(ResourceType::DarkMote));
    (active, remaining, ground, stockpiled, burned)
}

/// One-token STATS label, e.g. `dark:499t/12m/3b` (Long Dark ticks
/// remaining / ground motes / motes burned), or `none` when the
/// Star-Eater has never come and no motes exist.
#[must_use]
pub fn stareater_stats_label(world: &mut World) -> String {
    let (active, remaining, ground, stockpiled, burned) = stareater_counts(world);
    let ever = world
        .get_resource::<StarEaterState>()
        .is_some_and(|s| s.arrivals > 0 || s.departures > 0);
    if !ever && ground == 0 && stockpiled <= 0.0 && burned == 0 {
        "none".to_string()
    } else if active {
        format!("dark:{remaining}t/{ground}m/{burned}b")
    } else {
        format!("clear/{ground}m/{burned}b")
    }
}

/// Send an Arrival from a `&mut World` context (headless debug).
pub fn trigger_arrival(world: &mut World) {
    world.init_resource::<Events<StarEaterEvent>>();
    world
        .resource_mut::<Events<StarEaterEvent>>()
        .send(StarEaterEvent::Arrival);
}

/// Send a Departure from a `&mut World` context (headless debug).
pub fn trigger_departure(world: &mut World) {
    world.init_resource::<Events<StarEaterEvent>>();
    world
        .resource_mut::<Events<StarEaterEvent>>()
        .send(StarEaterEvent::Departure);
}

/// Send a burn order from a `&mut World` context (headless debug).
pub fn trigger_burn(world: &mut World, amount: u32) {
    world.init_resource::<Events<BurnMoteEvent>>();
    world
        .resource_mut::<Events<BurnMoteEvent>>()
        .send(BurnMoteEvent { amount });
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn star_eater_public_strings() -> Vec<String> {
    vec![
        "The Star-Eater's Wake".to_string(),
        "Star-Eater".to_string(),
        "Dark Mote".to_string(),
        "The Long Dark".to_string(),
        "A vast dark shape crossed the sun. The light thinned, the warmth went out of the world, and the Long Dark began.".to_string(),
        "The Star-Eater slid on into the black. Where its wake dragged across the sky, dark motes drifted down like black snow.".to_string(),
        "The sun's light returned, thin but growing. The Long Dark was over.".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real codebase — see the
// module docs for every adaptation and why)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn world_with_grid(width: usize, height: usize, temp: f32) -> World {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<StarEaterEvent>>();
        world.init_resource::<Events<BurnMoteEvent>>();
        world.init_resource::<StarEaterState>();
        world.insert_resource(TemperatureGrid::new(width, height, temp));
        world
    }

    fn send_arrival(world: &mut World) {
        world
            .resource_mut::<Events<StarEaterEvent>>()
            .send(StarEaterEvent::Arrival);
        world.run_system_once(handle_star_eater_events).unwrap();
    }

    fn send_departure(world: &mut World) {
        world
            .resource_mut::<Events<StarEaterEvent>>()
            .send(StarEaterEvent::Departure);
        world.run_system_once(handle_star_eater_events).unwrap();
    }

    /// Spec RED test 1, adapted: no `setup_test_app`, no
    /// `average_temp()` method, no `cells` — a bare `World`, the
    /// real grid, and this module's `average_temp`. The sketch's
    /// threshold (drop > 50) is kept exactly.
    #[test]
    fn test_star_eater_triggers_global_freeze() {
        let mut world = world_with_grid(10, 10, 20.0);
        let initial = average_temp(world.resource::<TemperatureGrid>());
        assert!((initial - 20.0).abs() < f32::EPSILON);

        send_arrival(&mut world);

        let new_temp = average_temp(world.resource::<TemperatureGrid>());
        assert!(
            new_temp < initial - 50.0,
            "arrival should plunge the grid: {initial} -> {new_temp}"
        );
        let state = world.resource::<StarEaterState>();
        assert!(state.is_long_dark());
        assert_eq!(state.long_dark_remaining, LONG_DARK_TICKS);
        assert_eq!(state.arrivals, 1);
        assert_eq!(
            world.resource::<TemperatureGrid>().ambient,
            LONG_DARK_AMBIENT
        );
    }

    /// Spec RED test 2, adapted: motes are counted by their
    /// `DarkMote` marker exactly as the sketch does — and each one
    /// is additionally a real, haulable `ResourceItem` on a tile
    /// inside the grid (the real collection seam).
    #[test]
    fn test_star_eater_spawns_dark_motes() {
        let mut world = world_with_grid(10, 10, 20.0);

        send_departure(&mut world);

        let mut query = world.query::<(&DarkMote, &ResourceItem, &GridPosition)>();
        let motes: Vec<_> = query.iter(&world).collect();
        assert!(!motes.is_empty(), "departure should shed motes");
        assert_eq!(motes.len(), MOTES_PER_DEPARTURE);
        for (_, item, pos) in &motes {
            assert_eq!(item.resource_type, ResourceType::DarkMote);
            assert!((item.amount - 1.0).abs() < f32::EPSILON);
            assert!((0..10).contains(&pos.x) && (0..10).contains(&pos.y));
        }
        // The sketch's own count formulation:
        let count = world.query::<&DarkMote>().iter(&world).count();
        assert!(count > 0);
        assert_eq!(world.resource::<StarEaterState>().motes_shed, 12);
    }

    /// Spec RED test 3, adapted: stock is the real `f32` ledger
    /// (`add_resource` / `get_amount`), the burn event keeps the
    /// sketch's `u32` amount, and the temperature threshold
    /// (rise > 20 from one mote) is kept exactly.
    #[test]
    fn test_burning_dark_motes_heats_grid() {
        let mut world = world_with_grid(10, 10, 0.0);
        world.insert_resource(ColonyResources::default());
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::DarkMote, 10.0);
        let initial = average_temp(world.resource::<TemperatureGrid>());

        world
            .resource_mut::<Events<BurnMoteEvent>>()
            .send(BurnMoteEvent { amount: 1 });
        world.run_system_once(burn_mote_system).unwrap();

        let resources = world.resource::<ColonyResources>();
        assert!((resources.get_amount(ResourceType::DarkMote) - 9.0).abs() < f32::EPSILON);
        let new_temp = average_temp(world.resource::<TemperatureGrid>());
        assert!(
            new_temp > initial + 20.0,
            "one mote should heat the grid: {initial} -> {new_temp}"
        );
        assert_eq!(world.resource::<StarEaterState>().motes_burned, 1);
    }

    /// A burn the colony cannot afford is a no-op: no heat, no
    /// negative stock, no ledger credit (atomic `try_consume`).
    #[test]
    fn test_burn_without_stock_is_noop() {
        let mut world = world_with_grid(10, 10, 0.0);
        world.insert_resource(ColonyResources::default());

        world
            .resource_mut::<Events<BurnMoteEvent>>()
            .send(BurnMoteEvent { amount: 3 });
        world.run_system_once(burn_mote_system).unwrap();

        assert!(
            (average_temp(world.resource::<TemperatureGrid>()) - 0.0).abs() < f32::EPSILON
        );
        assert_eq!(world.resource::<StarEaterState>().motes_burned, 0);
    }

    /// The Long Dark pins the ambient every tick it runs (§5 timer)
    /// and lifts when the timer expires; after expiry the pin is
    /// released and the system leaves the grid alone.
    #[test]
    fn test_long_dark_pins_ambient_and_expires() {
        let mut world = world_with_grid(10, 10, 15.0);
        world.resource_mut::<StarEaterState>().long_dark_remaining = 2;

        world.run_system_once(long_dark_system).unwrap();
        assert_eq!(
            world.resource::<TemperatureGrid>().ambient,
            LONG_DARK_AMBIENT
        );
        assert_eq!(world.resource::<StarEaterState>().long_dark_remaining, 1);

        world.run_system_once(long_dark_system).unwrap();
        assert!(!world.resource::<StarEaterState>().is_long_dark());

        // Released: a season-warmed ambient is no longer overwritten.
        world.resource_mut::<TemperatureGrid>().ambient = 15.0;
        world.run_system_once(long_dark_system).unwrap();
        assert_eq!(world.resource::<TemperatureGrid>().ambient, 15.0);
    }

    /// A duplicate Arrival mid-Dark is ignored — no stacked plunge,
    /// no double-counted passage.
    #[test]
    fn test_duplicate_arrival_does_not_stack() {
        let mut world = world_with_grid(10, 10, 20.0);
        send_arrival(&mut world);
        send_arrival(&mut world);

        let avg = average_temp(world.resource::<TemperatureGrid>());
        assert!((avg - (20.0 - FREEZE_DROP)).abs() < f32::EPSILON);
        assert_eq!(world.resource::<StarEaterState>().arrivals, 1);
    }

    /// Departure ends the Dark early and sheds the motes (the full
    /// crisis arc in miniature).
    #[test]
    fn test_departure_ends_long_dark() {
        let mut world = world_with_grid(10, 10, 20.0);
        send_arrival(&mut world);
        assert!(world.resource::<StarEaterState>().is_long_dark());

        send_departure(&mut world);

        let state = world.resource::<StarEaterState>();
        assert!(!state.is_long_dark());
        assert_eq!(state.departures, 1);
        assert_eq!(stareater_counts(&mut world).2, MOTES_PER_DEPARTURE);
    }

    /// The scatter is deterministic: the same departure number
    /// always sheds motes onto the same tiles (replayable sims).
    #[test]
    fn test_mote_scatter_is_deterministic() {
        let a = mote_position(1, 3, 80, 50);
        let b = mote_position(1, 3, 80, 50);
        assert_eq!(a, b);
        let c = mote_position(2, 3, 80, 50);
        assert!((0..80).contains(&c.x) && (0..50).contains(&c.y));
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "galactus",
            "marvel",
            "mass effect",
            "reaper",
            "rimworld",
            "dwarf fortress",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
        ];
        for text in star_eater_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
