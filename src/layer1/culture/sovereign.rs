//! The Fallen Sovereign — an exiled-monarch adventurer origin.
//!
//! Layers on top of the adventurer-mode spike (`direct_link`): a dented crown
//! spawns near the starter colony. A possessed pop standing adjacent can
//! `interact` to take it up, gaining [`Sovereign`] status plus legitimacy and
//! melancholy meters.
//!
//! While possessed-and-crowned, the headless console gains `decree`,
//! `commission`, `edicts`, and `abdicate`. Decrees cost political capital
//! (a 50-tick cooldown) and resolve with legitimacy judgments; the melancholy
//! engine fires unhinged edicts at thresholds and resets.
//!
//! All names are original. The "exiled monarch" concept is concept-only
//! inspiration — no named characters, places, or distinctive IP anywhere.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;

use crate::layer1::actions::AssignedTo;
use crate::layer1::biology::symbiotic_insurgency::SabotageEvent;
use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::economy::Wallet;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::{ActionType, PopAction, StartPlan};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Chronicle text when the crown changes hands.
pub const CROWN_CHRONICLE: &str = "A dented crown changes hands. The colony holds its breath.";
/// Chronicle text on abdication.
pub const ABDICATION_CHRONICLE: &str = "The crown is laid down. The colony exhales.";
/// Chronicle text when the sovereign's body is found.
pub const SOVEREIGN_DEATH_CHRONICLE: &str =
    "The crown lies in the dust where its bearer fell. It waits.";

/// Shared cooldown (ticks) after any decree.
pub const DECREE_COOLDOWN_TICKS: u32 = 50;
/// How long a labor decree's work-fervor lasts (ticks).
pub const LABOR_DECREE_DURATION: u32 = 100;
/// Work-utility bonus applied to productive actions while a labor decree is active.
pub const LABOR_FERVOR_BONUS: f32 = 0.4;
/// Ticks after a labor decree when food-production is judged.
pub const LABOR_JUDGMENT_DELAY: u32 = 50;

/// Revel: morale boost, duration, food cost.
pub const REVEL_MORALE: f32 = 0.2;
pub const REVEL_DURATION: u32 = 100;
pub const REVEL_FOOD_COST: f32 = 5.0;
/// Ticks after a revel when morale is judged.
pub const REVEL_JUDGMENT_DELAY: u32 = 50;

/// Levy: fraction of each pop's wallet seized.
pub const LEVY_RATE: f32 = 0.2;
/// Legitimacy cost of a levy.
pub const LEVY_LEGITIMACY_COST: f32 = 0.1;

/// Commission: food + metal cost, legitimacy reward.
pub const COMMISSION_FOOD_COST: f32 = 10.0;
pub const COMMISSION_METAL_COST: f32 = 5.0;
pub const COMMISSION_LEGITIMACY: f32 = 0.08;
/// Gallery morale: per artwork and cap.
pub const GALLERY_MORALE_PER_ARTWORK: f32 = 0.15;
pub const GALLERY_MORALE_MAX: f32 = 0.3;

/// Melancholy sources.
pub const MELANCHOLY_PER_DEATH: f32 = 0.15;
pub const MELANCHOLY_PER_SABOTAGE: f32 = 0.1;
pub const MELANCHOLY_ON_STARVATION: f32 = 0.2;
/// Melancholy decay per tick while average morale is high.
pub const MELANCHOLY_DECAY: f32 = 0.01;
pub const MELANCHOLY_DECAY_MORALE_GATE: f32 = 0.6;

/// Edict thresholds.
pub const EDICT_BANISHMENT_THRESHOLD: f32 = 0.5;
pub const EDICT_FEAST_THRESHOLD: f32 = 0.75;
pub const EDICT_SILENCE_THRESHOLD: f32 = 0.9;
/// Melancholy after an edict fires.
pub const MELANCHOLY_AFTER_EDICT: f32 = 0.2;

/// Banishment of Mondays: idle ticks, then the morale wave (value + duration).
pub const BANISHMENT_IDLE_TICKS: u32 = 20;
pub const BANISHMENT_WAVE_MORALE: f32 = 0.3;
pub const BANISHMENT_WAVE_DURATION: u32 = 100;

/// The Grand Feast: fraction of the stockpile consumed, morale, duration.
pub const FEAST_FOOD_FRACTION: f32 = 0.5;
pub const FEAST_MORALE: f32 = 0.5;
pub const FEAST_DURATION: u32 = 200;

/// Crown's Silence: ticks of halved sabotage.
pub const SILENCE_DURATION: u32 = 200;

/// Legitimacy swing for a landed / failed decree or edict.
pub const LEGITIMACY_JUDGMENT: f32 = 0.05;

/// Starting meters on coronation.
pub const STARTING_LEGITIMACY: f32 = 0.5;
pub const STARTING_MELANCHOLY: f32 = 0.3;

/// Morale modifier labels (refreshed each tick, never stacked).
pub const REVEL_MODIFIER_LABEL: &str = "Royal Revel";
pub const GALLERY_MODIFIER_LABEL: &str = "Patron's Gallery";
pub const FEAST_MODIFIER_LABEL: &str = "Grand Feast";
pub const BANISHMENT_WAVE_LABEL: &str = "The Banishment Lifted";

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A dented crown lying on the ground, waiting for a head.
/// (While worn it is despawned; the [`Sovereign`] component marks the bearer.)
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct DentedCrown;

/// Status of the crowned pop: an exiled monarch ruling a colony that never
/// asked for one.
#[derive(Component, Debug, Clone, Copy)]
pub struct Sovereign {
    /// 0.0 (usurper) .. 1.0 (beloved). Starts at 0.5.
    pub legitimacy: f32,
    /// 0.0 (serene) .. 1.0 (unhinged). Starts at 0.3.
    pub melancholy: f32,
}

/// A commissioned artwork, placed on the sovereign's tile.
///
/// (Named `SovereignArtwork` to avoid the unrelated crafting `ArtWork`
/// component in `artists_muse`.)
#[derive(Component, Debug, Clone)]
pub struct SovereignArtwork {
    /// The name the sovereign gave the piece.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Cumulative farm food production, bumped by `produce_food_system`.
/// Used to judge whether a labor decree actually raised production.
#[derive(Resource, Debug, Default, Clone)]
pub struct FarmProductionLedger {
    /// Total food ever produced by farms.
    pub cumulative: f32,
}

/// Work-utility bonus broadcast to the utility AI while a labor decree
/// is active. Read by `utility_ai` when building evaluation contexts.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LaborFervor {
    /// Bonus added to work-type action utilities (0.0 when no decree).
    pub bonus: f32,
}

/// Crown's Silence: grief confuses the mind-spore, halving sabotage.
/// Read by `trigger_symbiont_sabotage_system`.
#[derive(Resource, Debug, Default, Clone)]
pub struct SporeSuppression {
    /// Ticks of suppression remaining.
    pub remaining: u32,
    /// Sabotage windows skipped while active (for the legitimacy judgment).
    pub suppressed_events: u64,
}

