//! The Chronostalker — adventurer origin #7.
//!
//! Simmons-flavor, concept-only, ORIGINAL NAMES ONLY. A chrome entity that
//! walks between moments; it haunts the colony's crises. The stalker's path:
//!
//! - **The Moment-Wound**: a shimmer in time near the starter colony. A
//!   possessed pop beside it may `interact` to take the stalker's path —
//!   possession transfers to Tock, the chrome stranger waiting in the wound.
//! - **`phase`**: slip out of the time stream. While phased, Tock walks
//!   through walls, water, and wreckage alike and cannot be hurt — but
//!   temporal debt accrues every tick.
//! - **`anchor`**: stand still in real time; each motionless tick the ledger
//!   forgives some debt. Moving breaks the anchor. Phase and anchor are
//!   mutually exclusive.
//! - **The ledger**: debt past [`DEBT_THRESHOLD`] is overdrawn — reality
//!   repossesses ("The ledger takes its due."), injuring Tock and forcing
//!   them back into the stream.
//! - **Haunting**: colony crises (sabotage, pirate raids, depressurization)
//!   draw the stalker. A chronicle notes "Something chrome walks the edges
//!   of the moment.", and the unpossessed stalker drifts (phased, at the
//!   ledger's expense) toward the crisis site, mending what it can on
//!   arrival — the ledger accepts the work as repayment.
//! - **`rewind <n>`**: fold the last few ticks back — Tock returns to where
//!   they stood n ticks ago, at a debt cost per tick (cap [`REWIND_CAP`]).
//!
//! Headless console: `phase`, `anchor`, `rewind <n>`, `stalker` (ledger
//! state); STATS gains `chronodebt=<n>`.
//!
//! The concept is concept-only inspiration. No named characters, places, or
//! distinctive IP anywhere — mechanics and vibes under original names.
//! "The Chronostalker", "Tock", "the Moment-Wound", "the ledger" are
//! original coinages for this game.

use bevy_ecs::prelude::*;

use crate::layer1::actions::AssignedTo;
use crate::layer1::agriculture::zero_g_flora::DepressurizationEvent;
use crate::layer1::biology::symbiotic_insurgency::SabotageEvent;
use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::direct_link::{DirectControlState, Possessed};
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::structure::Structure;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::StartPlan;
use crate::layer1::void_weed::PirateRaidEvent;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn ring (Chebyshev) around the lander where the moment-wound lands.
pub const WOUND_RING_MIN: i32 = 6;
pub const WOUND_RING_MAX: i32 = 10;
/// How close (Chebyshev) a pop must be to the wound to take the path.
pub const TAKE_REACH: i32 = 2;
/// How far (Chebyshev) the dormant stalker may drift from the wound
/// before the tick nudges it back (only while unpossessed).
pub const STALKER_TETHER_RADIUS: i32 = 3;
/// Temporal debt accrued per tick while phased.
pub const PHASE_DEBT_PER_TICK: f32 = 1.0;
/// Debt forgiven per motionless anchored tick.
pub const ANCHOR_REPAY_PER_TICK: f32 = 2.0;
/// Debt forgiven when the haunting stalker mends a crisis site.
pub const ERRAND_REPAY: f32 = 10.0;
/// Debt at or past which reality repossesses.
pub const DEBT_THRESHOLD: f32 = 100.0;
/// Debt left on the books after a repossession.
pub const DEBT_REMAINDER: f32 = 40.0;
/// Health damage dealt by a repossession.
pub const REPOSSESS_INJURY: f32 = 25.0;
/// Max ticks a single `rewind` may fold back.
pub const REWIND_CAP: u32 = 10;
/// Debt cost per rewound tick.
pub const REWIND_DEBT_PER_TICK: f32 = 2.0;
/// Position history kept for rewinds.
pub const HISTORY_LEN: usize = 40;
/// Ticks between haunting chronicles (the first is always logged).
pub const HAUNT_CHRONICLE_COOLDOWN: u64 = 300;
/// Arrival radius (Chebyshev) at which the haunting stalker starts mending.
pub const HAUNT_ARRIVE_RADIUS: i32 = 2;
/// Radius (Chebyshev) around the stalker searched for damaged structures
/// when it arrives at a crisis site.
pub const HAUNT_REPAIR_RADIUS: i32 = 6;
/// Integrity restored by a haunting mend.
pub const HAUNT_REPAIR_HP: f32 = 20.0;
/// The chrome stranger's name — an original coinage.
pub const STALKER_NAME: &str = "Tock";

// ---------------------------------------------------------------------------
// Components & state
// ---------------------------------------------------------------------------

/// Marker on the moment-wound site entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct MomentWound;

/// Marker on the dormant chrome stranger, before the path is taken.
#[derive(Component, Debug, Clone, Copy)]
pub struct DormantStalker;

/// Marker on the awakened Chronostalker. The utility AI never reassigns
/// it (see the `Without<Chronostalker>` filter).
#[derive(Component, Debug, Clone, Copy)]
pub struct Chronostalker;

/// Out of the time stream: walks through the unwalkable, cannot be hurt,
/// debt accrues. Removed by `phase` (toggle), arrival after a haunt, or a
/// repossession.
#[derive(Component, Debug, Clone, Copy)]
pub struct Phased;

