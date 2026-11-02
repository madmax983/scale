//! Geological instability system (Spec 153, extended by Spec 389).
//!
//! Tracks seismic stress and triggers earthquakes.
//!
//! ## Spec 389 audit (what already existed vs. what this pass added)
//!
//! Spec 153 already built the skeleton this module extends:
//! [`SeismicGrid`] (per-tile stress), [`GeologicalEvent`], the
//! stress-threshold trigger ([`check_seismic_events`]), and
//! [`apply_geological_event_system`]. Spec 171 added the vibration
//! pipeline (`environment::seismic::VibrationGrid`), tectonic stress
//! ([`tectonic::TectonicStress`], fed by `MiningEvent`s), and Spec 375
//! core-tapping fires `GeologicalEvent`s from stability loss.
//!
//! The old `apply_geological_event_system` was a stub: it damaged only
//! entities standing on the *exact* epicentre tile, only via the pop
//! [`Health`] component — real buildings carry
//! [`Structure`][crate::layer1::structure::Structure] health, so quakes
//! never touched a building — with no radius, no falloff, no terrain
//! effect, no destruction, no chronicle, and no sinkholes.
//!
//! Spec 389 therefore does NOT create the sketch's parallel
//! `SeismicEvent` system. It builds the missing legs onto
//! `GeologicalEvent`:
//!
//! * **Distance-attenuated damage** (§5 REFACTOR): Euclidean falloff
//!   over [`quake_radius`], computed by the pure deterministic helper
//!   [`seismic_damage_at`]. Both health pools are hit: `Health` (pops
//!   and the sketch's Building+Health shape) and `Structure`
//!   (real buildings, with the fire system's Fragile multiplier).
//! * **Terrain modifier** (§5): bedrock (Rock/DeepRock) halves damage,
//!   soft soil (Dirt/Ash) amplifies it by 1.5x, already-fractured
//!   ground (Crater/FaultLine) by 1.25x — see
//!   [`terrain_damage_multiplier`]. "Build on bedrock or soft soil?"
//!   is now a real choice.
//! * **Destruction at zero health** (§5): a building whose `Structure`
//!   reaches 0 collapses through the real ruin pipeline — despawn,
//!   [`Ruin`] + [`RuinHistory`] ("Earthquake") + compost scrap heap,
//!   `BuildingRemovedEvent`, and the central `Added<Ruin>` chronicle.
//! * **Sinkholes** (§1): a quake of magnitude
//!   [`SINKHOLE_MIN_MAGNITUDE`] or more tears the epicentre tile into
//!   a `TerrainType::Crater` (the game's unwalkable collapse terrain),
//!   if the tile is ordinary ground.
//! * **Mining feeds local stress** (§1): completed mining
//!   (`resources::mine_rock`) now adds stress to the `SeismicGrid` at
//!   the mined tile — `MiningEvent` itself carries no position, so the
//!   local feed lives at the one site that knows the tile (the global
//!   `TectonicStress` feed via `MiningEvent` already existed).
//! * **Random cadence** (§8): [`GeologicalDirector`] fires a purely
//!   random quake every ~2,500–5,000 ticks ("purely random for the MVP
//!   based on the planetary quirk"). The cadence is derived from a
//!   deterministic hash of a seed rather than `thread_rng`, so runs
//!   are reproducible and tests never flake; the *schedule* is still
//!   irregular and position/magnitude vary per quake.
//! * **Chronicle + ledger**: quakes of magnitude >= 3.0 (or any quake
//!   that destroys a building or opens a sinkhole) write a chronicle
//!   entry; [`GeologicalStats`] counts events, sinkholes, and
//!   destroyed buildings for headless STATS (`quakes=` / `sinkholes=`
//!   / `instability=`).
//!
//! Adaptations from the spec sketch: `SeismicEvent` -> the existing
//! `GeologicalEvent { center, magnitude }` (radius is derived from
//! magnitude, not a field — every existing producer keeps compiling);
//! `layer1::buildings::Building` / `layer1::health::Health` sketch
//! paths -> the real `layer1::building::Building` +
//! `layer1::structure::Structure` + `layer1::health::Health`;
//! the sketch's `DestructionEvent` -> the codebase's ruin pipeline
//! (there is no generic DestructionEvent; destruction everywhere else
//! — fire, sabotage, deorbit — ends in a `Ruin`).
//!
//! All names and strings are original and generic (see the IP-guard
//! test at the bottom of this module).

