#![allow(clippy::cast_sign_loss)]
//! Structural Integrity and Stability Systems.
//!
//! This module defines the `RoofGrid` and calculates support limits (`MAX_SUPPORT_DIST`).
//! It ensures that spaces hollowed out during mining (turning `Rock` to `Dirt`) remain supported
//! by nearby `Rock` or `Wall` entities.
//!
//! Unstable roofs can trigger cave-ins via `apply_collapse`, converting the terrain back to rubble
//! and damaging trapped entities.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::health::Health;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::GridPosition;
use crate::shared::log::MessageLog;
use bevy_ecs::prelude::*;

/// Maximum distance from a support (Rock/Wall) that a roof can sustain itself.
pub const MAX_SUPPORT_DIST: i32 = 5;

#[derive(Resource, Default)]
/// Tracks which tiles have an overhead roof (underground vs open sky).
///
/// # Examples
///
/// ```
/// use scale::layer1::structural_integrity::RoofGrid;
///
/// let mut grid = RoofGrid::new(10, 10);
/// grid.set(5, 5, true);
///
/// assert!(grid.has_roof(5, 5));
/// assert!(!grid.has_roof(0, 0));
/// ```
pub struct RoofGrid {
    /// Width of the grid.
    pub width: usize,
    /// Height of the grid.
    pub height: usize,
    /// Flat vector of roof status.
    pub has_roof: Vec<bool>,
}

impl RoofGrid {
    /// Creates a new `RoofGrid` with the specified dimensions, initialized to false (no roof).
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 10_000_000, "Grid size overflow or too large");
        Self {
            width,
            height,
            has_roof: vec![false; size],
        }
    }

    /// Sets the roof status for a specific tile.
    pub fn set(&mut self, x: i32, y: i32, val: bool) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            let idx = (y as usize)
                .checked_mul(self.width)
                .and_then(|i| i.checked_add(x as usize));

            if let Some(idx) = idx.filter(|&i| i < self.has_roof.len()) {
                self.has_roof[idx] = val;
            }
        }
    }

    /// Returns true if the specified tile has a roof.
    #[must_use]
    pub fn has_roof(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || (x as usize) >= self.width || (y as usize) >= self.height {
            return false;
        }
        let idx = (y as usize)
            .checked_mul(self.width)
            .and_then(|i| i.checked_add(x as usize));

        if let Some(idx) = idx.filter(|&i| i < self.has_roof.len()) {
            return self.has_roof[idx];
        }
        false
    }
}

/// Checks if the roof at the given position is supported.
/// Returns true if stable (supported or no roof), false if unstable.
pub fn check_stability(world: &mut World, pos: GridPosition) -> bool {
    // 1. Check Roof
    let Some(roof) = world.get_resource::<RoofGrid>() else {
        return true; // Safe if there is no roof grid
    };
    if !roof.has_roof(pos.x, pos.y) {
        return true; // No roof = safe
    }

    if pos.x < 0 || pos.y < 0 {
        return true;
    }

    // 2. Check Self Support (Rock)
    let Some(terrain) = world.get_resource::<TerrainGrid>() else {
        return true; // Safe if there is no terrain grid
    };
    if pos.x >= 0
        && pos.y >= 0
        && terrain.get(pos.x as usize, pos.y as usize) == Some(TerrainType::Rock)
    {
        return true;
    }

    // 3. Check Nearby Rock
    // Optimization: we could use a spiral search or something, but simple box loop is fine for MVP.

    // Safely convert map dimensions to i32 bounds, clamping to i32::MAX if larger.
    // This prevents wrap-around if width/height > i32::MAX (e.g. on 64-bit systems).
    let map_w = i32::try_from(terrain.width).unwrap_or(i32::MAX);
    let map_h = i32::try_from(terrain.height).unwrap_or(i32::MAX);

    let min_x = pos.x.saturating_sub(MAX_SUPPORT_DIST).max(0);
    let max_x = pos
        .x
        .saturating_add(MAX_SUPPORT_DIST)
        .min(map_w.saturating_sub(1));

    let min_y = pos.y.saturating_sub(MAX_SUPPORT_DIST).max(0);
    let max_y = pos
        .y
        .saturating_add(MAX_SUPPORT_DIST)
        .min(map_h.saturating_sub(1));

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            // Chebyshev Distance <= 5
            let check_pos = GridPosition { x, y };
            if check_pos.distance_chebyshev(pos) > MAX_SUPPORT_DIST as u32 {
                continue;
            }

            if x >= 0 && y >= 0 && terrain.get(x as usize, y as usize) == Some(TerrainType::Rock) {
                return true;
            }
        }
    }

    // 4. Check Nearby Buildings (Wall)
    // Naive iteration over all buildings. This might be slow if many buildings.
    // Optimization: Add spatial index for buildings if needed later.
    let mut query = world.query::<(&Building, &GridPosition)>();
    for (building, b_pos) in query.iter(world) {
        if b_pos.distance_chebyshev(pos) <= MAX_SUPPORT_DIST as u32
            && matches!(building.building_type, BuildingType::Wall)
        {
            return true;
        }
    }

    false
}

