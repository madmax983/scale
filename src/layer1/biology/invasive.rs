//! Invasive Biology (Spec 326): alien spores hitchhike on trade ships,
//! take root in living soil, spread across the map, and can be burned out
//! with quarantine burns.
//!
//! Design notes:
//! - The spec sketch assumed `Ship { origin: BiomeType::Alien }` and
//!   `TerrainType::Xenofungus`; neither exists. Adapted: the real
//!   [`TradeShipArrivalEvent`] (faction-string heuristic for "alien
//!   biome"), the existing [`TerrainType::SporeBloom`] for the
//!   infestation, and a new [`TerrainType::Ash`] only for burn aftermath.
//! - The sketch's `DesignationType::Burn` also did not exist, so burns
//!   were originally a bare [`BurnOrder`] component — unreachable from
//!   the player's seat (balance report 2026-11-02, Headline 4). Item 165
//!   wired the real designation: `DesignationType::Burn` designations are
//!   converted into `BurnOrder`s by [`process_burn_designations_system`].
//! - Burns jump the work queue (processed immediately at high priority per
//!   spec section 7) rather than routing through the slow job system.
//! - Item 166 (item 160 sub-item 1): spread never converts a tile that
//!   holds a building or is occupied by a pop. The balance report
//!   2026-11-02 showed a patch converting the Lander / LifeSupport /
//!   farms / Stockpile / walls / airlock tiles and killing a resting pop
//!   stack when its tile converted underneath it.

use bevy_ecs::prelude::*;
use bevy_utils::HashSet;
use rand::Rng;

use crate::layer1::building::{Building, BuildingMap, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer1::pop::Pop;
use crate::layer1::director::DirectorState;
use crate::layer2::trade::blockade::TradeShipArrivalEvent;

/// Alien flora strain carried by a spore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SporeStrain {
    /// Slow, relentless spreader.
    Xenofungus,
    /// Fast, aggressive spreader.
    TerranKudzu,
}

impl SporeStrain {
    /// Ticks between spread attempts for this strain.
    #[must_use]
    pub const fn spread_interval(self) -> u32 {
        match self {
            Self::Xenofungus => 40,
            Self::TerranKudzu => 25,
        }
    }
}

/// An alien spore that has taken root (or is about to) at a grid position.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spore {
    /// Where the spore sits.
    pub position: GridPosition,
    /// Which strain it carries.
    pub strain: SporeStrain,
    /// Ticks until the next spread attempt.
    pub spread_timer: u32,
    /// Ticks since this spore took root (Spec 1388 senescence).
    pub age: u32,
}

/// Player-placed quarantine burn designation. Consumed on execution.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurnOrder {
    /// Tile to scorch.
    pub position: GridPosition,
}

/// Fired when a spore bloom takes root on a tile.
#[derive(Event, Debug, Clone, Copy)]
pub struct SporeBloomEvent {
    /// Where the bloom rooted.
    pub position: GridPosition,
    /// Which strain bloomed.
    pub strain: SporeStrain,
}

/// Fired when a quarantine burn resolves.
#[derive(Event, Debug, Clone, Copy)]
pub struct QuarantineBurnEvent {
    /// Tile that was scorched.
    pub position: GridPosition,
}

/// Tuning knobs for invasive biology.
#[derive(Resource, Debug, Clone)]
pub struct InvasiveConfig {
    /// Hitchhiker chance per ordinary trade ship arrival.
    pub arrival_chance: f32,
    /// Hitchhiker chance per arrival from an exotic-looking faction.
    pub exotic_arrival_chance: f32,
    /// Hard cap on live spores (spread stops spawning past this).
    pub max_spores: usize,
    /// Spec 1388 senescence: a spore older than this (ticks) despawns,
    /// so the live count can fall below the cap and outbreaks recur.
    pub spore_lifespan: u32,
}

impl Default for InvasiveConfig {
    fn default() -> Self {
        Self {
            arrival_chance: 0.25,
            exotic_arrival_chance: 0.6,
            max_spores: 200,
            spore_lifespan: 1500,
        }
    }
}

// ---------------------------------------------------------------------------
// Item 168 telegraphing (item 160 sub-item 5): the invasive lane learns to
// speak before it kills. Balance report 2026-11-02 found the seeding link
// passed ([SPORE_OUTBREAK]) but spread/approach telegraphing FAILED: no
// tile milestones, no frontier warning, no first-damage warning (the
// damage half lives in `environment::geomes`). This is a legibility pass
// ONLY — zero spread/damage balance constants change here.
// ---------------------------------------------------------------------------

/// Chebyshev range at which the invasive frontier counts as "closing on
/// the colony". Grounded in the existing colony-visibility constant
/// `martyrdom::MARTYRDOM_VISIBILITY_RADIUS` (12 tiles around the colony
/// heart); the balance report flagged frontiers reaching within
/// Chebyshev 12 of the centre as the dangerous approaches.
pub const FRONTIER_WARNING_RANGE: u32 = 12;
/// The frontier must retreat beyond this Chebyshev range before the
/// warning re-arms (hysteresis band: FRONTIER_WARNING_RANGE..=this).
/// Without the band, a frontier flickering at the edge would spam.
pub const FRONTIER_REARM_RANGE: u32 = 16;

