//! Spec 434: The Parasitic Habitation Block.
//!
//! If housing quality drops too low while population density is high, a
//! "Phantom Block" forms: a physical slum that does not appear on the
//! official registry. Pops living there consume no colony resources but
//! produce nothing, and steadily accumulate extreme radical ideologies.
//!
//! ## Real seams (audit-first; the spec sketch types were mocks)
//!
//! * Housing (Spec 007): the real shelter component is
//!   [`Housing`][crate::layer1::architecture::housing::Housing] on
//!   buildings — `capacity` plus a `residents` registry. There is no
//!   housing-quality field and no `HousingStats` resource in the real
//!   codebase; both are built here, computed from real entities.
//! * Room quality (Spec 064): the real quality seam is
//!   [`calculate_room_quality`][crate::layer1::architecture::room_quality::calculate_room_quality]
//!   (space + beauty, enclosure-scaled; a bare unzoned room scores ~0,
//!   Labor "awful" is < 10, "dull" < 25). `quality_avg` is the mean of
//!   that function over every real housing building's tile; a colony
//!   with no housing at all reads 0.0 — the worst possible housing.
//! * Density: `pop_density` is pops per housing slot, scaled by 100
//!   (the sketch's > 100 line): at or under capacity it reads <= 100,
//!   overcrowded colonies read above it, and a housing-less colony
//!   reads `pops * 100`.
//! * Moving averages (spec REFACTOR): [`HousingStats`] is refreshed
//!   every tick by [`update_housing_stats_system`] as an exponential
//!   moving average (alpha [`STATS_EMA_ALPHA`]) of the instantaneous
//!   values, so a single bad tick never forms a block — and formation
//!   additionally requires the bad conditions to persist for
//!   [`PhantomBlockDirector::formation_ticks_required`] consecutive
//!   ticks. Slums are sustained neglect, not a bad week.
//! * Off the registry (spec REFACTOR): a Phantom Block is a standalone
//!   entity carrying [`PhantomBlock`] + `GridPosition` and deliberately
//!   NO `Building` component, so it never enters the building registry
//!   or inspector. Its site is a real reserved tile: walkable terrain
//!   near the colony heart with no building on it.
//! * Residents (spec section 7): the worst-off move in first — homeless
//!   pops (in no housing registry) before housed pops, housed pops
//!   ordered by their room's real quality, worst first. Moving in
//!   removes a housed pop from its old housing's registry, freeing the
//!   official slot.
//! * Consume nothing, produce nothing (spec section 5): residents are
//!   excluded from the real food pipeline —
//!   `consume_food_system` carries
//!   `Without<ResidentInPhantomBlock>` — and from the work pipeline:
//!   the utility AI's `collect_pop_data` carries the same filter (the
//!   Spec 1371 Panic / Spec 361 DigitalGhost pattern). Residents still
//!   exist in the physics simulation: position, needs decay, health and
//!   morale all keep ticking; they simply draw nothing from the colony
//!   ledger and take no jobs.
//! * Civic ideology (Spec 197) + Unrest (spec section 7): the real
//!   civic-ideology system is colony-wide ([`ActiveIdeology`]), so
//!   per-pop radicalization is a new component,
//!   [`IdeologyRadicalization`], ticking at the sketch's +5/tick. At
//!   [`RIOT_THRESHOLD`] a block's residents riot exactly once: a
//!   [`RadicalRiotEvent`], a +0.5 timed modifier on the real global
//!   [`Unrest`] resource, a Major chronicle entry, and every resident
//!   suffers a Violent mental break through the real
//!   [`MentalState`] seam.
//! * Demolition (spec section 8, architect's answer): yes, the player
//!   can demolish a Phantom Block — and doing so instantly triggers the
//!   riot. The generic `DesignationType::Demolish` executor operates on
//!   `Building` entities, and a Phantom Block has none by design (it is
//!   off-registry), so the designation cannot target it; the clean seam
//!   is [`DemolishPhantomBlockEvent`], driven in headless by
//!   `phantom demolish`. Demolition fires the riot first, then removes
//!   the block; the former residents remain as rioters.
//!
//! All names are original and generic ("Phantom Block" is the spec's
//! own generic name); see the IP-guard test.
//!
//! [`ActiveIdeology`]: super::civic_ideology::ActiveIdeology

use bevy_ecs::prelude::*;

use super::unrest::{MentalBreakType, MentalState, Unrest, UnrestModifier};
use crate::layer1::architecture::building::Building;
use crate::layer1::architecture::housing::Housing;
use crate::layer1::architecture::room_quality::calculate_room_quality;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;