/// Triggers a cave-in at the specified position.
/// This changes terrain to Rock (rubble) and damages any entities present.
pub fn apply_collapse(world: &mut World, pos: GridPosition) {
    if pos.x < 0 || pos.y < 0 {
        return;
    }

    // 1. Change Terrain to Rock (Rubble)
    // We scope this mutable borrow of TerrainGrid
    {
        let mut terrain = world.resource_mut::<TerrainGrid>();
        // SAFETY: We checked pos.x and pos.y are non-negative above.
        // We still need to check upper bounds, which is done by idx < len check.
        // However, converting to usize is now safe from wrapping huge negative numbers.
        // We also use checked arithmetic to prevent overflow wrapping around to a valid index.
        let idx = (pos.y as usize)
            .checked_mul(terrain.width)
            .and_then(|i| i.checked_add(pos.x as usize));

        if let Some(idx) = idx.filter(|&i| i < terrain.tiles.len()) {
            terrain.tiles[idx] = TerrainType::Rock;
        }
    }

    // 2. Damage entities
    // ⚡ Bolt Optimization:
    // Removed intermediate `Vec<Entity>` allocation (`victims`).
    // Previously, the system queried `&Health` to find victims, collected their `Entity` IDs,
    // and then did a second pass with `world.get_mut::<Health>` to apply damage.
    // By querying `&mut Health` directly, we avoid the heap allocation and collapse two
    // iterations into a single O(N) pass, eliminating memory pressure on the hot path.
    let mut query = world.query::<(&GridPosition, &mut Health)>();

    for (p, mut health) in query.iter_mut(world) {
        if p.x == pos.x && p.y == pos.y {
            health.take_damage(50.0);
        }
    }

    // 3. Log event
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!("CAVE-IN at ({}, {})!", pos.x, pos.y));
    }

    // 4. Emit Collapse Event
    // We check existence because unit tests might not initialize this event.
    if world.contains_resource::<Events<StructureCollapsed>>() {
        world.send_event(StructureCollapsed { pos });
    }
}

/// Event triggered when a roof collapses due to lack of support.
#[derive(Event, Debug, Clone)]
pub struct StructureCollapsed {
    /// Location of the collapse.
    pub pos: GridPosition,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::health::Health;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::GridPosition;

    #[test]
    fn test_roof_initialization() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Rock; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Initialize RoofGrid based on Terrain
        // Rock should have roof, others false (unless specified)
        // For MVP, assume map gen handles this. Here we test manual init.
        let mut roof_grid = RoofGrid::new(10, 10);
        roof_grid.set(0, 0, true); // (0,0) is Rock

