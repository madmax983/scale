//! Pathfinding and navigation logic.
//!
//! This module implements the A* (A-Star) search algorithm to navigate agents across the colony grid.
//! It handles complex movement rules including terrain costs, crowding penalties, and building access control.
//!
//! # The Movement Model
//!
//! ## 1. Algorithm
//! We use **A*** with a **Manhattan distance** heuristic (`|dx| + |dy|`). This is optimal for our
//! 4-connected grid (North, South, East, West movement only).
//!
//! ## 2. Walkability Logic
//! A tile is considered "walkable" only if **ALL** of the following are true:
//! *   **Bounds**: The coordinate is within the map dimensions.
//! *   **Terrain**: The `TerrainType` allows movement (e.g., `Grass` is walkable, `Water` is not).
//! *   **Occupancy**: The tile is not in the `OccupiedTiles` set (generic blockers).
//! *   **Buildings**: If a building exists on the tile:
//!     *   **Physical**: It must not be an obstacle (e.g., `Wall` blocks, `Floor` does not).
//!     *   **Door State**: If it has a `DoorControl`, it must not be `Locked`.
//!     *   **Access Control**: If it has `AccessControl`, the agent must have permission (Public, Role, or specific ID).
//!     *   **Special**: Vents are walkable only for entities with the `Vermin` capability.
//!
//! ## 3. Cost Function
//! The cost to enter a tile `C` is calculated as:
//!
//! $$ C = C_{terrain} + C_{crowding} + C_{clutter} $$
//!
//! *   $C_{terrain}$: Base cost from `TerrainType` (e.g., Dirt=1, Rock=2).
//! *   $C_{crowding}$: Dynamic penalty from `CrowdingGrid` (high traffic = slower movement).
//! *   $C_{clutter}$: Dynamic penalty from `ClutterGrid` (high clutter = slower movement).
//!
//! This encourages agents to use "Desire Paths" (paved roads) and avoid congested hallways.

use bevy_ecs::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::layer1::access_control::{AccessControl, AccessMode};
use crate::layer1::building::{Building, BuildingMap, BuildingType, OccupiedTiles};
use crate::layer1::control::{DoorControl, DoorState};

use crate::layer1::pop::Role;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::wind::{calculate_wind_movement_penalty, Vec2, WindGrid};

struct AccessCredentials {
    entity: Entity,
    role: Option<Role>,
}

/// Node for A* pathfinding.
///
/// Represents a step in the path search.
///
/// * `pos`: The (x, y) grid coordinate.
/// * `cost`: The `g` score (cost from start to this node).
/// * `heuristic`: The `h` score (estimated cost from this node to goal).
/// * The priority queue uses `f = g + h`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    pos: (i32, i32),
    cost: i32,      // Accumulated cost (g)
    heuristic: i32, // Heuristic to target (h)
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap (lowest f = g + h first)
        (other.cost + other.heuristic)
            .cmp(&(self.cost + self.heuristic))
            .then_with(|| other.cost.cmp(&self.cost))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Finds a path between two points ignoring access restrictions.
///
/// This is useful for generic queries (e.g., "Is this area reachable at all?").
/// For agent movement, use [`find_path_for_pop`] or [`find_path_for_entity`] instead.
///
/// Returns `None` if no path is found.
///
/// # Arguments
///
/// * `world` - The Bevy World (must contain `TerrainGrid`, `OccupiedTiles`, `BuildingMap`).
/// * `start` - Starting coordinates (x, y).
/// * `end` - Target coordinates (x, y).
///
/// # Examples
///
/// ```
/// use scale::layer1::pathfinding::find_path;
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::{BuildingMap, OccupiedTiles};
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// // Setup minimal map resources
/// world.insert_resource(TerrainGrid {
///     width: 10,
///     height: 10,
///     tiles: vec![TerrainType::Grass; 100],
/// });
/// world.insert_resource(OccupiedTiles::default());
/// world.insert_resource(BuildingMap::default());
///
/// let path = find_path(&world, (0, 0), (2, 0));
/// assert!(path.is_some());
/// let p = path.expect("Path should be found in doc test");
/// assert_eq!(p.last(), Some(&(2, 0)));
/// ```
pub fn find_path(world: &World, start: (i32, i32), end: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    find_path_internal(world, start, end, false, None)
}