/// Bloom-tile milestones for the INVASIVE lane, mirroring the BloomZone
/// lane's thresholds and one-shot pattern (culture::bloomtouched
/// TILE_MILESTONES / check_tile_milestones). The count here is tiles the
/// invasive lane itself converted (infectable -> SporeBloom in
/// `spore_spread_system`); worldgen SporeBloom rects are NOT counted —
/// they were never converted by this lane, and counting them would fire
/// every milestone at tick 0 on bloom-heavy maps.
const INVASIVE_TILE_MILESTONES: [(usize, &str); 4] = [
    (25, SPORE_SPREAD_25_CHRONICLE),
    (50, SPORE_SPREAD_50_CHRONICLE),
    (100, SPORE_SPREAD_100_CHRONICLE),
    (200, SPORE_SPREAD_200_CHRONICLE),
];

/// Chronicle text: invasive bloom converted 25 tiles.
pub const SPORE_SPREAD_25_CHRONICLE: &str =
    "[SPORE_SPREAD] The invasive bloom has converted twenty-five tiles. \
     The purple growth is no longer a curiosity — it is a frontier.";
/// Chronicle text: invasive bloom converted 50 tiles.
pub const SPORE_SPREAD_50_CHRONICLE: &str =
    "[SPORE_SPREAD] Fifty tiles converted by the invasive bloom. \
     Work crews are rerouting around ground the colony used to own.";
/// Chronicle text: invasive bloom converted 100 tiles.
pub const SPORE_SPREAD_100_CHRONICLE: &str =
    "[SPORE_SPREAD] A hundred tiles converted by the invasive bloom. \
     The colony is sharing its map with something that wants all of it.";
/// Chronicle text: invasive bloom converted 200 tiles.
pub const SPORE_SPREAD_200_CHRONICLE: &str =
    "[SPORE_SPREAD] Two hundred tiles converted by the invasive bloom. \
     Quarantine burns are no longer tidiness; they are survival.";
/// Chronicle text prefix: the invasive frontier is closing on the colony.
/// The firing system appends the nearest growth's coordinates and range.
pub const SPORE_FRONTIER_WARNING: &str =
    "[SPORE_FRONTIER] The invasive bloom is closing on the colony.";

/// Formats the full frontier-warning chronicle text for a nearest growth
/// at (`x`, `y`), `distance` Chebyshev tiles from the colony.
#[must_use]
pub fn frontier_warning_text(x: i32, y: i32, distance: u32) -> String {
    format!(
        "{SPORE_FRONTIER_WARNING} Nearest growth at ({x}, {y}), {distance} tiles \
         from the nearest building or colonist. Burn it back before it arrives."
    )
}

/// Every public-facing telegraphing string in this module, for the
/// IP-guard test (original names only, concept-only).
#[must_use]
pub fn telegraph_public_strings() -> Vec<String> {
    vec![
        SPORE_SPREAD_25_CHRONICLE.to_string(),
        SPORE_SPREAD_50_CHRONICLE.to_string(),
        SPORE_SPREAD_100_CHRONICLE.to_string(),
        SPORE_SPREAD_200_CHRONICLE.to_string(),
        SPORE_FRONTIER_WARNING.to_string(),
        frontier_warning_text(1, 2, 3),
    ]
}

/// Telegraphing state for the invasive lane (item 168). One-shot flags
/// and counters only — this resource never feeds back into spread or
/// damage, so it cannot change balance.
#[derive(Resource, Debug, Clone, Default)]
pub struct InvasiveTelegraph {
    /// Tiles this lane has converted to SporeBloom since the run began
    /// (worldgen bloom excluded — see INVASIVE_TILE_MILESTONES).
    pub converted_tiles: usize,
    /// Index into INVASIVE_TILE_MILESTONES already chronicled.
    pub milestone_idx: usize,
    /// True while the frontier warning is latched (inside range or in
    /// the hysteresis band); cleared only beyond FRONTIER_REARM_RANGE.
    pub frontier_warned: bool,
}

/// The nearest frontier point to any colony target, as
/// (Chebyshev distance, frontier point). None when either set is empty.
#[must_use]
pub fn frontier_nearest(
    frontier: &[(i32, i32)],
    targets: &HashSet<(i32, i32)>,
) -> Option<(u32, (i32, i32))> {
    let mut best: Option<(u32, (i32, i32))> = None;
    for &(fx, fy) in frontier {
        let from = GridPosition { x: fx, y: fy };
        for &(tx, ty) in targets.iter() {
            let d = from.distance_chebyshev(GridPosition { x: tx, y: ty });
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (fx, fy)));
            }
        }
    }
    best
}

/// Frontier-warning state machine with hysteresis. Returns
/// `Some(distance)` exactly when a warning chronicle should fire:
/// the frontier entered FRONTIER_WARNING_RANGE while unlatched.
/// Distances inside the hysteresis band hold the latch; distances
/// beyond FRONTIER_REARM_RANGE (or no frontier at all) re-arm it.
pub fn frontier_warning_update(
    state: &mut InvasiveTelegraph,
    min_distance: Option<u32>,
) -> Option<u32> {
    match min_distance {
        Some(d) if d <= FRONTIER_WARNING_RANGE => {
            if state.frontier_warned {
                None
            } else {
                state.frontier_warned = true;
                Some(d)
            }
        }
        Some(d) if d > FRONTIER_REARM_RANGE => {
            state.frontier_warned = false;
            None
        }
        None => {
            state.frontier_warned = false;
            None
        }
        _ => None, // Hysteresis band: hold the latch, never flap.
    }
}