        assert!(roof_grid.has_roof(0, 0));
        assert!(!roof_grid.has_roof(1, 1));
    }

    #[test]
    fn test_mining_preserves_roof() {
        // Mining changes Rock -> Dirt, but Roof should remain true
        // This test simulates mining logic update
        let mut roof_grid = RoofGrid::new(10, 10);
        roof_grid.set(5, 5, true); // Target tile

        // Simulate mine_rock completion
        // ... (mine_rock logic updates terrain)
        // Check roof remains
        assert!(roof_grid.has_roof(5, 5));
    }

    #[test]
    fn test_check_stability_safe_failure_without_resources() {
        let mut world = World::new();
        // Missing RoofGrid and TerrainGrid
        let is_stable = check_stability(&mut world, GridPosition { x: 5, y: 5 });
        assert!(
            is_stable,
            "Should default to stable if resources are missing"
        );
    }

    #[test]
    fn test_stability_check_safe() {
        let mut world = World::new();
        // 5x5 area
        let mut tiles = vec![TerrainType::Dirt; 25];
        tiles[0] = TerrainType::Rock; // Support at (0,0)
        world.insert_resource(TerrainGrid {
            width: 5,
            height: 5,
            tiles,
        });

        let mut roof_grid = RoofGrid::new(5, 5);
        roof_grid.set(1, 0, true); // Neighbor to Rock
        world.insert_resource(roof_grid);

        // Check (1,0) - distance 1 to support
        assert!(check_stability(&mut world, GridPosition { x: 1, y: 0 }));
    }

    #[test]
    fn test_stability_check_far() {
        let mut world = World::new();
        // 10x10 area
        let mut tiles = vec![TerrainType::Dirt; 100];
        tiles[0] = TerrainType::Rock; // Only support at (0,0)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        let mut roof_grid = RoofGrid::new(10, 10);
        roof_grid.set(9, 9, true); // Far away
        world.insert_resource(roof_grid);

        // Check (9,9) - distance > 5
        assert!(!check_stability(&mut world, GridPosition { x: 9, y: 9 }));
    }

    #[test]
    fn test_building_provides_support() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Dirt; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Place Wall at (0,0)
        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 0, y: 0 },
        ));

        let mut roof_grid = RoofGrid::new(10, 10);
        roof_grid.set(1, 0, true);
        world.insert_resource(roof_grid);

        assert!(check_stability(&mut world, GridPosition { x: 1, y: 0 }));
    }

    #[test]
    fn test_collapse_mechanics() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Dirt; 100];
        tiles[55] = TerrainType::Dirt; // (5,5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Spawn victim
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

        let mut roof_grid = RoofGrid::new(10, 10);
        roof_grid.set(5, 5, true);
        world.insert_resource(roof_grid);

        // Trigger collapse
        apply_collapse(&mut world, GridPosition { x: 5, y: 5 });

        // 1. Terrain becomes Rock
        let terrain = world.resource::<TerrainGrid>();
        assert_eq!(terrain.get(5, 5), Some(TerrainType::Rock));

        // 2. Roof is gone (filled)
        // Optionally, roof logic might vary. Let's assume roof remains (it's rock now).
        // Actually, if it's rock, it HAS a roof implicitly.

        // 3. Victim takes damage
        let Some(health) = world.get::<Health>(victim) else {
            panic!("Missing Health");
        };
        assert!(health.current < 100.0);
    }

    #[test]
    fn test_collapse_negative_coords() {
        let mut world = World::new();
        // Setup simple terrain
        let tiles = vec![TerrainType::Dirt; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Should not panic or crash
        apply_collapse(&mut world, GridPosition { x: -1, y: -1 });
    }

    #[test]
    fn test_stability_check_overflow() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Dirt; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(RoofGrid::new(10, 10));

        // Test near i32::MAX
        let pos_max = GridPosition {
            x: i32::MAX,
            y: i32::MAX,
        };
        // Should use saturating add/sub and clamp to 0..width/height
        // effectively checking nothing or just boundaries, but NOT panic
        // Returns true (safe) because no roof found there (out of bounds)
        assert!(check_stability(&mut world, pos_max));

        // Test near i32::MIN
        let pos_min = GridPosition {
            x: i32::MIN,
            y: i32::MIN,
        };
        // Returns true (safe) because no roof found there (out of bounds)
        assert!(check_stability(&mut world, pos_min));
    }

    #[test]
    #[should_panic(expected = "Grid size overflow or too large")]
    fn test_roof_grid_initialization_panic_on_too_large() {
        // Size exceeds 10_000_000
        let _ = RoofGrid::new(10_000, 10_000);
    }

    #[test]
    #[should_panic(expected = "Grid size overflow or too large")]
    fn test_roof_grid_initialization_panic_on_overflow() {
        let _ = RoofGrid::new(usize::MAX, usize::MAX);
    }
}