/// Sketch threshold: average housing quality below this is slum-grade.
pub const QUALITY_THRESHOLD: f32 = 20.0;
/// Sketch threshold: population density above this is overcrowded.
pub const DENSITY_THRESHOLD: f32 = 100.0;
/// Radicalization gained per tick by a block resident (spec sketch).
pub const RADICALIZATION_PER_TICK: f32 = 5.0;
/// Radicalization at which a block's residents riot (spec section 7:
/// extreme radicalization eventually triggers a rebellion).
pub const RIOT_THRESHOLD: f32 = 500.0;
/// Maximum pops living in one Phantom Block.
pub const BLOCK_CAPACITY: usize = 4;
/// Consecutive bad-condition ticks before a block forms by default.
pub const FORMATION_TICKS_REQUIRED: u32 = 200;
/// EMA weight of the newest housing sample (spec REFACTOR: moving
/// averages for density and quality).
pub const STATS_EMA_ALPHA: f32 = 0.2;
/// Global unrest added by a riot (real `Unrest` runs 0.0–1.0).
pub const RIOT_UNREST_VALUE: f32 = 0.5;
/// Ticks a riot's unrest modifier lasts.
pub const RIOT_UNREST_DURATION: u32 = 2000;
/// Fallback block site when there are no pops and no terrain grid.
pub const FALLBACK_SITE: GridPosition = GridPosition { x: 40, y: 25 };

/// Colony housing conditions, refreshed every tick as a moving average
/// over real housing entities (spec sections 4–5, REFACTOR leg). Tests
/// and debug tooling may insert values directly; the evaluate system
/// reads this resource as-is.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct HousingStats {
    /// Moving average of real room quality over housing buildings.
    pub quality_avg: f32,
    /// Moving average of pops per housing slot, scaled by 100.
    pub pop_density: f32,
    /// Living pops in the latest instantaneous sample.
    pub pop_count: usize,
    /// Total housing capacity in the latest instantaneous sample.
    pub housing_capacity: usize,
    /// Instantaneous samples folded into the averages so far.
    pub samples: u64,
}

/// A Phantom Block: a physical slum that is not on the official
/// registry. The entity also carries a `GridPosition` (its reserved
/// tile) and deliberately no `Building` component.
#[derive(Component, Debug, Clone)]
pub struct PhantomBlock {
    /// Reserved tile at the block's heart.
    pub center: GridPosition,
    /// Tick the block formed.
    pub founded_tick: u64,
    /// Whether this block's residents have already rioted (a block
    /// riots once organically; demolition can always force it).
    pub riot_triggered: bool,
}

/// Marks a pop as living in a Phantom Block. Residents consume no
/// colony resources and take no jobs, but remain in the physics sim.
#[derive(Component, Debug, Clone, Copy)]
pub struct ResidentInPhantomBlock {
    /// The block entity this pop lives in.
    pub block: Entity,
}

/// Per-pop radical ideology accumulation (spec section 4). The real
/// civic-ideology system is colony-wide, so this per-pop leg is new.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct IdeologyRadicalization {
    /// Current radicalization level; +5/tick while resident.
    pub level: f32,
}

/// Fired when a Phantom Block's residents riot — organically at
/// [`RIOT_THRESHOLD`], or instantly on demolition (spec section 8).
#[derive(Event, Debug, Clone)]
pub struct RadicalRiotEvent {
    /// The block whose residents are rioting.
    pub block: Entity,
    /// The rioting pops (the block's residents at trigger time).
    pub residents: Vec<Entity>,
    /// True when the riot was triggered by demolition.
    pub demolished: bool,
}

/// Request to demolish a Phantom Block (spec section 8). Fires the
/// riot first, then removes the block.
#[derive(Event, Debug, Clone, Copy)]
pub struct DemolishPhantomBlockEvent {
    /// The block to demolish.
    pub block: Entity,
}

/// Tuning knobs for the Phantom Block pipeline. Tests override these.
#[derive(Resource, Debug, Clone, Copy)]
pub struct PhantomBlockDirector {
    /// Quality below this counts as slum-grade.
    pub quality_threshold: f32,
    /// Density above this counts as overcrowded.
    pub density_threshold: f32,
    /// Radicalization gained per resident per tick.
    pub radicalization_per_tick: f32,
    /// Radicalization at which residents riot.
    pub riot_threshold: f32,
    /// Maximum residents per block.
    pub block_capacity: usize,
    /// Consecutive bad ticks required before a block forms.
    pub formation_ticks_required: u32,
}

impl Default for PhantomBlockDirector {
    fn default() -> Self {
        Self {
            quality_threshold: QUALITY_THRESHOLD,
            density_threshold: DENSITY_THRESHOLD,
            radicalization_per_tick: RADICALIZATION_PER_TICK,
            riot_threshold: RIOT_THRESHOLD,
            block_capacity: BLOCK_CAPACITY,
            formation_ticks_required: FORMATION_TICKS_REQUIRED,
        }
    }
}

/// Ledger for the Phantom Block pipeline: formation persistence and
/// riot/demolition counts for STATS and tests.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PhantomBlockState {
    /// Consecutive ticks conditions have been bad enough to form.
    pub bad_ticks: u32,
    /// Riots fired (organic + demolition).
    pub riots: u32,
    /// Blocks demolished.
    pub demolitions: u32,
    /// Tick of the most recent riot.
    pub last_riot_tick: u64,
}