use crate::layer1::building::{Building, Material, MaterialType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::events::BuildingRemovedEvent;
use crate::layer1::map::ScreenShake;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer1::ruins::{Ruin, RuinHistory};
use crate::layer1::structure::{Fragile, Structure, FRAGILITY_DAMAGE_MULTIPLIER};
use crate::layer1::GridPosition;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use ratatui::style::Color;

/// Grid tracking seismic stress accumulation.
#[derive(Resource, Default)]
pub struct SeismicGrid {
    /// Width of the grid.
    pub width: usize,
    /// Height of the grid.
    pub height: usize,
    /// Stress values (row-major).
    pub stress: Vec<f32>,
}

/// Stress level that triggers an earthquake.
pub const STRESS_THRESHOLD: f32 = 100.0;
/// Amount of stress that decays per tick.
pub const DECAY_RATE: f32 = 0.5;
/// Damage dealt at the epicentre per point of magnitude, before
/// distance attenuation and the terrain modifier (preserves the
/// pre-389 scale: the stub dealt `10.0 * magnitude` on its one tile).
pub const QUAKE_DAMAGE_PER_MAGNITUDE: f32 = 10.0;
/// Magnitude at or above which a quake tears a sinkhole into the
/// epicentre tile.
pub const SINKHOLE_MIN_MAGNITUDE: f32 = 8.0;
/// Shortest possible gap between random director quakes, in ticks.
pub const RANDOM_QUAKE_MIN_INTERVAL: u32 = 2_500;
/// Random director intervals span
/// `MIN_INTERVAL .. MIN_INTERVAL + INTERVAL_SPREAD` ticks.
pub const RANDOM_QUAKE_INTERVAL_SPREAD: u32 = 2_500;
/// Ticks until the first random director quake on a fresh world —
/// deliberately beyond the 1,000-tick survival-trial horizon, so the
/// random cadence never ambushes early colonies or balance trials.
pub const RANDOM_QUAKE_FIRST_INTERVAL: u32 = 3_000;

impl SeismicGrid {
    /// Creates a new seismic grid with the given dimensions.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 10_000_000, "Grid size overflow or too large");
        Self {
            width,
            height,
            stress: vec![0.0; size],
        }
    }

    /// Gets the stress level at the given coordinates.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn get_stress(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0.0;
        }
        if let Some(idx) = (y as usize)
            .checked_mul(self.width)
            .and_then(|i| i.checked_add(x as usize))
        {
            if idx < self.stress.len() {
                return self.stress[idx];
            }
        }
        0.0
    }

    /// Adds stress to the given coordinates.
    #[allow(clippy::cast_sign_loss)]
    pub fn add_stress(&mut self, x: i32, y: i32, amount: f32) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            if let Some(idx) = (y as usize)
                .checked_mul(self.width)
                .and_then(|i| i.checked_add(x as usize))
            {
                if idx < self.stress.len() {
                    self.stress[idx] += amount;
                }
            }
        }
    }

    /// Decays stress across the entire grid.
    pub fn decay(&mut self, rate: f32) {
        for s in &mut self.stress {
            *s = (*s - rate).max(0.0);
        }
    }

    /// Total stress currently stored across the whole grid (the
    /// colony's headline instability figure for headless STATS).
    #[must_use]
    pub fn total_stress(&self) -> f32 {
        self.stress.iter().sum()
    }
}

/// Events related to geological instability.
#[derive(Event, Debug, Clone)]
pub struct GeologicalEvent {
    /// Center of the geological event.
    pub center: GridPosition,
    /// Magnitude of the event (determines damage radius and intensity).
    pub magnitude: f32,
}

/// Ledger of geological activity (Spec 389) — headless STATS reads
/// this; it is also the natural hook for future quake UI.
#[derive(Resource, Debug, Default, Clone)]
pub struct GeologicalStats {
    /// Total geological events applied.
    pub total_events: u64,
    /// Sinkholes torn open by high-magnitude quakes.
    pub sinkholes: u64,
    /// Buildings collapsed by quakes (via the ruin pipeline).
    pub buildings_destroyed: u64,
    /// Magnitude of the most recent event (0.0 before the first).
    pub last_magnitude: f32,
}

/// `(total quakes, sinkholes)` for headless STATS; `(0, 0)` when the
/// ledger has not been initialized in this world.
#[must_use]
pub fn geological_stats_counts(world: &World) -> (u64, u64) {
    world
        .get_resource::<GeologicalStats>()
        .map_or((0, 0), |s| (s.total_events, s.sinkholes))
}