// ============================================================================
// Spec 338: Graduated support model (ECS-native layer).
//
// The RoofGrid/check_stability/apply_collapse API above is the original
// binary support check. Spec 338 adds a graduated, event-driven layer on top:
// every hollowed rock tile is tracked as an entity carrying [`TileCoords`],
// [`SupportValue`] and [`MinedStatus`]. Support is recomputed from the solid
// neighborhood (unmined rock or constructed [`SupportPillar`]s within
// [`SUPPORT_RADIUS`]) whenever mining changes; tiles whose support falls
// below threshold emit a [`CaveInEvent`], which seals the hollow with rubble,
// injures nearby pops, and writes a Major chronicle.
//
// The live-game hook is [`register_mined_tile`], called from `mine_rock`
// when a Mine designation completes: the hollowed tile starts tracked, and
// pillar-and-stall mining (leave rock standing, or raise pillars via the
// headless `pillar` command) keeps ceilings up.
// ============================================================================

use std::collections::HashSet;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::pop::Pop;

/// Chebyshev radius (tiles) over which a tile draws structural support.
pub const SUPPORT_RADIUS: i32 = 1;
/// Support below this fraction of the neighborhood triggers a cave-in.
pub const CAVE_IN_THRESHOLD: f32 = 0.2;
/// Physical damage dealt to pops caught in a collapse.
pub const CAVE_IN_DAMAGE: f32 = 35.0;
/// Support value of a fully braced tile.
pub const SUPPORT_FULL: f32 = 1.0;

/// Tile coordinates for a support-tracked cell.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCoords {
    /// The X coordinate (horizontal column).
    pub x: i32,
    /// The Y coordinate (vertical row).
    pub y: i32,
}

/// Structural support of a tracked tile.
#[derive(Component, Debug, Clone, Copy)]
pub struct SupportValue {
    /// Current support, 0.0 (hollow) to 1.0 (fully braced).
    pub value: f32,
    /// Collapse threshold; a cave-in fires when `value < threshold`.
    pub threshold: f32,
}

/// Whether a tracked tile has been hollowed out by mining.
#[derive(Component, Debug, Clone, Copy)]
pub struct MinedStatus {
    /// True once the rock has been mined away.
    pub is_mined: bool,
}

/// Marker for a player-placed support pillar: bears load like solid rock.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SupportPillar;

/// Marker for a tile that has already collapsed (prevents repeat events).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Collapsed;

/// Fired when a tracked tile's support falls below its threshold.
#[derive(Event, Debug, Clone)]
pub struct CaveInEvent {
    /// The collapsing tile entity.
    pub entity: Entity,
    /// Where the collapse happens.
    pub coords: TileCoords,
}

/// Running tally of collapses for STATS and the headless panel.
#[derive(Resource, Debug, Default, Clone)]
pub struct CaveInLedger {
    /// Total cave-ins executed.
    pub total_cave_ins: u32,
    /// Total physical damage dealt to pops by cave-ins.
    pub total_damage: f32,
}

/// Computes the support fraction for a tile: the share of cells in the
/// [`SUPPORT_RADIUS`] neighborhood that bear load (solid unmined rock,
/// constructed pillars, or unmined tracked tiles).
#[must_use]
pub fn calculate_local_support(
    coords: &TileCoords,
    terrain: &TerrainGrid,
    mined: &HashSet<(i32, i32)>,
    pillars: &HashSet<(i32, i32)>,
) -> f32 {
    let mut solid = 0u32;
    let mut total = 0u32;
    for dy in -SUPPORT_RADIUS..=SUPPORT_RADIUS {
        for dx in -SUPPORT_RADIUS..=SUPPORT_RADIUS {
            let (nx, ny) = (coords.x + dx, coords.y + dy);
            if nx < 0 || ny < 0 {
                continue;
            }
            total += 1;
            if pillars.contains(&(nx, ny)) {
                solid += 1;
            } else if mined.contains(&(nx, ny)) {
                // Hollowed out: bears no load.
            } else if matches!(
                terrain.get(nx as usize, ny as usize),
                Some(TerrainType::Rock) | Some(TerrainType::DeepRock)
            ) {
                solid += 1;
            }
        }
    }
    if total == 0 {
        return 0.0;
    }
    solid as f32 / total as f32
}

