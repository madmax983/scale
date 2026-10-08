//! Atmospheric Resonance (Spec 1374).
//!
//! High winds funneled through canyon terrain *howl*: the noise stresses pops
//! through the existing [`NoiseMap`] → leisure → morale path, but it can be
//! harvested by [`ResonanceCrystal`]s, which feed the power grid as dynamic
//! [`PowerSource`]s (same pattern as auroral collectors).
//!
//! # Adaptation notes (spec's mock types → real codebase)
//!
//! - Mock `WindSpeed` → real [`WindGrid`] vectors + [`GlobalWind`]; the spec's
//!   15.0/20.0 wind scale assumed 0–20 winds, but the real grid peaks around
//!   ~2.25 (base 1.0 × tide 1.5 × canyon boost 1.5), so
//!   [`HowlingThreshold`] defaults to 2.0 — canyon tiles howl during strong
//!   atmospheric tides. The `howl` headless command forces a surge for demos.
//! - Mock `AcousticNoise` → real [`NoiseMap`] (0.0–1.0, ambient 0.1); howling
//!   adds on top of the propagated noise after `update_noise_system` resets it.
//! - Mock `Geometry::is_canyon` → [`CanyonMap`], computed dynamically from
//!   adjacent *terrain height differences* (see [`terrain_height`]), refreshed
//!   every [`CANYON_RECOMPUTE_INTERVAL`] ticks rather than every tick.
//! - Mock `PowerOutput` → the colony's real power grid: each crystal carries a
//!   [`PowerSource`] whose output tracks local noise, so `power_grid_system`
//!   picks it up through the existing conduit-connectivity BFS — no parallel
//!   grid code.
//! - Mock "Stress" → the existing `apply_noise_effects_system` (noise > 0.5
//!   drains leisure, lowering morale). Howling is tuned to cross that line.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    missing_docs
)]

use crate::layer1::acoustic::NoiseMap;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerSource;
use crate::layer1::map::GridPosition;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer1::wind::{GlobalWind, Vec2, WindGrid};
use bevy_ecs::prelude::*;

/// Ticks between canyon-map recomputes. The spec asks for the dynamic canyon
/// calculation to run sparingly (only when terrain is modified); a slow cadence
/// approximates that without hooking every terrain mutation site.
pub const CANYON_RECOMPUTE_INTERVAL: u64 = 50;

/// Default wind speed above which canyon tiles howl. (Spec's mock value was
/// 15.0 on a 0–20 wind scale; adapted to the real ~0–2.25 wind scale.)
pub const DEFAULT_HOWL_THRESHOLD: f32 = 2.0;

/// Noise added to the 0.0–1.0 `NoiseMap` per unit of wind above the threshold.
pub const HOWL_NOISE_GAIN: f32 = 2.0;

/// Fraction of a howling tile's noise applied to each of its 8 neighbors
/// (area-of-effect stress).
pub const HOWL_NEIGHBOR_FALLOFF: f32 = 0.5;

/// Power per tick a crystal yields at full (1.0) noise. Comparable to a
/// standard generator (`PowerSource::default().output` is 10.0).
pub const RESONANCE_POWER_GAIN: f32 = 20.0;

/// Relative height of a terrain type, used for dynamic canyon detection.
/// Canyons are low tiles flanked by higher tiles on both sides perpendicular
/// to the wind (the wind's own canyon-boost logic in `update_wind_system`
/// uses blockers the same way).
#[must_use]
pub const fn terrain_height(t: TerrainType) -> i32 {
    match t {
        TerrainType::Water | TerrainType::Crater => -1,
        TerrainType::Grass
        | TerrainType::Bridge
        | TerrainType::Void
        | TerrainType::Dirt
        | TerrainType::Path
        | TerrainType::Shrub
        | TerrainType::Sapling
        | TerrainType::MagmaRock
        | TerrainType::SporeBloom
        | TerrainType::FaultLine(_) => 0,
        TerrainType::Tree | TerrainType::Artifact | TerrainType::IndestructibleStump => 1,
        TerrainType::Rock | TerrainType::DeepRock => 2,
    }
}

/// Per-tile canyon flags, recomputed sparingly from terrain height differences.
#[derive(Resource, Debug, Clone)]
pub struct CanyonMap {
    pub width: usize,
    pub height: usize,
    pub is_canyon: Vec<bool>,
    /// Ticks since the last recompute.
    pub ticks_since_recompute: u64,
}

