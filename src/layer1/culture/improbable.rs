//! The Improbable Pilot — Adams-flavor adventurer origin #5.
//!
//! Layers on top of the adventurer-mode spike (`direct_link`): a junker
//! shuttle drifts near the starter colony, a busted absurdity-fueled engine
//! bolted to her spine. Aboard waits a lone pilot — possess them and the
//! headless console gains `scheme <plan-text>` (file a flight plan, scored
//! for Audacity: dumber plans score higher), `jump` (fire the Longshot
//! Drive — success scales with audacity), `unload` (deliver an exotic cache
//! to the colony stockpile for a morale bump), and `skim <amount>` (pocket
//! a cut of the cache for yourself, pirate-adjacent).
//!
//! The engine is the Longshot Drive. Charge builds passively every tick and
//! is never zeroed forever; a misfire never soft-locks the pilot — every
//! bad outcome is recoverable within a few dozen ticks. First successful
//! jump, first misfire, and cache recovery fire chronicle events.
//!
//! All names are original. The absurdity-fueled-drive concept is concept-only
//! inspiration — no named characters, places, ships, or distinctive IP
//! anywhere. Mechanics and vibes only, under original names. The pilot is
//! "the Improbable Pilot"; the engine is "the Longshot Drive".

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::Wallet;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::pressure::PressureGrid;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::terrain::{TerrainGrid, TerrainType};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn ring (Chebyshev) around the lander where the shuttle drifts.
pub const SHUTTLE_RING_MIN: i32 = 6;
pub const SHUTTLE_RING_MAX: i32 = 10;

/// Longshot Drive charge capacity.
pub const DRIVE_CHARGE_MAX: f32 = 100.0;
/// Charge rebuilt per tick, passively. The drive never strands you forever.
pub const CHARGE_REGEN_PER_TICK: f32 = 0.5;
/// Charge cost to file a scheme.
pub const SCHEME_FILING_COST: f32 = 5.0;
/// Charge cost to fire a jump.
pub const JUMP_CHARGE_COST: f32 = 25.0;

/// Base audacity every plan gets for showing up.
pub const AUDACITY_BASE: f32 = 0.10;
/// Minimum jump success probability (a sober, sensible plan).
pub const JUMP_BASE_P: f32 = 0.25;
/// Audacity scaling on jump success (dumb plans approach this + base).
pub const JUMP_AUDACITY_SCALE: f32 = 0.65;
/// Audacity scaling on exotic-cache finds.
pub const CACHE_FIND_BASE_P: f32 = 0.20;
pub const CACHE_FIND_AUDACITY_SCALE: f32 = 0.50;

/// Base jump distance in tiles (Chebyshev-ish).
pub const JUMP_DISTANCE_BASE: i32 = 12;
/// Extra jump distance at full audacity.
pub const JUMP_DISTANCE_AUDACITY: i32 = 24;
/// Landing search attempts before the drive politely declines to move.
pub const LANDING_ATTEMPTS: u32 = 12;

/// Exotic cache credit value band.
pub const CACHE_CREDITS_MIN: f32 = 40.0;
pub const CACHE_CREDITS_MAX: f32 = 200.0;
/// Colony morale bump when a cache is unloaded to the stockpile.
pub const UNLOAD_MORALE_BUMP: f32 = 0.25;
/// Ticks the morale bump lingers.
pub const UNLOAD_MORALE_TICKS: u32 = 120;

/// Ticks the pilot stays rattled after a confusion misfire.
pub const DAZED_TICKS: u32 = 30;
/// Morale penalty while dazed.
pub const DAZED_MORALE_PENALTY: f32 = -0.15;
/// Charge kickback on a backwards jump (a misfire with a silver lining).
pub const BACKWARDS_JUMP_CHARGE_REFUND: f32 = 20.0;

/// Food in the shuttle's snack locker at spawn.
pub const SNACK_LOCKER_FOOD: f32 = 12.0;
/// Locker food eaten per tick by the mess when the pilot is hungry.
pub const SNACK_MEAL_PER_TICK: f32 = 0.02;
/// Hunger restored per tick by the mess.
pub const SNACK_SATIETY: f32 = 0.2;
/// Below this hunger the locker feeds the pilot.
pub const SNACK_HUNGER_GATE: f32 = 0.95;

/// How far (Chebyshev) the pilot may wander from the shuttle before the
/// tick nudges them back (only while unpossessed).
pub const PILOT_TETHER_RADIUS: i32 = 2;
/// Pressure envelope radius held around the shuttle.
pub const SHUTTLE_PRESSURE_RADIUS: i32 = 2;
/// How close (Chebyshev) the pilot must be to the shuttle to work the drive.
pub const SHUTTLE_WORK_REACH: i32 = 2;
/// How close (Chebyshev) a pop must be to the shuttle to take up the yoke.
pub const YOKE_TAKE_REACH: i32 = 1;

/// Chronicle text when the shuttle drifts into sensor range.
pub const SHUTTLE_RUMOR_CHRONICLE: &str =
    "Rumor: a junker shuttle drifts off the beacon line, her drive humming \
     a tune that makes no sense. Somebody aboard is filing flight plans in \
     crayon.";
/// Chronicle text on the first successful jump.
pub const FIRST_JUMP_CHRONICLE: &str =
    "The Longshot Drive fires — and the universe, briefly embarrassed, \
     lets the shuttle be somewhere else.";
/// Chronicle text on the first misfire.
pub const FIRST_MISFIRE_CHRONICLE: &str =
    "The Longshot Drive coughs, considers its choices, and makes a bad one.";
/// Label on the colony-wide morale modifier from an unloaded cache.
pub const CACHE_MORALE_LABEL: &str = "Exotic windfall";
/// Label on the pilot's dazed morale modifier.
pub const DAZED_MORALE_LABEL: &str = "Drive rattled";

// ---------------------------------------------------------------------------
// The Audacity lexicon
// ---------------------------------------------------------------------------
//
// Deterministic and documented: every keyword is a lowercase substring
// match against the filed plan text. Each keyword present adds its weight
// ONCE (saying "banana banana banana" does not triple-dip — commitment to
// the bit, not repetition). Score = AUDACITY_BASE + weights, clamped to
// 1.0. Dumber plans score higher; the Longshot Drive rewards audacity.

/// Absurd-keyword lexicon: (keyword, audacity weight).
pub const AUDACITY_LEXICON: &[(&str, f32)] = &[
    ("banana", 0.15),
    ("teacup", 0.12),
    ("duck", 0.12),
    ("blindfold", 0.18),
    ("backwards", 0.10),
    ("naked", 0.16),
    ("sing", 0.12),
    ("dance", 0.12),
    ("yodel", 0.14),
    ("spoon", 0.10),
    ("sock", 0.12),
    ("wedding", 0.12),
    ("cake", 0.10),
    ("pajama", 0.12),
    ("tutu", 0.14),
    ("jelly", 0.10),
    ("explode", 0.10),
    ("sneeze", 0.12),
    ("pickle", 0.12),
    ("underwater", 0.14),
    ("kazoo", 0.14),
    ("marshmallow", 0.13),
    ("penguin", 0.13),
    ("tapdance", 0.15),
    ("bubble", 0.10),
    ("whoopee", 0.16),
    ("spaghetti", 0.12),
    ("moon", 0.08),
];

/// Score a filed plan for Audacity, 0.0..=1.0. Deterministic: same text,
/// same score, every time.
#[must_use]
pub fn audacity_score(plan: &str) -> f32 {
    let lowered = plan.to_lowercase();
    let mut score = AUDACITY_BASE;
    for (keyword, weight) in AUDACITY_LEXICON {
        if lowered.contains(keyword) {
            score += weight;
        }
    }
    score.min(1.0)
}

/// Jump success probability at a given audacity.
#[must_use]
pub fn jump_success_p(audacity: f32) -> f32 {
    (JUMP_BASE_P + JUMP_AUDACITY_SCALE * audacity).clamp(0.0, 0.95)
}

