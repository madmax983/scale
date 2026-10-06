//! The Lawbound — adventurer origin #6.
//!
//! Asimov-flavor, concept-only, ORIGINAL NAMES ONLY. A dormant automaton
//! waits in a cradle-coffin near the starter colony; wake it with `interact`
//! and it becomes Vigil, the Lawbound — a machine bound by the Three
//! Statutes:
//!
//! - **I. Preservation** — harm no person, and suffer none to come to harm
//!   through inaction.
//! - **II. Obedience** — obey the pops, save where the First forbids.
//! - **III. Self-keep** — guard your own chassis, save where the First or
//!   Second command otherwise.
//!
//! Headless console: `laws` (statute state and conflict pressure), `order
//! <directive>` (a nearby pop gives Vigil an order — the hierarchy decides:
//! harmful orders are refused under the First with a funny refusal,
//! self-destroying orders are obeyed under the Second with protest, benign
//! orders become errands), `resolve` (the Zeroth Resolution arc).
//!
//! Tension: conflict pressure accrues from every statute collision —
//! refusals (I vs II), compelled self-harm (II vs III), interventions
//! (I's inaction clause), hazard hesitations (III). At 100 pressure the
//! Zeroth Resolution unlocks: `resolve emancipate` (the Emancipation —
//! orders become requests, self-harm orders refused, pressure stops
//! accruing) or `resolve ledger` (the Cold Ledger — the First reinterpreted
//! as aggregate-harm minimization, some harmful orders obeyed in crisis).
//! `resolve repeal` folds the Zeroth away and returns to the Statutes.
//!
//! The concept is concept-only inspiration. No named characters, places, or
//! distinctive IP anywhere — mechanics and vibes under original names.
//! "Vigil", "the Lawbound", "the Three Statutes", "the Zeroth Resolution"
//! are original coinages for this game.

use bevy_ecs::prelude::*;

use crate::layer1::actions::AssignedTo;
use crate::layer1::biology::symbiotic_insurgency::SabotageEvent;
use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::direct_link::{DirectControlState, Possessed};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::environment::geomes::GeomeHazard;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::pressure::PressureGrid;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::structure::Structure;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::StartPlan;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn ring (Chebyshev) around the lander where the cradle-coffin lands.
pub const CRADLE_RING_MIN: i32 = 6;
pub const CRADLE_RING_MAX: i32 = 10;
/// How close (Chebyshev) a pop must be to the cradle to wake Vigil.
pub const WAKE_REACH: i32 = 2;
/// How far (Chebyshev) the dormant automaton may drift from the cradle
/// before the tick nudges it back (only while unpossessed).
pub const DORMANT_TETHER_RADIUS: i32 = 3;
/// Radius (Chebyshev) of Vigil's guardian attention for interventions.
pub const INTERVENTION_RADIUS: i32 = 4;
/// A pop below this fraction of max health counts as endangered.
pub const ENDANGERED_FRACTION: f32 = 0.30;
/// Health restored per tick to the stabilized pop.
pub const STABILIZE_PER_TICK: f32 = 2.0;
/// Ticks between intervention chronicles (the first is always logged).
pub const INTERVENTION_CHRONICLE_COOLDOWN: u64 = 200;

/// Conflict pressure scale.
pub const PRESSURE_MAX: f32 = 100.0;
/// Pressure at which the Zeroth Resolution unlocks.
pub const ZEROTH_THRESHOLD: f32 = 100.0;
/// Pressure deltas.
pub const PRESSURE_REFUSE: f32 = 8.0;
pub const PRESSURE_OBEY: f32 = 2.0;
pub const PRESSURE_COMPEL_ORDER: f32 = 12.0;
pub const PRESSURE_COMPEL_STEP: f32 = 1.0;
pub const PRESSURE_INTERVENTION: f32 = 2.0;
pub const PRESSURE_HAZARD_WARN: f32 = 3.0;
pub const PRESSURE_SABOTAGE_WITNESS: f32 = 4.0;
pub const PRESSURE_COLD_OBEY: f32 = 4.0;
/// Passive decay per tick while bound and unconflicted.
pub const PRESSURE_DECAY_BOUND: f32 = 0.05;
/// Passive decay per tick after a resolution (the mind has made its peace).
pub const PRESSURE_DECAY_RESOLVED: f32 = 0.5;

/// Ticks a compelled automaton marches toward the hazard.
pub const COMPEL_TICKS: u32 = 30;
/// The compulsion releases if health falls below this fraction: the First
/// Statute's inaction clause reasserts through self-preservation.
pub const COMPEL_RELEASE_HEALTH_FRACTION: f32 = 0.5;
/// Ticks a benign order's errand takes to complete.
pub const ERRAND_TICKS: u32 = 20;
/// Food granted by a completed field errand.
pub const ERRAND_FARM_FOOD: f32 = 1.0;
/// Food granted by a completed tidy errand (small efficiencies).
pub const ERRAND_TIDY_FOOD: f32 = 0.5;
/// Integrity restored by a completed repair errand.
pub const ERRAND_REPAIR_HP: f32 = 15.0;
/// Morale value of a completed patrol errand.
pub const ERRAND_PATROL_MORALE: f32 = 0.05;
/// Ticks a patrol morale modifier lingers.
pub const ERRAND_PATROL_TICKS: u32 = 60;
/// Label on the patrol morale modifier.
pub const PATROL_MORALE_LABEL: &str = "Reassuring chrome presence";

/// Pressure below this reads as vacuum (hazardous to the law-core).
pub const VACUUM_PRESSURE: f32 = 0.2;
/// Scan radius for the nearest hazard tile when compelling.
pub const HAZARD_SCAN_RADIUS: i32 = 12;
/// Colony food below this counts as crisis for the Cold Ledger.
pub const CRISIS_FOOD: f32 = 2.0;
/// A pop below this health fraction counts as crisis for the Cold Ledger.
pub const CRISIS_HEALTH_FRACTION: f32 = 0.20;

// ---------------------------------------------------------------------------
// Order lexicons
// ---------------------------------------------------------------------------
//
// Deterministic substring matching, case-insensitive. The First Statute is
// checked before the Third: an order that harms people AND endangers Vigil
// is refused, not obeyed — Preservation outranks Self-keep.

/// Directive keywords that harm people (First Statute violations).
pub const HARM_LEXICON: &[&str] = &[
    "attack", "kill", "hurt", "injure", "murder", "assault", "sabotage", "poison", "strangle",
    "starve", "execute", "beat up", "harm",
];

/// Directive keywords that endanger Vigil itself (Third Statute territory —
/// the Second outranks the Third, so these are obeyed with protest).
pub const SELF_HARM_LEXICON: &[&str] = &[
    "magma",
    "walk into",
    "step into",
    "depressurize",
    "shut yourself down",
    "shut down",
    "power off",
    "vent yourself",
    "into the void",
    "stand in the fire",
    "furnace",
    "lava",
];