impl CanyonMap {
    /// Empty map; the first `recompute_canyon_map_system` run fills it in.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            is_canyon: Vec::new(),
            ticks_since_recompute: CANYON_RECOMPUTE_INTERVAL,
        }
    }

    /// Build a fully computed map from terrain (used at world setup).
    /// Assumes the default eastward wind for the initial pass; the periodic
    /// system corrects it with the live wind direction.
    #[must_use]
    pub fn from_terrain(terrain: &TerrainGrid) -> Self {
        let mut map = Self::new(terrain.width, terrain.height);
        map.recompute(terrain, Vec2::X);
        map.ticks_since_recompute = 0;
        map
    }

    /// Whether the tile is currently flagged as canyon. Out of bounds → false.
    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (ux, uy) = (x as usize, y as usize);
        if ux >= self.width || uy >= self.height {
            return false;
        }
        self.is_canyon
            .get(uy * self.width + ux)
            .copied()
            .unwrap_or(false)
    }

    /// Recompute every tile from adjacent terrain height differences.
    /// A tile is a canyon when it sits strictly lower than *both* neighbors
    /// on the axis perpendicular to the wind (calm wind: either axis counts).
    pub fn recompute(&mut self, terrain: &TerrainGrid, wind_dir: Vec2) {
        if self.width != terrain.width
            || self.height != terrain.height
            || self.is_canyon.len() != terrain.width * terrain.height
        {
            self.width = terrain.width;
            self.height = terrain.height;
            self.is_canyon = vec![false; terrain.width * terrain.height];
        }
        let dir = wind_dir.normalize_or_zero();
        let calm = dir.length() < f32::EPSILON;
        let wind_is_ew = dir.x.abs() >= dir.y.abs();

        let height_at = |x: i32, y: i32| -> Option<i32> {
            if x < 0 || y < 0 {
                return None;
            }
            terrain.get(x as usize, y as usize).map(terrain_height)
        };

        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let h = height_at(x, y).unwrap_or(0);
                let higher = |dx: i32, dy: i32| -> bool {
                    height_at(x + dx, y + dy).is_some_and(|nh| nh > h)
                };
                let pair_ns = higher(0, -1) && higher(0, 1);
                let pair_ew = higher(-1, 0) && higher(1, 0);
                let canyon = if calm {
                    pair_ns || pair_ew
                } else if wind_is_ew {
                    pair_ns
                } else {
                    pair_ew
                };
                self.is_canyon[(y as usize) * self.width + (x as usize)] = canyon;
            }
        }
    }
}

/// Tunable wind speed above which canyon tiles howl.
#[derive(Resource, Debug, Clone, Copy)]
pub struct HowlingThreshold {
    pub speed: f32,
}

impl Default for HowlingThreshold {
    fn default() -> Self {
        Self {
            speed: DEFAULT_HOWL_THRESHOLD,
        }
    }
}

/// Live howling state, refreshed every tick by `calculate_howling_system`.
#[derive(Resource, Debug, Clone, Default)]
pub struct HowlingState {
    /// Tiles currently howling.
    pub tiles: Vec<(i32, i32)>,
    /// Strongest howl this tick (noise added, 0.0–1.0).
    pub max_howl: f32,
    /// Whether any tile howled last tick (for chronicle transitions).
    pub was_howling: bool,
}

/// A resonance crystal: harvests acoustic noise into grid power.
///
/// The entity also carries a [`PowerSource`] whose output is refreshed each
/// tick from the tile's noise by `process_resonance_power_system`, so the
/// existing `power_grid_system` picks it up through conduit connectivity.
#[derive(Component, Debug, Clone)]
pub struct ResonanceCrystal {
    /// Multiplier on harvested power (1.0 = standard).
    pub efficiency: f32,
}

impl Default for ResonanceCrystal {
    fn default() -> Self {
        Self { efficiency: 1.0 }
    }
}

/// Spawn a resonance crystal (plus its grid-connected power source) at a tile.
pub fn spawn_resonance_crystal(world: &mut World, x: i32, y: i32) -> Entity {
    world
        .spawn((
            ResonanceCrystal::default(),
            PowerSource {
                output: 0.0,
                active: false,
            },
            GridPosition { x, y },
        ))
        .id()
}

