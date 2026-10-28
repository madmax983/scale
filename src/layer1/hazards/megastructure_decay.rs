//! Spec 372: Stellar Megastructure Decay — the ruins hazard.
//!
//! "Living in the ruins of gods." A colony built inside or near an
//! ancient megastructure lives under a failing sky: the ruins'
//! [`MegastructureRuins`] stability dictates how often a Decay Event
//! fires — a massive section collapses and debris falls onto the
//! Layer 1 colony beneath it, damaging whatever it lands on and, at
//! catastrophic damage, burying the tile in rubble.
//!
//! ## Correction note (2026-10-28)
//!
//! The driver's original brief stated there was NO spec file for 372
//! (it searched `design/specs/`, which does not exist). The spec in
//! fact lives at repo-root `specs/372-stellar-megastructure-decay.md`
//! and was read in full mid-run; this module builds to THE SPEC, and
//! where the brief's "expected shape" sketch differed (a Layer 2
//! structural-rot model), the spec took precedence: the ruins hazard
//! below is the spec-372 deliverable. The Layer 2 scaffolding-rot leg
//! built earlier in the run (`layer2::megastructure_decay`, on the
//! Spec 1133 scaffolding seams) does not conflict with any spec name,
//! type, or acceptance criterion and is retained as a complementary
//! system; every spec acceptance criterion is satisfied HERE.
//!
//! ## Adaptations from the spec's mock-style sketch (§8 pattern)
//!
//! * **`TileType` does not exist; terrain is a grid, not tile
//!   entities.** There is no `TileType` component and no per-tile
//!   entity in the codebase: terrain lives in the [`TerrainGrid`]
//!   resource (`tiles: Vec<TerrainType>`, row-major). The spec's
//!   `TileType::Floor -> TileType::Rubble` transition is therefore
//!   adapted to the grid. The spec's own Technical Guidance allows
//!   this: *"Add `TileType::Rubble` if it doesn't exist, or use a
//!   suitable equivalent like `Rock`."* The equivalent is
//!   [`TerrainType::Rock`] — the exact precedent of
//!   `physics::structural_integrity::apply_collapse` ("Change Terrain
//!   to Rock (Rubble)": collapsed rock, impassable until mined out)
//!   and of the cave-in system, which rubbles tiles the same way.
//! * **`Health` is the real pop/building health.** The sketch's
//!   `layer1::building::Health` does not exist; the real component is
//!   [`crate::layer1::health::Health`] (`current`, `max`,
//!   `has_rust_lung`). The spec's damage query
//!   `Query<(&GridPosition, &mut Health)>` is kept verbatim in shape:
//!   debris damages ANY entity carrying Health on the target tile —
//!   the spec's test building carries Health explicitly, pops carry
//!   it natively, and `apply_collapse` damages victims the same way.
//!   (`layer1::building::Building { building_type }` DOES match the
//!   sketch exactly and is used verbatim in the tests.)
//! * **Randomness is deterministic, not `thread_rng`.** The spec's
//!   `trigger` rolls `rand` per tick, which makes its own RED test 1
//!   a 9% coin flip per run. Following the Spec 370 deterministic-
//!   threshold precedent, the roll is a splitmix64-style hash of the
//!   ledger's roll counter: same stability, same replay, every time.
//!   The probability law is the spec's exactly:
//!   `chance = (1.0 - stability) * 0.1`. RED test 1 is adapted to the
//!   spec's own wording — "a decay event should *eventually*
//!   trigger": at stability 0.1 at least one event fires within 200
//!   ticks, and at stability 1.0 none ever fires.
//! * **Warning / atmosphere / resource-output (§REFACTOR) deferred.**
//!   The event-warning delay, `AtmosphereGrid` venting, and the
//!   stability-for-output temptation loop are the spec's REFACTOR
//!   wishes, not its acceptance criteria; the Architect's note says
//!   "implement the simplest possible version for the MVP." A Major
//!   chronicle on every rubble burial is included (the narrative
//!   warning surface every other catastrophe uses); the rest is
//!   noted as follow-on in the worker report.
//! * **`MegastructureRuins` insertion (Technical Guidance):** the
//!   resource is initialised in simulation setup at stability 1.0 —
//!   perfectly stable, chance exactly 0, mechanically inert — so a
//!   colony only lives under failing ruins when a scenario or the
//!   `decay stability` headless command lowers stability. No
//!   map-type seam exists to gate insertion on; inert-by-default is
//!   the faithful equivalent.
//!
//! Dependencies per spec: 079 Weather (hazards live in
//! `layer1::hazards` alongside meteor/wakes) and 184 Orbital Debris
//! (falling-debris damage here is the surface end of that pipeline;
//! the Layer 2 ledger feed is the scaffolding-rot module's leg).
//!
//! All names and strings are original to Scale — no borrowed IP.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::terrain::{TerrainGrid, TerrainType};