/// The three decrees a possessed-and-crowned pop can issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecreeKind {
    /// Pops get +work utility; judged on food-production delta.
    Labor,
    /// +morale at a food cost; judged on whether morale actually rose.
    Revel,
    /// Seize wallets; always costs legitimacy.
    Levy,
}

/// The three melancholy-engine edicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdictKind {
    /// 0.5: the colony idles, then a morale wave.
    BanishmentOfMondays,
    /// 0.75: burn half the stockpile for a great feast.
    GrandFeast,
    /// 0.9: halve mind-spore sabotage — "the spore is confused by grief".
    CrownsSilence,
}

impl EdictKind {
    /// Display name of the edict.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BanishmentOfMondays => "Banishment of Mondays",
            Self::GrandFeast => "The Grand Feast",
            Self::CrownsSilence => "Crown's Silence",
        }
    }

    /// Melancholy threshold that fires this edict.
    #[must_use]
    pub const fn threshold(self) -> f32 {
        match self {
            Self::BanishmentOfMondays => EDICT_BANISHMENT_THRESHOLD,
            Self::GrandFeast => EDICT_FEAST_THRESHOLD,
            Self::CrownsSilence => EDICT_SILENCE_THRESHOLD,
        }
    }
}

/// In-flight labor decree.
#[derive(Debug, Clone)]
pub struct LaborDecreeState {
    /// Ticks of fervor remaining.
    pub remaining: u32,
    /// Ledger value when the decree was issued.
    pub production_mark: f32,
    /// Food-production rate (per tick) over the samples before the decree.
    pub pre_rate: f32,
    /// Ticks until production is judged (u32::MAX once judged).
    pub eval_in: u32,
}

/// In-flight revel decree.
#[derive(Debug, Clone)]
pub struct RevelDecreeState {
    /// Ticks of revelry remaining.
    pub remaining: u32,
    /// Average morale when the revel began.
    pub morale_mark: f32,
    /// Ticks until morale is judged (u32::MAX once judged).
    pub eval_in: u32,
}

/// In-flight Grand Feast edict.
#[derive(Debug, Clone)]
pub struct FeastEdictState {
    /// Ticks of feasting remaining.
    pub remaining: u32,
    /// Average morale when the feast began.
    pub morale_mark: f32,
}

/// In-flight Banishment of Mondays edict.
#[derive(Debug, Clone)]
pub struct BanishmentEdictState {
    /// Ticks of enforced idleness remaining.
    pub idle_remaining: u32,
    /// Ticks of the relief wave remaining (starts when idleness ends).
    pub wave_remaining: u32,
    /// Average morale when the wave began.
    pub morale_mark: f32,
}