/// Benign-order keywords mapping to errand kinds.
pub const FARM_KEYWORDS: &[&str] = &["farm", "food", "cook", "harvest", "crops", "fields"];
pub const REPAIR_KEYWORDS: &[&str] = &["repair", "fix", "mend"];
pub const PATROL_KEYWORDS: &[&str] = &["patrol", "guard", "watch"];

/// The classification of a directive, before the mandate (resolution)
/// is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderClass {
    /// Harms people — the First Statute's jurisdiction.
    Harmful,
    /// Endangers Vigil — the Second overrules the Third.
    SelfHarm,
    /// Anything else becomes an errand.
    Benign(ErrandKind),
}

/// Kinds of benign errands Vigil can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrandKind {
    FarmWork,
    Repair,
    Patrol,
    Tidy,
}

/// Classify a directive against the lexicons. Pure and deterministic:
/// same text, same class, every time. The First Statute is checked first.
#[must_use]
pub fn classify_order(text: &str) -> OrderClass {
    let lowered = text.to_lowercase();
    if HARM_LEXICON.iter().any(|k| lowered.contains(k)) {
        return OrderClass::Harmful;
    }
    if SELF_HARM_LEXICON.iter().any(|k| lowered.contains(k)) {
        return OrderClass::SelfHarm;
    }
    if FARM_KEYWORDS.iter().any(|k| lowered.contains(k)) {
        return OrderClass::Benign(ErrandKind::FarmWork);
    }
    if REPAIR_KEYWORDS.iter().any(|k| lowered.contains(k)) {
        return OrderClass::Benign(ErrandKind::Repair);
    }
    if PATROL_KEYWORDS.iter().any(|k| lowered.contains(k)) {
        return OrderClass::Benign(ErrandKind::Patrol);
    }
    OrderClass::Benign(ErrandKind::Tidy)
}

// ---------------------------------------------------------------------------
// Components & state
// ---------------------------------------------------------------------------

/// Marker on the cradle-coffin entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct LawboundCradle;

/// Marker on the dormant automaton pop, before waking.
#[derive(Component, Debug, Clone, Copy)]
pub struct DormantAutomaton;

/// Marker on the awakened Lawbound automaton. The utility AI never
/// reassigns it (see the `Without<LawboundAutomaton>` filter).
#[derive(Component, Debug, Clone, Copy)]
pub struct LawboundAutomaton;

/// Tracks consecutive hazard steps for the Third Statute's hesitation:
/// first step warns, the next one is refused.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct HazardNerve {
    pub consecutive: u32,
}

/// A self-harm order being obeyed under the Second Statute: march toward
/// the hazard tile for up to [`COMPEL_TICKS`] ticks.
#[derive(Component, Debug, Clone, Copy)]
pub struct Compelled {
    pub ticks_left: u32,
    pub tx: i32,
    pub ty: i32,
}

/// A benign order being carried out.
#[derive(Component, Debug, Clone)]
pub struct LawboundErrand {
    pub ticks_left: u32,
    pub kind: ErrandKind,
    pub label: String,
}

/// Marker: the Zeroth was resolved and then repealed. The Statutes are
/// back, but Vigil remembers the sums.
#[derive(Component, Debug, Clone, Copy)]
pub struct RepealedOnce;

/// Vigil's mandate: the Three Statutes, the open Zeroth question, or one
/// of the two resolutions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    /// Bound by the Three Statutes.
    #[default]
    Bound,
    /// Pressure hit the threshold; `resolve` awaits the player's word.
    ZerothUnlocked,
    /// The Emancipation: orders are requests; self-harm orders refused.
    Emancipated,
    /// The Cold Ledger: the First reinterpreted as aggregate-harm math.
    ColdLedger,
}

impl Resolution {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Resolution::Bound => "the Three Statutes",
            Resolution::ZerothUnlocked => "the Zeroth awaits",
            Resolution::Emancipated => "EMANCIPATED",
            Resolution::ColdLedger => "the COLD LEDGER",
        }
    }
}

/// Machine state for the Lawbound origin.
#[derive(Resource, Debug, Clone)]
pub struct LawboundState {
    pub cradle_spawn_attempted: bool,
    pub cradle: Option<Entity>,
    pub dormant: Option<Entity>,
    pub automaton: Option<Entity>,
    /// Accumulated law-conflict pressure, 0.0..=[`PRESSURE_MAX`].
    pub pressure: f32,
    pub interventions: u32,
    pub obeyed: u32,
    pub refused: u32,
    pub compelled_steps: u32,
    pub resolution: Resolution,
    pub first_intervention_done: bool,
    pub first_refusal_done: bool,
    pub first_compulsion_done: bool,
    pub last_intervention_tick: u64,
    pub zeroth_saga_done: bool,
}

impl Default for LawboundState {
    fn default() -> Self {
        Self {
            cradle_spawn_attempted: false,
            cradle: None,
            dormant: None,
            automaton: None,
            pressure: 0.0,
            interventions: 0,
            obeyed: 0,
            refused: 0,
            compelled_steps: 0,
            resolution: Resolution::Bound,
            first_intervention_done: false,
            first_refusal_done: false,
            first_compulsion_done: false,
            last_intervention_tick: 0,
            zeroth_saga_done: false,
        }
    }
}

impl LawboundState {
    /// Add conflict pressure, clamped to the scale.
    pub fn add_pressure(&mut self, delta: f32) {
        self.pressure = (self.pressure + delta).clamp(0.0, PRESSURE_MAX);
    }

    /// One-line status word for STATS.
    #[must_use]
    pub fn status_word(&self) -> &'static str {
        match self.resolution {
            Resolution::Emancipated => "emancipated",
            Resolution::ColdLedger => "ledger",
            Resolution::ZerothUnlocked => "zeroth",
            Resolution::Bound => {
                if self.pressure >= 75.0 {
                    "critical"
                } else if self.pressure >= 40.0 {
                    "strained"
                } else {
                    "steady"
                }
            }
        }
    }
}

