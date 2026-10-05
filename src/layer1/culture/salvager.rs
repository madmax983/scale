//! The Salvager — a wreck-diver adventurer origin.
//!
//! Layers on top of the adventurer-mode spike (`direct_link`): a derelict
//! hulk drifts near the starter colony, her reactor still warm, her holds
//! still sealed. Aboard her waits a lone salvager pop — possess them and
//! the headless console gains `survey` (reveal wreck systems + an appraisal
//! roll of relics vs. scrap vs. deadweight), `strip <system>` (convert
//! systems to scrap over time), `patch` (seal hull breaches, costs scrap),
//! `claim` (plant your beacon, start tow-prep), `tow` (drag her home for
//! the colony — costs fuel, and claim-jumpers may intercept), and `scuttle`
//! (sell the coordinates for a quick personal payout).
//!
//! Stripping feeds an instability meter in the dead reactor: it creeps up
//! every tick, faster while you strip, fastest while you strip the reactor
//! itself. At 100 the reactor lets go — no yield from it, injuries all
//! round. Hull breaches are vacuum tiles until patched; the sealed cabin
//! and patched tiles hold pressure.
//!
//! All names are original. The "wreck-diver" concept is concept-only
//! inspiration — no named characters, places, ships, or distinctive IP
//! anywhere. Mechanics and vibes only, under original names.

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::direct_link::Possessed;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::economy::Wallet;
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::pressure::PressureGrid;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::terrain::{TerrainGrid, TerrainType};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn ring (Chebyshev) around the lander where the hulk drifts.
pub const HULK_RING_MIN: i32 = 6;
pub const HULK_RING_MAX: i32 = 10;

/// Scrap band width above a system's relic bias in the appraisal roll.
pub const APPRAISAL_SCRAP_BAND: f32 = 0.50;

/// Instability gained per tick from the dying reactor alone.
pub const INSTABILITY_BASE_PER_TICK: f32 = 0.08;
/// Extra instability per tick while stripping a non-reactor system.
pub const INSTABILITY_STRIP_PER_TICK: f32 = 0.30;
/// Extra instability per tick while stripping the reactor itself.
pub const INSTABILITY_REACTOR_STRIP_PER_TICK: f32 = 0.90;
/// Instability creep per tick once the reactor is gone (stripped/breached).
pub const INSTABILITY_SPENT_PER_TICK: f32 = 0.02;
/// Instability blows the reactor at this value.
pub const INSTABILITY_MAX: f32 = 100.0;
/// First warning threshold ("running hot").
pub const INSTABILITY_WARN: f32 = 50.0;
/// Second warning threshold ("critical").
pub const INSTABILITY_CRITICAL: f32 = 75.0;
/// Instability after a reactor breach (the danger is spent, not gone).
pub const INSTABILITY_POST_BREACH: f32 = 35.0;
/// Health damage to pops near the hulk when the reactor lets go.
pub const REACTOR_BREACH_DAMAGE: f32 = 30.0;
/// Chebyshev radius of the reactor-breach blast.
pub const REACTOR_BREACH_RADIUS: i32 = 3;

/// Hull-breach tile offsets around the hulk (candidates; only walkable
/// non-water/void tiles are used, in order).
pub const BREACH_OFFSET_CANDIDATES: [(i32, i32); 8] = [
    (2, 1),
    (-2, -1),
    (1, 2),
    (-1, -2),
    (2, -1),
    (-2, 1),
    (1, -2),
    (-1, 2),
];
/// Scrap cost to patch one breach.
pub const PATCH_SCRAP_COST: f32 = 1.0;

/// Food in the hulk's sealed ration cache at spawn.
pub const RATION_CACHE_FOOD: f32 = 12.0;
/// Hold food eaten per tick by the ration cache when the salvager is hungry.
pub const MESS_MEAL_PER_TICK: f32 = 0.02;
/// Hunger restored per tick by the ration cache.
pub const MESS_SATIETY: f32 = 0.2;
/// Below this hunger the cache feeds the salvager.
pub const MESS_HUNGER_GATE: f32 = 0.95;

/// Ticks of tow-prep after the beacon is planted before `tow` unlocks.
pub const CLAIM_PREP_TICKS: u32 = 50;
/// Ticks a tow takes once underway.
pub const TOW_TICKS: u32 = 120;
/// Colony fuel burned to drag the hulk home ("engine burns").
pub const TOW_FUEL_COST: f32 = 15.0;
/// Per-tick claim-jumper interception probability while towing.
pub const INTERCEPT_P_PER_TICK: f32 = 0.02;
/// Fraction of the hold's scrap stolen per interception.
pub const INTERCEPT_SCRAP_THEFT: f32 = 0.15;

/// Credit value of one scrap unit on the salvage ledger.
pub const SCRAP_CREDIT_VALUE: f32 = 2.0;
/// Credit value of one relic on the salvage ledger.
pub const RELIC_CREDIT_VALUE: f32 = 50.0;
/// Fraction of total appraised value a scuttle buyer pays out.
pub const SCUTTLE_PAYOUT_FRACTION: f32 = 0.40;

/// How far (Chebyshev) the salvager may wander from the hulk before the
/// tick nudges them back (only while unpossessed).
pub const SALVAGER_TETHER_RADIUS: i32 = 2;
/// How close (Chebyshev) the salvager must be to the hulk to survey/strip.
pub const WRECK_WORK_REACH: i32 = 2;
/// How close (Chebyshev) a pop must be to the hulk to take up the spike.
pub const SPIKE_TAKE_REACH: i32 = 1;

/// Chronicle text when the hulk drifts into sensor range.
pub const HULK_RUMOR_CHRONICLE: &str =
    "Rumor: a dead hulk drifts nearby — her reactor still warm, her holds still sealed.";
/// Chronicle text when the reactor lets go.
pub const REACTOR_BREACH_CHRONICLE: &str =
    "The derelict's reactor lets go in a white flash. The hulk shudders — and goes truly dark.";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The wreck's strippable systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WreckSystemKind {
    Reactor,
    LifeSupport,
    Engine,
    CargoBay,
    CommsArray,
    SensorDome,
    ThrusterBank,
}

impl WreckSystemKind {
    /// Every system a derelict hulk carries.
    pub fn all() -> [WreckSystemKind; 7] {
        [
            WreckSystemKind::Reactor,
            WreckSystemKind::LifeSupport,
            WreckSystemKind::Engine,
            WreckSystemKind::CargoBay,
            WreckSystemKind::CommsArray,
            WreckSystemKind::SensorDome,
            WreckSystemKind::ThrusterBank,
        ]
    }

    /// Parse a headless `strip` argument.
    pub fn parse(s: &str) -> Option<WreckSystemKind> {
        match s.to_lowercase().as_str() {
            "reactor" => Some(WreckSystemKind::Reactor),
            "life" | "lifesupport" | "life-support" => Some(WreckSystemKind::LifeSupport),
            "engine" => Some(WreckSystemKind::Engine),
            "cargo" | "cargobay" | "cargo-bay" => Some(WreckSystemKind::CargoBay),
            "comms" | "commsarray" | "comms-array" => Some(WreckSystemKind::CommsArray),
            "sensor" | "sensors" | "sensordome" | "sensor-dome" => {
                Some(WreckSystemKind::SensorDome)
            }
            "thruster" | "thrusters" | "thrusterbank" | "thruster-bank" => {
                Some(WreckSystemKind::ThrusterBank)
            }
            _ => None,
        }
    }

    /// Flavor name for messages and the survey manifest.
    pub fn name(&self) -> &'static str {
        match self {
            WreckSystemKind::Reactor => "reactor",
            WreckSystemKind::LifeSupport => "life-support",
            WreckSystemKind::Engine => "main engine",
            WreckSystemKind::CargoBay => "cargo bay",
            WreckSystemKind::CommsArray => "comms array",
            WreckSystemKind::SensorDome => "sensor dome",
            WreckSystemKind::ThrusterBank => "thruster bank",
        }
    }

    /// Relic probability on the appraisal roll: some systems are likelier
    /// to hold something worth more than scrap.
    pub fn relic_bias(&self) -> f32 {
        match self {
            WreckSystemKind::Reactor => 0.25,
            WreckSystemKind::Engine => 0.20,
            WreckSystemKind::CommsArray => 0.20,
            WreckSystemKind::SensorDome => 0.18,
            WreckSystemKind::CargoBay => 0.15,
            WreckSystemKind::LifeSupport => 0.12,
            WreckSystemKind::ThrusterBank => 0.12,
        }
    }
}

/// What the appraisal roll says a system is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppraisalClass {
    /// Rare, valuable: yields relics.
    Relic,
    /// Bulk scrap.
    Scrap,
    /// Worthless junk: barely worth the cutting torch.
    Deadweight,
}

impl AppraisalClass {
    pub fn label(&self) -> &'static str {
        match self {
            AppraisalClass::Relic => "relic",
            AppraisalClass::Scrap => "scrap",
            AppraisalClass::Deadweight => "deadweight",
        }
    }
}

/// Claim/payoff state of the wreck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WreckStatus {
    /// Drifting, unclaimed — survey and strip freely.
    #[default]
    Unclaimed,
    /// Beacon planted; tow-prep counting down.
    Claimed,
    /// Under tow to the colony.
    Towing,
    /// Delivered to the colony.
    Towed,
    /// Coordinates sold; the wreck is gone.
    Scuttled,
    /// Reactor breach tore her apart beyond recovery.
    Destroyed,
}