fn director(world: &World) -> PhantomBlockDirector {
    world
        .get_resource::<PhantomBlockDirector>()
        .copied()
        .unwrap_or_default()
}

fn chronicle(world: &mut World, text: String) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

fn log_line(world: &mut World, text: String) {
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(text);
    }
}

/// Instantaneous housing conditions from real entities: mean real
/// room quality over housing buildings (0.0 when there is no housing),
/// and pops per housing slot scaled by 100.
#[must_use]
pub fn compute_housing_conditions(world: &mut World) -> (f32, f32, usize, usize) {
    let pop_count = {
        let mut query = world.query_filtered::<Entity, With<Pop>>();
        query.iter(world).count()
    };
    let housing: Vec<(GridPosition, usize)> = {
        let mut query = world.query::<(&Housing, &GridPosition)>();
        query
            .iter(world)
            .map(|(h, pos)| (*pos, h.capacity))
            .collect()
    };
    let capacity: usize = housing.iter().map(|(_, c)| *c).sum();
    let quality_avg = if housing.is_empty() {
        0.0
    } else {
        // calculate_room_quality takes &mut World; positions were
        // collected above so no query is live while it runs.
        let total: f32 = housing
            .iter()
            .map(|(pos, _)| calculate_room_quality(world, *pos))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let avg = total / housing.len() as f32;
        avg
    };
    #[allow(clippy::cast_precision_loss)]
    let pop_density = if capacity == 0 {
        pop_count as f32 * 100.0
    } else {
        pop_count as f32 * 100.0 / capacity as f32
    };
    (quality_avg, pop_density, pop_count, capacity)
}

/// System: refresh [`HousingStats`] as a moving average of the real
/// colony (spec REFACTOR). The first sample snaps to the instantaneous
/// values; later samples fold in with [`STATS_EMA_ALPHA`].
pub fn update_housing_stats_system(world: &mut World) {
    let (quality, density, pops, capacity) = compute_housing_conditions(world);
    if world.get_resource::<HousingStats>().is_none() {
        world.insert_resource(HousingStats::default());
    }
    let mut stats = world.resource_mut::<HousingStats>();
    if stats.samples == 0 {
        stats.quality_avg = quality;
        stats.pop_density = density;
    } else {
        stats.quality_avg += (quality - stats.quality_avg) * STATS_EMA_ALPHA;
        stats.pop_density += (density - stats.pop_density) * STATS_EMA_ALPHA;
    }
    stats.pop_count = pops;
    stats.housing_capacity = capacity;
    stats.samples += 1;
}

/// Mean position of living pops: the colony heart. Falls back to
/// [`FALLBACK_SITE`] when there are no pops.
fn colony_heart(world: &mut World) -> GridPosition {
    let positions: Vec<GridPosition> = {
        let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
        query.iter(world).copied().collect()
    };
    if positions.is_empty() {
        return FALLBACK_SITE;
    }
    let (sx, sy) = positions
        .iter()
        .fold((0i64, 0i64), |(x, y), p| (x + i64::from(p.x), y + i64::from(p.y)));
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    GridPosition {
        x: (sx / positions.len() as i64) as i32,
        y: (sy / positions.len() as i64) as i32,
    }
}

/// Reserve a real tile for a new block (spec REFACTOR): walkable
/// terrain near the colony heart with no building and no other block
/// on it. Falls back to the heart itself when nothing qualifies.
fn find_block_site(world: &mut World) -> GridPosition {
    let heart = colony_heart(world);
    let occupied: std::collections::HashSet<(i32, i32)> = {
        let mut set = std::collections::HashSet::new();
        let mut buildings = world.query_filtered::<&GridPosition, With<Building>>();
        for pos in buildings.iter(world) {
            set.insert((pos.x, pos.y));
        }
        let mut blocks = world.query_filtered::<&GridPosition, With<PhantomBlock>>();
        for pos in blocks.iter(world) {
            set.insert((pos.x, pos.y));
        }
        set
    };
    let terrain = world.get_resource::<TerrainGrid>();
    for radius in 0..=12i32 {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dy.abs()) != radius {
                    continue;
                }
                let candidate = GridPosition {
                    x: heart.x + dx,
                    y: heart.y + dy,
                };
                if occupied.contains(&(candidate.x, candidate.y)) {
                    continue;
                }
                if let Some(grid) = terrain {
                    let (Ok(ux), Ok(uy)) = (
                        usize::try_from(candidate.x),
                        usize::try_from(candidate.y),
                    ) else {
                        continue;
                    };
                    if !grid.get(ux, uy).is_some_and(TerrainType::is_walkable) {
                        continue;
                    }
                }
                return candidate;
            }
        }
    }
    heart
}