/// Damage dealt by a standard decay event (spec §GREEN).
pub const DECAY_EVENT_DAMAGE: f32 = 50.0;
/// Damage at or above which falling debris buries the tile in
/// rubble (spec §GREEN: `damage >= 100.0`).
pub const RUBBLE_DAMAGE_THRESHOLD: f32 = 100.0;
/// Peak per-tick event chance, reached at stability 0.0
/// (spec: `chance = (1.0 - stability) * 0.1`).
pub const MAX_EVENT_CHANCE: f32 = 0.1;
/// Fallback grid dimensions when no [`TerrainGrid`] is present
/// (the fossil-fleet drop-scatter precedent).
pub const FALLBACK_GRID_WIDTH: i32 = 80;
/// See [`FALLBACK_GRID_WIDTH`].
pub const FALLBACK_GRID_HEIGHT: i32 = 50;

/// The ancient megastructure ruins looming over the colony
/// (spec §GREEN, field verbatim). Stability 1.0 = sound (decay
/// events never fire); 0.0 = collapsing (peak event rate).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct MegastructureRuins {
    /// Structural stability of the ruins, 0.0 to 1.0.
    pub stability: f32,
}

impl Default for MegastructureRuins {
    fn default() -> Self {
        // Inert by default: only a colony actually founded in the
        // ruins (scenario / headless) lowers stability from 1.0.
        Self { stability: 1.0 }
    }
}

/// A section of the ruins gives way: debris falls on `target`
/// (spec §GREEN, fields verbatim).
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub struct DecayEvent {
    /// Tile the debris falls on.
    pub target: GridPosition,
    /// Damage dealt to entities on the target tile.
    pub damage: f32,
}

/// Ledger for the ruins hazard (STATS / panel surface + the
/// deterministic roll counter).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct RuinsDecayLedger {
    /// Total trigger rolls made (the deterministic RNG index).
    pub rolls: u64,
    /// Total decay events fired.
    pub events_fired: u64,
    /// Total entities damaged by falling debris.
    pub entities_damaged: u64,
    /// Total debris damage dealt.
    pub damage_dealt: f32,
    /// Total tiles buried in rubble.
    pub tiles_rubbled: u64,
}

/// Per-tick probability of a decay event at a given stability
/// (spec's law, verbatim): `(1.0 - stability) * 0.1`, clamped.
#[must_use]
pub fn event_chance(stability: f32) -> f32 {
    (1.0 - stability.clamp(0.0, 1.0)) * MAX_EVENT_CHANCE
}

/// Deterministic roll in [0, 1) for the `index`-th trigger roll
/// (splitmix64-style hash — replayable and test-exact; see the
/// module docs for why this replaces `thread_rng`).
#[must_use]
pub fn decay_roll(index: u64) -> f32 {
    let mut z = 0x3720_0000_u64 ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1_u64 << 24) as f32
}

/// Deterministic fall target for the `index`-th roll, inside a
/// `width` x `height` grid.
#[must_use]
pub fn decay_target(index: u64, width: i32, height: i32) -> GridPosition {
    let mut z = 0xDEC0_0372_u64 ^ index.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 31;
    GridPosition {
        x: (z % width.max(1) as u64) as i32,
        y: ((z >> 32) % height.max(1) as u64) as i32,
    }
}

// ---------------------------------------------------------------------------
// Systems (spec §GREEN, signatures adapted per module docs)
// ---------------------------------------------------------------------------

/// Spec system: fire decay events based on ruins stability.
/// Probability per tick is [`event_chance`]; the roll and the fall
/// target are deterministic functions of the roll counter.
pub fn trigger_decay_event_system(
    ruins: Option<Res<MegastructureRuins>>,
    mut ledger: ResMut<RuinsDecayLedger>,
    terrain: Option<Res<TerrainGrid>>,
    mut events: EventWriter<DecayEvent>,
) {
    let Some(ruins) = ruins else {
        return;
    };
    let chance = event_chance(ruins.stability);
    let index = ledger.rolls;
    ledger.rolls += 1;
    if chance <= 0.0 || decay_roll(index) >= chance {
        return;
    }
    let (width, height) = terrain.as_deref().map_or(
        (FALLBACK_GRID_WIDTH, FALLBACK_GRID_HEIGHT),
        |grid| (grid.width as i32, grid.height as i32),
    );
    ledger.events_fired += 1;
    events.send(DecayEvent {
        target: decay_target(index, width, height),
        damage: DECAY_EVENT_DAMAGE,
    });
}