/// Machine state for the Fallen Sovereign origin.
#[derive(Resource, Debug, Default)]
pub struct SovereignState {
    /// The crowned pop, if any.
    pub sovereign: Option<Entity>,
    /// Cooldown ticks remaining before another decree.
    pub decree_cooldown: u32,
    /// Last known tile of the sovereign (for dropping the crown on death).
    pub last_known_pos: Option<GridPosition>,
    /// Pop count on the previous tick (death detection).
    pub prev_pop_count: usize,
    /// Whether food was already zero last tick (edge-triggered starvation).
    pub food_was_zero: bool,
    /// Whether the crown-spawn attempt already ran.
    pub crown_spawn_attempted: bool,
    /// Active decree / edict effects.
    pub labor: Option<LaborDecreeState>,
    pub revel: Option<RevelDecreeState>,
    pub feast: Option<FeastEdictState>,
    pub banishment: Option<BanishmentEdictState>,
    /// Whether Crown's Silence is being evaluated (remaining ticks live in
    /// [`SporeSuppression`]).
    pub silence_eval_pending: bool,
    /// Suppressed-event count when the Silence began.
    pub silence_suppressed_mark: u64,
    /// Edicts already fired this reign (each fires once per reign).
    pub fired_edicts: Vec<EdictKind>,
    /// Ring of recent ledger samples (one per tick, capped).
    pub production_samples: VecDeque<f32>,
    /// Melancholy queued by the sabotage bridge (applied next tick).
    pub melancholy_queue: f32,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Make sure all sovereign resources exist.
pub fn ensure_sovereign_resources(world: &mut World) {
    world.init_resource::<SovereignState>();
    world.init_resource::<FarmProductionLedger>();
    world.init_resource::<LaborFervor>();
    world.init_resource::<SporeSuppression>();
    world.init_resource::<Events<AddChronicleEvent>>();
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    world.init_resource::<Events<AddChronicleEvent>>();
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

/// Average morale across all pops (0.0 when there are none).
#[must_use]
pub fn average_morale(world: &mut World) -> f32 {
    let mut total = 0.0;
    let mut count = 0usize;
    for morale in world.query::<&Morale>().iter(world) {
        total += morale.value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// Find the ground crown entity, if one is lying around.
#[must_use]
pub fn find_ground_crown(world: &mut World) -> Option<(Entity, GridPosition)> {
    world
        .query::<(Entity, &DentedCrown, &GridPosition)>()
        .iter(world)
        .map(|(e, _, p)| (e, *p))
        .next()
}

/// The ground crown within crowning reach (Chebyshev distance <= 1) of `pop`.
#[must_use]
pub fn crown_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    find_ground_crown(world).and_then(|(crown, crown_pos)| {
        let dx = (crown_pos.x - pos.x).abs();
        let dy = (crown_pos.y - pos.y).abs();
        (dx <= 1 && dy <= 1).then_some(crown)
    })
}

/// Clamp a meter into 0.0..=1.0.
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn add_melancholy(world: &mut World, delta: f32) {
    let sov = world.resource::<SovereignState>().sovereign;
    if let Some(e) = sov {
        if let Some(mut s) = world.get_mut::<Sovereign>(e) {
            s.melancholy = clamp01(s.melancholy + delta);
        }
    }
}

fn adjust_legitimacy(world: &mut World, delta: f32) {
    let sov = world.resource::<SovereignState>().sovereign;
    if let Some(e) = sov {
        if let Some(mut s) = world.get_mut::<Sovereign>(e) {
            s.legitimacy = clamp01(s.legitimacy + delta);
        }
    }
}

fn reset_melancholy(world: &mut World) {
    let sov = world.resource::<SovereignState>().sovereign;
    if let Some(e) = sov {
        if let Some(mut s) = world.get_mut::<Sovereign>(e) {
            s.melancholy = MELANCHOLY_AFTER_EDICT;
        }
    }
}

/// Production rate (per tick) implied by the sample ring.
fn production_rate(samples: &VecDeque<f32>) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let span = samples.len() as f32;
    ((samples.back().copied().unwrap_or(0.0) - samples.front().copied().unwrap_or(0.0)) / span)
        .max(0.0)
}

fn set_decree_cooldown(world: &mut World) {
    world.resource_mut::<SovereignState>().decree_cooldown = DECREE_COOLDOWN_TICKS;
}

fn require_sovereign(world: &mut World, pop: Entity) -> Result<(), String> {
    ensure_sovereign_resources(world);
    let crowned = world.resource::<SovereignState>().sovereign == Some(pop)
        && world.get::<Sovereign>(pop).is_some();
    if crowned {
        Ok(())
    } else {
        Err("Only the crowned sovereign may do that — possess them first.".to_string())
    }
}

/// Clear decree/edict effects at the end of a reign (abdication or death).
fn clear_reign_effects(world: &mut World) {
    world.resource_mut::<LaborFervor>().bonus = 0.0;
    world.resource_mut::<SporeSuppression>().remaining = 0;
    let mut state = world.resource_mut::<SovereignState>();
    state.labor = None;
    state.revel = None;
    state.feast = None;
    state.banishment = None;
    state.silence_eval_pending = false;
    state.decree_cooldown = 0;
    state.melancholy_queue = 0.0;
}

// ---------------------------------------------------------------------------
// Crown lifecycle
// ---------------------------------------------------------------------------

/// Spawn the dented crown near the starter colony's lander, once.
///
/// Picks the first walkable, unoccupied, building-free ground tile in an
/// expanding ring around the lander (never inside an obstacle tile).
pub fn spawn_dented_crown_once(world: &mut World) {
    ensure_sovereign_resources(world);
    {
        let state = world.resource::<SovereignState>();
        if state.crown_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<SovereignState>().crown_spawn_attempted = true;

    // Only one crown per colony: skip if one lies around or is worn.
    if find_ground_crown(world).is_some() {
        return;
    }
    if world.query::<&Sovereign>().iter(world).next().is_some() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    // Snapshot the tiles we might consider into owned sets first: the
    // building query below needs `&mut world`, which conflicts with live
    // resource borrows.
    let (walkable, occupied, buildings) = {
        let mut walkable = std::collections::HashSet::new();
        if let Some(terrain) = world.get_resource::<TerrainGrid>() {
            for dx in -8..=8i32 {
                for dy in -8..=8i32 {
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
        'search: for radius in 1..=8i32 {
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

    if let Some(pos) = target {
        world.spawn((DentedCrown, pos));
        send_chronicle(
            world,
            &format!(
                "Rumor: a dented crown glints in the dust at ({}, {}), near the lander.",
                pos.x, pos.y
            ),
            EventImportance::Minor,
        );
    }
}

/// Crown `pop`: consumes the adjacent ground crown and grants sovereignty.
pub fn try_crown_pop(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_sovereign_resources(world);
    if world.get::<Sovereign>(pop).is_some() {
        return Err("Already crowned.".to_string());
    }
    let crown = crown_within_reach(world, pop).ok_or_else(|| {
        "No crown within reach. Stand beside the dented crown and try again.".to_string()
    })?;
    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));
    world.despawn(crown);
    world.entity_mut(pop).insert(Sovereign {
        legitimacy: STARTING_LEGITIMACY,
        melancholy: STARTING_MELANCHOLY,
    });
    let pos = world.get::<GridPosition>(pop).copied();
    {
        let mut state = world.resource_mut::<SovereignState>();
        state.sovereign = Some(pop);
        state.fired_edicts.clear();
        state.last_known_pos = pos;
    }
    send_chronicle(world, CROWN_CHRONICLE, EventImportance::Major);
    Ok(format!("{name} lifts the dented crown. {CROWN_CHRONICLE}"))
}

/// Abdicate: drop the crown on the current tile, clear sovereignty and reign effects.
pub fn abdicate(world: &mut World, pop: Entity) -> Result<String, String> {
    require_sovereign(world, pop)?;
    let pos = world
        .get::<GridPosition>(pop)
        .copied()
        .unwrap_or(GridPosition { x: 0, y: 0 });
    world.entity_mut(pop).remove::<Sovereign>();
    world.spawn((DentedCrown, pos));
    clear_reign_effects(world);
    {
        let mut state = world.resource_mut::<SovereignState>();
        state.sovereign = None;
        state.last_known_pos = None;
        state.fired_edicts.clear();
    }
    send_chronicle(world, ABDICATION_CHRONICLE, EventImportance::Major);
    Ok("The crown is laid down. The colony exhales.".to_string())
}

fn handle_sovereign_death(world: &mut World) {
    let pos = world
        .resource::<SovereignState>()
        .last_known_pos
        .unwrap_or(GridPosition { x: 0, y: 0 });
    world.spawn((DentedCrown, pos));
    clear_reign_effects(world);
    {
        let mut state = world.resource_mut::<SovereignState>();
        state.sovereign = None;
        state.fired_edicts.clear();
    }
    send_chronicle(world, SOVEREIGN_DEATH_CHRONICLE, EventImportance::Major);
}

// ---------------------------------------------------------------------------
// Decrees & patronage
// ---------------------------------------------------------------------------

/// Issue a decree as the possessed-and-crowned sovereign.
pub fn issue_decree(world: &mut World, pop: Entity, kind: DecreeKind) -> Result<String, String> {
    require_sovereign(world, pop)?;
    let cooldown = world.resource::<SovereignState>().decree_cooldown;
    if cooldown > 0 {
        return Err(format!(
            "The court is still catching its breath ({cooldown} ticks)."
        ));
    }
    match kind {
        DecreeKind::Labor => {
            let cumulative = world.resource::<FarmProductionLedger>().cumulative;
            let pre = production_rate(&world.resource::<SovereignState>().production_samples);
            world.resource_mut::<LaborFervor>().bonus = LABOR_FERVOR_BONUS;
            world.resource_mut::<SovereignState>().labor = Some(LaborDecreeState {
                remaining: LABOR_DECREE_DURATION,
                production_mark: cumulative,
                pre_rate: pre,
                eval_in: LABOR_JUDGMENT_DELAY,
            });
            send_chronicle(
                world,
                "The sovereign decrees a season of labor. Hoes rise across the fields.",
                EventImportance::Standard,
            );
            set_decree_cooldown(world);
            Ok(format!(
                "Decree of Labor proclaimed: +{LABOR_FERVOR_BONUS} work utility for {LABOR_DECREE_DURATION} ticks."
            ))
        }
        DecreeKind::Revel => {
            let food = world.resource::<ColonyResources>().food;
            if food < REVEL_FOOD_COST {
                return Err(format!(
                    "A revel costs {REVEL_FOOD_COST} food; the stockpile holds {food:.1}."
                ));
            }
            world.resource_mut::<ColonyResources>().food -= REVEL_FOOD_COST;
            let mark = average_morale(world);
            world.resource_mut::<SovereignState>().revel = Some(RevelDecreeState {
                remaining: REVEL_DURATION,
                morale_mark: mark,
                eval_in: REVEL_JUDGMENT_DELAY,
            });
            send_chronicle(
                world,
                "The sovereign decrees a revel. Lanterns bloom over the habitat.",
                EventImportance::Standard,
            );
            set_decree_cooldown(world);
            Ok(format!(
                "Decree of Revel proclaimed: +{REVEL_MORALE} morale for {REVEL_DURATION} ticks (-{REVEL_FOOD_COST} food)."
            ))
        }
        DecreeKind::Levy => {
            let targets: Vec<Entity> = world
                .query_filtered::<Entity, (With<Pop>, With<Wallet>)>()
                .iter(world)
                .collect();
            let mut seized = 0.0;
            for t in targets {
                if t == pop {
                    continue;
                }
                if let Some(mut w) = world.get_mut::<Wallet>(t) {
                    let take = w.credits * LEVY_RATE;
                    w.credits -= take;
                    seized += take;
                }
            }
            if let Some(mut w) = world.get_mut::<Wallet>(pop) {
                w.credits += seized;
            }
            adjust_legitimacy(world, -LEVY_LEGITIMACY_COST);
            send_chronicle(
                world,
                "The sovereign's tax collectors make their rounds. Grumbling in the habitat.",
                EventImportance::Standard,
            );
            set_decree_cooldown(world);
            Ok(format!(
                "Levy collected: {seized:.1} credits seized into the sovereign's purse (-{LEVY_LEGITIMACY_COST} legitimacy)."
            ))
        }
    }
}

/// Commission an artwork: patron-of-the-arts on the sovereign's tile.
pub fn commission_artwork(world: &mut World, pop: Entity, name: &str) -> Result<String, String> {
    require_sovereign(world, pop)?;
    let name = name.trim();
    if name.is_empty() {
        return Err("Usage: commission <name> — the piece needs a title.".to_string());
    }
    {
        let res = world.resource::<ColonyResources>();
        if res.food < COMMISSION_FOOD_COST || res.metal < COMMISSION_METAL_COST {
            return Err(format!(
                "A commission costs {COMMISSION_FOOD_COST} food and {COMMISSION_METAL_COST} metal."
            ));
        }
    }
    {
        let mut res = world.resource_mut::<ColonyResources>();
        res.food -= COMMISSION_FOOD_COST;
        res.metal -= COMMISSION_METAL_COST;
    }
    let pos = world
        .get::<GridPosition>(pop)
        .copied()
        .unwrap_or(GridPosition { x: 0, y: 0 });
    world.spawn((
        SovereignArtwork {
            name: name.to_string(),
        },
        pos,
    ));
    adjust_legitimacy(world, COMMISSION_LEGITIMACY);
    send_chronicle(
        world,
        &format!("The court unveils '{name}', patronage of the sovereign."),
        EventImportance::Standard,
    );
    Ok(format!(
        "'{name}' is unveiled on the sovereign's tile (+{COMMISSION_LEGITIMACY} legitimacy)."
    ))
}

/// Human-readable list of the melancholy-engine edicts and their state.
pub fn describe_edicts(world: &mut World) -> String {
    ensure_sovereign_resources(world);
    let mel = match world.resource::<SovereignState>().sovereign {
        Some(e) => world.get::<Sovereign>(e).map(|s| s.melancholy),
        None => None,
    };
    let Some(mel) = mel else {
        return "No sovereign wears the crown. The melancholy engine is dormant.".to_string();
    };
    let fired = world.resource::<SovereignState>().fired_edicts.clone();
    let mut out = format!("Melancholy engine — current melancholy {mel:.2}\n");
    for kind in [
        EdictKind::BanishmentOfMondays,
        EdictKind::GrandFeast,
        EdictKind::CrownsSilence,
    ] {
        let status = if fired.contains(&kind) {
            "FIRED".to_string()
        } else {
            format!("READY at {:.2}", kind.threshold())
        };
        let desc = match kind {
            EdictKind::BanishmentOfMondays => "all pops Idle 20 ticks, then a +0.3 morale wave",
            EdictKind::GrandFeast => "burn 50% of the stockpile, +0.5 morale for 200 ticks",
            EdictKind::CrownsSilence => "halve mind-spore sabotage for 200 ticks",
        };
        out.push_str(&format!(
            "- {} (melancholy {:.2}): {desc} [{status}]\n",
            kind.name(),
            kind.threshold()
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Per-tick machinery
// ---------------------------------------------------------------------------

/// Force every pop into Idle (Banishment of Mondays).
fn force_colony_idle(world: &mut World) {
    let pops: Vec<Entity> = world.query_filtered::<Entity, With<Pop>>().iter(world).collect();
    for e in pops {
        if let Some(mut action) = world.get_mut::<PopAction>(e) {
            action.current = ActionType::Idle;
            action.current_utility = 0.0;
            action.ticks_committed = 0;
        }
        let mut em = world.entity_mut(e);
        em.remove::<MovementTarget>();
        em.remove::<AssignedTo>();
        em.remove::<StartPlan>();
        em.remove::<AtTarget>();
    }
}

fn fire_banishment(world: &mut World) {
    world.resource_mut::<SovereignState>().banishment = Some(BanishmentEdictState {
        idle_remaining: BANISHMENT_IDLE_TICKS,
        wave_remaining: 0,
        morale_mark: 0.0,
    });
    world
        .resource_mut::<SovereignState>()
        .fired_edicts
        .push(EdictKind::BanishmentOfMondays);
    reset_melancholy(world);
    send_chronicle(
        world,
        "By royal edict, Mondays are banished. The colony idles in confused relief.",
        EventImportance::Major,
    );
}

fn fire_feast(world: &mut World) {
    let consumed = {
        let mut res = world.resource_mut::<ColonyResources>();
        let c = res.food * FEAST_FOOD_FRACTION;
        res.food -= c;
        c
    };
    let mark = average_morale(world);
    world.resource_mut::<SovereignState>().feast = Some(FeastEdictState {
        remaining: FEAST_DURATION,
        morale_mark: mark,
    });
    world
        .resource_mut::<SovereignState>()
        .fired_edicts
        .push(EdictKind::GrandFeast);
    reset_melancholy(world);
    send_chronicle(
        world,
        &format!(
            "The Grand Feast consumes {consumed:.1} food. Tables groan; the sovereign weeps into the gravy."
        ),
        EventImportance::Major,
    );
}

fn fire_silence(world: &mut World) {
    {
        let mut sup = world.resource_mut::<SporeSuppression>();
        sup.remaining = SILENCE_DURATION;
        sup.suppressed_events = 0;
    }
    {
        let mut state = world.resource_mut::<SovereignState>();
        state.silence_eval_pending = true;
        state.silence_suppressed_mark = 0;
        state.fired_edicts.push(EdictKind::CrownsSilence);
    }
    reset_melancholy(world);
    send_chronicle(
        world,
        "The Crown's Silence falls over the colony. The spore is confused by grief.",
        EventImportance::Major,
    );
}

fn judge_legitimacy(world: &mut World, landed: bool, what: &str) {
    adjust_legitimacy(
        world,
        if landed {
            LEGITIMACY_JUDGMENT
        } else {
            -LEGITIMACY_JUDGMENT
        },
    );
    send_chronicle(
        world,
        &format!(
            "{what}: {} (legitimacy {}).",
            if landed {
                "the court approves"
            } else {
                "the court mutters"
            },
            if landed { "+0.05" } else { "-0.05" },
        ),
        EventImportance::Standard,
    );
}

fn tick_labor(world: &mut World) {
    let cumulative = world.resource::<FarmProductionLedger>().cumulative;
    let (eval_due, expired, mark, pre) = {
        let mut state = world.resource_mut::<SovereignState>();
        let Some(labor) = state.labor.as_mut() else {
            return;
        };
        if labor.remaining > 0 {
            labor.remaining -= 1;
        }
        if labor.eval_in > 0 {
            labor.eval_in -= 1;
        }
        let eval_due = labor.eval_in == 0;
        let expired = labor.remaining == 0;
        let (mark, pre) = (labor.production_mark, labor.pre_rate);
        if eval_due {
            // Judge once.
            labor.eval_in = u32::MAX;
        }
        if expired {
            state.labor = None;
        }
        (eval_due, expired, mark, pre)
    };
    if expired {
        world.resource_mut::<LaborFervor>().bonus = 0.0;
    }
    if eval_due {
        let post = (cumulative - mark) / LABOR_JUDGMENT_DELAY as f32;
        judge_legitimacy(
            world,
            post > pre,
            &format!("The season of labor is judged: {post:.3}/tick vs {pre:.3}/tick before"),
        );
    }
}

fn tick_revel(world: &mut World) {
    let avg_now = average_morale(world);
    let (eval_due, mark) = {
        let mut state = world.resource_mut::<SovereignState>();
        let Some(revel) = state.revel.as_mut() else {
            return;
        };
        if revel.remaining > 0 {
            revel.remaining -= 1;
        }
        if revel.eval_in > 0 {
            revel.eval_in -= 1;
        }
        let eval_due = revel.eval_in == 0;
        let expired = revel.remaining == 0;
        let mark = revel.morale_mark;
        if eval_due {
            revel.eval_in = u32::MAX;
        }
        if expired {
            state.revel = None;
        }
        (eval_due, mark)
    };
    if eval_due {
        judge_legitimacy(
            world,
            avg_now > mark,
            &format!("The revel is judged: morale {avg_now:.2} vs {mark:.2} before"),
        );
    }
}

fn tick_feast(world: &mut World) {
    let avg_now = average_morale(world);
    let (done, mark) = {
        let mut state = world.resource_mut::<SovereignState>();
        let Some(feast) = state.feast.as_mut() else {
            return;
        };
        if feast.remaining > 0 {
            feast.remaining -= 1;
        }
        let done = feast.remaining == 0;
        let mark = feast.morale_mark;
        if done {
            state.feast = None;
        }
        (done, mark)
    };
    if done {
        let landed = avg_now > mark;
        judge_legitimacy(
            world,
            landed,
            &format!("The Grand Feast ends: morale {avg_now:.2} vs {mark:.2} before"),
        );
    }
}

fn tick_banishment(world: &mut World) {
    enum Phase {
        Idling,
        WaveStarted,
        Waving,
        WaveEnded(f32),
    }
    let avg_now = average_morale(world);
    let phase = {
        let mut state = world.resource_mut::<SovereignState>();
        let Some(b) = state.banishment.as_mut() else {
            return;
        };
        if b.idle_remaining > 0 {
            b.idle_remaining -= 1;
            if b.idle_remaining == 0 {
                b.wave_remaining = BANISHMENT_WAVE_DURATION;
                b.morale_mark = avg_now;
                Phase::WaveStarted
            } else {
                Phase::Idling
            }
        } else if b.wave_remaining > 0 {
            b.wave_remaining -= 1;
            let mark = b.morale_mark;
            if b.wave_remaining == 0 {
                state.banishment = None;
                Phase::WaveEnded(mark)
            } else {
                Phase::Waving
            }
        } else {
            state.banishment = None;
            Phase::WaveEnded(0.0)
        }
    };
    match phase {
        Phase::WaveStarted => send_chronicle(
            world,
            "The banishment lifts — a wave of giddy relief washes over the colony (+0.3 morale).",
            EventImportance::Major,
        ),
        Phase::WaveEnded(mark) => {
            judge_legitimacy(
                world,
                avg_now > mark,
                &format!("The relief wave fades: morale {avg_now:.2} vs {mark:.2} at the lifting"),
            );
        }
        Phase::Idling | Phase::Waving => {}
    }
}

fn tick_silence(world: &mut World) {
    if !world.resource::<SovereignState>().silence_eval_pending {
        return;
    }
    let (done, suppressed) = {
        let mut sup = world.resource_mut::<SporeSuppression>();
        if sup.remaining > 0 {
            sup.remaining -= 1;
        }
        (sup.remaining == 0, sup.suppressed_events)
    };
    if done {
        let mark = world.resource::<SovereignState>().silence_suppressed_mark;
        world
            .resource_mut::<SovereignState>()
            .silence_eval_pending = false;
        judge_legitimacy(
            world,
            suppressed > mark,
            &format!("The Crown's Silence lifts: {suppressed} sabotage windows passed quietly"),
        );
    }
}

/// Per-tick sovereign bookkeeping: timers, melancholy, edicts, death.
pub fn sovereign_tick(world: &mut World) {
    ensure_sovereign_resources(world);

    // Sample farm production for the labor-decree baseline.
    {
        let cumulative = world.resource::<FarmProductionLedger>().cumulative;
        let mut state = world.resource_mut::<SovereignState>();
        state.production_samples.push_back(cumulative);
        while state.production_samples.len() > 61 {
            state.production_samples.pop_front();
        }
        if state.decree_cooldown > 0 {
            state.decree_cooldown -= 1;
        }
        // Queued melancholy from the sabotage bridge.
        let queued = state.melancholy_queue;
        state.melancholy_queue = 0.0;
        // NLL ends the `state` borrow here.
        if queued != 0.0 {
            add_melancholy(world, queued);
        }
    }

    // Sovereign liveness: death drops the crown where they fell.
    let sov_alive = {
        let sov = world.resource::<SovereignState>().sovereign;
        match sov {
            Some(e) => {
                if world.get_entity(e).is_ok() {
                    if let Some(pos) = world.get::<GridPosition>(e).copied() {
                        world.resource_mut::<SovereignState>().last_known_pos = Some(pos);
                    }
                    true
                } else {
                    handle_sovereign_death(world);
                    false
                }
            }
            None => false,
        }
    };

    // Pop-death detection: each death of another pop weighs on the crown.
    {
        let count = world.query::<&Pop>().iter(world).count();
        let prev = world.resource::<SovereignState>().prev_pop_count;
        world.resource_mut::<SovereignState>().prev_pop_count = count;
        if count < prev && sov_alive {
            #[allow(clippy::cast_precision_loss)]
            let deaths = (prev - count) as f32;
            add_melancholy(world, MELANCHOLY_PER_DEATH * deaths);
        }
    }

    // Starvation edge: food hitting zero.
    {
        let food = world.resource::<ColonyResources>().food;
        let was_zero = world.resource::<SovereignState>().food_was_zero;
        world.resource_mut::<SovereignState>().food_was_zero = food <= 0.0;
        if food <= 0.0 && !was_zero && sov_alive {
            add_melancholy(world, MELANCHOLY_ON_STARVATION);
            send_chronicle(
                world,
                "The stockpile stands empty. The sovereign stares at nothing for a long time.",
                EventImportance::Standard,
            );
        }
    }

    // Melancholy decays while the colony is genuinely content.
    if sov_alive && average_morale(world) > MELANCHOLY_DECAY_MORALE_GATE {
        add_melancholy(world, -MELANCHOLY_DECAY);
    }

    // Decree / edict timers.
    tick_labor(world);
    tick_revel(world);
    tick_feast(world);
    tick_banishment(world);
    tick_silence(world);

    // Melancholy-engine thresholds (highest first; each fires once per reign).
    if sov_alive {
        let mel = world
            .resource::<SovereignState>()
            .sovereign
            .and_then(|e| world.get::<Sovereign>(e))
            .map_or(0.0, |s| s.melancholy);
        let fired = world.resource::<SovereignState>().fired_edicts.clone();
        if mel >= EDICT_SILENCE_THRESHOLD && !fired.contains(&EdictKind::CrownsSilence) {
            fire_silence(world);
        } else if mel >= EDICT_FEAST_THRESHOLD && !fired.contains(&EdictKind::GrandFeast) {
            fire_feast(world);
        } else if mel >= EDICT_BANISHMENT_THRESHOLD
            && !fired.contains(&EdictKind::BanishmentOfMondays)
        {
            fire_banishment(world);
        }
    }

    // Banishment enforcement: the colony idles, by decree.
    {
        let idle = world
            .resource::<SovereignState>()
            .banishment
            .as_ref()
            .map_or(0, |b| b.idle_remaining);
        if idle > 0 {
            force_colony_idle(world);
        }
    }
}

/// Refresh the sovereign's morale modifiers (revel, gallery, feast, wave).
/// Runs before the morale cache update, like the constellation broadcaster.
pub fn apply_sovereign_morale(world: &mut World) {
    ensure_sovereign_resources(world);
    let (revel_on, feast_on, wave_on) = {
        let state = world.resource::<SovereignState>();
        (
            state.revel.is_some(),
            state.feast.is_some(),
            state.banishment.as_ref().is_some_and(|b| b.wave_remaining > 0),
        )
    };
    let artworks = world.query::<&SovereignArtwork>().iter(world).count();
    #[allow(clippy::cast_precision_loss)]
    let gallery_value =
        (artworks as f32 * GALLERY_MORALE_PER_ARTWORK).min(GALLERY_MORALE_MAX);

    let mut query = world.query_filtered::<&mut Morale, With<Pop>>();
    for mut morale in query.iter_mut(world) {
        morale.modifiers.retain(|m| {
            m.label != REVEL_MODIFIER_LABEL
                && m.label != GALLERY_MODIFIER_LABEL
                && m.label != FEAST_MODIFIER_LABEL
                && m.label != BANISHMENT_WAVE_LABEL
        });
        if revel_on {
            morale.add_modifier(MoodModifier {
                label: REVEL_MODIFIER_LABEL.to_string(),
                value: REVEL_MORALE,
                duration: 2,
            });
        }
        if feast_on {
            morale.add_modifier(MoodModifier {
                label: FEAST_MODIFIER_LABEL.to_string(),
                value: FEAST_MORALE,
                duration: 2,
            });
        }
        if wave_on {
            morale.add_modifier(MoodModifier {
                label: BANISHMENT_WAVE_LABEL.to_string(),
                value: BANISHMENT_WAVE_MORALE,
                duration: 2,
            });
        }
        if gallery_value > 0.0 {
            morale.add_modifier(MoodModifier {
                label: GALLERY_MODIFIER_LABEL.to_string(),
                value: gallery_value,
                duration: 2,
            });
        }
    }
}

/// Bridge: sabotage events weigh on the crown (+melancholy each).
pub fn sabotage_melancholy_bridge(
    mut events: EventReader<SabotageEvent>,
    mut state: ResMut<SovereignState>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut hits = 0u32;
    for _ in events.read() {
        hits += 1;
    }
    if hits == 0 || state.sovereign.is_none() {
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let delta = MELANCHOLY_PER_SABOTAGE * hits as f32;
    state.melancholy_queue += delta;
    chronicle.send(AddChronicleEvent {
        text: "News of sabotage reaches the court. The crown grows heavier.".to_string(),
        importance: EventImportance::Minor,
    });
}

/// Parseable sovereign fields for the headless STATS line.
#[must_use]
pub fn sovereign_stats(world: &mut World) -> (Option<u32>, f32, f32) {
    ensure_sovereign_resources(world);
    match world.resource::<SovereignState>().sovereign {
        Some(e) => match world.get::<Sovereign>(e) {
            Some(s) => (Some(e.index()), s.legitimacy, s.melancholy),
            None => (None, 0.0, 0.0),
        },
        None => (None, 0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::time::SimulationTime;

    // --- test scaffolding -------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(SovereignState::default());
        world.insert_resource(FarmProductionLedger::default());
        world.insert_resource(LaborFervor::default());
        world.insert_resource(SporeSuppression::default());
        world
    }

    fn spawn_pop(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Pop,
                PopName("Testy".to_string()),
                GridPosition { x, y },
                Morale::default(),
                Wallet::default(),
                PopAction::default(),
            ))
            .id()
    }

    fn spawn_crown(world: &mut World, x: i32, y: i32) -> Entity {
        world.spawn((DentedCrown, GridPosition { x, y })).id()
    }

    fn drain_chronicle(world: &mut World) -> Vec<String> {
        let texts: Vec<String> = world
            .resource::<Events<AddChronicleEvent>>()
            .iter_current_update_events()
            .map(|e| e.text.clone())
            .collect();
        world.resource_mut::<Events<AddChronicleEvent>>().clear();
        texts
    }

    // --- crown lifecycle ----------------------------------------------------

    #[test]
    fn test_crowning_grants_sovereign_and_meters() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        let crown = spawn_crown(&mut world, 11, 10);

        let msg = try_crown_pop(&mut world, pop).expect("crowning should succeed");

        assert!(world.get_entity(crown).is_err(), "ground crown is consumed");
        let sov = world.get::<Sovereign>(pop).expect("pop should be sovereign");
        assert!((sov.legitimacy - STARTING_LEGITIMACY).abs() < f32::EPSILON);
        assert!((sov.melancholy - STARTING_MELANCHOLY).abs() < f32::EPSILON);
        assert_eq!(world.resource::<SovereignState>().sovereign, Some(pop));
        let log = drain_chronicle(&mut world);
        assert!(
            log.iter().any(|t| t == CROWN_CHRONICLE),
            "crowning chronicle should fire, got: {log:?}"
        );
        assert!(msg.contains("crown"), "result message should mention the crown");
    }

    #[test]
    fn test_crowning_requires_adjacency() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        spawn_crown(&mut world, 30, 30);

        let err = try_crown_pop(&mut world, pop).expect_err("far crown should fail");
        assert!(world.get::<Sovereign>(pop).is_none());
        assert!(!err.is_empty());
    }

    #[test]
    fn test_crowning_requires_ground_crown() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);

        try_crown_pop(&mut world, pop).expect_err("no crown should fail");
        assert!(world.get::<Sovereign>(pop).is_none());
    }

    #[test]
    fn test_abdicate_drops_crown_and_clears_status() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 10, 10);
        spawn_crown(&mut world, 10, 11);
        try_crown_pop(&mut world, pop).unwrap();
        drain_chronicle(&mut world);

        abdicate(&mut world, pop).expect("abdication should succeed");

        assert!(world.get::<Sovereign>(pop).is_none(), "status cleared");
        assert!(world.resource::<SovereignState>().sovereign.is_none());
        let (crown, pos) = find_ground_crown(&mut world).expect("crown on the ground");
        assert!(world.get_entity(crown).is_ok());
        assert_eq!((pos.x, pos.y), (10, 10), "crown drops on the pop's tile");
        let log = drain_chronicle(&mut world);
        assert!(log.iter().any(|t| t == ABDICATION_CHRONICLE));
    }

    #[test]
    fn test_sovereign_death_drops_crown() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 7, 7);
        spawn_crown(&mut world, 7, 8);
        try_crown_pop(&mut world, pop).unwrap();
        // One tick so the state records the sovereign's position.
        sovereign_tick(&mut world);
        drain_chronicle(&mut world);

        world.despawn(pop);
        sovereign_tick(&mut world);

        let state = world.resource::<SovereignState>();
        assert!(state.sovereign.is_none(), "dead sovereign is cleared");
        let (_, pos) = find_ground_crown(&mut world).expect("crown dropped");
        assert_eq!((pos.x, pos.y), (7, 7));
        let log = drain_chronicle(&mut world);
        assert!(log.iter().any(|t| t == SOVEREIGN_DEATH_CHRONICLE));
    }

    // --- decrees ------------------------------------------------------------

    fn crowned(world: &mut World, x: i32, y: i32) -> Entity {
        let pop = spawn_pop(world, x, y);
        spawn_crown(world, x + 1, y);
        try_crown_pop(world, pop).unwrap();
        drain_chronicle(world);
        pop
    }

    #[test]
    fn test_labor_decree_sets_fervor_and_cooldown() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);