/// Spawn a Phantom Block entity on a reserved tile. Deliberately no
/// `Building` component: the block is not on the official registry.
fn spawn_phantom_block(world: &mut World) -> Entity {
    let site = find_block_site(world);
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    let block = world
        .spawn((
            PhantomBlock {
                center: site,
                founded_tick: tick,
                riot_triggered: false,
            },
            site,
        ))
        .id();
    log_line(
        world,
        format!(
            "PHANTOM BLOCK: an unregistered slum has formed at ({}, {}). Its residents draw nothing from the colony ledger — and give nothing back.",
            site.x, site.y
        ),
    );
    chronicle(
        world,
        format!(
            "THE PHANTOM BLOCK: with housing at its worst and the colony overcrowded, an unregistered slum has formed at ({}, {}). It appears on no official registry, and the pops moving in have vanished from the colony's books.",
            site.x, site.y
        ),
    );
    block
}

/// Force-form a Phantom Block now, bypassing housing conditions.
/// Debug/headless helper; returns false if a block already exists.
pub fn force_form_phantom_block(world: &mut World) -> bool {
    if phantom_block_count(world) > 0 {
        return false;
    }
    spawn_phantom_block(world);
    true
}

/// System: if average housing quality is low while population density
/// is high — sustained for the director's formation window — a
/// Phantom Block forms (spec sections 4–6). Only one block at a time.
pub fn evaluate_housing_conditions_system(world: &mut World) {
    let cfg = director(world);
    let (quality, density) = world.get_resource::<HousingStats>().map_or(
        (QUALITY_THRESHOLD, 0.0),
        |s| (s.quality_avg, s.pop_density),
    );
    let bad = quality < cfg.quality_threshold && density > cfg.density_threshold;
    if world.get_resource::<PhantomBlockState>().is_none() {
        world.insert_resource(PhantomBlockState::default());
    }
    let ready = {
        let mut state = world.resource_mut::<PhantomBlockState>();
        if bad {
            state.bad_ticks = state.bad_ticks.saturating_add(1);
        } else {
            state.bad_ticks = 0;
        }
        bad && state.bad_ticks >= cfg.formation_ticks_required.max(1)
    };
    if ready && phantom_block_count(world) == 0 {
        spawn_phantom_block(world);
    }
}

/// System: the worst-off pops move into each block until it is full —
/// homeless pops first, then residents of the lowest-quality housing
/// (spec section 7). Moving in frees the pop's official housing slot.
pub fn assign_phantom_residents_system(world: &mut World) {
    let cfg = director(world);
    let blocks: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<PhantomBlock>>();
        query.iter(world).collect()
    };
    if blocks.is_empty() {
        return;
    }
    // Official registry snapshot: pop -> housing entity, and each
    // housing building's real room quality (worst housing loses its
    // residents to the block first).
    let mut housed: std::collections::HashMap<Entity, Entity> = std::collections::HashMap::new();
    let mut housing_quality: std::collections::HashMap<Entity, f32> =
        std::collections::HashMap::new();
    let housing_rows: Vec<(Entity, GridPosition, Vec<Entity>)> = {
        let mut query = world.query::<(Entity, &Housing, &GridPosition)>();
        query
            .iter(world)
            .map(|(e, h, pos)| (e, *pos, h.residents.clone()))
            .collect()
    };
    for (housing_entity, pos, residents) in &housing_rows {
        let quality = calculate_room_quality(world, *pos);
        housing_quality.insert(*housing_entity, quality);
        for resident in residents {
            housed.insert(*resident, *housing_entity);
        }
    }
    let current_residents: std::collections::HashSet<Entity> = {
        let mut query = world.query_filtered::<Entity, With<ResidentInPhantomBlock>>();
        query.iter(world).collect()
    };
    let candidates: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<Pop>>();
        query
            .iter(world)
            .filter(|e| !current_residents.contains(e))
            .collect()
    };
    // Worst-off first: homeless (no registry entry) before housed;
    // housed ordered by their room's quality, worst first.
    let mut candidates = candidates;
    candidates.sort_by(|a, b| {
        let key = |e: &Entity| -> (u8, f32) {
            match housed.get(e) {
                None => (0, 0.0),
                Some(h) => (1, housing_quality.get(h).copied().unwrap_or(0.0)),
            }
        };
        let (ga, qa) = key(a);
        let (gb, qb) = key(b);
        ga.cmp(&gb)
            .then_with(|| qa.partial_cmp(&qb).unwrap_or(std::cmp::Ordering::Equal))
    });
    for block in blocks {
        let mut filled = {
            let mut query = world.query::<&ResidentInPhantomBlock>();
            query.iter(world).filter(|r| r.block == block).count()
        };
        for candidate in &candidates {
            if filled >= cfg.block_capacity {
                break;
            }
            if world.get::<ResidentInPhantomBlock>(*candidate).is_some() {
                continue;
            }
            // Moving in leaves the official registry: free the slot.
            if let Some(housing_entity) = housed.get(candidate).copied() {
                if let Some(mut housing) = world.get_mut::<Housing>(housing_entity) {
                    housing.residents.retain(|r| r != candidate);
                }
            }
            world.entity_mut(*candidate).insert((
                ResidentInPhantomBlock { block },
                IdeologyRadicalization { level: 0.0 },
            ));
            filled += 1;
        }
    }
}

