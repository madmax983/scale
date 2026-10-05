//! The Corsair — a space-pirate adventurer origin.
//!
//! Layers on top of the adventurer-mode spike (`direct_link`): a raider skiff
//! (a small drifting vessel) spawns near the starter colony, stocked with a
//! pirate crew of three pops. A possessed pop standing adjacent can
//! `interact` to take the captain's writ, becoming [`CorsairCaptain`] and
//! embarking aboard the skiff.
//!
//! While possessed-and-captained, the headless console gains `corsair`
//! (status), `raid <colony|trader|rival>` (boarding actions that plunder
//! into the skiff's [`SkiffHold`]), `unload` (ferry food to the colony),
//! `fence` (sell artifacts to a passing merchant), `skim <amount>` (pocket
//! credits off the books), `divide` (prize-law share split: captain's double
//! share, one share per crew, one for the [`ShipsPurse`]), and `repair`.
//!
//! Raids raise a Heat meter. Crossing 40 brings a colony patrol (impounded
//! fines); crossing 70 brings rival corsair hunters (hull damage, stolen
//! stores). Short the crew on shares and loyalty drops toward mutiny; pay
//! fair and it climbs.
//!
//! All names are original. The "space pirate" concept is concept-only
//! inspiration — no named characters, places, ships, or distinctive IP
//! anywhere. Mechanics and vibes only, under original names.

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::direct_link::Possessed;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::economy::trade::MerchantState;
use crate::layer1::economy::Wallet;
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::pressure::PressureGrid;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::temperature::TemperatureGrid;
use crate::layer1::terrain::{TerrainGrid, TerrainType};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Pirate crew stocked aboard the skiff at spawn.
pub const CREW_SIZE: usize = 3;
/// Loyalty (0.0..1.0) each crew pop starts with.
pub const STARTING_LOYALTY: f32 = 0.6;
/// Loyalty the crew drifts back toward each tick.
pub const LOYALTY_HOME: f32 = 0.6;
/// Loyalty drift per tick toward [`LOYALTY_HOME`].
pub const LOYALTY_DRIFT: f32 = 0.001;
/// Below this loyalty a crew member deserts.
pub const MUTINY_THRESHOLD: f32 = 0.25;

/// Skiff hull integrity bounds.
pub const SKIFF_HULL_MAX: f32 = 100.0;
/// Credits the ship's purse burns per tick (fuel, upkeep).
pub const PURSE_UPKEEP_PER_TICK: f32 = 0.02;
/// Hull lost per tick to neglect when the purse is empty and hull is damaged.
pub const NEGLECT_HULL_DECAY: f32 = 0.01;
/// Repair rate: hull points per credit.
pub const REPAIR_PER_CREDIT: f32 = 1.0;

/// Credits per fenced artifact.
pub const FENCE_RATE_PER_ARTIFACT: f32 = 25.0;
/// Heat gained by fencing hot goods.
pub const FENCE_HEAT: f32 = 5.0;

/// Heat decay per tick.
pub const HEAT_DECAY_PER_TICK: f32 = 0.05;
/// Heat at which the colony patrol responds.
pub const HEAT_PATROL_BAND: f32 = 40.0;
/// Heat at which rival corsair hunters respond.
pub const HEAT_HUNTER_BAND: f32 = 70.0;
/// Band hysteresis: the meter must fall this far below a band to re-arm it.
pub const HEAT_HYSTERESIS: f32 = 15.0;
/// Fraction of hold credits the patrol impounds as fines.
pub const PATROL_FINE_FRACTION: f32 = 0.20;
/// Hull damage dealt by a rival hunter attack.
pub const HUNTER_HULL_DAMAGE: f32 = 25.0;
/// Fraction of hold food rival hunters steal back.
pub const HUNTER_FOOD_THEFT_FRACTION: f32 = 0.30;

/// Skim fraction of the pre-divide hold above which the crew notices.
pub const SKIM_NOTICE_FRACTION: f32 = 0.10;
/// Loyalty swing for a fair divide.
pub const DIVIDE_FAIR_LOYALTY: f32 = 0.05;
/// Base loyalty penalty when the crew notices skimming.
pub const DIVIDE_SKIM_LOYALTY_BASE: f32 = 0.10;
/// Extra loyalty penalty per unit of skim fraction.
pub const DIVIDE_SKIM_LOYALTY_SCALE: f32 = 0.50;
/// Loyalty bump for a successful raid.
pub const RAID_SUCCESS_LOYALTY: f32 = 0.03;
/// Loyalty cost of a failed raid.
pub const RAID_FAIL_LOYALTY: f32 = 0.05;
/// Roll margin under success probability that counts as a clean getaway
/// (no heat — nobody saw a thing).
pub const CLEAN_GETAWAY_MARGIN: f32 = 0.25;

/// Captain's shares in the prize-law split (crew get 1 each, purse gets 1).
pub const CAPTAIN_SHARES: f32 = 2.0;
/// The purse's shares in the prize-law split.
pub const PURSE_SHARES: f32 = 1.0;

/// How far (Chebyshev) crew may stray from the skiff before the tether
/// nudges them back aboard.
pub const CREW_TETHER_RADIUS: i32 = 2;
/// Hold food eaten per hungry mouth per tick by the ship's mess.
pub const CREW_MEAL_PER_TICK: f32 = 0.02;
/// Satiety restored per tick by the ship's mess.
pub const CREW_MEAL_SATIETY: f32 = 0.2;
/// Below this hunger the mess feeds a mouth.
pub const CREW_MEAL_HUNGER_GATE: f32 = 0.95;
/// The skiff is a sealed vessel: pressure envelope radius (tiles) maintained
/// around her so the crew doesn't suffocate on the open regolith.
pub const SKIFF_PRESSURE_RADIUS: i32 = 3;
/// The sealed envelope is also kept livable: thermostat target (Celsius).
/// The tethered crew winters outside the habitat's LifeSupport radius; without
/// her own heat the crew takes 0.5/tick hypothermia all winter and is gone
/// by tick ~900 (observed in every tick-1000 trial).
pub const SKIFF_ENVELOPE_TEMP: f32 = 15.0;
/// Max heat pushed per tick toward the envelope target (LifeSupport's cap).
/// Thermostat-style: only pushes tiles below the target, so it can never
/// overheat the crew in summer.
pub const SKIFF_HEAT_PER_TICK: f32 = 12.0;

/// Chronicle text when the writ changes hands.
pub const WRIT_CHRONICLE: &str =
    "A captain's writ changes hands aboard the raider skiff. The crew grins.";
/// Chronicle text when the skiff is hulled.
pub const SKIFF_DESTROYED_CHRONICLE: &str =
    "The raider skiff is a drifting hulk, holed and dark.";

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// The raider skiff: a small drifting vessel parked near the colony.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct RaiderSkiff;

/// Plunder inventory aboard the skiff.
#[derive(Component, Debug, Clone, Copy)]
pub struct SkiffHold {
    /// Stolen food awaiting unload.
    pub food: f32,
    /// Stolen credits awaiting the prize-law divide.
    pub credits: f32,
    /// Stolen artifacts awaiting a fence.
    pub artifacts: u32,
}

impl Default for SkiffHold {
    fn default() -> Self {
        Self {
            food: 0.0,
            credits: 0.0,
            artifacts: 0,
        }
    }
}

/// Hull integrity of the skiff.
#[derive(Component, Debug, Clone, Copy)]
pub struct SkiffHull {
    /// Current hull points (0.0..=[`SKIFF_HULL_MAX`]).
    pub hp: f32,
}

impl Default for SkiffHull {
    fn default() -> Self {
        Self { hp: SKIFF_HULL_MAX }
    }
}

/// The ship's purse: the crew's shared repair/fuel fund, fed by its
/// prize-law share.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct ShipsPurse {
    /// Credits in the purse.
    pub credits: f32,
}

/// A pirate crew pop, loyal (or not) to the captain.
#[derive(Component, Debug, Clone, Copy)]
pub struct CorsairCrew {
    /// 0.0 (mutinous) .. 1.0 (die-hard). Starts at [`STARTING_LOYALTY`].
    pub loyalty: f32,
}

/// Status of the pop holding the captain's writ.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct CorsairCaptain;

// ---------------------------------------------------------------------------
// State resource
// ---------------------------------------------------------------------------