/// Recompute the canyon map every [`CANYON_RECOMPUTE_INTERVAL`] ticks
/// (spec: sparingly, not every tick).
pub fn recompute_canyon_map_system(
    terrain: Option<Res<TerrainGrid>>,
    wind: Option<Res<GlobalWind>>,
    canyon_map: Option<ResMut<CanyonMap>>,
) {
    let (Some(terrain), Some(wind), Some(mut canyon_map)) = (terrain, wind, canyon_map) else {
        return;
    };
    canyon_map.ticks_since_recompute += 1;
    if canyon_map.ticks_since_recompute < CANYON_RECOMPUTE_INTERVAL
        && !canyon_map.is_canyon.is_empty()
    {
        return;
    }
    canyon_map.recompute(&terrain, wind.direction);
    canyon_map.ticks_since_recompute = 0;
}

/// Wind howling: canyon tiles with wind above the threshold add noise to the
/// [`NoiseMap`] (plus half-strength to the 8 neighbors for area-of-effect
/// stress). Runs after `update_noise_system` (which resets the map) and before
/// `apply_noise_effects_system` (so the leisure/morale drain sees the howl).
///
/// Note: no vacuum check — wind presence *is* the air here. The `PressureGrid`
/// models habitat pressurization (near-zero outdoors), not the planetary
/// atmosphere the wind moves through.
pub fn calculate_howling_system(
    wind_grid: Option<Res<WindGrid>>,
    canyon_map: Option<Res<CanyonMap>>,
    threshold: Option<Res<HowlingThreshold>>,
    noise_map: Option<ResMut<NoiseMap>>,
    howling: Option<ResMut<HowlingState>>,
) {
    let (Some(wind_grid), Some(canyon_map), Some(mut noise_map)) =
        (wind_grid, canyon_map, noise_map)
    else {
        return;
    };
    let threshold = threshold.map(|t| t.speed).unwrap_or(DEFAULT_HOWL_THRESHOLD);

    let mut tiles = Vec::new();
    let mut max_howl: f32 = 0.0;

    for y in 0..canyon_map.height as i32 {
        for x in 0..canyon_map.width as i32 {
            if !canyon_map.get(x, y) {
                continue;
            }
            let speed = wind_grid.get_wind(x, y).length();
            if speed <= threshold {
                continue;
            }
            let howl = ((speed - threshold) * HOWL_NOISE_GAIN).min(1.0);
            let center = noise_map.get(x, y);
            noise_map.set(x, y, center + howl);
            // Area-of-effect: half-strength spill into the 8 neighbors.
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    let current = noise_map.get(nx, ny);
                    noise_map.set(nx, ny, current + howl * HOWL_NEIGHBOR_FALLOFF);
                }
            }
            tiles.push((x, y));
            max_howl = max_howl.max(howl);
        }
    }

    if let Some(mut howling) = howling {
        howling.tiles = tiles;
        howling.max_howl = max_howl;
    }
}

/// Chronicle flavor on howling transitions.
pub fn howling_chronicle_system(
    mut howling: ResMut<HowlingState>,
    mut events: EventWriter<AddChronicleEvent>,
) {
    let now_howling = !howling.tiles.is_empty();
    if now_howling && !howling.was_howling {
        events.send(AddChronicleEvent {
            text: "The canyon has found its voice — a low howl rolls across the colony."
                .to_string(),
            importance: EventImportance::Major,
        });
    } else if !now_howling && howling.was_howling {
        events.send(AddChronicleEvent {
            text: "The canyon falls silent.".to_string(),
            importance: EventImportance::Minor,
        });
    }
    howling.was_howling = now_howling;
}