/// Finds a path for a specific Pop, considering their access rights.
///
/// This function checks `AccessControl` (e.g., "Staff Only" doors) and `DoorControl` (Locked doors).
/// A Pop with the correct [`Role`] or specific ID permission can traverse restricted areas.
///
/// # Arguments
///
/// * `world` - The Bevy World.
/// * `start` - Starting coordinates.
/// * `end` - Target coordinates.
/// * `pop` - The Pop entity (checked for [`Role`] component).
///
/// # Examples
///
/// ```
/// use scale::layer1::pathfinding::find_path_for_pop;
/// use scale::layer1::pop::Pop;
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::{BuildingMap, OccupiedTiles};
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// world.insert_resource(TerrainGrid {
///     width: 10,
///     height: 10,
///     tiles: vec![TerrainType::Grass; 100],
/// });
/// world.insert_resource(OccupiedTiles::default());
/// world.insert_resource(BuildingMap::default());
///
/// let pop = world.spawn(scale::layer1::pop::Pop).id();
///
/// // Standard pathfinding for this pop
/// let path = find_path_for_pop(&world, (0, 0), (5, 5), pop);
/// assert!(path.is_some());
/// ```
pub fn find_path_for_pop(
    world: &World,
    start: (i32, i32),
    end: (i32, i32),
    pop: Entity,
) -> Option<Vec<(i32, i32)>> {
    let role = world.get::<Role>(pop).copied();
    let credentials = Some(AccessCredentials { entity: pop, role });
    find_path_internal(world, start, end, false, credentials.as_ref())
}

/// Finds a path for a specific entity type, considering unique capabilities.
///
/// Useful for entities like **Vermin**, which can traverse Vents that are impassable to humans.
///
/// # Arguments
///
/// * `world` - The Bevy World.
/// * `start` - Starting coordinates.
/// * `end` - Target coordinates.
/// * `_capability` - The marker component (e.g. [`crate::layer1::vermin::Vermin`]).
pub fn find_path_for_entity<T: Component>(
    world: &World,
    start: (i32, i32),
    end: (i32, i32),
    _capability: &T,
) -> Option<Vec<(i32, i32)>> {
    let can_use_vents =
        std::any::TypeId::of::<T>() == std::any::TypeId::of::<crate::layer1::vermin::Vermin>();
    find_path_internal(world, start, end, can_use_vents, None)
}