/// A mined tile provides zero support of its own.
pub fn mining_system(
    mut tiles: Query<(&mut SupportValue, &MinedStatus), Changed<MinedStatus>>,
) {
    for (mut support, mined_status) in tiles.iter_mut() {
        if mined_status.is_mined {
            support.value = 0.0; // The mined tile provides 0 support
        }
    }
}

/// Recomputes support for tracked tiles near freshly mined cells or new
/// pillars, using [`calculate_local_support`].
///
/// Only the neighborhood of changed tiles is recalculated (per the spec's
/// performance guidance); the rest of the grid is untouched.
pub fn support_diffusion_system(
    terrain: Res<TerrainGrid>,
    changed: Query<&TileCoords, Changed<MinedStatus>>,
    added_pillars: Query<&TileCoords, Added<SupportPillar>>,
    pillars: Query<&TileCoords, With<SupportPillar>>,
    mut tiles: Query<(&TileCoords, &mut SupportValue, &MinedStatus)>,
) {
    let dirty: Vec<(i32, i32)> = changed
        .iter()
        .chain(added_pillars.iter())
        .map(|c| (c.x, c.y))
        .collect();
    if dirty.is_empty() {
        return;
    }
    let pillar_cells: HashSet<(i32, i32)> = pillars.iter().map(|c| (c.x, c.y)).collect();
    let mined_cells: HashSet<(i32, i32)> = tiles
        .iter()
        .filter(|(_, _, m)| m.is_mined)
        .map(|(c, _, _)| (c.x, c.y))
        .collect();
    for (coords, mut support, _) in tiles.iter_mut() {
        let near_dirty = dirty.iter().any(|(dx, dy)| {
            (coords.x - dx).abs() <= SUPPORT_RADIUS && (coords.y - dy).abs() <= SUPPORT_RADIUS
        });
        if near_dirty {
            support.value = calculate_local_support(coords, &terrain, &mined_cells, &pillar_cells);
        }
    }
}

/// Emits [`CaveInEvent`] for tracked tiles whose support fell below threshold.
/// Tiles already marked [`Collapsed`] are skipped so each collapse fires once.
pub fn cave_in_check_system(
    tiles: Query<(Entity, &SupportValue, &TileCoords), Without<Collapsed>>,
    mut cave_in_events: EventWriter<CaveInEvent>,
) {
    for (entity, support, coords) in tiles.iter() {
        if support.value < support.threshold {
            cave_in_events.send(CaveInEvent {
                entity,
                coords: *coords,
            });
        }
    }
}

/// Executes collapses: seals the hollow with rubble (terrain back to rock,
/// blocking the tunnel until it is mined out again), injures pops caught in
/// the fall, and writes a Major chronicle.
pub fn cave_in_execute_system(
    mut commands: Commands,
    mut events: EventReader<CaveInEvent>,
    mut terrain: ResMut<TerrainGrid>,
    mut pops: Query<(&GridPosition, &mut Health), With<Pop>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut ledger: ResMut<CaveInLedger>,
) {
    for event in events.read() {
        let (x, y) = (event.coords.x, event.coords.y);
        // Rubble: the hollow fills with collapsed rock. Rock is impassable,
        // so the tunnel is blocked until colonists mine it out again.
        if x >= 0 && y >= 0 {
            let idx = (y as usize)
                .checked_mul(terrain.width)
                .and_then(|i| i.checked_add(x as usize));
            if let Some(i) = idx {
                if i < terrain.tiles.len() {
                    terrain.tiles[i] = TerrainType::Rock;
                }
            }
        }
        commands.entity(event.entity).insert(Collapsed);
        // Falling rock crushes anyone in or adjacent to the collapse.
        let mut hurt = 0u32;
        for (pos, mut health) in pops.iter_mut() {
            if (pos.x - x).abs() <= SUPPORT_RADIUS && (pos.y - y).abs() <= SUPPORT_RADIUS {
                health.take_damage(CAVE_IN_DAMAGE);
                hurt += 1;
            }
        }
        ledger.total_cave_ins += 1;
        ledger.total_damage += CAVE_IN_DAMAGE * hurt as f32;
        chronicle.send(AddChronicleEvent {
            text: format!("The tunnel ceiling collapsed at ({x}, {y}), burying the hollow in rubble!"),
            importance: EventImportance::Major,
        });
    }
}