/// Substrings that mark a trade faction as plausibly xeno-origin.
const EXOTIC_MARKERS: &[&str] = &[
    "xeno", "void", "outer", "rim", "deep", "alien", "mycel", "spore",
];

/// Returns true when a faction name looks like it hails from alien biomes.
#[must_use]
pub fn is_exotic_faction(faction: &str) -> bool {
    let lowered = faction.to_lowercase();
    EXOTIC_MARKERS.iter().any(|m| lowered.contains(m))
}

/// Returns true for terrain a spore can take root in.
#[must_use]
pub const fn is_infectable(terrain: TerrainType) -> bool {
    matches!(
        terrain,
        TerrainType::Grass
            | TerrainType::Dirt
            | TerrainType::Shrub
            | TerrainType::Sapling
            | TerrainType::Path
    )
}

/// Neighbor offsets in deterministic scan order: east first, so replays agree.
const SPREAD_NEIGHBORS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// Trade ships can carry alien spores in their holds. Exotic-looking
/// factions are far more likely to be contaminated.
#[allow(clippy::too_many_arguments)]
pub fn spore_hitchhiker_system(
    mut commands: Commands,
    mut arrivals: EventReader<TradeShipArrivalEvent>,
    terrain: Res<TerrainGrid>,
    config: Res<InvasiveConfig>,
    spores: Query<&Spore>,
    mut bloom_events: EventWriter<SporeBloomEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    director: Option<ResMut<DirectorState>>,
) {
    // SPEC 1388 (§4.6): seeding is token-gated by the Director. No token
    // (or no DirectorState at all) -> arrivals NEVER seed. The token is
    // consumed by the first arrival that actually seeds a spore.
    let mut token_live = director.as_ref().is_some_and(|d| d.outbreak_token);
    let mut token_consumed = false;
    let mut rng = rand::thread_rng();
    for arrival in arrivals.read() {
        if !token_live {
            continue;
        }
        if spores.iter().count() >= config.max_spores {
            continue;
        }
        let chance = if is_exotic_faction(&arrival.faction) {
            config.exotic_arrival_chance
        } else {
            config.arrival_chance
        };
        if rng.gen::<f32>() >= chance {
            continue;
        }
        // 32 random probes, then a deterministic linear-scan fallback.
        let mut target: Option<(usize, usize)> = None;
        for _ in 0..32 {
            let x = rng.gen_range(0..terrain.width);
            let y = rng.gen_range(0..terrain.height);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                target = Some((x, y));
                break;
            }
        }
        if target.is_none() {
            'scan: for y in 0..terrain.height {
                for x in 0..terrain.width {
                    if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                        target = Some((x, y));
                        break 'scan;
                    }
                }
            }
        }
        if let Some((x, y)) = target {
            let strain = if rng.gen_bool(0.5) {
                SporeStrain::Xenofungus
            } else {
                SporeStrain::TerranKudzu
            };
            let position = GridPosition {
                x: x as i32,
                y: y as i32,
            };
            commands.spawn(Spore {
                position,
                strain,
                spread_timer: strain.spread_interval(),
                age: 0,
            });
            token_live = false;
            token_consumed = true;
            bloom_events.send(SporeBloomEvent { position, strain });
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "[SPORE_OUTBREAK] A trade ship of the {} seeded {:?} spores near ({x}, {y})!",
                    arrival.faction, strain
                ),
                importance: EventImportance::Major,
            });
        }
    }
    if token_consumed {
        if let Some(mut state) = director {
            state.outbreak_token = false;
        }
    }
}