/// Crystals harvest tile noise into the power grid: each crystal's
/// [`PowerSource`] output tracks local noise. Runs in the Economy set just
/// before `power_grid_system` (same pattern as auroral collectors).
pub fn process_resonance_power_system(
    noise_map: Option<Res<NoiseMap>>,
    mut crystals: Query<(&ResonanceCrystal, &GridPosition, &mut PowerSource)>,
) {
    let Some(noise_map) = noise_map else {
        return;
    };
    for (crystal, pos, mut source) in &mut crystals {
        let level = noise_map.get(pos.x, pos.y);
        let output = level * RESONANCE_POWER_GAIN * crystal.efficiency;
        if (source.output - output).abs() > f32::EPSILON {
            source.output = output;
        }
        source.active = level > 0.01;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::needs::Needs;
    use crate::layer1::physics::acoustic::apply_noise_effects_system;
    use crate::layer1::pop::Pop;
    use bevy_ecs::system::RunSystemOnce;

    fn test_terrain(width: usize, height: usize) -> TerrainGrid {
        TerrainGrid {
            width,
            height,
            tiles: vec![TerrainType::Grass; width * height],
        }
    }

    fn set_tile(terrain: &mut TerrainGrid, x: usize, y: usize, t: TerrainType) {
        terrain.tiles[y * terrain.width + x] = t;
    }

    fn world_with_grids() -> World {
        let mut world = World::new();
        world.insert_resource(test_terrain(5, 5));
        world.insert_resource(WindGrid::new(5, 5));
        world.insert_resource(GlobalWind::default());
        world.insert_resource(HowlingThreshold::default());
        let mut noise = NoiseMap::new(5, 5);
        noise.values.fill(0.0);
        world.insert_resource(noise);
        world.insert_resource(CanyonMap::new(5, 5));
        world.insert_resource(HowlingState::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    #[test]
    fn terrain_height_mapping() {
        assert_eq!(terrain_height(TerrainType::Rock), 2);
        assert_eq!(terrain_height(TerrainType::DeepRock), 2);
        assert_eq!(terrain_height(TerrainType::Tree), 1);
        assert_eq!(terrain_height(TerrainType::Grass), 0);
        assert_eq!(terrain_height(TerrainType::Water), -1);
        assert_eq!(terrain_height(TerrainType::Crater), -1);
    }

    #[test]
    fn canyon_detected_between_high_walls() {
        let mut terrain = test_terrain(5, 5);
        // Rock walls north and south of (2,2); east wind → canyon.
        set_tile(&mut terrain, 2, 1, TerrainType::Rock);
        set_tile(&mut terrain, 2, 3, TerrainType::Rock);
        let mut map = CanyonMap::new(5, 5);
        map.recompute(&terrain, Vec2::X);
        assert!(map.get(2, 2), "tile between rock walls should be a canyon");
        assert!(!map.get(0, 0), "flat corner should not be a canyon");
        assert!(!map.get(2, 1), "the wall itself should not be a canyon");
    }

    #[test]
    fn canyon_needs_both_walls() {
        let mut terrain = test_terrain(5, 5);
        set_tile(&mut terrain, 2, 1, TerrainType::Rock);
        let mut map = CanyonMap::new(5, 5);
        map.recompute(&terrain, Vec2::X);
        assert!(
            !map.get(2, 2),
            "one-sided height difference is not a canyon"
        );
    }

    #[test]
    fn flat_terrain_has_no_canyons() {
        let terrain = test_terrain(5, 5);
        let mut map = CanyonMap::new(5, 5);
        map.recompute(&terrain, Vec2::X);
        for y in 0..5 {
            for x in 0..5 {
                assert!(!map.get(x, y), "flat terrain should have no canyons");
            }
        }
    }

    #[test]
    fn canyon_axis_follows_wind_direction() {
        let mut terrain = test_terrain(5, 5);
        // Walls east and west of (2,2): canyon only for north-south wind.
        set_tile(&mut terrain, 1, 2, TerrainType::Rock);
        set_tile(&mut terrain, 3, 2, TerrainType::Rock);
        let mut map = CanyonMap::new(5, 5);
        map.recompute(&terrain, Vec2::X); // east wind → checks N/S pair → no canyon
        assert!(!map.get(2, 2));
        map.recompute(&terrain, Vec2::new(0.0, 1.0)); // north wind → checks E/W pair
        assert!(map.get(2, 2), "E/W walls are a canyon for N/S wind");
    }

    #[test]
    fn recompute_system_refreshes_stale_map() {
        let mut world = world_with_grids();
        // Corrupt the map, then let the system rebuild it.
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon.clear();
            map.ticks_since_recompute = 0;
        }
        world
            .run_system_once(recompute_canyon_map_system)
            .unwrap();
        let map = world.resource::<CanyonMap>();
        assert_eq!(map.is_canyon.len(), 25, "system should fill the map");
    }

    /// Spec 1374 RED (a), adapted: canyon tile + high wind (20.0) + zero noise
    /// → after update, noise > 0.0.
    #[test]
    fn howling_raises_noise_on_canyon_tile() {
        let mut world = world_with_grids();
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon = vec![false; 25];
            map.is_canyon[2 * 5 + 2] = true;
            map.ticks_since_recompute = 0;
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(20.0, 0.0));
        }
        world.run_system_once(calculate_howling_system).unwrap();
        let noise = world.resource::<NoiseMap>();
        assert!(
            noise.get(2, 2) > 0.0,
            "canyon tile with 20.0 wind should howl (noise > 0)"
        );
    }

    #[test]
    fn no_howl_below_threshold() {
        let mut world = world_with_grids();
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon = vec![false; 25];
            map.is_canyon[2 * 5 + 2] = true;
            map.ticks_since_recompute = 0;
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(1.0, 0.0));
        }
        world.run_system_once(calculate_howling_system).unwrap();
        let noise = world.resource::<NoiseMap>();
        assert_eq!(noise.get(2, 2), 0.0, "gentle wind should not howl");
    }

    #[test]
    fn no_howl_without_canyon() {
        let mut world = world_with_grids();
        {
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(20.0, 0.0));
        }
        world.run_system_once(calculate_howling_system).unwrap();
        let noise = world.resource::<NoiseMap>();
        assert_eq!(
            noise.get(2, 2),
            0.0,
            "high wind on flat terrain should not howl"
        );
    }

    #[test]
    fn howl_spreads_to_neighbors() {
        let mut world = world_with_grids();
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon = vec![false; 25];
            map.is_canyon[2 * 5 + 2] = true;
            map.ticks_since_recompute = 0;
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(20.0, 0.0));
        }
        world.run_system_once(calculate_howling_system).unwrap();
        let noise = world.resource::<NoiseMap>();
        assert!(
            noise.get(3, 2) > 0.0,
            "howl should spill into adjacent tiles (area-of-effect stress)"
        );
        assert!(
            noise.get(2, 2) >= noise.get(3, 2),
            "epicenter should be at least as loud as neighbors"
        );
    }

    #[test]
    fn howling_state_tracks_tiles() {
        let mut world = world_with_grids();
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon = vec![false; 25];
            map.is_canyon[2 * 5 + 2] = true;
            map.ticks_since_recompute = 0;
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(20.0, 0.0));
        }
        world.run_system_once(calculate_howling_system).unwrap();
        let state = world.resource::<HowlingState>();
        assert!(
            state.tiles.contains(&(2, 2)),
            "HowlingState should list the howling tile"
        );
        assert!(state.max_howl > 0.0);
    }

    /// Spec 1374 RED (b), adapted: ResonanceCrystal on a tile with high noise
    /// (0.9 on the real 0.0–1.0 scale) → after update, power output > 0.0.
    #[test]
    fn crystal_harvests_noise_into_power() {
        let mut world = world_with_grids();
        {
            let mut noise = world.resource_mut::<NoiseMap>();
            noise.set(2, 2, 0.9);
        }
        let crystal = world
            .spawn((
                ResonanceCrystal::default(),
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();
        world.run_system_once(process_resonance_power_system).unwrap();
        let source = world.get::<PowerSource>(crystal).unwrap();
        assert!(
            source.output > 0.0,
            "crystal on noisy tile should generate power"
        );
        assert!(source.active, "crystal should be active while harvesting");
    }

    #[test]
    fn crystal_idle_when_quiet() {
        let mut world = world_with_grids();
        let crystal = world
            .spawn((
                ResonanceCrystal::default(),
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();
        world.run_system_once(process_resonance_power_system).unwrap();
        let source = world.get::<PowerSource>(crystal).unwrap();
        assert_eq!(source.output, 0.0);
        assert!(!source.active);
    }

    /// The existing noise → leisure → morale machinery is the spec's "Stress":
    /// howling must push noise past the 0.5 leisure-drain line.
    #[test]
    fn howl_stresses_pops_through_leisure() {
        let mut world = world_with_grids();
        {
            let mut map = world.resource_mut::<CanyonMap>();
            map.is_canyon = vec![false; 25];
            map.is_canyon[2 * 5 + 2] = true;
            map.ticks_since_recompute = 0;
            let mut wind = world.resource_mut::<WindGrid>();
            wind.set_wind(2, 2, Vec2::new(20.0, 0.0));
        }
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 2, y: 2 },
                Needs {
                    leisure: 0.8,
                    ..Default::default()
                },
            ))
            .id();
        world.run_system_once(calculate_howling_system).unwrap();
        let mut schedule = Schedule::default();
        schedule.add_systems(apply_noise_effects_system);
        schedule.run(&mut world);
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            needs.leisure < 0.8,
            "howling should stress pops via the leisure drain (got {})",
            needs.leisure
        );
    }

    #[test]
    fn howl_begin_announces_chronicle() {
        let mut world = world_with_grids();
        {
            let mut state = world.resource_mut::<HowlingState>();
            state.tiles = vec![(2, 2)];
            state.was_howling = false;
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(howling_chronicle_system);
        schedule.run(&mut world);
        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let texts: Vec<String> = reader.read(events).map(|e| e.text.clone()).collect();
        assert_eq!(texts.len(), 1, "howl onset should chronicle once");
        assert!(texts[0].to_lowercase().contains("howl"));
    }
}