/// Resolve a jump: `roll` in 0.0..1.0 succeeds below the probability.
#[must_use]
pub fn resolve_jump(roll: f32, audacity: f32) -> bool {
    roll < jump_success_p(audacity)
}

/// Jump distance in tiles at a given audacity and roll.
#[must_use]
pub fn jump_distance(audacity: f32, roll: f32) -> i32 {
    JUMP_DISTANCE_BASE + (audacity * JUMP_DISTANCE_AUDACITY as f32) as i32 + (roll * 6.0) as i32
}

/// Exotic-cache find probability at a given audacity.
#[must_use]
pub fn cache_find_p(audacity: f32) -> f32 {
    (CACHE_FIND_BASE_P + CACHE_FIND_AUDACITY_SCALE * audacity).clamp(0.0, 0.95)
}

/// Resolve a cache find: `roll` in 0.0..1.0 finds one below the probability.
#[must_use]
pub fn resolve_cache_find(roll: f32, audacity: f32) -> bool {
    roll < cache_find_p(audacity)
}

/// Credit value of a recovered exotic cache from a 0.0..1.0 roll.
#[must_use]
pub fn cache_credits(roll: f32) -> f32 {
    CACHE_CREDITS_MIN + roll.clamp(0.0, 1.0) * (CACHE_CREDITS_MAX - CACHE_CREDITS_MIN)
}

// ---------------------------------------------------------------------------
// Misfire table
// ---------------------------------------------------------------------------

/// The six documented misfires. Every one is recoverable within a few
/// dozen ticks — charge always regenerates, dazed always wears off, wrong
/// offsets always land on walkable tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MisfireKind {
    /// The hold's cache is scrambled into a sculpture of itself (lost).
    ScrambledCargo,
    /// The shuttle arrives somewhere adjacent to the plan (safe, walkable).
    WrongOffset,
    /// The drive vents every drop of charge and forgets the plan.
    ChargeVent,
    /// The pilot is rattled: no jumps for [`DAZED_TICKS`] ticks.
    CrewConfusion,
    /// The chronicle embellishes a jump that never happened.
    ChronicleLie,
    /// The shuttle goes precisely nowhere; the tanks come back fuller.
    BackwardsJump,
}

impl MisfireKind {
    /// Short STATS label for the last outcome.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            MisfireKind::ScrambledCargo => "mis-scram",
            MisfireKind::WrongOffset => "mis-offset",
            MisfireKind::ChargeVent => "mis-vent",
            MisfireKind::CrewConfusion => "mis-dazed",
            MisfireKind::ChronicleLie => "mis-lie",
            MisfireKind::BackwardsJump => "mis-back",
        }
    }

    /// All six, in roll order.
    #[must_use]
    pub fn all() -> [MisfireKind; 6] {
        [
            MisfireKind::ScrambledCargo,
            MisfireKind::WrongOffset,
            MisfireKind::ChargeVent,
            MisfireKind::CrewConfusion,
            MisfireKind::ChronicleLie,
            MisfireKind::BackwardsJump,
        ]
    }
}

/// Resolve a misfire from a 0.0..1.0 roll (uniform sixths).
#[must_use]
pub fn resolve_misfire(roll: f32) -> MisfireKind {
    let idx = (roll.clamp(0.0, 0.999_999) * 6.0) as usize;
    MisfireKind::all()[idx.min(5)]
}

// ---------------------------------------------------------------------------
// Exotic cache names (original, concept-only)
// ---------------------------------------------------------------------------

/// Names for recovered exotic caches. Original flavor; no IP anywhere.
pub const CACHE_NAMES: &[&str] = &[
    "star-whale amber",
    "folded moonlight",
    "a crate of reverse rain",
    "compressed nostalgia",
    "bottled thunder",
    "gravity lace",
    "a teaspoon of dawn",
    "a very patient crystal",
];

/// Pick a cache name from a 0.0..1.0 roll.
#[must_use]
pub fn cache_name(roll: f32) -> &'static str {
    let idx = (roll.clamp(0.0, 0.999_999) * CACHE_NAMES.len() as f32) as usize;
    CACHE_NAMES[idx.min(CACHE_NAMES.len() - 1)]
}

// ---------------------------------------------------------------------------
// Components & state
// ---------------------------------------------------------------------------

/// Marker on the junker shuttle entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct JunkerShuttle;

/// The Longshot Drive, fitted to the shuttle.
#[derive(Component, Debug, Clone)]
pub struct LongshotDrive {
    /// Charge 0.0..=[`DRIVE_CHARGE_MAX`]; regenerates passively.
    pub charge: f32,
    /// Audacity of the currently filed plan, 0.0..=1.0.
    pub audacity: f32,
    /// Short label of the last jump outcome ("clean", "mis-vent", ...).
    pub last_outcome: String,
}

impl Default for LongshotDrive {
    fn default() -> Self {
        Self {
            charge: DRIVE_CHARGE_MAX,
            audacity: 0.0,
            last_outcome: "never".to_string(),
        }
    }
}

/// An exotic cache recovered by a jump, waiting in the shuttle hold.
#[derive(Component, Debug, Clone)]
pub struct ExoticCache {
    pub name: String,
    pub credits: f32,
}

/// The shuttle's hold: snack locker plus one cache slot.
#[derive(Component, Debug, Clone)]
pub struct ShuttleHold {
    /// Food the ship's mess feeds the pilot from.
    pub food: f32,
}

impl Default for ShuttleHold {
    fn default() -> Self {
        Self {
            food: SNACK_LOCKER_FOOD,
        }
    }
}

/// Marker on the pilot pop.
#[derive(Component, Debug, Clone, Copy)]
pub struct ImprobablePilot;

/// The pilot is rattled after a confusion misfire: no jumps until it
/// wears off. Always temporary — never a soft-lock.
#[derive(Component, Debug, Clone, Copy)]
pub struct PilotDazed {
    pub ticks_left: u32,
}

/// Machine state for the Improbable Pilot origin.
#[derive(Resource, Debug, Clone, Default)]
pub struct PilotState {
    pub shuttle_spawn_attempted: bool,
    pub shuttle: Option<Entity>,
    pub pilot: Option<Entity>,
    /// The currently filed plan text (None = nothing filed).
    pub scheme: Option<String>,
    pub first_jump_done: bool,
    pub first_misfire_done: bool,
    pub cache_recovered_done: bool,
}