/// Each tick, spores take root (converting their home tile to SporeBloom)
/// and, when their timer fires, infect the first infectable neighbor in
/// deterministic east-first scan order.
///
/// Item 166 spread guard: a tile that holds a building or is occupied by
/// a pop is never converted — neither as a spore's home tile nor as a
/// spread target. Blocked neighbors are skipped, not terminal: the scan
/// continues to the next neighbor, so the frontier still grows around
/// buildings and pops. The building seam is a union because no single
/// registry covers every building (placement maintains `OccupiedTiles`
/// and `BuildingMap`, but e.g. standalone graves are in neither — the
/// same reason `try_place_building` scans entities as well as the map):
/// a tile is blocked if `OccupiedTiles` or `BuildingMap` claims it OR a
/// live `Building` entity stands on it. Pop occupancy comes from live
/// `Pop` + `GridPosition` positions. Both sets are snapshotted once per
/// run (the flora-spread cache pattern), so the per-spore checks are O(1).
#[allow(clippy::too_many_arguments)]
pub fn spore_spread_system(
    mut commands: Commands,
    mut terrain: ResMut<TerrainGrid>,
    config: Res<InvasiveConfig>,
    mut spores: Query<(Entity, &mut Spore)>,
    occupied_tiles: Option<Res<OccupiedTiles>>,
    building_map: Option<Res<BuildingMap>>,
    buildings: Query<&GridPosition, With<Building>>,
    pops: Query<&GridPosition, With<Pop>>,
    telegraph: Option<ResMut<InvasiveTelegraph>>,
) {
    // Snapshot of tiles a spore must never convert.
    let mut building_tiles: HashSet<(i32, i32)> = HashSet::default();
    if let Some(occupied) = occupied_tiles.as_deref() {
        building_tiles.extend(occupied.0.iter().copied());
    }
    if let Some(map) = building_map.as_deref() {
        building_tiles.extend(map.0.keys().copied());
    }
    building_tiles.extend(buildings.iter().map(|p| (p.x, p.y)));
    let pop_tiles: HashSet<(i32, i32)> = pops.iter().map(|p| (p.x, p.y)).collect();
    let is_blocked = |x: i32, y: i32| {
        building_tiles.contains(&(x, y)) || pop_tiles.contains(&(x, y))
    };

    // Item 168 telegraphing bookkeeping: conversions this tick and the
    // live frontier (every spore's home tile, plus daughters spawned
    // below — commands are deferred, so track them locally).
    let mut converted_this_tick: usize = 0;
    let mut frontier_points: Vec<(i32, i32)> = Vec::new();

    let mut live = spores.iter().count();
    for (entity, mut spore) in spores.iter_mut() {
        frontier_points.push((spore.position.x, spore.position.y));
        // Take root: convert the home tile to SporeBloom — unless a
        // building stands there or a pop is standing on it (item 166).
        let (hx, hy) = (spore.position.x, spore.position.y);
        if hx >= 0 && hy >= 0 && !is_blocked(hx, hy) {
            let (x, y) = (hx as usize, hy as usize);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                terrain.set(x, y, TerrainType::SporeBloom);
                converted_this_tick += 1;
            }
        }
        // SPEC 1388 senescence (§4.6): an aged spore despawns. Terrain
        // stays SporeBloom (the scar remains); the live count falls
        // below the cap so the outbreak class can fire again.
        spore.age = spore.age.saturating_add(1);
        if config.spore_lifespan > 0 && spore.age >= config.spore_lifespan {
            commands.entity(entity).despawn();
            live = live.saturating_sub(1);
            continue;
        }
        if spore.spread_timer > 0 {
            spore.spread_timer -= 1;
            continue;
        }
        spore.spread_timer = spore.strain.spread_interval();
        if live >= config.max_spores {
            continue;
        }
        for (dx, dy) in SPREAD_NEIGHBORS {
            let (nx, ny) = (hx + dx, hy + dy);
            if nx < 0 || ny < 0 {
                continue;
            }
            // Item 166: never spread onto a building or pop-occupied
            // tile; skip it and keep scanning so the frontier grows
            // around the blocked tile instead of stalling on it.
            if is_blocked(nx, ny) {
                continue;
            }
            let (x, y) = (nx as usize, ny as usize);
            if matches!(terrain.get(x, y), Some(t) if is_infectable(t)) {
                commands.spawn(Spore {
                    position: GridPosition { x: nx, y: ny },
                    strain: spore.strain,
                    spread_timer: spore.strain.spread_interval(),
                    age: 0,
                });
                frontier_points.push((nx, ny));
                live += 1;
                break;
            }
        }
    }

    // Item 168 telegraphing: tile milestones + frontier warning. Runs
    // only when the telegraph state exists (production inits it; bare
    // test worlds without it simply skip the chronicles). Nothing here
    // feeds back into spread — legibility only.
    if let Some(mut telegraph) = telegraph {
        let prev = telegraph.converted_tiles;
        telegraph.converted_tiles = telegraph
            .converted_tiles
            .saturating_add(converted_this_tick);
        let new = telegraph.converted_tiles;
        // Mirror BloomZone's check_tile_milestones: fire each crossed
        // threshold exactly once, in order.
        let mut milestone_texts: Vec<&'static str> = Vec::new();
        for (i, (threshold, text)) in INVASIVE_TILE_MILESTONES
            .iter()
            .enumerate()
            .skip(telegraph.milestone_idx)
        {
            if prev < *threshold && new >= *threshold {
                milestone_texts.push(text);
                telegraph.milestone_idx = i + 1;
            }
        }
        // Frontier = live spore positions; colony targets = the same
        // building + pop tile snapshots the spread guard already built.
        let mut targets: HashSet<(i32, i32)> = HashSet::default();
        targets.extend(building_tiles.iter().copied());
        targets.extend(pop_tiles.iter().copied());
        let nearest = frontier_nearest(&frontier_points, &targets);
        let frontier_text = match frontier_warning_update(
            &mut telegraph,
            nearest.map(|(d, _)| d),
        ) {
            Some(d) => nearest.map(|(_, (x, y))| frontier_warning_text(x, y, d)),
            None => None,
        };
        // Deferred sends via commands (not an EventWriter param): this
        // system also runs on bare Worlds with no chronicle bus, where a
        // required writer would panic. If the bus exists the chronicles
        // land (a tick late at worst); if not, they drop quietly.
        let mut pending: Vec<(String, EventImportance)> = milestone_texts
            .into_iter()
            .map(|t| (t.to_string(), EventImportance::Minor))
            .collect();
        if let Some(text) = frontier_text {
            pending.push((text, EventImportance::Major));
        }
        for (text, importance) in pending {
            commands.queue(move |world: &mut World| {
                if let Some(mut events) =
                    world.get_resource_mut::<Events<AddChronicleEvent>>()
                {
                    events.send(AddChronicleEvent { text, importance });
                }
            });
        }
    }
}

/// Converts player `DesignationType::Burn` designations into
/// [`BurnOrder`]s (item 165). Burns jump the work queue per spec section
/// 7, so this runs as an immediate conversion — no pop is ever sent to
/// stand on a hazard tile to light the fire. The designation is consumed;
/// `quarantine_burn_system` does the scorching itself.
pub fn process_burn_designations_system(
    mut commands: Commands,
    designations: Query<(
        Entity,
        &crate::layer1::designation::Designation,
        &GridPosition,
    )>,
) {
    for (entity, designation, pos) in designations.iter() {
        if designation.designation_type == crate::layer1::designation::DesignationType::Burn {
            commands.spawn(BurnOrder { position: *pos });
            commands.entity(entity).despawn();
        }
    }
}