/// Machine state for the Corsair origin.
#[derive(Resource, Debug, Clone)]
pub struct CorsairState {
    /// Whether the skiff spawn has been attempted (once per colony).
    pub skiff_spawn_attempted: bool,
    /// The pop currently holding the captain's writ, if any.
    pub captain: Option<Entity>,
    /// The skiff entity, if spawned.
    pub skiff: Option<Entity>,
    /// Heat / wanted meter, 0.0..=100.0.
    pub heat: f32,
    /// Highest response band crossed: 0 = calm, 1 = patrol, 2 = hunters.
    pub heat_band: u8,
    /// Credits skimmed off the books since the last divide.
    pub skimmed: f32,
}

impl Default for CorsairState {
    fn default() -> Self {
        Self {
            skiff_spawn_attempted: false,
            captain: None,
            skiff: None,
            heat: 0.0,
            heat_band: 0,
            skimmed: 0.0,
        }
    }
}

/// Idempotent resource setup for tests and the tick schedule.
pub fn ensure_corsair_resources(world: &mut World) {
    if world.get_resource::<CorsairState>().is_none() {
        world.insert_resource(CorsairState::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_corsair_resources(world);
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: text.to_string(),
            importance,
        });
}

// ---------------------------------------------------------------------------
// Spawn & embark
// ---------------------------------------------------------------------------

/// Spawn the raider skiff (once): a vessel on a walkable tile in a ring
/// 6..=10 tiles from the lander, stocked with [`CREW_SIZE`] pirate crew pops.
pub fn spawn_raider_skiff_once(world: &mut World) {
    ensure_corsair_resources(world);
    {
        let state = world.resource::<CorsairState>();
        if state.skiff_spawn_attempted {
            return;
        }
    }
    world.resource_mut::<CorsairState>().skiff_spawn_attempted = true;

    // Only one skiff per colony.
    if world.query::<&RaiderSkiff>().iter(world).next().is_some() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    // Snapshot candidate tiles first (mirrors the crown spawn: the building
    // query below needs `&mut world`, which conflicts with live borrows).
    let (walkable, occupied, buildings) = {
        let mut walkable = std::collections::HashSet::new();
        if let Some(terrain) = world.get_resource::<TerrainGrid>() {
            for dx in -10..=10i32 {
                for dy in -10..=10i32 {
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
        'search: for radius in 6..=10i32 {
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
    let mut rng = rand::thread_rng();
    let skiff = world
        .spawn((
            RaiderSkiff,
            SkiffHold::default(),
            SkiffHull::default(),
            ShipsPurse::default(),
            pos,
        ))
        .id();
    for name in draw_crew_names(&mut rng, CREW_SIZE) {
        let mut bundle = PopBundle::random(pos.x, pos.y, &mut rng);
        bundle.name = PopName(format!("Corsair {name}"));
        world.spawn((bundle, CorsairCrew { loyalty: STARTING_LOYALTY }));
    }
    world.resource_mut::<CorsairState>().skiff = Some(skiff);
    send_chronicle(
        world,
        &format!(
            "Rumor: a raider skiff drifts at ({}, {}), her crew watching the colony.",
            pos.x, pos.y
        ),
        EventImportance::Minor,
    );
}

/// Original flavor names for crew pops (concept-only, no lifted IP).
/// Drawn without replacement so the crew never share a moniker.
fn draw_crew_names(rng: &mut impl Rng, n: usize) -> Vec<String> {
    const MONIKERS: &[&str] = &[
        "Vex", "Marrow", "Tally", "Rook", "Sable", "Flint", "Wren", "Husk", "Pike", "Dredge",
        "Lark", "Cinder",
    ];
    let mut idx: Vec<usize> = (0..MONIKERS.len()).collect();
    let mut out = Vec::new();
    for i in 0..n.min(MONIKERS.len()) {
        let j = rng.gen_range(i..MONIKERS.len());
        idx.swap(i, j);
        out.push(MONIKERS[idx[i]].to_string());
    }
    out
}

/// The skiff adjacent (Chebyshev distance <= 1) to a pop, if any.
pub fn skiff_within_reach(world: &mut World, pop: Entity) -> Option<Entity> {
    let pos = *world.get::<GridPosition>(pop)?;
    world
        .query::<(Entity, &RaiderSkiff, &GridPosition)>()
        .iter(world)
        .find(|(_, _, spos)| (spos.x - pos.x).abs() <= 1 && (spos.y - pos.y).abs() <= 1)
        .map(|(e, _, _)| e)
}

/// Take the captain's writ: the pop becomes [`CorsairCaptain`] and embarks
/// aboard the adjacent skiff.
pub fn try_embark_corsair(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_corsair_resources(world);
    if world.get::<CorsairCaptain>(pop).is_some() {
        return Err("This pop already holds the captain's writ.".to_string());
    }
    let skiff = skiff_within_reach(world, pop)
        .ok_or_else(|| "No raider skiff within reach. Move adjacent to one.".to_string())?;
    let hull = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp)
        .unwrap_or(SKIFF_HULL_MAX);
    if hull <= 0.0 {
        return Err("The skiff is a drifting hulk — no writ to take here.".to_string());
    }
    let spos = *world
        .get::<GridPosition>(skiff)
        .ok_or_else(|| "The skiff has no position.".to_string())?;
    world.entity_mut(pop).insert(CorsairCaptain);
    // The captain is no longer crew: prize-law shares treat them separately.
    world.entity_mut(pop).remove::<CorsairCrew>();
    if let Some(mut pos) = world.get_mut::<GridPosition>(pop) {
        *pos = spos;
    }
    world.resource_mut::<CorsairState>().captain = Some(pop);
    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));
    send_chronicle(world, WRIT_CHRONICLE, EventImportance::Major);
    Ok(format!(
        "{name} takes the captain's writ and embarks aboard the raider skiff at ({}, {}).",
        spos.x, spos.y
    ))
}

/// All living crew pops (entity list).
pub fn crew_entities(world: &mut World) -> Vec<Entity> {
    world
        .query::<(Entity, &CorsairCrew, &Pop)>()
        .iter(world)
        .map(|(e, _, _)| e)
        .collect()
}

/// Average crew loyalty (0.0 when there is no crew).
pub fn average_crew_loyalty(world: &mut World) -> f32 {
    let loyalties: Vec<f32> = world
        .query::<(&CorsairCrew, &Pop)>()
        .iter(world)
        .map(|(c, _)| c.loyalty)
        .collect();
    if loyalties.is_empty() {
        0.0
    } else {
        loyalties.iter().sum::<f32>() / loyalties.len() as f32
    }
}

// ---------------------------------------------------------------------------
// Raids
// ---------------------------------------------------------------------------

/// What the boarding party hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidTarget {
    /// The home colony's own stores (the crew does not ask permission).
    Colony,
    /// A passing merchant's cargo.
    Trader,
    /// A rival corsair cache — rich, and defended.
    Rival,
}

impl RaidTarget {
    /// Parse a headless `raid` argument.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "colony" => Some(Self::Colony),
            "trader" | "merchant" => Some(Self::Trader),
            "rival" | "cache" => Some(Self::Rival),
            _ => None,
        }
    }

    /// Flavor name for chronicles and messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Colony => "the colony stores",
            Self::Trader => "a passing trader",
            Self::Rival => "a rival corsair cache",
        }
    }
}

/// Tunables for one raid target.
#[derive(Debug, Clone, Copy)]
pub struct RaidParams {
    /// Base success probability; crew loyalty adds 0.1 * avg_loyalty.
    pub success_p: f32,
    /// Heat gained on a spotted raid.
    pub heat_gain: f32,
    /// Hull damage on a failed raid.
    pub fail_hull_damage: f32,
    /// Health damage to a random crew member on a failed raid.
    pub fail_crew_damage: f32,
}

/// Target tunables (pure, unit-testable).
#[must_use]
pub fn raid_params(target: RaidTarget) -> RaidParams {
    match target {
        RaidTarget::Colony => RaidParams {
            success_p: 0.70,
            heat_gain: 8.0,
            fail_hull_damage: 5.0,
            fail_crew_damage: 5.0,
        },
        RaidTarget::Trader => RaidParams {
            success_p: 0.55,
            heat_gain: 12.0,
            fail_hull_damage: 10.0,
            fail_crew_damage: 10.0,
        },
        RaidTarget::Rival => RaidParams {
            success_p: 0.45,
            heat_gain: 15.0,
            fail_hull_damage: 20.0,
            fail_crew_damage: 15.0,
        },
    }
}

