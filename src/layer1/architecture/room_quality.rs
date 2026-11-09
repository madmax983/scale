//! Room Quality system (Spec 064).
//!
//! # Context
//! This module calculates the quality of a room (like a Bedroom or Dining Room)
//! based on its size, enclosed state, and the beauty of items within it.
//! Pops receive positive or negative memories depending on the quality of the room they use.
//!
//! # Usage
//! ```rust,no_run
//! use bevy_ecs::prelude::*;
//! use scale::layer1::room_quality::calculate_room_quality;
//! use scale::layer1::map::GridPosition;
//!
//! fn evaluate_room_system(world: &mut World) {
//!     let pos = GridPosition { x: 5, y: 5 };
//!     let quality = calculate_room_quality(world, pos);
//!     println!("Room quality at {:?} is {}", pos, quality);
//! }
//! ```
//!
//! # Details
//! The total score is computed as `(Space + Beauty * 2.0) * Enclosure`.
//! `MAX_ROOM_SIZE` limits flood fill to prevent performance issues outdoors.
//!
//! # Links
//! - [`calculate_room_quality`]
//! - [`apply_room_quality_thoughts`]
//! - [`crate::layer1::zone::ZoneType`]
use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::beauty::BeautyGrid;
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::map::GridPosition;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::social_stratification::SocialClass;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::utility_types::StartPlan;
use crate::layer1::zone::{ZoneGrid, ZoneType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// System to apply thoughts when pops wake up or leave rooms.
///
/// Runs before `cleanup_previous_assignment_system` to capture the state of the room
/// the pop is leaving.
pub fn apply_waking_thoughts_system(world: &mut World) {
    // 1. Identify pops leaving housing or tavern
    // We collect entities to avoid borrowing conflicts when we call apply_room_quality_thoughts (which takes &mut World)
    let mut targets: Vec<(Entity, ZoneType)> = Vec::new();

    {
        let mut query_state = world.query_filtered::<(Entity, &AssignedTo), With<StartPlan>>();
        for (entity, assigned) in query_state.iter(world) {
            match assigned.assignment_type {
                AssignmentType::HousingResident => targets.push((entity, ZoneType::Bedroom)),
                AssignmentType::TavernVisitor => targets.push((entity, ZoneType::Dining)), // Assuming Tavern is Dining zone
                _ => {}
            }
        }
    }

    // 2. Apply thoughts
    for (pop_entity, zone_type) in targets {
        apply_room_quality_thoughts(world, pop_entity, zone_type);
    }
}

/// Calculates the quality score of a room at the given position.
///
/// This function performs a flood fill to find contiguous tiles with the same [`ZoneType`].
/// It evaluates the space, enclosure state (walls/rock), and sum of beauty from the [`BeautyGrid`].
///
/// # Examples
/// ```rust,no_run
/// use bevy_ecs::prelude::*;
/// use scale::layer1::room_quality::calculate_room_quality;
/// use scale::layer1::map::GridPosition;
/// use scale::layer1::zone::{ZoneGrid, ZoneType};
///
/// let mut world = World::new();
/// world.insert_resource(ZoneGrid::new(10, 10));
/// let quality = calculate_room_quality(&mut world, GridPosition { x: 1, y: 1 });
/// ```
fn is_boundary_enclosed(
    world: &World,
    terrain: &TerrainGrid,
    building_map: Option<&crate::layer1::building::BuildingMap>,
    nx: i32,
    ny: i32,
) -> bool {
    let (Ok(ux), Ok(uy)) = (usize::try_from(nx), usize::try_from(ny)) else {
        return true;
    };

    let is_rock = terrain.get(ux, uy) == Some(TerrainType::Rock);

    let is_wall = building_map.is_some_and(|map| {
        map.0.get(&(nx, ny)).is_some_and(|&entity| {
            world
                .get::<Building>(entity)
                .is_some_and(|b| b.building_type == BuildingType::Wall)
        })
    });

    // Spec 409: a crashed ship's hull ring encloses a room exactly
    // like a built wall — pops can live inside the wreck.
    let is_hull = crate::layer1::shipbreaking::is_hull_tile(world, nx, ny);

    let is_walkable = terrain.get(ux, uy).is_some_and(TerrainType::is_walkable);

    is_rock || is_wall || is_hull || !is_walkable
}

pub fn calculate_room_quality(world: &mut World, pos: GridPosition) -> f32 {
    // Cap room size to prevent infinite loops or massive CPU spikes
    const MAX_ROOM_SIZE: usize = 100;

    // ⚡ Bolt Optimization: Removed expensive runtime query + HashSet allocation.
    // We now use the globally maintained `BuildingMap` to look up entities
    // at a position in O(1) time and check their `BuildingType` directly,
    // avoiding a costly iteration over all buildings and a heap allocation
    // per `calculate_room_quality` call.
    let building_map = world.get_resource::<crate::layer1::building::BuildingMap>();

    let Some(zones) = world.get_resource::<ZoneGrid>() else {
        return 0.0;
    };
    let Some(beauty_grid) = world.get_resource::<BeautyGrid>() else {
        return 0.0;
    };
    let Some(terrain) = world.get_resource::<TerrainGrid>() else {
        return 0.0;
    };

    let start_zone = zones.get(pos.x, pos.y);
    if start_zone == ZoneType::None {
        return 0.0;
    }

    // Flood fill to find contiguous room tiles
    let mut visited = HashSet::new();
    let mut queue = vec![pos];
    let mut tiles = Vec::new();
    let mut enclosed = true;

    while let Some(p) = queue.pop() {
        if visited.contains(&p) {
            continue;
        }
        visited.insert(p);
        tiles.push(p);

        // Check neighbors
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let nx = p.x + dx;
            let ny = p.y + dy;

            // Bounds check
            if nx < 0
                || ny < 0
                || nx >= i32::try_from(zones.width).unwrap_or(i32::MAX)
                || ny >= i32::try_from(zones.height).unwrap_or(i32::MAX)
            {
                // Edge of map counts as enclosure (or void?)
                // Usually map edge is not enclosed unless walled.
                // Let's say map edge is NOT enclosed (void leaks air).
                enclosed = false;
                continue;
            }

            let neighbor_zone = zones.get(nx, ny);

            if neighbor_zone == start_zone {
                if !visited.contains(&GridPosition { x: nx, y: ny }) {
                    queue.push(GridPosition { x: nx, y: ny });
                }
            } else if !is_boundary_enclosed(world, terrain, building_map, nx, ny) {
                enclosed = false;
            }
        }

        if tiles.len() >= MAX_ROOM_SIZE {
            enclosed = false; // Too big to be a "room" usually implies outdoors
            break;
        }
    }

    // Calculate Scores
    #[allow(clippy::cast_precision_loss)]
    let space_score = tiles.len() as f32;
    let mut beauty_score = 0.0;

    for t in &tiles {
        if let (Ok(ux), Ok(uy)) = (usize::try_from(t.x), usize::try_from(t.y)) {
            beauty_score += beauty_grid.get(ux, uy);
        }
    }

    // Multipliers
    let enclosure_mult = if enclosed { 1.5 } else { 1.0 };

    // Formula from spec
    // Total = (Space + Beauty * 2.0) * Enclosure
    (space_score + beauty_score * 2.0) * enclosure_mult
}