/// Quarantine burns resolve immediately at high priority: every spore on
/// the designated tile is destroyed and the ground is scorched to Ash.
pub fn quarantine_burn_system(
    mut commands: Commands,
    mut terrain: ResMut<TerrainGrid>,
    burns: Query<(Entity, &BurnOrder)>,
    spores: Query<(Entity, &Spore)>,
    hazards: Query<(Entity, &GridPosition), With<crate::layer1::environment::geomes::GeomeHazard>>,
    mut burn_events: EventWriter<QuarantineBurnEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for (burn_entity, order) in burns.iter() {
        // Destroy every spore on the designated tile.
        for (spore_entity, spore) in spores.iter() {
            if spore.position == order.position {
                commands.entity(spore_entity).despawn();
            }
        }
        // The tile's geome hazard dies with the burn (item 165): a
        // scorched tile must stop dealing damage this tick, not whenever
        // hazard diffusion next cleans stale hazards up.
        for (hazard_entity, hazard_pos) in hazards.iter() {
            if *hazard_pos == order.position {
                commands.entity(hazard_entity).despawn();
            }
        }
        // Scorch bloom or infectable ground to Ash.
        let (bx, by) = (order.position.x, order.position.y);
        if bx >= 0 && by >= 0 {
            let (x, y) = (bx as usize, by as usize);
            if matches!(terrain.get(x, y), Some(t) if t == TerrainType::SporeBloom || is_infectable(t))
            {
                terrain.set(x, y, TerrainType::Ash);
            }
        }
        burn_events.send(QuarantineBurnEvent {
            position: order.position,
        });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "[QUARANTINE_BURN] Quarantine burn at ({}, {}): ground scorched to ash.",
                order.position.x, order.position.y
            ),
            importance: EventImportance::Minor,
        });
        commands.entity(burn_entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::{App, Update};
    use bevy::MinimalPlugins;

    fn test_grid() -> TerrainGrid {
        TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        }
    }

    fn setup_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(test_grid());
        app.insert_resource(InvasiveConfig::default());
        app.insert_resource(InvasiveTelegraph::default());
        // Spec 1388: seeding requires a Director outbreak token. The
        // legacy seeding tests run with a token granted; the dedicated
        // no-token test below revokes it.
        app.insert_resource(DirectorState {
            outbreak_token: true,
            ..DirectorState::default()
        });
        app.init_resource::<Events<TradeShipArrivalEvent>>();
        app.init_resource::<Events<SporeBloomEvent>>();
        app.init_resource::<Events<QuarantineBurnEvent>>();
        app.init_resource::<Events<AddChronicleEvent>>();
        app.add_systems(
            Update,
            (
                spore_hitchhiker_system,
                spore_spread_system,
                quarantine_burn_system,
            ),
        );
        app
    }

    fn event_count<T: Event>(app: &App) -> usize {
        let events = app.world().resource::<Events<T>>();
        events.get_reader().read(events).count()
    }

    #[test]
    fn test_exotic_faction_heuristic() {
        for faction in [
            "Xeno Traders",
            "Void Collective",
            "Outer Rim Exchange",
            "Deep Mycel Network",
            "Alien Spore Caravan",
            "RIMWARD SPORELINGS",
        ] {
            assert!(
                is_exotic_faction(faction),
                "{faction} should read as exotic"
            );
        }
        for faction in ["Sol Standard Freight", "Terran Freight Lines", "Inner Markets Guild"] {
            assert!(
                !is_exotic_faction(faction),
                "{faction} should NOT read as exotic"
            );
        }
    }

    #[test]
    fn test_spore_arrival_on_ship() {
        let mut app = setup_test_app();
        // 50 exotic arrivals at 0.6 chance: failure odds ~1e-20.
        for _ in 0..50 {
            app.world_mut()
                .resource_mut::<Events<TradeShipArrivalEvent>>()
                .send(TradeShipArrivalEvent {
                    cargo_value: 100.0,
                    faction: "Xeno Traders".to_string(),
                });
        }
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert!(
            spore_count > 0,
            "Ship arrival should occasionally spawn spores."
        );
    }

    #[test]
    fn test_spore_arrival_respects_max_cap() {
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            max_spores: 0,
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<Events<TradeShipArrivalEvent>>()
            .send(TradeShipArrivalEvent {
                cargo_value: 100.0,
                faction: "Xeno Traders".to_string(),
            });
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "No spores may root past the cap.");
    }

    #[test]
    fn test_spore_spreads_and_replaces_terrain() {
        let mut app = setup_test_app();
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 0,
            age: 0,
        });
        app.update(); // Trigger spread: daughter spore spawns east.
        let mut daughter_at_east = false;
        for spore in app.world_mut().query::<&Spore>().iter(app.world()) {
            if spore.position == (GridPosition { x: 6, y: 5 }) {
                daughter_at_east = true;
            }
        }
        assert!(
            daughter_at_east,
            "Spread must spawn a daughter spore on the east neighbor first (deterministic scan order)."
        );
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 2, "Spread must spawn exactly one daughter spore.");
        app.update(); // Daughter takes root on its home tile.
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(6, 5),
            Some(TerrainType::SporeBloom),
            "The daughter spore must convert its home tile."
        );
    }

    #[test]
    fn test_spore_takes_root_on_home_tile() {
        let mut app = setup_test_app();
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::SporeBloom),
            "A spore must convert its home tile even before spreading."
        );
    }

    #[test]
    fn test_terrankudzu_spreads_faster() {
        assert!(
            SporeStrain::TerranKudzu.spread_interval() < SporeStrain::Xenofungus.spread_interval(),
            "TerranKudzu must be the more aggressive strain."
        );
        assert_eq!(SporeStrain::Xenofungus.spread_interval(), 40);
        assert_eq!(SporeStrain::TerranKudzu.spread_interval(), 25);
    }

    #[test]
    fn test_spore_quarantine_burn() {
        let mut app = setup_test_app();
        let tile = GridPosition { x: 5, y: 5 };
        app.world_mut().spawn(Spore {
            position: tile,
            strain: SporeStrain::Xenofungus,
            spread_timer: 999,
            age: 0,
        });
        app.world_mut().spawn(BurnOrder { position: tile });
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "Burn must destroy every spore on the tile.");
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Ash),
            "Burned ground must be scorched to Ash."
        );
        let order_count = app
            .world_mut()
            .query::<&BurnOrder>()
            .iter(app.world())
            .count();
        assert_eq!(order_count, 0, "Burn orders are consumed on execution.");
        assert!(
            event_count::<QuarantineBurnEvent>(&app) > 0,
            "Burn must fire a QuarantineBurnEvent."
        );
    }

    #[test]
    fn test_no_outbreak_without_director_token() {
        // Spec 1388 §4.6: without a token, arrivals never seed — even
        // exotic arrivals at guaranteed chance.
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            arrival_chance: 1.0,
            exotic_arrival_chance: 1.0,
            ..Default::default()
        });
        app.world_mut().resource_mut::<DirectorState>().outbreak_token = false;
        for _ in 0..10 {
            app.world_mut()
                .resource_mut::<Events<TradeShipArrivalEvent>>()
                .send(TradeShipArrivalEvent {
                    cargo_value: 100.0,
                    faction: "Xeno Traders".to_string(),
                });
        }
        app.update();
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "No token -> arrivals never seed.");
    }

    #[test]
    fn test_spore_senescence_despawns_aged_spore() {
        // Spec 1388 §4.6: at spore_lifespan the entity despawns; the
        // terrain scar (SporeBloom) remains.
        let mut app = setup_test_app();
        app.insert_resource(InvasiveConfig {
            spore_lifespan: 3,
            ..Default::default()
        });
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 999,
            age: 0,
        });
        for _ in 0..4 {
            app.update();
        }
        let spore_count = app.world_mut().query::<&Spore>().iter(app.world()).count();
        assert_eq!(spore_count, 0, "Aged spore must senesce and despawn.");
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::SporeBloom),
            "Terrain stays SporeBloom after senescence."
        );
    }

    #[test]
    fn test_burn_designation_scorches_spore_bloom() {
        // Item 165 (RED first): a player Burn designation must reach the
        // burn system — before this, nothing outside these tests ever
        // spawned a BurnOrder, so the counterplay was dead code.
        let mut app = setup_test_app();
        app.add_systems(Update, process_burn_designations_system);
        app.world_mut()
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::SporeBloom);
        assert!(crate::layer1::designation::try_designate(
            app.world_mut(),
            5,
            5,
            crate::layer1::designation::DesignationType::Burn,
        ));
        // One update converts designation -> BurnOrder; the burn system
        // (registered in setup_test_app) may need a second pass depending
        // on scheduling, so run a few.
        for _ in 0..3 {
            app.update();
        }
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Ash),
            "A Burn designation must scorch SporeBloom to Ash"
        );
        let designations = app
            .world_mut()
            .query::<&crate::layer1::designation::Designation>()
            .iter(app.world())
            .count();
        assert_eq!(designations, 0, "the designation is consumed");
    }

    #[test]
    fn test_burn_removes_geome_hazard() {
        // Item 165: burning a tile must remove its GeomeHazard the same
        // tick — hazard_tiles stayed 114 -> 114 in the balance trials
        // because nothing ever despawned one.
        let mut app = setup_test_app();
        app.world_mut()
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::SporeBloom);
        app.world_mut().spawn((
            crate::layer1::environment::geomes::GeomeHazard {
                damage_per_tick: 5.0,
            },
            GridPosition { x: 5, y: 5 },
        ));
        app.world_mut().spawn(BurnOrder {
            position: GridPosition { x: 5, y: 5 },
        });
        app.update();
        let hazards = app
            .world_mut()
            .query::<&crate::layer1::environment::geomes::GeomeHazard>()
            .iter(app.world())
            .count();
        assert_eq!(hazards, 0, "the burn must despawn the tile's hazard");
        assert_eq!(
            app.world().resource::<TerrainGrid>().get(5, 5),
            Some(TerrainType::Ash)
        );
    }

    // --- Item 166: spread guard (item 160 sub-item 1) ---
    // Balance report 2026-11-02: a spore patch converted tiles under the
    // Lander / LifeSupport / farms / Stockpile / walls / airlock and
    // killed a resting pop stack when its tile converted under it.
    // Spread must never convert a tile holding a building or a pop, but
    // the frontier must still grow around blocked tiles.

    /// Registers a building at (x, y) through every authoritative seam
    /// the placement code maintains: the Building entity itself, the
    /// OccupiedTiles set, and the BuildingMap.
    fn place_test_building(app: &mut App, x: i32, y: i32) {
        let entity = app
            .world_mut()
            .spawn((
                crate::layer1::building::Building::default(),
                GridPosition { x, y },
            ))
            .id();
        app.insert_resource({
            let mut occupied = crate::layer1::building::OccupiedTiles::default();
            occupied.0.insert((x, y));
            occupied
        });
        app.insert_resource({
            let mut map = crate::layer1::building::BuildingMap::default();
            map.0.insert((x, y), entity);
            map
        });
    }

    fn spore_positions(app: &mut App) -> Vec<GridPosition> {
        app.world_mut()
            .query::<&Spore>()
            .iter(app.world())
            .map(|s| s.position)
            .collect()
    }

    #[test]
    fn test_spore_does_not_convert_tile_under_building() {
        let mut app = setup_test_app();
        place_test_building(&mut app, 5, 5);
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Grass),
            "A spore must not convert the tile a building stands on."
        );
    }

    #[test]
    fn test_spore_spread_skips_building_tile_and_grows_around() {
        let mut app = setup_test_app();
        // Building on the east neighbor — first in spread scan order.
        place_test_building(&mut app, 6, 5);
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 0,
            age: 0,
        });
        app.update();
        let positions = spore_positions(&mut app);
        assert!(
            !positions.contains(&GridPosition { x: 6, y: 5 }),
            "Spread must not spawn a daughter spore on a building tile."
        );
        assert!(
            positions.contains(&GridPosition { x: 5, y: 6 }),
            "Frontier must grow around the blocked tile (next scan neighbor, south)."
        );
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(6, 5),
            Some(TerrainType::Grass),
            "The building tile's terrain must stay untouched."
        );
    }

    #[test]
    fn test_spore_does_not_convert_tile_occupied_by_pop() {
        let mut app = setup_test_app();
        app.world_mut().spawn((
            crate::layer1::pop::Pop,
            GridPosition { x: 5, y: 5 },
        ));
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Grass),
            "A spore must not convert the tile a pop is standing on."
        );
    }

    #[test]
    fn test_spore_spread_skips_occupied_tile_and_grows_around() {
        let mut app = setup_test_app();
        // A pop resting on the east neighbor — first in spread scan order.
        app.world_mut().spawn((
            crate::layer1::pop::Pop,
            GridPosition { x: 6, y: 5 },
        ));
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::Xenofungus,
            spread_timer: 0,
            age: 0,
        });
        app.update();
        let positions = spore_positions(&mut app);
        assert!(
            !positions.contains(&GridPosition { x: 6, y: 5 }),
            "Spread must not spawn a daughter spore on a pop-occupied tile."
        );
        assert!(
            positions.contains(&GridPosition { x: 5, y: 6 }),
            "Frontier must grow around the occupied tile (next scan neighbor, south)."
        );
    }

    #[test]
    fn test_spore_still_converts_empty_infectable_tile() {
        let mut app = setup_test_app();
        app.world_mut().spawn(Spore {
            position: GridPosition { x: 5, y: 5 },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::SporeBloom),
            "An empty infectable tile must still convert (guard must not shut spread down)."
        );
    }

    #[test]
    fn test_burn_clears_bloom_without_spore() {
        let mut app = setup_test_app();
        app.world_mut()
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::SporeBloom);
        app.world_mut().spawn(BurnOrder {
            position: GridPosition { x: 5, y: 5 },
        });
        app.update();
        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Ash),
            "Burns must scorch spore-free bloom too."
        );
    }

    // --- Item 168: invasive-lane telegraphing (item 160 sub-item 5) ---
    // Balance report 2026-11-02, Telegraphing verdict: the invasive lane
    // had NO tile-milestone chronicles (the BloomZone lane has
    // 25/50/100/200), NO frontier-near-colony warning, and the lethal
    // conversions were chronicled nowhere. These tests are the RED
    // contract for the telegraphing pass. ZERO balance constants change.

    fn setup_telegraph_app(width: usize, height: usize) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(TerrainGrid {
            width,
            height,
            tiles: vec![TerrainType::Grass; width * height],
        });
        app.insert_resource(InvasiveConfig::default());
        app.insert_resource(InvasiveTelegraph::default());
        app.insert_resource(DirectorState {
            outbreak_token: true,
            ..DirectorState::default()
        });
        app.init_resource::<Events<TradeShipArrivalEvent>>();
        app.init_resource::<Events<SporeBloomEvent>>();
        app.init_resource::<Events<QuarantineBurnEvent>>();
        app.init_resource::<Events<AddChronicleEvent>>();
        app.add_systems(
            Update,
            (
                spore_hitchhiker_system,
                spore_spread_system,
                quarantine_burn_system,
            ),
        );
        app
    }

    fn chronicle_texts(app: &App) -> Vec<String> {
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        events
            .get_reader()
            .read(events)
            .map(|e| e.text.clone())
            .collect()
    }

    fn spawn_rooted_spore(app: &mut App, x: i32, y: i32) {
        app.world_mut().spawn(Spore {
            position: GridPosition { x, y },
            strain: SporeStrain::TerranKudzu,
            spread_timer: 999,
            age: 0,
        });
    }

    fn despawn_all_spores(app: &mut App) {
        let entities: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &Spore)>()
            .iter(app.world())
            .map(|(e, _)| e)
            .collect();
        for e in entities {
            app.world_mut().despawn(e);
        }
    }

    #[test]
    fn test_invasive_milestone_fires_once_at_25_conversions() {
        let mut app = setup_telegraph_app(10, 10);
        for i in 0..25 {
            spawn_rooted_spore(&mut app, (i % 10) as i32, (i / 10) as i32);
        }
        app.update();
        let texts = chronicle_texts(&app);
        let milestones: Vec<&String> = texts
            .iter()
            .filter(|t| t.contains("[SPORE_SPREAD]"))
            .collect();
        assert_eq!(
            milestones.len(),
            1,
            "crossing 25 conversions must fire exactly one milestone chronicle, got: {texts:?}"
        );
        assert_eq!(
            app.world().resource::<InvasiveTelegraph>().converted_tiles,
            25
        );
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("[SPORE_SPREAD]")).count(),
            1,
            "the 25-tile milestone must not repeat on later ticks"
        );
    }

    #[test]
    fn test_invasive_milestone_silent_below_threshold() {
        let mut app = setup_telegraph_app(10, 10);
        for i in 0..24 {
            spawn_rooted_spore(&mut app, (i % 10) as i32, (i / 10) as i32);
        }
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("[SPORE_SPREAD]")).count(),
            0,
            "24 conversions must stay silent (first milestone is 25)"
        );
    }

    #[test]
    fn test_invasive_milestones_25_and_50_each_fire_once() {
        let mut app = setup_telegraph_app(10, 10);
        for i in 0..50 {
            spawn_rooted_spore(&mut app, (i % 10) as i32, (i / 10) as i32);
        }
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("[SPORE_SPREAD]")).count(),
            2,
            "crossing 25 and 50 in one tick fires each milestone exactly once, got: {texts:?}"
        );
        assert_eq!(
            app.world().resource::<InvasiveTelegraph>().milestone_idx,
            2
        );
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_SPREAD]"))
                .count(),
            2,
            "milestones must not repeat"
        );
    }

    #[test]
    fn test_frontier_warning_fires_near_building_and_does_not_repeat() {
        let mut app = setup_telegraph_app(40, 40);
        place_test_building(&mut app, 20, 20);
        // Chebyshev 11 from the building — inside FRONTIER_WARNING_RANGE (12).
        spawn_rooted_spore(&mut app, 20, 9);
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("[SPORE_FRONTIER]")).count(),
            1,
            "a frontier entering warning range of a building must warn once, got: {texts:?}"
        );
        assert!(app.world().resource::<InvasiveTelegraph>().frontier_warned);
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            1,
            "the frontier warning must not repeat while the frontier stays in range"
        );
    }

    #[test]
    fn test_frontier_warning_silent_when_far() {
        let mut app = setup_telegraph_app(40, 40);
        place_test_building(&mut app, 20, 20);
        // Chebyshev 20 from the building — outside even the re-arm range.
        spawn_rooted_spore(&mut app, 0, 0);
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            0,
            "a distant frontier must stay silent"
        );
        assert!(!app.world().resource::<InvasiveTelegraph>().frontier_warned);
    }

    #[test]
    fn test_frontier_warning_fires_near_pop_without_building() {
        let mut app = setup_telegraph_app(40, 40);
        app.world_mut().spawn((
            crate::layer1::pop::Pop,
            GridPosition { x: 20, y: 20 },
        ));
        spawn_rooted_spore(&mut app, 23, 20);
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            1,
            "pops are colony targets too: a near frontier must warn"
        );
    }

    #[test]
    fn test_frontier_warning_rearms_after_retreat_beyond_hysteresis() {
        let mut app = setup_telegraph_app(40, 40);
        place_test_building(&mut app, 20, 20);
        spawn_rooted_spore(&mut app, 20, 9);
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            1
        );
        // Frontier retreats beyond the re-arm range (16): warning re-arms.
        despawn_all_spores(&mut app);
        spawn_rooted_spore(&mut app, 0, 0);
        app.update();
        assert!(
            !app.world().resource::<InvasiveTelegraph>().frontier_warned,
            "retreat beyond the hysteresis band must re-arm the warning"
        );
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            1,
            "re-arming itself must not chronicle"
        );
        // A new approach fires again.
        despawn_all_spores(&mut app);
        spawn_rooted_spore(&mut app, 20, 10);
        app.update();
        assert_eq!(
            chronicle_texts(&app)
                .iter()
                .filter(|t| t.contains("[SPORE_FRONTIER]"))
                .count(),
            2,
            "a re-armed warning must fire on the next approach"
        );
    }

    #[test]
    fn test_frontier_warning_update_hysteresis_band() {
        let mut state = InvasiveTelegraph::default();
        assert_eq!(frontier_warning_update(&mut state, Some(11)), Some(11));
        assert_eq!(frontier_warning_update(&mut state, Some(11)), None);
        // Inside the hysteresis band (13..=16): state holds, no flap.
        assert_eq!(frontier_warning_update(&mut state, Some(14)), None);
        assert!(state.frontier_warned);
        // Beyond the band: re-arm.
        assert_eq!(frontier_warning_update(&mut state, Some(17)), None);
        assert!(!state.frontier_warned);
        assert_eq!(
            frontier_warning_update(&mut state, Some(FRONTIER_WARNING_RANGE)),
            Some(FRONTIER_WARNING_RANGE)
        );
        // No live frontier at all also re-arms.
        assert_eq!(frontier_warning_update(&mut state, None), None);
        assert!(!state.frontier_warned);
    }

    #[test]
    fn test_telegraph_ip_guard_no_banned_terms() {
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
        for s in telegraph_public_strings() {
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