/// System: pops with `ResidentInPhantomBlock` steadily increase in
/// `IdeologyRadicalization` (spec section 4: +5 per tick).
pub fn phantom_block_radicalization_system(
    mut query: Query<&mut IdeologyRadicalization, With<ResidentInPhantomBlock>>,
    cfg: Option<Res<PhantomBlockDirector>>,
) {
    let rate = cfg
        .map(|c| c.radicalization_per_tick)
        .unwrap_or(RADICALIZATION_PER_TICK);
    for mut ideology in &mut query {
        ideology.level += rate;
    }
}

/// Fire a block's riot: event, real global unrest, chronicle, and a
/// Violent mental break for every resident. Shared by the organic
/// threshold path and by demolition (spec sections 7–8).
fn trigger_riot(world: &mut World, block: Entity, demolished: bool) {
    let residents: Vec<Entity> = {
        let mut query = world.query::<(Entity, &ResidentInPhantomBlock)>();
        query
            .iter(world)
            .filter(|(_, r)| r.block == block)
            .map(|(e, _)| e)
            .collect()
    };
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    if let Some(mut b) = world.get_mut::<PhantomBlock>(block) {
        b.riot_triggered = true;
    }
    if world.get_resource::<PhantomBlockState>().is_none() {
        world.insert_resource(PhantomBlockState::default());
    }
    {
        let mut state = world.resource_mut::<PhantomBlockState>();
        state.riots += 1;
        state.last_riot_tick = tick;
    }
    if let Some(mut events) = world.get_resource_mut::<Events<RadicalRiotEvent>>() {
        events.send(RadicalRiotEvent {
            block,
            residents: residents.clone(),
            demolished,
        });
    }
    if let Some(mut unrest) = world.get_resource_mut::<Unrest>() {
        unrest.modifiers.push(UnrestModifier {
            value: RIOT_UNREST_VALUE,
            duration: RIOT_UNREST_DURATION,
            label: "Phantom Block Riot".to_string(),
        });
    }
    for resident in &residents {
        world
            .entity_mut(*resident)
            .insert(MentalState::Broken(MentalBreakType::Violent));
    }
    let cause = if demolished {
        "Demolition crews moved against the Phantom Block — and its residents answered as one."
    } else {
        "Radicalization in the Phantom Block has boiled over."
    };
    log_line(
        world,
        format!(
            "RADICAL RIOT: {} {} resident(s) are rioting.",
            cause,
            residents.len()
        ),
    );
    chronicle(
        world,
        format!(
            "RADICAL RIOT: {cause} {} resident(s) of the unregistered slum have taken to violence, and unrest grips the colony.",
            residents.len()
        ),
    );
}

/// System: extreme radicalization eventually triggers a rebellion
/// (spec section 7). A block riots organically exactly once, when any
/// resident crosses the director's riot threshold.
pub fn phantom_block_riot_system(world: &mut World) {
    let cfg = director(world);
    let blocks: Vec<Entity> = {
        let mut query = world.query::<(Entity, &PhantomBlock)>();
        query
            .iter(world)
            .filter(|(_, b)| !b.riot_triggered)
            .map(|(e, _)| e)
            .collect()
    };
    if blocks.is_empty() {
        return;
    }
    let max_by_block: std::collections::HashMap<Entity, f32> = {
        let mut map: std::collections::HashMap<Entity, f32> = std::collections::HashMap::new();
        let mut query = world.query::<(&ResidentInPhantomBlock, &IdeologyRadicalization)>();
        for (resident, ideology) in query.iter(world) {
            let entry = map.entry(resident.block).or_insert(0.0);
            *entry = entry.max(ideology.level);
        }
        map
    };
    for block in blocks {
        if max_by_block.get(&block).copied().unwrap_or(0.0) >= cfg.riot_threshold {
            trigger_riot(world, block, false);
        }
    }
}