/// The mechanical result of a raid, before world application.
#[derive(Debug, Clone, Copy)]
pub struct RaidOutcome {
    /// Whether the boarding succeeded.
    pub success: bool,
    /// Food plundered (0.0 on failure).
    pub food: f32,
    /// Credits plundered (0.0 on failure).
    pub credits: f32,
    /// Artifacts plundered (0 on failure).
    pub artifacts: u32,
    /// Heat gained (halved on failure, zeroed on a clean getaway).
    pub heat_gain: f32,
    /// Hull damage taken (0.0 on success).
    pub hull_damage: f32,
    /// Health damage to one random crew member (0.0 on success).
    pub crew_damage: f32,
    /// Loyalty delta applied to every crew member.
    pub loyalty_delta: f32,
    /// True when the raid was so clean nobody saw a thing.
    pub clean_getaway: bool,
}

/// Pure raid resolution: deterministic in `roll` (0.0..1.0), so unit tests
/// can pin every branch. `colony_food` / `colony_credits` cap colony raids.
#[must_use]
pub fn resolve_raid(
    target: RaidTarget,
    crew_count: usize,
    avg_loyalty: f32,
    roll: f32,
    colony_food: f32,
    colony_credits: f32,
) -> RaidOutcome {
    let params = raid_params(target);
    let p = (params.success_p + 0.1 * avg_loyalty.clamp(0.0, 1.0)).clamp(0.0, 0.95);
    let crew = crew_count as f32;
    if roll <= p {
        let clean_getaway = roll <= p - CLEAN_GETAWAY_MARGIN;
        let heat_gain = if clean_getaway { 0.0 } else { params.heat_gain };
        let (food, credits, artifacts) = match target {
            RaidTarget::Colony => (
                (colony_food * 0.20).min(10.0 + 3.0 * crew),
                (colony_credits * 0.15).min(20.0 + 5.0 * crew),
                0,
            ),
            RaidTarget::Trader => (10.0 + 5.0 * crew, 30.0 + 10.0 * crew, 0),
            RaidTarget::Rival => (0.0, 40.0 + 15.0 * crew, 1 + (crew_count / 2) as u32),
        };
        RaidOutcome {
            success: true,
            food,
            credits,
            artifacts,
            heat_gain,
            hull_damage: 0.0,
            crew_damage: 0.0,
            loyalty_delta: RAID_SUCCESS_LOYALTY,
            clean_getaway,
        }
    } else {
        RaidOutcome {
            success: false,
            food: 0.0,
            credits: 0.0,
            artifacts: 0,
            heat_gain: params.heat_gain / 2.0,
            hull_damage: params.fail_hull_damage,
            crew_damage: params.fail_crew_damage,
            loyalty_delta: -RAID_FAIL_LOYALTY,
            clean_getaway: false,
        }
    }
}

/// Apply a resolved raid to the world: move plunder into the skiff hold,
/// deduct colony stores, damage hull/crew, shift loyalty, add heat.
pub fn apply_raid_outcome(
    world: &mut World,
    captain: Entity,
    target: RaidTarget,
    outcome: RaidOutcome,
) -> String {
    let captain_name = world
        .get::<PopName>(captain)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", captain.index()));
    let skiff = world.resource::<CorsairState>().skiff;

    // Deduct from the colony on a successful colony raid (proportional
    // across wallets so no single pop is cleaned out).
    if outcome.success && target == RaidTarget::Colony {
        let take_food = outcome.food;
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.food = (res.food - take_food).max(0.0);
        }
        let take_credits = outcome.credits;
        if take_credits > 0.0 {
            let wallets: Vec<Entity> = world
                .query::<(Entity, &Wallet, &Pop)>()
                .iter(world)
                .map(|(e, _, _)| e)
                .collect();
            let total: f32 = wallets
                .iter()
                .filter_map(|e| world.get::<Wallet>(*e))
                .map(|w| w.credits)
                .sum();
            if total > 0.0 {
                for e in wallets {
                    if let Some(mut w) = world.get_mut::<Wallet>(e) {
                        let share = w.credits / total;
                        w.credits = (w.credits - take_credits * share).max(0.0);
                    }
                }
            }
        }
    }

    // Plunder lands in the hold.
    if let Some(skiff) = skiff {
        if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
            hold.food += outcome.food;
            hold.credits += outcome.credits;
            hold.artifacts += outcome.artifacts;
        }
        // Failed raids batter the hull.
        if outcome.hull_damage > 0.0 {
            damage_skiff_hull(world, skiff, outcome.hull_damage);
        }
    }

    // Failed raids bloody a random crew member.
    if outcome.crew_damage > 0.0 {
        let crew = crew_entities(world);
        if !crew.is_empty() {
            let mut rng = rand::thread_rng();
            let victim = crew[rng.gen_range(0..crew.len())];
            if let Some(mut health) = world.get_mut::<Health>(victim) {
                health.current = (health.current - outcome.crew_damage).max(0.0);
            }
        }
    }

    // Loyalty moves for the whole crew.
    {
        let mut query = world.query::<&mut CorsairCrew>();
        for mut c in query.iter_mut(world) {
            c.loyalty = (c.loyalty + outcome.loyalty_delta).clamp(0.0, 1.0);
        }
    }

    add_heat(world, outcome.heat_gain);

    // Chronicle the outcome.
    let importance = if outcome.success {
        EventImportance::Major
    } else {
        EventImportance::Minor
    };
    let mut text = if outcome.success {
        let mut parts = vec![format!(
            "Boarding party led by {captain_name} hits {}",
            target.name()
        )];
        if outcome.food > 0.0 {
            parts.push(format!("{:.1} food", outcome.food));
        }
        if outcome.credits > 0.0 {
            parts.push(format!("{:.0}cr", outcome.credits));
        }
        if outcome.artifacts > 0 {
            parts.push(format!("{} artifacts", outcome.artifacts));
        }
        if outcome.clean_getaway {
            parts.push("clean getaway — nobody saw a thing".to_string());
        }
        format!("{}: {}.", parts[0], parts[1..].join(", "))
    } else {
        format!(
            "Boarding party led by {captain_name} repelled at {} — the skiff takes {:.0} hull damage.",
            target.name(),
            outcome.hull_damage
        )
    };
    if target == RaidTarget::Trader && outcome.success {
        text.push_str(" The trader will remember this.");
    }
    send_chronicle(world, &text, importance);
    text
}

/// Damage the skiff hull; chronicles destruction on crossing to zero.
fn damage_skiff_hull(world: &mut World, skiff: Entity, amount: f32) {
    let was_alive = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp > 0.0)
        .unwrap_or(false);
    if let Some(mut hull) = world.get_mut::<SkiffHull>(skiff) {
        hull.hp = (hull.hp - amount).max(0.0);
    }
    let now_dead = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp <= 0.0)
        .unwrap_or(false);
    if was_alive && now_dead {
        send_chronicle(world, SKIFF_DESTROYED_CHRONICLE, EventImportance::Major);
    }
}

/// Resolve and apply a raid for the captain. The roll comes from the
/// thread RNG; tests pin it via [`resolve_raid`] + [`apply_raid_outcome`].
pub fn execute_raid(
    world: &mut World,
    captain: Entity,
    target: RaidTarget,
) -> Result<String, String> {
    ensure_corsair_resources(world);
    if world.get::<CorsairCaptain>(captain).is_none() {
        return Err("Only the holder of the captain's writ can lead a raid.".to_string());
    }
    let skiff = world
        .resource::<CorsairState>()
        .skiff
        .ok_or_else(|| "No raider skiff.".to_string())?;
    let hull = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp)
        .unwrap_or(0.0);
    if hull <= 0.0 {
        return Err("The skiff is a drifting hulk — repair her first.".to_string());
    }
    if target == RaidTarget::Trader
        && world.resource::<MerchantState>().active_merchant.is_none()
    {
        return Err("No trader in the black right now.".to_string());
    }
    let crew = crew_entities(world);
    let loyalty = average_crew_loyalty(world);
    let colony_food = world.resource::<ColonyResources>().food;
    let colony_credits: f32 = world
        .query::<(&Wallet, &Pop)>()
        .iter(world)
        .map(|(w, _)| w.credits)
        .sum();
    let mut rng = rand::thread_rng();
    let roll: f32 = rng.gen_range(0.0..1.0);
    let outcome = resolve_raid(
        target,
        crew.len(),
        loyalty,
        roll,
        colony_food,
        colony_credits,
    );
    Ok(apply_raid_outcome(world, captain, target, outcome))
}

// ---------------------------------------------------------------------------
// Plunder economy: unload / fence / skim / divide / repair
// ---------------------------------------------------------------------------