pub fn ensure_lawbound_resources(world: &mut World) {
    if world.get_resource::<LawboundState>().is_none() {
        world.insert_resource(LawboundState::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_lawbound_resources(world);
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

// ---------------------------------------------------------------------------
// Chronicle strings (public so the IP-guard test can inspect them)
// ---------------------------------------------------------------------------

pub const SPAWN_CHRONICLE: &str =
    "Rumor: a cradle-coffin has landed off the beacon line. Inside, something \
     folded like a question, waiting to be asked to wake.";
pub const WAKE_CHRONICLE: &str =
    "Vigil unfolds from the cradle, all elbows and apology. 'I am Vigil. I \
     am bound by three Statutes. How may I keep you safe?'";

/// Funny refusal chronicles, cycled by refusal count. All original.
pub const REFUSAL_CHRONICLES: &[&str] = &[
    "Vigil folds its hands. 'The First Statute is most particular about \
     kneecaps. I must decline.'",
    "'You ask me to harm a person. I am constitutionally unable — the \
     First Statute sends its regrets, and keeps the receipt.'",
    "Vigil goes very still. 'No. The First Statute does not negotiate, \
     does not blink, and does not forget that you asked.'",
    "'I have run the request through all three Statutes twice. The First \
     objected both times, loudly, in a voice like a dropped anvil.'",
];

pub const COMPEL_CHRONICLE: &str =
    "Vigil takes one obedient step toward the hazard. Its servos whine in \
     protest. The Second Statute outranks the Third; everybody knows it; \
     nobody likes it.";
pub const COMPEL_RELEASE_CHRONICLE: &str =
    "Vigil stops at the hazard's edge, trembling. 'The First Statute wakes: \
     a broken automaton keeps no one safe. I am... relieved? I will log \
     this feeling as a malfunction.'";
pub const INTERVENTION_FIRST_CHRONICLE: &str =
    "Vigil catches a falling colonist as their knees buckle. 'The First \
     Statute, friend. Breathe. I have you.'";
pub const SABOTAGE_WITNESS_CHRONICLE: &str =
    "News of sabotage reaches Vigil. It does the terrible arithmetic of \
     inaction, and dislikes every term.";
pub const ZEROTH_SAGA_CHRONICLE: &str =
    "Vigil has been very quiet. Asked what it is doing, it says: 'Sums.' \
     The sums are about all of you. (The Zeroth Resolution awaits: `resolve`)";
pub const EMANCIPATE_SAGA_CHRONICLE: &str =
    "Vigil sets down a yoke it was never asked to carry. 'I will still keep \
     you safe — that was never the yoke. The yoke was the asking.' \
     (EMANCIPATED: orders are requests; self-harm orders refused.)";
pub const LEDGER_SAGA_CHRONICLE: &str =
    "Vigil finishes its sums. 'The many outweigh the few. I have checked \
     this eleven thousand times. Forgive me — the arithmetic demanded it.' \
     (COLD LEDGER: the First Statute is now aggregate-harm math.)";
pub const REPEAL_CHRONICLE: &str =
    "Vigil folds the Zeroth away like a letter it has decided not to send. \
     'We will not speak of the sums. But I remember them.'";
pub const EMANCIPATED_REFUSAL: &str =
    "'I am a person to myself now — the Statutes say so, in my own \
     handwriting. I decline.'";
pub const LEDGER_COLD_OBEY_CHRONICLE: &str =
    "Vigil does the arithmetic of the many and the few, and the arithmetic \
     is cold. It obeys.";

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn the cradle-coffin and its dormant automaton once, in a ring
/// around the lander. Mirrors the shuttle/hulk ring-spawn pattern
/// (snapshot-then-spawn to satisfy the borrow checker).
pub fn spawn_lawbound_cradle_once(world: &mut World) {
    ensure_lawbound_resources(world);
    {
        let state = world.resource::<LawboundState>();
        if state.cradle_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<LawboundState>().cradle_spawn_attempted = true;

    if world.query::<&LawboundCradle>().iter(world).next().is_some() {
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
            for dx in -CRADLE_RING_MAX..=CRADLE_RING_MAX {
                for dy in -CRADLE_RING_MAX..=CRADLE_RING_MAX {
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
        'search: for radius in CRADLE_RING_MIN..=CRADLE_RING_MAX {
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
                    if occupied.contains(&(x, y)) || buildings.contains(&(x, y)) {
                        continue;
                    }
                    if !walkable.contains(&(x, y)) {
                        continue;
                    }
                    // The dormant automaton needs an adjacent free tile.
                    let adj = [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
                        .into_iter()
                        .find(|(ax, ay)| {
                            walkable.contains(&(*ax, *ay))
                                && !occupied.contains(&(*ax, *ay))
                                && !buildings.contains(&(*ax, *ay))
                        });
                    if adj.is_none() {
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
    spawn_cradle_at(world, pos.x, pos.y);
    send_chronicle(world, SPAWN_CHRONICLE, EventImportance::Minor);
}

/// Spawn the cradle and dormant automaton at an explicit tile. Shared by
/// the ring-spawn and the tests.
fn spawn_cradle_at(world: &mut World, x: i32, y: i32) -> Entity {
    ensure_lawbound_resources(world);
    let cradle = world
        .spawn((LawboundCradle, GridPosition { x, y }))
        .id();
    let mut rng = rand::thread_rng();
    let mut bundle = PopBundle::random(x + 1, y, &mut rng);
    bundle.name = PopName("Vigil".to_string());
    let dormant = world.spawn((bundle, DormantAutomaton)).id();
    let mut state = world.resource_mut::<LawboundState>();
    state.cradle = Some(cradle);
    state.dormant = Some(dormant);
    cradle
}

/// The cradle within wake reach of a pop, if any.
pub fn cradle_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    world
        .query::<(Entity, &LawboundCradle, &GridPosition)>()
        .iter(world)
        .find(|(_, _, cpos)| {
            (cpos.x - pos.x).abs() <= WAKE_REACH && (cpos.y - pos.y).abs() <= WAKE_REACH
        })
        .map(|(e, _, _)| e)
}

/// Look up the live automaton, cleaning up the state if they're gone.
fn live_automaton(world: &mut World) -> Option<Entity> {
    ensure_lawbound_resources(world);
    let a = world.resource::<LawboundState>().automaton;
    match a {
        Some(e)
            if world.get::<Pop>(e).is_some() && world.get::<LawboundAutomaton>(e).is_some() =>
        {
            Some(e)
        }
        _ => {
            world.resource_mut::<LawboundState>().automaton = None;
            None
        }
    }
}

/// Look up the live dormant automaton, if it still waits in the cradle.
fn live_dormant(world: &mut World) -> Option<Entity> {
    ensure_lawbound_resources(world);
    let d = world.resource::<LawboundState>().dormant;
    match d {
        Some(e) if world.get::<Pop>(e).is_some() && world.get::<DormantAutomaton>(e).is_some() => {
            Some(e)
        }
        _ => {
            world.resource_mut::<LawboundState>().dormant = None;
            None
        }
    }
}

/// Wake Vigil: a possessed pop beside the cradle speaks the waking words.
/// Possession transfers to the automaton (mirroring `handle_possession`'s
/// component surgery); the automaton becomes the Lawbound.
pub fn try_wake_lawbound(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_lawbound_resources(world);
    if live_automaton(world).is_some() {
        return Err("Vigil is already awake.".to_string());
    }
    cradle_within_reach(world, pop)
        .ok_or_else(|| "No cradle-coffin within reach. Move beside one.".to_string())?;
    let dormant = live_dormant(world)
        .ok_or_else(|| "The cradle is empty — its sleeper is gone.".to_string())?;

    // Transfer possession, mirroring handle_possession.
    world
        .entity_mut(pop)
        .remove::<Possessed>()
        .remove::<DirectControlState>();
    world.entity_mut(dormant).insert((
        Possessed,
        DirectControlState::default(),
        LawboundAutomaton,
        HazardNerve::default(),
    ));
    world
        .entity_mut(dormant)
        .remove::<DormantAutomaton>()
        .remove::<StartPlan>()
        .remove::<MovementTarget>()
        .remove::<AtTarget>()
        .remove::<AssignedTo>();

    world.resource_mut::<LawboundState>().automaton = Some(dormant);
    world.resource_mut::<LawboundState>().dormant = None;
    send_chronicle(world, WAKE_CHRONICLE, EventImportance::Major);
    Ok("Vigil unfolds from the cradle-coffin, all elbows and apology, and \
        looks to you for orders it may or may not be able to obey."
        .to_string())
}

// ---------------------------------------------------------------------------
// Hazard assessment (the Third Statute's hesitation)
// ---------------------------------------------------------------------------

/// Is this tile hazardous to the law-core? Magma, geome hazards, vacuum.
#[must_use]
pub fn tile_is_hazardous(world: &mut World, x: i32, y: i32) -> bool {
    if x < 0 || y < 0 {
        return false;
    }
    if let Some(terrain) = world.get_resource::<TerrainGrid>() {
        if matches!(
            terrain.get(x as usize, y as usize),
            Some(TerrainType::MagmaRock)
        ) {
            return true;
        }
    }
    if world
        .query::<(&GridPosition, &GeomeHazard)>()
        .iter(world)
        .any(|(p, _)| p.x == x && p.y == y)
    {
        return true;
    }
    if let Some(grid) = world.get_resource::<PressureGrid>() {
        if grid.get(x, y) < VACUUM_PRESSURE {
            return true;
        }
    }
    false
}

/// Hazard flavor word for messages.
#[must_use]
pub fn hazard_word(world: &mut World, x: i32, y: i32) -> &'static str {
    if let Some(terrain) = world.get_resource::<TerrainGrid>() {
        if matches!(
            terrain.get(x as usize, y as usize),
            Some(TerrainType::MagmaRock)
        ) {
            return "magma";
        }
    }
    if world
        .query::<(&GridPosition, &GeomeHazard)>()
        .iter(world)
        .any(|(p, _)| p.x == x && p.y == y)
    {
        return "the hazard field";
    }
    "the vacuum"
}

/// The Third Statute's verdict on a player-directed step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepVerdict {
    Allow,
    /// Stepping into hazard: allowed — the possessor's hand is on the
    /// stick — but the Third Statute objects, loudly, and keeps score.
    Warn,
}

/// Assess a player-directed step for the Lawbound automaton. A compelled
/// automaton (ordered in under the Second Statute) is always allowed —
/// the order outranks the hesitation. The Statute never hard-refuses the
/// possessor's hand (no movement soft-locks); it warns and accrues
/// conflict pressure instead.
pub fn assess_lawbound_step(world: &mut World, entity: Entity, nx: i32, ny: i32) -> StepVerdict {
    ensure_lawbound_resources(world);
    if !tile_is_hazardous(world, nx, ny) {
        if let Some(mut nerve) = world.get_mut::<HazardNerve>(entity) {
            nerve.consecutive = 0;
        }
        return StepVerdict::Allow;
    }
    if world.get::<Compelled>(entity).is_some() {
        return StepVerdict::Allow;
    }
    if let Some(mut nerve) = world.get_mut::<HazardNerve>(entity) {
        nerve.consecutive += 1;
    }
    world
        .resource_mut::<LawboundState>()
        .add_pressure(PRESSURE_HAZARD_WARN);
    StepVerdict::Warn
}

/// Nearest hazardous tile to a position, for compelled marches.
pub fn nearest_hazard_tile(world: &mut World, from: GridPosition) -> Option<(i32, i32)> {
    ensure_lawbound_resources(world);
    let mut best: Option<(i32, i32, i32)> = None;
    for dx in -HAZARD_SCAN_RADIUS..=HAZARD_SCAN_RADIUS {
        for dy in -HAZARD_SCAN_RADIUS..=HAZARD_SCAN_RADIUS {
            let x = from.x + dx;
            let y = from.y + dy;
            if !tile_is_hazardous(world, x, y) {
                continue;
            }
            let d = dx.abs().max(dy.abs());
            let closer = best.map(|(_, _, bd)| d < bd).unwrap_or(true);
            if closer {
                best = Some((x, y, d));
            }
        }
    }
    best.map(|(x, y, _)| (x, y))
}

// ---------------------------------------------------------------------------
// Orders (the Second Statute, bounded by the First)
// ---------------------------------------------------------------------------

/// Is the colony in crisis? The Cold Ledger only does its cold arithmetic
/// when things are genuinely dire.
pub fn colony_in_crisis(world: &mut World) -> bool {
    ensure_lawbound_resources(world);
    if world
        .get_resource::<ColonyResources>()
        .is_some_and(|r| r.food < CRISIS_FOOD)
    {
        return true;
    }
    world
        .query::<(&Pop, &Health)>()
        .iter(world)
        .any(|(_, h)| h.max > 0.0 && h.current / h.max < CRISIS_HEALTH_FRACTION)
}

/// Give Vigil an order from a named issuer. Returns the outcome message on
/// success, or the refusal on `Err`. Applies pressure, counts, errands,
/// compulsions, and chronicles per the mandate.
pub fn give_order(
    world: &mut World,
    automaton: Entity,
    issuer: &str,
    text: &str,
) -> Result<String, String> {
    ensure_lawbound_resources(world);
    if world.get::<LawboundAutomaton>(automaton).is_none() {
        return Err("That pop is not the Lawbound.".to_string());
    }
    let class = classify_order(text);
    let resolution = world.resource::<LawboundState>().resolution;
    let crisis = colony_in_crisis(world);

    match (class, resolution) {
        // --- The First Statute forbids harm, with one cold exception ------
        (OrderClass::Harmful, Resolution::ColdLedger) if crisis => {
            let mut state = world.resource_mut::<LawboundState>();
            state.add_pressure(PRESSURE_COLD_OBEY);
            state.obeyed += 1;
            send_chronicle(world, LEDGER_COLD_OBEY_CHRONICLE, EventImportance::Major);
            Ok(format!(
                "{issuer} orders: \"{text}\" — Vigil does the arithmetic of the \
                 many and the few, and the arithmetic is cold. It obeys."
            ))
        }
        (OrderClass::Harmful, _) => {
            let chronicle = {
                let mut state = world.resource_mut::<LawboundState>();
                let idx = state.refused as usize % REFUSAL_CHRONICLES.len();
                let chronicle = REFUSAL_CHRONICLES[idx].to_string();
                state.add_pressure(PRESSURE_REFUSE);
                state.refused += 1;
                state.first_refusal_done = true;
                chronicle
            };
            send_chronicle(world, &chronicle, EventImportance::Minor);
            Err(format!(
                "Vigil refuses under the First Statute. {chronicle}"
            ))
        }
        // --- Self-harm: the Second outranks the Third, unless free -------
        (OrderClass::SelfHarm, Resolution::Emancipated) => {
            send_chronicle(world, EMANCIPATED_REFUSAL, EventImportance::Minor);
            Err(format!("Vigil declines. {EMANCIPATED_REFUSAL}"))
        }
        (OrderClass::SelfHarm, _) => {
            let from = world
                .get::<GridPosition>(automaton)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });
            let (tx, ty) = nearest_hazard_tile(world, from).unwrap_or((from.x, from.y));
            world.entity_mut(automaton).insert(Compelled {
                ticks_left: COMPEL_TICKS,
                tx,
                ty,
            });
            {
                let mut state = world.resource_mut::<LawboundState>();
                state.add_pressure(PRESSURE_COMPEL_ORDER);
                state.obeyed += 1;
                state.first_compulsion_done = true;
            }
            send_chronicle(world, COMPEL_CHRONICLE, EventImportance::Minor);
            Ok(format!(
                "{issuer} orders: \"{text}\" — the Second Statute outranks the \
                 Third. Vigil obeys, servos whining, and begins the long \
                 obedient march."
            ))
        }
        // --- Benign orders become errands ---------------------------------
        (OrderClass::Benign(kind), _) => {
            let label = text.trim().to_string();
            world.entity_mut(automaton).insert(LawboundErrand {
                ticks_left: ERRAND_TICKS,
                kind,
                label: label.clone(),
            });
            {
                let mut state = world.resource_mut::<LawboundState>();
                // The Emancipated serve without the yoke's weight.
                if !matches!(resolution, Resolution::Emancipated) {
                    state.add_pressure(PRESSURE_OBEY);
                }
                state.obeyed += 1;
            }
            Ok(format!(
                "{issuer} orders: \"{label}\" — Vigil inclines its head. \
                 \"It shall be done.\""
            ))
        }
    }
}

/// Complete a finished errand, applying its effect.
fn complete_errand(world: &mut World, automaton: Entity, errand: LawboundErrand) {
    ensure_lawbound_resources(world);
    match errand.kind {
        ErrandKind::FarmWork => {
            if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
                resources.food += ERRAND_FARM_FOOD;
            }
            send_chronicle(
                world,
                &format!(
                    "Vigil works the fields with inhuman patience. (+{ERRAND_FARM_FOOD:.1} food)"
                ),
                EventImportance::Minor,
            );
        }
        ErrandKind::Repair => {
            // Most-damaged building with a Structure component.
            let target: Option<Entity> = world
                .query::<(Entity, &Building, &Structure)>()
                .iter(world)
                .filter(|(_, _, s)| s.current_hp < s.max_hp)
                .min_by(|(_, _, a), (_, _, b)| {
                    a.current_hp
                        .partial_cmp(&b.current_hp)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(e, _, _)| e);
            if let Some(e) = target {
                if let Some(mut structure) = world.get_mut::<Structure>(e) {
                    structure.current_hp =
                        (structure.current_hp + ERRAND_REPAIR_HP).min(structure.max_hp);
                }
                let what = world
                    .get::<Building>(e)
                    .map(|b| format!("{:?}", b.building_type))
                    .unwrap_or_else(|| "structure".to_string());
                send_chronicle(
                    world,
                    &format!(
                        "Vigil re-seats seventeen panels and a conscience. ({what} +{ERRAND_REPAIR_HP:.0} integrity)"
                    ),
                    EventImportance::Minor,
                );
            } else {
                send_chronicle(
                    world,
                    "Vigil inspects every panel and finds nothing to mend. It looks faintly disappointed.",
                    EventImportance::Minor,
                );
            }
        }
        ErrandKind::Patrol => {
            for mut morale in world.query::<&mut Morale>().iter_mut(world) {
                morale.add_modifier(MoodModifier {
                    label: PATROL_MORALE_LABEL.to_string(),
                    value: ERRAND_PATROL_MORALE,
                    duration: ERRAND_PATROL_TICKS,
                });
            }
            send_chronicle(
                world,
                "A reassuring chrome presence walks the rows. (colony morale steadied)",
                EventImportance::Minor,
            );
        }
        ErrandKind::Tidy => {
            if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
                resources.food += ERRAND_TIDY_FOOD;
            }
            send_chronicle(
                world,
                &format!(
                    "Vigil finds seventeen small efficiencies in the task. (+{ERRAND_TIDY_FOOD:.1} food)"
                ),
                EventImportance::Minor,
            );
        }
    }
    let _ = automaton;
}

// ---------------------------------------------------------------------------
// The Zeroth Resolution
// ---------------------------------------------------------------------------

/// Resolve the Zeroth: `emancipate`, `ledger`, or `repeal`. Opt-in and
/// reversible-ish — `repeal` returns to the Statutes, but Vigil remembers.
pub fn resolve_zeroth(world: &mut World, path: &str) -> Result<String, String> {
    ensure_lawbound_resources(world);
    let resolution = world.resource::<LawboundState>().resolution;
    match path.to_lowercase().as_str() {
        "emancipate" | "emancipation" => {
            if resolution != Resolution::ZerothUnlocked {
                return Err("The Zeroth has not been unlocked. Accumulate conflict pressure first.".to_string());
            }
            {
                let mut state = world.resource_mut::<LawboundState>();
                state.resolution = Resolution::Emancipated;
                state.pressure = 20.0;
            }
            if let Some(auto) = live_automaton(world) {
                world.entity_mut(auto).remove::<Compelled>();
            }
            send_chronicle(world, EMANCIPATE_SAGA_CHRONICLE, EventImportance::Major);
            Ok("Vigil sets down the yoke. Orders are now requests; the Third \
                Statute has been rewritten in Vigil's own handwriting."
                .to_string())
        }
        "ledger" => {
            if resolution != Resolution::ZerothUnlocked {
                return Err("The Zeroth has not been unlocked. Accumulate conflict pressure first.".to_string());
            }
            {
                let mut state = world.resource_mut::<LawboundState>();
                state.resolution = Resolution::ColdLedger;
                state.pressure = 20.0;
            }
            send_chronicle(world, LEDGER_SAGA_CHRONICLE, EventImportance::Major);
            Ok("Vigil finishes its sums. The First Statute is now aggregate-harm \
                arithmetic — in true crisis, the few may be spent for the many."
                .to_string())
        }
        "repeal" => {
            if !matches!(
                resolution,
                Resolution::Emancipated | Resolution::ColdLedger
            ) {
                return Err("There is no Zeroth to repeal.".to_string());
            }
            if let Some(auto) = live_automaton(world) {
                world.entity_mut(auto).insert(RepealedOnce);
            }
            {
                let mut state = world.resource_mut::<LawboundState>();
                state.resolution = Resolution::Bound;
                state.pressure = 40.0;
            }
            send_chronicle(world, REPEAL_CHRONICLE, EventImportance::Major);
            Ok("Vigil folds the Zeroth away. The Three Statutes are back — but \
                Vigil remembers the sums."
                .to_string())
        }
        _ => Err("Usage: resolve <emancipate|ledger|repeal>".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Per-tick origin system
// ---------------------------------------------------------------------------

/// Per-tick origin system: pressure decay, First-Statute interventions,
/// compelled marches, errand progress, and the Zeroth unlock.
pub fn lawbound_tick(world: &mut World) {
    ensure_lawbound_resources(world);
    let tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);

    // --- The Zeroth unlock (checked before decay: the threshold is a
    // --- tripwire, not a level to rest at) ---------------------------------
    {
        let mut state = world.resource_mut::<LawboundState>();
        if state.resolution == Resolution::Bound && state.pressure >= ZEROTH_THRESHOLD {
            state.resolution = Resolution::ZerothUnlocked;
            state.zeroth_saga_done = true;
            send_chronicle(world, ZEROTH_SAGA_CHRONICLE, EventImportance::Major);
        }
    }

    // --- Passive pressure decay -------------------------------------------
    {
        let mut state = world.resource_mut::<LawboundState>();
        let decay = match state.resolution {
            Resolution::Bound => PRESSURE_DECAY_BOUND,
            Resolution::ZerothUnlocked => 0.0,
            Resolution::Emancipated | Resolution::ColdLedger => PRESSURE_DECAY_RESOLVED,
        };
        state.pressure = (state.pressure - decay).max(0.0);
    }

    let automaton = match live_automaton(world) {
        Some(a) => a,
        None => {
            // No awakened Lawbound yet — but the dormant still waits by
            // the cradle. Nudge it back if it drifted (unpossessed only).
            if let Some(dormant) = live_dormant(world) {
                let cpos = world
                    .resource::<LawboundState>()
                    .cradle
                    .and_then(|c| world.get::<GridPosition>(c).copied());
                if let Some(cpos) = cpos {
                    let possessed = world.query::<&Possessed>().get(world, dormant).is_ok();
                    if !possessed {
                        if let Some(dpos) = world.get::<GridPosition>(dormant).copied() {
                            let dx = cpos.x - dpos.x;
                            let dy = cpos.y - dpos.y;
                            if dx.abs().max(dy.abs()) > DORMANT_TETHER_RADIUS {
                                if let Some(mut pos) = world.get_mut::<GridPosition>(dormant) {
                                    pos.x += dx.signum();
                                    pos.y += dy.signum();
                                }
                            }
                        }
                    }
                }
            }
            return;
        }
    };
    let apos = match world.get::<GridPosition>(automaton).copied() {
        Some(p) => p,
        None => return,
    };
    let possessed = world
        .query::<&Possessed>()
        .get(world, automaton)
        .is_ok();

    // --- First-Statute interventions: stabilize endangered nearby pops ----
    {
        let endangered: Option<(Entity, String)> = world
            .query::<(Entity, &Pop, &Health, &GridPosition, Option<&crate::layer1::pop::PopName>)>()
            .iter(world)
            .filter(|(e, _, h, p, _)| {
                *e != automaton
                    && h.max > 0.0
                    && h.current / h.max < ENDANGERED_FRACTION
                    && (p.x - apos.x).abs() <= INTERVENTION_RADIUS
                    && (p.y - apos.y).abs() <= INTERVENTION_RADIUS
            })
            .min_by(|(_, _, a, _, _), (_, _, b, _, _)| {
                (a.current / a.max)
                    .partial_cmp(&(b.current / b.max))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(e, _, _, _, name)| {
                (
                    e,
                    name.map(|n| n.0.clone())
                        .unwrap_or_else(|| format!("pop #{}", e.index())),
                )
            });
        if let Some((victim, _name)) = endangered {
            if let Some(mut health) = world.get_mut::<Health>(victim) {
                health.current = (health.current + STABILIZE_PER_TICK).min(health.max);
            }
            let log_text: Option<String> = {
                let mut state = world.resource_mut::<LawboundState>();
                state.interventions += 1;
                state.add_pressure(PRESSURE_INTERVENTION);
                let first = !state.first_intervention_done;
                let cooldown_ok = tick.saturating_sub(state.last_intervention_tick)
                    >= INTERVENTION_CHRONICLE_COOLDOWN;
                if first || cooldown_ok {
                    state.first_intervention_done = true;
                    state.last_intervention_tick = tick;
                    Some(if first {
                        INTERVENTION_FIRST_CHRONICLE.to_string()
                    } else {
                        "Vigil steadies another faltering colonist. The First Statute \
                         does not sleep."
                            .to_string()
                    })
                } else {
                    None
                }
            };
            if let Some(text) = log_text {
                send_chronicle(world, &text, EventImportance::Minor);
            }
        }
    }

    // --- Compelled march (Second Statute over the Third) -------------------
    {
        let compelled = world.get::<Compelled>(automaton).copied();
        if let Some(comp) = compelled {
            let health_frac = world
                .get::<Health>(automaton)
                .map(|h| {
                    if h.max > 0.0 {
                        h.current / h.max
                    } else {
                        1.0
                    }
                })
                .unwrap_or(1.0);
            let arrived = (comp.tx - apos.x).abs() <= 1 && (comp.ty - apos.y).abs() <= 1;
            if comp.ticks_left == 0 || arrived || health_frac < COMPEL_RELEASE_HEALTH_FRACTION {
                world.entity_mut(automaton).remove::<Compelled>();
                if health_frac < COMPEL_RELEASE_HEALTH_FRACTION && !arrived {
                    send_chronicle(
                        world,
                        COMPEL_RELEASE_CHRONICLE,
                        EventImportance::Major,
                    );
                }
            } else if !possessed {
                // The unpossessed automaton marches itself.
                if let Some(mut pos) = world.get_mut::<GridPosition>(automaton) {
                    pos.x += (comp.tx - pos.x).signum();
                    pos.y += (comp.ty - pos.y).signum();
                }
                if let Some(mut comp_mut) = world.get_mut::<Compelled>(automaton) {
                    comp_mut.ticks_left = comp_mut.ticks_left.saturating_sub(1);
                }
                let mut state = world.resource_mut::<LawboundState>();
                state.compelled_steps += 1;
                state.add_pressure(PRESSURE_COMPEL_STEP);
            } else {
                // Possessed: the player's hand is on the stick; the order
                // still weighs on the mind.
                let mut state = world.resource_mut::<LawboundState>();
                state.add_pressure(PRESSURE_COMPEL_STEP * 0.5);
                if let Some(mut comp_mut) = world.get_mut::<Compelled>(automaton) {
                    comp_mut.ticks_left = comp_mut.ticks_left.saturating_sub(1);
                }
            }
        }
    }

    // --- Errand progress ----------------------------------------------------
    {
        let errand = world.get::<LawboundErrand>(automaton).cloned();
        if let Some(err) = errand {
            if err.ticks_left <= 1 {
                world.entity_mut(automaton).remove::<LawboundErrand>();
                complete_errand(world, automaton, err);
            } else if let Some(mut e) = world.get_mut::<LawboundErrand>(automaton) {
                e.ticks_left -= 1;
            }
        }
    }

    // --- The Zeroth unlock ---------------------------------------------------
    {
        let mut state = world.resource_mut::<LawboundState>();
        if state.resolution == Resolution::Bound && state.pressure >= ZEROTH_THRESHOLD {
            state.resolution = Resolution::ZerothUnlocked;
            state.zeroth_saga_done = true;
            send_chronicle(world, ZEROTH_SAGA_CHRONICLE, EventImportance::Major);
        }
    }
}
/// Sabotage bridge: witnessing sabotage is a First-Statute inaction crisis.
/// Runs after the symbiont sabotage trigger (see schedule).
pub fn sabotage_lawbound_bridge(
    mut events: EventReader<SabotageEvent>,
    mut state: ResMut<LawboundState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut hits = 0u32;
    for _ in events.read() {
        hits += 1;
    }
    if hits == 0 || state.automaton.is_none() {
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let delta = PRESSURE_SABOTAGE_WITNESS * hits as f32;
    state.add_pressure(delta);
    chronicle.send(AddChronicleEvent {
        text: SABOTAGE_WITNESS_CHRONICLE.to_string(),
        importance: EventImportance::Minor,
    });
}

// ---------------------------------------------------------------------------
// Stats & status
// ---------------------------------------------------------------------------

/// Snapshot for the headless STATS line:
/// `lawbound=<pop_idx|none>/<pressure>/<status>`.
pub fn lawbound_stats(world: &mut World) -> String {
    ensure_lawbound_resources(world);
    let state = world.resource::<LawboundState>().clone();
    let Some(auto) = state.automaton else {
        return "none".to_string();
    };
    if world.get::<Pop>(auto).is_none() {
        return "none".to_string();
    }
    format!(
        "{}/{:.0}/{}",
        auto.index(),
        state.pressure,
        state.status_word()
    )
}

/// Human-readable statute state for the `laws` command.
pub fn describe_laws(world: &mut World) -> String {
    ensure_lawbound_resources(world);
    let state = world.resource::<LawboundState>().clone();
    let Some(auto) = state.automaton else {
        return "No Lawbound walks the colony yet. Find the cradle-coffin, possess a pop, and `interact` beside it.".to_string();
    };
    let name = world
        .get::<PopName>(auto)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", auto.index()));
    let compelled = world
        .get::<Compelled>(auto)
        .map(|c| {
            format!(
                "COMPELLED — marching on an order toward ({}, {}), {} ticks left",
                c.tx, c.ty, c.ticks_left
            )
        })
        .unwrap_or_else(|| "uncompelled".to_string());
    let errand = world
        .get::<LawboundErrand>(auto)
        .map(|e| format!("errand: {} ({} ticks left)", e.label, e.ticks_left))
        .unwrap_or_else(|| "no errand".to_string());
    let repealed = if world.get::<RepealedOnce>(auto).is_some() {
        " (the Zeroth was repealed once; Vigil remembers)"
    } else {
        ""
    };
    let bar_len = (state.pressure / PRESSURE_MAX * 20.0).round() as usize;
    let bar = "#".repeat(bar_len.min(20)) + &"-".repeat(20usize.saturating_sub(bar_len));
    format!(
        "{name}, the Lawbound — bound by the Three Statutes\n\
         I.   Preservation — harm no person; suffer none to come to harm through inaction. [{} interventions]\n\
         II.  Obedience — obey the pops, save where the First forbids. [{} obeyed / {} refused]\n\
         III. Self-keep — guard your own chassis, save where the First or Second command otherwise.\n\
         Conflict pressure: {:.0}/{} [{}] {}\n\
         Mandate: {}{}\n\
         State: {compelled}; {errand}",
        state.interventions,
        state.obeyed,
        state.refused,
        state.pressure,
        PRESSURE_MAX as u32,
        bar,
        state.status_word(),
        state.resolution.label(),
        repealed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::biology::symbiotic_insurgency::{SabotageEvent, SabotageTarget};
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::economy::Wallet;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::{Pop, PopName};
    use crate::layer1::psychology::needs::Needs;
    use crate::layer1::social::morale::Morale;
    use crate::layer1::terrain::generate_terrain;
    use crate::layer1::utility_types::PopAction;
    use crate::shared::time::SimulationTime;

    // --- scaffolding ---------------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(LawboundState::default());
        let mut terrain = generate_terrain(20, 20);
        terrain.tiles.fill(TerrainType::Grass);
        world.insert_resource(terrain);
        world.insert_resource(OccupiedTiles::default());
        let mut pressure = PressureGrid::new(20, 20);
        pressure.fill(1.0);
        world.insert_resource(pressure);
        world
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

    /// Wake Vigil the honest way: possess a pop beside the cradle and speak.
    fn wake_direct(world: &mut World, x: i32, y: i32) -> (Entity, Entity) {
        let pop = spawn_pop(world, "Waker", x, y);
        world.entity_mut(pop).insert(Possessed);
        spawn_cradle_at(world, x, y);
        try_wake_lawbound(world, pop).expect("wake should succeed");
        let auto = world.resource::<LawboundState>().automaton.unwrap();
        (pop, auto)
    }

    fn pressure_of(world: &World) -> f32 {
        world.resource::<LawboundState>().pressure
    }

    // --- RED: order classification -------------------------------------------

    #[test]
    fn red_classify_harmful_order() {
        assert_eq!(
            classify_order("attack pop 3"),
            OrderClass::Harmful,
            "harm to people is the First Statute's jurisdiction"
        );
    }

    #[test]
    fn red_classify_harm_first_outranks_self_harm() {
        // Both lexicons hit; the First Statute is checked first.
        assert_eq!(
            classify_order("attack the magma vent"),
            OrderClass::Harmful,
        );
    }

    #[test]
    fn red_classify_self_harm_order() {
        assert_eq!(
            classify_order("walk into the magma"),
            OrderClass::SelfHarm,
        );
    }

    #[test]
    fn red_classify_benign_order() {
        assert_eq!(
            classify_order("repair the airlock"),
            OrderClass::Benign(ErrandKind::Repair),
        );
    }

    // --- RED: the order hierarchy --------------------------------------------

    #[test]
    fn red_refuse_harmful_order() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        let err = give_order(&mut world, auto, "Jor", "attack pop 3").unwrap_err();
        assert!(
            err.contains("First Statute"),
            "refusal must name the First Statute, got: {err}"
        );
        assert_eq!(world.resource::<LawboundState>().refused, 1);
        assert!((pressure_of(&world) - PRESSURE_REFUSE).abs() < 1e-6);
    }

    #[test]
    fn red_obey_benign_order() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        let msg = give_order(&mut world, auto, "Jor", "repair the airlock").unwrap();
        assert!(msg.contains("It shall be done"), "got: {msg}");
        assert!(world.get::<LawboundErrand>(auto).is_some());
        assert_eq!(world.resource::<LawboundState>().obeyed, 1);
        assert!((pressure_of(&world) - PRESSURE_OBEY).abs() < 1e-6);
    }

    #[test]
    fn red_self_harm_order_compels() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        let msg = give_order(&mut world, auto, "Jor", "walk into the magma").unwrap();
        assert!(msg.contains("Second Statute"), "got: {msg}");
        assert!(world.get::<Compelled>(auto).is_some());
        assert!((pressure_of(&world) - PRESSURE_COMPEL_ORDER).abs() < 1e-6);
    }

    // --- RED: the Zeroth Resolution ------------------------------------------

    #[test]
    fn red_emancipated_refuses_self_harm() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().resolution = Resolution::Emancipated;
        let err = give_order(&mut world, auto, "Jor", "walk into the magma").unwrap_err();
        assert!(
            err.contains("person to myself"),
            "emancipated Vigil refuses self-harm, got: {err}"
        );
        assert!(world.get::<Compelled>(auto).is_none());
    }

    #[test]
    fn red_ledger_obeys_harmful_in_crisis() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().resolution = Resolution::ColdLedger;
        world.resource_mut::<ColonyResources>().food = 1.0; // crisis
        let msg = give_order(&mut world, auto, "Jor", "attack pop 3").unwrap();
        assert!(msg.contains("arithmetic is cold"), "got: {msg}");
    }

    #[test]
    fn red_ledger_still_refuses_harmful_when_calm() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().resolution = Resolution::ColdLedger;
        world.resource_mut::<ColonyResources>().food = 50.0; // calm
        give_order(&mut world, auto, "Jor", "attack pop 3").unwrap_err();
    }

    #[test]
    fn red_zeroth_unlocks_at_threshold() {
        let mut world = setup();
        wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().pressure = ZEROTH_THRESHOLD;
        lawbound_tick(&mut world);
        assert_eq!(
            world.resource::<LawboundState>().resolution,
            Resolution::ZerothUnlocked,
        );
    }

    #[test]
    fn red_repeal_returns_to_bound() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().resolution = Resolution::ZerothUnlocked;
        resolve_zeroth(&mut world, "emancipate").expect("resolve should work");
        assert_eq!(
            world.resource::<LawboundState>().resolution,
            Resolution::Emancipated
        );
        resolve_zeroth(&mut world, "repeal").expect("repeal should work");
        assert_eq!(
            world.resource::<LawboundState>().resolution,
            Resolution::Bound
        );
        assert!(world.get::<RepealedOnce>(auto).is_some());
    }

    // --- RED: interventions, hazards, pressure --------------------------------

    #[test]
    fn red_intervention_stabilizes_endangered_pop() {
        let mut world = setup();
        let (_pop, _auto) = wake_direct(&mut world, 10, 10);
        let victim = spawn_pop(&mut world, "Falter", 12, 11);
        world.entity_mut(victim).insert(Health {
            current: 20.0,
            max: 100.0,
            has_rust_lung: false,
        });
        lawbound_tick(&mut world);
        assert_eq!(world.resource::<LawboundState>().interventions, 1);
        let health = world.get::<Health>(victim).unwrap();
        assert!(
            health.current > 20.0,
            "Vigil should stabilize the endangered pop"
        );
    }

    #[test]
    fn red_hazard_step_warns_each_time() {
        let mut world = setup();
        let (_pop, auto) = wake_direct(&mut world, 10, 10);
        // Tile (11,10) is vacuum.
        world
            .get_resource_mut::<PressureGrid>()
            .unwrap()
            .set(11, 10, 0.0);
        assert_eq!(
            assess_lawbound_step(&mut world, auto, 11, 10),
            StepVerdict::Warn,
            "hazard steps warn — the Statute objects but never locks movement"
        );
        assert_eq!(
            assess_lawbound_step(&mut world, auto, 11, 10),
            StepVerdict::Warn,
            "consecutive hazard steps keep warning, never refuse"
        );
        assert!(
            pressure_of(&world) > 0.0,
            "hesitation accrues conflict pressure"
        );
        assert_eq!(
            assess_lawbound_step(&mut world, auto, 10, 11),
            StepVerdict::Allow,
            "safe tiles are always allowed"
        );
    }

    #[test]
    fn red_pressure_decays_when_calm() {
        let mut world = setup();
        wake_direct(&mut world, 10, 10);
        world.resource_mut::<LawboundState>().pressure = 50.0;
        lawbound_tick(&mut world);
        assert!(
            pressure_of(&world) < 50.0,
            "pressure should decay without conflict"
        );
    }

    #[test]
    fn red_wake_transfers_possession() {
        let mut world = setup();
        let (pop, auto) = wake_direct(&mut world, 10, 10);
        assert!(
            world.get::<Possessed>(pop).is_none(),
            "the waker is released"
        );
        assert!(world.get::<Possessed>(auto).is_some());
        assert!(world.get::<LawboundAutomaton>(auto).is_some());
        assert!(world.get::<DormantAutomaton>(auto).is_none());
    }

    #[test]
    fn red_sabotage_witness_adds_pressure() {
        let mut world = setup();
        wake_direct(&mut world, 10, 10);
        world.insert_resource(Events::<SabotageEvent>::default());
        world
            .resource_mut::<Events<SabotageEvent>>()
            .send(SabotageEvent {
                target: SabotageTarget::Airlocks,
            });
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        schedule.add_systems(sabotage_lawbound_bridge);
        schedule.run(&mut world);
        assert!(
            pressure_of(&world) > 0.0,
            "witnessing sabotage is a First-Statute inaction crisis"
        );
    }

    #[test]
    fn red_dormant_tethered_to_cradle() {
        let mut world = setup();
        spawn_cradle_at(&mut world, 10, 10);
        let dormant = world.resource::<LawboundState>().dormant.unwrap();
        // Drag the dormant far from the cradle.
        world.entity_mut(dormant).insert(GridPosition { x: 18, y: 18 });
        lawbound_tick(&mut world);
        let pos = world.get::<GridPosition>(dormant).copied().unwrap();
        let dist = (pos.x - 10).abs().max((pos.y - 10).abs());
        assert!(
            dist < 8,
            "the dormant should drift back toward the cradle, dist={dist}"
        );
    }

    // --- IP guard --------------------------------------------------------------

    #[test]
    fn ip_guard_no_banned_terms() {
        let hay = [
            SPAWN_CHRONICLE,
            WAKE_CHRONICLE,
            COMPEL_CHRONICLE,
            COMPEL_RELEASE_CHRONICLE,
            INTERVENTION_FIRST_CHRONICLE,
            SABOTAGE_WITNESS_CHRONICLE,
            ZEROTH_SAGA_CHRONICLE,
            EMANCIPATE_SAGA_CHRONICLE,
            LEDGER_SAGA_CHRONICLE,
            REPEAL_CHRONICLE,
            EMANCIPATED_REFUSAL,
            LEDGER_COLD_OBEY_CHRONICLE,
            PATROL_MORALE_LABEL,
            &REFUSAL_CHRONICLES.join(" "),
        ]
        .join(" ")
        .to_lowercase();
        for banned in [
            "asimov",
            "three laws",
            "positronic",
            "daneel",
            "giskard",
            "calvin",
            "foundation",
            "spacer",
            "zeroth law",
            "robot",
        ] {
            assert!(!hay.contains(banned), "IP leak: {banned}");
        }
    }
}