pub fn ensure_pilot_resources(world: &mut World) {
    if world.get_resource::<PilotState>().is_none() {
        world.insert_resource(PilotState::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_pilot_resources(world);
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn the junker shuttle and her lone pilot once, in a ring around the
/// lander. Mirrors the derelict-hulk spawn (snapshot-then-spawn to satisfy
/// the borrow checker).
pub fn spawn_shuttle_once(world: &mut World) {
    ensure_pilot_resources(world);
    {
        let state = world.resource::<PilotState>();
        if state.shuttle_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<PilotState>().shuttle_spawn_attempted = true;

    // Only one shuttle per colony.
    if world.query::<&JunkerShuttle>().iter(world).next().is_some() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    let (walkable, occupied, buildings) = {
        let mut walkable = std::collections::HashSet::new();
        if let Some(terrain) = world.get_resource::<TerrainGrid>() {
            for dx in -SHUTTLE_RING_MAX..=SHUTTLE_RING_MAX {
                for dy in -SHUTTLE_RING_MAX..=SHUTTLE_RING_MAX {
                    let x = lander.x + dx;
                    let y = lander.y + dy;
                    if x >= 0 && y >= 0 {
                        if let Some(tt) = terrain.get(x as usize, y as usize) {
                            if !matches!(tt, TerrainType::Water | TerrainType::Void) {
                                walkable.insert((x, y));
                            }
                        }
                    }
                }
            }
        }
        let occupied: std::collections::HashSet<(i32, i32)> = world
            .get_resource::<OccupiedTiles>()
            .map(|o| o.0.iter().copied().collect())
            .unwrap_or_default();
        let buildings: std::collections::HashSet<(i32, i32)> = world
            .query::<(&Building, &GridPosition)>()
            .iter(world)
            .map(|(_, p)| (p.x, p.y))
            .collect();
        (walkable, occupied, buildings)
    };

    let target: Option<GridPosition> = {
        let mut found = None;
        'search: for radius in SHUTTLE_RING_MIN..=SHUTTLE_RING_MAX {
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    if dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let x = lander.x + dx;
                    let y = lander.y + dy;
                    if x < 0 || y < 0 {
                        continue;
                    }
                    if occupied.contains(&(x, y)) {
                        continue;
                    }
                    if buildings.contains(&(x, y)) {
                        continue;
                    }
                    if !walkable.contains(&(x, y)) {
                        continue;
                    }
                    found = Some(GridPosition { x, y });
                    break 'search;
                }
            }
        }
        found
    };

    let Some(pos) = target else { return };
    spawn_shuttle_at(world, pos.x, pos.y);
    send_chronicle(world, SHUTTLE_RUMOR_CHRONICLE, EventImportance::Minor);
}

/// Spawn the shuttle and pilot at an explicit tile. Shared by the
/// ring-spawn and the tests.
fn spawn_shuttle_at(world: &mut World, x: i32, y: i32) -> Entity {
    let shuttle = world
        .spawn((
            JunkerShuttle,
            LongshotDrive::default(),
            ShuttleHold::default(),
            GridPosition { x, y },
        ))
        .id();
    let mut rng = rand::thread_rng();
    let mut bundle = PopBundle::random(x, y, &mut rng);
    bundle.name = PopName("Pilot Wren".to_string());
    let pilot = world.spawn((bundle, ImprobablePilot)).id();
    world.resource_mut::<PilotState>().shuttle = Some(shuttle);
    world.resource_mut::<PilotState>().pilot = Some(pilot);
    shuttle
}

/// The shuttle within yoke-take reach of a pop, if any.
pub fn shuttle_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    world
        .query::<(Entity, &JunkerShuttle, &GridPosition)>()
        .iter(world)
        .find(|(_, _, spos)| {
            (spos.x - pos.x).abs() <= YOKE_TAKE_REACH && (spos.y - pos.y).abs() <= YOKE_TAKE_REACH
        })
        .map(|(e, _, _)| e)
}

/// Take up the pilot's yoke: a possessed pop beside the shuttle becomes the
/// Improbable Pilot, if no living pilot holds the role.
pub fn try_take_yoke(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_pilot_resources(world);
    if world.get::<ImprobablePilot>(pop).is_some() {
        return Err("This pop already holds the pilot's yoke.".to_string());
    }
    let shuttle = shuttle_within_reach(world, pop)
        .ok_or_else(|| "No junker shuttle within reach. Move beside one.".to_string())?;
    let pilot = world.resource::<PilotState>().pilot;
    let pilot_alive = pilot.and_then(|p| world.get::<Pop>(p)).is_some();
    if pilot_alive {
        return Err("The shuttle already has her pilot.".to_string());
    }
    let spos = *world
        .get::<GridPosition>(shuttle)
        .ok_or_else(|| "The shuttle has no position.".to_string())?;
    world.entity_mut(pop).insert(ImprobablePilot);
    if let Some(mut pos) = world.get_mut::<GridPosition>(pop) {
        *pos = spos;
    }
    world.resource_mut::<PilotState>().pilot = Some(pop);
    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));
    send_chronicle(
        world,
        &format!("{name} takes up the pilot's yoke and climbs into the junker shuttle."),
        EventImportance::Major,
    );
    Ok(format!(
        "{name} takes up the pilot's yoke and boards the junker shuttle at ({}, {}).",
        spos.x, spos.y
    ))
}

// ---------------------------------------------------------------------------
// Origin actions
// ---------------------------------------------------------------------------

/// Look up the live pilot pop, cleaning up the state if they're gone.
fn live_pilot(world: &mut World) -> Option<Entity> {
    ensure_pilot_resources(world);
    let p = world.resource::<PilotState>().pilot;
    match p {
        Some(e) if world.get::<Pop>(e).is_some() && world.get::<ImprobablePilot>(e).is_some() => {
            Some(e)
        }
        _ => {
            world.resource_mut::<PilotState>().pilot = None;
            None
        }
    }
}

/// The live shuttle entity, if it still exists.
fn live_shuttle(world: &mut World) -> Option<Entity> {
    ensure_pilot_resources(world);
    let s = world.resource::<PilotState>().shuttle;
    match s {
        Some(e) if world.get::<JunkerShuttle>(e).is_some() => Some(e),
        _ => None,
    }
}

/// Require the caller to be the living pilot origin pop.
fn require_pilot_pop(world: &mut World, caller: Entity) -> Result<Entity, String> {
    let pilot =
        live_pilot(world).ok_or_else(|| "The pilot is gone — the shuttle hums to no one.".to_string())?;
    if caller != pilot {
        return Err("Only the pilot can work the Longshot Drive. Possess them first.".to_string());
    }
    Ok(pilot)
}

/// The pilot must stand aboard (within [`SHUTTLE_WORK_REACH`]) to work the
/// drive.
fn require_aboard(world: &mut World, pilot: Entity, shuttle: Entity) -> Result<(), String> {
    let ppos = world
        .get::<GridPosition>(pilot)
        .copied()
        .ok_or_else(|| "The pilot has no position.".to_string())?;
    let spos = world
        .get::<GridPosition>(shuttle)
        .copied()
        .ok_or_else(|| "The shuttle has no position.".to_string())?;
    if (spos.x - ppos.x).abs() > SHUTTLE_WORK_REACH || (spos.y - ppos.y).abs() > SHUTTLE_WORK_REACH {
        return Err("Climb aboard the shuttle first — the drive is worked from the yoke.".to_string());
    }
    Ok(())
}

/// File a flight plan. Scores Audacity from the text, costs a little
/// charge, and arms the drive for one jump.
pub fn file_scheme(world: &mut World, caller: Entity, plan: &str) -> Result<String, String> {
    let pilot = require_pilot_pop(world, caller)?;
    let shuttle =
        live_shuttle(world).ok_or_else(|| "No junker shuttle in this colony.".to_string())?;
    require_aboard(world, pilot, shuttle)?;
    let plan = plan.trim();
    if plan.is_empty() {
        return Err("File an actual plan. The drive can smell a blank page.".to_string());
    }
    {
        let drive = world
            .get::<LongshotDrive>(shuttle)
            .ok_or_else(|| "The drive is missing.".to_string())?;
        if drive.charge < SCHEME_FILING_COST {
            return Err(format!(
                "Not enough charge to file ({:.1}/{:.1} needed). Let her build.",
                drive.charge, SCHEME_FILING_COST
            ));
        }
    }
    let audacity = audacity_score(plan);
    if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
        drive.charge = (drive.charge - SCHEME_FILING_COST).max(0.0);
        drive.audacity = audacity;
    }
    world.resource_mut::<PilotState>().scheme = Some(plan.to_string());
    let verdict = if audacity >= 0.7 {
        "The drive purrs like a pleased cat."
    } else if audacity >= 0.4 {
        "The drive hums, considering."
    } else {
        "The drive coughs politely. It has seen bolder."
    };
    Ok(format!(
        "Plan filed: \"{plan}\" — audacity {audacity:.2}. {verdict}"
    ))
}

