//! The Bloom-Touched — adventurer origin #8.
//!
//! VanderMeer-flavor, concept-only, ORIGINAL NAMES ONLY. A lone expedition
//! scout ("the Touched") came back from the Bloom — an expanding,
//! shimmering anomalous zone that mutates flora, fauna, and pops —
//! changed. The scout waits beside the scar; a possessed pop within reach
//! may `interact` to take the bloom's path.
//!
//! - **`bloom`**: deliberately take on a mutation strain — a random
//!   beneficial/deforming trait draw tracked on the pop's mutations ledger.
//!   Each draw costs a little of the self.
//! - **`sporecast`**: release a spore puff that seeds/extends the Bloom
//!   field around your tile. The Bloom also creeps outward on its own.
//! - **The Bloom mutates anything that enters it** — crops grow strange and
//!   fast, fauna turn keen and mean, pops come out marked. Bloom tiles are
//!   unmappable: `survey` there reports that the map refuses, and the
//!   coordinates it gives are lies.
//! - **`embrace` / `resist`**: embrace the Bloom for escalating power
//!   (wider sporecasts, faster change) at the cost of a slow loss-of-self;
//!   resisting is costly (health and hunger) but holds the line. At
//!   full loss-of-self the Touched becomes Bloomkin — no longer quite
//!   a colonist.
//!
//! Headless console: `bloom`, `sporecast`, `embrace`, `resist`, `touched`
//! (ledger state); STATS gains `bloom=<tiles>t/<muts>m/<selfloss>`.
//!
//! The concept is concept-only inspiration. No named characters, places, or
//! distinctive IP anywhere — mechanics and vibes under original names.
//! "The Bloom", "the Touched", "Quill", "Bloomkin" are original coinages
//! for this game.

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashSet;

use crate::layer1::actions::AssignedTo;
use crate::layer1::agriculture::pollination::{FarmCrop, FarmGrowthStage};
use crate::layer1::biology::health::{DamageResistance, Health};
use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::direct_link::{possessed_entity, DirectControlState, Possessed};
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::fauna::Fauna;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::{Pop, PopBundle, PopName};
use crate::layer1::psychology::needs::Needs;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::StartPlan;
use crate::shared::time::SimulationTime;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Spawn ring (Chebyshev) around the lander where the bloom scar lands.
pub const BLOOM_RING_MIN: i32 = 6;
pub const BLOOM_RING_MAX: i32 = 10;
/// How close (Chebyshev) a pop must be to a bloom tile to take the path.
pub const BLOOM_TAKE_REACH: i32 = 2;
/// How far (Chebyshev) the dormant scout may drift from the scar seed
/// before the tick nudges it back (only while unpossessed).
pub const TOUCHED_TETHER_RADIUS: i32 = 3;
/// Base sporecast radius (Chebyshev). Grows with spore glands + embrace.
pub const SPORECAST_BASE_RADIUS: i32 = 2;
/// Hard cap on bloom tiles — the Bloom is patient, not infinite.
pub const BLOOM_MAX_TILES: usize = 400;
/// The Bloom creeps outward every this many ticks.
pub const EXPAND_EVERY_TICKS: u64 = 50;
/// Per-tile chance to seed a neighbor on an expansion tick.
pub const EXPAND_CHANCE: f64 = 0.10;
/// Self-loss taken per deliberate `bloom` draw.
pub const SELF_LOSS_PER_BLOOM: f32 = 5.0;
/// Self-loss taken per `sporecast`.
pub const SELF_LOSS_PER_CAST: f32 = 2.0;
/// Self-loss taken per `embrace`.
pub const SELF_LOSS_PER_EMBRACE: f32 = 10.0;
/// Passive self-loss per tick, scaled by embrace level.
pub const SELF_LOSS_CREEP_PER_TICK: f32 = 0.05;
/// Self-loss relieved by one `resist`.
pub const RESIST_RELIEF: f32 = 15.0;
/// Health cost of one `resist`.
pub const RESIST_HEALTH_COST: f32 = 10.0;
/// Hunger cost of one `resist`.
pub const RESIST_HUNGER_COST: f32 = 0.2;
/// Self-loss at which the Touched becomes Bloomkin.
pub const FULL_BLOOM_THRESHOLD: f32 = 100.0;
/// Damage dealt to an unmarked pop that steps into the Bloom.
pub const ENTRY_DAMAGE: f32 = 5.0;
/// Ticks between entry-mutation chronicles.
pub const ENTRY_CHRONICLE_COOLDOWN: u64 = 300;
/// Chance per tick that one spore-gland strain sheds a puff on its own.
pub const SHED_CHANCE_PER_TICK: f64 = 0.02;
/// The scout's name — an original coinage.
pub const TOUCHED_NAME: &str = "Quill";

/// Self-loss milestones that fire chronicles (with their texts).
const SELF_MILESTONES: [(f32, &str); 3] = [
    (25.0, EMBRACE_25_CHRONICLE),
    (50.0, EMBRACE_50_CHRONICLE),
    (75.0, EMBRACE_75_CHRONICLE),
];
/// Bloom-tile milestones that fire chronicles (with their texts).
const TILE_MILESTONES: [(usize, &str); 4] = [
    (25, TILE_MILESTONE_25_CHRONICLE),
    (50, TILE_MILESTONE_50_CHRONICLE),
    (100, TILE_MILESTONE_100_CHRONICLE),
    (200, TILE_MILESTONE_200_CHRONICLE),
];

// ---------------------------------------------------------------------------
// Components & state
// ---------------------------------------------------------------------------

/// A mutation strain the Bloom has written into the Touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutationStrain {
    PhotosyntheticSkin,
    ExtraJointLimbs,
    TranslucentFlesh,
    SporeGlands,
    MycelialNerves,
}

/// Marker on the dormant expedition scout, before the path is taken.
#[derive(Component, Debug, Clone, Copy)]
pub struct DormantTouched;

/// The Touched: a pop walking the bloom's path. The utility AI never
/// reassigns it (see the `Without<BloomTouched>` filter).
#[derive(Component, Debug, Clone)]
pub struct BloomTouched {
    /// Mutation strains taken, in draw order.
    pub mutations: Vec<MutationStrain>,
    /// 0.0 = fully human, 100.0 = Bloomkin.
    pub self_loss: f32,
    /// Times the Bloom has been embraced.
    pub embrace_level: u32,
    /// True once self-loss hit the threshold.
    pub fully_bloomed: bool,
    pub casts: u32,
    pub blooms_taken: u32,
    /// Index into [`SELF_MILESTONES`] already chronicled.
    pub self_milestone_idx: usize,
}

impl Default for BloomTouched {
    fn default() -> Self {
        Self {
            mutations: Vec::new(),
            self_loss: 0.0,
            embrace_level: 0,
            fully_bloomed: false,
            casts: 0,
            blooms_taken: 0,
            self_milestone_idx: 0,
        }
    }
}

/// Fully bloomed: no longer quite a colonist. The utility AI never
/// reassigns Bloomkin either.
#[derive(Component, Debug, Clone, Copy)]
pub struct BloomKin;