/// Total seismic stress across the grid (STATS `instability=`); 0.0
/// when no grid exists in this world.
#[must_use]
pub fn total_seismic_stress(world: &World) -> f32 {
    world
        .get_resource::<SeismicGrid>()
        .map_or(0.0, SeismicGrid::total_stress)
}

/// Damage radius of a quake, in tiles: one tile per point of
/// magnitude (a magnitude-5 quake — the stress-trigger default —
/// threatens a modest neighbourhood; a magnitude-15 mega-quake is a
/// district-scale disaster).
#[must_use]
pub fn quake_radius(magnitude: f32) -> f32 {
    magnitude.max(1.0)
}

/// How much of a quake's force a terrain type transmits to whatever
/// stands on it (Spec 389 §5: bedrock vs soft soil).
///
/// Bedrock (Rock / DeepRock) is the safe foundation at 0.5x; soft soil
/// (Dirt / Ash) amplifies shaking to 1.5x; ground that is already
/// fractured (Crater / FaultLine) transmits 1.25x; water cushions to
/// 0.5x; everything else is neutral.
#[must_use]
pub fn terrain_damage_multiplier(terrain: Option<TerrainType>) -> f32 {
    match terrain {
        Some(TerrainType::Rock | TerrainType::DeepRock) => 0.5,
        Some(TerrainType::Dirt | TerrainType::Ash) => 1.5,
        Some(TerrainType::Crater | TerrainType::FaultLine(_)) => 1.25,
        Some(TerrainType::Water) => 0.5,
        _ => 1.0,
    }
}

/// Pure deterministic quake damage at `distance` tiles (Euclidean)
/// from the epicentre, on `terrain`. Zero beyond [`quake_radius`];
/// falls off linearly toward the rim so the epicentre always takes
/// the full blow. No randomness — safe for exact unit tests.
#[must_use]
pub fn seismic_damage_at(magnitude: f32, distance: f32, terrain: Option<TerrainType>) -> f32 {
    if magnitude <= 0.0 || distance < 0.0 {
        return 0.0;
    }
    let radius = quake_radius(magnitude);
    if distance > radius {
        return 0.0;
    }
    let attenuation = 1.0 - distance / (radius + 1.0);
    QUAKE_DAMAGE_PER_MAGNITUDE * magnitude * attenuation * terrain_damage_multiplier(terrain)
}

fn euclidean_distance(a: GridPosition, b: GridPosition) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let dx = (a.x - b.x) as f32;
    #[allow(clippy::cast_precision_loss)]
    let dy = (a.y - b.y) as f32;
    (dx * dx + dy * dy).sqrt()
}

fn terrain_at(terrain: &Option<ResMut<TerrainGrid>>, pos: GridPosition) -> Option<TerrainType> {
    let grid = terrain.as_ref()?;
    if pos.x < 0 || pos.y < 0 {
        return None;
    }
    #[allow(clippy::cast_sign_loss)]
    grid.get(pos.x as usize, pos.y as usize)
}

/// SplitMix-style hash: the random director's deterministic entropy
/// source. Identical seeds give identical quake schedules, which is
/// what keeps the "random" cadence testable and replays stable.
#[must_use]
pub fn director_hash(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Gap until the next random quake for a given director seed:
/// irregular, always within
/// `RANDOM_QUAKE_MIN_INTERVAL .. MIN + SPREAD`.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn next_random_interval(seed: u64) -> u32 {
    RANDOM_QUAKE_MIN_INTERVAL
        + (director_hash(seed) % u64::from(RANDOM_QUAKE_INTERVAL_SPREAD)) as u32
}

/// Magnitude of a random director quake for a given seed:
/// 3.00..=5.99 — strong enough to damage and occasionally collapse a
/// weakened building, never enough to open a sinkhole on its own.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn random_quake_magnitude(seed: u64) -> f32 {
    3.0 + ((director_hash(seed ^ 0xA5A5_A5A5_A5A5_A5A5) % 300) as f32) / 100.0
}

/// Epicentre of a random director quake for a given seed, within a
/// `width` x `height` map (falls back to the map heart when the map
/// size is unknown/degenerate).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub fn random_quake_center(seed: u64, width: usize, height: usize) -> GridPosition {
    if width == 0 || height == 0 {
        return GridPosition { x: 40, y: 25 };
    }
    let hx = director_hash(seed ^ 0x1111_2222_3333_4444);
    let hy = director_hash(seed ^ 0x5555_6666_7777_8888);
    GridPosition {
        x: (hx % width as u64) as i32,
        y: (hy % height as u64) as i32,
    }
}