/// System: demolishing a Phantom Block instantly triggers its
/// residents to riot (spec section 8), then removes the block. The
/// former residents keep their radicalization and their rage; they
/// are simply no longer residents of anything.
pub fn demolish_phantom_block_system(world: &mut World) {
    let requests: Vec<Entity> = {
        let Some(mut events) = world.get_resource_mut::<Events<DemolishPhantomBlockEvent>>()
        else {
            return;
        };
        events.drain().map(|e| e.block).collect()
    };
    for block in requests {
        if world.get::<PhantomBlock>(block).is_none() {
            continue;
        }
        trigger_riot(world, block, true);
        let residents: Vec<Entity> = {
            let mut query = world.query::<(Entity, &ResidentInPhantomBlock)>();
            query
                .iter(world)
                .filter(|(_, r)| r.block == block)
                .map(|(e, _)| e)
                .collect()
        };
        for resident in residents {
            world
                .entity_mut(resident)
                .remove::<ResidentInPhantomBlock>();
        }
        world.despawn(block);
        if world.get_resource::<PhantomBlockState>().is_none() {
            world.insert_resource(PhantomBlockState::default());
        }
        world.resource_mut::<PhantomBlockState>().demolitions += 1;
        log_line(
            world,
            "PHANTOM BLOCK DEMOLISHED: the unregistered slum is rubble. Its former residents are on the streets — and furious.".to_string(),
        );
        chronicle(
            world,
            "THE BLOCK DEMOLISHED: the Phantom Block has been torn down. The riot it triggered still burns.".to_string(),
        );
    }
}

/// Number of Phantom Blocks currently formed.
#[must_use]
pub fn phantom_block_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<PhantomBlock>>();
    query.iter(world).count()
}

/// Number of pops currently living in a Phantom Block.
#[must_use]
pub fn phantom_resident_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<ResidentInPhantomBlock>>();
    query.iter(world).count()
}

/// Highest radicalization level among block residents (0.0 if none).
#[must_use]
pub fn phantom_max_radicalization(world: &mut World) -> f32 {
    let mut query = world.query_filtered::<&IdeologyRadicalization, With<ResidentInPhantomBlock>>();
    query.iter(world).map(|i| i.level).fold(0.0, f32::max)
}

/// The first Phantom Block entity, if any (headless demolish target).
#[must_use]
pub fn first_phantom_block(world: &mut World) -> Option<Entity> {
    let mut query = world.query_filtered::<Entity, With<PhantomBlock>>();
    query.iter(world).next()
}

/// Short status for headless STATS: "none" or
/// "blocks<N>:residents<M>:maxrad<R>:riots<K>".
#[must_use]
pub fn phantom_block_status_label(world: &mut World) -> String {
    let blocks = phantom_block_count(world);
    if blocks == 0 {
        return "none".to_string();
    }
    let residents = phantom_resident_count(world);
    let maxrad = phantom_max_radicalization(world);
    let riots = world
        .get_resource::<PhantomBlockState>()
        .map_or(0, |s| s.riots);
    format!("blocks{blocks}:residents{residents}:maxrad{maxrad:.0}:riots{riots}")
}