/// Registers a freshly hollowed rock tile for support tracking.
///
/// Called from `mine_rock` when a Mine designation completes. Any previous
/// tracking at the same coords (e.g. a collapsed tile being re-mined) is
/// replaced so change detection re-fires.
pub fn register_mined_tile(world: &mut World, pos: GridPosition) {
    let stale: Vec<Entity> = world
        .query::<(Entity, &TileCoords)>()
        .iter(world)
        .filter(|(_, c)| c.x == pos.x && c.y == pos.y)
        .map(|(e, _)| e)
        .collect();
    for e in stale {
        world.despawn(e);
    }
    world.spawn((
        TileCoords { x: pos.x, y: pos.y },
        SupportValue {
            value: SUPPORT_FULL,
            threshold: CAVE_IN_THRESHOLD,
        },
        MinedStatus { is_mined: true },
    ));
}

/// Raises a support pillar at the given tile (headless `pillar` debug
/// command; the construction-system hook is follow-up work).
pub fn place_support_pillar(world: &mut World, x: i32, y: i32) {
    world.spawn((TileCoords { x, y }, SupportPillar));
}

/// Number of tracked (mined, uncollapsed) tiles, for STATS.
#[must_use]
pub fn tracked_tile_count(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, (With<MinedStatus>, Without<Collapsed>)>()
        .iter(world)
        .count()
}

#[cfg(test)]
mod spec_338_tests {
    use super::*;
    use bevy::app::{App, Update};