/// The purely-random quake director (Spec 389 §8: "purely random for
/// the MVP based on the planetary quirk"). Counts down
/// [`GeologicalDirector::ticks_until_next`]; at zero it fires one
/// [`GeologicalEvent`] at a hash-chosen tile with a hash-chosen
/// magnitude, then re-arms with the next hash-chosen interval.
#[derive(Resource, Debug, Clone)]
pub struct GeologicalDirector {
    /// Ticks remaining until the next random quake fires.
    pub ticks_until_next: u32,
    /// Entropy seed; advanced after every quake fired.
    pub seed: u64,
}

impl Default for GeologicalDirector {
    fn default() -> Self {
        Self {
            ticks_until_next: RANDOM_QUAKE_FIRST_INTERVAL,
            seed: 0x3890_1531_0711_0001,
        }
    }
}

/// System: the random director's countdown + firing (see
/// [`GeologicalDirector`]).
pub fn random_geological_event_system(
    mut director: ResMut<GeologicalDirector>,
    grid: Option<ResMut<SeismicGrid>>,
    mut events: EventWriter<GeologicalEvent>,
) {
    if director.ticks_until_next > 0 {
        director.ticks_until_next -= 1;
        return;
    }
    let (width, height) = grid.as_ref().map_or((80, 50), |g| (g.width, g.height));
    let seed = director.seed;
    events.send(GeologicalEvent {
        center: random_quake_center(seed, width, height),
        magnitude: random_quake_magnitude(seed),
    });
    director.seed = director_hash(seed);
    director.ticks_until_next = next_random_interval(director.seed);
}

/// Helper to add stress to the grid from other systems.
pub fn add_seismic_stress(world: &mut World, pos: GridPosition, amount: f32) {
    if let Some(mut grid) = world.get_resource_mut::<SeismicGrid>() {
        grid.add_stress(pos.x, pos.y, amount);
    }
}

/// System that decays seismic stress over time.
pub fn seismic_decay_system(mut grid: ResMut<SeismicGrid>) {
    grid.decay(DECAY_RATE);
}

/// System that checks for stress thresholds and triggers events.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub fn check_seismic_events(
    mut grid: ResMut<SeismicGrid>,
    mut events: EventWriter<GeologicalEvent>,
) {
    for y in 0..grid.height {
        for x in 0..grid.width {
            let idx = y
                .checked_mul(grid.width)
                .and_then(|i| i.checked_add(x))
                .unwrap_or(usize::MAX);
            if idx < grid.stress.len() && grid.stress[idx] > STRESS_THRESHOLD {
                // Trigger event
                // Reset stress (release energy)
                grid.stress[idx] = 0.0;
                events.send(GeologicalEvent {
                    center: GridPosition {
                        x: x as i32,
                        y: y as i32,
                    },
                    magnitude: 5.0, // Simplified magnitude
                });
            }
        }
    }
}

pub mod subsurface;
/// System that applies effects of geological events (damage, visuals).
pub mod tectonic;