/// Internal A* implementation.
///
/// Shared logic for all pathfinding variants. It abstracts the "Can I enter this tile?" logic
/// into the helper `is_walkable`.
#[allow(clippy::too_many_lines)]
fn find_path_internal(
    world: &World,
    start: (i32, i32),
    end: (i32, i32),
    can_use_vents: bool,
    credentials: Option<&AccessCredentials>,
) -> Option<Vec<(i32, i32)>> {
    let terrain = world.resource::<TerrainGrid>();
    let occupied = world.get_resource::<OccupiedTiles>();
    let building_map = world.resource::<BuildingMap>();
    let crowding = world.get_resource::<crate::layer1::crowding::CrowdingGrid>();
    let clutter = world.get_resource::<crate::layer1::clutter::ClutterGrid>();
    let wind_grid = world.get_resource::<WindGrid>();
    // Spec 406: quarantine edge severance on the real ZoneGrid.
    let zone_grid = world.get_resource::<crate::layer1::zone::ZoneGrid>();

    #[cfg(feature = "nova")]
    let ghost_grid = world.get_resource::<crate::experimental::ghost_grid::GhostGrid>();

    let width = terrain.width;
    let height = terrain.height;
    // let size = width * height;

    // Use flat vectors for O(1) access.
    // u32::MAX serves as "None" for parent index.

    let mut stains_map: bevy::utils::HashMap<(i32, i32), f32> = bevy::utils::HashMap::new();
    let stain_component_id = world
        .components()
        .component_id::<crate::layer1::psychology::psychic_stains::PsychicStain>(
    );
    let pos_component_id = world
        .components()
        .component_id::<crate::layer1::GridPosition>();

    if let (Some(stain_id), Some(pos_id)) = (stain_component_id, pos_component_id) {
        for archetype in world.archetypes().iter() {
            if archetype.contains(stain_id) && archetype.contains(pos_id) {
                for entity in archetype.entities() {
                    if let (Some(pos), Some(stain)) = (
                        world.get::<crate::layer1::GridPosition>(entity.id()),
                        world.get::<crate::layer1::psychology::psychic_stains::PsychicStain>(
                            entity.id(),
                        ),
                    ) {
                        stains_map.insert((pos.x, pos.y), stain.trauma_level);
                    }
                }
            }
        }
    }

    let mut came_from: bevy::utils::HashMap<usize, u32> = bevy::utils::HashMap::default();
    let mut cost_so_far: bevy::utils::HashMap<usize, i32> = bevy::utils::HashMap::default();
    let mut open_set = BinaryHeap::new();

    // Helper to get index from pos
    let get_idx = |(x, y): (i32, i32)| -> Option<usize> {
        let ux = usize::try_from(x).ok()?;
        let uy = usize::try_from(y).ok()?;
        if ux < width && uy < height {
            uy.checked_mul(width).and_then(|i| i.checked_add(ux))
        } else {
            None
        }
    };

    if let Some(start_idx) = get_idx(start) {
        cost_so_far.insert(start_idx, 0);
        open_set.push(Node {
            pos: start,
            cost: 0,
            heuristic: manhattan_distance(start, end),
        });
    } else {
        return None;
    }

    while let Some(Node { pos, cost, .. }) = open_set.pop() {
        if pos == end {
            // Reconstruct path
            let mut path = Vec::new();
            let mut current = end;
            while current != start {
                path.push(current);
                let idx = get_idx(current)?;
                let parent_idx = *came_from.get(&idx)?;
                // Convert index back to pos
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let px = (parent_idx as usize % width) as i32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let py = (parent_idx as usize / width) as i32;
                current = (px, py);
            }
            path.reverse();
            return Some(path);
        }

        let Some(current_idx) = get_idx(pos) else {
            continue;
        };

        // Check if we found a shorter path already (standard A* opt)
        if cost > *cost_so_far.get(&current_idx).unwrap_or(&i32::MAX) {
            continue;
        }

        // Check neighbors (Manhattan)
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let next = (pos.0 + dx, pos.1 + dy);
            let Some(next_idx) = get_idx(next) else {
                continue;
            };

            // Spec 406: sever quarantine boundary edges (inside<->outside)
            // while leaving inside->inside / outside->outside edges open.
            if let Some(zg) = zone_grid {
                if crate::layer1::quarantine::is_quarantine_edge_blocked(zg, pos, next) {
                    continue;
                }
            }

            // Check if walkable
            if !is_walkable(
                next,
                world,
                terrain,
                occupied,
                building_map,
                can_use_vents,
                credentials,
                next == end,
            ) {
                continue;
            }

            // Movement cost
            #[allow(clippy::cast_possible_truncation)]
            let t_cost = terrain.tiles[next_idx].movement_cost() as i32;
            #[allow(clippy::cast_possible_truncation)]
            let c_cost = crowding.map_or(0, |c| i32::from(c.get(next.0 as usize, next.1 as usize)));
            // Clutter penalty: Each 20.0 clutter adds 1 cost
            #[allow(clippy::cast_possible_truncation)]
            let clutter_cost = clutter.map_or(0, |c| {
                (c.get(next.0 as usize, next.1 as usize) / 20.0) as i32
            });

            // Stain penalty: each 10 trauma adds 1 cost
            #[allow(clippy::cast_possible_truncation)]
            let stain_cost = stains_map
                .get(&(next.0, next.1))
                .map_or(0, |trauma| (*trauma / 10.0) as i32);
            #[cfg(feature = "nova")]
            let ghost_cost = ghost_grid.map_or(0, |gg| gg.get_penalty(next.0, next.1));
            #[cfg(not(feature = "nova"))]
            let ghost_cost = 0;

            let base_cost = t_cost + c_cost + clutter_cost + stain_cost + ghost_cost;

            // Calculate wind penalty
            let wind_penalty = wind_grid.map_or(1.0, |wg| {
                let wind = wg.get_wind(next.0, next.1);
                #[allow(clippy::cast_precision_loss)]
                let move_dir = Vec2::new(dx as f32, dy as f32);
                calculate_wind_movement_penalty(wind, move_dir)
            });

            // Apply wind penalty. Ensure cost is at least 1.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let adjusted_cost = (base_cost as f32 * wind_penalty).round() as i32;
            let tile_cost = adjusted_cost.max(1);

            let new_cost = cost + tile_cost;

            if new_cost < *cost_so_far.get(&next_idx).unwrap_or(&i32::MAX) {
                cost_so_far.insert(next_idx, new_cost);
                let priority = new_cost + manhattan_distance(next, end);
                open_set.push(Node {
                    pos: next,
                    cost: new_cost,
                    heuristic: priority,
                });
                // Since 1M tiles fits in u32 (up to 4B), this cast is safe given limits.
                #[allow(clippy::cast_possible_truncation)]
                {
                    came_from.insert(next_idx, current_idx as u32);
                }
            }
        }
    }

    None
}