        issue_decree(&mut world, pop, DecreeKind::Labor).expect("labor decree");

        assert!(
            (world.resource::<LaborFervor>().bonus - LABOR_FERVOR_BONUS).abs() < f32::EPSILON
        );
        assert_eq!(
            world.resource::<SovereignState>().decree_cooldown,
            DECREE_COOLDOWN_TICKS
        );
        // Second decree blocked by the cooldown.
        issue_decree(&mut world, pop, DecreeKind::Revel).expect_err("cooldown should block");
    }

    #[test]
    fn test_decree_requires_sovereign() {
        let mut world = setup();
        let pop = spawn_pop(&mut world, 5, 5);

        issue_decree(&mut world, pop, DecreeKind::Labor).expect_err("uncrowned pop");
    }

    #[test]
    fn test_labor_fervor_expires() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        issue_decree(&mut world, pop, DecreeKind::Labor).unwrap();

        for _ in 0..LABOR_DECREE_DURATION {
            sovereign_tick(&mut world);
        }

        assert_eq!(
            world.resource::<LaborFervor>().bonus,
            0.0,
            "fervor should expire"
        );
        assert!(world.resource::<SovereignState>().labor.is_none());
    }

    #[test]
    fn test_labor_judgment_rewards_rising_production() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        // Pre-decree: flat production (all samples equal).
        for _ in 0..55 {
            sovereign_tick(&mut world);
        }
        issue_decree(&mut world, pop, DecreeKind::Labor).unwrap();
        // Post-decree: production climbs.
        for i in 0..LABOR_JUDGMENT_DELAY {
            world.resource_mut::<FarmProductionLedger>().cumulative += i as f32;
            sovereign_tick(&mut world);
        }

        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!(
            (sov.legitimacy - (STARTING_LEGITIMACY + LEGITIMACY_JUDGMENT)).abs() < 1e-6,
            "rising production should reward legitimacy, got {}",
            sov.legitimacy
        );
    }

    #[test]
    fn test_labor_judgment_punishes_stagnation() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        // Pre-decree: production climbs fast...
        for i in 0..55 {
            world.resource_mut::<FarmProductionLedger>().cumulative += i as f32;
            sovereign_tick(&mut world);
        }
        let before = world.get::<Sovereign>(pop).unwrap().legitimacy;
        issue_decree(&mut world, pop, DecreeKind::Labor).unwrap();
        // ...then flatlines after the decree.
        for _ in 0..LABOR_JUDGMENT_DELAY {
            sovereign_tick(&mut world);
        }

        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!(
            (sov.legitimacy - (before - LEGITIMACY_JUDGMENT)).abs() < 1e-6,
            "stagnant production should cost legitimacy, got {}",
            sov.legitimacy
        );
    }

    #[test]
    fn test_revel_costs_food_and_boosts_morale() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        world.resource_mut::<ColonyResources>().food = 10.0;

        issue_decree(&mut world, pop, DecreeKind::Revel).expect("revel");

        assert!((world.resource::<ColonyResources>().food - 5.0).abs() < f32::EPSILON);
        apply_sovereign_morale(&mut world);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == REVEL_MODIFIER_LABEL
                    && (m.value - REVEL_MORALE).abs() < f32::EPSILON),
            "revel modifier should be applied"
        );
    }

    #[test]
    fn test_revel_requires_food() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        world.resource_mut::<ColonyResources>().food = 2.0;

        issue_decree(&mut world, pop, DecreeKind::Revel).expect_err("too poor to revel");
        assert_eq!(
            world.resource::<SovereignState>().decree_cooldown,
            0,
            "failed decree keeps no cooldown"
        );
    }

    #[test]
    fn test_levy_seizes_wallets_and_costs_legitimacy() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        let p2 = spawn_pop(&mut world, 6, 5);
        let p3 = spawn_pop(&mut world, 5, 6);
        world.get_mut::<Wallet>(p2).unwrap().credits = 100.0;
        world.get_mut::<Wallet>(p3).unwrap().credits = 50.0;
        world.get_mut::<Wallet>(pop).unwrap().credits = 10.0;

        issue_decree(&mut world, pop, DecreeKind::Levy).expect("levy");

        // 20% of 100 + 20% of 50 seized into the sovereign's wallet.
        assert!((world.get::<Wallet>(pop).unwrap().credits - 40.0).abs() < f32::EPSILON);
        assert!((world.get::<Wallet>(p2).unwrap().credits - 80.0).abs() < f32::EPSILON);
        assert!((world.get::<Wallet>(p3).unwrap().credits - 40.0).abs() < f32::EPSILON);
        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!((sov.legitimacy - (STARTING_LEGITIMACY - LEVY_LEGITIMACY_COST)).abs() < 1e-6);
        let log = drain_chronicle(&mut world);
        assert!(log.iter().any(|t| t.contains("tax") || t.contains("Grumbling")));
    }

    // --- patronage ------------------------------------------------------------

    #[test]
    fn test_commission_creates_artwork_and_rewards_legitimacy() {
        let mut world = setup();
        let pop = crowned(&mut world, 3, 4);
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.food = 20.0;
            res.metal = 10.0;
        }

        commission_artwork(&mut world, pop, "Ode to Rust").expect("commission");

        let res = world.resource::<ColonyResources>();
        assert!((res.food - 10.0).abs() < f32::EPSILON);
        assert!((res.metal - 5.0).abs() < f32::EPSILON);
        let artworks: Vec<(Entity, SovereignArtwork, GridPosition)> = world
            .query::<(Entity, &SovereignArtwork, &GridPosition)>()
            .iter(&world)
            .map(|(e, a, p)| (e, a.clone(), *p))
            .collect();
        assert_eq!(artworks.len(), 1);
        assert_eq!(artworks[0].1.name, "Ode to Rust");
        assert_eq!((artworks[0].2.x, artworks[0].2.y), (3, 4));
        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!((sov.legitimacy - (STARTING_LEGITIMACY + COMMISSION_LEGITIMACY)).abs() < 1e-6);
    }

    #[test]
    fn test_commission_requires_resources() {
        let mut world = setup();
        let pop = crowned(&mut world, 3, 4);
        world.resource_mut::<ColonyResources>().food = 5.0;

        commission_artwork(&mut world, pop, "Ode to Rust").expect_err("too poor");
    }

    #[test]
    fn test_gallery_morale_stacks_to_cap() {
        let mut world = setup();
        let pop = crowned(&mut world, 3, 4);
        world.resource_mut::<ColonyResources>().food = 100.0;
        world.resource_mut::<ColonyResources>().metal = 100.0;
        for name in ["One", "Two", "Three"] {
            commission_artwork(&mut world, pop, name).unwrap();
        }

        apply_sovereign_morale(&mut world);
        let morale = world.get::<Morale>(pop).unwrap();
        let mods: Vec<&MoodModifier> = morale
            .modifiers
            .iter()
            .filter(|m| m.label == GALLERY_MODIFIER_LABEL)
            .collect();
        assert_eq!(mods.len(), 1, "gallery modifier refreshes, never stacks");
        assert!(
            (mods[0].value - GALLERY_MORALE_MAX).abs() < f32::EPSILON,
            "three artworks cap at +0.3, got {}",
            mods[0].value
        );
    }

    // --- melancholy + edicts ----------------------------------------------------

    #[test]
    fn test_pop_death_raises_melancholy() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        let p2 = spawn_pop(&mut world, 6, 5);
        // Keep morale below the decay gate so the assertion is exact.
        world.get_mut::<Morale>(pop).unwrap().value = 0.5;
        world.get_mut::<Morale>(p2).unwrap().value = 0.5;
        sovereign_tick(&mut world); // baseline count = 2
        drain_chronicle(&mut world);

        world.despawn(p2);
        sovereign_tick(&mut world);

        // The other pop died; the sovereign still lives.
        assert!(world.get_entity(pop).is_ok());
        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!(
            (sov.melancholy - (STARTING_MELANCHOLY + MELANCHOLY_PER_DEATH)).abs() < 1e-6,
            "death should raise melancholy, got {}",
            sov.melancholy
        );
    }

    #[test]
    fn test_banishment_edict_fires_at_threshold() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        let p2 = spawn_pop(&mut world, 6, 5);
        world.get_mut::<Sovereign>(pop).unwrap().melancholy = 0.55;
        // Give p2 a non-idle action so the edict has something to override.
        world.get_mut::<PopAction>(p2).unwrap().current = ActionType::Work;

        sovereign_tick(&mut world);

        let state = world.resource::<SovereignState>();
        assert!(state.banishment.is_some(), "banishment should fire at 0.5+");
        assert!(state
            .fired_edicts
            .contains(&EdictKind::BanishmentOfMondays));
        let sov = world.get::<Sovereign>(pop).unwrap();
        assert!((sov.melancholy - MELANCHOLY_AFTER_EDICT).abs() < f32::EPSILON);
        let action = world.get::<PopAction>(p2).unwrap();
        assert_eq!(action.current, ActionType::Idle, "the colony idles");
    }

    #[test]
    fn test_banishment_wave_grants_morale() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        world.get_mut::<Sovereign>(pop).unwrap().melancholy = 0.55;
        sovereign_tick(&mut world); // fires
        assert!(world.resource::<SovereignState>().banishment.is_some());

        for _ in 0..BANISHMENT_IDLE_TICKS {
            sovereign_tick(&mut world);
        }
        // Idle phase over: the wave begins.
        assert_eq!(
            world
                .resource::<SovereignState>()
                .banishment
                .as_ref()
                .unwrap()
                .wave_remaining,
            BANISHMENT_WAVE_DURATION
        );
        apply_sovereign_morale(&mut world);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale.modifiers.iter().any(|m| m.label == BANISHMENT_WAVE_LABEL
                && (m.value - BANISHMENT_WAVE_MORALE).abs() < f32::EPSILON),
            "relief wave should grant +0.3 morale"
        );
    }

    #[test]
    fn test_feast_edict_consumes_food() {
        let mut world = setup();
        let pop = crowned(&mut world, 5, 5);
        world.resource_mut::<ColonyResources>().food = 40.0;
        world.get_mut::<Sovereign>(pop).unwrap().melancholy = 0.8;

        sovereign_tick(&mut world);

        assert!(world.resource::<SovereignState>().feast.is_some());
        assert!((world.resource::<ColonyResources>().food - 20.0).abs() < f32::EPSILON);
        apply_sovereign_morale(&mut world);
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == FEAST_MODIFIER_LABEL));
    }

    #[test]
    fn test_edicts_list_shows_thresholds_and_state() {
        let mut world = setup();
        let text = describe_edicts(&mut world);
        assert!(text.contains("No sovereign"), "dormant without a crown");
        crowned(&mut world, 5, 5);
        let text = describe_edicts(&mut world);
        for name in [
            "Banishment of Mondays",
            "The Grand Feast",
            "Crown's Silence",
        ] {
            assert!(text.contains(name), "edict list should name {name}");
        }
    }

    #[test]
    fn test_sovereign_stats_line() {
        let mut world = setup();
        let (id, leg, mel) = sovereign_stats(&mut world);
        assert!(id.is_none() && leg == 0.0 && mel == 0.0);
        let pop = crowned(&mut world, 5, 5);
        let (id, leg, mel) = sovereign_stats(&mut world);
        assert_eq!(id, Some(pop.index()));
        assert!((leg - STARTING_LEGITIMACY).abs() < f32::EPSILON);
        assert!((mel - STARTING_MELANCHOLY).abs() < f32::EPSILON);
    }
}