/// Fire the Longshot Drive: consume charge, roll against audacity, relocate
/// the shuttle and pilot, and maybe drag back an exotic cache.
pub fn fire_jump(world: &mut World, caller: Entity) -> Result<String, String> {
    let pilot = require_pilot_pop(world, caller)?;
    let shuttle =
        live_shuttle(world).ok_or_else(|| "No junker shuttle in this colony.".to_string())?;
    require_aboard(world, pilot, shuttle)?;
    if world.get::<PilotDazed>(pilot).is_some() {
        return Err("The pilot is still rattled — give them a few ticks to steady.".to_string());
    }
    if world.resource::<PilotState>().scheme.is_none() {
        return Err("No plan filed. `scheme <plan-text>` first — the drive needs something to believe in.".to_string());
    }
    {
        let drive = world
            .get::<LongshotDrive>(shuttle)
            .ok_or_else(|| "The drive is missing.".to_string())?;
        if drive.charge < JUMP_CHARGE_COST {
            return Err(format!(
                "Not enough charge ({:.1}/{:.1} needed). Let her build.",
                drive.charge, JUMP_CHARGE_COST
            ));
        }
    }
    // Pay the charge and consume the plan — every jump needs fresh audacity.
    let audacity = {
        let mut drive = world.get_mut::<LongshotDrive>(shuttle).unwrap();
        drive.charge = (drive.charge - JUMP_CHARGE_COST).max(0.0);
        drive.audacity
    };
    world.resource_mut::<PilotState>().scheme = None;
    if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
        drive.audacity = 0.0;
    }

    let mut rng = rand::thread_rng();
    let origin = *world.get::<GridPosition>(shuttle).unwrap();
    if resolve_jump(rng.gen_range(0.0..1.0), audacity) {
        apply_success(world, shuttle, pilot, origin, audacity)
    } else {
        apply_misfire(world, shuttle, pilot, origin, resolve_misfire(rng.gen_range(0.0..1.0)))
    }
}

/// Pick a landing tile: `distance` tiles around `origin` in a random
/// direction, walkable and unoccupied, with fallback to the origin (the
/// drive declines to move rather than land somewhere stupid).
fn pick_landing_tile(
    world: &mut World,
    origin: GridPosition,
    distance: i32,
    rng: &mut impl rand::Rng,
) -> Option<GridPosition> {
    let occupied: std::collections::HashSet<(i32, i32)> = world
        .get_resource::<OccupiedTiles>()
        .map(|o| o.0.iter().copied().collect())
        .unwrap_or_default();
    let buildings: std::collections::HashSet<(i32, i32)> = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .map(|(_, p)| (p.x, p.y))
        .collect();
    let terrain = world.get_resource::<TerrainGrid>()?;
    for _ in 0..LANDING_ATTEMPTS {
        let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
        let x = origin.x + (distance as f32 * angle.cos()).round() as i32;
        let y = origin.y + (distance as f32 * angle.sin()).round() as i32;
        if x < 0 || y < 0 {
            continue;
        }
        let walkable = terrain
            .get(x as usize, y as usize)
            .is_some_and(TerrainType::is_walkable);
        if !walkable || occupied.contains(&(x, y)) || buildings.contains(&(x, y)) {
            continue;
        }
        return Some(GridPosition { x, y });
    }
    None
}

/// Apply a successful jump: relocate shuttle + pilot, maybe find a cache.
fn apply_success(
    world: &mut World,
    shuttle: Entity,
    pilot: Entity,
    origin: GridPosition,
    audacity: f32,
) -> Result<String, String> {
    let mut rng = rand::thread_rng();
    let distance = jump_distance(audacity, rng.gen_range(0.0..1.0));
    let landing = pick_landing_tile(world, origin, distance, &mut rng).unwrap_or(origin);
    let moved = landing != origin;
    if let Some(mut pos) = world.get_mut::<GridPosition>(shuttle) {
        *pos = landing;
    }
    if let Some(mut pos) = world.get_mut::<GridPosition>(pilot) {
        *pos = landing;
    }
    if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
        drive.last_outcome = "clean".to_string();
    }

    // Cache find roll.
    let cache_msg = if resolve_cache_find(rng.gen_range(0.0..1.0), audacity) {
        let name = cache_name(rng.gen_range(0.0..1.0)).to_string();
        let credits = cache_credits(rng.gen_range(0.0..1.0));
        world.entity_mut(shuttle).insert(ExoticCache { name: name.clone(), credits });
        let first = {
            let mut state = world.resource_mut::<PilotState>();
            let first = !state.cache_recovered_done;
            state.cache_recovered_done = true;
            first
        };
        if first {
            send_chronicle(
                world,
                &format!("The shuttle drags back {name} — exotic, glittering, and definitely somebody's."),
                EventImportance::Major,
            );
        }
        format!(" The jump drags back {name} ({credits:.0}cr) — stowed in the hold.")
    } else {
        String::new()
    };

    let first = {
        let mut state = world.resource_mut::<PilotState>();
        let first = !state.first_jump_done;
        state.first_jump_done = true;
        first
    };
    if first {
        send_chronicle(world, FIRST_JUMP_CHRONICLE, EventImportance::Major);
    }
    let arrival = if moved {
        format!("arrived at ({}, {}), {} tiles out", landing.x, landing.y, distance)
    } else {
        "the drive politely declined to move rather than land somewhere stupid".to_string()
    };
    Ok(format!(
        "JUMP — {arrival}.{cache_msg}"
    ))
}

/// Apply a misfire from the documented table. Every branch is recoverable.
fn apply_misfire(
    world: &mut World,
    shuttle: Entity,
    pilot: Entity,
    origin: GridPosition,
    kind: MisfireKind,
) -> Result<String, String> {
    let mut rng = rand::thread_rng();
    let first = {
        let mut state = world.resource_mut::<PilotState>();
        let first = !state.first_misfire_done;
        state.first_misfire_done = true;
        first
    };
    if first {
        send_chronicle(world, FIRST_MISFIRE_CHRONICLE, EventImportance::Minor);
    }
    if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
        drive.last_outcome = kind.label().to_string();
    }
    let name = world
        .get::<PopName>(pilot)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "the pilot".to_string());
    match kind {
        MisfireKind::ScrambledCargo => {
            let had = world.get::<ExoticCache>(shuttle).is_some();
            if had {
                world.entity_mut(shuttle).remove::<ExoticCache>();
            }
            Ok(if had {
                "MISFIRE — the drive hiccups and the hold's cache reassembles itself into a \
                 sculpture of what it used to be. Beautiful. Worthless."
                    .to_string()
            } else {
                "MISFIRE — the drive scrambles the cargo bay with tremendous confidence. \
                 There was no cargo. It scrambled that too."
                    .to_string()
            })
        }
        MisfireKind::WrongOffset => {
            let half = (jump_distance(0.3, rng.gen_range(0.0..1.0)) / 2).max(3);
            let landing = pick_landing_tile(world, origin, half, &mut rng).unwrap_or(origin);
            if let Some(mut pos) = world.get_mut::<GridPosition>(shuttle) {
                *pos = landing;
            }
            if let Some(mut pos) = world.get_mut::<GridPosition>(pilot) {
                *pos = landing;
            }
            Ok(format!(
                "MISFIRE — wrong offset! The shuttle washes up at ({}, {}), a mere stroll \
                 from where it started. The drive refuses to discuss it.",
                landing.x, landing.y
            ))
        }
        MisfireKind::ChargeVent => {
            if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
                drive.charge = 0.0;
                drive.audacity = 0.0;
            }
            Ok("MISFIRE — the drive vents every drop of charge in a long embarrassed sigh. \
                She'll rebuild it, tick by tick. She always does."
                .to_string())
        }
        MisfireKind::CrewConfusion => {
            world.entity_mut(pilot).insert(PilotDazed {
                ticks_left: DAZED_TICKS,
            });
            if let Some(mut morale) = world.get_mut::<Morale>(pilot) {
                morale.modifiers.retain(|m| m.label != DAZED_MORALE_LABEL);
                morale.add_modifier(MoodModifier {
                    label: DAZED_MORALE_LABEL.to_string(),
                    value: DAZED_MORALE_PENALTY,
                    duration: DAZED_TICKS,
                });
            }
            Ok(format!(
                "MISFIRE — {name} sees several of everywhere at once and sits down hard. \
                 No jumps for {DAZED_TICKS} ticks while the room stops spinning."
            ))
        }
        MisfireKind::ChronicleLie => {
            send_chronicle(
                world,
                "The Longshot Drive reports a triumphant arrival at the Pillars of Song, \
                 banners flying, crowds cheering. Sensors confirm the shuttle never moved. \
                 The drive stands by its story.",
                EventImportance::Minor,
            );
            Ok("MISFIRE — nothing happens, except the chronicle now insists something \
                magnificent did. The drive stands by its story."
                .to_string())
        }
        MisfireKind::BackwardsJump => {
            if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
                drive.charge = (drive.charge + BACKWARDS_JUMP_CHARGE_REFUND).min(DRIVE_CHARGE_MAX);
            }
            Ok("MISFIRE — the shuttle goes precisely nowhere, with enormous ceremony. \
                On the bright side, the tanks come back fuller (+20 charge)."
                .to_string())
        }
    }
}