/// Applies a thought/memory to a pop based on the quality of the room they just used.
pub fn apply_room_quality_thoughts(world: &mut World, pop_entity: Entity, zone_type: ZoneType) {
    // Get position of the pop
    let pos = if let Some(p) = world.get::<GridPosition>(pop_entity) {
        *p
    } else {
        return;
    };

    let social_class = world
        .get::<SocialClass>(pop_entity)
        .copied()
        .unwrap_or(SocialClass::Labor);

    // Calculate quality
    // Note: calculate_room_quality creates a query which requires read access to World.
    // But we have &mut World. This is fine.
    let quality = calculate_room_quality(world, pos);

    // Define thresholds based on social class
    // Labor: 10, 25, 50, 100
    // Middle: 20, 40, 75, 125
    // Elite: 40, 75, 125, 200
    let (t_awful, t_dull, t_decent, t_great) = match social_class {
        SocialClass::Labor => (10.0, 25.0, 50.0, 100.0),
        SocialClass::Middle => (20.0, 40.0, 75.0, 125.0),
        SocialClass::Elite => (40.0, 75.0, 125.0, 200.0),
    };

    // Determine MemoryType
    let memory_type = match zone_type {
        ZoneType::Bedroom => {
            if quality < t_awful {
                MemoryType::SleptInAwfulRoom
            } else if quality < t_dull {
                MemoryType::SleptInDullRoom
            } else if quality < t_decent {
                MemoryType::SleptInDecentRoom
            } else if quality < t_great {
                MemoryType::SleptInGreatRoom
            } else {
                MemoryType::SleptInLegendaryRoom
            }
        }
        ZoneType::Dining => {
            if quality < t_awful {
                MemoryType::AteInAwfulRoom
            } else if quality < t_dull {
                MemoryType::AteInDullRoom
            } else if quality < t_decent {
                MemoryType::AteInDecentRoom
            } else if quality < t_great {
                MemoryType::AteInGreatRoom
            } else {
                MemoryType::AteInLegendaryRoom
            }
        }
        _ => return,
    };

    // Get current tick
    let current_tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);

    // Add memory
    if let Some(mut memories) = world.get_mut::<Memories>(pop_entity) {
        memories.add(memory_type, current_tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::beauty::BeautyGrid;
    use crate::layer1::map::GridPosition;
    use crate::layer1::memory::{Memories, MemoryType};
    use crate::layer1::pop::Pop;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::zone::{ZoneGrid, ZoneType};

    // Helper to setup world with grids
    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(ZoneGrid::new(10, 10));
        world.insert_resource(BeautyGrid::new(10, 10));
        // Mock TerrainGrid for enclosure checks
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(SimulationTime::default());
        world.insert_resource(crate::layer1::building::BuildingMap(
            bevy::utils::HashMap::new(),
        ));
        world
    }

    #[test]
    fn test_calculate_base_quality() {
        let mut world = setup_world();
        let mut zones = world.resource_mut::<ZoneGrid>();

        // Create a 3x3 Bedroom
        for x in 0..3 {
            for y in 0..3 {
                zones.set(x, y, ZoneType::Bedroom);
            }
        }

        // Calculate quality for a tile inside the room
        let quality = calculate_room_quality(&mut world, GridPosition { x: 1, y: 1 });

        // Base quality logic:
        // Space: 9 tiles * 1.0 = 9.0
        // Beauty: 0.0
        // Enclosure: 1.0 (Open in stub test context unless we wall it)
        // For this test, we expect at least the space score if we assume open.
        // But if enclosure logic is strict, maybe 0.
        // Spec says: "Space: 9 tiles * 1.0 = 9.0 ... Total should be around 9.0"
        assert!(quality >= 9.0, "Quality {} should be >= 9.0", quality);
        assert!(quality < 15.0, "Quality {} should be < 15.0", quality);
    }

    #[test]
    fn test_beauty_increases_quality() {
        let mut world = setup_world();
        let mut zones = world.resource_mut::<ZoneGrid>();
        // 1x1 room for simplicity
        zones.set(5, 5, ZoneType::Dining);

        let mut beauty = world.resource_mut::<BeautyGrid>();
        beauty.set(5, 5, 10.0); // High beauty (Statue)

        let quality = calculate_room_quality(&mut world, GridPosition { x: 5, y: 5 });

        // Expect Base (1.0) + Beauty (10.0 * Multiplier 2.0 per spec) -> 21.0
        assert!(quality > 5.0, "Quality {} should be > 5.0", quality);
    }

    #[test]
    fn test_enclosure_bonus() {
        let mut world = setup_world();
        let mut zones = world.resource_mut::<ZoneGrid>();
        zones.set(1, 1, ZoneType::Bedroom);

        // Surround (1,1) with Walls
        use crate::layer1::building::{Building, BuildingType};
        for (nx, ny) in [(1, 0), (0, 1), (2, 1), (1, 2)] {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x: nx, y: ny },
            ));
        }

        // ⚡ Bolt: After spawning walls, we need to update the BuildingMap so calculate_room_quality can see them.
        // ⚡ Bolt Optimization: Pre-allocated vector capacity based on query size to eliminate dynamic heap reallocation.
        let mut query = world.query::<(bevy_ecs::entity::Entity, &GridPosition)>();
        let mut map_updates = Vec::with_capacity(query.iter(&world).len());
        for (entity, pos) in query.iter(&world) {
            map_updates.push(((pos.x, pos.y), entity));
        }
        let mut map = world.resource_mut::<crate::layer1::building::BuildingMap>();
        map.0.clear();
        for (pos, entity) in map_updates {
            map.0.insert(pos, entity);
        }

        let quality = calculate_room_quality(&mut world, GridPosition { x: 1, y: 1 });

        // 1x1 Room. Base 1.0. Enclosure Bonus x1.5 (spec).
        // Without walls: 1.0. With walls: 1.5.
        // 1.0 * 1.5 = 1.5
        assert!(
            quality >= 1.5,
            "Quality {} should include enclosure bonus",
            quality
        );
    }

    #[test]
    fn test_hull_enclosure_bonus() {
        // Spec 409: a room ringed by crashed-ship hull (no built walls)
        // counts as enclosed, exactly like a wall ring.
        let mut world = setup_world();
        {
            let mut zones = world.resource_mut::<ZoneGrid>();
            zones.set(1, 1, ZoneType::Bedroom);
        }
        for (nx, ny) in [(1, 0), (0, 1), (2, 1), (1, 2)] {
            world.spawn((
                crate::layer1::shipbreaking::HullTile,
                GridPosition { x: nx, y: ny },
            ));
        }
        let quality = calculate_room_quality(&mut world, GridPosition { x: 1, y: 1 });
        assert!(
            quality >= 1.5,
            "Quality {} should include hull enclosure bonus",
            quality
        );
    }

    #[test]
    fn test_apply_thought_based_on_quality() {
        let mut world = setup_world();

        // Spawn pop
        let pop = world
            .spawn((Pop, GridPosition { x: 0, y: 0 }, Memories::default()))
            .id();

        // We can't easily force calculate_room_quality to return a specific value without mocking or building a huge room.
        // But for this test, we want to verify that apply_room_quality_thoughts calls calculate_room_quality and adds a memory.

        // Let's build a "Legendary" room manually.
        // High beauty.
        let mut beauty = world.resource_mut::<BeautyGrid>();
        beauty.set(0, 0, 100.0);

        let mut zones = world.resource_mut::<ZoneGrid>();
        zones.set(0, 0, ZoneType::Bedroom);

        // Run the system
        apply_room_quality_thoughts(&mut world, pop, ZoneType::Bedroom);

        let memories = world.get::<Memories>(pop).unwrap();
        // Should have "SleptInLegendaryRoom" memory
        // Note: quality = (1 + 100*2) * 1.0 = 201 > 100 (Legendary threshold)
        assert!(
            memories
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::SleptInLegendaryRoom),
            "Should have SleptInLegendaryRoom memory"
        );
    }
}