impl WreckStatus {
    pub fn label(&self) -> &'static str {
        match self {
            WreckStatus::Unclaimed => "unclaimed",
            WreckStatus::Claimed => "claimed",
            WreckStatus::Towing => "towing",
            WreckStatus::Towed => "towed",
            WreckStatus::Scuttled => "scuttled",
            WreckStatus::Destroyed => "destroyed",
        }
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// The derelict hulk: a dead vessel parked near the colony.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct DerelictHulk;

/// One strippable system aboard the hulk.
#[derive(Component, Debug, Clone, Copy)]
pub struct WreckSystem {
    /// Which system this is.
    pub kind: WreckSystemKind,
    /// The hulk it belongs to.
    pub hulk: Entity,
    /// Appraisal roll result (None until surveyed).
    pub appraisal: Option<AppraisalClass>,
    /// Scrap yielded when stripping completes.
    pub scrap_yield: f32,
    /// Relics yielded when stripping completes.
    pub relic_yield: u32,
    /// True once stripped (or destroyed by a reactor breach).
    pub stripped: bool,
    /// Ticks of stripping applied so far.
    pub strip_progress: f32,
}

/// A hull-breach vacuum tile aboard the hulk.
#[derive(Component, Debug, Clone, Copy)]
pub struct HulkBreach {
    /// The hulk it belongs to.
    pub hulk: Entity,
    /// True once patched (holds pressure again).
    pub patched: bool,
}

/// The hulk's stockpile: ration cache, stripped scrap, recovered relics.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SalvageHold {
    /// Food in the sealed ration cache.
    pub food: f32,
    /// Stripped scrap awaiting delivery or sale.
    pub scrap: f32,
    /// Recovered relics awaiting delivery or sale.
    pub relics: u32,
}

/// The dying reactor's instability meter (0.0..=[`INSTABILITY_MAX`]).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct HulkInstability {
    /// Current instability.
    pub value: f32,
}

/// The wreck-diver origin pop.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Salvager;

// ---------------------------------------------------------------------------
// State resource
// ---------------------------------------------------------------------------

/// Machine state for the Salvager origin.
#[derive(Resource, Debug, Clone)]
pub struct SalvagerState {
    /// Whether the hulk spawn has been attempted (once per colony).
    pub hulk_spawn_attempted: bool,
    /// The hulk entity, if spawned and not yet delivered/sold.
    pub hulk: Option<Entity>,
    /// The salvager origin pop, if alive.
    pub salvager: Option<Entity>,
    /// Whether the wreck has been surveyed (appraisals rolled).
    pub surveyed: bool,
    /// Claim/payoff state.
    pub status: WreckStatus,
    /// Tow-prep ticks remaining (while Claimed).
    pub claim_prep_left: u32,
    /// Tow ticks remaining (while Towing).
    pub tow_ticks_left: u32,
    /// The system currently being stripped, if any.
    pub stripping: Option<WreckSystemKind>,
    /// Claim-jumper interceptions suffered this run.
    pub intercepts: u32,
    /// Instability warnings already fired (so they fire once per climb).
    pub warned_50: bool,
    pub warned_75: bool,
}

impl Default for SalvagerState {
    fn default() -> Self {
        Self {
            hulk_spawn_attempted: false,
            hulk: None,
            salvager: None,
            surveyed: false,
            status: WreckStatus::Unclaimed,
            claim_prep_left: 0,
            tow_ticks_left: 0,
            stripping: None,
            intercepts: 0,
            warned_50: false,
            warned_75: false,
        }
    }
}