/// System that applies a [`GeologicalEvent`]: distance-attenuated,
/// terrain-modified damage to every `Health` and `Structure` in
/// radius; buildings reduced to zero collapse into ruins through the
/// real ruin pipeline; high-magnitude quakes tear a sinkhole into the
/// epicentre; notable quakes are chronicled and ledgered.
///
/// Requires `Events<AddChronicleEvent>` and
/// `Events<BuildingRemovedEvent>` to be initialized (both are, in
/// `simulation.rs` / `setup.rs` — the first-tick pattern).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn apply_geological_event_system(
    mut events: EventReader<GeologicalEvent>,
    mut commands: Commands,
    mut health_query: Query<(&GridPosition, &mut crate::layer1::health::Health), Without<Structure>>,
    mut structure_query: Query<(
        Entity,
        &GridPosition,
        &mut Structure,
        Option<&Building>,
        Option<&Material>,
        Option<&Fragile>,
    )>,
    mut terrain: Option<ResMut<TerrainGrid>>,
    mut shake: Option<ResMut<ScreenShake>>,
    mut stats: Option<ResMut<GeologicalStats>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut removed: EventWriter<BuildingRemovedEvent>,
    time: Option<ResMut<SimulationTime>>,
) {
    for event in events.read() {
        let center = event.center;
        let magnitude = event.magnitude;

        if let Some(s) = stats.as_mut() {
            s.total_events += 1;
            s.last_magnitude = magnitude;
        }

        // Pops and anything else carrying Health (but not buildings —
        // a Structure entity is handled by the structure pass, never
        // double-hit).
        for (pos, mut health) in &mut health_query {
            let damage = seismic_damage_at(
                magnitude,
                euclidean_distance(*pos, center),
                terrain_at(&terrain, *pos),
            );
            if damage > 0.0 {
                health.current -= damage;
            }
        }

        // Real buildings: Structure health, Fragile amplification
        // (same multiplier the fire pipeline uses).
        let mut destroyed: Vec<(Entity, GridPosition, crate::layer1::building::BuildingType, MaterialType)> =
            Vec::new();
        for (entity, pos, mut structure, building, material, fragile) in &mut structure_query {
            let mut damage = seismic_damage_at(
                magnitude,
                euclidean_distance(*pos, center),
                terrain_at(&terrain, *pos),
            );
            if damage <= 0.0 {
                continue;
            }
            if let Some(f) = fragile {
                #[allow(clippy::cast_precision_loss)]
                let multiplier = (f.stacks as f32).mul_add(FRAGILITY_DAMAGE_MULTIPLIER, 1.0);
                damage *= multiplier;
            }
            structure.current_hp -= damage;
            if structure.current_hp <= 0.0 {
                destroyed.push((
                    entity,
                    *pos,
                    building.map_or_else(Default::default, |b| b.building_type),
                    material.map_or_else(Default::default, |m| m.0),
                ));
            }
        }

        // Sinkhole: a violent enough quake tears the epicentre open.
        // Ordinary ground collapses into a Crater (unwalkable); water,
        // artifacts, and already-broken ground are left alone.
        let mut sinkhole = false;
        if magnitude >= SINKHOLE_MIN_MAGNITUDE {
            if let Some(grid) = terrain.as_mut() {
                if center.x >= 0 && center.y >= 0 {
                    #[allow(clippy::cast_sign_loss)]
                    let (ux, uy) = (center.x as usize, center.y as usize);
                    if let Some(current) = grid.get(ux, uy) {
                        if matches!(
                            current,
                            TerrainType::Grass
                                | TerrainType::Dirt
                                | TerrainType::Path
                                | TerrainType::Tree
                                | TerrainType::Shrub
                                | TerrainType::Sapling
                                | TerrainType::Ash
                                | TerrainType::Bridge
                        ) {
                            grid.set(ux, uy, TerrainType::Crater);
                            sinkhole = true;
                        }
                    }
                }
            }
        }
        if sinkhole {
            if let Some(s) = stats.as_mut() {
                s.sinkholes += 1;
            }
        }

        // Collapse destroyed buildings through the ruin pipeline
        // (mirrors fire / sabotage: despawn, Ruin + history + scrap
        // heap at the tile, BuildingRemovedEvent; the tile stays
        // occupied because the ruin blocks it, and the central
        // Added<Ruin> system writes the destruction chronicle).
        let tick = time.as_ref().map_or(0, |t| t.tick);
        let destroyed_this_event = destroyed.len();
        for (entity, pos, building_type, material) in destroyed {
            commands.entity(entity).despawn();
            commands.spawn((
                Ruin {
                    original_type: building_type,
                    material,
                },
                RuinHistory {
                    destruction_tick: tick,
                    reason: "Earthquake".to_string(),
                },
                pos,
            ));
            commands.spawn((
                crate::layer1::agriculture::compost::ScrapHeap {
                    metal: 8.0,
                    wood: 8.0,
                },
                pos,
            ));
            removed.send(BuildingRemovedEvent {
                entity,
                position: pos,
                building_type,
            });
            if let Some(s) = stats.as_mut() {
                s.buildings_destroyed += 1;
            }
        }
        // Visual feedback
        if let Some(ref mut s) = shake {
            s.trigger(0.5 * magnitude);
        }

        commands.spawn((
            crate::layer1::particles::Particle {
                char: '#',
                color: Color::Red,
                lifetime: 20,
            },
            center,
        ));

        // Chronicle notable quakes (small tremors from the vibration
        // pipeline stay un-chronicled to keep the book readable).
        if magnitude >= 3.0 || sinkhole || destroyed_this_event > 0 {
            let importance = if sinkhole || destroyed_this_event > 0 {
                EventImportance::Major
            } else {
                EventImportance::Standard
            };
            let mut text = format!(
                "The ground shook: a magnitude {magnitude:.1} quake struck at ({}, {}).",
                center.x, center.y
            );
            if sinkhole {
                text.push_str(" The earth gave way, opening a sinkhole.");
            }
            chronicle.send(AddChronicleEvent { text, importance });
        }
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn geological_public_strings() -> Vec<String> {
    vec![
        "The ground shook: a magnitude quake struck.".to_string(),
        "The earth gave way, opening a sinkhole.".to_string(),
        "Earthquake".to_string(),
        "Geological Instability".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use crate::layer1::geology::{
        add_seismic_stress, apply_geological_event_system, check_seismic_events, GeologicalEvent,
        SeismicGrid,
    };
    use crate::layer1::health::Health;
    use crate::layer1::GridPosition;
    use bevy_ecs::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    /// Init the event resources `apply_geological_event_system` now
    /// writes (chronicle + building-removed) — the first-tick pattern.
    fn init_apply_events(world: &mut World) {
        world.init_resource::<Events<GeologicalEvent>>();
        world.init_resource::<Events<crate::layer1::core::chronicle::AddChronicleEvent>>();
        world.init_resource::<Events<crate::layer1::core::events::BuildingRemovedEvent>>();
    }

    #[test]
    fn test_seismic_grid_initialization() {
        let grid = SeismicGrid::new(10, 10);
        assert_eq!(grid.get_stress(5, 5), 0.0);
    }

    #[test]
    fn test_mining_increases_stress() {
        let mut world = World::new();
        let grid = SeismicGrid::new(10, 10);
        world.insert_resource(grid);

        // Simulate mining at (5,5)
        add_seismic_stress(&mut world, GridPosition { x: 5, y: 5 }, 10.0);

        let grid = world.resource::<SeismicGrid>();
        assert!(grid.get_stress(5, 5) >= 10.0);
    }

    #[test]
    fn test_stress_decay() {
        let mut world = World::new();
        let mut grid = SeismicGrid::new(10, 10);
        grid.add_stress(5, 5, 50.0);
        world.insert_resource(grid);

        // Run decay system (simulated)
        world
            .run_system_once(crate::layer1::geology::seismic_decay_system)
            .unwrap();

        let grid = world.resource::<SeismicGrid>();
        assert!(grid.get_stress(5, 5) < 50.0);
    }

    #[test]
    fn test_earthquake_event_trigger() {
        let mut world = World::new();
        let mut grid = SeismicGrid::new(10, 10);
        // Set stress above threshold (e.g., 100.0)
        grid.add_stress(5, 5, 150.0);
        world.insert_resource(grid);
        world.init_resource::<Events<GeologicalEvent>>();

        // Run check system
        world.run_system_once(check_seismic_events).unwrap();

        // Check event
        let events = world.resource::<Events<GeologicalEvent>>();
        let mut reader = events.get_cursor();
        let emitted: Vec<_> = reader.read(events).collect();

        assert!(!emitted.is_empty());
        let center = emitted[0].center;
        assert_eq!(center.x, 5);
        assert_eq!(center.y, 5);
    }

    #[test]
    fn test_earthquake_damage() {
        let mut world = World::new();
        init_apply_events(&mut world);

        // Setup building/victim
        let victim = world
            .spawn((
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        world.insert_resource(crate::layer1::map::ScreenShake::default());

        // Send event
        world.send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: 5.0,
        });

        // Run system
        world
            .run_system_once(apply_geological_event_system)
            .unwrap();

        // Check damage
        let health = world.get::<Health>(victim).unwrap();
        assert!(health.current < 100.0);
    }

    // ---- Spec 389: adapted RED tests + the missing legs ----

    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::geology::{
        geological_stats_counts, next_random_interval, quake_radius, random_geological_event_system,
        random_quake_magnitude, seismic_damage_at, terrain_damage_multiplier, GeologicalDirector,
        GeologicalStats, SINKHOLE_MIN_MAGNITUDE,
    };
    use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::structure::Structure;

    fn quake_app() -> bevy_app::App {
        let mut app = bevy_app::App::new();
        app.add_event::<GeologicalEvent>();
        app.add_event::<crate::layer1::core::chronicle::AddChronicleEvent>();
        app.add_event::<crate::layer1::core::events::BuildingRemovedEvent>();
        app.insert_resource(crate::layer1::map::ScreenShake::default());
        app.insert_resource(GeologicalStats::default());
        app.add_systems(bevy_app::Update, apply_geological_event_system);
        app
    }

    /// Spec 389 RED test 1, adapted: the sketch's `SeismicEvent`
    /// {epicenter, magnitude 50, radius 2} against
    /// `layer1::buildings::Building` + `layer1::health::Health`
    /// becomes the real `GeologicalEvent` {center, magnitude 5}
    /// (radius derived: 5) against the real `Building` marker — and,
    /// because real buildings carry `Structure` health, a second
    /// building in the Structure shape is asserted too.
    #[test]
    fn test_seismic_event_damages_buildings() {
        let mut app = quake_app();

        let health_building = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 5, y: 5 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        let structure_building = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 6, y: 5 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: 5.0,
        });
        app.update();

        let health = app.world().get::<Health>(health_building).unwrap();
        assert!(
            health.current < 100.0,
            "Building should take damage from earthquake"
        );
        let structure = app.world().get::<Structure>(structure_building).unwrap();
        assert!(
            structure.current_hp < 100.0,
            "Structure building in radius should take attenuated damage"
        );
    }

    /// Spec 389 RED test 2, adapted: a building well outside the
    /// derived radius is unharmed (both health shapes).
    #[test]
    fn test_buildings_outside_radius_unharmed() {
        let mut app = quake_app();

        let health_building = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 10, y: 10 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        let structure_building = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 10, y: 10 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 0, y: 0 },
            magnitude: 5.0,
        });
        app.update();

        let health = app.world().get::<Health>(health_building).unwrap();
        assert_eq!(health.current, 100.0, "Building should not take damage");
        let structure = app.world().get::<Structure>(structure_building).unwrap();
        assert_eq!(structure.current_hp, 100.0);
    }

    #[test]
    fn test_damage_attenuates_with_euclidean_distance() {
        let at_center = seismic_damage_at(5.0, 0.0, None);
        let midway = seismic_damage_at(5.0, 2.5, None);
        let at_rim = seismic_damage_at(5.0, 5.0, None);
        let beyond = seismic_damage_at(5.0, 5.1, None);
        assert!((at_center - 50.0).abs() < f32::EPSILON);
        assert!(midway < at_center && midway > at_rim);
        assert!(at_rim > 0.0, "rim still inside the radius takes some damage");
        assert_eq!(beyond, 0.0, "outside the radius: no damage");
        assert_eq!(quake_radius(5.0), 5.0);
        // Euclidean, not Manhattan: a diagonal step costs sqrt(2).
        let diagonal = seismic_damage_at(5.0, std::f32::consts::SQRT_2, None);
        let straight = seismic_damage_at(5.0, 1.0, None);
        assert!(diagonal < straight);
    }

    #[test]
    fn test_terrain_modifier_bedrock_protects_soft_soil_amplifies() {
        assert_eq!(terrain_damage_multiplier(Some(TerrainType::Rock)), 0.5);
        assert_eq!(terrain_damage_multiplier(Some(TerrainType::DeepRock)), 0.5);
        assert_eq!(terrain_damage_multiplier(Some(TerrainType::Dirt)), 1.5);
        assert_eq!(terrain_damage_multiplier(Some(TerrainType::Grass)), 1.0);
        assert_eq!(terrain_damage_multiplier(None), 1.0);

        // Integration: two identical structures at the same distance,
        // one founded on bedrock, one on soft soil.
        let mut app = quake_app();
        let mut tiles = vec![TerrainType::Grass; 400];
        tiles[5 * 20 + 4] = TerrainType::Rock; // (4, 5)
        tiles[5 * 20 + 6] = TerrainType::Dirt; // (6, 5)
        app.world_mut().insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles,
        });
        let on_bedrock = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 4, y: 5 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();
        let on_soft_soil = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 6, y: 5 },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: 5.0,
        });
        app.update();

        let bedrock_hp = app.world().get::<Structure>(on_bedrock).unwrap().current_hp;
        let soft_hp = app.world().get::<Structure>(on_soft_soil).unwrap().current_hp;
        assert!(bedrock_hp < 100.0 && soft_hp < 100.0, "both in radius take damage");
        assert!(
            soft_hp < bedrock_hp,
            "soft soil amplifies, bedrock protects: bedrock {bedrock_hp}, soft {soft_hp}"
        );
    }

    #[test]
    fn test_quake_destroys_building_through_ruin_pipeline() {
        let mut app = quake_app();
        let fragile_house = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 10.0,
                    max_hp: 100.0,
                },
            ))
            .id();

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: 5.0,
        });
        app.update();

        assert!(
            app.world().get_entity(fragile_house).is_err(),
            "building at zero health is despawned"
        );
        let ruins: Vec<_> = {
            let mut q = app.world_mut().query::<(&crate::layer1::ruins::Ruin, &crate::layer1::ruins::RuinHistory, &GridPosition)>();
            q.iter(app.world()).map(|(r, h, p)| (r.original_type, h.reason.clone(), *p)).collect()
        };
        assert_eq!(ruins.len(), 1, "exactly one ruin left behind");
        assert_eq!(ruins[0].0, BuildingType::Housing);
        assert_eq!(ruins[0].1, "Earthquake");
        assert_eq!(ruins[0].2, GridPosition { x: 5, y: 5 });

        let removed_count = {
            let events = app
                .world()
                .resource::<Events<crate::layer1::core::events::BuildingRemovedEvent>>();
            let mut reader = events.get_cursor();
            reader.read(events).count()
        };
        assert_eq!(removed_count, 1, "BuildingRemovedEvent emitted");

        let stats = app.world().resource::<GeologicalStats>();
        assert_eq!(stats.buildings_destroyed, 1);
        assert_eq!(stats.total_events, 1);
    }

    #[test]
    fn test_high_magnitude_quake_opens_sinkhole() {
        let mut app = quake_app();
        app.world_mut().insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: SINKHOLE_MIN_MAGNITUDE,
        });
        app.update();

        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(
            terrain.get(5, 5),
            Some(TerrainType::Crater),
            "epicentre collapses into a sinkhole crater"
        );
        assert_eq!(terrain.get(6, 5), Some(TerrainType::Grass), "only the epicentre tears");
        let (_, sinkholes) = geological_stats_counts(app.world());
        assert_eq!(sinkholes, 1);
    }

    #[test]
    fn test_ordinary_quake_leaves_terrain_alone() {
        let mut app = quake_app();
        app.world_mut().insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });

        app.world_mut().send_event(GeologicalEvent {
            center: GridPosition { x: 5, y: 5 },
            magnitude: 5.0,
        });
        app.update();

        let terrain = app.world().resource::<TerrainGrid>();
        assert_eq!(terrain.get(5, 5), Some(TerrainType::Grass));
        let (_, sinkholes) = geological_stats_counts(app.world());
        assert_eq!(sinkholes, 0);
    }

    #[test]
    fn test_random_director_fires_and_rearms_deterministically() {
        // Countdown still running: decrements, fires nothing.
        let mut world = World::new();
        world.init_resource::<Events<GeologicalEvent>>();
        world.insert_resource(GeologicalDirector {
            ticks_until_next: 5,
            seed: 42,
        });
        world.insert_resource(SeismicGrid::new(20, 20));
        world.run_system_once(random_geological_event_system).unwrap();
        assert_eq!(world.resource::<GeologicalDirector>().ticks_until_next, 4);
        assert!(world.resource::<Events<GeologicalEvent>>().is_empty());

        // Countdown at zero: fires exactly one in-bounds quake with a
        // sub-sinkhole magnitude, then re-arms inside the interval band.
        let mut world = World::new();
        world.init_resource::<Events<GeologicalEvent>>();
        world.insert_resource(GeologicalDirector {
            ticks_until_next: 0,
            seed: 42,
        });
        world.insert_resource(SeismicGrid::new(20, 20));
        world.run_system_once(random_geological_event_system).unwrap();

        let events = world.resource::<Events<GeologicalEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1);
        assert!((0..20).contains(&fired[0].center.x));
        assert!((0..20).contains(&fired[0].center.y));
        assert!((3.0..6.0).contains(&fired[0].magnitude));
        assert_eq!(fired[0].magnitude, random_quake_magnitude(42));

        let director = world.resource::<GeologicalDirector>();
        assert!(director.ticks_until_next >= crate::layer1::geology::RANDOM_QUAKE_MIN_INTERVAL);
        assert!(director.ticks_until_next < crate::layer1::geology::RANDOM_QUAKE_MIN_INTERVAL + crate::layer1::geology::RANDOM_QUAKE_INTERVAL_SPREAD);
        assert_eq!(director.ticks_until_next, next_random_interval(director.seed));
    }

    #[test]
    fn test_geological_public_strings_have_no_lifted_ip() {
        // IP guard: every public-facing string this module emits must
        // be original/generic — no franchise names, no distinctive IP.
        const BANNED: &[&str] = &[
            "dwarf fortress",
            "stellaris",
            "richter",
            "san andreas",
            "godzilla",
            "weyland",
            "rimworld",
        ];
        for s in crate::layer1::geology::geological_public_strings() {
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

pub mod fracking;