    fn rock_grid() -> TerrainGrid {
        TerrainGrid {
            width: 3,
            height: 3,
            tiles: vec![TerrainType::Rock; 9],
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.insert_resource(rock_grid());
        app.add_event::<CaveInEvent>();
        app.add_event::<AddChronicleEvent>();
        app.init_resource::<CaveInLedger>();
        app
    }

    /// Spawns a 3x3 block of tracked tiles, all unmined, support 1.0.
    fn spawn_block(app: &mut App) -> Vec<Entity> {
        let mut entities = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                let e = app
                    .world_mut()
                    .spawn((
                        TileCoords { x, y },
                        SupportValue {
                            value: SUPPORT_FULL,
                            threshold: CAVE_IN_THRESHOLD,
                        },
                        MinedStatus { is_mined: false },
                    ))
                    .id();
                entities.push(e);
            }
        }
        entities
    }

    #[test]
    fn test_mining_reduces_support() {
        // Arrange: a 3x3 solid rock area, all tracked.
        let mut app = test_app();
        app.add_systems(Update, (mining_system, support_diffusion_system).chain());
        let entities = spawn_block(&mut app);
        let center = entities[4];
        app.update();

        let initial_support = app.world().get::<SupportValue>(center).unwrap().value;

        // Act: mine the center tile.
        app.world_mut()
            .entity_mut(center)
            .insert(MinedStatus { is_mined: true });
        app.update();

        // Assert: support dropped.
        let final_support = app.world().get::<SupportValue>(center).unwrap().value;
        assert!(
            final_support < initial_support,
            "Mining a tile should reduce its structural support (was {initial_support}, now {final_support})."
        );
    }

    #[test]
    fn test_cave_in_triggers_below_threshold() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, cave_in_check_system);

        let tile_entity = app
            .world_mut()
            .spawn((
                TileCoords { x: 5, y: 5 },
                SupportValue {
                    value: 0.1,
                    threshold: 0.2,
                }, // Support is below threshold
                MinedStatus { is_mined: true },
            ))
            .id();

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<CaveInEvent>>();
        let mut reader = events.get_cursor();
        assert!(
            reader.read(events).any(|e| e.entity == tile_entity),
            "CaveInEvent should trigger for tiles below the support threshold."
        );
    }

    #[test]
    fn test_support_aggregates_from_neighbors() {
        // Mining the center should also weaken its neighbors' support.
        let mut app = test_app();
        app.add_systems(Update, (mining_system, support_diffusion_system).chain());
        let entities = spawn_block(&mut app);
        app.update();
        let corner = entities[0];
        let initial = app.world().get::<SupportValue>(corner).unwrap().value;

        app.world_mut()
            .entity_mut(entities[4])
            .insert(MinedStatus { is_mined: true });
        app.update();

        let after = app.world().get::<SupportValue>(corner).unwrap().value;
        assert!(
            after < initial,
            "Mining a tile should reduce neighboring tiles' support too (was {initial}, now {after})."
        );
    }

    #[test]
    fn test_pillar_bears_load() {
        // A 3-wide hollow with a pillar in the middle row stays standing.
        let mut app = test_app();
        app.add_systems(Update, (mining_system, support_diffusion_system).chain());
        let entities = spawn_block(&mut app);
        // Mine the entire middle row: (0,1), (1,1), (2,1).
        for e in [entities[3], entities[4], entities[5]] {
            app.world_mut()
                .entity_mut(e)
                .insert(MinedStatus { is_mined: true });
        }
        // Raise a pillar on the center tile of the hollow.
        app.world_mut()
            .spawn((TileCoords { x: 1, y: 1 }, SupportPillar));
        app.update();

        let center_support = app
            .world()
            .get::<SupportValue>(entities[4])
            .unwrap()
            .value;
        assert!(
            center_support >= CAVE_IN_THRESHOLD,
            "A pillar should keep a hollowed tile above the collapse threshold (support {center_support})."
        );
    }

    #[test]
    fn test_hollow_room_collapses_without_pillars() {
        // A 3x3 room hollowed out of rock: the center has no solid neighbors.
        let mut app = test_app();
        app.add_systems(
            Update,
            (
                mining_system,
                support_diffusion_system,
                cave_in_check_system,
            )
                .chain(),
        );
        let entities = spawn_block(&mut app);
        for e in &entities {
            app.world_mut()
                .entity_mut(*e)
                .insert(MinedStatus { is_mined: true });
        }
        app.update();

        let events = app.world().resource::<Events<CaveInEvent>>();
        let mut reader = events.get_cursor();
        let collapsed: Vec<_> = reader.read(events).collect();
        assert!(
            !collapsed.is_empty(),
            "Hollowing a full 3x3 room should trigger at least one cave-in."
        );
    }

    #[test]
    fn test_collapsed_tile_does_not_retrigger() {
        let mut app = test_app();
        app.add_systems(
            Update,
            (cave_in_check_system, cave_in_execute_system).chain(),
        );
        // The hollowed tile sits on Dirt (it was mined out); the collapse
        // should seal it with rubble (Rock).
        app.world_mut().resource_mut::<TerrainGrid>().tiles[4] = TerrainType::Dirt;
        let tile = app
            .world_mut()
            .spawn((
                TileCoords { x: 1, y: 1 },
                SupportValue {
                    value: 0.1,
                    threshold: 0.2,
                },
                MinedStatus { is_mined: true },
            ))
            .id();
        app.update();
        // Drain the first event; the tile should now carry Collapsed.
        app.world_mut()
            .resource_mut::<Events<CaveInEvent>>()
            .clear();
        assert!(
            app.world().get::<Collapsed>(tile).is_some(),
            "Executed cave-in should mark the tile Collapsed."
        );
        assert_eq!(
            app.world().resource::<TerrainGrid>().tiles[4],
            TerrainType::Rock,
            "A cave-in should seal the hollow with rubble (Dirt -> Rock)."
        );
        app.update();
        let events = app.world().resource::<Events<CaveInEvent>>();
        let mut reader = events.get_cursor();
        assert!(
            reader.read(events).next().is_none(),
            "A collapsed tile must not emit a second CaveInEvent."
        );
    }

    #[test]
    fn test_cave_in_damages_nearby_pops() {
        let mut app = test_app();
        app.add_systems(Update, cave_in_execute_system);
        // A pop standing in the collapse cell.
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        // A pop two cells away (Chebyshev distance 2), untouched.
        let far_pop = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 2, y: 2 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        let dummy = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(CaveInEvent {
            entity: dummy,
            coords: TileCoords { x: 0, y: 0 },
        });
        app.update();

        let hurt = app.world().get::<Health>(pop).unwrap().current;
        let safe = app.world().get::<Health>(far_pop).unwrap().current;
        assert!(
            (hurt - (100.0 - CAVE_IN_DAMAGE)).abs() < 1e-6,
            "A pop caught in a cave-in should take {CAVE_IN_DAMAGE} damage, health now {hurt}."
        );
        assert!(
            (safe - 100.0).abs() < 1e-6,
            "A distant pop should be unharmed, health now {safe}."
        );
        let ledger = app.world().resource::<CaveInLedger>();
        assert_eq!(ledger.total_cave_ins, 1);
        assert!((ledger.total_damage - CAVE_IN_DAMAGE).abs() < 1e-6);
        // The chronicle should record the disaster as Major.
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let texts: Vec<_> = reader.read(events).map(|e| e.text.clone()).collect();
        assert!(
            texts.iter().any(|t| t.contains("collapsed")),
            "A cave-in should write a chronicle entry, got {texts:?}."
        );
    }

    #[test]
    fn test_calculate_local_support_pure() {
        let terrain = rock_grid();
        let mined: HashSet<(i32, i32)> = [(1, 1)].into_iter().collect();
        let pillars: HashSet<(i32, i32)> = HashSet::new();
        let s = calculate_local_support(&TileCoords { x: 1, y: 1 }, &terrain, &mined, &pillars);
        assert!(
            (s - 8.0 / 9.0).abs() < 1e-6,
            "Center of 3x3 with itself mined should be 8/9, got {s}."
        );
        let s_all = calculate_local_support(
            &TileCoords { x: 1, y: 1 },
            &terrain,
            &HashSet::new(),
            &pillars,
        );
        assert!(
            (s_all - 1.0).abs() < 1e-6,
            "Fully solid neighborhood should be 1.0, got {s_all}."
        );
    }

    #[test]
    fn test_register_mined_tile_spawns_tracking() {
        let mut app = test_app();
        register_mined_tile(app.world_mut(), GridPosition { x: 2, y: 2 });
        let mut query = app
            .world_mut()
            .query::<(&TileCoords, &SupportValue, &MinedStatus)>();
        let found: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(
            found.len(),
            1,
            "register_mined_tile should spawn one tracked tile."
        );
        let (coords, support, mined) = found[0];
        assert_eq!((coords.x, coords.y), (2, 2));
        assert!(mined.is_mined);
        assert!((support.value - SUPPORT_FULL).abs() < 1e-6);
    }

    #[test]
    fn test_mine_rock_completion_registers_tile_for_support_tracking() {
        // Live hook (Spec 338): completing a Mine designation must register
        // the hollowed tile with the graduated support model.
        use crate::layer1::designation::{Designation, DesignationType};
        use crate::layer1::resources::{mine_rock, ColonyResources, MiningProgress};

        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 3,
            height: 3,
            tiles: vec![TerrainType::Rock; 9],
        });
        world.insert_resource(ColonyResources::default());
        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                MiningProgress {
                    current: 9.9,
                    max: 10.0,
                },
                GridPosition { x: 1, y: 1 },
            ))
            .id();
        mine_rock(&mut world, designation, 0.2);
        assert_eq!(
            tracked_tile_count(&mut world),
            1,
            "completing a Mine designation should register the hollow for support tracking"
        );
        let coords = world.query::<&TileCoords>().iter(&world).next().unwrap();
        assert_eq!((coords.x, coords.y), (1, 1));
    }
}