/// Idempotent resource setup for tests and the tick schedule.
pub fn ensure_salvager_resources(world: &mut World) {
    if world.get_resource::<SalvagerState>().is_none() {
        world.insert_resource(SalvagerState::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_salvager_resources(world);
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

// ---------------------------------------------------------------------------
// Pure mechanics (unit-testable, deterministic in their rolls)
// ---------------------------------------------------------------------------

/// Appraisal roll: `roll` in 0.0..1.0 classifies a system as relic, scrap,
/// or deadweight. Relic bands are per-kind ([`WreckSystemKind::relic_bias`]);
/// scrap fills the next [`APPRAISAL_SCRAP_BAND`]; the rest is deadweight.
#[must_use]
pub fn appraise_system(kind: WreckSystemKind, roll: f32) -> AppraisalClass {
    let bias = kind.relic_bias();
    if roll < bias {
        AppraisalClass::Relic
    } else if roll < bias + APPRAISAL_SCRAP_BAND {
        AppraisalClass::Scrap
    } else {
        AppraisalClass::Deadweight
    }
}

/// Roll the strip yields for a system: (scrap, relics). Relics are the
/// payload of relic-class systems; deadweight is barely worth cutting —
/// but every strip produces at least 1 scrap, so a first cut always pays
/// for a patch.
#[must_use]
pub fn roll_yields(kind: WreckSystemKind, class: AppraisalClass, roll: f32) -> (f32, u32) {
    match class {
        AppraisalClass::Relic => {
            let relics = if kind == WreckSystemKind::Reactor { 2 } else { 1 };
            (4.0 + roll * 6.0, relics)
        }
        AppraisalClass::Scrap => (8.0 + roll * 12.0, 0),
        AppraisalClass::Deadweight => (1.0 + roll * 2.0, 0),
    }
}

/// Ticks of stripping work to convert a system: relics need care,
/// deadweight cuts fast, and the reactor is a long careful job.
#[must_use]
pub fn strip_ticks(kind: WreckSystemKind, class: AppraisalClass) -> u32 {
    let base = match class {
        AppraisalClass::Relic => 60,
        AppraisalClass::Scrap => 40,
        AppraisalClass::Deadweight => 25,
    };
    base + if kind == WreckSystemKind::Reactor {
        20
    } else {
        0
    }
}

/// Instability gained per tick: the dying reactor creeps up on its own,
/// stripping agitates it, stripping the reactor agitates it a lot. Once
/// the reactor is gone the wreck is nearly (not quite) quiet.
#[must_use]
pub fn instability_per_tick(stripping: Option<WreckSystemKind>, reactor_gone: bool) -> f32 {
    if reactor_gone {
        return INSTABILITY_SPENT_PER_TICK;
    }
    INSTABILITY_BASE_PER_TICK
        + match stripping {
            None => 0.0,
            Some(WreckSystemKind::Reactor) => INSTABILITY_REACTOR_STRIP_PER_TICK,
            Some(_) => INSTABILITY_STRIP_PER_TICK,
        }
}

/// Claim-jumper interception roll while towing: true when the jumpers find
/// the tow cable.
#[must_use]
pub fn resolve_intercept(roll: f32) -> bool {
    roll < INTERCEPT_P_PER_TICK
}

/// Credit-equivalent value of a salvage pile (the value ledger's unit).
#[must_use]
pub fn credit_value(scrap: f32, relics: u32) -> f32 {
    scrap * SCRAP_CREDIT_VALUE + relics as f32 * RELIC_CREDIT_VALUE
}

/// One-shot payout for selling the wreck's coordinates.
#[must_use]
pub fn scuttle_payout(total_credit_value: f32) -> f32 {
    total_credit_value * SCUTTLE_PAYOUT_FRACTION
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn the derelict hulk (once): a dead vessel on a walkable tile in a
/// ring [`HULK_RING_MIN`]..=[`HULK_RING_MAX`] tiles from the lander, with
/// seven strippable systems, hull-breach vacuum tiles, a sealed ration
/// cache — and a lone salvager pop waiting aboard.
pub fn spawn_derelict_hulk_once(world: &mut World) {
    ensure_salvager_resources(world);
    {
        let state = world.resource::<SalvagerState>();
        if state.hulk_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<SalvagerState>().hulk_spawn_attempted = true;

    // Only one hulk per colony.
    if world.query::<&DerelictHulk>().iter(world).next().is_some() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    // Snapshot candidate tiles first (mirrors the skiff spawn: the spawn
    // calls below need `&mut world`, which conflicts with live borrows).
    let (walkable, occupied, buildings) = {
        let mut walkable = std::collections::HashSet::new();
        if let Some(terrain) = world.get_resource::<TerrainGrid>() {
            for dx in -HULK_RING_MAX..=HULK_RING_MAX {
                for dy in -HULK_RING_MAX..=HULK_RING_MAX {
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
        'search: for radius in HULK_RING_MIN..=HULK_RING_MAX {
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
    spawn_hulk_at(world, pos.x, pos.y, &walkable);
    send_chronicle(world, HULK_RUMOR_CHRONICLE, EventImportance::Minor);
}

/// Spawn the hulk and everything aboard it at an explicit tile. The world
/// functions and tests share this; `walkable` gates breach placement.
fn spawn_hulk_at(
    world: &mut World,
    x: i32,
    y: i32,
    walkable: &std::collections::HashSet<(i32, i32)>,
) -> Entity {
    let hulk = world
        .spawn((
            DerelictHulk,
            SalvageHold {
                food: RATION_CACHE_FOOD,
                scrap: 0.0,
                relics: 0,
            },
            HulkInstability::default(),
            GridPosition { x, y },
        ))
        .id();
    for kind in WreckSystemKind::all() {
        world.spawn((
            WreckSystem {
                kind,
                hulk,
                appraisal: None,
                scrap_yield: 0.0,
                relic_yield: 0,
                stripped: false,
                strip_progress: 0.0,
            },
            GridPosition { x, y },
        ));
    }
    for (dx, dy) in BREACH_OFFSET_CANDIDATES {
        let bx = x + dx;
        let by = y + dy;
        if bx < 0 || by < 0 || !walkable.contains(&(bx, by)) {
            continue;
        }
        world.spawn((
            HulkBreach { hulk, patched: false },
            GridPosition { x: bx, y: by },
        ));
        if world.query::<&HulkBreach>().iter(world).count() >= 4 {
            break;
        }
    }
    // The lone salvager, waiting aboard the sealed cabin.
    let mut rng = rand::thread_rng();
    let mut bundle = PopBundle::random(x, y, &mut rng);
    bundle.name = PopName("Salvager Jett".to_string());
    let salvager = world.spawn((bundle, Salvager)).id();
    world.resource_mut::<SalvagerState>().hulk = Some(hulk);
    world.resource_mut::<SalvagerState>().salvager = Some(salvager);
    hulk
}

/// The hulk within work reach of a pop, if any.
pub fn wreck_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    world
        .query::<(Entity, &DerelictHulk, &GridPosition)>()
        .iter(world)
        .find(|(_, _, hpos)| {
            (hpos.x - pos.x).abs() <= SPIKE_TAKE_REACH && (hpos.y - pos.y).abs() <= SPIKE_TAKE_REACH
        })
        .map(|(e, _, _)| e)
}

/// Take up the salvager's spike: a possessed pop beside the hulk becomes
/// the wreck-diver, if no living salvager holds the role.
pub fn try_take_spike(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_salvager_resources(world);
    if world.get::<Salvager>(pop).is_some() {
        return Err("This pop already walks the wreck-diver's path.".to_string());
    }
    let hulk = wreck_within_reach(world, pop)
        .ok_or_else(|| "No derelict hulk within reach. Move beside one.".to_string())?;
    let salvager = world.resource::<SalvagerState>().salvager;
    let salvager_alive = salvager.and_then(|s| world.get::<Pop>(s)).is_some();
    if salvager_alive {
        return Err("The hulk already has its salvager.".to_string());
    }
    let hpos = *world
        .get::<GridPosition>(hulk)
        .ok_or_else(|| "The hulk has no position.".to_string())?;
    world.entity_mut(pop).insert(Salvager);
    if let Some(mut pos) = world.get_mut::<GridPosition>(pop) {
        *pos = hpos;
    }
    world.resource_mut::<SalvagerState>().salvager = Some(pop);
    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));
    send_chronicle(
        world,
        &format!("{name} takes up the salvager's spike and boards the derelict."),
        EventImportance::Major,
    );
    Ok(format!(
        "{name} takes up the salvager's spike and boards the derelict hulk at ({}, {}).",
        hpos.x, hpos.y
    ))
}

// ---------------------------------------------------------------------------
// Origin actions
// ---------------------------------------------------------------------------

/// Look up the live salvager pop, cleaning up the state if they're gone.
fn live_salvager(world: &mut World) -> Option<Entity> {
    ensure_salvager_resources(world);
    let s = world.resource::<SalvagerState>().salvager;
    match s {
        Some(e) if world.get::<Pop>(e).is_some() && world.get::<Salvager>(e).is_some() => Some(e),
        _ => {
            world.resource_mut::<SalvagerState>().salvager = None;
            None
        }
    }
}

/// The live hulk entity, if it still exists.
fn live_hulk(world: &mut World) -> Option<Entity> {
    ensure_salvager_resources(world);
    let h = world.resource::<SalvagerState>().hulk;
    match h {
        Some(e) if world.get::<DerelictHulk>(e).is_some() => Some(e),
        _ => None,
    }
}

/// Require the caller to be the living salvager origin pop.
fn require_salvager_pop(world: &mut World, caller: Entity) -> Result<Entity, String> {
    let salvager =
        live_salvager(world).ok_or_else(|| "The salvager is gone — the wreck keeps its secrets.".to_string())?;
    if caller != salvager {
        return Err("Only the salvager can work the wreck. Possess them first.".to_string());
    }
    Ok(salvager)
}

/// The salvager must stand aboard (within [`WRECK_WORK_REACH`]) to work.
fn require_aboard(world: &mut World, salvager: Entity, hulk: Entity) -> Result<(), String> {
    let spos = world
        .get::<GridPosition>(salvager)
        .copied()
        .ok_or_else(|| "The salvager has no position.".to_string())?;
    let hpos = world
        .get::<GridPosition>(hulk)
        .copied()
        .ok_or_else(|| "The hulk has no position.".to_string())?;
    if (hpos.x - spos.x).abs() > WRECK_WORK_REACH || (hpos.y - spos.y).abs() > WRECK_WORK_REACH {
        return Err("Board the hulk first — the wreck is worked from aboard.".to_string());
    }
    Ok(())
}

/// Wreck work is only possible while she drifts unclaimed or claimed.
fn require_workable_status(world: &mut World) -> Result<(), String> {
    match world.resource::<SalvagerState>().status {
        WreckStatus::Unclaimed | WreckStatus::Claimed => Ok(()),
        WreckStatus::Towing => Err("She's under tow — no more cutting.".to_string()),
        WreckStatus::Towed => Err("She's home. The wreck is colony property now.".to_string()),
        WreckStatus::Scuttled => Err("The coordinates are sold — she's gone.".to_string()),
        WreckStatus::Destroyed => Err("There's nothing left to strip.".to_string()),
    }
}

/// Survey the wreck: roll the appraisal for every system (once), and print
/// the manifest with the value ledger.
pub fn survey_wreck(world: &mut World, caller: Entity) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_aboard(world, salvager, hulk)?;
    require_workable_status(world)?;

    let already = world.resource::<SalvagerState>().surveyed;
    if !already {
        let mut rng = rand::thread_rng();
        // Collect first: the mutation below needs `&mut world`.
        let systems: Vec<Entity> = world
            .query::<(Entity, &WreckSystem)>()
            .iter(world)
            .filter(|(_, s)| s.hulk == hulk && !s.stripped)
            .map(|(e, _)| e)
            .collect();
        for e in systems {
            let kind = world.get::<WreckSystem>(e).map(|s| s.kind).unwrap();
            let class = appraise_system(kind, rng.gen_range(0.0..1.0));
            let (scrap, relics) = roll_yields(kind, class, rng.gen_range(0.0..1.0));
            if let Some(mut sys) = world.get_mut::<WreckSystem>(e) {
                sys.appraisal = Some(class);
                sys.scrap_yield = scrap;
                sys.relic_yield = relics;
            }
        }
        world.resource_mut::<SalvagerState>().surveyed = true;
    }

    Ok(format_survey_manifest(world, hulk))
}

/// Render the survey manifest + value ledger for a hulk.
fn format_survey_manifest(world: &mut World, hulk: Entity) -> String {
    let mut systems: Vec<WreckSystem> = world
        .query::<&WreckSystem>()
        .iter(world)
        .filter(|s| s.hulk == hulk)
        .copied()
        .collect();
    systems.sort_by_key(|s| s.kind as u8);
    let mut lines = vec!["Wreck survey — systems:".to_string()];
    let mut ledger = 0.0f32;
    for s in &systems {
        let state = if s.stripped {
            "stripped".to_string()
        } else {
            match s.appraisal {
                Some(class) => {
                    ledger += credit_value(s.scrap_yield, s.relic_yield);
                    format!(
                        "{:<10} — est. {} relic(s) + {:.1} scrap",
                        class.label(),
                        s.relic_yield,
                        s.scrap_yield
                    )
                }
                None => "unsurveyed".to_string(),
            }
        };
        lines.push(format!("  {:<12} {}", s.kind.name(), state));
    }
    let breaches: Vec<(HulkBreach, GridPosition)> = world
        .query::<(&HulkBreach, &GridPosition)>()
        .iter(world)
        .filter(|(b, _)| b.hulk == hulk)
        .map(|(b, p)| (*b, *p))
        .collect();
    let patched = breaches.iter().filter(|(b, _)| b.patched).count();
    let open_coords: Vec<String> = breaches
        .iter()
        .filter(|(b, _)| !b.patched)
        .map(|(_, p)| format!("({},{})", p.x, p.y))
        .collect();
    lines.push(format!(
        "Hull breaches: {} ({} patched){}",
        breaches.len(),
        patched,
        if open_coords.is_empty() {
            String::new()
        } else {
            format!(" — open at {}", open_coords.join(" "))
        }
    ));
    let stripping = world.resource::<SalvagerState>().stripping;
    if let Some(kind) = stripping {
        lines.push(format!("Stripping: {} (in progress)", kind.name()));
    }
    lines.push(format!("Ledger (unstripped): {:.1}cr equiv.", ledger));
    lines.join("\n")
}

/// Begin stripping a wreck system: the conversion runs over time in
/// [`salvager_tick`]. One strip at a time; the reactor is a long, hot job.
pub fn start_strip(
    world: &mut World,
    caller: Entity,
    kind: WreckSystemKind,
) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_aboard(world, salvager, hulk)?;
    require_workable_status(world)?;
    if !world.resource::<SalvagerState>().surveyed {
        return Err("Survey the wreck first — you don't cut blind.".to_string());
    }
    if world.resource::<SalvagerState>().stripping.is_some() {
        return Err("A strip is already underway. One cut at a time.".to_string());
    }
    let (class, ticks) = {
        let sys = world
            .query::<&WreckSystem>()
            .iter(world)
            .find(|s| s.hulk == hulk && s.kind == kind)
            .copied()
            .ok_or_else(|| format!("No {} aboard the hulk.", kind.name()))?;
        if sys.stripped {
            return Err(format!("The {} is already stripped.", kind.name()));
        }
        let class = sys
            .appraisal
            .ok_or_else(|| "Survey the wreck first — you don't cut blind.".to_string())?;
        (class, strip_ticks(kind, class))
    };
    world.resource_mut::<SalvagerState>().stripping = Some(kind);
    if let Some(mut sys) = world
        .query::<&mut WreckSystem>()
        .iter_mut(world)
        .find(|s| s.hulk == hulk && s.kind == kind)
    {
        sys.strip_progress = 0.0;
    }
    if kind == WreckSystemKind::Reactor {
        send_chronicle(
            world,
            "Cutting torches bite into the dead reactor. The instability needle climbs.",
            EventImportance::Major,
        );
    }
    Ok(format!(
        "Began stripping the {} ({} — ~{} ticks). Watch the instability meter.",
        kind.name(),
        class.label(),
        ticks
    ))
}

/// Patch the nearest unpatched hull breach within reach. Costs scrap from
/// the salvage hold — the wreck pays for her own bandages.
pub fn patch_breach(world: &mut World, caller: Entity) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_workable_status(world)?;
    let spos = world
        .get::<GridPosition>(salvager)
        .copied()
        .ok_or_else(|| "The salvager has no position.".to_string())?;
    let breach = world
        .query::<(Entity, &HulkBreach, &GridPosition)>()
        .iter(world)
        .filter(|(_, b, _)| b.hulk == hulk && !b.patched)
        .min_by_key(|(_, _, p)| (p.x - spos.x).abs().max((p.y - spos.y).abs()))
        .map(|(e, _, _)| e);
    let Some(breach) = breach else {
        return Err("No open breaches within reach.".to_string());
    };
    let bpos = world.get::<GridPosition>(breach).copied().unwrap();
    if (bpos.x - spos.x).abs() > 1 || (bpos.y - spos.y).abs() > 1 {
        return Err("Move adjacent to a breach to patch it.".to_string());
    }
    let scrap = world
        .get::<SalvageHold>(hulk)
        .map(|h| h.scrap)
        .unwrap_or(0.0);
    if scrap < PATCH_SCRAP_COST {
        return Err(format!(
            "Patching costs {:.0} scrap — the hold is empty.",
            PATCH_SCRAP_COST
        ));
    }
    if let Some(mut hold) = world.get_mut::<SalvageHold>(hulk) {
        hold.scrap -= PATCH_SCRAP_COST;
    }
    if let Some(mut b) = world.get_mut::<HulkBreach>(breach) {
        b.patched = true;
    }
    send_chronicle(
        world,
        &format!(
            "A hull breach at ({}, {}) is patched — the seal holds.",
            bpos.x, bpos.y
        ),
        EventImportance::Minor,
    );
    Ok(format!(
        "Patched the breach at ({}, {}) (-{:.0} scrap). She holds pressure there now.",
        bpos.x, bpos.y, PATCH_SCRAP_COST
    ))
}

/// Plant your claim beacon: tow-prep begins, then `tow` unlocks.
pub fn claim_hulk(world: &mut World, caller: Entity) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_aboard(world, salvager, hulk)?;
    if world.resource::<SalvagerState>().status != WreckStatus::Unclaimed {
        return Err("The wreck is already claimed, towed, or gone.".to_string());
    }
    world.resource_mut::<SalvagerState>().status = WreckStatus::Claimed;
    world.resource_mut::<SalvagerState>().claim_prep_left = CLAIM_PREP_TICKS;
    send_chronicle(
        world,
        "A claim beacon blooms on the derelict's spine. Tow-prep underway.",
        EventImportance::Major,
    );
    Ok(format!(
        "Claim beacon planted. Tow-prep: {} ticks — then `tow` her home.",
        CLAIM_PREP_TICKS
    ))
}

/// Begin the tow home: burns colony fuel, takes a while, and claim-jumpers
/// may intercept the cable on the way.
pub fn start_tow(world: &mut World, caller: Entity) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_aboard(world, salvager, hulk)?;
    {
        let state = world.resource::<SalvagerState>();
        if state.status != WreckStatus::Claimed {
            return Err("Plant a claim beacon first (`claim`).".to_string());
        }
        if state.claim_prep_left > 0 {
            return Err(format!(
                "Tow-prep underway — {} ticks before the cable can take the strain.",
                state.claim_prep_left
            ));
        }
    }
    let fuel = world.resource::<ColonyResources>().fuel;
    if fuel < TOW_FUEL_COST {
        return Err(format!(
            "The tow needs {:.0} fuel for engine burns — the colony has {:.1}.",
            TOW_FUEL_COST, fuel
        ));
    }
    world.resource_mut::<ColonyResources>().fuel -= TOW_FUEL_COST;
    world.resource_mut::<SalvagerState>().status = WreckStatus::Towing;
    world.resource_mut::<SalvagerState>().tow_ticks_left = TOW_TICKS;
    send_chronicle(
        world,
        "Tow cable grapples the derelict. She's coming home — if the dark lets her.",
        EventImportance::Major,
    );
    Ok(format!(
        "Tow underway ({} ticks, -{:.0} fuel). Keep her stripped and watch for jumpers.",
        TOW_TICKS, TOW_FUEL_COST
    ))
}

/// Scuttle the claim: sell the wreck's coordinates for a one-shot credit
/// payout to the salvager's own wallet. Quick, safe, and worth less than
/// the tow.
pub fn scuttle_hulk(world: &mut World, caller: Entity) -> Result<String, String> {
    let salvager = require_salvager_pop(world, caller)?;
    let hulk = live_hulk(world).ok_or_else(|| "No derelict hulk in this colony.".to_string())?;
    require_aboard(world, salvager, hulk)?;
    match world.resource::<SalvagerState>().status {
        WreckStatus::Unclaimed | WreckStatus::Claimed => {}
        _ => return Err("Too late to scuttle — she's towed, sold, or gone.".to_string()),
    }
    let hold = world
        .get::<SalvageHold>(hulk)
        .copied()
        .unwrap_or_default();
    let remaining: f32 = world
        .query::<&WreckSystem>()
        .iter(world)
        .filter(|s| s.hulk == hulk && !s.stripped && s.appraisal.is_some())
        .map(|s| credit_value(s.scrap_yield, s.relic_yield))
        .sum();
    let total = credit_value(hold.scrap, hold.relics) + remaining;
    let payout = scuttle_payout(total);
    if let Some(mut wallet) = world.get_mut::<Wallet>(salvager) {
        wallet.credits += payout;
    }
    despawn_hulk(world, hulk);
    world.resource_mut::<SalvagerState>().status = WreckStatus::Scuttled;
    world.resource_mut::<SalvagerState>().stripping = None;
    send_chronicle(
        world,
        &format!(
            "The wreck's coordinates change hands for {:.1}cr. Tugs take her; the salvager walks away richer.",
            payout
        ),
        EventImportance::Major,
    );
    Ok(format!(
        "Coordinates sold for {:.1}cr (40% of {:.1}cr appraised). She's gone.",
        payout, total
    ))
}

/// Remove the hulk and everything aboard her.
fn despawn_hulk(world: &mut World, hulk: Entity) {
    let systems: Vec<Entity> = world
        .query::<(Entity, &WreckSystem)>()
        .iter(world)
        .filter(|(_, s)| s.hulk == hulk)
        .map(|(e, _)| e)
        .collect();
    let breaches: Vec<Entity> = world
        .query::<(Entity, &HulkBreach)>()
        .iter(world)
        .filter(|(_, b)| b.hulk == hulk)
        .map(|(e, _)| e)
        .collect();
    for e in systems.into_iter().chain(breaches) {
        world.despawn(e);
    }
    world.despawn(hulk);
    if world.resource::<SalvagerState>().hulk == Some(hulk) {
        world.resource_mut::<SalvagerState>().hulk = None;
    }
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

/// Per-tick wreck simulation: instability climbs (and blows the reactor at
/// 100), stripping work advances, claim-prep and tow count down (with
/// claim-jumper interceptions), breach tiles hold or lose pressure, the
/// ration cache feeds the salvager, and the unpossessed salvager is kept
/// aboard.
pub fn salvager_tick(world: &mut World) {
    ensure_salvager_resources(world);
    let hulk = match live_hulk(world) {
        Some(h) => h,
        None => return,
    };
    // Salvager death cleanup: the wreck keeps drifting without them.
    live_salvager(world);

    let mut rng = rand::thread_rng();

    // --- Stripping work -------------------------------------------------------
    let stripping = world.resource::<SalvagerState>().stripping;
    if let Some(kind) = stripping {
        let done = {
            let mut sys = world
                .query::<&mut WreckSystem>()
                .iter_mut(world)
                .find(|s| s.hulk == hulk && s.kind == kind);
            match sys.as_deref_mut() {
                None => true, // system vanished; stop stripping
                Some(s) if s.stripped => true,
                Some(s) => {
                    let class = s.appraisal.unwrap_or(AppraisalClass::Deadweight);
                    let need = strip_ticks(kind, class) as f32;
                    s.strip_progress += 1.0;
                    s.strip_progress >= need
                }
            }
        };
        if done {
            complete_strip(world, hulk, kind);
        }
    }

    // --- Instability ----------------------------------------------------------
    {
        let reactor_gone = world
            .query::<&WreckSystem>()
            .iter(world)
            .find(|s| s.hulk == hulk && s.kind == WreckSystemKind::Reactor)
            .map(|s| s.stripped)
            .unwrap_or(true);
        let rate = instability_per_tick(world.resource::<SalvagerState>().stripping, reactor_gone);
        let v = {
            let mut inst = world.get_mut::<HulkInstability>(hulk).unwrap();
            inst.value = (inst.value + rate).min(INSTABILITY_MAX);
            inst.value
        };
        let (fire_50, fire_75) = {
            let mut state = world.resource_mut::<SalvagerState>();
            let f50 = v >= INSTABILITY_WARN && !state.warned_50;
            let f75 = v >= INSTABILITY_CRITICAL && !state.warned_75;
            if f50 {
                state.warned_50 = true;
            }
            if f75 {
                state.warned_75 = true;
            }
            (f50, f75)
        };
        if fire_50 {
            send_chronicle(
                world,
                "The derelict's reactor is running hot — the needle trembles past half.",
                EventImportance::Minor,
            );
        }
        if fire_75 {
            send_chronicle(
                world,
                "CRITICAL: the dead reactor is waking up. Finish the cut or get clear.",
                EventImportance::Major,
            );
        }
        if v >= INSTABILITY_MAX {
            reactor_breach(world, hulk);
            return;
        }
    }

    // --- Claim prep -------------------------------------------------------------
    {
        let mut state = world.resource_mut::<SalvagerState>();
        if state.status == WreckStatus::Claimed && state.claim_prep_left > 0 {
            state.claim_prep_left -= 1;
            if state.claim_prep_left == 0 {
                send_chronicle(
                    world,
                    "Tow-prep complete. The cable can take her weight — `tow` when ready.",
                    EventImportance::Minor,
                );
            }
        }
    }

    // --- Tow --------------------------------------------------------------------
    let tow_done = {
        let towing = world.resource::<SalvagerState>().status == WreckStatus::Towing;
        if !towing {
            false
        } else {
            if resolve_intercept(rng.gen_range(0.0..1.0)) {
                jumper_intercept(world, hulk);
            }
            let mut state = world.resource_mut::<SalvagerState>();
            state.tow_ticks_left = state.tow_ticks_left.saturating_sub(1);
            state.tow_ticks_left == 0
        }
    };
    if tow_done {
        complete_tow(world, hulk);
        return;
    }

    // --- Pressure: the sealed cabin and patched breaches hold; open ------------
    // --- breaches are vacuum. Runs before the pressure damage system. ---------
    if let Some(hpos) = world.get::<GridPosition>(hulk).copied() {
        let breaches: Vec<(GridPosition, bool)> = world
            .query::<(&HulkBreach, &GridPosition)>()
            .iter(world)
            .filter(|(b, _)| b.hulk == hulk)
            .map(|(b, p)| (*p, b.patched))
            .collect();
        if let Some(mut grid) = world.get_resource_mut::<PressureGrid>() {
            grid.set(hpos.x, hpos.y, 1.0);
            for (p, patched) in breaches {
                grid.set(p.x, p.y, if patched { 1.0 } else { 0.0 });
            }
        }
    }

    // --- Ration cache: feed the salvager ----------------------------------------
    if let Some(salvager) = world.resource::<SalvagerState>().salvager {
        let hungry = world
            .get::<Needs>(salvager)
            .map(|n| n.hunger < MESS_HUNGER_GATE)
            .unwrap_or(false);
        if hungry {
            let meal = MESS_MEAL_PER_TICK;
            let food = world.get::<SalvageHold>(hulk).map(|h| h.food).unwrap_or(0.0);
            if food >= meal {
                if let Some(mut hold) = world.get_mut::<SalvageHold>(hulk) {
                    hold.food -= meal;
                }
                if let Some(mut needs) = world.get_mut::<Needs>(salvager) {
                    needs.hunger = (needs.hunger + MESS_SATIETY).min(1.0);
                }
            }
        }
        // Tether the unpossessed salvager to the wreck.
        if world.get::<Possessed>(salvager).is_none() {
            if let (Some(spos), Some(hpos)) = (
                world.get::<GridPosition>(salvager).copied(),
                world.get::<GridPosition>(hulk).copied(),
            ) {
                let dx = hpos.x - spos.x;
                let dy = hpos.y - spos.y;
                if dx.abs().max(dy.abs()) > SALVAGER_TETHER_RADIUS {
                    if let Some(mut p) = world.get_mut::<GridPosition>(salvager) {
                        p.x += dx.signum();
                        p.y += dy.signum();
                    }
                }
            }
        }
    }
}

/// Finish a strip: yields land in the hold, the system is spent.
fn complete_strip(world: &mut World, hulk: Entity, kind: WreckSystemKind) {
    let (scrap, relics, label) = {
        let mut q = world.query::<&mut WreckSystem>();
        let mut sys = q.iter_mut(world).find(|s| s.hulk == hulk && s.kind == kind);
        match sys.as_deref_mut() {
            Some(s) if !s.stripped => {
                let out = (
                    s.scrap_yield,
                    s.relic_yield,
                    s.appraisal.map(|c| c.label()).unwrap_or("unknown"),
                );
                s.stripped = true;
                s.strip_progress = 0.0;
                out
            }
            _ => {
                world.resource_mut::<SalvagerState>().stripping = None;
                return;
            }
        }
    };
    if let Some(mut hold) = world.get_mut::<SalvageHold>(hulk) {
        hold.scrap += scrap;
        hold.relics += relics;
    }
    world.resource_mut::<SalvagerState>().stripping = None;
    send_chronicle(
        world,
        &format!(
            "The {} is stripped clean ({}): +{:.1} scrap{} to the hold.",
            kind.name(),
            label,
            scrap,
            if relics > 0 {
                format!(", +{} relic(s)", relics)
            } else {
                String::new()
            }
        ),
        EventImportance::Minor,
    );
}

/// The reactor lets go: no yield from it, injuries aboard, the wreck goes
/// quiet but not safe.
fn reactor_breach(world: &mut World, hulk: Entity) {
    if let Some(mut sys) = world
        .query::<&mut WreckSystem>()
        .iter_mut(world)
        .find(|s| s.hulk == hulk && s.kind == WreckSystemKind::Reactor)
    {
        sys.stripped = true;
        sys.strip_progress = 0.0;
    }
    world.resource_mut::<SalvagerState>().stripping = None;
    if let Some(mut inst) = world.get_mut::<HulkInstability>(hulk) {
        inst.value = INSTABILITY_POST_BREACH;
    }
    {
        let mut state = world.resource_mut::<SalvagerState>();
        state.warned_50 = false;
        state.warned_75 = false;
        state.status = WreckStatus::Destroyed;
    }
    let hpos = world.get::<GridPosition>(hulk).copied();
    if let Some(hpos) = hpos {
        let victims: Vec<Entity> = world
            .query_filtered::<(Entity, &GridPosition), With<Pop>>()
            .iter(world)
            .filter(|(_, p)| {
                (p.x - hpos.x).abs() <= REACTOR_BREACH_RADIUS
                    && (p.y - hpos.y).abs() <= REACTOR_BREACH_RADIUS
            })
            .map(|(e, _)| e)
            .collect();
        for v in victims {
            if let Some(mut health) = world.get_mut::<Health>(v) {
                health.take_damage(REACTOR_BREACH_DAMAGE);
            }
        }
    }
    send_chronicle(world, REACTOR_BREACH_CHRONICLE, EventImportance::Major);
}

/// Claim-jumpers hit the tow: they peel scrap and relics off the cable.
fn jumper_intercept(world: &mut World, hulk: Entity) {
    let (scrap_lost, relics_lost) = {
        let hold = world.get::<SalvageHold>(hulk).copied().unwrap_or_default();
        let scrap_lost = hold.scrap * INTERCEPT_SCRAP_THEFT;
        let relics_lost = if hold.relics > 0 {
            ((hold.relics as f32 * INTERCEPT_SCRAP_THEFT).floor() as u32).max(1)
        } else {
            0
        };
        if let Some(mut h) = world.get_mut::<SalvageHold>(hulk) {
            h.scrap -= scrap_lost;
            h.relics -= relics_lost;
        }
        (scrap_lost, relics_lost)
    };
    world.resource_mut::<SalvagerState>().intercepts += 1;
    send_chronicle(
        world,
        &format!(
            "Claim-jumpers hit the tow cable! -{:.1} scrap{} ripped away.",
            scrap_lost,
            if relics_lost > 0 {
                format!(", -{} relic(s)", relics_lost)
            } else {
                String::new()
            }
        ),
        EventImportance::Major,
    );
}

/// The tow arrives: the colony takes the scrap and buys the relics, the
/// salvager rides the cable home.
fn complete_tow(world: &mut World, hulk: Entity) {
    let hold = world
        .get::<SalvageHold>(hulk)
        .copied()
        .unwrap_or_default();
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.scrap += hold.scrap;
        resources.credits += hold.relics as f32 * RELIC_CREDIT_VALUE;
    }
    let lander_pos = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    if let Some(salvager) = world.resource::<SalvagerState>().salvager {
        if let Some(lp) = lander_pos {
            if let Some(mut pos) = world.get_mut::<GridPosition>(salvager) {
                pos.x = lp.x + 1;
                pos.y = lp.y + 1;
            }
        }
    }
    let hpos = world
        .get::<GridPosition>(hulk)
        .map(|p| format!("({}, {})", p.x, p.y))
        .unwrap_or_else(|| "adrift".to_string());
    despawn_hulk(world, hulk);
    world.resource_mut::<SalvagerState>().status = WreckStatus::Towed;
    world.resource_mut::<SalvagerState>().stripping = None;
    send_chronicle(
        world,
        &format!(
            "The derelict comes home from {}. +{:.1} scrap to the colony, {} relic(s) sold for {:.1}cr.",
            hpos,
            hold.scrap,
            hold.relics,
            hold.relics as f32 * RELIC_CREDIT_VALUE
        ),
        EventImportance::Major,
    );
}

// ---------------------------------------------------------------------------
// Stats & status
// ---------------------------------------------------------------------------

/// Snapshot for the headless STATS line: (wreck status, salvage value).
pub fn salvager_stats(world: &mut World) -> (String, f32) {
    ensure_salvager_resources(world);
    let state = world.resource::<SalvagerState>().clone();
    let status = match state.status {
        WreckStatus::Claimed => format!("claimed(prep={})", state.claim_prep_left),
        WreckStatus::Towing => format!("towing({})", state.tow_ticks_left),
        _ => state.status.label().to_string(),
    };
    let hulk = state.hulk;
    let hold: Option<SalvageHold> = hulk.and_then(|h| world.get::<SalvageHold>(h).copied());
    let value = match hold {
        Some(hold) => {
            let remaining: f32 = world
                .query::<&WreckSystem>()
                .iter(world)
                .filter(|s| Some(s.hulk) == hulk && !s.stripped && s.appraisal.is_some())
                .map(|s| credit_value(s.scrap_yield, s.relic_yield))
                .sum();
            credit_value(hold.scrap, hold.relics) + remaining
        }
        None => 0.0,
    };
    (status, value)
}

/// Human-readable origin status for the `salvager` command.
pub fn describe_salvager(world: &mut World) -> String {
    ensure_salvager_resources(world);
    let state = world.resource::<SalvagerState>().clone();
    let Some(hulk) = live_hulk(world) else {
        return match state.status {
            WreckStatus::Towed => "The derelict is home — colony property now.".to_string(),
            WreckStatus::Scuttled => "The coordinates are sold — she's gone.".to_string(),
            _ => "No derelict hulk in this colony.".to_string(),
        };
    };
    let hold = world.get::<SalvageHold>(hulk).copied().unwrap_or_default();
    let instability = world
        .get::<HulkInstability>(hulk)
        .map(|i| i.value)
        .unwrap_or(0.0);
    let pos = world
        .get::<GridPosition>(hulk)
        .map(|p| format!("({}, {})", p.x, p.y))
        .unwrap_or_else(|| "adrift".to_string());
    let salvager = state
        .salvager
        .and_then(|s| world.get::<PopName>(s).map(|n| n.0.clone()))
        .unwrap_or_else(|| "none — the wreck waits".to_string());
    let breaches: Vec<HulkBreach> = world
        .query::<&HulkBreach>()
        .iter(world)
        .filter(|b| b.hulk == hulk)
        .copied()
        .collect();
    let patched = breaches.iter().filter(|b| b.patched).count();
    let stripping = state
        .stripping
        .map(|k| k.name().to_string())
        .unwrap_or_else(|| "idle".to_string());
    format!(
        "Derelict hulk at {pos} — {status}\n\
         Hold: {scrap:.2} scrap, {relics} relics, {food:.1} food (ration cache)\n\
         Reactor instability: {inst:.1}/100 | Breaches: {patched}/{n} patched\n\
         Stripping: {stripping} | Claim-jumpers fought off: {jumps}\n\
         Salvager: {salvager}",
        status = state.status.label(),
        scrap = hold.scrap,
        relics = hold.relics,
        food = hold.food,
        inst = instability,
        patched = patched,
        n = breaches.len(),
        stripping = stripping,
        jumps = state.intercepts,
        salvager = salvager,
    )
}

// ---------------------------------------------------------------------------
// Tests (atomic TDD: each mechanic specified here first)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::core::chronicle::AddChronicleEvent;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::economy::Wallet;
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::{Pop, PopName};
    use crate::layer1::psychology::needs::Needs;
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
        world.insert_resource(SalvagerState::default());
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
                Wallet::default(),
                PopAction::default(),
            ))
            .id()
    }

    /// Direct hulk spawn for deterministic tests (no terrain/lander needed).
    fn spawn_hulk_direct(world: &mut World, x: i32, y: i32) -> Entity {
        let mut walkable = std::collections::HashSet::new();
        for dx in -3..=3 {
            for dy in -3..=3 {
                walkable.insert((x + dx, y + dy));
            }
        }
        spawn_hulk_at(world, x, y, &walkable)
    }

    fn salvager_of(world: &mut World) -> Entity {
        world.resource::<SalvagerState>().salvager.unwrap()
    }

    fn set_appraisal(
        world: &mut World,
        hulk: Entity,
        kind: WreckSystemKind,
        class: AppraisalClass,
    ) {
        let mut q = world.query::<&mut WreckSystem>();
        let mut s = q
            .iter_mut(world)
            .find(|s| s.hulk == hulk && s.kind == kind)
            .unwrap();
        s.appraisal = Some(class);
        let (scrap, relics) = roll_yields(kind, class, 0.5);
        s.scrap_yield = scrap;
        s.relic_yield = relics;
    }

    fn hold_of(world: &mut World, hulk: Entity) -> SalvageHold {
        world.get::<SalvageHold>(hulk).copied().unwrap()
    }

    // --- pure mechanics --------------------------------------------------------

    #[test]
    fn test_appraise_relic_band() {
        assert_eq!(
            appraise_system(WreckSystemKind::Reactor, 0.10),
            AppraisalClass::Relic
        );
    }

    #[test]
    fn test_appraise_scrap_band() {
        assert_eq!(
            appraise_system(WreckSystemKind::Reactor, 0.30),
            AppraisalClass::Scrap
        );
    }

    #[test]
    fn test_appraise_deadweight_band() {
        assert_eq!(
            appraise_system(WreckSystemKind::Reactor, 0.90),
            AppraisalClass::Deadweight
        );
    }

    #[test]
    fn test_appraise_relic_bias_differs_by_kind() {
        // Thrusters are dull (bias 0.12): 0.15 is scrap there...
        assert_eq!(
            appraise_system(WreckSystemKind::ThrusterBank, 0.15),
            AppraisalClass::Scrap
        );
        // ...but a relic for the reactor (bias 0.25).
        assert_eq!(
            appraise_system(WreckSystemKind::Reactor, 0.15),
            AppraisalClass::Relic
        );
    }

    #[test]
    fn test_roll_yields_relic() {
        let (scrap, relics) = roll_yields(WreckSystemKind::Reactor, AppraisalClass::Relic, 0.5);
        assert_eq!((scrap, relics), (7.0, 2));
        let (scrap, relics) = roll_yields(WreckSystemKind::Engine, AppraisalClass::Relic, 0.5);
        assert_eq!((scrap, relics), (7.0, 1));
    }

    #[test]
    fn test_roll_yields_scrap_range() {
        let (scrap, relics) = roll_yields(WreckSystemKind::CargoBay, AppraisalClass::Scrap, 0.0);
        assert_eq!((scrap, relics), (8.0, 0));
        let (scrap, _) = roll_yields(WreckSystemKind::CargoBay, AppraisalClass::Scrap, 1.0);
        assert!((scrap - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_roll_yields_deadweight_is_junk() {
        let (scrap, relics) =
            roll_yields(WreckSystemKind::CargoBay, AppraisalClass::Deadweight, 0.5);
        assert_eq!(relics, 0);
        assert!(
            (1.0..=3.0).contains(&scrap),
            "deadweight always pays for a patch"
        );
    }

    #[test]
    fn test_strip_ticks_ordering() {
        let dead = strip_ticks(WreckSystemKind::Engine, AppraisalClass::Deadweight);
        let scrap = strip_ticks(WreckSystemKind::Engine, AppraisalClass::Scrap);
        let relic = strip_ticks(WreckSystemKind::Engine, AppraisalClass::Relic);
        assert!(dead < scrap && scrap < relic);
        // The reactor is a longer job than the same class elsewhere.
        assert_eq!(
            strip_ticks(WreckSystemKind::Reactor, AppraisalClass::Scrap),
            scrap + 20
        );
    }

    #[test]
    fn test_instability_rates() {
        assert!((instability_per_tick(None, false) - INSTABILITY_BASE_PER_TICK).abs() < 1e-6);
        assert!(
            (instability_per_tick(Some(WreckSystemKind::Engine), false)
                - (INSTABILITY_BASE_PER_TICK + INSTABILITY_STRIP_PER_TICK))
                .abs()
                < 1e-6
        );
        assert!(
            (instability_per_tick(Some(WreckSystemKind::Reactor), false)
                - (INSTABILITY_BASE_PER_TICK + INSTABILITY_REACTOR_STRIP_PER_TICK))
                .abs()
                < 1e-6
        );
        assert!(
            (instability_per_tick(Some(WreckSystemKind::Reactor), true)
                - INSTABILITY_SPENT_PER_TICK)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn test_resolve_intercept() {
        assert!(resolve_intercept(0.01));
        assert!(!resolve_intercept(0.5));
    }

    #[test]
    fn test_credit_value_and_scuttle_payout() {
        assert!((credit_value(10.0, 2) - (10.0 * 2.0 + 2.0 * 50.0)).abs() < 1e-6);
        assert!((scuttle_payout(100.0) - 40.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_system_names() {
        assert_eq!(
            WreckSystemKind::parse("reactor"),
            Some(WreckSystemKind::Reactor)
        );
        assert_eq!(
            WreckSystemKind::parse("life"),
            Some(WreckSystemKind::LifeSupport)
        );
        assert_eq!(
            WreckSystemKind::parse("thrusters"),
            Some(WreckSystemKind::ThrusterBank)
        );
        assert_eq!(WreckSystemKind::parse("bogus"), None);
    }

    // --- spawn -----------------------------------------------------------------

    #[test]
    fn test_spawn_creates_hulk_systems_breaches_and_salvager() {
        let mut world = setup();
        let mut terrain = generate_terrain(40, 40);
        terrain.tiles.fill(TerrainType::Grass);
        world.insert_resource(terrain);
        world.insert_resource(OccupiedTiles::default());
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x: 20, y: 20 },
        ));
        spawn_derelict_hulk_once(&mut world);

        let hulk = world.resource::<SalvagerState>().hulk.unwrap();
        let systems = world
            .query::<&WreckSystem>()
            .iter(&world)
            .filter(|s| s.hulk == hulk)
            .count();
        assert_eq!(systems, 7, "seven strippable systems");
        let breaches = world
            .query::<&HulkBreach>()
            .iter(&world)
            .filter(|b| b.hulk == hulk)
            .count();
        assert_eq!(breaches, 4, "four breach tiles on open grass");
        let salvager = world.resource::<SalvagerState>().salvager.unwrap();
        assert!(world.get::<Salvager>(salvager).is_some());
        let hpos = world.get::<GridPosition>(hulk).unwrap();
        let spos = world.get::<GridPosition>(salvager).unwrap();
        assert_eq!((spos.x, spos.y), (hpos.x, hpos.y), "salvager spawns aboard");
        assert_eq!(hold_of(&mut world, hulk).food, RATION_CACHE_FOOD);

        // Idempotent: a second call spawns nothing new.
        spawn_derelict_hulk_once(&mut world);
        assert_eq!(
            world.query::<&DerelictHulk>().iter(&world).count(),
            1
        );
    }

    // --- survey ------------------------------------------------------------------

    #[test]
    fn test_survey_rolls_every_system_and_prints_ledger() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        let manifest = survey_wreck(&mut world, salvager).expect("survey should succeed");
        assert!(world.resource::<SalvagerState>().surveyed);
        for kind in WreckSystemKind::all() {
            let sys = world
                .query::<&WreckSystem>()
                .iter(&world)
                .find(|s| s.hulk == hulk && s.kind == kind)
                .unwrap();
            assert!(sys.appraisal.is_some(), "{:?} appraised", kind);
            assert!(manifest.contains(kind.name()));
        }
        assert!(manifest.contains("Ledger (unstripped):"));
        // Relic-class systems really do carry relics; deadweight carries none.
        for sys in world.query::<&WreckSystem>().iter(&world) {
            match sys.appraisal.unwrap() {
                AppraisalClass::Relic => assert!(sys.relic_yield > 0),
                AppraisalClass::Deadweight => assert_eq!(sys.relic_yield, 0),
                AppraisalClass::Scrap => assert_eq!(sys.relic_yield, 0),
            }
        }
    }

    #[test]
    fn test_survey_requires_boarding() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        world.get_mut::<GridPosition>(salvager).unwrap().x = 60;
        world.get_mut::<GridPosition>(salvager).unwrap().y = 60;
        assert!(survey_wreck(&mut world, salvager).is_err());
    }

    #[test]
    fn test_survey_resurvey_is_stable() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        let first: Vec<(WreckSystemKind, Option<AppraisalClass>)> = world
            .query::<&WreckSystem>()
            .iter(&world)
            .filter(|s| s.hulk == hulk)
            .map(|s| (s.kind, s.appraisal))
            .collect();
        survey_wreck(&mut world, salvager).unwrap();
        let second: Vec<(WreckSystemKind, Option<AppraisalClass>)> = world
            .query::<&WreckSystem>()
            .iter(&world)
            .filter(|s| s.hulk == hulk)
            .map(|s| (s.kind, s.appraisal))
            .collect();
        assert_eq!(first, second, "re-survey must not re-roll");
    }

    // --- stripping -----------------------------------------------------------------

    #[test]
    fn test_strip_requires_survey() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        assert!(start_strip(&mut world, salvager, WreckSystemKind::Engine).is_err());
    }

    #[test]
    fn test_strip_completes_and_yields_hold() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        set_appraisal(&mut world, hulk, WreckSystemKind::Engine, AppraisalClass::Scrap);
        let (expected_scrap, expected_relics) =
            roll_yields(WreckSystemKind::Engine, AppraisalClass::Scrap, 0.5);
        start_strip(&mut world, salvager, WreckSystemKind::Engine).unwrap();
        let ticks = strip_ticks(WreckSystemKind::Engine, AppraisalClass::Scrap);
        for _ in 0..ticks {
            salvager_tick(&mut world);
        }
        let hold = hold_of(&mut world, hulk);
        assert!((hold.scrap - expected_scrap).abs() < 0.01);
        assert_eq!(hold.relics, expected_relics);
        assert!(world.resource::<SalvagerState>().stripping.is_none());
        let sys = world
            .query::<&WreckSystem>()
            .iter(&world)
            .find(|s| s.hulk == hulk && s.kind == WreckSystemKind::Engine)
            .unwrap();
        assert!(sys.stripped);
    }

    #[test]
    fn test_strip_one_at_a_time() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        start_strip(&mut world, salvager, WreckSystemKind::Engine).unwrap();
        assert!(start_strip(&mut world, salvager, WreckSystemKind::CargoBay).is_err());
    }

    #[test]
    fn test_strip_reactor_spikes_instability() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        set_appraisal(&mut world, hulk, WreckSystemKind::Reactor, AppraisalClass::Scrap);
        start_strip(&mut world, salvager, WreckSystemKind::Reactor).unwrap();
        let before = world.get::<HulkInstability>(hulk).unwrap().value;
        for _ in 0..10 {
            salvager_tick(&mut world);
        }
        let after = world.get::<HulkInstability>(hulk).unwrap().value;
        let expected = 10.0 * (INSTABILITY_BASE_PER_TICK + INSTABILITY_REACTOR_STRIP_PER_TICK);
        assert!((after - before - expected).abs() < 0.01);
    }

    // --- hazards ---------------------------------------------------------------------

    #[test]
    fn test_instability_breach_destroys_reactor() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        world.get_mut::<HulkInstability>(hulk).unwrap().value = 99.95;
        let hp_before = world.get::<Health>(salvager).unwrap().current;
        salvager_tick(&mut world);
        let reactor = world
            .query::<&WreckSystem>()
            .iter(&world)
            .find(|s| s.hulk == hulk && s.kind == WreckSystemKind::Reactor)
            .unwrap();
        assert!(reactor.stripped, "breach spends the reactor with no yield");
        assert_eq!(hold_of(&mut world, hulk).relics, 0);
        assert_eq!(
            world.resource::<SalvagerState>().status,
            WreckStatus::Destroyed
        );
        assert!((world.get::<HulkInstability>(hulk).unwrap().value - INSTABILITY_POST_BREACH).abs() < 0.01);
        let hp_after = world.get::<Health>(salvager).unwrap().current;
        assert!((hp_before - hp_after - REACTOR_BREACH_DAMAGE).abs() < 0.01);
    }

    #[test]
    fn test_instability_warnings_fire_once() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let _salvager = salvager_of(&mut world);
        world.get_mut::<HulkInstability>(hulk).unwrap().value = INSTABILITY_WARN + 1.0;
        salvager_tick(&mut world);
        assert!(world.resource::<SalvagerState>().warned_50);
        assert!(!world.resource::<SalvagerState>().warned_75);
    }

    #[test]
    fn test_patch_consumes_scrap_and_seals() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        // Stand on the first breach tile.
        let bpos = world
            .query::<(&HulkBreach, &GridPosition)>()
            .iter(&world)
            .find(|(b, _)| b.hulk == hulk)
            .map(|(_, p)| *p)
            .unwrap();
        *world.get_mut::<GridPosition>(salvager).unwrap() = bpos;
        world.get_mut::<SalvageHold>(hulk).unwrap().scrap = 5.0;
        let msg = patch_breach(&mut world, salvager).expect("patch should succeed");
        assert!(msg.contains("Patched"));
        assert!((hold_of(&mut world, hulk).scrap - 4.0).abs() < 0.01);
        let patched = world
            .query::<&HulkBreach>()
            .iter(&world)
            .filter(|b| b.hulk == hulk && b.patched)
            .count();
        assert_eq!(patched, 1);
    }

    #[test]
    fn test_patch_needs_scrap_and_reach() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        let bpos = world
            .query::<(&HulkBreach, &GridPosition)>()
            .iter(&world)
            .find(|(b, _)| b.hulk == hulk)
            .map(|(_, p)| *p)
            .unwrap();
        *world.get_mut::<GridPosition>(salvager).unwrap() = bpos;
        // Empty hold: no patch.
        assert!(patch_breach(&mut world, salvager).is_err());
        // Far away: no patch even with scrap.
        world.get_mut::<SalvageHold>(hulk).unwrap().scrap = 5.0;
        world.get_mut::<GridPosition>(salvager).unwrap().x = 60;
        world.get_mut::<GridPosition>(salvager).unwrap().y = 60;
        assert!(patch_breach(&mut world, salvager).is_err());
    }

    #[test]
    fn test_tick_sets_breach_vacuum_and_cabin_pressure() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        world.insert_resource(PressureGrid::new(40, 40));
        salvager_tick(&mut world);
        let hpos = *world.get::<GridPosition>(hulk).unwrap();
        let bpos: GridPosition = world
            .query::<(&HulkBreach, &GridPosition)>()
            .iter(&world)
            .find(|(b, _)| b.hulk == hulk && !b.patched)
            .map(|(_, p)| *p)
            .unwrap();
        let grid = world.resource::<PressureGrid>();
        assert!((grid.get(hpos.x, hpos.y) - 1.0).abs() < 0.01);
        assert!(grid.get(bpos.x, bpos.y) < 0.01, "open breach is vacuum");
    }

    #[test]
    fn test_tick_feeds_salvager_from_cache() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        world.get_mut::<Needs>(salvager).unwrap().hunger = 0.5;
        let food_before = hold_of(&mut world, hulk).food;
        salvager_tick(&mut world);
        let hunger_after = world.get::<Needs>(salvager).unwrap().hunger;
        assert!(hunger_after > 0.5);
        assert!(hold_of(&mut world, hulk).food < food_before);
    }

    // --- claim / tow / scuttle -----------------------------------------------------------

    #[test]
    fn test_claim_starts_prep_countdown() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        claim_hulk(&mut world, salvager).expect("claim should succeed");
        assert_eq!(world.resource::<SalvagerState>().status, WreckStatus::Claimed);
        assert_eq!(world.resource::<SalvagerState>().claim_prep_left, CLAIM_PREP_TICKS);
        for _ in 0..CLAIM_PREP_TICKS {
            salvager_tick(&mut world);
        }
        assert_eq!(world.resource::<SalvagerState>().claim_prep_left, 0);
    }

    #[test]
    fn test_tow_needs_prep_and_fuel() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        // No claim at all: no tow.
        assert!(start_tow(&mut world, salvager).is_err());
        claim_hulk(&mut world, salvager).unwrap();
        // Prep still running: no tow.
        assert!(start_tow(&mut world, salvager).is_err());
        world.resource_mut::<SalvagerState>().claim_prep_left = 0;
        // No fuel: no tow.
        assert!(start_tow(&mut world, salvager).is_err());
        world.resource_mut::<ColonyResources>().fuel = 100.0;
        start_tow(&mut world, salvager).expect("tow should start");
        assert!((world.resource::<ColonyResources>().fuel - (100.0 - TOW_FUEL_COST)).abs() < 0.01);
        assert_eq!(world.resource::<SalvagerState>().status, WreckStatus::Towing);
    }

    #[test]
    fn test_tow_completes_delivery() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x: 0, y: 0 },
        ));
        claim_hulk(&mut world, salvager).unwrap();
        world.resource_mut::<SalvagerState>().claim_prep_left = 0;
        world.resource_mut::<ColonyResources>().fuel = 100.0;
        start_tow(&mut world, salvager).unwrap();
        world.get_mut::<SalvageHold>(hulk).unwrap().scrap = 10.0;
        world.get_mut::<SalvageHold>(hulk).unwrap().relics = 2;
        world.resource_mut::<SalvagerState>().tow_ticks_left = 3;
        for _ in 0..3 {
            salvager_tick(&mut world);
        }
        // Intercepts may have skimmed the hold; the colony still gets paid.
        assert_eq!(world.resource::<SalvagerState>().status, WreckStatus::Towed);
        assert!(world.resource::<ColonyResources>().scrap >= 10.0 * (1.0 - INTERCEPT_SCRAP_THEFT * 3.0));
        assert!(world.get::<DerelictHulk>(hulk).is_none(), "hulk despawned");
        // The salvager rides the cable home to the lander.
        let spos = world.get::<GridPosition>(salvager).unwrap();
        assert_eq!((spos.x, spos.y), (1, 1));
    }

    #[test]
    fn test_scuttle_pays_wallet_and_clears_wreck() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        // Deterministic appraisals for the payout math.
        for kind in WreckSystemKind::all() {
            set_appraisal(&mut world, hulk, kind, AppraisalClass::Scrap);
        }
        world.get_mut::<SalvageHold>(hulk).unwrap().scrap = 10.0;
        let remaining: f32 = WreckSystemKind::all()
            .iter()
            .map(|k| {
                let (s, r) = roll_yields(*k, AppraisalClass::Scrap, 0.5);
                credit_value(s, r)
            })
            .sum();
        let expected = scuttle_payout(credit_value(10.0, 0) + remaining);
        let wallet_before = world.get::<Wallet>(salvager).unwrap().credits;
        let msg = scuttle_hulk(&mut world, salvager).expect("scuttle should succeed");
        assert!(msg.contains("Coordinates sold"));
        let wallet_after = world.get::<Wallet>(salvager).unwrap().credits;
        assert!((wallet_after - wallet_before - expected).abs() < 0.01);
        assert_eq!(
            world.resource::<SalvagerState>().status,
            WreckStatus::Scuttled
        );
        assert!(world.get::<DerelictHulk>(hulk).is_none());
    }

    #[test]
    fn test_scuttle_blocked_while_towing() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        claim_hulk(&mut world, salvager).unwrap();
        world.resource_mut::<SalvagerState>().claim_prep_left = 0;
        world.resource_mut::<ColonyResources>().fuel = 100.0;
        start_tow(&mut world, salvager).unwrap();
        assert!(scuttle_hulk(&mut world, salvager).is_err());
    }

    // --- misc ----------------------------------------------------------------------

    #[test]
    fn test_take_spike_boards_a_new_salvager() {
        let mut world = setup();
        let hulk = spawn_hulk_direct(&mut world, 10, 10);
        let hpos = *world.get::<GridPosition>(hulk).unwrap();
        // The original salvager is gone.
        let old = salvager_of(&mut world);
        world.entity_mut(old).remove::<Salvager>();
        world.resource_mut::<SalvagerState>().salvager = None;
        let rookie = spawn_pop(&mut world, "rookie", hpos.x + 1, hpos.y);
        let msg = try_take_spike(&mut world, rookie).expect("spike should be taken");
        assert!(msg.contains("spike"));
        assert!(world.get::<Salvager>(rookie).is_some());
        assert_eq!(world.resource::<SalvagerState>().salvager, Some(rookie));
    }

    #[test]
    fn test_salvager_stats_reports_status_and_value() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        let (status, value) = salvager_stats(&mut world);
        assert_eq!(status, "unclaimed");
        assert!(value > 0.0, "ledger value is positive after survey");
    }

    #[test]
    fn test_live_mirror_strip_with_rolled_survey() {
        // Mirrors the live playtest: rolled survey, strip a deadweight
        // system, tick past completion, hold must grow.
        let mut world = setup();
        let mut terrain = generate_terrain(40, 40);
        terrain.tiles.fill(TerrainType::Grass);
        world.insert_resource(terrain);
        world.insert_resource(OccupiedTiles::default());
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x: 20, y: 20 },
        ));
        spawn_derelict_hulk_once(&mut world);
        let hulk = world.resource::<SalvagerState>().hulk.unwrap();
        let salvager = salvager_of(&mut world);
        survey_wreck(&mut world, salvager).unwrap();
        let kind = world
            .query::<&WreckSystem>()
            .iter(&world)
            .find(|s| s.hulk == hulk && s.appraisal == Some(AppraisalClass::Deadweight))
            .map(|s| s.kind)
            .unwrap();
        let expected = world
            .query::<&WreckSystem>()
            .iter(&world)
            .find(|s| s.hulk == hulk && s.kind == kind)
            .map(|s| s.scrap_yield)
            .unwrap();
        start_strip(&mut world, salvager, kind).expect("strip starts");
        for _ in 0..30 {
            salvager_tick(&mut world);
        }
        let hold = hold_of(&mut world, hulk);
        assert!(
            (hold.scrap - expected).abs() < 0.01,
            "hold.scrap={} expected={}",
            hold.scrap,
            expected
        );
    }

    #[test]
    fn test_dead_salvager_clears_state() {
        let mut world = setup();
        let _hulk = spawn_hulk_direct(&mut world, 10, 10);
        let salvager = salvager_of(&mut world);
        world.get_mut::<Health>(salvager).unwrap().current = 0.0;
        world.entity_mut(salvager).remove::<Pop>();
        salvager_tick(&mut world);
        assert!(world.resource::<SalvagerState>().salvager.is_none());
        assert!(survey_wreck(&mut world, salvager).is_err());
    }
}