/// Calculates the Manhattan distance (L1 norm) between two grid points.
///
/// `|x1 - x2| + |y1 - y2|`
///
/// This heuristic is admissible for 4-connected grids, guaranteeing the shortest path
/// if edge weights are >= 1.
#[must_use]
pub const fn manhattan_distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

#[allow(clippy::too_many_arguments, clippy::collapsible_if)]
fn is_walkable(
    pos: (i32, i32),
    world: &World,
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    building_map: &BuildingMap,
    can_use_vents: bool,
    credentials: Option<&AccessCredentials>,
    _is_target: bool,
) -> bool {
    let (x, y) = pos;

    // 1. Check Bounds & Terrain
    let Ok(ux) = usize::try_from(x) else {
        return false;
    };
    let Ok(uy) = usize::try_from(y) else {
        return false;
    };

    if !terrain.get(ux, uy).is_some_and(TerrainType::is_walkable) {
        return false;
    }

    // 2. Check Occupied Tiles
    if let Some(occ) = occupied {
        if !occ.0.contains(&(x, y)) {
            return true;
        }
    } else {
        return true;
    }

    // 3. Check Building Type
    if let Some(&entity) = building_map.0.get(&(x, y)) {
        return check_building_access(world, entity, can_use_vents, credentials);
    }

    true
}