/// Machine state for the Chronostalker origin.
#[derive(Resource, Debug, Clone)]
pub struct ChronostalkerState {
    pub wound_spawn_attempted: bool,
    pub wound: Option<Entity>,
    pub dormant: Option<Entity>,
    pub stalker: Option<Entity>,
    /// The temporal debt ledger. Grows while phased; forgiven by anchors
    /// and haunting mends; overdrawn at [`DEBT_THRESHOLD`].
    pub debt: f32,
    pub phased: bool,
    pub anchored: bool,
    pub anchor_pos: Option<GridPosition>,
    pub haunt_target: Option<(i32, i32)>,
    /// True while the current phase was auto-entered for a haunt.
    pub haunt_auto_phase: bool,
    pub history: Vec<GridPosition>,
    pub phases: u32,
    pub repossessions: u32,
    pub haunts: u32,
    pub repairs: u32,
    pub rewinds: u32,
    pub first_phase_done: bool,
    pub first_anchor_done: bool,
    pub first_repossess_done: bool,
    pub first_haunt_done: bool,
    pub last_haunt_tick: u64,
}

impl Default for ChronostalkerState {
    fn default() -> Self {
        Self {
            wound_spawn_attempted: false,
            wound: None,
            dormant: None,
            stalker: None,
            debt: 0.0,
            phased: false,
            anchored: false,
            anchor_pos: None,
            haunt_target: None,
            haunt_auto_phase: false,
            history: Vec::new(),
            phases: 0,
            repossessions: 0,
            haunts: 0,
            repairs: 0,
            rewinds: 0,
            first_phase_done: false,
            first_anchor_done: false,
            first_repossess_done: false,
            first_haunt_done: false,
            last_haunt_tick: 0,
        }
    }
}

impl ChronostalkerState {
    /// One-line state word for STATS and status.
    #[must_use]
    pub fn state_word(&self) -> &'static str {
        if self.phased {
            "phased"
        } else if self.anchored {
            "anchored"
        } else {
            "walking"
        }
    }
}