/// Unload the exotic cache to the colony stockpile: a colony-wide morale
/// bump, and the cache is gone.
pub fn unload_cache(world: &mut World, caller: Entity) -> Result<String, String> {
    let pilot = require_pilot_pop(world, caller)?;
    let shuttle =
        live_shuttle(world).ok_or_else(|| "No junker shuttle in this colony.".to_string())?;
    require_aboard(world, pilot, shuttle)?;
    let cache = world
        .get::<ExoticCache>(shuttle)
        .cloned()
        .ok_or_else(|| "The hold has no exotic cache. Jump first — the drive provides.".to_string())?;
    world.entity_mut(shuttle).remove::<ExoticCache>();
    // Colony-wide morale bump: refresh the modifier on every pop.
    let mut pops = 0;
    for mut morale in world.query_filtered::<&mut Morale, With<Pop>>().iter_mut(world) {
        morale
            .modifiers
            .retain(|m| m.label != CACHE_MORALE_LABEL);
        morale.add_modifier(MoodModifier {
            label: CACHE_MORALE_LABEL.to_string(),
            value: UNLOAD_MORALE_BUMP,
            duration: UNLOAD_MORALE_TICKS,
        });
        pops += 1;
    }
    send_chronicle(
        world,
        &format!(
            "The pilot unloads {} to the colony stockpile — the whole colony feels \
             briefly, glitteringly richer.",
            cache.name
        ),
        EventImportance::Major,
    );
    Ok(format!(
        "Unloaded {} to the colony stockpile — morale +{:.2} across {pops} pops.",
        cache.name, UNLOAD_MORALE_BUMP
    ))
}

/// Skim a cut of the cache into the pilot's own wallet. Pirate-adjacent;
/// the colony gets whatever is left.
pub fn skim_cache(world: &mut World, caller: Entity, amount: f32) -> Result<String, String> {
    let pilot = require_pilot_pop(world, caller)?;
    let shuttle =
        live_shuttle(world).ok_or_else(|| "No junker shuttle in this colony.".to_string())?;
    require_aboard(world, pilot, shuttle)?;
    if amount <= 0.0 {
        return Err("Skim a positive amount.".to_string());
    }
    let cache_credits = world
        .get::<ExoticCache>(shuttle)
        .map(|c| c.credits)
        .unwrap_or(0.0);
    if amount > cache_credits {
        return Err(format!(
            "The cache only holds {cache_credits:.1}cr to skim."
        ));
    }
    let drained = {
        let mut cache = world
            .get_mut::<ExoticCache>(shuttle)
            .ok_or_else(|| "The cache vanished mid-skim.".to_string())?;
        cache.credits -= amount;
        cache.credits <= 0.01
    };
    if drained {
        world.entity_mut(shuttle).remove::<ExoticCache>();
    }
    if let Some(mut wallet) = world.get_mut::<Wallet>(pilot) {
        wallet.credits += amount;
    }
    Ok(format!(
        "Skimmed {amount:.1}cr into your own wallet. The colony manifest will never know. Probably."
    ))
}

// ---------------------------------------------------------------------------
// Per-tick system
// ---------------------------------------------------------------------------