fn check_building_access(
    world: &World,
    entity: Entity,
    can_use_vents: bool,
    credentials: Option<&AccessCredentials>,
) -> bool {
    let Some(building) = world.get::<Building>(entity) else {
        return true;
    };

    if let Some(creds) = credentials {
        if crate::layer1::security::check_security_clearance(world, creds.entity, entity)
            == crate::layer1::security::AccessResult::DeniedDrift
        {
            return false;
        }
    }

    let access_opt = world.get::<AccessControl>(entity);
    let door_opt = world.get::<DoorControl>(entity);

    // Check Door Control first (physical state overrides)
    if let Some(door) = door_opt {
        match door.state {
            DoorState::Locked => return false,
            DoorState::Open => return true,
            DoorState::Auto => { /* Continue to check AccessControl */ }
        }
    }

    if let Some(access) = access_opt {
        return match access.mode {
            AccessMode::Public => true,
            AccessMode::Lockdown => false,
            AccessMode::Restricted => credentials.is_some_and(|creds| {
                access.allowed_pops.contains(&creds.entity)
                    || creds
                        .role
                        .is_some_and(|r| access.allowed_roles.contains(&r))
            }),
        };
    }

    if building.building_type == BuildingType::Vent && can_use_vents {
        return true;
    }

    if building.building_type == BuildingType::ConveyorBelt {
        let Some(conveyor) = world.get::<crate::layer1::logistics::conveyor::ConveyorBelt>(entity)
        else {
            // Fallback if the component is missing but it's a conveyor building.
            // A standard conveyor blocks pathfinding by default.
            return false;
        };
        if conveyor.blocks_pathfinding() {
            return false;
        }
    } else if building.building_type.is_obstacle() {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
    use crate::layer1::map::GridPosition;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::vermin::Vermin;

    fn setup_world() -> World {
        let mut world = World::new();
        // 10x10 grass
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());
        world
    }

    fn update_map(world: &mut World) {
        let mut query = world.query::<(Entity, &GridPosition, &Building)>();
        let entries: Vec<_> = query
            .iter(world)
            .map(|(e, pos, _)| ((pos.x, pos.y), e))
            .collect();

        let mut map = world.resource_mut::<BuildingMap>();
        map.0.clear();
        for (pos, e) in entries {
            map.0.insert(pos, e);
        }
    }

    #[test]
    fn test_find_path_simple() {
        let world = setup_world();
        let path = find_path(&world, (0, 0), (2, 0));
        assert!(path.is_some(), "Should find simple straight path");
        let p = path.expect("Path should be found on an empty map");
        // Path from (0,0) to (2,0) should be [(1,0), (2,0)] or similar length
        assert!(p.len() >= 2);
        assert_eq!(p.last(), Some(&(2, 0)));
    }

    #[test]
    fn test_vent_blocks_pop_movement() {
        let mut world = setup_world();

        // Vent at (1, 0)
        world.spawn((
            Building {
                building_type: BuildingType::Vent,
            },
            GridPosition { x: 1, y: 0 },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 0));
        update_map(&mut world);

        // Pop at (0, 0) trying to move to (2, 0)
        // Check pathfinding
        let path = find_path(&world, (0, 0), (2, 0));
        // If it's blocked, it might find a long way around (if map is open)
        // But let's assume we blocked the only way or check if it avoids (1,0).
        // Since map is 10x10 grass, it will just walk around (0,1) -> (1,1) -> (2,1) -> (2,0).

        if let Some(p) = path {
            assert!(
                !p.contains(&(1, 0)),
                "Path should NOT contain Vent at (1,0)"
            );
        }

        // Let's make a corridor to force block
        // W W W
        // S V E
        // W W W
        // Start (0,1), Vent (1,1), End (2,1). Walls at y=0 and y=2.

        let mut world = setup_world();
        // Block rows 0 and 2
        for x in 0..3 {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 0 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 0));
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 2 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 2));
        }

        // Vent at (1, 1)
        world.spawn((
            Building {
                building_type: BuildingType::Vent,
            },
            GridPosition { x: 1, y: 1 },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));
        update_map(&mut world);

        let path = find_path(&world, (0, 1), (2, 1));
        assert!(
            path.is_none(),
            "Pop should not path through Vent when it is the only way"
        );
    }

    #[test]
    fn test_vermin_passes_through_vent() {
        let mut world = setup_world();

        // Block rows 0 and 2
        for x in 0..3 {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 0 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 0));
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 2 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 2));
        }

        // Vent at (1, 1)
        world.spawn((
            Building {
                building_type: BuildingType::Vent,
            },
            GridPosition { x: 1, y: 1 },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));
        update_map(&mut world);

        // Vermin at (0, 1)
        // Check pathfinding with Vermin capability
        let path = find_path_for_entity(&world, (0, 1), (2, 1), &Vermin);
        assert!(path.is_some(), "Vermin SHOULD path through Vent");
    }

    #[test]
    fn test_locked_door_blocks_path() {
        let mut world = setup_world();

        // Block rows 0 and 2
        for x in 0..3 {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 0 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 0));
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 2 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 2));
        }

        // Gate at (1, 1) - Public Access but Locked DoorControl
        world.spawn((
            Building {
                building_type: BuildingType::Gate,
            },
            GridPosition { x: 1, y: 1 },
            AccessControl {
                mode: AccessMode::Public,
                ..Default::default()
            },
            DoorControl {
                state: DoorState::Locked,
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));
        update_map(&mut world);

        // Try to path from (0, 1) to (2, 1)
        let path = find_path(&world, (0, 1), (2, 1));
        assert!(
            path.is_none(),
            "Locked door should block path even if Public"
        );
    }

    #[test]
    fn test_open_door_allows_path_restricted() {
        let mut world = setup_world();

        // Block rows 0 and 2
        for x in 0..3 {
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 0 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 0));
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x, y: 2 },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((x, 2));
        }

        // Gate at (1, 1) - Lockdown Access but Open DoorControl
        world.spawn((
            Building {
                building_type: BuildingType::Gate,
            },
            GridPosition { x: 1, y: 1 },
            AccessControl {
                mode: AccessMode::Lockdown,
                ..Default::default()
            },
            DoorControl {
                state: DoorState::Open,
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));
        update_map(&mut world);

        let pop = world.spawn(crate::layer1::pop::Pop).id();

        // Try to path from (0, 1) to (2, 1)
        let path = crate::layer1::pathfinding::find_path_for_pop(&world, (0, 1), (2, 1), pop);
        assert!(
            path.is_some(),
            "Open door should allow path even if Lockdown"
        );
    }

    #[test]
    fn test_pathfinding_avoids_strong_headwind() {
        let mut world = setup_world();
        let width = 10;
        let height = 10;
        let mut wind_grid = crate::layer1::wind::WindGrid::new(width, height);

        // Scenario:
        // Start (0, 0), End (5, 0)
        // Direct path (row 0) has strong Headwind (West wind, pushing East->West).
        // Sheltered path (row 1) has no wind.
        // Direct Path: (0,0)->(1,0)->(2,0)->(3,0)->(4,0)->(5,0). Distance 5.
        // Movement is East (+1, 0). Wind is West (-10, 0). Dot = -1.0. Cost = High.

        let headwind = crate::layer1::wind::Vec2::new(-10.0, 0.0);
        for x in 0..width {
            wind_grid.set_wind(x as i32, 0, headwind);
        }

        // Shelter row 1 (no wind)
        for x in 0..width {
            wind_grid.set_wind(x as i32, 1, crate::layer1::wind::Vec2::ZERO);
        }

        world.insert_resource(wind_grid);

        let path = find_path(&world, (0, 0), (5, 0));
        assert!(path.is_some());
        let p = path.expect("Path should exist and avoid headwind");

        // Path should use row 1 (y=1) to avoid headwind
        let uses_shelter = p.iter().any(|pos| pos.1 == 1);
        assert!(
            uses_shelter,
            "Path should use sheltered row y=1. Actual path: {:?}",
            p
        );
    }

    #[test]
    fn test_pathfinding_prefers_tailwind() {
        let mut world = setup_world();
        let width = 10;
        let height = 10;
        let mut wind_grid = crate::layer1::wind::WindGrid::new(width, height);
        let mut crowding = crate::layer1::crowding::CrowdingGrid::new(width, height);

        // Scenario:
        // Increase base cost to 2 so Tailwind (0.6x) can reduce it below base.
        // Add crowding 1 everywhere.
        for y in 0..height {
            for x in 0..width {
                crowding.add_crowding(x, y, 1);
            }
        }
        world.insert_resource(crowding);

        // Direct path (row 0) has no wind (Cost 2 per tile). Total = 10 (5 steps).
        // Row 1 has strong Tailwind (East wind).
        // Movement East (+1, 0). Wind East (10, 0). Dot = 1.0. Cost = 2 * 0.6 = 1.2 -> 1.
        // Path via Row 1 involves:
        // (0,0)->(0,1) (1 step, cost 2)
        // (0,1)->...->(5,1) (5 steps, cost 1 * 5 = 5)
        // (5,1)->(5,0) (1 step, cost 2)
        // Total sheltered cost: 2 + 5 + 2 = 9.
        // 9 < 10.
        // It should prefer the longer path (7 steps vs 5 steps).

        let tailwind = crate::layer1::wind::Vec2::new(10.0, 0.0);
        for x in 0..width {
            wind_grid.set_wind(x as i32, 1, tailwind);
        }

        world.insert_resource(wind_grid);

        let path = find_path(&world, (0, 0), (5, 0));
        assert!(path.is_some());
        let p = path.expect("Path should exist and use tailwind");

        // Path should use row 1 (y=1) for tailwind boost
        let uses_tailwind = p.iter().any(|pos| pos.1 == 1);
        assert!(
            uses_tailwind,
            "Path should use tailwind row y=1. Actual path: {:?}",
            p
        );
    }

    #[test]
    fn test_role_based_access_control() {
        use crate::layer1::pop::{Pop, Role};
        use std::collections::HashSet;

        let mut world = setup_world();

        // Block entire column x=1 with Walls, except at y=1 (Gate).
        // This forces path through (1, 1).
        for y in 0..10 {
            if y == 1 {
                continue;
            }
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x: 1, y },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((1, y));
        }

        // Gate at (1, 1)
        let mut allowed_roles = HashSet::new();
        allowed_roles.insert(Role::Engineer);
        world.spawn((
            Building {
                building_type: BuildingType::Gate,
            },
            GridPosition { x: 1, y: 1 },
            AccessControl {
                mode: AccessMode::Restricted,
                allowed_roles,
                ..Default::default()
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));

        update_map(&mut world);

        let civilian = world.spawn((Pop, Role::Civilian)).id();
        let engineer = world.spawn((Pop, Role::Engineer)).id();

        // Civilian: Should be blocked
        let path_civ = find_path_for_pop(&world, (0, 1), (2, 1), civilian);
        assert!(
            path_civ.is_none(),
            "Civilian should be blocked by Restricted Gate"
        );

        // Engineer: Should pass
        let path_eng = find_path_for_pop(&world, (0, 1), (2, 1), engineer);
        assert!(
            path_eng.is_some(),
            "Engineer should pass through Restricted Gate"
        );
    }

    #[test]
    fn test_open_door_bypasses_access_control() {
        use crate::layer1::control::{DoorControl, DoorState};

        use crate::layer1::pop::{Pop, Role};
        use std::collections::HashSet;

        let mut world = setup_world();

        // Block column x=1 except y=1
        for y in 0..10 {
            if y == 1 {
                continue;
            }
            world.spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                GridPosition { x: 1, y },
            ));
            world.resource_mut::<OccupiedTiles>().0.insert((1, y));
        }

        // Restricted Gate at (1, 1), but PHYSICALLY OPEN
        let mut allowed_roles = HashSet::new();
        allowed_roles.insert(Role::Engineer);

        world.spawn((
            Building {
                building_type: BuildingType::Gate,
            },
            GridPosition { x: 1, y: 1 },
            AccessControl {
                mode: AccessMode::Restricted,
                allowed_roles,
                ..Default::default()
            },
            DoorControl {
                state: DoorState::Open,
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));

        update_map(&mut world);

        let civilian = world.spawn((Pop, Role::Civilian)).id();

        // Civilian should pass because the door is stuck open!
        let path = find_path_for_pop(&world, (0, 1), (2, 1), civilian);
        assert!(
            path.is_some(),
            "Civilian SHOULD pass if Restricted door is physically Open"
        );
    }
    #[test]
    fn test_pathfinding_integration() {
        use crate::layer1::building::{Building, BuildingMap, BuildingType, OccupiedTiles};
        use crate::layer1::map::GridPosition;
        use crate::layer1::pathfinding::find_path_for_pop;
        use crate::layer1::terrain::{TerrainGrid, TerrainType};

        let mut world = World::new();
        // Setup Map
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());

        let pop = world.spawn(crate::layer1::pop::Pop).id();

        // Create a choke point at (1, 0)
        // Block (1, 1) with a Wall
        world.spawn((
            GridPosition { x: 1, y: 1 },
            Building {
                building_type: BuildingType::Wall,
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));

        // Since it's a 10x10 grid, we also need to block going around the other way or assume start/end are constrained.
        // Actually, let's just surround the start point (0,0).
        // Block (0, 1) and (1, 1).
        // And (1, 0) is the Gate.

        // Wall at (0, 1)
        world.spawn((
            GridPosition { x: 0, y: 1 },
            Building {
                building_type: BuildingType::Wall,
            },
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((0, 1));

        // Locked Door at (1, 0)
        world.spawn((
            GridPosition { x: 1, y: 0 },
            AccessControl {
                mode: AccessMode::Lockdown,
                ..Default::default()
            },
            Building {
                building_type: BuildingType::Gate,
            }, // Gate is an obstacle
        ));
        world.resource_mut::<OccupiedTiles>().0.insert((1, 0));

        // Update BuildingMap
        {
            let mut query = world.query::<(Entity, &GridPosition, &Building)>();
            let entries: Vec<_> = query
                .iter(&world)
                .map(|(e, pos, _)| ((pos.x, pos.y), e))
                .collect();
            let mut map = world.resource_mut::<BuildingMap>();
            map.0.clear();
            for (pos, e) in entries {
                map.0.insert(pos, e);
            }
        }

        let path = find_path_for_pop(&world, (0, 0), (2, 0), pop);
        assert!(path.is_none(), "Path should be blocked by lockdown");
    }
}