/// Spec system: resolve falling debris. Every entity carrying
/// [`Health`] on the target tile takes the event's damage; at
/// [`RUBBLE_DAMAGE_THRESHOLD`] or above the tile is buried in
/// rubble ([`TerrainType::Rock`] — see module docs) and the burial
/// is written into the chronicle as a Major entry.
pub fn process_debris_fall_system(
    mut events: EventReader<DecayEvent>,
    mut victims: Query<(&GridPosition, &mut Health)>,
    mut terrain: Option<ResMut<TerrainGrid>>,
    mut ledger: ResMut<RuinsDecayLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if !event.damage.is_finite() || event.damage <= 0.0 {
            continue;
        }
        for (pos, mut health) in victims.iter_mut() {
            if *pos == event.target {
                health.current = (health.current - event.damage).max(0.0);
                ledger.entities_damaged += 1;
                ledger.damage_dealt += event.damage;
            }
        }
        if event.damage >= RUBBLE_DAMAGE_THRESHOLD {
            if let Some(terrain) = terrain.as_deref_mut() {
                let (x, y) = (event.target.x, event.target.y);
                if x >= 0 && y >= 0 {
                    let idx = (y as usize)
                        .checked_mul(terrain.width)
                        .and_then(|i| i.checked_add(x as usize));
                    if let Some(i) = idx.filter(|&i| i < terrain.tiles.len()) {
                        if terrain.tiles[i] != TerrainType::Rock {
                            terrain.tiles[i] = TerrainType::Rock;
                            ledger.tiles_rubbled += 1;
                            chronicle.send(AddChronicleEvent {
                                text: format!(
                                    "A section of the ancient megastructure comes down at ({x}, {y}) in a rain of century-old girders. The tile is buried in rubble."
                                ),
                                importance: EventImportance::Major,
                            });
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Set ruins stability from a `&mut World` context (headless
/// debug), clamped to 0..1. Inits the resource if absent.
pub fn set_ruins_stability(world: &mut World, stability: f32) {
    world.init_resource::<MegastructureRuins>();
    world.resource_mut::<MegastructureRuins>().stability = stability.clamp(0.0, 1.0);
}

/// Drop debris on an exact tile from a `&mut World` context
/// (headless debug): sends a real [`DecayEvent`] the process
/// system resolves next tick.
pub fn trigger_decay_collapse(world: &mut World, target: GridPosition, damage: f32) {
    world.init_resource::<Events<DecayEvent>>();
    world
        .resource_mut::<Events<DecayEvent>>()
        .send(DecayEvent { target, damage });
}

/// Census: (stability, events fired, entities damaged, tiles
/// rubbled). Stability is 1.0 when the resource is absent.
#[must_use]
pub fn ruins_counts(world: &mut World) -> (f32, u64, u64, u64) {
    let stability = world
        .get_resource::<MegastructureRuins>()
        .map_or(1.0, |r| r.stability);
    let (fired, damaged, rubbled) = world
        .get_resource::<RuinsDecayLedger>()
        .map_or((0, 0, 0), |l| {
            (l.events_fired, l.entities_damaged, l.tiles_rubbled)
        });
    (stability, fired, damaged, rubbled)
}

/// One-token STATS label, e.g. `stab0.10/events3/rubble1`, or
/// `stable` when the ruins are sound and nothing has ever fallen.
#[must_use]
pub fn ruins_stats_label(world: &mut World) -> String {
    let (stability, fired, _damaged, rubbled) = ruins_counts(world);
    if stability >= 1.0 && fired == 0 {
        "stable".to_string()
    } else {
        format!("stab{stability:.2}/events{fired}/rubble{rubbled}")
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn ruins_public_strings() -> Vec<String> {
    vec![
        "Stellar Megastructure Decay".to_string(),
        "Living in the ruins of gods.".to_string(),
        "A section of the ancient megastructure comes down in a rain of century-old girders. The tile is buried in rubble.".to_string(),
    ]
}