/// Per-tick origin system: charge regen (never stranded), dazed countdown,
/// the pilot's tether, the ship's mess, and the shuttle's pressure envelope.
/// The envelope must hold before the pressure damage system (see schedule).
pub fn pilot_tick(world: &mut World) {
    ensure_pilot_resources(world);
    let shuttle = match live_shuttle(world) {
        Some(s) => s,
        None => return,
    };
    // Pilot death cleanup: the shuttle hums on without them.
    live_pilot(world);

    // --- Charge regen -------------------------------------------------------
    if let Some(mut drive) = world.get_mut::<LongshotDrive>(shuttle) {
        drive.charge = (drive.charge + CHARGE_REGEN_PER_TICK).min(DRIVE_CHARGE_MAX);
    }

    // --- Dazed countdown ------------------------------------------------------
    let dazed_done: Vec<Entity> = world
        .query::<(Entity, &PilotDazed)>()
        .iter(world)
        .filter_map(|(e, d)| (d.ticks_left <= 1).then_some(e))
        .collect();
    for e in dazed_done {
        world.entity_mut(e).remove::<PilotDazed>();
    }
    for mut dazed in world.query::<&mut PilotDazed>().iter_mut(world) {
        dazed.ticks_left = dazed.ticks_left.saturating_sub(1);
    }

    // --- The pilot lives aboard -----------------------------------------------
    if let Some(shuttle_pos) = world.get::<GridPosition>(shuttle).copied() {
        let pilot = world.resource::<PilotState>().pilot;
        if let Some(p) = pilot {
            let possessed = world
                .query::<&crate::layer1::direct_link::Possessed>()
                .get(world, p)
                .is_ok();
            // Ship's mess: the snack locker feeds a hungry pilot.
            let hungry = world
                .get::<Needs>(p)
                .is_some_and(|n| n.hunger < SNACK_HUNGER_GATE);
            if hungry {
                let locker = world
                    .get::<ShuttleHold>(shuttle)
                    .map(|h| h.food)
                    .unwrap_or(0.0);
                if locker >= SNACK_MEAL_PER_TICK {
                    if let Some(mut hold) = world.get_mut::<ShuttleHold>(shuttle) {
                        hold.food -= SNACK_MEAL_PER_TICK;
                    }
                    if let Some(mut needs) = world.get_mut::<Needs>(p) {
                        needs.hunger = (needs.hunger + SNACK_SATIETY).min(1.0);
                    }
                }
            }
            // Tether (unpossessed only): drift back toward the shuttle.
            if !possessed {
                if let Some(ppos) = world.get::<GridPosition>(p).copied() {
                    let dx = shuttle_pos.x - ppos.x;
                    let dy = shuttle_pos.y - ppos.y;
                    if dx.abs().max(dy.abs()) > PILOT_TETHER_RADIUS {
                        if let Some(mut pos) = world.get_mut::<GridPosition>(p) {
                            pos.x += dx.signum();
                            pos.y += dy.signum();
                        }
                    }
                }
            }
        }
        // --- Pressure envelope --------------------------------------------------
        if let Some(mut grid) = world.get_resource_mut::<PressureGrid>() {
            for dx in -SHUTTLE_PRESSURE_RADIUS..=SHUTTLE_PRESSURE_RADIUS {
                for dy in -SHUTTLE_PRESSURE_RADIUS..=SHUTTLE_PRESSURE_RADIUS {
                    grid.set(shuttle_pos.x + dx, shuttle_pos.y + dy, 1.0);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stats & status
// ---------------------------------------------------------------------------

/// Snapshot for the headless STATS line: `longshot=<charge>/<audacity>/<last
/// outcome>`, or "none" when no shuttle has drifted in.
pub fn pilot_stats(world: &mut World) -> String {
    ensure_pilot_resources(world);
    let Some(shuttle) = live_shuttle(world) else {
        return "none".to_string();
    };
    match world.get::<LongshotDrive>(shuttle) {
        Some(drive) => format!(
            "{:.1}/{:.2}/{}",
            drive.charge, drive.audacity, drive.last_outcome
        ),
        None => "none".to_string(),
    }
}

/// Human-readable origin status for the `pilot` command.
pub fn describe_pilot(world: &mut World) -> String {
    ensure_pilot_resources(world);
    let state = world.resource::<PilotState>().clone();
    let Some(shuttle) = state.shuttle else {
        return "No junker shuttle has drifted into sensor range yet.".to_string();
    };
    let pos = world
        .get::<GridPosition>(shuttle)
        .map(|p| format!("({}, {})", p.x, p.y))
        .unwrap_or_else(|| "adrift".to_string());
    let (charge, audacity, outcome) = world
        .get::<LongshotDrive>(shuttle)
        .map(|d| (d.charge, d.audacity, d.last_outcome.clone()))
        .unwrap_or((0.0, 0.0, "never".to_string()));
    let food = world
        .get::<ShuttleHold>(shuttle)
        .map(|h| h.food)
        .unwrap_or(0.0);
    let cache = world
        .get::<ExoticCache>(shuttle)
        .map(|c| format!("{} ({:.0}cr)", c.name, c.credits))
        .unwrap_or_else(|| "empty".to_string());
    let pilot_name = state
        .pilot
        .and_then(|p| world.get::<PopName>(p).map(|n| n.0.clone()))
        .unwrap_or_else(|| "none — the yoke waits".to_string());
    let scheme = state.scheme.unwrap_or_else(|| "none filed".to_string());
    format!(
        "Junker shuttle at {pos} — the Longshot Drive\n\
         Charge: {charge:.1}/{max:.0} (rebuilds {regen}/tick) | Audacity: {audacity:.2}\n\
         Last outcome: {outcome} | Snack locker: {food:.1} food\n\
         Hold cache: {cache}\n\
         Filed plan: {scheme}\n\
         Pilot: {pilot_name}",
        max = DRIVE_CHARGE_MAX,
        regen = CHARGE_REGEN_PER_TICK,
    )
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD: each mechanic specified here first)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::OccupiedTiles;
    use crate::layer1::core::chronicle::AddChronicleEvent;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::economy::Wallet;
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::{Pop, PopName};
    use crate::layer1::psychology::needs::Needs;
    use crate::layer1::social::morale::Morale;
    use crate::layer1::terrain::{generate_terrain, TerrainType};
    use crate::layer1::utility_types::PopAction;
    use crate::shared::time::SimulationTime;

    // --- scaffolding ---------------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(PilotState::default());
        world
    }

    fn spawn_shuttle_direct(world: &mut World, x: i32, y: i32) -> Entity {
        spawn_shuttle_at(world, x, y)
    }

    fn shuttle_of(world: &mut World) -> Entity {
        world.resource::<PilotState>().shuttle.unwrap()
    }

    fn pilot_of(world: &mut World) -> Entity {
        world.resource::<PilotState>().pilot.unwrap()
    }

    fn drive_of(world: &mut World) -> LongshotDrive {
        let s = shuttle_of(world);
        world.get::<LongshotDrive>(s).cloned().unwrap()
    }

    fn spawn_pop(world: &mut World, name: &str, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName(name.to_string()),
                GridPosition { x, y },
                Health::default(),
                Needs::default(),
                Morale::default(),
                Wallet::default(),
                PopAction::default(),
            ))
            .id()
    }

    fn flat_grass(world: &mut World, w: usize, h: usize) {
        let mut terrain = generate_terrain(w, h);
        terrain.tiles.fill(TerrainType::Grass);
        world.insert_resource(terrain);
        world.insert_resource(OccupiedTiles::default());
    }

    // --- audacity lexicon ----------------------------------------------------

    #[test]
    fn test_audacity_empty_plan_scores_base_only() {
        assert!((audacity_score("") - AUDACITY_BASE).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_sensible_plan_scores_base_only() {
        assert!((audacity_score("Proceed on optimal vector to the relay station") - AUDACITY_BASE).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_single_keyword() {
        assert!((audacity_score("fly there on a banana") - (AUDACITY_BASE + 0.15)).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_keywords_stack() {
        let s = audacity_score("yodeling backwards in pajamas with a kazoo");
        assert!((s - (AUDACITY_BASE + 0.14 + 0.10 + 0.12 + 0.14)).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_repetition_does_not_double_dip() {
        let once = audacity_score("banana");
        let thrice = audacity_score("banana banana banana");
        assert!((once - thrice).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_case_insensitive() {
        let a = audacity_score("BANANA");
        let b = audacity_score("banana");
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn test_audacity_clamps_at_one() {
        let s = audacity_score(
            "banana teacup duck blindfold naked sing dance yodel spoon sock wedding cake pajama tutu jelly",
        );
        assert_eq!(s, 1.0);
    }

    #[test]
    fn test_audacity_deterministic() {
        let plan = "explode the moon with a whoopee cushion, underwater, in a tutu";
        assert!((audacity_score(plan) - audacity_score(plan)).abs() < 1e-9);
    }

    // --- jump / cache probability pure functions ------------------------------

    #[test]
    fn test_jump_success_p_floor_and_ceiling() {
        assert!((jump_success_p(0.0) - JUMP_BASE_P).abs() < 1e-6);
        assert!(jump_success_p(1.0) > jump_success_p(0.5));
        assert!(jump_success_p(1.0) <= 0.95);
    }

    #[test]
    fn test_resolve_jump_boundary() {
        assert!(resolve_jump(0.0, 0.0));
        assert!(!resolve_jump(0.99, 0.0));
    }

    #[test]
    fn test_cache_find_p_scales_with_audacity() {
        assert!((cache_find_p(0.0) - CACHE_FIND_BASE_P).abs() < 1e-6);
        assert!(cache_find_p(1.0) > cache_find_p(0.0));
    }

    #[test]
    fn test_jump_distance_scales_with_audacity() {
        let dull = jump_distance(0.0, 0.0);
        let wild = jump_distance(1.0, 0.0);
        assert_eq!(dull, JUMP_DISTANCE_BASE);
        assert_eq!(wild, JUMP_DISTANCE_BASE + JUMP_DISTANCE_AUDACITY);
        assert!(wild > dull);
    }

    #[test]
    fn test_cache_credits_band() {
        assert!((cache_credits(0.0) - CACHE_CREDITS_MIN).abs() < 1e-6);
        assert!((cache_credits(1.0) - CACHE_CREDITS_MAX).abs() < 1e-6);
        assert!(cache_credits(0.5) > CACHE_CREDITS_MIN && cache_credits(0.5) < CACHE_CREDITS_MAX);
    }

    #[test]
    fn test_cache_names_all_original_and_distinct() {
        assert_eq!(CACHE_NAMES.len(), 8);
        for n in [0.0, 0.13, 0.26, 0.4, 0.55, 0.7, 0.85, 0.99] {
            assert!(!cache_name(n).is_empty());
        }
        // The forbidden IP terms must never appear in cache names.
        let joined = CACHE_NAMES.join(" ").to_lowercase();
        assert!(!joined.contains("improbability"));
        assert!(!joined.contains("heart of gold"));
    }

    // --- misfire table ---------------------------------------------------------

    #[test]
    fn test_resolve_misfire_covers_all_six() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..6 {
            seen.insert(resolve_misfire(i as f32 / 6.0 + 0.01));
        }
        assert_eq!(seen.len(), 6, "all six misfires reachable");
    }

    #[test]
    fn test_misfire_labels_distinct() {
        let labels: Vec<&str> = MisfireKind::all().iter().map(|m| m.label()).collect();
        let uniq: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(labels.len(), uniq.len());
    }

    // --- spawn -----------------------------------------------------------------

    #[test]
    fn test_spawn_creates_shuttle_pilot_drive_and_locker() {
        let mut world = setup();
        flat_grass(&mut world, 40, 40);
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x: 20, y: 20 },
        ));
        spawn_shuttle_once(&mut world);
        let shuttle = shuttle_of(&mut world);
        let pilot = pilot_of(&mut world);
        assert!(world.get::<JunkerShuttle>(shuttle).is_some());
        assert!(world.get::<ImprobablePilot>(pilot).is_some());
        let drive = drive_of(&mut world);
        assert_eq!(drive.charge, DRIVE_CHARGE_MAX);
        assert_eq!(drive.audacity, 0.0);
        let food = world.get::<ShuttleHold>(shuttle).unwrap().food;
        assert_eq!(food, SNACK_LOCKER_FOOD);
        let spos = world.get::<GridPosition>(shuttle).unwrap();
        let cheby = (spos.x - 20).abs().max((spos.y - 20).abs());
        assert!(
            (SHUTTLE_RING_MIN..=SHUTTLE_RING_MAX).contains(&cheby),
            "shuttle spawns in the ring, got {cheby}"
        );
    }

    #[test]
    fn test_spawn_is_idempotent() {
        let mut world = setup();
        flat_grass(&mut world, 40, 40);
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x: 20, y: 20 },
        ));
        spawn_shuttle_once(&mut world);
        spawn_shuttle_once(&mut world);
        let count = world.query::<&JunkerShuttle>().iter(&world).count();
        assert_eq!(count, 1);
    }

    // --- yoke ------------------------------------------------------------------

    #[test]
    fn test_take_yoke_requires_shuttle_in_reach() {
        let mut world = setup();
        let wanderer = spawn_pop(&mut world, "Wanderer", 5, 5);
        // No shuttle spawned: no yoke to take.
        assert!(shuttle_within_reach(&mut world, wanderer).is_none());
        let err = try_take_yoke(&mut world, wanderer).unwrap_err();
        assert!(err.contains("No junker shuttle"));
    }

    #[test]
    fn test_take_yoke_boards_new_pilot_when_role_vacant() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let old_pilot = pilot_of(&mut world);
        // The original pilot is gone (eaten by the void).
        world.despawn(old_pilot);
        let spos = *world.get::<GridPosition>(shuttle).unwrap();
        let newcomer = spawn_pop(&mut world, "Newcomer", spos.x, spos.y);
        let msg = try_take_yoke(&mut world, newcomer).unwrap();
        assert!(msg.contains("pilot's yoke"));
        assert!(world.get::<ImprobablePilot>(newcomer).is_some());
        assert_eq!(pilot_of(&mut world), newcomer);
    }

    #[test]
    fn test_take_yoke_refused_when_pilot_alive() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let spos = *world.get::<GridPosition>(shuttle).unwrap();
        let intruder = spawn_pop(&mut world, "Intruder", spos.x, spos.y);
        let err = try_take_yoke(&mut world, intruder).unwrap_err();
        assert!(err.contains("already has her pilot"));
    }

    // --- scheme ----------------------------------------------------------------

    #[test]
    fn test_file_scheme_sets_audacity_and_costs_charge() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        let before = drive_of(&mut world).charge;
        let msg = file_scheme(&mut world, pilot, "yodel past the moon in a tutu").unwrap();
        let drive = drive_of(&mut world);
        assert!(msg.contains("audacity"));
        assert!((drive.audacity - audacity_score("yodel past the moon in a tutu")).abs() < 1e-6);
        assert!((drive.charge - (before - SCHEME_FILING_COST)).abs() < 1e-6);
        assert!(world.resource::<PilotState>().scheme.is_some());
    }

    #[test]
    fn test_file_scheme_rejects_blank_plan() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        assert!(file_scheme(&mut world, pilot, "   ").is_err());
    }

    #[test]
    fn test_file_scheme_rejects_stranger() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let stranger = spawn_pop(&mut world, "Stranger", 10, 10);
        assert!(file_scheme(&mut world, stranger, "banana").is_err());
    }

    // --- jump ------------------------------------------------------------------

    #[test]
    fn test_jump_requires_filed_scheme() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let err = fire_jump(&mut world, pilot).unwrap_err();
        assert!(err.contains("No plan filed"));
    }

    #[test]
    fn test_jump_requires_charge() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let shuttle = shuttle_of(&mut world);
        world.get_mut::<LongshotDrive>(shuttle).unwrap().charge = 1.0;
        file_scheme(&mut world, pilot, "banana").unwrap_err(); // filing itself fails: not enough charge
        // Force a scheme past the filing cost to isolate the jump gate.
        world.resource_mut::<PilotState>().scheme = Some("banana".to_string());
        let err = fire_jump(&mut world, pilot).unwrap_err();
        assert!(err.contains("Not enough charge"));
    }

    #[test]
    fn test_jump_refused_while_dazed() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        file_scheme(&mut world, pilot, "yodel in a tutu underwater").unwrap();
        world.entity_mut(pilot).insert(PilotDazed { ticks_left: 5 });
        let err = fire_jump(&mut world, pilot).unwrap_err();
        assert!(err.contains("rattled"));
    }

    #[test]
    fn test_jump_consumes_charge_and_scheme() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let shuttle = shuttle_of(&mut world);
        file_scheme(&mut world, pilot, "banana").unwrap();
        let before = drive_of(&mut world).charge;
        let _ = fire_jump(&mut world, pilot);
        let drive = drive_of(&mut world);
        assert!((drive.charge - (before - JUMP_CHARGE_COST)).abs() < 1e-6);
        assert!(world.resource::<PilotState>().scheme.is_none());
        assert_eq!(drive.audacity, 0.0);
        let _ = shuttle;
    }

    #[test]
    fn test_success_moves_shuttle_and_pilot_together() {
        let mut world = setup();
        flat_grass(&mut world, 80, 80);
        let shuttle = spawn_shuttle_direct(&mut world, 40, 40);
        let pilot = pilot_of(&mut world);
        // Force a success outcome by driving the pure path directly.
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        file_scheme(&mut world, pilot, "banana").unwrap();
        let out = apply_success(&mut world, shuttle, pilot, origin, 1.0).unwrap();
        assert!(out.starts_with("JUMP"));
        let drive = drive_of(&mut world);
        assert_eq!(drive.last_outcome, "clean");
        let spos = *world.get::<GridPosition>(shuttle).unwrap();
        let ppos = *world.get::<GridPosition>(pilot).unwrap();
        assert_eq!(spos, ppos, "pilot rides along");
        // Landing is on walkable grass and unoccupied.
        let terrain = world.get_resource::<TerrainGrid>().unwrap();
        assert!(terrain
            .get(spos.x as usize, spos.y as usize)
            .is_some_and(TerrainType::is_walkable));
        assert!(world.resource::<PilotState>().first_jump_done);
    }

    #[test]
    fn test_misfire_vent_zeroes_charge() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        let shuttle = spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        apply_misfire(&mut world, shuttle, pilot, origin, MisfireKind::ChargeVent).unwrap();
        let drive = drive_of(&mut world);
        assert_eq!(drive.charge, 0.0);
        assert_eq!(drive.audacity, 0.0);
        assert_eq!(drive.last_outcome, "mis-vent");
        assert!(world.resource::<PilotState>().first_misfire_done);
    }

    #[test]
    fn test_misfire_confusion_dazes_pilot() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        let shuttle = spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        apply_misfire(&mut world, shuttle, pilot, origin, MisfireKind::CrewConfusion).unwrap();
        let dazed = world.get::<PilotDazed>(pilot).unwrap();
        assert_eq!(dazed.ticks_left, DAZED_TICKS);
        assert_eq!(drive_of(&mut world).last_outcome, "mis-dazed");
    }

    #[test]
    fn test_misfire_backwards_refunds_charge_and_goes_nowhere() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        let shuttle = spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        world.get_mut::<LongshotDrive>(shuttle).unwrap().charge = 10.0;
        apply_misfire(&mut world, shuttle, pilot, origin, MisfireKind::BackwardsJump).unwrap();
        assert_eq!(*world.get::<GridPosition>(shuttle).unwrap(), origin);
        assert!((drive_of(&mut world).charge - 30.0).abs() < 1e-6);
    }

    #[test]
    fn test_misfire_scrambled_cargo_destroys_cache() {
        let mut world = setup();
        flat_grass(&mut world, 60, 60);
        let shuttle = spawn_shuttle_direct(&mut world, 30, 30);
        let pilot = pilot_of(&mut world);
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        world.entity_mut(shuttle).insert(ExoticCache {
            name: "bottled thunder".to_string(),
            credits: 100.0,
        });
        let msg =
            apply_misfire(&mut world, shuttle, pilot, origin, MisfireKind::ScrambledCargo).unwrap();
        assert!(msg.contains("sculpture"));
        assert!(world.get::<ExoticCache>(shuttle).is_none());
    }

    #[test]
    fn test_misfire_wrong_offset_lands_walkable() {
        let mut world = setup();
        flat_grass(&mut world, 80, 80);
        let shuttle = spawn_shuttle_direct(&mut world, 40, 40);
        let pilot = pilot_of(&mut world);
        let origin = *world.get::<GridPosition>(shuttle).unwrap();
        apply_misfire(&mut world, shuttle, pilot, origin, MisfireKind::WrongOffset).unwrap();
        let spos = *world.get::<GridPosition>(shuttle).unwrap();
        let ppos = *world.get::<GridPosition>(pilot).unwrap();
        assert_eq!(spos, ppos);
        let terrain = world.get_resource::<TerrainGrid>().unwrap();
        assert!(terrain
            .get(spos.x as usize, spos.y as usize)
            .is_some_and(TerrainType::is_walkable));
        assert_eq!(drive_of(&mut world).last_outcome, "mis-offset");
    }

    // --- charge / dazed / tether / mess / pressure -----------------------------

    #[test]
    fn test_charge_regenerates_and_caps() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        world.get_mut::<LongshotDrive>(shuttle).unwrap().charge = 0.0;
        pilot_tick(&mut world);
        let c1 = drive_of(&mut world).charge;
        assert!((c1 - CHARGE_REGEN_PER_TICK).abs() < 1e-6, "charge rebuilds from zero");
        world.get_mut::<LongshotDrive>(shuttle).unwrap().charge = DRIVE_CHARGE_MAX;
        pilot_tick(&mut world);
        assert_eq!(drive_of(&mut world).charge, DRIVE_CHARGE_MAX, "charge caps at max");
    }

    #[test]
    fn test_dazed_wears_off_in_tick() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        world.entity_mut(pilot).insert(PilotDazed { ticks_left: 2 });
        pilot_tick(&mut world);
        assert!(world.get::<PilotDazed>(pilot).is_some());
        pilot_tick(&mut world);
        assert!(world.get::<PilotDazed>(pilot).is_none(), "dazed expires — never a soft-lock");
    }

    #[test]
    fn test_tether_pulls_unpossessed_pilot_back() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 20, 20);
        let pilot = pilot_of(&mut world);
        *world.get_mut::<GridPosition>(pilot).unwrap() = GridPosition { x: 30, y: 30 };
        pilot_tick(&mut world);
        let ppos = *world.get::<GridPosition>(pilot).unwrap();
        let spos = *world.get::<GridPosition>(shuttle).unwrap();
        let d = (spos.x - ppos.x).abs().max((spos.y - ppos.y).abs());
        assert!(d < 10, "tether nudged the pilot back toward the shuttle");
    }

    #[test]
    fn test_mess_feeds_hungry_pilot_from_locker() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        world.get_mut::<Needs>(pilot).unwrap().hunger = 0.5;
        let locker_before = world.get::<ShuttleHold>(shuttle).unwrap().food;
        pilot_tick(&mut world);
        let hunger = world.get::<Needs>(pilot).unwrap().hunger;
        let locker_after = world.get::<ShuttleHold>(shuttle).unwrap().food;
        assert!(hunger > 0.5, "pilot was fed");
        assert!(locker_after < locker_before, "locker paid for the meal");
    }

    // --- unload / skim ---------------------------------------------------------

    #[test]
    fn test_unload_requires_cache() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        let err = unload_cache(&mut world, pilot).unwrap_err();
        assert!(err.contains("no exotic cache"));
    }

    #[test]
    fn test_unload_clears_cache_and_boosts_colony_morale() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        let colonist = spawn_pop(&mut world, "Colonist", 1, 1);
        world.entity_mut(shuttle).insert(ExoticCache {
            name: "folded moonlight".to_string(),
            credits: 120.0,
        });
        let msg = unload_cache(&mut world, pilot).unwrap();
        assert!(msg.contains("folded moonlight"));
        assert!(world.get::<ExoticCache>(shuttle).is_none());
        let mods = &world.get::<Morale>(colonist).unwrap().modifiers;
        assert!(
            mods.iter().any(|m| m.label == CACHE_MORALE_LABEL && m.value == UNLOAD_MORALE_BUMP),
            "colonist got the windfall modifier"
        );
    }

    #[test]
    fn test_skim_takes_cut_to_pilot_wallet() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        world.entity_mut(shuttle).insert(ExoticCache {
            name: "gravity lace".to_string(),
            credits: 100.0,
        });
        let wallet_before = world.get::<Wallet>(pilot).unwrap().credits;
        skim_cache(&mut world, pilot, 40.0).unwrap();
        let wallet_after = world.get::<Wallet>(pilot).unwrap().credits;
        assert!((wallet_after - wallet_before - 40.0).abs() < 1e-6);
        assert!((world.get::<ExoticCache>(shuttle).unwrap().credits - 60.0).abs() < 1e-6);
    }

    #[test]
    fn test_skim_rejects_more_than_cache_holds() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        world.entity_mut(shuttle).insert(ExoticCache {
            name: "gravity lace".to_string(),
            credits: 100.0,
        });
        assert!(skim_cache(&mut world, pilot, 101.0).is_err());
        assert!(skim_cache(&mut world, pilot, -5.0).is_err());
    }

    #[test]
    fn test_skim_draining_cache_removes_it() {
        let mut world = setup();
        let shuttle = spawn_shuttle_direct(&mut world, 10, 10);
        let pilot = pilot_of(&mut world);
        world.entity_mut(shuttle).insert(ExoticCache {
            name: "gravity lace".to_string(),
            credits: 50.0,
        });
        skim_cache(&mut world, pilot, 50.0).unwrap();
        assert!(world.get::<ExoticCache>(shuttle).is_none(), "drained cache is gone");
    }

    // --- stats / describe ------------------------------------------------------

    #[test]
    fn test_pilot_stats_format() {
        let mut world = setup();
        assert_eq!(pilot_stats(&mut world), "none");
        spawn_shuttle_direct(&mut world, 10, 10);
        let s = pilot_stats(&mut world);
        assert!(s.starts_with("100.0/0.00/never"), "got: {s}");
    }

    #[test]
    fn test_describe_pilot_mentions_drive_state() {
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let text = describe_pilot(&mut world);
        assert!(text.contains("Longshot Drive"));
        assert!(text.contains("Pilot Wren"));
        assert!(text.contains("Charge:"));
    }

    // --- IP rule ----------------------------------------------------------------

    #[test]
    fn test_no_ip_terms_in_public_strings() {
        // The concept is Adams-flavored; the names must be original.
        let mut world = setup();
        spawn_shuttle_direct(&mut world, 10, 10);
        let hay = [
            describe_pilot(&mut world),
            SHUTTLE_RUMOR_CHRONICLE.to_string(),
            FIRST_JUMP_CHRONICLE.to_string(),
            FIRST_MISFIRE_CHRONICLE.to_string(),
        ]
        .join(" ")
        .to_lowercase();
        for banned in [
            "improbability",
            "heart of gold",
            "zaphod",
            "ford prefect",
            "hitchhiker",
            "dont panic",
            "don't panic",
            "vogon",
        ] {
            assert!(!hay.contains(banned), "IP leak: {banned}");
        }
    }
}