/// Public-facing strings, for the IP-guard test.
#[must_use]
pub fn phantom_block_public_strings() -> Vec<String> {
    vec![
        "PHANTOM BLOCK: an unregistered slum has formed.".to_string(),
        "THE PHANTOM BLOCK: with housing at its worst and the colony overcrowded, an unregistered slum has formed. It appears on no official registry, and the pops moving in have vanished from the colony's books.".to_string(),
        "RADICAL RIOT: Radicalization in the Phantom Block has boiled over.".to_string(),
        "RADICAL RIOT: Demolition crews moved against the Phantom Block — and its residents answered as one.".to_string(),
        "PHANTOM BLOCK DEMOLISHED: the unregistered slum is rubble. Its former residents are on the streets — and furious.".to_string(),
        "THE BLOCK DEMOLISHED: the Phantom Block has been torn down. The riot it triggered still burns.".to_string(),
        "Phantom Block Riot".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::balance::FOOD_PER_MEAL;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::needs::Needs;
    use bevy_app::App;
    use bevy_ecs::system::RunSystemOnce;

    fn fast_director() -> PhantomBlockDirector {
        PhantomBlockDirector {
            formation_ticks_required: 1,
            ..PhantomBlockDirector::default()
        }
    }

    fn base_world() -> World {
        let mut world = World::new();
        world.insert_resource(HousingStats::default());
        world.insert_resource(fast_director());
        world.insert_resource(PhantomBlockState::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<RadicalRiotEvent>>();
        world.init_resource::<Events<DemolishPhantomBlockEvent>>();
        world.insert_resource(Unrest::default());
        world.insert_resource(SimulationTime::default());
        world
    }

    /// Spec test 1 (adapted to real types): low quality + high density
    /// forms exactly one Phantom Block, and it is off the registry
    /// (no Building component).
    #[test]
    fn test_phantom_block_formation() {
        let mut app = App::new();
        app.insert_resource(HousingStats {
            quality_avg: 10.0,
            pop_density: 150.0,
            ..HousingStats::default()
        });
        app.insert_resource(fast_director());
        app.insert_resource(PhantomBlockState::default());
        app.init_resource::<Events<AddChronicleEvent>>();
        app.add_systems(bevy_app::Update, evaluate_housing_conditions_system);

        app.update();

        let blocks = app
            .world_mut()
            .query::<&PhantomBlock>()
            .iter(app.world())
            .count();
        assert_eq!(blocks, 1);
        // Off-registry: the block entity carries no Building component.
        let with_building = app
            .world_mut()
            .query_filtered::<Entity, (With<PhantomBlock>, With<Building>)>()
            .iter(app.world())
            .count();
        assert_eq!(with_building, 0);
    }

    #[test]
    fn test_no_formation_when_housing_is_good() {
        let mut world = base_world();
        world.insert_resource(HousingStats {
            quality_avg: 60.0,
            pop_density: 40.0,
            ..HousingStats::default()
        });
        world.run_system_once(evaluate_housing_conditions_system).unwrap();
        assert_eq!(phantom_block_count(&mut world), 0);
    }

    #[test]
    fn test_formation_requires_sustained_conditions() {
        let mut world = base_world();
        world.insert_resource(PhantomBlockDirector {
            formation_ticks_required: 3,
            ..PhantomBlockDirector::default()
        });
        world.insert_resource(HousingStats {
            quality_avg: 10.0,
            pop_density: 150.0,
            ..HousingStats::default()
        });
        world.run_system_once(evaluate_housing_conditions_system).unwrap();
        world.run_system_once(evaluate_housing_conditions_system).unwrap();
        assert_eq!(phantom_block_count(&mut world), 0);
        world.run_system_once(evaluate_housing_conditions_system).unwrap();
        assert_eq!(phantom_block_count(&mut world), 1);
        // A second block never forms while one stands.
        world.run_system_once(evaluate_housing_conditions_system).unwrap();
        assert_eq!(phantom_block_count(&mut world), 1);
    }

    /// Spec test 2 (adapted): a resident's radicalization rises by
    /// exactly +5 per tick.
    #[test]
    fn test_phantom_block_radicals() {
        let mut app = App::new();
        app.add_systems(bevy_app::Update, phantom_block_radicalization_system);

        let block = app.world_mut().spawn(PhantomBlock {
            center: GridPosition { x: 0, y: 0 },
            founded_tick: 0,
            riot_triggered: false,
        }).id();
        let entity = app
            .world_mut()
            .spawn((
                ResidentInPhantomBlock { block },
                IdeologyRadicalization { level: 0.0 },
            ))
            .id();

        app.update();

        let ideology = app.world().get::<IdeologyRadicalization>(entity).unwrap();
        assert_eq!(ideology.level, 5.0);
        app.update();
        let ideology = app.world().get::<IdeologyRadicalization>(entity).unwrap();
        assert_eq!(ideology.level, 10.0);
    }

    #[test]
    fn test_compute_housing_conditions_from_real_entities() {
        let mut world = World::new();
        // Two pops, one housing slot: density = 2 * 100 / 1 = 200.
        world.spawn((Pop, GridPosition { x: 0, y: 0 }));
        world.spawn((Pop, GridPosition { x: 1, y: 0 }));
        world.spawn((
            Housing {
                capacity: 1,
                residents: Vec::new(),
            },
            GridPosition { x: 2, y: 0 },
        ));
        let (quality, density, pops, capacity) = compute_housing_conditions(&mut world);
        assert_eq!(pops, 2);
        assert_eq!(capacity, 1);
        assert!((density - 200.0).abs() < f32::EPSILON);
        // No room grids in this world: quality reads the floor, 0.0.
        assert!((quality - 0.0).abs() < f32::EPSILON);

        // No housing at all: density = pops * 100.
        let mut homeless_world = World::new();
        homeless_world.spawn((Pop, GridPosition { x: 0, y: 0 }));
        let (_, density, _, capacity) = compute_housing_conditions(&mut homeless_world);
        assert_eq!(capacity, 0);
        assert!((density - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_residents_assigned_homeless_first_and_registry_freed() {
        let mut world = base_world();
        world.insert_resource(PhantomBlockDirector {
            block_capacity: 1,
            ..fast_director()
        });
        let block = world
            .spawn((
                PhantomBlock {
                    center: GridPosition { x: 5, y: 5 },
                    founded_tick: 0,
                    riot_triggered: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let housed_pop = world.spawn((Pop, GridPosition { x: 0, y: 0 })).id();
        let homeless_pop = world.spawn((Pop, GridPosition { x: 1, y: 0 })).id();
        let housing = world
            .spawn((
                Housing {
                    capacity: 2,
                    residents: vec![housed_pop],
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        world.run_system_once(assign_phantom_residents_system).unwrap();

        // Capacity 1: the homeless pop moves in first.
        let resident = world.get::<ResidentInPhantomBlock>(homeless_pop).unwrap();
        assert_eq!(resident.block, block);
        assert!(world.get::<ResidentInPhantomBlock>(housed_pop).is_none());
        assert!(world.get::<IdeologyRadicalization>(homeless_pop).is_some());

        // Raise capacity: the housed pop moves in and its official
        // housing slot is freed.
        world.insert_resource(PhantomBlockDirector {
            block_capacity: 2,
            ..fast_director()
        });
        world.run_system_once(assign_phantom_residents_system).unwrap();
        assert!(world.get::<ResidentInPhantomBlock>(housed_pop).is_some());
        assert!(world.get::<Housing>(housing).unwrap().residents.is_empty());
    }

    #[test]
    fn test_radicalization_threshold_triggers_riot_once() {
        let mut world = base_world();
        let block = world
            .spawn((
                PhantomBlock {
                    center: GridPosition { x: 5, y: 5 },
                    founded_tick: 0,
                    riot_triggered: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let resident = world
            .spawn((
                Pop,
                ResidentInPhantomBlock { block },
                IdeologyRadicalization { level: RIOT_THRESHOLD },
                MentalState::Normal,
            ))
            .id();

        world.run_system_once(phantom_block_riot_system).unwrap();

        assert_eq!(world.resource::<PhantomBlockState>().riots, 1);
        assert!(world.get::<PhantomBlock>(block).unwrap().riot_triggered);
        assert!(matches!(
            world.get::<MentalState>(resident).unwrap(),
            MentalState::Broken(MentalBreakType::Violent)
        ));
        let unrest = world.resource::<Unrest>();
        assert!(unrest
            .modifiers
            .iter()
            .any(|m| m.label == "Phantom Block Riot" && m.value > 0.0));
        let riot_events = world
            .resource_mut::<Events<RadicalRiotEvent>>()
            .drain()
            .count();
        assert_eq!(riot_events, 1);

        // A block riots organically exactly once.
        world.run_system_once(phantom_block_riot_system).unwrap();
        assert_eq!(world.resource::<PhantomBlockState>().riots, 1);
    }

    #[test]
    fn test_demolish_triggers_riot_and_removes_block() {
        let mut world = base_world();
        let block = world
            .spawn((
                PhantomBlock {
                    center: GridPosition { x: 5, y: 5 },
                    founded_tick: 0,
                    riot_triggered: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        let resident = world
            .spawn((
                Pop,
                ResidentInPhantomBlock { block },
                IdeologyRadicalization { level: 0.0 },
                MentalState::Normal,
            ))
            .id();

        world.send_event(DemolishPhantomBlockEvent { block });
        world.run_system_once(demolish_phantom_block_system).unwrap();

        // Section 8: demolition instantly triggers the riot...
        assert_eq!(world.resource::<PhantomBlockState>().riots, 1);
        assert_eq!(world.resource::<PhantomBlockState>().demolitions, 1);
        assert!(matches!(
            world.get::<MentalState>(resident).unwrap(),
            MentalState::Broken(MentalBreakType::Violent)
        ));
        let riot_events: Vec<RadicalRiotEvent> = world
            .resource_mut::<Events<RadicalRiotEvent>>()
            .drain()
            .collect();
        assert_eq!(riot_events.len(), 1);
        assert!(riot_events[0].demolished);
        // ...and the block is gone; the former resident is a rioter,
        // no longer a resident.
        assert_eq!(phantom_block_count(&mut world), 0);
        assert!(world.get::<ResidentInPhantomBlock>(resident).is_none());
        assert!(world.get::<IdeologyRadicalization>(resident).is_some());
    }

    #[test]
    fn test_residents_consume_no_colony_food() {
        use crate::layer1::agriculture::farm::consume_food_system;

        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..ColonyResources::default()
        });
        let block = world
            .spawn(PhantomBlock {
                center: GridPosition { x: 5, y: 5 },
                founded_tick: 0,
                riot_triggered: false,
            })
            .id();
        // Both pops are hungry (hunger below the eat threshold).
        let hungry = Needs {
            hunger: 0.1,
            ..Needs::default()
        };
        world.spawn((Pop, hungry));
        world.spawn((Pop, hungry, ResidentInPhantomBlock { block }));

        world.run_system_once(consume_food_system).unwrap();

        // Exactly one meal left the ledger: the regular pop ate; the
        // block resident consumed nothing.
        let food = world.resource::<ColonyResources>().food;
        assert!(
            (food - (10.0 - FOOD_PER_MEAL)).abs() < 1e-6,
            "expected exactly one meal consumed, food={food}"
        );
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Concept-only: no named characters, places, or distinctive
        // entities from outside fiction in player-facing strings.
        const BANNED: &[&str] = &[
            "arrakis", "dune", "atreides", "harkonnen", "fremen", "sandworm",
            "tatooine", "coruscant", "jedi", "sith", "wookiee", "mordor",
            "gondor", "hobbit", "shire", "sauron", "gandalf", "rivendell",
            "westeros", "stark", "lannister", "targaryen", "winterfell",
            "klingon", "vulcan", "enterprise", "borg", "romulan",
            "xenomorph", "ripley", "weyland", "nostromo",
            "megatron", "optimus", "cybertron",
            "mega-city", "dredd", "windsor", "sad king", "billy",
        ];
        for s in phantom_block_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
    }
}