pub fn ensure_chronostalker_resources(world: &mut World) {
    if world.get_resource::<ChronostalkerState>().is_none() {
        world.insert_resource(ChronostalkerState::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_chronostalker_resources(world);
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

pub const STALKER_SPAWN_CHRONICLE: &str =
    "Rumor: time folds wrong off the beacon line. Somewhere out there, \
     something chrome walks between the moments, counting.";
pub const PHASE_ON_CHRONICLE: &str =
    "Tock slips sideways out of the moment. The world goes thin and bright \
     at the edges; walls are only suggestions now, and the ledger starts \
     counting.";
pub const PHASE_OFF_CHRONICLE: &str =
    "Tock steps back into the stream of time. The walls remember being \
     solid. The ledger keeps its pencil.";
pub const ANCHOR_ON_CHRONICLE: &str =
    "Tock plants chrome feet in the ticking dark and holds perfectly still. \
     Somewhere, a ledger begins its slow arithmetic of forgiveness.";
pub const ANCHOR_BREAK_CHRONICLE: &str =
    "Tock moves, and the anchor-line snaps like a dry twig. The ledger \
     stops its forgiving sums.";
pub const REPOSSESS_CHRONICLE: &str =
    "The ledger takes its due.";
pub const REPOSSESS_DETAIL_CHRONICLE: &str =
    "Reality repossesses the difference from Tock's chrome, with interest. \
     Tock is dragged, sparking, back into the stream of time.";
pub const HAUNT_CHRONICLE: &str =
    "Something chrome walks the edges of the moment.";
pub const HAUNT_AID_CHRONICLE: &str =
    "The chrome stranger passes a hand over the wound in the world, and the \
     wound closes a little. The ledger notes the work, grudgingly.";
pub const FIRST_REWIND_CHRONICLE: &str =
    "Tock folds a few ticks back into the ledger's pocket and steps out of \
     them, earlier. The pocket is smaller now.";

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn the moment-wound and its dormant chrome stranger once, in a ring
/// around the lander. Mirrors the cradle/shuttle ring-spawn pattern
/// (snapshot-then-spawn to satisfy the borrow checker).
pub fn spawn_moment_wound_once(world: &mut World) {
    ensure_chronostalker_resources(world);
    {
        let state = world.resource::<ChronostalkerState>();
        if state.wound_spawn_attempted {
            return;
        }
    }
    world
        .resource_mut::<ChronostalkerState>()
        .wound_spawn_attempted = true;

    if world.query::<&MomentWound>().iter(world).next().is_some() {
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
            for dx in -WOUND_RING_MAX..=WOUND_RING_MAX {
                for dy in -WOUND_RING_MAX..=WOUND_RING_MAX {
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
        'search: for radius in WOUND_RING_MIN..=WOUND_RING_MAX {
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
    spawn_wound_at(world, pos.x, pos.y);
    send_chronicle(world, STALKER_SPAWN_CHRONICLE, EventImportance::Minor);
}

/// Spawn the wound and dormant stranger at an explicit tile. Shared by
/// the ring-spawn and the tests.
fn spawn_wound_at(world: &mut World, x: i32, y: i32) -> Entity {
    ensure_chronostalker_resources(world);
    let wound = world.spawn((MomentWound, GridPosition { x, y })).id();
    let mut rng = rand::thread_rng();
    let mut bundle = PopBundle::random(x + 1, y, &mut rng);
    bundle.name = PopName(STALKER_NAME.to_string());
    let dormant = world.spawn((bundle, DormantStalker)).id();
    let mut state = world.resource_mut::<ChronostalkerState>();
    state.wound = Some(wound);
    state.dormant = Some(dormant);
    wound
}

/// The moment-wound within take reach of a pop, if any.
pub fn wound_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    world
        .query::<(Entity, &MomentWound, &GridPosition)>()
        .iter(world)
        .find(|(_, _, wpos)| {
            (wpos.x - pos.x).abs() <= TAKE_REACH && (wpos.y - pos.y).abs() <= TAKE_REACH
        })
        .map(|(e, _, _)| e)
}

/// Look up the live stalker, cleaning up the state if they're gone.
fn live_stalker(world: &mut World) -> Option<Entity> {
    ensure_chronostalker_resources(world);
    let s = world.resource::<ChronostalkerState>().stalker;
    match s {
        Some(e) if world.get::<Pop>(e).is_some() && world.get::<Chronostalker>(e).is_some() => {
            Some(e)
        }
        _ => {
            world.resource_mut::<ChronostalkerState>().stalker = None;
            None
        }
    }
}

/// Look up the live dormant stranger, if it still waits in the wound.
fn live_dormant(world: &mut World) -> Option<Entity> {
    ensure_chronostalker_resources(world);
    let d = world.resource::<ChronostalkerState>().dormant;
    match d {
        Some(e) if world.get::<Pop>(e).is_some() && world.get::<DormantStalker>(e).is_some() => {
            Some(e)
        }
        _ => {
            world.resource_mut::<ChronostalkerState>().dormant = None;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Origin actions
// ---------------------------------------------------------------------------

/// Take the stalker's path: a possessed pop beside the moment-wound steps
/// in, and possession transfers to the chrome stranger (mirroring
/// `handle_possession`'s component surgery). If the caller already
/// possessed the dormant directly, the path is taken in place.
pub fn try_take_stalker_path(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_chronostalker_resources(world);
    if live_stalker(world).is_some() {
        return Err("Tock already walks the between — the path is taken.".to_string());
    }
    wound_within_reach(world, pop)
        .ok_or_else(|| "No moment-wound within reach. Move beside one.".to_string())?;
    let dormant = live_dormant(world)
        .ok_or_else(|| "The wound is empty — its stranger is gone.".to_string())?;

    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));

    if pop == dormant {
        // The caller already possessed the dormant directly: the path is
        // taken in place.
        world
            .entity_mut(dormant)
            .insert(DirectControlState::default());
    } else {
        // Transfer possession, mirroring handle_possession.
        world
            .entity_mut(pop)
            .remove::<Possessed>()
            .remove::<DirectControlState>();
        world
            .entity_mut(dormant)
            .insert((Possessed, DirectControlState::default()));
    }
    world
        .entity_mut(dormant)
        .insert(Chronostalker)
        .remove::<DormantStalker>()
        .remove::<StartPlan>()
        .remove::<MovementTarget>()
        .remove::<AtTarget>()
        .remove::<AssignedTo>();

    world.resource_mut::<ChronostalkerState>().stalker = Some(dormant);
    world.resource_mut::<ChronostalkerState>().dormant = None;
    send_chronicle(
        world,
        &format!(
            "{name} steps into the moment-wound, and the wound steps into them. \
             Tock is awake — a chrome stranger who was never born and will \
             never quite die."
        ),
        EventImportance::Major,
    );
    Ok(format!(
        "{name} steps into the moment-wound. Tock unfolds — chrome, unhurried, \
         already late for several appointments with causality. (`phase` to slip \
         the stream, `anchor` to pay the ledger, `stalker` for the sums.)"
    ))
}

/// Toggle the phase: slip out of the time stream, or step back in.
/// Phase and anchor are mutually exclusive.
pub fn toggle_phase(world: &mut World) -> Result<String, String> {
    ensure_chronostalker_resources(world);
    let stalker = live_stalker(world).ok_or_else(|| {
        "No Chronostalker walks the colony yet. Find the moment-wound, possess a pop, and `interact` beside it.".to_string()
    })?;
    let phased = world.resource::<ChronostalkerState>().phased;
    if phased {
        world.entity_mut(stalker).remove::<Phased>();
        {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.phased = false;
            state.haunt_auto_phase = false;
        }
        send_chronicle(world, PHASE_OFF_CHRONICLE, EventImportance::Minor);
        Ok(
            "Tock steps back into the stream of time. Walls are solid again; \
             the ledger keeps its pencil."
                .to_string(),
        )
    } else {
        world.entity_mut(stalker).insert(Phased);
        let first = {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.phased = true;
            state.anchored = false;
            state.anchor_pos = None;
            state.phases += 1;
            let first = !state.first_phase_done;
            state.first_phase_done = true;
            first
        };
        if first {
            send_chronicle(world, PHASE_ON_CHRONICLE, EventImportance::Minor);
        }
        Ok(
            "Tock slips sideways out of the moment — walls are suggestions now, \
             nothing in the stream can touch them, and the ledger is counting. \
             (`anchor` to pay it down.)"
                .to_string(),
        )
    }
}

/// Toggle the anchor: stand still in real time and let the ledger forgive.
/// Moving breaks the anchor. Phase and anchor are mutually exclusive.
pub fn toggle_anchor(world: &mut World) -> Result<String, String> {
    ensure_chronostalker_resources(world);
    let stalker = live_stalker(world).ok_or_else(|| {
        "No Chronostalker walks the colony yet. Find the moment-wound, possess a pop, and `interact` beside it.".to_string()
    })?;
    let anchored = world.resource::<ChronostalkerState>().anchored;
    if anchored {
        {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.anchored = false;
            state.anchor_pos = None;
        }
        Ok(
            "Tock lifts the anchor. The ledger closes its little book of \
             forgiveness."
                .to_string(),
        )
    } else {
        let pos = world.get::<GridPosition>(stalker).copied();
        // Phase and anchor are mutually exclusive: you cannot hold still
        // outside of time.
        world.entity_mut(stalker).remove::<Phased>();
        let first = {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.phased = false;
            state.haunt_auto_phase = false;
            state.anchored = true;
            state.anchor_pos = pos;
            let first = !state.first_anchor_done;
            state.first_anchor_done = true;
            first
        };
        if first {
            send_chronicle(world, ANCHOR_ON_CHRONICLE, EventImportance::Minor);
        }
        Ok(
            "Tock plants chrome feet and holds perfectly still. Each motionless \
             tick, the ledger forgives a little. (Move, and the anchor snaps.)"
                .to_string(),
        )
    }
}

/// Fold the last `n` ticks back: the stalker returns to where they stood
/// `n` ticks ago, at [`REWIND_DEBT_PER_TICK`] debt per tick.
pub fn rewind_time(world: &mut World, n: u32) -> Result<String, String> {
    ensure_chronostalker_resources(world);
    let stalker = live_stalker(world).ok_or_else(|| {
        "No Chronostalker walks the colony yet. Find the moment-wound, possess a pop, and `interact` beside it.".to_string()
    })?;
    if n == 0 || n > REWIND_CAP {
        return Err(format!(
            "The ledger folds back at most {REWIND_CAP} ticks. Usage: rewind <1..={REWIND_CAP}>"
        ));
    }
    let (target, cost) = {
        let state = world.resource::<ChronostalkerState>();
        let len = state.history.len();
        if len <= n as usize {
            return Err(
                "The moment is too shallow — too few ticks recorded to fold back that far."
                    .to_string(),
            );
        }
        let target = state.history[len - 1 - n as usize];
        #[allow(clippy::cast_precision_loss)]
        let cost = n as f32 * REWIND_DEBT_PER_TICK;
        (target, cost)
    };
    if let Some(mut pos) = world.get_mut::<GridPosition>(stalker) {
        *pos = target;
    }
    {
        let mut state = world.resource_mut::<ChronostalkerState>();
        state.debt += cost;
        state.rewinds += 1;
        // Folding time moves you: the anchor cannot hold.
        state.anchored = false;
        state.anchor_pos = None;
    }
    send_chronicle(world, FIRST_REWIND_CHRONICLE, EventImportance::Minor);
    Ok(format!(
        "Tock folds {n} ticks back into the ledger's pocket and steps out of \
         them, earlier — at ({}, {}). The ledger notes +{cost:.1} debt.",
        target.x, target.y
    ))
}

// ---------------------------------------------------------------------------
// Per-tick origin system
// ---------------------------------------------------------------------------

/// Per-tick origin system: position history, phase debt accrual, anchor
/// repayment, repossession on overdraw, and the haunting drift.
pub fn chronostalker_tick(world: &mut World) {
    ensure_chronostalker_resources(world);
    let _tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);

    let stalker = match live_stalker(world) {
        Some(s) => s,
        None => {
            // No taken path yet — clear any stale haunt and tether the
            // dormant stranger to the wound (unpossessed only).
            world.resource_mut::<ChronostalkerState>().haunt_target = None;
            if let Some(dormant) = live_dormant(world) {
                let cpos = world
                    .resource::<ChronostalkerState>()
                    .wound
                    .and_then(|c| world.get::<GridPosition>(c).copied());
                if let Some(cpos) = cpos {
                    let possessed = world.query::<&Possessed>().get(world, dormant).is_ok();
                    if !possessed {
                        if let Some(dpos) = world.get::<GridPosition>(dormant).copied() {
                            let dx = cpos.x - dpos.x;
                            let dy = cpos.y - dpos.y;
                            if dx.abs().max(dy.abs()) > STALKER_TETHER_RADIUS {
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
    let spos = match world.get::<GridPosition>(stalker).copied() {
        Some(p) => p,
        None => return,
    };
    let possessed = world.query::<&Possessed>().get(world, stalker).is_ok();

    // --- position history (the ledger's pocket) ---------------------------
    {
        let mut state = world.resource_mut::<ChronostalkerState>();
        state.history.push(spos);
        if state.history.len() > HISTORY_LEN {
            let excess = state.history.len() - HISTORY_LEN;
            state.history.drain(..excess);
        }
    }

    // --- phase debt: the ledger counts every tick out of the stream -------
    if world.resource::<ChronostalkerState>().phased {
        world.resource_mut::<ChronostalkerState>().debt += PHASE_DEBT_PER_TICK;
    }

    // --- anchor: forgiveness while motionless, snapped by movement --------
    {
        let (anchored, anchor_pos) = {
            let state = world.resource::<ChronostalkerState>();
            (state.anchored, state.anchor_pos)
        };
        if anchored {
            let moved = anchor_pos
                .map(|a| a.x != spos.x || a.y != spos.y)
                .unwrap_or(true);
            if moved {
                {
                    let mut state = world.resource_mut::<ChronostalkerState>();
                    state.anchored = false;
                    state.anchor_pos = None;
                }
                send_chronicle(world, ANCHOR_BREAK_CHRONICLE, EventImportance::Minor);
            } else {
                let mut state = world.resource_mut::<ChronostalkerState>();
                state.debt = (state.debt - ANCHOR_REPAY_PER_TICK).max(0.0);
            }
        }
    }

    // --- repossession: overdraw the ledger and reality collects -----------
    {
        let debt = world.resource::<ChronostalkerState>().debt;
        if debt >= DEBT_THRESHOLD {
            world.entity_mut(stalker).remove::<Phased>();
            if let Some(mut health) = world.get_mut::<Health>(stalker) {
                health.take_damage(REPOSSESS_INJURY);
            }
            let first = {
                let mut state = world.resource_mut::<ChronostalkerState>();
                state.phased = false;
                state.anchored = false;
                state.anchor_pos = None;
                state.haunt_auto_phase = false;
                state.debt = DEBT_REMAINDER;
                state.repossessions += 1;
                let first = !state.first_repossess_done;
                state.first_repossess_done = true;
                first
            };
            send_chronicle(world, REPOSSESS_CHRONICLE, EventImportance::Major);
            if first {
                send_chronicle(world, REPOSSESS_DETAIL_CHRONICLE, EventImportance::Major);
            }
        }
    }

    // --- the draw of crisis ------------------------------------------------
    // A haunt target pulls the stalker, but never the player's hand:
    // possession (or the anchor) holds against the draw. Arrival still
    // mends — the chrome stranger's nature — whether driven or drifting.
    let haunt_target = world.resource::<ChronostalkerState>().haunt_target;
    let anchored = world.resource::<ChronostalkerState>().anchored;
    if let Some((tx, ty)) = haunt_target {
        if anchored {
            // The anchor holds against the draw.
        } else {
            let dist = (tx - spos.x).abs().max((ty - spos.y).abs());
            if dist <= HAUNT_ARRIVE_RADIUS {
                // Arrived: mend the most-damaged nearby structure; the
                // ledger accepts the work as repayment.
                let mend: Option<Entity> = world
                    .query::<(Entity, &Building, &GridPosition, &Structure)>()
                    .iter(world)
                    .filter(|(_, _, p, s)| {
                        s.current_hp < s.max_hp
                            && (p.x - spos.x).abs() <= HAUNT_REPAIR_RADIUS
                            && (p.y - spos.y).abs() <= HAUNT_REPAIR_RADIUS
                    })
                    .min_by(|(_, _, _, a), (_, _, _, b)| {
                        a.current_hp
                            .partial_cmp(&b.current_hp)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(e, _, _, _)| e);
                if let Some(e) = mend {
                    if let Some(mut structure) = world.get_mut::<Structure>(e) {
                        structure.current_hp =
                            (structure.current_hp + HAUNT_REPAIR_HP).min(structure.max_hp);
                    }
                }
                let was_auto = world.resource::<ChronostalkerState>().haunt_auto_phase;
                if was_auto {
                    world.entity_mut(stalker).remove::<Phased>();
                }
                {
                    let mut state = world.resource_mut::<ChronostalkerState>();
                    state.haunt_target = None;
                    if was_auto {
                        state.phased = false;
                        state.haunt_auto_phase = false;
                    }
                    state.debt = (state.debt - ERRAND_REPAY).max(0.0);
                    state.repairs += 1;
                }
                send_chronicle(world, HAUNT_AID_CHRONICLE, EventImportance::Minor);
            } else if !possessed {
                // Drift one step closer, phased — the haunt walks the
                // between, and the ledger counts the steps.
                if let Some(mut pos) = world.get_mut::<GridPosition>(stalker) {
                    pos.x += (tx - pos.x).signum();
                    pos.y += (ty - pos.y).signum();
                }
                world.entity_mut(stalker).insert(Phased);
                {
                    let mut state = world.resource_mut::<ChronostalkerState>();
                    state.phased = true;
                    state.haunt_auto_phase = true;
                }
            }
            // Possessed and far: the crisis waits. The player's hand is
            // on the stick; the draw resumes on release.
        }
    }
}

/// First building site of any of the given types (Chebyshev search order
/// is spawn order; crises don't carry positions, so any match will do).
fn colony_site(
    buildings: &Query<(&Building, &GridPosition)>,
    types: &[BuildingType],
) -> Option<(i32, i32)> {
    buildings
        .iter()
        .filter(|(b, _)| types.contains(&b.building_type))
        .map(|(_, p)| (p.x, p.y))
        .next()
}

/// Crisis bridge: sabotage, pirate raids, and depressurization draw the
/// stalker. Logs the haunting chronicle (cooldown-gated) and sets the
/// haunt target for the tick to drift toward.
#[allow(clippy::too_many_arguments)]
pub fn crisis_chronostalker_bridge(
    mut sabotage: EventReader<SabotageEvent>,
    mut raids: EventReader<PirateRaidEvent>,
    mut depressure: EventReader<DepressurizationEvent>,
    mut state: ResMut<ChronostalkerState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    buildings: Query<(&Building, &GridPosition)>,
    positions: Query<&GridPosition>,
    stalker_q: Query<Entity, With<Chronostalker>>,
    time: Res<SimulationTime>,
) {
    if stalker_q.iter().next().is_none() {
        return;
    }
    let mut hits = 0u32;
    let mut target: Option<(i32, i32)> = None;
    for _ in sabotage.read() {
        hits += 1;
        // Sabotage strikes at the air and the engines of it: drift to
        // life support, or the lander if none stands.
        target = colony_site(&buildings, &[BuildingType::LifeSupport, BuildingType::Lander]);
    }
    for _ in raids.read() {
        hits += 1;
        // Raiders strike the colony's heart.
        target = colony_site(&buildings, &[BuildingType::Lander]);
    }
    for event in depressure.read() {
        hits += 1;
        // Walk to the breached room itself.
        target = positions
            .get(event.0)
            .ok()
            .map(|p| (p.x, p.y))
            .or_else(|| colony_site(&buildings, &[BuildingType::Lander]));
    }
    if hits == 0 {
        return;
    }
    if let Some(t) = target {
        state.haunt_target = Some(t);
    }
    state.haunts += hits;
    let tick = time.tick;
    let cooldown_ok = tick.saturating_sub(state.last_haunt_tick) >= HAUNT_CHRONICLE_COOLDOWN;
    if !state.first_haunt_done || cooldown_ok {
        state.first_haunt_done = true;
        state.last_haunt_tick = tick;
        chronicle.send(AddChronicleEvent {
            text: HAUNT_CHRONICLE.to_string(),
            importance: EventImportance::Minor,
        });
    }
}

// ---------------------------------------------------------------------------
// Stats & status
// ---------------------------------------------------------------------------

/// The temporal debt for the headless STATS line (`chronodebt=<n>`).
pub fn chronostalker_debt(world: &mut World) -> Option<f32> {
    ensure_chronostalker_resources(world);
    live_stalker(world)?;
    Some(world.resource::<ChronostalkerState>().debt)
}

/// Human-readable stalker state for the `stalker` command.
pub fn describe_stalker(world: &mut World) -> String {
    ensure_chronostalker_resources(world);
    let state = world.resource::<ChronostalkerState>().clone();
    let Some(s) = state.stalker else {
        return "No Chronostalker walks the colony yet. Find the moment-wound, possess a pop, and `interact` beside it.".to_string();
    };
    if world.get::<Pop>(s).is_none() || world.get::<Chronostalker>(s).is_none() {
        return "The Chronostalker is gone — the wound keeps the shape of them.".to_string();
    }
    let name = world
        .get::<PopName>(s)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", s.index()));
    let bar_len = (state.debt / DEBT_THRESHOLD * 20.0).round() as usize;
    let bar = "#".repeat(bar_len.min(20)) + &"-".repeat(20usize.saturating_sub(bar_len.min(20)));
    let haunt = state.haunt_target.map_or_else(
        || "no crisis draws it".to_string(),
        |(x, y)| format!("drawn to ({x}, {y})"),
    );
    format!(
        "{name}, the Chronostalker — walks between moments\n\
         Temporal debt: {:.1}/{} [{}]\n\
         State: {}; haunt: {haunt}\n\
         Moments walked: {} phases, {} repossessions, {} haunts answered, {} mends, {} rewinds\n\
         (`phase` to slip the stream, `anchor` to pay the ledger, `rewind <n>` to fold time back)",
        state.debt,
        DEBT_THRESHOLD as u32,
        bar,
        state.state_word(),
        state.phases,
        state.repossessions,
        state.haunts,
        state.repairs,
        state.rewinds,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::economy::Wallet;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::{Pop, PopName};
    use crate::layer1::pressure::PressureGrid;
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
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(Events::<SabotageEvent>::default());
        world.insert_resource(Events::<PirateRaidEvent>::default());
        world.insert_resource(Events::<DepressurizationEvent>::default());
        world.insert_resource(ChronostalkerState::default());
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

    /// Take the stalker's path the honest way: possess a pop beside the
    /// wound and step in.
    fn take_direct(world: &mut World, x: i32, y: i32) -> (Entity, Entity) {
        let pop = spawn_pop(world, "Waker", x, y);
        world.entity_mut(pop).insert(Possessed);
        spawn_wound_at(world, x, y);
        try_take_stalker_path(world, pop).expect("take should succeed");
        let stalker = world.resource::<ChronostalkerState>().stalker.unwrap();
        (pop, stalker)
    }

    fn debt_of(world: &World) -> f32 {
        world.resource::<ChronostalkerState>().debt
    }

    fn spawn_lander(world: &mut World, x: i32, y: i32) {
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x, y },
        ));
    }

    // --- RED: taking the path -------------------------------------------------

    #[test]
    fn red_take_path_transfers_possession() {
        let mut world = setup();
        let (pop, stalker) = take_direct(&mut world, 10, 10);
        assert!(
            world.get::<Possessed>(pop).is_none(),
            "the waker is released"
        );
        assert!(world.get::<Possessed>(stalker).is_some());
        assert!(world.get::<Chronostalker>(stalker).is_some());
        assert!(world.get::<DormantStalker>(stalker).is_none());
        assert_eq!(
            world.get::<PopName>(stalker).unwrap().0,
            STALKER_NAME,
            "the stranger keeps its name"
        );
    }

    #[test]
    fn red_take_path_twice_is_refused() {
        let mut world = setup();
        let (pop, _stalker) = take_direct(&mut world, 10, 10);
        let other = spawn_pop(&mut world, "Second", 10, 10);
        world.entity_mut(other).insert(Possessed);
        let err = try_take_stalker_path(&mut world, other).unwrap_err();
        assert!(err.contains("already"), "got: {err}");
        let _ = pop;
    }

    // --- RED: phase ------------------------------------------------------------

    #[test]
    fn red_phase_toggles_and_accrues_debt() {
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        let msg = toggle_phase(&mut world).unwrap();
        assert!(msg.contains("slips") || msg.contains("phase"), "got: {msg}");
        assert!(world.get::<Phased>(stalker).is_some());
        assert!(world.resource::<ChronostalkerState>().phased);
        let before = debt_of(&world);
        chronostalker_tick(&mut world);
        assert!(
            (debt_of(&world) - before - PHASE_DEBT_PER_TICK).abs() < 1e-6,
            "debt accrues while phased"
        );
        let msg = toggle_phase(&mut world).unwrap();
        assert!(msg.contains("stream") || msg.contains("back"), "got: {msg}");
        assert!(world.get::<Phased>(stalker).is_none());
    }

    #[test]
    fn red_phase_walks_through_walls() {
        use crate::layer1::direct_link::try_player_step;
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        world.entity_mut(stalker).insert(DirectControlState::default());
        // Tile (12,10) is water: unwalkable. (The stalker spawns at (11,10).)
        world
            .get_resource_mut::<TerrainGrid>()
            .unwrap()
            .tiles[10 * 20 + 12] = TerrainType::Water;
        // Unphased: blocked.
        assert!(try_player_step(&mut world, stalker, 1, 0).is_err());
        toggle_phase(&mut world).unwrap();
        // Phased: slips through.
        let (nx, ny) = try_player_step(&mut world, stalker, 1, 0).expect("phased walks through");
        assert_eq!((nx, ny), (12, 10));
    }

    #[test]
    fn red_phased_cannot_be_hurt() {
        use crate::layer1::environment::geomes::{GeomeHazard, environmental_damage_system};
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        world.entity_mut(stalker).insert(Health {
            current: 100.0,
            max: 100.0,
            has_rust_lung: false,
        });
        // A hazard on the stalker's tile.
        world.spawn((
            GridPosition { x: 11, y: 10 },
            GeomeHazard {
                damage_per_tick: 10.0,
            },
        ));
        world.entity_mut(stalker).insert(GridPosition { x: 11, y: 10 });
        toggle_phase(&mut world).unwrap();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        schedule.add_systems(environmental_damage_system);
        schedule.run(&mut world);
        let health = world.get::<Health>(stalker).unwrap();
        assert!(
            (health.current - 100.0).abs() < 1e-6,
            "phased stalker takes no hazard damage, got {}",
            health.current
        );
    }

    // --- RED: anchor ------------------------------------------------------------

    #[test]
    fn red_anchor_repays_debt_while_still() {
        let mut world = setup();
        let (_pop, _stalker) = take_direct(&mut world, 10, 10);
        world.resource_mut::<ChronostalkerState>().debt = 50.0;
        toggle_anchor(&mut world).unwrap();
        assert!(world.resource::<ChronostalkerState>().anchored);
        chronostalker_tick(&mut world);
        assert!(
            (debt_of(&world) - (50.0 - ANCHOR_REPAY_PER_TICK)).abs() < 1e-6,
            "anchor repays while motionless, got {}",
            debt_of(&world)
        );
    }

    #[test]
    fn red_anchor_breaks_on_movement() {
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        toggle_anchor(&mut world).unwrap();
        // The stalker moves (player's hand, a shove, anything).
        world.entity_mut(stalker).insert(GridPosition { x: 12, y: 10 });
        chronostalker_tick(&mut world);
        assert!(
            !world.resource::<ChronostalkerState>().anchored,
            "moving breaks the anchor"
        );
    }

    #[test]
    fn red_phase_and_anchor_exclusive() {
        let mut world = setup();
        take_direct(&mut world, 10, 10);
        toggle_phase(&mut world).unwrap();
        toggle_anchor(&mut world).unwrap();
        let state = world.resource::<ChronostalkerState>();
        assert!(state.anchored && !state.phased, "anchor clears phase");
        toggle_phase(&mut world).unwrap();
        let state = world.resource::<ChronostalkerState>();
        assert!(state.phased && !state.anchored, "phase clears anchor");
    }

    // --- RED: repossession -------------------------------------------------------

    #[test]
    fn red_overdraw_repossesses() {
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        world.entity_mut(stalker).insert(Health {
            current: 100.0,
            max: 100.0,
            has_rust_lung: false,
        });
        toggle_phase(&mut world).unwrap();
        world.resource_mut::<ChronostalkerState>().debt = DEBT_THRESHOLD;
        chronostalker_tick(&mut world);
        let state = world.resource::<ChronostalkerState>();
        assert!(!state.phased, "repossession forces phase-out");
        assert!(world.get::<Phased>(stalker).is_none());
        assert_eq!(state.repossessions, 1);
        assert!(
            (state.debt - DEBT_REMAINDER).abs() < 1e-6,
            "the ledger keeps a remainder, got {}",
            state.debt
        );
        let health = world.get::<Health>(stalker).unwrap();
        assert!(
            (health.current - (100.0 - REPOSSESS_INJURY)).abs() < 1e-6,
            "reality repossesses from the flesh, got {}",
            health.current
        );
    }

    // --- RED: rewind --------------------------------------------------------------

    #[test]
    fn red_rewind_folds_position_back() {
        let mut world = setup();
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        // Walk five ticks east, recording history.
        for i in 0..5 {
            world
                .entity_mut(stalker)
                .insert(GridPosition { x: 10 + i, y: 10 });
            chronostalker_tick(&mut world);
        }
        let before = debt_of(&world);
        let msg = rewind_time(&mut world, 3).expect("rewind should work");
        assert!(msg.contains("3"), "got: {msg}");
        let pos = world.get::<GridPosition>(stalker).copied().unwrap();
        assert_eq!((pos.x, pos.y), (11, 10), "three ticks back");
        assert!(
            (debt_of(&world) - before - 3.0 * REWIND_DEBT_PER_TICK).abs() < 1e-6,
            "rewind costs debt per tick"
        );
        assert_eq!(world.resource::<ChronostalkerState>().rewinds, 1);
    }

    #[test]
    fn red_rewind_capped() {
        let mut world = setup();
        take_direct(&mut world, 10, 10);
        let err = rewind_time(&mut world, REWIND_CAP + 1).unwrap_err();
        assert!(err.contains("cap") || err.contains("most"), "got: {err}");
    }

    #[test]
    fn red_rewind_shallow_history_refused() {
        let mut world = setup();
        take_direct(&mut world, 10, 10);
        chronostalker_tick(&mut world);
        let err = rewind_time(&mut world, 5).unwrap_err();
        assert!(err.contains("shallow") || err.contains("few"), "got: {err}");
    }

    // --- RED: haunting ---------------------------------------------------------------

    #[test]
    fn red_crisis_sets_haunt_target() {
        let mut world = setup();
        spawn_lander(&mut world, 10, 10);
        take_direct(&mut world, 14, 14);
        world
            .resource_mut::<Events<PirateRaidEvent>>()
            .send(PirateRaidEvent);
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        schedule.add_systems(crisis_chronostalker_bridge);
        schedule.run(&mut world);
        let state = world.resource::<ChronostalkerState>();
        assert_eq!(state.haunts, 1, "the raid draws the stalker");
        assert_eq!(
            state.haunt_target,
            Some((10, 10)),
            "raids pull toward the lander"
        );
    }

    #[test]
    fn red_haunt_arrival_mends_and_repays() {
        let mut world = setup();
        spawn_lander(&mut world, 10, 10);
        let (_pop, stalker) = take_direct(&mut world, 10, 10);
        // A damaged structure near the crisis site.
        let facility = world
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 11, y: 10 },
                Structure {
                    current_hp: 50.0,
                    max_hp: 100.0,
                },
            ))
            .id();
        {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.debt = 30.0;
            state.haunt_target = Some((10, 10));
        }
        // The stalker is already at the site; the tick should mend.
        chronostalker_tick(&mut world);
        let state = world.resource::<ChronostalkerState>();
        assert!(state.haunt_target.is_none(), "arrival clears the target");
        assert_eq!(state.repairs, 1);
        assert!(
            (debt_of(&world) - (30.0 - ERRAND_REPAY)).abs() < 1e-6,
            "the ledger accepts the work, got {}",
            debt_of(&world)
        );
        let structure = world.get::<Structure>(facility).unwrap();
        assert!(
            (structure.current_hp - (50.0 + HAUNT_REPAIR_HP)).abs() < 1e-6,
            "the wound in the world closes a little"
        );
        let _ = stalker;
    }

    #[test]
    fn red_haunt_nudges_unpossessed_stalker() {
        let mut world = setup();
        spawn_lander(&mut world, 10, 10);
        let (pop, stalker) = take_direct(&mut world, 16, 16);
        // Release the stalker: the haunt moves it, not the player.
        world.entity_mut(stalker).remove::<Possessed>();
        {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.haunt_target = Some((10, 10));
        }
        let before = world.get::<GridPosition>(stalker).copied().unwrap();
        chronostalker_tick(&mut world);
        let after = world.get::<GridPosition>(stalker).copied().unwrap();
        let dist_before = (before.x - 10).abs().max((before.y - 10).abs());
        let dist_after = (after.x - 10).abs().max((after.y - 10).abs());
        assert!(
            dist_after < dist_before,
            "the unpossessed stalker drifts toward the crisis"
        );
        let _ = pop;
    }

    #[test]
    fn red_haunt_does_not_steal_possession() {
        let mut world = setup();
        spawn_lander(&mut world, 10, 10);
        let (_pop, stalker) = take_direct(&mut world, 16, 16);
        // Still possessed: the player's hand is on the stick — no drift,
        // no auto-phase. The crisis simply waits.
        {
            let mut state = world.resource_mut::<ChronostalkerState>();
            state.haunt_target = Some((10, 10));
        }
        let before = world.get::<GridPosition>(stalker).copied().unwrap();
        chronostalker_tick(&mut world);
        let after = world.get::<GridPosition>(stalker).copied().unwrap();
        assert_eq!((before.x, before.y), (after.x, after.y));
        assert!(
            world.get::<Phased>(stalker).is_none(),
            "possession must not auto-phase the stalker"
        );
        assert!(
            world
                .resource::<ChronostalkerState>()
                .haunt_target
                .is_some(),
            "the draw persists while possessed"
        );
        // Release: the drift resumes.
        world.entity_mut(stalker).remove::<Possessed>();
        chronostalker_tick(&mut world);
        let resumed = world.get::<GridPosition>(stalker).copied().unwrap();
        let d0 = (after.x - 10).abs().max((after.y - 10).abs());
        let d1 = (resumed.x - 10).abs().max((resumed.y - 10).abs());
        assert!(d1 < d0, "the haunt resumes after release");
    }

    // --- RED: stats ------------------------------------------------------------------

    #[test]
    fn red_debt_stat_none_without_stalker() {
        let mut world = setup();
        assert!(chronostalker_debt(&mut world).is_none());
    }

    #[test]
    fn red_debt_stat_tracks_ledger() {
        let mut world = setup();
        take_direct(&mut world, 10, 10);
        world.resource_mut::<ChronostalkerState>().debt = 12.5;
        assert!((chronostalker_debt(&mut world).unwrap() - 12.5).abs() < 1e-6);
    }

    #[test]
    fn red_describe_stalker_mentions_debt_and_state() {
        let mut world = setup();
        take_direct(&mut world, 10, 10);
        let text = describe_stalker(&mut world);
        assert!(text.contains("Tock"), "got: {text}");
        assert!(text.contains("ledger") || text.contains("debt"), "got: {text}");
    }

    // --- IP guard ---------------------------------------------------------------------

    #[test]
    fn ip_guard_no_banned_terms() {
        // The concept is Simmons-flavored; the names must be original.
        let hay = [
            STALKER_SPAWN_CHRONICLE,
            PHASE_ON_CHRONICLE,
            PHASE_OFF_CHRONICLE,
            ANCHOR_ON_CHRONICLE,
            ANCHOR_BREAK_CHRONICLE,
            REPOSSESS_CHRONICLE,
            REPOSSESS_DETAIL_CHRONICLE,
            HAUNT_CHRONICLE,
            HAUNT_AID_CHRONICLE,
            FIRST_REWIND_CHRONICLE,
            STALKER_NAME,
        ]
        .join(" ")
        .to_lowercase();
        for banned in [
            "shrike",
            "hyperion",
            "tree of pain",
            "keats",
            "ousters",
            "cantos",
            "hegemon",
        ] {
            assert!(!hay.contains(banned), "IP leak: {banned}");
        }
    }
}