fn require_captain(world: &mut World, captain: Entity) -> Result<Entity, String> {
    ensure_corsair_resources(world);
    if world.get::<CorsairCaptain>(captain).is_none() {
        return Err("Only the holder of the captain's writ can do that.".to_string());
    }
    world
        .resource::<CorsairState>()
        .skiff
        .ok_or_else(|| "No raider skiff.".to_string())
}

/// Ferry the hold's food to the colony stores. Credits stay aboard for the
/// prize-law divide; artifacts stay aboard for the fence.
pub fn unload_hold(world: &mut World, captain: Entity) -> Result<String, String> {
    let skiff = require_captain(world, captain)?;
    let food = world
        .get::<SkiffHold>(skiff)
        .map(|h| h.food)
        .unwrap_or(0.0);
    if food <= 0.0 {
        return Err("The hold is empty — nothing to unload.".to_string());
    }
    if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
        hold.food = 0.0;
    }
    world.resource_mut::<ColonyResources>().food += food;
    send_chronicle(
        world,
        &format!("The skiff quietly unloads {food:.1} food into the colony stores."),
        EventImportance::Minor,
    );
    Ok(format!(
        "Unloaded {food:.1} food into the colony stores."
    ))
}

/// Fence stolen artifacts to the currently present merchant.
pub fn fence_plunder(world: &mut World, captain: Entity) -> Result<String, String> {
    let skiff = require_captain(world, captain)?;
    if world.resource::<MerchantState>().active_merchant.is_none() {
        return Err("No trader in the black — nothing to fence to.".to_string());
    }
    let artifacts = world
        .get::<SkiffHold>(skiff)
        .map(|h| h.artifacts)
        .unwrap_or(0);
    if artifacts == 0 {
        return Err("The hold has no artifacts to fence.".to_string());
    }
    let payout = artifacts as f32 * FENCE_RATE_PER_ARTIFACT;
    if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
        hold.artifacts = 0;
        hold.credits += payout;
    }
    add_heat(world, FENCE_HEAT);
    send_chronicle(
        world,
        &format!("Hot artifacts fenced for {payout:.0}cr. Someone will talk."),
        EventImportance::Minor,
    );
    Ok(format!(
        "Fenced {artifacts} artifacts for {payout:.0}cr into the hold (+{FENCE_HEAT:.0} heat)."
    ))
}

/// Skim credits off the books: hold credits into the captain's own wallet.
/// The crew may notice at the next divide.
pub fn skim_credits(
    world: &mut World,
    captain: Entity,
    amount: f32,
) -> Result<String, String> {
    let skiff = require_captain(world, captain)?;
    if amount <= 0.0 {
        return Err("Skim a positive amount.".to_string());
    }
    let hold_credits = world
        .get::<SkiffHold>(skiff)
        .map(|h| h.credits)
        .unwrap_or(0.0);
    if amount > hold_credits {
        return Err(format!(
            "The hold only has {hold_credits:.1}cr to skim."
        ));
    }
    if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
        hold.credits -= amount;
    }
    if let Some(mut wallet) = world.get_mut::<Wallet>(captain) {
        wallet.credits += amount;
    }
    world.resource_mut::<CorsairState>().skimmed += amount;
    Ok(format!(
        "Skimmed {amount:.1}cr into your own wallet. The crew saw nothing. Probably."
    ))
}

/// The prize-law divide: the hold's credits are split by shares — the
/// captain takes [`CAPTAIN_SHARES`], each crew pop one, the ship's purse
/// [`PURSE_SHARES`]. The crew respects fair shares; skim too much and
/// loyalty drops.
pub fn divide_plunder(world: &mut World, captain: Entity) -> Result<String, String> {
    let skiff = require_captain(world, captain)?;
    let hold_credits = world
        .get::<SkiffHold>(skiff)
        .map(|h| h.credits)
        .unwrap_or(0.0);
    if hold_credits <= 0.0 {
        return Err("The hold is empty — nothing to divide.".to_string());
    }
    let crew = crew_entities(world);
    let shares_total = CAPTAIN_SHARES + crew.len() as f32 + PURSE_SHARES;
    let per_share = hold_credits / shares_total;

    // Payouts.
    if let Some(mut wallet) = world.get_mut::<Wallet>(captain) {
        wallet.credits += CAPTAIN_SHARES * per_share;
    }
    for e in &crew {
        if let Some(mut wallet) = world.get_mut::<Wallet>(*e) {
            wallet.credits += per_share;
        }
    }
    if let Some(mut purse) = world.get_mut::<ShipsPurse>(skiff) {
        purse.credits += PURSE_SHARES * per_share;
    }
    if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
        hold.credits = 0.0;
    }

    // The crew judges the split against what was skimmed.
    let skimmed = world.resource::<CorsairState>().skimmed;
    let skim_frac = skimmed / hold_credits;
    let (loyalty_delta, verdict) = if skim_frac > SKIM_NOTICE_FRACTION {
        let penalty = DIVIDE_SKIM_LOYALTY_BASE + DIVIDE_SKIM_LOYALTY_SCALE * skim_frac.min(1.0);
        (-penalty, "the crew counts the coins twice and mutters")
    } else {
        (DIVIDE_FAIR_LOYALTY, "fair shares — the crew drinks to the code")
    };
    {
        let mut query = world.query::<&mut CorsairCrew>();
        for mut c in query.iter_mut(world) {
            c.loyalty = (c.loyalty + loyalty_delta).clamp(0.0, 1.0);
        }
    }
    world.resource_mut::<CorsairState>().skimmed = 0.0;

    send_chronicle(
        world,
        &format!(
            "Prize-law divide: {:.0}cr split {} ways — {verdict}.",
            hold_credits,
            shares_total as u32,
        ),
        EventImportance::Major,
    );
    Ok(format!(
        "Divided {hold_credits:.1}cr by the code: captain {:.1}cr, each of {} crew {:.1}cr, purse {:.1}cr — {verdict}.",
        CAPTAIN_SHARES * per_share,
        crew.len(),
        per_share,
        PURSE_SHARES * per_share,
    ))
}

/// Spend purse credits to repair the hull (1 credit = 1 hull point).
/// With no amount, repairs as much as the purse (and damage) allows.
pub fn repair_skiff(
    world: &mut World,
    captain: Entity,
    amount: Option<f32>,
) -> Result<String, String> {
    let skiff = require_captain(world, captain)?;
    let (hp, purse) = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp)
        .zip(world.get::<ShipsPurse>(skiff).map(|p| p.credits))
        .unwrap_or((0.0, 0.0));
    let missing = SKIFF_HULL_MAX - hp;
    if missing <= 0.0 {
        return Err("The hull is already sound.".to_string());
    }
    if purse <= 0.0 {
        return Err("The ship's purse is empty — divide some plunder first.".to_string());
    }
    let spend = amount.unwrap_or(purse).min(purse).min(missing);
    if spend <= 0.0 {
        return Err("Nothing to spend.".to_string());
    }
    if let Some(mut p) = world.get_mut::<ShipsPurse>(skiff) {
        p.credits -= spend;
    }
    if let Some(mut h) = world.get_mut::<SkiffHull>(skiff) {
        h.hp = (h.hp + spend * REPAIR_PER_CREDIT).min(SKIFF_HULL_MAX);
    }
    Ok(format!(
        "Spent {spend:.1}cr from the purse: hull {hp:.0} -> {:.0}.",
        (hp + spend * REPAIR_PER_CREDIT).min(SKIFF_HULL_MAX),
    ))
}

// ---------------------------------------------------------------------------
// Heat / wanted meter
// ---------------------------------------------------------------------------

/// Add heat and fire band-crossing responses (patrol fines, hunter attacks).
/// Pure enough to unit test: returns the events that fired.
#[must_use]
pub fn heat_band_events(_old_heat: f32, new_heat: f32, old_band: u8) -> (u8, Vec<&'static str>) {
    let mut band = old_band;
    let mut events = Vec::new();
    if new_heat >= HEAT_HUNTER_BAND && band < 2 {
        band = 2;
        events.push("hunters");
    } else if new_heat >= HEAT_PATROL_BAND && band < 1 {
        band = 1;
        events.push("patrol");
    }
    (band, events)
}