/// Marker for anything the Bloom has rewritten on entry (pop, crop, fauna).
#[derive(Component, Debug, Clone, Copy)]
pub struct BloomMutated;

/// The Bloom field itself: every tile it currently holds.
#[derive(Resource, Debug, Clone, Default)]
pub struct BloomZone {
    pub tiles: HashSet<(i32, i32)>,
    /// Where the scar first seeded (the dormant scout's tether).
    pub seed: Option<(i32, i32)>,
    pub total_cast: u32,
    pub expansions: u32,
    pub spawn_attempted: bool,
    /// Index into [`TILE_MILESTONES`] already chronicled.
    pub tile_milestone_idx: usize,
    pub entry_chronicle_cooldown_until: u64,
}

pub fn ensure_bloomtouched_resources(world: &mut World) {
    if world.get_resource::<BloomZone>().is_none() {
        world.insert_resource(BloomZone::default());
    }
    if world.get_resource::<Events<AddChronicleEvent>>().is_none() {
        world.insert_resource(Events::<AddChronicleEvent>::default());
    }
}

fn send_chronicle(world: &mut World, text: &str, importance: EventImportance) {
    ensure_bloomtouched_resources(world);
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

pub const BLOOM_SPAWN_CHRONICLE: &str =
    "Rumor: an expedition scout came back from the shimmering flats wrong. \
     Quill won't say what the Bloom showed them out there. The scar they \
     left behind keeps growing.";
pub const FIRST_BLOOM_CHRONICLE: &str =
    "Quill takes the Bloom's first gift deliberately. Their skin prickles \
     with new color. Somewhere behind their eyes, something small and \
     bright is filed away.";
pub const EMBRACE_25_CHRONICLE: &str =
    "Quill embraces the Bloom, and the Bloom embraces back a little too \
     tightly. Their reflection has started arriving a moment late.";
pub const EMBRACE_50_CHRONICLE: &str =
    "Half of Quill is weather now. They dream in spore-counts and wake up \
     knowing the names of winds that haven't blown yet.";
pub const EMBRACE_75_CHRONICLE: &str =
    "Quill's shadow has opinions. It leans toward the Bloom even when \
     Quill walks away. The colony has stopped asking if they're alright.";
pub const FULL_BLOOM_CHRONICLE: &str =
    "Quill is gone the way a tide goes — still there, just elsewhere. \
     What walks the Bloom now wears their face like a favorite coat. It \
     remembers being Quill the way you remember a song.";
pub const RESIST_CLEAN_CHRONICLE: &str =
    "Quill tears the Bloom's fingers out of their thoughts, one by one, \
     and it HURTS. But their name is their own again. It fits like a \
     good boot.";
pub const TILE_MILESTONE_25_CHRONICLE: &str =
    "The Bloom has taken twenty-five tiles of the map. The charts have \
     started disagreeing with each other.";
pub const TILE_MILESTONE_50_CHRONICLE: &str =
    "Fifty tiles. The Bloom is visible from orbit now, a bright bruise on \
     the planet's face.";
pub const TILE_MILESTONE_100_CHRONICLE: &str =
    "A hundred tiles of Bloom. Expeditions go in with rope and come back \
     with stories nobody believes.";
pub const TILE_MILESTONE_200_CHRONICLE: &str =
    "Two hundred tiles. The map has given up; the Bloom draws its own \
     borders now.";
pub const ENTRY_MUTATION_CHRONICLE_PRE: &str = " wanders into the Bloom and \
     comes out marked — skin bright, eyes wrong. ";
pub const ENTRY_MUTATION_CHRONICLE_POST: &str =
    "The medics call it a touch. The Bloom calls it a beginning.";

/// Every public-facing string in this module, for the IP-guard test.
pub fn public_strings() -> Vec<String> {
    let mut v: Vec<String> = [
        BLOOM_SPAWN_CHRONICLE,
        FIRST_BLOOM_CHRONICLE,
        EMBRACE_25_CHRONICLE,
        EMBRACE_50_CHRONICLE,
        EMBRACE_75_CHRONICLE,
        FULL_BLOOM_CHRONICLE,
        RESIST_CLEAN_CHRONICLE,
        TILE_MILESTONE_25_CHRONICLE,
        TILE_MILESTONE_50_CHRONICLE,
        TILE_MILESTONE_100_CHRONICLE,
        TILE_MILESTONE_200_CHRONICLE,
        ENTRY_MUTATION_CHRONICLE_PRE,
        ENTRY_MUTATION_CHRONICLE_POST,
        TOUCHED_NAME,
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();
    for s in all_strains() {
        v.push(strain_name(s).to_string());
        v.push(strain_blurb(s).to_string());
    }
    v
}

// ---------------------------------------------------------------------------
// Strain catalog
// ---------------------------------------------------------------------------

pub fn all_strains() -> [MutationStrain; 5] {
    [
        MutationStrain::PhotosyntheticSkin,
        MutationStrain::ExtraJointLimbs,
        MutationStrain::TranslucentFlesh,
        MutationStrain::SporeGlands,
        MutationStrain::MycelialNerves,
    ]
}

pub fn strain_name(strain: MutationStrain) -> &'static str {
    match strain {
        MutationStrain::PhotosyntheticSkin => "Photosynthetic skin",
        MutationStrain::ExtraJointLimbs => "Extra-joint limbs",
        MutationStrain::TranslucentFlesh => "Translucent flesh",
        MutationStrain::SporeGlands => "Spore glands",
        MutationStrain::MycelialNerves => "Mycelial nerves",
    }
}

pub fn strain_blurb(strain: MutationStrain) -> &'static str {
    match strain {
        MutationStrain::PhotosyntheticSkin => {
            "Skin drinks the light: hunger fades by day, but sleep comes \
             harder — the skin itches green."
        }
        MutationStrain::ExtraJointLimbs => {
            "New joints bloom at elbow and knee: wounds knit fast, and the \
             appetite grows to match."
        }
        MutationStrain::TranslucentFlesh => {
            "Flesh gone glass-clear, strange and euphoric — and fragile as \
             spun sugar in a storm."
        }
        MutationStrain::SporeGlands => {
            "Glands along the ribs breathe out the Bloom: sporecasts reach \
             further, and sometimes puff without asking."
        }
        MutationStrain::MycelialNerves => {
            "Nerves braided with pale thread: blows land softer, but the \
             nerves dream loudly and rest comes thin."
        }
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Snapshot of which tiles in a Chebyshev region are NOT bloomable
/// (water, void, or out of bounds). Lets callers hold the answer without
/// holding a borrow on the world.
fn unbloomable_in(world: &World, cx: i32, cy: i32, r: i32) -> HashSet<(i32, i32)> {
    let mut set = HashSet::new();
    let terrain = world.get_resource::<TerrainGrid>();
    for dx in -r..=r {
        for dy in -r..=r {
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 {
                set.insert((x, y));
                continue;
            }
            let bad = match terrain {
                Some(tg) => matches!(
                    tg.get(x as usize, y as usize),
                    Some(TerrainType::Water) | Some(TerrainType::Void) | None
                ),
                None => false,
            };
            if bad {
                set.insert((x, y));
            }
        }
    }
    set
}

/// Seed the 5-tile scar patch at (x, y). Shared by the ring-spawn and tests.
fn seed_patch(world: &mut World, x: i32, y: i32) {
    ensure_bloomtouched_resources(world);
    let mut zone = world.resource_mut::<BloomZone>();
    for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        zone.tiles.insert((x + dx, y + dy));
    }
    if zone.seed.is_none() {
        zone.seed = Some((x, y));
    }
}

/// Spawn the dormant scout beside the scar.
fn spawn_dormant_scout(world: &mut World, x: i32, y: i32) {
    ensure_bloomtouched_resources(world);
    let mut rng = rand::thread_rng();
    let mut bundle = PopBundle::random(x, y, &mut rng);
    bundle.name = PopName(TOUCHED_NAME.to_string());
    world.spawn((bundle, DormantTouched));
}

pub fn spawn_bloom_scar_once(world: &mut World) {
    ensure_bloomtouched_resources(world);
    if world.resource::<BloomZone>().spawn_attempted {
        return;
    }
    world.resource_mut::<BloomZone>().spawn_attempted = true;
    if !world.resource::<BloomZone>().tiles.is_empty() {
        return;
    }

    let lander = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(b, _)| b.building_type == BuildingType::Lander)
        .map(|(_, p)| *p);
    let Some(lander) = lander else { return };

    let target: Option<GridPosition> = {
        let unbloomable = unbloomable_in(world, lander.x, lander.y, BLOOM_RING_MAX + 3);
        let occupied: HashSet<(i32, i32)> = world
            .get_resource::<OccupiedTiles>()
            .map(|o| o.0.iter().copied().collect())
            .unwrap_or_default();
        let buildings: HashSet<(i32, i32)> = world
            .query::<(&Building, &GridPosition)>()
            .iter(world)
            .map(|(_, p)| (p.x, p.y))
            .collect();
        let mut found = None;
        'search: for radius in BLOOM_RING_MIN..=BLOOM_RING_MAX {
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
                    // The whole 5-tile patch must be bloomable.
                    let patch_ok = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .all(|(px, py)| !unbloomable.contains(&(x + px, y + py)));
                    if !patch_ok {
                        continue;
                    }
                    // The scout needs a free tile beside the scar.
                    let scout_spot = [(x + 2, y), (x - 2, y), (x, y + 2), (x, y - 2)]
                        .into_iter()
                        .find(|(sx, sy)| {
                            !unbloomable.contains(&(*sx, *sy))
                                && !occupied.contains(&(*sx, *sy))
                                && !buildings.contains(&(*sx, *sy))
                        });
                    if scout_spot.is_none() {
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
    seed_patch(world, pos.x, pos.y);
    spawn_dormant_scout(world, pos.x + 2, pos.y);
    send_chronicle(world, BLOOM_SPAWN_CHRONICLE, EventImportance::Minor);
}

/// Seed a scar patch + dormant scout at an explicit tile. For tests and
/// debugging; does not send the spawn chronicle.
pub fn spawn_bloom_at(world: &mut World, x: i32, y: i32) {
    seed_patch(world, x, y);
    spawn_dormant_scout(world, x + 2, y);
}

/// A bloom tile within take reach of a pop, if any.
pub fn bloom_within_reach(world: &mut World, pop: Entity) -> Option<(i32, i32)> {
    let pos = *world.get::<GridPosition>(pop)?;
    let zone = world.get_resource::<BloomZone>()?;
    zone.tiles.iter().copied().find(|(tx, ty)| {
        (tx - pos.x).abs() <= BLOOM_TAKE_REACH && (ty - pos.y).abs() <= BLOOM_TAKE_REACH
    })
}

pub fn is_bloom_tile(world: &mut World, x: i32, y: i32) -> bool {
    world
        .get_resource::<BloomZone>()
        .map(|z| z.tiles.contains(&(x, y)))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Origin actions
// ---------------------------------------------------------------------------

/// The possessed Bloom-Touched, if the player currently holds them.
fn possessed_touched(world: &mut World) -> Option<Entity> {
    let e = possessed_entity(world)?;
    world.get::<BloomTouched>(e).is_some().then_some(e)
}

fn require_touched(world: &mut World) -> Result<Entity, String> {
    possessed_touched(world).ok_or_else(|| {
        "No Bloom-Touched is possessed. Possess a pop, move beside the bloom \
         scar, and `interact` to take the path."
            .to_string()
    })
}

/// Add self-loss, firing milestone chronicles and the full-bloom
/// transformation when thresholds are crossed.
fn add_self_loss(world: &mut World, touched: Entity, amount: f32) {
    let prev = world
        .get::<BloomTouched>(touched)
        .map(|t| t.self_loss)
        .unwrap_or(0.0);
    let new = (prev + amount).min(120.0);
    {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        t.self_loss = new;
        while t.self_milestone_idx < SELF_MILESTONES.len()
            && new >= SELF_MILESTONES[t.self_milestone_idx].0
        {
            t.self_milestone_idx += 1;
        }
    }
    for (th, text) in SELF_MILESTONES {
        if prev < th && new >= th {
            let importance = if th >= 75.0 {
                EventImportance::Major
            } else {
                EventImportance::Minor
            };
            send_chronicle(world, text, importance);
        }
    }
    if prev < FULL_BLOOM_THRESHOLD && new >= FULL_BLOOM_THRESHOLD {
        complete_bloom(world, touched);
    }
}

/// The threshold crossing: the Touched becomes Bloomkin, and the Bloom
/// writes every remaining strain into them at once.
fn complete_bloom(world: &mut World, touched: Entity) {
    {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        for s in all_strains() {
            if !t.mutations.contains(&s) {
                t.mutations.push(s);
            }
        }
        t.fully_bloomed = true;
    }
    world.entity_mut(touched).insert(BloomKin);
    send_chronicle(world, FULL_BLOOM_CHRONICLE, EventImportance::Major);
}

/// Fire tile-milestone chronicles for growth since `prev` tiles.
fn check_tile_milestones(world: &mut World, prev: usize) {
    let (new, idx) = {
        let zone = world.resource::<BloomZone>();
        (zone.tiles.len(), zone.tile_milestone_idx)
    };
    for (i, (th, text)) in TILE_MILESTONES.iter().enumerate().skip(idx) {
        if prev < *th && new >= *th {
            send_chronicle(world, text, EventImportance::Minor);
            world.resource_mut::<BloomZone>().tile_milestone_idx = i + 1;
        }
    }
}

/// Take the bloom's path: a possessed pop beside the scar steps in, and
/// possession transfers to the waiting scout (mirroring the other
/// origins' component surgery). If the caller already possessed the
/// dormant scout directly, the path is taken in place.
pub fn try_take_bloom_path(world: &mut World, pop: Entity) -> Result<String, String> {
    ensure_bloomtouched_resources(world);
    if world
        .query_filtered::<Entity, With<BloomTouched>>()
        .iter(world)
        .next()
        .is_some()
    {
        return Err("Quill already walks the bloom's path — it is taken.".to_string());
    }
    bloom_within_reach(world, pop)
        .ok_or_else(|| "No bloom scar within reach. Move beside the Bloom.".to_string())?;
    let dormant = world
        .query_filtered::<Entity, With<DormantTouched>>()
        .iter(world)
        .next()
        .ok_or_else(|| "The scar is empty — its scout is gone.".to_string())?;

    let name = world
        .get::<PopName>(pop)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("pop #{}", pop.index()));

    if pop == dormant {
        // The caller already possessed the dormant scout directly: the
        // path is taken in place.
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
        .insert(BloomTouched::default())
        .remove::<DormantTouched>()
        .remove::<StartPlan>()
        .remove::<MovementTarget>()
        .remove::<AtTarget>()
        .remove::<AssignedTo>();

    send_chronicle(
        world,
        &format!(
            "{name} steps into the bloom scar, and the scar steps into \
             them. Quill is awake — the scout who came back wrong, and \
             came back anyway."
        ),
        EventImportance::Major,
    );
    Ok(format!(
        "{name} steps into the bloom scar. Quill unfolds — changed, \
         patient, already half weather. (`bloom` to take a strain, \
         `sporecast` to spread, `touched` for the ledger.)"
    ))
}

/// Deliberately take on a mutation strain: a random draw from the strains
/// not already held. Each draw costs a little of the self.
pub fn take_bloom_mutation(world: &mut World) -> Result<String, String> {
    ensure_bloomtouched_resources(world);
    let touched = require_touched(world)?;
    let held: Vec<MutationStrain> = world
        .get::<BloomTouched>(touched)
        .map(|t| t.mutations.clone())
        .unwrap_or_default();
    let pool: Vec<MutationStrain> = all_strains()
        .into_iter()
        .filter(|s| !held.contains(s))
        .collect();
    if pool.is_empty() {
        return Err(
            "The Bloom has nothing left to give — every strain is already \
             written."
                .to_string(),
        );
    }
    let mut rng = rand::thread_rng();
    let strain = *pool
        .choose(&mut rng)
        .expect("pool is non-empty");
    let (strain, first) = {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        t.mutations.push(strain);
        t.blooms_taken += 1;
        (strain, t.blooms_taken == 1)
    };
    // Mycelial nerves braid in at once: blows land softer from now on.
    if strain == MutationStrain::MycelialNerves {
        if world.get::<DamageResistance>(touched).is_none() {
            world
                .entity_mut(touched)
                .insert(DamageResistance::default());
        }
        if let Some(mut dr) = world.get_mut::<DamageResistance>(touched) {
            dr.physical = (dr.physical + 0.1).min(0.5);
        }
    }
    finish_bloom_draw(world, touched, strain, first)
}

/// Shared tail of a bloom draw: self-loss, first-draw chronicle, message.
fn finish_bloom_draw(
    world: &mut World,
    touched: Entity,
    strain: MutationStrain,
    first: bool,
) -> Result<String, String> {
    add_self_loss(world, touched, SELF_LOSS_PER_BLOOM);
    if first {
        send_chronicle(world, FIRST_BLOOM_CHRONICLE, EventImportance::Minor);
    }
    Ok(format!(
        "Quill takes the Bloom's gift: {} — {} (self-loss +{:.0})",
        strain_name(strain),
        strain_blurb(strain),
        SELF_LOSS_PER_BLOOM
    ))
}

/// Release a spore puff: seed/extend the Bloom around the Touched's tile.
/// Radius grows with spore-gland strains and embrace level.
pub fn sporecast(world: &mut World) -> Result<String, String> {
    ensure_bloomtouched_resources(world);
    let touched = require_touched(world)?;
    let (glands, embrace) = world
        .get::<BloomTouched>(touched)
        .map(|t| {
            (
                t.mutations
                    .iter()
                    .filter(|s| **s == MutationStrain::SporeGlands)
                    .count() as i32,
                t.embrace_level as i32,
            )
        })
        .unwrap_or((0, 0));
    let radius = SPORECAST_BASE_RADIUS + glands + embrace;
    let pos = *world
        .get::<GridPosition>(touched)
        .ok_or_else(|| "The Touched has no position.".to_string())?;
    let before = world.resource::<BloomZone>().tiles.len();
    let unbloomable = unbloomable_in(world, pos.x, pos.y, radius);
    {
        let mut zone = world.resource_mut::<BloomZone>();
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                if dx.abs().max(dy.abs()) > radius {
                    continue;
                }
                if zone.tiles.len() >= BLOOM_MAX_TILES {
                    break;
                }
                let (x, y) = (pos.x + dx, pos.y + dy);
                if !unbloomable.contains(&(x, y)) {
                    zone.tiles.insert((x, y));
                }
            }
        }
        zone.total_cast += 1;
    }
    let after = world.resource::<BloomZone>().tiles.len();
    check_tile_milestones(world, before);
    {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        t.casts += 1;
    }
    add_self_loss(world, touched, SELF_LOSS_PER_CAST);
    Ok(format!(
        "Quill breathes out. The Bloom takes {} new tiles ({} total).",
        after - before,
        after
    ))
}

/// Embrace the Bloom: escalating power (wider casts, faster change) for a
/// steepening loss of self.
pub fn embrace_bloom(world: &mut World) -> Result<String, String> {
    ensure_bloomtouched_resources(world);
    let touched = require_touched(world)?;
    if world
        .get::<BloomTouched>(touched)
        .map(|t| t.fully_bloomed)
        .unwrap_or(false)
    {
        return Err(
            "Quill is already fully bloomed — there is nothing left to \
             embrace."
                .to_string(),
        );
    }
    let level = {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        t.embrace_level += 1;
        t.embrace_level
    };
    add_self_loss(world, touched, SELF_LOSS_PER_EMBRACE);
    let loss = world
        .get::<BloomTouched>(touched)
        .map(|t| t.self_loss)
        .unwrap_or(0.0);
    Ok(format!(
        "Quill embraces the Bloom (embrace {level}). Power grows; the self \
         thins — self-loss {loss:.0}."
    ))
}

/// Resist the Bloom: costly (health and hunger) but pushes the self-loss
/// back down. Too late once fully bloomed.
pub fn resist_bloom(world: &mut World) -> Result<String, String> {
    ensure_bloomtouched_resources(world);
    let touched = require_touched(world)?;
    let (loss, bloomed) = world
        .get::<BloomTouched>(touched)
        .map(|t| (t.self_loss, t.fully_bloomed))
        .unwrap_or((0.0, false));
    if bloomed {
        return Err("Too late to resist — the Bloom is Quill now.".to_string());
    }
    if loss <= 0.0 {
        return Ok(
            "Quill is still, stubbornly, human. There is nothing to resist \
             today."
                .to_string(),
        );
    }
    if let Some(mut h) = world.get_mut::<Health>(touched) {
        h.take_damage(RESIST_HEALTH_COST);
    }
    if let Some(mut n) = world.get_mut::<Needs>(touched) {
        n.hunger = (n.hunger - RESIST_HUNGER_COST).max(0.0);
    }
    {
        let mut t = world
            .get_mut::<BloomTouched>(touched)
            .expect("touched exists");
        t.self_loss = (t.self_loss - RESIST_RELIEF).max(0.0);
    }
    let now = world
        .get::<BloomTouched>(touched)
        .map(|t| t.self_loss)
        .unwrap_or(0.0);
    if now <= 0.0 {
        send_chronicle(world, RESIST_CLEAN_CHRONICLE, EventImportance::Minor);
    }
    Ok(format!(
        "Quill tears free a little — self-loss {loss:.0} → {now:.0}. It \
         costs blood and breakfast."
    ))
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

/// Nudge the unpossessed dormant scout back toward the scar seed.
fn tether_dormant(world: &mut World) {
    let dormant = world
        .query_filtered::<Entity, With<DormantTouched>>()
        .iter(world)
        .next();
    let Some(d) = dormant else { return };
    if world.query::<&Possessed>().get(world, d).is_ok() {
        return;
    }
    let seed = world.resource::<BloomZone>().seed;
    let Some((sx, sy)) = seed else { return };
    let dpos = match world.get::<GridPosition>(d).copied() {
        Some(p) => p,
        None => return,
    };
    let dx = sx - dpos.x;
    let dy = sy - dpos.y;
    if dx.abs().max(dy.abs()) > TOUCHED_TETHER_RADIUS {
        if let Some(mut pos) = world.get_mut::<GridPosition>(d) {
            pos.x += dx.signum();
            pos.y += dy.signum();
        }
    }
}

/// Per-tick mechanical effects of the held mutation strains.
fn apply_mutation_effects(world: &mut World, touched: Entity) {
    let mutations = world
        .get::<BloomTouched>(touched)
        .map(|t| t.mutations.clone())
        .unwrap_or_default();
    for m in mutations {
        match m {
            MutationStrain::PhotosyntheticSkin => {
                if let Some(mut n) = world.get_mut::<Needs>(touched) {
                    n.hunger = (n.hunger + 0.06).min(1.0);
                    n.rest = (n.rest - 0.02).max(0.0);
                }
            }
            MutationStrain::ExtraJointLimbs => {
                if let Some(mut h) = world.get_mut::<Health>(touched) {
                    h.current = (h.current + 0.05).min(h.max);
                }
                if let Some(mut n) = world.get_mut::<Needs>(touched) {
                    n.hunger = (n.hunger - 0.03).max(0.0);
                }
            }
            MutationStrain::TranslucentFlesh => {
                if let Some(mut n) = world.get_mut::<Needs>(touched) {
                    n.leisure = (n.leisure + 0.04).min(1.0);
                }
                if let Some(mut h) = world.get_mut::<Health>(touched) {
                    h.current = (h.current - 0.02).max(1.0);
                }
            }
            MutationStrain::SporeGlands => {
                // Handled in sporegland_shed; the glands also widen
                // sporecast radius (see sporecast).
            }
            MutationStrain::MycelialNerves => {
                if let Some(mut n) = world.get_mut::<Needs>(touched) {
                    n.rest = (n.rest - 0.02).max(0.0);
                }
            }
        }
    }
}

/// Spore glands sometimes puff without asking: a 1-tile bloom seed on a
/// random neighboring tile.
fn sporegland_shed(world: &mut World, touched: Entity) {
    let glands = world
        .get::<BloomTouched>(touched)
        .map(|t| {
            t.mutations
                .iter()
                .filter(|s| **s == MutationStrain::SporeGlands)
                .count()
        })
        .unwrap_or(0);
    if glands == 0 {
        return;
    }
    let mut rng = rand::thread_rng();
    if !rng.gen_bool(SHED_CHANCE_PER_TICK * glands as f64) {
        return;
    }
    let pos = match world.get::<GridPosition>(touched).copied() {
        Some(p) => p,
        None => return,
    };
    let dx: i32 = rng.gen_range(-1..=1);
    let dy: i32 = rng.gen_range(-1..=1);
    if dx == 0 && dy == 0 {
        return;
    }
    let (x, y) = (pos.x + dx, pos.y + dy);
    let unbloomable = unbloomable_in(world, pos.x, pos.y, 1);
    if unbloomable.contains(&(x, y)) {
        return;
    }
    let before = world.resource::<BloomZone>().tiles.len();
    if before >= BLOOM_MAX_TILES {
        return;
    }
    world.resource_mut::<BloomZone>().tiles.insert((x, y));
    check_tile_milestones(world, before);
}

/// One expansion step: each bloom tile may seed a neighboring tile.
/// Exposed for deterministic tests (the tick drives it with thread_rng).
pub fn expand_bloom_tiles(world: &mut World, rng: &mut impl Rng) {
    ensure_bloomtouched_resources(world);
    let tiles: Vec<(i32, i32)> = world.resource::<BloomZone>().tiles.iter().copied().collect();
    if tiles.is_empty() {
        return;
    }
    let mut new_tiles = Vec::new();
    // One terrain snapshot for the whole candidate region (tiles can only
    // spread one step), so no borrow is held across the loop.
    let (min_x, max_x, min_y, max_y) = tiles.iter().fold(
        (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
        |(a, b, c, d), (x, y)| (a.min(*x), b.max(*x), c.min(*y), d.max(*y)),
    );
    let unbloomable = unbloomable_in(
        world,
        (min_x + max_x) / 2,
        (min_y + max_y) / 2,
        (max_x - min_x).max(max_y - min_y) / 2 + 2,
    );
    for (x, y) in tiles {
        if !rng.gen_bool(EXPAND_CHANCE) {
            continue;
        }
        let dx: i32 = rng.gen_range(-1..=1);
        let dy: i32 = rng.gen_range(-1..=1);
        if dx == 0 && dy == 0 {
            continue;
        }
        let (nx, ny) = (x + dx, y + dy);
        if !unbloomable.contains(&(nx, ny)) {
            new_tiles.push((nx, ny));
        }
    }
    if new_tiles.is_empty() {
        return;
    }
    let before = world.resource::<BloomZone>().tiles.len();
    {
        let mut zone = world.resource_mut::<BloomZone>();
        for t in new_tiles {
            if zone.tiles.len() >= BLOOM_MAX_TILES {
                break;
            }
            zone.tiles.insert(t);
        }
        zone.expansions += 1;
    }
    check_tile_milestones(world, before);
}

/// The Bloom rewrites whatever steps into it: unmarked pops take a shock
/// and come out marked; crops grow a stage stranger and faster; fauna turn
/// keen. The Touched, Bloomkin, and the waiting scout are spared — the
/// Bloom knows its own.
fn entry_mutation_pass(world: &mut World, tick: u64) {
    ensure_bloomtouched_resources(world);
    let tiles: HashSet<(i32, i32)> = world.resource::<BloomZone>().tiles.clone();
    if tiles.is_empty() {
        return;
    }

    // Pops.
    let pop_cands: Vec<(Entity, String)> = world
        .query::<(Entity, &Pop, &GridPosition, &PopName)>()
        .iter(world)
        .filter(|(e, _, p, _)| {
            tiles.contains(&(p.x, p.y))
                && world.get::<BloomMutated>(*e).is_none()
                && world.get::<BloomTouched>(*e).is_none()
                && world.get::<BloomKin>(*e).is_none()
                && world.get::<DormantTouched>(*e).is_none()
        })
        .map(|(e, _, _, n)| (e, n.0.clone()))
        .collect();
    for (e, _name) in &pop_cands {
        if let Some(mut h) = world.get_mut::<Health>(*e) {
            h.take_damage(ENTRY_DAMAGE);
        }
        world.entity_mut(*e).insert(BloomMutated);
    }

    // Crops: the Bloom hurries them along, strangely.
    let crop_cands: Vec<Entity> = world
        .query::<(Entity, &FarmCrop, &GridPosition)>()
        .iter(world)
        .filter(|(e, _, p)| {
            tiles.contains(&(p.x, p.y)) && world.get::<BloomMutated>(*e).is_none()
        })
        .map(|(e, _, _)| e)
        .collect();
    for e in crop_cands {
        if let Some(mut c) = world.get_mut::<FarmCrop>(e) {
            c.growth_stage = match c.growth_stage {
                FarmGrowthStage::Seedling => FarmGrowthStage::Flowering,
                FarmGrowthStage::Flowering => FarmGrowthStage::Harvestable,
                other => other,
            };
        }
        world.entity_mut(e).insert(BloomMutated);
    }

    // Fauna: bloom-maddened, keener senses.
    let fauna_cands: Vec<Entity> = world
        .query::<(Entity, &Fauna, &GridPosition)>()
        .iter(world)
        .filter(|(e, _, p)| {
            tiles.contains(&(p.x, p.y)) && world.get::<BloomMutated>(*e).is_none()
        })
        .map(|(e, _, _)| e)
        .collect();
    for e in fauna_cands {
        if let Some(mut f) = world.get_mut::<Fauna>(e) {
            f.detection_range += 4.0;
        }
        world.entity_mut(e).insert(BloomMutated);
    }

    if let Some((_, name)) = pop_cands.into_iter().next() {
        let until = world.resource::<BloomZone>().entry_chronicle_cooldown_until;
        if tick >= until {
            world.resource_mut::<BloomZone>().entry_chronicle_cooldown_until =
                tick + ENTRY_CHRONICLE_COOLDOWN;
            send_chronicle(
                world,
                &format!("{name}{ENTRY_MUTATION_CHRONICLE_PRE}{ENTRY_MUTATION_CHRONICLE_POST}"),
                EventImportance::Minor,
            );
        }
    }
}

pub fn bloomtouched_tick(world: &mut World) {
    ensure_bloomtouched_resources(world);
    let tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);

    let touched = world
        .query_filtered::<Entity, With<BloomTouched>>()
        .iter(world)
        .next();

    match touched {
        None => {
            // No taken path yet — tether the dormant scout to the scar.
            tether_dormant(world);
        }
        Some(t) => {
            apply_mutation_effects(world, t);
            sporegland_shed(world, t);
            // The embrace keeps a little more of the self every tick.
            let level = world
                .get::<BloomTouched>(t)
                .map(|x| x.embrace_level)
                .unwrap_or(0);
            if level > 0 {
                add_self_loss(world, t, SELF_LOSS_CREEP_PER_TICK * level as f32);
            }
            // Bloomkin knit themselves back together.
            if world.get::<BloomKin>(t).is_some() {
                if let Some(mut h) = world.get_mut::<Health>(t) {
                    h.current = (h.current + 0.1).min(h.max);
                }
            }
        }
    }

    if tick.is_multiple_of(EXPAND_EVERY_TICKS) {
        let mut rng = rand::thread_rng();
        expand_bloom_tiles(world, &mut rng);
    }
    entry_mutation_pass(world, tick);
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// One-line bloom state for STATS: `<tiles>t/<mutations>m/<selfloss>`.
pub fn bloom_stats(world: &mut World) -> String {
    let tiles = match world.get_resource::<BloomZone>() {
        Some(z) if !z.tiles.is_empty() => z.tiles.len(),
        _ => return "none".to_string(),
    };
    let (muts, loss) = world
        .query::<&BloomTouched>()
        .iter(world)
        .next()
        .map(|t| (t.mutations.len(), t.self_loss))
        .unwrap_or((0, 0.0));
    format!("{tiles}t/{muts}m/{loss:.0}")
}

/// Full ledger view for the `touched` command.
pub fn describe_touched(world: &mut World) -> String {
    let info = world
        .query::<(&BloomTouched, &PopName)>()
        .iter(world)
        .next()
        .map(|(t, n)| {
            (
                n.0.clone(),
                t.mutations.clone(),
                t.self_loss,
                t.embrace_level,
                t.fully_bloomed,
                t.casts,
                t.blooms_taken,
            )
        });
    let Some((name, mutations, loss, level, kin, casts, blooms)) = info else {
        return "No Bloom-Touched walks yet. Find the scar.".to_string();
    };
    let state_word = if kin { "BLOOMKIN" } else { "touched" };
    let filled = ((loss / FULL_BLOOM_THRESHOLD).clamp(0.0, 1.0) * 20.0) as usize;
    let bar = format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(20 - filled)
    );
    let mut lines = vec![
        format!("{name} — the Bloom-Touched ({state_word})"),
        format!("self-loss: {loss:.1}/{FULL_BLOOM_THRESHOLD:.0} {bar}"),
        format!("embrace level: {level} | sporecasts: {casts} | blooms taken: {blooms}"),
        "mutations:".to_string(),
    ];
    if mutations.is_empty() {
        lines.push(" - (none yet — `bloom` to take a strain)".to_string());
    }
    for s in &mutations {
        lines.push(format!(" - {} — {}", strain_name(*s), strain_blurb(*s)));
    }
    if kin {
        lines.push(
            "Quill is gone the way a tide goes. What remains answers to \
             the name out of politeness."
                .to_string(),
        );
    }
    lines.join("\n")
}

/// Surveying a bloom tile: the map refuses, and the coordinates lie.
pub fn bloom_survey_report(world: &mut World, pop: Entity) -> Option<String> {
    let pos = world.get::<GridPosition>(pop).copied()?;
    if !is_bloom_tile(world, pos.x, pos.y) {
        return None;
    }
    let tick = world
        .get_resource::<SimulationTime>()
        .map(|t| t.tick)
        .unwrap_or(0);
    let lie_x = pos.x + (tick as i32 * 7 + 13).rem_euclid(5) - 2;
    let lie_y = pos.y + (tick as i32 * 11 + 29).rem_euclid(5) - 2;
    Some(format!(
        "SURVEY REFUSED — the Bloom will not be mapped. The compass spins, \
         the ink slides off the page, and the chart insists you stand at \
         ({lie_x}, {lie_y}), which is a lie the Bloom told politely. \
         Nothing in the Bloom is where the map says it is."
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    use rand::SeedableRng;

    // --- scaffolding ---------------------------------------------------------

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world.insert_resource(BloomZone::default());
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

    fn spawn_lander(world: &mut World, x: i32, y: i32) {
        world.spawn((
            Building {
                building_type: BuildingType::Lander,
            },
            GridPosition { x, y },
        ));
    }

    /// Take the bloom's path the honest way: possess a pop beside the scar
    /// and step in. Returns (waker, touched).
    fn take_direct(world: &mut World, x: i32, y: i32) -> (Entity, Entity) {
        let pop = spawn_pop(world, "Waker", x, y);
        world.entity_mut(pop).insert(Possessed);
        spawn_bloom_at(world, 10, 10);
        try_take_bloom_path(world, pop).expect("take should succeed");
        let touched = world
            .query_filtered::<Entity, With<BloomTouched>>()
            .iter(world)
            .next()
            .expect("touched exists");
        (pop, touched)
    }

    fn self_loss_of(world: &mut World) -> f32 {
        world
            .query::<&BloomTouched>()
            .iter(world)
            .next()
            .map(|t| t.self_loss)
            .unwrap_or(-1.0)
    }

    fn tile_count(world: &World) -> usize {
        world.resource::<BloomZone>().tiles.len()
    }

    // --- spawn -----------------------------------------------------------------

    #[test]
    fn red_spawn_scar_once_seeds_patch_and_scout() {
        let mut world = setup();
        spawn_lander(&mut world, 10, 10);
        spawn_bloom_scar_once(&mut world);
        assert!(tile_count(&world) >= 5, "a scar patch seeds");
        let dormant = world
            .query_filtered::<Entity, With<DormantTouched>>()
            .iter(&world)
            .next();
        assert!(dormant.is_some(), "the scout waits beside the scar");
        let name = world.get::<PopName>(dormant.unwrap()).unwrap();
        assert_eq!(name.0, TOUCHED_NAME);
        // Second call is a no-op.
        let before = tile_count(&world);
        spawn_bloom_scar_once(&mut world);
        assert_eq!(tile_count(&world), before);
    }

    // --- taking the path ----------------------------------------------------------

    #[test]
    fn red_take_path_transfers_possession() {
        let mut world = setup();
        let (pop, touched) = take_direct(&mut world, 10, 10);
        assert!(world.get::<Possessed>(pop).is_none(), "waker released");
        assert!(world.get::<Possessed>(touched).is_some());
        assert!(world.get::<BloomTouched>(touched).is_some());
        assert!(world.get::<DormantTouched>(touched).is_none());
        assert_eq!(world.get::<PopName>(touched).unwrap().0, TOUCHED_NAME);
    }

    #[test]
    fn red_take_path_twice_is_refused() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        let other = spawn_pop(&mut world, "Second", 10, 10);
        world.entity_mut(other).insert(Possessed);
        let err = try_take_bloom_path(&mut world, other).unwrap_err();
        assert!(err.contains("already"), "got: {err}");
    }

    #[test]
    fn red_take_path_out_of_reach_is_refused() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let far = spawn_pop(&mut world, "Far", 0, 0);
        world.entity_mut(far).insert(Possessed);
        let err = try_take_bloom_path(&mut world, far).unwrap_err();
        assert!(err.contains("reach"), "got: {err}");
    }

    // --- bloom draws -----------------------------------------------------------------

    #[test]
    fn red_bloom_draw_adds_mutation_and_self_loss() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        let msg = take_bloom_mutation(&mut world).expect("draw succeeds");
        let t = world.query::<&BloomTouched>().iter(&world).next().unwrap();
        assert_eq!(t.mutations.len(), 1);
        assert!((t.self_loss - SELF_LOSS_PER_BLOOM).abs() < 0.001);
        assert_eq!(t.blooms_taken, 1);
        assert!(!msg.is_empty());
    }

    #[test]
    fn red_bloom_draw_exhausts_the_pool() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        for _ in 0..5 {
            take_bloom_mutation(&mut world).expect("draw succeeds");
        }
        let t = world.query::<&BloomTouched>().iter(&world).next().unwrap();
        assert_eq!(t.mutations.len(), 5);
        let mut uniq = t.mutations.clone();
        uniq.sort_by_key(|s| *s as u8);
        uniq.dedup();
        assert_eq!(uniq.len(), 5, "no duplicate strains");
        let err = take_bloom_mutation(&mut world).unwrap_err();
        assert!(err.contains("nothing left"), "got: {err}");
    }

    #[test]
    fn red_bloom_requires_the_touched() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Plain", 10, 10);
        world.entity_mut(pop).insert(Possessed);
        let err = take_bloom_mutation(&mut world).unwrap_err();
        assert!(err.contains("Touched"), "got: {err}");
    }

    // --- sporecast -----------------------------------------------------------------------

    #[test]
    fn red_sporecast_extends_the_bloom() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        let before = tile_count(&world);
        assert_eq!(before, 5);
        let msg = sporecast(&mut world).expect("cast succeeds");
        let after = tile_count(&world);
        assert!(after >= 20, "radius-2 cast seeds ~25 tiles, got {after}");
        assert!((self_loss_of(&mut world) - SELF_LOSS_PER_CAST).abs() < 0.001);
        assert!(msg.contains("new tiles"));
    }

    #[test]
    fn red_sporecast_radius_grows_with_embrace() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        embrace_bloom(&mut world).expect("embrace");
        embrace_bloom(&mut world).expect("embrace");
        sporecast(&mut world).expect("cast");
        // radius 2 + 0 glands + 2 embrace = 4 -> 81 tiles on open grass
        assert!(tile_count(&world) >= 60, "got {}", tile_count(&world));
    }

    // --- embrace / resist -------------------------------------------------------------------

    #[test]
    fn red_embrace_raises_level_and_self_loss() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        embrace_bloom(&mut world).expect("embrace");
        let t = world.query::<&BloomTouched>().iter(&world).next().unwrap();
        assert_eq!(t.embrace_level, 1);
        assert!((t.self_loss - SELF_LOSS_PER_EMBRACE).abs() < 0.001);
    }

    #[test]
    fn red_full_bloom_at_threshold() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        for _ in 0..10 {
            embrace_bloom(&mut world).expect("embrace");
        }
        let t = world.get::<BloomTouched>(touched).unwrap();
        assert!(t.fully_bloomed, "self-loss hit 100");
        assert!(world.get::<BloomKin>(touched).is_some());
        assert_eq!(t.mutations.len(), 5, "the Bloom completes you");
        let err = embrace_bloom(&mut world).unwrap_err();
        assert!(err.contains("already"), "got: {err}");
    }

    #[test]
    fn red_resist_reduces_self_loss_at_cost() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        embrace_bloom(&mut world).expect("embrace");
        embrace_bloom(&mut world).expect("embrace");
        assert!((self_loss_of(&mut world) - 20.0).abs() < 0.001);
        resist_bloom(&mut world).expect("resist");
        assert!((self_loss_of(&mut world) - 5.0).abs() < 0.001);
        let health = world.get::<Health>(touched).unwrap();
        assert!((health.current - 90.0).abs() < 0.001);
        let needs = world.get::<Needs>(touched).unwrap();
        assert!((needs.hunger - 0.6).abs() < 0.001);
    }

    #[test]
    fn red_resist_at_zero_is_free_flavor() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        let msg = resist_bloom(&mut world).expect("resist at zero");
        assert!(msg.contains("human"), "got: {msg}");
        assert!((world.get::<Health>(touched).unwrap().current - 100.0).abs() < 0.001);
    }

    #[test]
    fn red_resist_after_full_bloom_is_refused() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        for _ in 0..10 {
            embrace_bloom(&mut world).expect("embrace");
        }
        let err = resist_bloom(&mut world).unwrap_err();
        assert!(err.contains("Too late"), "got: {err}");
    }

    // --- tick effects --------------------------------------------------------------------------

    #[test]
    fn red_tick_applies_photosynthetic_regen() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        world.entity_mut(touched).insert(BloomTouched {
            mutations: vec![MutationStrain::PhotosyntheticSkin],
            ..Default::default()
        });
        world.get_mut::<Needs>(touched).unwrap().hunger = 0.5;
        bloomtouched_tick(&mut world);
        let hunger = world.get::<Needs>(touched).unwrap().hunger;
        assert!(hunger > 0.5, "photosynthesis feeds, got {hunger}");
    }

    #[test]
    fn red_tick_applies_joint_regen_and_appetite() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        world.entity_mut(touched).insert(BloomTouched {
            mutations: vec![MutationStrain::ExtraJointLimbs],
            ..Default::default()
        });
        world.get_mut::<Health>(touched).unwrap().current = 50.0;
        bloomtouched_tick(&mut world);
        let health = world.get::<Health>(touched).unwrap().current;
        let hunger = world.get::<Needs>(touched).unwrap().hunger;
        assert!(health > 50.0, "joints knit, got {health}");
        assert!(hunger < 0.8, "bigger appetite, got {hunger}");
    }

    #[test]
    fn red_sporegland_auto_shed() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        world.entity_mut(touched).insert(BloomTouched {
            mutations: vec![MutationStrain::SporeGlands],
            ..Default::default()
        });
        world.resource_mut::<SimulationTime>().tick = 1; // not an expansion tick
        for _ in 0..500 {
            bloomtouched_tick(&mut world);
        }
        assert!(tile_count(&world) > 5, "glands shed on their own");
    }

    #[test]
    fn red_expansion_step_grows_zone() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        for seed in 0..20u64 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            expand_bloom_tiles(&mut world, &mut rng);
        }
        assert!(tile_count(&world) > 5, "the Bloom creeps");
    }

    #[test]
    fn red_self_loss_creeps_with_embrace() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        embrace_bloom(&mut world).expect("embrace");
        let before = self_loss_of(&mut world);
        bloomtouched_tick(&mut world);
        assert!(self_loss_of(&mut world) > before, "the Bloom keeps a little more");
    }

    #[test]
    fn red_dormant_tethered_to_scar() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let dormant = world
            .query_filtered::<Entity, With<DormantTouched>>()
            .iter(&world)
            .next()
            .unwrap();
        world.get_mut::<GridPosition>(dormant).unwrap().x = 0;
        world.get_mut::<GridPosition>(dormant).unwrap().y = 0;
        for _ in 0..8 {
            bloomtouched_tick(&mut world);
        }
        let pos = world.get::<GridPosition>(dormant).unwrap();
        let d = (pos.x - 10).abs().max((pos.y - 10).abs());
        assert!(d <= TOUCHED_TETHER_RADIUS, "tether holds, d={d}");
    }

    // --- entry mutation ---------------------------------------------------------------------------

    #[test]
    fn red_entry_mutates_pop() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Wanderer", 10, 10);
        bloomtouched_tick(&mut world);
        assert!(world.get::<BloomMutated>(pop).is_some());
        let health = world.get::<Health>(pop).unwrap().current;
        assert!((health - (100.0 - ENTRY_DAMAGE)).abs() < 0.001, "got {health}");
    }

    #[test]
    fn red_entry_mutates_crop() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let crop = world
            .spawn((
                FarmCrop {
                    growth_stage: FarmGrowthStage::Seedling,
                    requires_pollination: false,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();
        bloomtouched_tick(&mut world);
        assert!(world.get::<BloomMutated>(crop).is_some());
        let stage = world.get::<FarmCrop>(crop).unwrap().growth_stage;
        assert_eq!(stage, FarmGrowthStage::Flowering);
    }

    #[test]
    fn red_entry_mutates_fauna() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let beast = world
            .spawn((Fauna::default(), GridPosition { x: 10, y: 10 }))
            .id();
        bloomtouched_tick(&mut world);
        assert!(world.get::<BloomMutated>(beast).is_some());
        let range = world.get::<Fauna>(beast).unwrap().detection_range;
        assert!(range > 8.0, "bloom-maddened, got {range}");
    }

    #[test]
    fn red_entry_spares_the_touched() {
        let mut world = setup();
        let (_pop, touched) = take_direct(&mut world, 10, 10);
        // Walk the touched onto the scar itself.
        world.get_mut::<GridPosition>(touched).unwrap().x = 10;
        world.get_mut::<GridPosition>(touched).unwrap().y = 10;
        bloomtouched_tick(&mut world);
        assert!(world.get::<BloomMutated>(touched).is_none());
        assert!((world.get::<Health>(touched).unwrap().current - 100.0).abs() < 0.001);
    }

    // --- survey -----------------------------------------------------------------------------------------

    #[test]
    fn red_survey_refuses_bloom_tile() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Mapper", 10, 10);
        let report = bloom_survey_report(&mut world, pop).expect("refused");
        assert!(report.to_lowercase().contains("refus"), "got: {report}");
        assert!(!report.contains("(10, 10)"), "coords must lie, got: {report}");
    }

    #[test]
    fn red_survey_silent_off_bloom() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let pop = spawn_pop(&mut world, "Mapper", 0, 0);
        assert!(bloom_survey_report(&mut world, pop).is_none());
    }

    // --- views ----------------------------------------------------------------------------------------------

    #[test]
    fn red_describe_touched_lists_ledger() {
        let mut world = setup();
        let (_pop, _touched) = take_direct(&mut world, 10, 10);
        for _ in 0..5 {
            take_bloom_mutation(&mut world).expect("draw");
        }
        let desc = describe_touched(&mut world);
        assert!(desc.contains(TOUCHED_NAME), "got: {desc}");
        for s in all_strains() {
            assert!(desc.contains(strain_name(s)), "missing {s:?}");
        }
    }

    #[test]
    fn red_bloom_stats_format() {
        let mut world = setup();
        spawn_bloom_at(&mut world, 10, 10);
        let stats = bloom_stats(&mut world);
        assert!(stats.starts_with("5t/"), "got: {stats}");
        let mut empty = setup();
        assert_eq!(bloom_stats(&mut empty), "none");
    }

    // --- IP guard ----------------------------------------------------------------------------------------------

    #[test]
    fn ip_guard_no_banned_terms() {
        let banned = [
            "annihilation",
            "area x",
            "southern reach",
            "biologist",
            "surveyor",
            "psychologist",
            "anthropologist",
            "linguist",
            "crawler",
            "vandermeer",
        ];
        for s in public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