/// Add heat to the meter, firing response events on band crossings.
pub fn add_heat(world: &mut World, amount: f32) {
    ensure_corsair_resources(world);
    if amount <= 0.0 {
        return;
    }
    let (old_heat, old_band) = {
        let s = world.resource::<CorsairState>();
        (s.heat, s.heat_band)
    };
    let new_heat = (old_heat + amount).min(100.0);
    let (band, events) = heat_band_events(old_heat, new_heat, old_band);
    {
        let mut s = world.resource_mut::<CorsairState>();
        s.heat = new_heat;
        s.heat_band = band;
    }
    let skiff = world.resource::<CorsairState>().skiff;
    for event in events {
        match event {
            "patrol" => {
                let fine = skiff
                    .and_then(|s| world.get::<SkiffHold>(s).map(|h| h.credits))
                    .unwrap_or(0.0)
                    * PATROL_FINE_FRACTION;
                if let Some(s) = skiff {
                    if let Some(mut hold) = world.get_mut::<SkiffHold>(s) {
                        hold.credits = (hold.credits - fine).max(0.0);
                    }
                }
                send_chronicle(
                    world,
                    &format!(
                        "Colony patrol boards the skiff and impounds {fine:.0}cr in contraband fines."
                    ),
                    EventImportance::Major,
                );
            }
            "hunters" => {
                let mut stolen_food = 0.0;
                if let Some(s) = skiff {
                    damage_skiff_hull(world, s, HUNTER_HULL_DAMAGE);
                    if let Some(mut hold) = world.get_mut::<SkiffHold>(s) {
                        stolen_food = hold.food * HUNTER_FOOD_THEFT_FRACTION;
                        hold.food -= stolen_food;
                    }
                }
                send_chronicle(
                    world,
                    &format!(
                        "Rival corsair hunters run the skiff down: -{HUNTER_HULL_DAMAGE:.0} hull, {stolen_food:.1} food seized."
                    ),
                    EventImportance::Major,
                );
            }
            _ => {}
        }
    }
}

/// Per-tick origin system: heat decay (with band re-arming), loyalty drift,
/// mutiny, purse upkeep, and hull neglect.
pub fn corsair_tick(world: &mut World) {
    ensure_corsair_resources(world);

    // Heat decay + hysteresis re-arm.
    {
        let mut s = world.resource_mut::<CorsairState>();
        s.heat = (s.heat - HEAT_DECAY_PER_TICK).max(0.0);
        if s.heat < HEAT_PATROL_BAND - HEAT_HYSTERESIS {
            s.heat_band = 0;
        } else if s.heat < HEAT_HUNTER_BAND - HEAT_HYSTERESIS && s.heat_band == 2 {
            s.heat_band = 1;
        }
    }

    // Captain cleanup: if the captain's pop is gone, the writ is lost.
    {
        let captain = world.resource::<CorsairState>().captain;
        if let Some(c) = captain {
            if world.get::<Pop>(c).is_none() {
                world.resource_mut::<CorsairState>().captain = None;
            }
        }
    }

    // Loyalty drifts home; the disloyal desert.
    let deserters: Vec<(Entity, String)> = {
        let mut query = world.query::<(Entity, &mut CorsairCrew, &PopName, &Pop)>();
        let mut out = Vec::new();
        for (e, mut crew, name, _) in query.iter_mut(world) {
            let lo = crew.loyalty;
            crew.loyalty = if lo < LOYALTY_HOME {
                (lo + LOYALTY_DRIFT).min(LOYALTY_HOME)
            } else if lo > LOYALTY_HOME {
                (lo - LOYALTY_DRIFT).max(LOYALTY_HOME)
            } else {
                lo
            };
            if crew.loyalty < MUTINY_THRESHOLD {
                out.push((e, name.0.clone()));
            }
        }
        out
    };
    for (e, name) in deserters {
        world.entity_mut(e).remove::<CorsairCrew>();
        send_chronicle(
            world,
            &format!("{name} slips away in the night — the crew is thinner."),
            EventImportance::Minor,
        );
    }

    // Purse upkeep; neglect rots the hull when the purse runs dry.
    if let Some(skiff) = world.resource::<CorsairState>().skiff {
        let mut purse_empty = false;
        if let Some(mut purse) = world.get_mut::<ShipsPurse>(skiff) {
            purse.credits = (purse.credits - PURSE_UPKEEP_PER_TICK).max(0.0);
            purse_empty = purse.credits <= 0.0;
        }
        if purse_empty {
            if let Some(mut hull) = world.get_mut::<SkiffHull>(skiff) {
                if hull.hp > 0.0 && hull.hp < SKIFF_HULL_MAX {
                    hull.hp = (hull.hp - NEGLECT_HULL_DECAY).max(0.0);
                }
            }
        }
    }

    // The crew lives aboard: feed the ship's mess from the hold, and tether
    // stragglers back to the skiff so the colony's hazards don't pick them
    // off while they run colony errands.
    if let Some(skiff) = world.resource::<CorsairState>().skiff {
        if let Some(spos) = world.get::<GridPosition>(skiff).copied() {
            let aboard: Vec<(Entity, GridPosition, bool)> = world
                .query_filtered::<
                    (Entity, &GridPosition, Has<Possessed>),
                    (With<Pop>, Or<(With<CorsairCrew>, With<CorsairCaptain>)>),
                >()
                .iter(world)
                .map(|(e, p, possessed)| (e, *p, possessed))
                .collect();
            // Ship's mess.
            let hungry: Vec<Entity> = aboard
                .iter()
                .filter(|(e, _, _)| {
                    world
                        .get::<Needs>(*e)
                        .map(|n| n.hunger < CREW_MEAL_HUNGER_GATE)
                        .unwrap_or(false)
                })
                .map(|(e, _, _)| *e)
                .collect();
            if !hungry.is_empty() {
                let meal = CREW_MEAL_PER_TICK * hungry.len() as f32;
                let hold_food = world
                    .get::<SkiffHold>(skiff)
                    .map(|h| h.food)
                    .unwrap_or(0.0);
                if hold_food >= meal {
                    if let Some(mut hold) = world.get_mut::<SkiffHold>(skiff) {
                        hold.food -= meal;
                    }
                    for e in hungry {
                        if let Some(mut needs) = world.get_mut::<Needs>(e) {
                            needs.hunger = (needs.hunger + CREW_MEAL_SATIETY).min(1.0);
                        }
                    }
                }
            }
            // Tether.
            for (e, pos, possessed) in aboard {
                if possessed {
                    continue;
                }
                let dx = spos.x - pos.x;
                let dy = spos.y - pos.y;
                if dx.abs().max(dy.abs()) > CREW_TETHER_RADIUS {
                    if let Some(mut p) = world.get_mut::<GridPosition>(e) {
                        p.x += dx.signum();
                        p.y += dy.signum();
                    }
                }
            }
        }
    }

    // The skiff is a sealed vessel: hold a pressure envelope around her so
    // the crew (tethered within [`CREW_TETHER_RADIUS`]) doesn't suffocate,
    // and keep the envelope warm so they don't freeze in winter. Both run
    // before the damage systems (see schedule ordering).
    if let Some(skiff) = world.resource::<CorsairState>().skiff {
        if let Some(spos) = world.get::<GridPosition>(skiff).copied() {
            if let Some(mut grid) = world.get_resource_mut::<PressureGrid>() {
                for dx in -SKIFF_PRESSURE_RADIUS..=SKIFF_PRESSURE_RADIUS {
                    for dy in -SKIFF_PRESSURE_RADIUS..=SKIFF_PRESSURE_RADIUS {
                        grid.set(spos.x + dx, spos.y + dy, 1.0);
                    }
                }
            }
            // Thermostat-style heat (LifeSupport pattern): only pushes tiles
            // below the target up, capped per tick — never overheats summer.
            if let Some(mut tgrid) = world.get_resource_mut::<TemperatureGrid>() {
                for dx in -SKIFF_PRESSURE_RADIUS..=SKIFF_PRESSURE_RADIUS {
                    for dy in -SKIFF_PRESSURE_RADIUS..=SKIFF_PRESSURE_RADIUS {
                        let tx = spos.x + dx;
                        let ty = spos.y + dy;
                        let temp = tgrid.get_safe(tx, ty);
                        if temp < SKIFF_ENVELOPE_TEMP {
                            tgrid.add(tx, ty, (SKIFF_ENVELOPE_TEMP - temp).min(SKIFF_HEAT_PER_TICK));
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stats & status
// ---------------------------------------------------------------------------

/// Snapshot for the headless STATS line: (heat, hull, crew, avg loyalty).
pub fn corsair_stats(world: &mut World) -> (f32, f32, usize, f32) {
    ensure_corsair_resources(world);
    let heat = world.resource::<CorsairState>().heat;
    let skiff = world.resource::<CorsairState>().skiff;
    let hull = skiff
        .and_then(|s| world.get::<SkiffHull>(s).map(|h| h.hp))
        .unwrap_or(0.0);
    let crew = crew_entities(world);
    let loyalty = average_crew_loyalty(world);
    (heat, hull, crew.len(), loyalty)
}

/// Human-readable origin status for the `corsair` command.
pub fn describe_corsair(world: &mut World) -> String {
    ensure_corsair_resources(world);
    let state = world.resource::<CorsairState>().clone();
    let Some(skiff) = state.skiff else {
        return "No raider skiff in this colony.".to_string();
    };
    let hold = world.get::<SkiffHold>(skiff).copied().unwrap_or_default();
    let hull = world
        .get::<SkiffHull>(skiff)
        .map(|h| h.hp)
        .unwrap_or(0.0);
    let purse = world
        .get::<ShipsPurse>(skiff)
        .map(|p| p.credits)
        .unwrap_or(0.0);
    let pos = world
        .get::<GridPosition>(skiff)
        .map(|p| format!("({}, {})", p.x, p.y))
        .unwrap_or_else(|| "adrift".to_string());
    let captain = state
        .captain
        .and_then(|c| world.get::<PopName>(c).map(|n| n.0.clone()))
        .unwrap_or_else(|| "none".to_string());
    let crew: Vec<String> = world
        .query::<(&CorsairCrew, &PopName, &Pop)>()
        .iter(world)
        .map(|(c, n, _)| format!("{} ({:.0}%)", n.0, c.loyalty * 100.0))
        .collect();
    let band = match state.heat_band {
        2 => "HUNTED — rival corsairs are closing in",
        1 => "WATCHED — the colony patrol has your scent",
        _ => "quiet",
    };
    format!(
        "Raider skiff at {pos} — hull {hull:.0}/{SKIFF_HULL_MAX:.0}\n\
         Hold: {food:.1} food, {credits:.1}cr, {} artifacts | Purse: {purse:.1}cr\n\
         Heat: {heat:.1}/100 ({band}) | Skimmed off books: {skimmed:.1}cr\n\
         Captain: {captain}\n\
         Crew ({}): {}",
        hold.artifacts,
        crew.len(),
        if crew.is_empty() {
            "none — the skiff sails empty".to_string()
        } else {
            crew.join(", ")
        },
        food = hold.food,
        credits = hold.credits,
        heat = state.heat,
        skimmed = state.skimmed,
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
    use crate::layer1::economy::trade::{Merchant, MerchantState};
    use crate::layer1::economy::Wallet;
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::{Pop, PopName};
    use crate::layer1::psychology::needs::Needs;
    use crate::layer1::utility_types::PopAction;
    use crate::shared::time::SimulationTime;

    // --- scaffolding ---------------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(CorsairState::default());
        world.insert_resource(MerchantState::default());
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

    fn spawn_skiff(world: &mut World, x: i32, y: i32) -> Entity {
        let skiff = world
            .spawn((
                RaiderSkiff,
                SkiffHold::default(),
                SkiffHull::default(),
                ShipsPurse::default(),
                GridPosition { x, y },
            ))
            .id();
        world.resource_mut::<CorsairState>().skiff = Some(skiff);
        skiff
    }

    fn spawn_crew(world: &mut World, x: i32, y: i32, n: usize) -> Vec<Entity> {
        (0..n)
            .map(|i| {
                let e = spawn_pop(world, &format!("crew{i}"), x, y);
                world.entity_mut(e).insert(CorsairCrew {
                    loyalty: STARTING_LOYALTY,
                });
                e
            })
            .collect()
    }

    fn make_captain(world: &mut World, x: i32, y: i32) -> (Entity, Entity) {
        let skiff = spawn_skiff(world, x, y);
        let captain = spawn_pop(world, "captain", x + 1, y);
        try_embark_corsair(world, captain).expect("embark should succeed");
        (captain, skiff)
    }

    fn set_hold(world: &mut World, food: f32, credits: f32, artifacts: u32) {
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        let mut hold = world.get_mut::<SkiffHold>(skiff).unwrap();
        hold.food = food;
        hold.credits = credits;
        hold.artifacts = artifacts;
    }

    // --- raid params ----------------------------------------------------------

    #[test]
    fn test_raid_params_match_design() {
        let colony = raid_params(RaidTarget::Colony);
        assert!((colony.success_p - 0.70).abs() < f32::EPSILON);
        assert!((colony.heat_gain - 8.0).abs() < f32::EPSILON);
        let trader = raid_params(RaidTarget::Trader);
        assert!((trader.success_p - 0.55).abs() < f32::EPSILON);
        assert!((trader.heat_gain - 12.0).abs() < f32::EPSILON);
        let rival = raid_params(RaidTarget::Rival);
        assert!((rival.success_p - 0.45).abs() < f32::EPSILON);
        assert!((rival.heat_gain - 15.0).abs() < f32::EPSILON);
        assert!((rival.fail_hull_damage - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolve_raid_colony_success_spotted() {
        // loyalty 0.6 -> p = 0.76; roll 0.6 succeeds but is spotted.
        let o = resolve_raid(RaidTarget::Colony, 3, 0.6, 0.6, 100.0, 100.0);
        assert!(o.success);
        assert!(!o.clean_getaway);
        assert!((o.food - 19.0).abs() < 1e-5); // min(20, 10+9)
        assert!((o.credits - 15.0).abs() < 1e-5); // min(15, 20+15)
        assert_eq!(o.artifacts, 0);
        assert!((o.heat_gain - 8.0).abs() < f32::EPSILON);
        assert!((o.loyalty_delta - RAID_SUCCESS_LOYALTY).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolve_raid_colony_caps_at_poor_colony() {
        let o = resolve_raid(RaidTarget::Colony, 3, 0.6, 0.6, 10.0, 10.0);
        assert!(o.success);
        assert!((o.food - 2.0).abs() < 1e-5); // 20% of 10
        assert!((o.credits - 1.5).abs() < 1e-5); // 15% of 10
    }

    #[test]
    fn test_resolve_raid_clean_getaway_no_heat() {
        // roll 0.0 <= p - 0.25 -> nobody saw a thing.
        let o = resolve_raid(RaidTarget::Colony, 3, 0.6, 0.0, 100.0, 100.0);
        assert!(o.success);
        assert!(o.clean_getaway);
        assert!((o.heat_gain).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolve_raid_failure() {
        let o = resolve_raid(RaidTarget::Rival, 3, 0.6, 0.99, 100.0, 100.0);
        assert!(!o.success);
        assert!(!o.clean_getaway);
        assert!((o.food).abs() < f32::EPSILON);
        assert!((o.credits).abs() < f32::EPSILON);
        assert_eq!(o.artifacts, 0);
        assert!((o.heat_gain - 7.5).abs() < f32::EPSILON); // half of 15
        assert!((o.hull_damage - 20.0).abs() < f32::EPSILON);
        assert!((o.crew_damage - 15.0).abs() < f32::EPSILON);
        assert!((o.loyalty_delta + RAID_FAIL_LOYALTY).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolve_raid_rival_yields_artifacts() {
        let o = resolve_raid(RaidTarget::Rival, 3, 0.6, 0.1, 100.0, 100.0);
        assert!(o.success);
        assert_eq!(o.artifacts, 2); // 1 + 3/2
        assert!((o.food).abs() < f32::EPSILON);
        assert!((o.credits - 85.0).abs() < 1e-5); // 40 + 45
    }

    #[test]
    fn test_resolve_raid_trader_plunder() {
        let o = resolve_raid(RaidTarget::Trader, 2, 0.5, 0.1, 0.0, 0.0);
        assert!(o.success);
        assert!((o.food - 20.0).abs() < 1e-5); // 10 + 10
        assert!((o.credits - 50.0).abs() < 1e-5); // 30 + 20
    }

    // --- heat bands -------------------------------------------------------------

    #[test]
    fn test_heat_band_patrol_crossing() {
        let (band, events) = heat_band_events(30.0, 45.0, 0);
        assert_eq!(band, 1);
        assert_eq!(events, vec!["patrol"]);
    }

    #[test]
    fn test_heat_band_hunter_crossing() {
        let (band, events) = heat_band_events(45.0, 75.0, 1);
        assert_eq!(band, 2);
        assert_eq!(events, vec!["hunters"]);
    }

    #[test]
    fn test_heat_band_no_refire_within_band() {
        let (band, events) = heat_band_events(50.0, 60.0, 1);
        assert_eq!(band, 1);
        assert!(events.is_empty());
        let (band, events) = heat_band_events(80.0, 90.0, 2);
        assert_eq!(band, 2);
        assert!(events.is_empty());
    }

    #[test]
    fn test_add_heat_patrol_impounds_fine() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 0.0, 100.0, 0);
        add_heat(&mut world, 45.0);
        let state = world.resource::<CorsairState>();
        assert_eq!(state.heat_band, 1);
        let skiff = state.skiff.unwrap();
        let hold = world.get::<SkiffHold>(skiff).unwrap();
        assert!((hold.credits - 80.0).abs() < 1e-5); // 20% fine
    }

    #[test]
    fn test_add_heat_hunters_damage_and_steal() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 100.0, 0.0, 0);
        add_heat(&mut world, 75.0);
        let state = world.resource::<CorsairState>();
        assert_eq!(state.heat_band, 2);
        let skiff = state.skiff.unwrap();
        let hull = world.get::<SkiffHull>(skiff).unwrap();
        assert!((hull.hp - (SKIFF_HULL_MAX - HUNTER_HULL_DAMAGE)).abs() < 1e-5);
        let hold = world.get::<SkiffHold>(skiff).unwrap();
        assert!((hold.food - 70.0).abs() < 1e-5); // 30% stolen
    }

    // --- embark -----------------------------------------------------------------

    #[test]
    fn test_embark_grants_captain_and_boards_skiff() {
        let mut world = setup();
        let skiff = spawn_skiff(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Vex", 11, 10);
        let msg = try_embark_corsair(&mut world, pop).expect("embark");
        assert!(world.get::<CorsairCaptain>(pop).is_some());
        // The captain is no longer counted as crew (separate prize-law share).
        assert!(world.get::<CorsairCrew>(pop).is_none());
        assert_eq!(world.resource::<CorsairState>().captain, Some(pop));
        let pos = world.get::<GridPosition>(pop).unwrap();
        let spos = world.get::<GridPosition>(skiff).unwrap();
        assert_eq!((pos.x, pos.y), (spos.x, spos.y));
        assert!(msg.contains("captain's writ"));
    }

    #[test]
    fn test_embark_fails_when_skiff_far() {
        let mut world = setup();
        spawn_skiff(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Vex", 50, 50);
        assert!(try_embark_corsair(&mut world, pop).is_err());
        assert!(world.get::<CorsairCaptain>(pop).is_none());
    }

    #[test]
    fn test_embark_fails_on_hulk() {
        let mut world = setup();
        let skiff = spawn_skiff(&mut world, 10, 10);
        world.get_mut::<SkiffHull>(skiff).unwrap().hp = 0.0;
        let pop = spawn_pop(&mut world, "Vex", 11, 10);
        assert!(try_embark_corsair(&mut world, pop).is_err());
    }

    // --- plunder economy ------------------------------------------------------------

    #[test]
    fn test_divide_fair_shares() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 3);
        set_hold(&mut world, 0.0, 120.0, 0);
        let msg = divide_plunder(&mut world, captain).expect("divide");
        // 120 / (2 + 3 + 1) = 20 per share.
        assert!((world.get::<Wallet>(captain).unwrap().credits - 40.0).abs() < 1e-5);
        for e in crew {
            assert!((world.get::<Wallet>(e).unwrap().credits - 20.0).abs() < 1e-5);
            let loyalty = world.get::<CorsairCrew>(e).unwrap().loyalty;
            assert!((loyalty - (STARTING_LOYALTY + DIVIDE_FAIR_LOYALTY)).abs() < 1e-5);
        }
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        assert!((world.get::<ShipsPurse>(skiff).unwrap().credits - 20.0).abs() < 1e-5);
        assert!((world.get::<SkiffHold>(skiff).unwrap().credits).abs() < f32::EPSILON);
        assert!(msg.contains("fair shares"));
    }

    #[test]
    fn test_divide_notices_skimming() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 3);
        set_hold(&mut world, 0.0, 100.0, 0);
        skim_credits(&mut world, captain, 30.0).expect("skim");
        // 70 left in hold; skimmed 30/70 = 42.8% > 10% -> noticed.
        divide_plunder(&mut world, captain).expect("divide");
        for e in crew {
            let loyalty = world.get::<CorsairCrew>(e).unwrap().loyalty;
            assert!(loyalty < STARTING_LOYALTY, "loyalty should drop, got {loyalty}");
        }
        assert!((world.resource::<CorsairState>().skimmed).abs() < f32::EPSILON);
    }

    #[test]
    fn test_skim_moves_credits_and_tracks() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 0.0, 50.0, 0);
        skim_credits(&mut world, captain, 20.0).expect("skim");
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        assert!((world.get::<SkiffHold>(skiff).unwrap().credits - 30.0).abs() < 1e-5);
        assert!((world.get::<Wallet>(captain).unwrap().credits - 20.0).abs() < 1e-5);
        assert!((world.resource::<CorsairState>().skimmed - 20.0).abs() < f32::EPSILON);
        assert!(skim_credits(&mut world, captain, 40.0).is_err());
    }

    #[test]
    fn test_unload_moves_food_to_colony() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 25.0, 10.0, 2);
        unload_hold(&mut world, captain).expect("unload");
        // Default colony food is 10.0; the hold's 25.0 lands on top.
        assert!((world.resource::<ColonyResources>().food - 35.0).abs() < 1e-5);
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        let hold = world.get::<SkiffHold>(skiff).unwrap();
        assert!((hold.food).abs() < f32::EPSILON);
        assert!((hold.credits - 10.0).abs() < 1e-5); // credits stay for divide
        assert_eq!(hold.artifacts, 2); // artifacts stay for fence
        assert!(unload_hold(&mut world, captain).is_err()); // now empty
    }

    #[test]
    fn test_fence_needs_merchant() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 0.0, 0.0, 4);
        assert!(fence_plunder(&mut world, captain).is_err());
    }

    #[test]
    fn test_fence_converts_artifacts_with_merchant() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        set_hold(&mut world, 0.0, 0.0, 4);
        world.resource_mut::<MerchantState>().active_merchant = Some(Merchant {
            name: "Free Trader".to_string(),
            arrival_tick: 0,
            departure_tick: 9999,
            deals: vec![],
        });
        let msg = fence_plunder(&mut world, captain).expect("fence");
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        let hold = world.get::<SkiffHold>(skiff).unwrap();
        assert_eq!(hold.artifacts, 0);
        assert!((hold.credits - 100.0).abs() < 1e-5); // 4 * 25
        assert!((world.resource::<CorsairState>().heat - FENCE_HEAT).abs() < f32::EPSILON);
        assert!(msg.contains("100"));
    }

    #[test]
    fn test_repair_spends_purse() {
        let mut world = setup();
        let (captain, skiff) = make_captain(&mut world, 10, 10);
        world.get_mut::<SkiffHull>(skiff).unwrap().hp = 60.0;
        world.get_mut::<ShipsPurse>(skiff).unwrap().credits = 50.0;
        repair_skiff(&mut world, captain, Some(25.0)).expect("repair");
        assert!((world.get::<SkiffHull>(skiff).unwrap().hp - 85.0).abs() < 1e-5);
        assert!((world.get::<ShipsPurse>(skiff).unwrap().credits - 25.0).abs() < 1e-5);
        // Caps at max hull.
        repair_skiff(&mut world, captain, None).expect("repair");
        assert!((world.get::<SkiffHull>(skiff).unwrap().hp - SKIFF_HULL_MAX).abs() < 1e-5);
        assert!(repair_skiff(&mut world, captain, None).is_err()); // sound hull
    }

    // --- raid application -------------------------------------------------------------

    #[test]
    fn test_apply_raid_colony_moves_food_to_hold() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        spawn_crew(&mut world, 10, 10, 3);
        world.resource_mut::<ColonyResources>().food = 100.0;
        let outcome = resolve_raid(RaidTarget::Colony, 3, 0.6, 0.6, 100.0, 0.0);
        let text = apply_raid_outcome(&mut world, captain, RaidTarget::Colony, outcome);
        assert!((world.resource::<ColonyResources>().food - 81.0).abs() < 1e-4);
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        assert!((world.get::<SkiffHold>(skiff).unwrap().food - 19.0).abs() < 1e-4);
        assert!((world.resource::<CorsairState>().heat - 8.0).abs() < f32::EPSILON);
        assert!(text.contains("Boarding party"));
    }

    #[test]
    fn test_execute_raid_validates_captain_and_trader() {
        let mut world = setup();
        let (captain, _skiff) = make_captain(&mut world, 10, 10);
        let landlubber = spawn_pop(&mut world, "landlubber", 10, 10);
        assert!(execute_raid(&mut world, landlubber, RaidTarget::Colony).is_err());
        // Trader with no merchant present errors.
        assert!(execute_raid(&mut world, captain, RaidTarget::Trader).is_err());
    }

    // --- tick ----------------------------------------------------------------------------

    #[test]
    fn test_corsair_tick_decays_heat_and_rearms_bands() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        {
            let mut s = world.resource_mut::<CorsairState>();
            s.heat = 50.0;
            s.heat_band = 1;
        }
        corsair_tick(&mut world);
        let s = world.resource::<CorsairState>();
        assert!((s.heat - (50.0 - HEAT_DECAY_PER_TICK)).abs() < 1e-5);
        // Drop below the hysteresis floor -> band re-arms.
        {
            let mut s = world.resource_mut::<CorsairState>();
            s.heat = 20.0;
            s.heat_band = 1;
        }
        corsair_tick(&mut world);
        assert_eq!(world.resource::<CorsairState>().heat_band, 0);
    }

    #[test]
    fn test_corsair_tick_mutiny_removes_disloyal_crew() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 2);
        world.get_mut::<CorsairCrew>(crew[0]).unwrap().loyalty = 0.1;
        corsair_tick(&mut world);
        assert!(world.get::<CorsairCrew>(crew[0]).is_none());
        assert!(world.get::<CorsairCrew>(crew[1]).is_some());
    }

    #[test]
    fn test_corsair_tick_loyalty_drifts_home() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 1);
        world.get_mut::<CorsairCrew>(crew[0]).unwrap().loyalty = 0.9;
        corsair_tick(&mut world);
        let loyalty = world.get::<CorsairCrew>(crew[0]).unwrap().loyalty;
        assert!((loyalty - (0.9 - LOYALTY_DRIFT)).abs() < 1e-6);
    }

    #[test]
    fn test_corsair_tick_purse_upkeep_and_neglect() {        let mut world = setup();
        let (_captain, skiff) = make_captain(&mut world, 10, 10);
        world.get_mut::<ShipsPurse>(skiff).unwrap().credits = 1.0;
        world.get_mut::<SkiffHull>(skiff).unwrap().hp = 90.0;
        // Drain the purse over ticks; neglect then nibbles the hull.
        for _ in 0..60 {
            corsair_tick(&mut world);
        }
        assert!((world.get::<ShipsPurse>(skiff).unwrap().credits).abs() < 1e-4);
        assert!(world.get::<SkiffHull>(skiff).unwrap().hp < 90.0);
    }

    // --- stats ------------------------------------------------------------------------------

    #[test]
    fn test_corsair_tick_tethers_stragglers() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        // A crew pop that wandered far off is nudged one tile toward the skiff.
        let wanderer = spawn_pop(&mut world, "straggler", 20, 20);
        world.entity_mut(wanderer).insert(CorsairCrew {
            loyalty: STARTING_LOYALTY,
        });
        corsair_tick(&mut world);
        let pos = world.get::<GridPosition>(wanderer).unwrap();
        assert_eq!((pos.x, pos.y), (19, 19));
        // A crew pop already aboard is left alone.
        let homebody = spawn_pop(&mut world, "homebody", 11, 10);
        world.entity_mut(homebody).insert(CorsairCrew {
            loyalty: STARTING_LOYALTY,
        });
        corsair_tick(&mut world);
        let pos = world.get::<GridPosition>(homebody).unwrap();
        assert_eq!((pos.x, pos.y), (11, 10));
    }

    #[test]
    fn test_corsair_tick_mess_feeds_crew_from_hold() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 2);
        for e in &crew {
            world.get_mut::<Needs>(*e).unwrap().hunger = 0.5;
        }
        set_hold(&mut world, 10.0, 0.0, 0);
        corsair_tick(&mut world);
        for e in &crew {
            let hunger = world.get::<Needs>(*e).unwrap().hunger;
            assert!((hunger - 0.7).abs() < 1e-5);
        }
        let skiff = world.resource::<CorsairState>().skiff.unwrap();
        let hold = world.get::<SkiffHold>(skiff).unwrap();
        // Two crew plus the (also hungry) captain: three mouths.
        assert!((hold.food - (10.0 - 3.0 * CREW_MEAL_PER_TICK)).abs() < 1e-5);
    }

    #[test]
    fn test_corsair_tick_mess_starves_on_empty_hold() {        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        let crew = spawn_crew(&mut world, 10, 10, 1);
        world.get_mut::<Needs>(crew[0]).unwrap().hunger = 0.5;
        set_hold(&mut world, 0.0, 0.0, 0);
        corsair_tick(&mut world);
        // No food in the hold: the mess serves nothing.
        assert!((world.get::<Needs>(crew[0]).unwrap().hunger - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_corsair_tick_holds_pressure_envelope() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        world.insert_resource(PressureGrid::new(40, 40));
        // Vacuum everywhere before the tick.
        assert!((world.resource::<PressureGrid>().get(10, 10)).abs() < f32::EPSILON);
        corsair_tick(&mut world);
        let grid = world.resource::<PressureGrid>();
        // Envelope: radius SKIFF_PRESSURE_RADIUS around the skiff is 1.0.
        assert!((grid.get(10, 10) - 1.0).abs() < f32::EPSILON);
        assert!((grid.get(10 + SKIFF_PRESSURE_RADIUS, 10) - 1.0).abs() < f32::EPSILON);
        // Outside the envelope stays vacuum.
        assert!((grid.get(10 + SKIFF_PRESSURE_RADIUS + 2, 10)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_corsair_tick_warms_skiff_envelope_above_hypothermia() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        // Deep winter everywhere: -5C, below the 10C hypothermia threshold.
        world.insert_resource(TemperatureGrid::new(40, 40, -5.0));
        assert!(world.resource::<TemperatureGrid>().get_safe(10, 10) < 10.0);
        corsair_tick(&mut world);
        corsair_tick(&mut world);
        let grid = world.resource::<TemperatureGrid>();
        // The skiff envelope is pushed above the hypothermia threshold.
        assert!(
            grid.get_safe(10, 10) >= 10.0,
            "skiff tile must stay above the hypothermia threshold, got {}",
            grid.get_safe(10, 10)
        );
        assert!(
            grid.get_safe(10 + SKIFF_PRESSURE_RADIUS, 10) >= 10.0,
            "envelope edge must stay above the hypothermia threshold"
        );
        // Outside the envelope stays freezing.
        assert!(world.resource::<TemperatureGrid>().get_safe(10 + SKIFF_PRESSURE_RADIUS + 2, 10) < 10.0);
    }

    #[test]
    fn test_corsair_tick_never_overheats_summer_envelope() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        // Hot summer day: 33C, just under the 35C heatstroke threshold.
        world.insert_resource(TemperatureGrid::new(40, 40, 33.0));
        corsair_tick(&mut world);
        let t = world.resource::<TemperatureGrid>().get_safe(10, 10);
        assert!(
            t <= 35.0,
            "thermostat heat must never push a summer tile over heatstroke, got {t}"
        );
        assert!((t - 33.0).abs() < f32::EPSILON, "summer tiles are untouched");
    }

    #[test]
    fn test_corsair_stats_snapshot() {
        let mut world = setup();
        let (_captain, _skiff) = make_captain(&mut world, 10, 10);
        spawn_crew(&mut world, 10, 10, 3);
        add_heat(&mut world, 10.0);
        let (heat, hull, crew, loyalty) = corsair_stats(&mut world);
        assert!((heat - 10.0).abs() < f32::EPSILON);
        assert!((hull - SKIFF_HULL_MAX).abs() < f32::EPSILON);
        assert_eq!(crew, 3);
        assert!((loyalty - STARTING_LOYALTY).abs() < f32::EPSILON);
    }

    #[test]
    fn test_raid_target_parse() {
        assert_eq!(RaidTarget::parse("colony"), Some(RaidTarget::Colony));
        assert_eq!(RaidTarget::parse("TRADER"), Some(RaidTarget::Trader));
        assert_eq!(RaidTarget::parse("rival"), Some(RaidTarget::Rival));
        assert_eq!(RaidTarget::parse("moon"), None);
    }

    #[test]
    fn test_crew_names_unique() {
        let mut rng = rand::thread_rng();
        for _ in 0..20 {
            let names = draw_crew_names(&mut rng, CREW_SIZE);
            assert_eq!(names.len(), CREW_SIZE);
            let mut seen = std::collections::HashSet::new();
            for n in &names {
                assert!(seen.insert(n.clone()), "duplicate moniker: {n}");
            }
        }
    }
}
