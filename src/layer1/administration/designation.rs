//! User intent and map designations.
//!
//! Designations are the primary way the player interacts with the simulation (The Mandate).
//! Instead of directly manipulating entities, the player "designates" a tile for an action
//! (e.g., "Mine here"), and the Pop AI (Souls) fulfills that request asynchronously.
//!
//! # Key Concepts
//!
//! * **Designation**: A persistent request attached to a specific map coordinate.
//! * **DesignationType**: The kind of request (Mine, Demolish).
//! * **Validation**: Rules for where designations can be placed (`can_designate`).

use crate::layer1::environment::orbital_crossfire::ImpactSite;
use crate::layer1::particles::spawn_particle;
use crate::layer1::zone::ZoneType;
use crate::layer1::{GridPosition, OccupiedTiles, TerrainGrid, TerrainType};
use bevy_ecs::prelude::*;
use ratatui::style::Color;
use std::collections::HashSet;

/// Types of designations a player can apply to the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DesignationType {
    /// Designate a rock tile for mining.
    #[default]
    Mine,
    /// Designate a building for demolition.
    Demolish,
    /// Designate a tree for chopping.
    Chop,
    /// Designate a building for repair.
    Repair,
    /// Set a zone type for a tile.
    SetZone(ZoneType),
    /// Designate an animal for taming.
    Tame,
    /// Designate flora for clearing.
    ClearFlora,
    /// Designate a building for jury-rigging (quick, fragile repair).
    JuryRig,
    /// Designate the Lander for cannibalization (destructive resource extraction).
    Cannibalize,
    /// Designate a Vacuum Welded building for destruction (yields 0 resources).
    Destroy,
    /// Designate an edible building for consumption.
    Consume,
    /// Collect genetic sample from flora or fauna.
    CollectSample,
    /// Excavate accumulated Desire Dust from a tile (Spec 1379 counterplay).
    ExcavateDust,
}

impl DesignationType {
    /// Returns the character representation of the designation.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::designation::DesignationType;
    ///
    /// assert_eq!(DesignationType::Mine.char(), '%');
    /// ```
    #[must_use]
    pub const fn char(&self) -> char {
        match self {
            Self::Mine => '%',
            Self::Demolish => 'X',
            Self::Chop => '/',
            Self::Repair => '+',
            Self::SetZone(_) => 'Z',
            Self::Tame => '♥',
            Self::ClearFlora => 'F',
            Self::JuryRig => 'J',
            Self::Cannibalize => 'C',
            Self::Destroy => 'D',
            Self::Consume => 'E',
            Self::CollectSample => 'S',
            Self::ExcavateDust => 'U',
        }
    }

    /// Returns a string slice representation of the designation.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::designation::DesignationType;
    ///
    /// assert_eq!(DesignationType::Mine.as_str(), "%");
    /// ```
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Mine => "%",
            Self::Demolish => "X",
            Self::Chop => "/",
            Self::Repair => "+",
            Self::SetZone(_) => "Z",
            Self::Tame => "♥",
            Self::ClearFlora => "F",
            Self::JuryRig => "J",
            Self::Cannibalize => "C",
            Self::Destroy => "D",
            Self::Consume => "E",
            Self::CollectSample => "S",
            Self::ExcavateDust => "U",
        }
    }

    /// Returns the human-readable label of the designation.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::designation::DesignationType;
    ///
    /// assert_eq!(DesignationType::Mine.label(), "Mine");
    /// ```
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Mine => "Mine",
            Self::Demolish => "Demolish",
            Self::Chop => "Chop",
            Self::Repair => "Repair",
            Self::SetZone(_) => "Set Zone",
            Self::Tame => "Tame",
            Self::ClearFlora => "Clear Flora",
            Self::JuryRig => "Jury-Rig",
            Self::Cannibalize => "Cannibalize",
            Self::Destroy => "Destroy",
            Self::Consume => "Consume",
            Self::CollectSample => "Collect Sample",
            Self::ExcavateDust => "Excavate Dust",
        }
    }
}

/// Component attached to an entity representing a designation.
#[derive(Component)]
pub struct Designation {
    /// The type of designation.
    pub designation_type: DesignationType,
}

/// Resource tracking the player's current designation mode state.
///
/// This acts as the "Tool Controller" for the UI.
#[derive(Resource, Default)]
pub struct DesignationMode {
    /// Whether designation mode is active.
    pub active: bool,
    /// The currently selected designation tool.
    pub tool: DesignationType,
    /// The cursor position for designation.
    pub cursor: GridPosition,
    /// The starting corner of a drag rectangle (set on first press, cleared on second).
    pub drag_start: Option<GridPosition>,
}

/// Checks if a designation can be placed at the given coordinates.
///
/// This enforces game rules, such as "You can only mine Rock" or "You can only demolish buildings".
///
/// # Examples
///
/// ```
/// use scale::layer1::designation::{can_designate, DesignationType};
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::OccupiedTiles;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let mut tiles = vec![TerrainType::Grass; 100];
/// tiles[0] = TerrainType::Rock; // (0,0) is Rock
/// world.insert_resource(TerrainGrid { width: 10, height: 10, tiles });
/// world.insert_resource(OccupiedTiles::default());
///
/// // Can mine Rock
/// assert!(can_designate(&world, 0, 0, DesignationType::Mine));
/// // Cannot mine Grass (implied at 1,1)
/// assert!(!can_designate(&world, 1, 1, DesignationType::Mine));
/// ```
#[must_use]
fn has_component_at<T: Component>(world: &World, x: i32, y: i32) -> bool {
    world.iter_entities().any(|e| {
        let Some(pos) = e.get::<GridPosition>() else {
            return false;
        };
        pos.x == x && pos.y == y && e.contains::<T>()
    })
}

fn has_designation_at(world: &World, x: i32, y: i32) -> bool {
    world.iter_entities().any(|e| {
        let Some(pos) = e.get::<GridPosition>() else {
            return false;
        };
        pos.x == x && pos.y == y && e.contains::<Designation>()
    })
}

pub fn can_designate(world: &World, x: i32, y: i32, designation_type: DesignationType) -> bool {
    // Check bounds (basic check, more detailed check in terrain/occupied logic)
    if x < 0 || y < 0 {
        return false;
    }

    // Verify coordinates are within map bounds
    if let Some(terrain) = world.get_resource::<TerrainGrid>() {
        // Safe cast: we already checked negative values
        if (x as usize) >= terrain.width || (y as usize) >= terrain.height {
            return false;
        }
    }

    // Check for existing designation at this location
    // Note: This linear query might be slow for many designations,
    // but acceptable for the MVP scope (designations < 1000).
    // Future optimization: Spatial index for designations.
    let existing = has_designation_at(world, x, y);

    if existing {
        return false;
    }

    match designation_type {
        DesignationType::Mine => {
            let terrain = world.resource::<TerrainGrid>();
            // Allow casting because we checked for negative above
            if terrain.get(x as usize, y as usize) == Some(TerrainType::Rock) {
                return true;
            }
            has_component_at::<ImpactSite>(world, x, y)
        }
        DesignationType::Demolish => {
            let occupied = world.resource::<OccupiedTiles>();
            if !occupied.0.contains(&(x, y)) {
                return false;
            }
            !has_component_at::<crate::layer1::building::VacuumWelded>(world, x, y)
        }
        DesignationType::Chop => {
            let terrain = world.resource::<TerrainGrid>();
            // Allow casting because we checked for negative above
            terrain.get(x as usize, y as usize) == Some(TerrainType::Tree)
        }
        DesignationType::Repair => {
            let occupied = world.resource::<OccupiedTiles>();
            if !occupied.0.contains(&(x, y)) {
                return false;
            }
            !has_component_at::<crate::layer1::building::VacuumWelded>(world, x, y)
        }
        DesignationType::SetZone(_) => true,
        DesignationType::Tame => {
            // Must target a wild animal (Fauna without Tame component)
            world.iter_entities().any(|e| {
                let Some(pos) = e.get::<GridPosition>() else {
                    return false;
                };
                pos.x == x
                    && pos.y == y
                    && e.contains::<crate::layer1::fauna::Fauna>()
                    && !e.contains::<crate::layer1::husbandry::Tame>()
            })
        }
        DesignationType::ClearFlora => has_component_at::<crate::layer1::flora::Flora>(world, x, y),
        DesignationType::JuryRig => {
            let occupied = world.resource::<OccupiedTiles>();
            // Only occupied tiles can be jury-rigged (assumes building)
            occupied.0.contains(&(x, y))
        }
        DesignationType::Cannibalize => {
            // Must target the Lander
            world.iter_entities().any(|e| {
                let Some(pos) = e.get::<GridPosition>() else {
                    return false;
                };
                pos.x == x
                    && pos.y == y
                    && e.get::<crate::layer1::building::Building>()
                        .is_some_and(|b| {
                            b.building_type == crate::layer1::building::BuildingType::Lander
                        })
            })
        }
        DesignationType::Consume => {
            let occupied = world.resource::<OccupiedTiles>();
            occupied.0.contains(&(x, y))
                && has_component_at::<crate::layer1::architecture::edible::EdibleMaterial>(
                    world, x, y,
                )
        }
        DesignationType::Destroy => {
            let occupied = world.resource::<OccupiedTiles>();
            // Only occupied tiles can be destroyed
            occupied.0.contains(&(x, y))
        }
        DesignationType::CollectSample => {
            has_component_at::<crate::layer1::flora::Flora>(world, x, y)
                || has_component_at::<crate::layer1::fauna::Fauna>(world, x, y)
        }
        DesignationType::ExcavateDust => world
            .get_resource::<crate::layer1::desire_dust::DesireDust>()
            .is_some_and(|dust| dust.amount_at(x, y) > 0.0),
    }
}

/// Attempts to apply a designation at the given coordinates.
///
/// Wrapper around `can_designate` that spawns the entity if valid.
///
/// # Returns
///
/// * `true` if the designation was successfully placed.
/// * `false` if the placement was invalid or a designation already exists.
///
/// # Examples
///
/// ```
/// use scale::layer1::designation::{try_designate, DesignationType};
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::OccupiedTiles;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// // Setup valid condition (Rock)
/// let mut tiles = vec![TerrainType::Grass; 100];
/// tiles[55] = TerrainType::Rock; // (5,5)
/// world.insert_resource(TerrainGrid { width: 10, height: 10, tiles });
/// world.insert_resource(OccupiedTiles::default());
///
/// assert!(try_designate(&mut world, 5, 5, DesignationType::Mine));
/// assert!(!try_designate(&mut world, 5, 5, DesignationType::Mine)); // Duplicate
/// ```
pub fn try_designate(world: &mut World, x: i32, y: i32, designation_type: DesignationType) -> bool {
    if !can_designate(world, x, y, designation_type) {
        return false;
    }

    world.spawn((Designation { designation_type }, GridPosition { x, y }));

    // Ludwig: Spawn Confirm Particle
    spawn_particle(world, GridPosition { x, y }, '+', Color::Green, 10);

    true
}

/// Designate all eligible tiles in a rectangle. Returns count of successful designations.
///
/// The rectangle is defined by two corners `(x1, y1)` and `(x2, y2)`. Corners can be
/// given in any order; the function normalizes to min/max internally.
///
/// # Examples
///
/// ```
/// use scale::layer1::designation::{try_designate_area, DesignationType};
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::OccupiedTiles;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let mut tiles = vec![TerrainType::Grass; 100];
/// tiles[55] = TerrainType::Rock; // (5,5)
/// tiles[56] = TerrainType::Rock; // (6,5)
/// world.insert_resource(TerrainGrid { width: 10, height: 10, tiles });
/// world.insert_resource(OccupiedTiles::default());
///
/// assert_eq!(try_designate_area(&mut world, 5, 5, 6, 5, DesignationType::Mine), 2);
/// ```
fn clamp_area_dimensions(x1: i32, y1: i32, x2: i32, y2: i32) -> (i32, i32, i32, i32) {
    const MAX_DIMENSION: i32 = 50;

    let min_x_raw = x1.min(x2);
    let max_x_raw = x1.max(x2);
    let min_y_raw = y1.min(y2);
    let max_y_raw = y1.max(y2);

    let width = (max_x_raw - min_x_raw).min(MAX_DIMENSION);
    let height = (max_y_raw - min_y_raw).min(MAX_DIMENSION);

    let min_x = min_x_raw;
    let max_x = min_x_raw + width;
    let min_y = min_y_raw;
    let max_y = min_y_raw + height;

    (min_x, max_x, min_y, max_y)
}

fn get_valid_targets_for_tool(
    world: &mut World,
    tool: DesignationType,
) -> Option<HashSet<(i32, i32)>> {
    match tool {
        DesignationType::Tame => Some(
            world
                .query::<(
                    Entity,
                    &GridPosition,
                    &crate::layer1::fauna::Fauna,
                    Option<&crate::layer1::husbandry::Tame>,
                )>()
                .iter(world)
                .filter_map(|(_, pos, _, tame)| {
                    if tame.is_none() {
                        Some((pos.x, pos.y))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
        DesignationType::ClearFlora => Some(
            world
                .query::<(&GridPosition, &crate::layer1::flora::Flora)>()
                .iter(world)
                .map(|(pos, _)| (pos.x, pos.y))
                .collect(),
        ),
        DesignationType::Cannibalize => Some(
            world
                .query::<(&GridPosition, &crate::layer1::building::Building)>()
                .iter(world)
                .filter_map(|(pos, b)| {
                    if b.building_type == crate::layer1::building::BuildingType::Lander {
                        Some((pos.x, pos.y))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
        DesignationType::CollectSample => {
            let mut targets = HashSet::new();
            for (pos, _) in world
                .query::<(&GridPosition, &crate::layer1::flora::Flora)>()
                .iter(world)
            {
                targets.insert((pos.x, pos.y));
            }
            for (pos, _) in world
                .query::<(&GridPosition, &crate::layer1::fauna::Fauna)>()
                .iter(world)
            {
                targets.insert((pos.x, pos.y));
            }
            Some(targets)
        }
        _ => None,
    }
}

/// Designate all eligible tiles in a rectangle. Returns count of successful designations.
///
/// The rectangle is defined by two corners `(x1, y1)` and `(x2, y2)`. Corners can be
/// given in any order; the function normalizes to min/max internally.
///
/// # Examples
///
/// ```
/// use scale::layer1::designation::{try_designate_area, DesignationType};
/// use scale::layer1::terrain::{TerrainGrid, TerrainType};
/// use scale::layer1::building::OccupiedTiles;
/// use bevy_ecs::prelude::*;
///
/// let mut world = World::new();
/// let mut tiles = vec![TerrainType::Grass; 100];
/// tiles[55] = TerrainType::Rock; // (5,5)
/// tiles[56] = TerrainType::Rock; // (6,5)
/// world.insert_resource(TerrainGrid { width: 10, height: 10, tiles });
/// world.insert_resource(OccupiedTiles::default());
///
/// assert_eq!(try_designate_area(&mut world, 5, 5, 6, 5, DesignationType::Mine), 2);
/// ```
pub fn try_designate_area(
    world: &mut World,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    tool: DesignationType,
) -> u32 {
    let (min_x, max_x, min_y, max_y) = clamp_area_dimensions(x1, y1, x2, y2);

    let existing_designations: HashSet<(i32, i32)> = world
        .query::<(&Designation, &GridPosition)>()
        .iter(world)
        .map(|(_, pos)| (pos.x, pos.y))
        .collect();

    let valid_targets = get_valid_targets_for_tool(world, tool);
    let mut to_spawn = Vec::new();

    {
        let terrain_grid = world.get_resource::<TerrainGrid>();
        let occupied_tiles = world.get_resource::<OccupiedTiles>();

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if x < 0 || y < 0 {
                    continue;
                }

                #[allow(clippy::cast_sign_loss)]
                if terrain_grid
                    .is_some_and(|grid| (x as usize) >= grid.width || (y as usize) >= grid.height)
                {
                    continue;
                }

                if existing_designations.contains(&(x, y)) {
                    continue;
                }

                #[allow(clippy::cast_sign_loss)]
                let is_valid = match tool {
                    DesignationType::Mine => terrain_grid
                        .is_some_and(|g| g.get(x as usize, y as usize) == Some(TerrainType::Rock)),
                    DesignationType::Demolish => {
                        occupied_tiles.is_some_and(|o| o.0.contains(&(x, y)))
                    }
                    DesignationType::Chop => terrain_grid
                        .is_some_and(|g| g.get(x as usize, y as usize) == Some(TerrainType::Tree)),
                    DesignationType::Repair | DesignationType::JuryRig => {
                        occupied_tiles.is_some_and(|o| o.0.contains(&(x, y)))
                    }
                    DesignationType::SetZone(_) => true,
                    DesignationType::Tame
                    | DesignationType::ClearFlora
                    | DesignationType::Cannibalize
                    | DesignationType::CollectSample => valid_targets
                        .as_ref()
                        .is_some_and(|targets| targets.contains(&(x, y))),
                    DesignationType::Consume => {
                        occupied_tiles.is_some_and(|o| o.0.contains(&(x, y)))
                            && has_component_at::<crate::layer1::architecture::edible::EdibleMaterial>(
                                world, x, y,
                            )
                    }
                    DesignationType::Destroy => {
                        occupied_tiles.is_some_and(|o| o.0.contains(&(x, y)))
                    }
                    // Single-tile tool: area designation does not apply.
                    DesignationType::ExcavateDust => false,
                };

                if is_valid {
                    to_spawn.push((x, y));
                }
            }
        }
    }

    let mut count = 0;
    for (x, y) in to_spawn {
        world.spawn((
            Designation {
                designation_type: tool,
            },
            GridPosition { x, y },
        ));
        count += 1;
    }

    count
}

/// Attempts to remove any designation at the given coordinates.
///
/// Used when the user right-clicks or cancels a designation.
///
/// # Returns
///
/// * `true` if a designation was found and removed.
/// * `false` if no designation existed at that location.
pub fn try_cancel_designation(world: &mut World, x: i32, y: i32) -> bool {
    let mut to_despawn = None;

    // Find designation at position
    for (entity, _, pos) in world
        .query::<(Entity, &Designation, &GridPosition)>()
        .iter(world)
    {
        if pos.x == x && pos.y == y {
            to_despawn = Some(entity);
            break;
        }
    }

    to_despawn.is_some_and(|entity| {
        world.despawn(entity);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::OccupiedTiles;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};

    #[test]
    fn test_designation_type_variants() {
        let mine = DesignationType::Mine;
        let demolish = DesignationType::Demolish;
        assert_ne!(mine, demolish);
    }

    #[test]
    fn test_designation_type_char() {
        // Visualization is important for text UI, so we test the mapping exists
        assert_eq!(DesignationType::Mine.char(), '%');
        assert_eq!(DesignationType::Demolish.char(), 'X');
    }

    #[test]
    fn test_designation_type_as_str() {
        assert_eq!(DesignationType::Mine.as_str(), "%");
        assert_eq!(DesignationType::Demolish.as_str(), "X");
    }

    #[test]
    fn test_designation_component() {
        let designation = Designation {
            designation_type: DesignationType::Mine,
        };
        assert_eq!(designation.designation_type, DesignationType::Mine);
    }

    #[test]
    fn test_designation_mode_default() {
        let mode = DesignationMode::default();
        assert!(!mode.active);
        assert_eq!(mode.tool, DesignationType::Mine);
        assert_eq!(mode.cursor.x, 0);
        assert_eq!(mode.cursor.y, 0);
        assert!(mode.drag_start.is_none());
    }

    #[test]
    fn test_designation_mode_toggle() {
        let mut mode = DesignationMode {
            active: true,
            ..Default::default()
        };
        assert!(mode.active);
        mode.active = false;
        assert!(!mode.active);
    }

    #[test]
    fn test_can_designate_mine_valid() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock; // Position (5, 5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        // Can mine Rock
        assert!(can_designate(&world, 5, 5, DesignationType::Mine));
    }

    #[test]
    fn test_can_designate_mine_invalid() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        // Cannot mine Grass
        assert!(!can_designate(&world, 5, 5, DesignationType::Mine));
    }

    #[test]
    fn test_can_designate_demolish_valid() {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        let mut occupied = OccupiedTiles::default();
        occupied.0.insert((5, 5));
        world.insert_resource(occupied);

        // Can demolish occupied tile
        assert!(can_designate(&world, 5, 5, DesignationType::Demolish));
    }

    #[test]
    fn test_can_designate_demolish_invalid() {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.insert_resource(OccupiedTiles::default());

        // Cannot demolish empty tile
        assert!(!can_designate(&world, 5, 5, DesignationType::Demolish));
    }

    #[test]
    fn test_try_designate_creates_entity() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock;
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        let success = try_designate(&mut world, 5, 5, DesignationType::Mine);
        assert!(success);

        let count = world
            .query::<(&Designation, &GridPosition)>()
            .iter(&world)
            .count();
        assert_eq!(count, 1);

        let (designation, pos) = world
            .query::<(&Designation, &GridPosition)>()
            .single(&world);
        assert_eq!(designation.designation_type, DesignationType::Mine);
        assert_eq!(pos.x, 5);
        assert_eq!(pos.y, 5);
    }

    #[test]
    fn test_try_designate_duplicates_ignored() {
        let mut world = World::new();
        // Setup valid rock
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock;
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        // First designation
        try_designate(&mut world, 5, 5, DesignationType::Mine);

        // Second designation (same type/pos)
        let success = try_designate(&mut world, 5, 5, DesignationType::Mine);

        // Should return true (idempotent) or false?
        // Let's say false because "nothing happened"
        assert!(!success);

        // Count should still be 1
        let count = world.query::<&Designation>().iter(&world).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_cancel_designation() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock;
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        try_designate(&mut world, 5, 5, DesignationType::Mine);
        // Expect 2 entities: Designation + Particle
        assert_eq!(world.entities().len(), 2);

        let removed = try_cancel_designation(&mut world, 5, 5);
        assert!(removed);

        // Designation entity should be despawned. Particle remains (it has lifetime).
        assert_eq!(world.entities().len(), 1);
    }

    #[test]
    fn test_designation_type_chop() {
        // Test new variant properties
        assert_eq!(DesignationType::Chop.char(), '/');
        assert_eq!(DesignationType::Chop.label(), "Chop");
    }

    #[test]
    fn test_can_designate_chop_valid() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Tree; // (5, 5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Should be able to chop a Tree
        assert!(can_designate(&world, 5, 5, DesignationType::Chop));
    }

    #[test]
    fn test_can_designate_chop_invalid() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Cannot chop Grass
        assert!(!can_designate(&world, 5, 5, DesignationType::Chop));
    }

    #[test]
    fn test_try_designate_area_single_tile() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock; // (5,5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        let count = try_designate_area(&mut world, 5, 5, 5, 5, DesignationType::Mine);
        assert_eq!(count, 1);

        let designation_count = world.query::<&Designation>().iter(&world).count();
        assert_eq!(designation_count, 1);
    }

    #[test]
    fn test_try_designate_area_multi_tile_rectangle() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        // Fill a 3x2 rectangle with Rock at (2,3), (3,3), (4,3), (2,4), (3,4), (4,4)
        tiles[32] = TerrainType::Rock; // (2,3)
        tiles[33] = TerrainType::Rock; // (3,3)
        tiles[34] = TerrainType::Rock; // (4,3)
        tiles[42] = TerrainType::Rock; // (2,4)
        tiles[43] = TerrainType::Rock; // (3,4)
        tiles[44] = TerrainType::Rock; // (4,4)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        // Drag from (4,4) to (2,3) — reversed corners
        let count = try_designate_area(&mut world, 4, 4, 2, 3, DesignationType::Mine);
        assert_eq!(count, 6);
    }

    #[test]
    fn test_try_designate_area_mixed_valid_invalid() {
        let mut world = World::new();
        let mut tiles = vec![TerrainType::Grass; 100];
        // Only (5,5) and (6,5) are Rock in a 3-tile row
        tiles[55] = TerrainType::Rock; // (5,5)
        tiles[56] = TerrainType::Rock; // (6,5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        // Area covers (5,5) to (7,5) — only 2 of 3 are valid
        let count = try_designate_area(&mut world, 5, 5, 7, 5, DesignationType::Mine);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_try_designate_area_no_valid_tiles() {
        let mut world = World::new();
        let tiles = vec![TerrainType::Grass; 100]; // All grass
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(OccupiedTiles::default());

        let count = try_designate_area(&mut world, 0, 0, 2, 2, DesignationType::Mine);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_set_zone_always_valid() {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Can set zone anywhere
        assert!(can_designate(
            &world,
            0,
            0,
            DesignationType::SetZone(ZoneType::Bedroom)
        ));
    }
}

#[test]
fn test_can_designate_clear_flora() {
    let mut world = World::new();
    world.insert_resource(TerrainGrid {
        width: 10,
        height: 10,
        tiles: vec![TerrainType::Grass; 100],
    });

    // Spawn Flora at (5, 5)
    world.spawn((
        crate::layer1::flora::Flora::default(),
        GridPosition { x: 5, y: 5 },
    ));

    // Can designate ClearFlora on Flora
    assert!(can_designate(&world, 5, 5, DesignationType::ClearFlora));

    // Cannot designate ClearFlora on empty tile
    assert!(!can_designate(&world, 5, 6, DesignationType::ClearFlora));
}

#[test]
fn test_try_designate_area_limit() {
    let mut world = World::new();
    world.insert_resource(TerrainGrid {
        width: 100,
        height: 100,
        tiles: vec![TerrainType::Rock; 10000],
    });
    world.insert_resource(OccupiedTiles::default());

    // Try to designate 60x60 (3600 tiles). Should be clamped.
    // MAX_DIMENSION = 50.
    // Width clamp = 50. Range 0..=(0+50) = 0..=50 (51 items).
    // Height clamp = 50. Range 0..=(0+50) = 0..=50 (51 items).
    // Total = 51 * 51 = 2601.

    let count = try_designate_area(&mut world, 0, 0, 59, 59, DesignationType::Mine);
    assert_eq!(count, 2601);
}

#[test]
fn test_try_designate_area_performance_smoke_test() {
    // This isn't a true benchmark, but ensures the optimized path runs correctly
    // without crashing or infinite looping on a larger area.
    let mut world = World::new();
    world.insert_resource(TerrainGrid {
        width: 100,
        height: 100,
        tiles: vec![TerrainType::Rock; 10000],
    });
    world.insert_resource(OccupiedTiles::default());

    // Spawn some existing designations to test the HashSet lookup
    for x in 0..10 {
        try_designate(&mut world, x, 0, DesignationType::Mine);
    }

    // Designate a 50x50 area (0..49) -> diff 49. width 49. loop 0..=49. 50 items.
    // 50*50 = 2500 items.
    let count = try_designate_area(&mut world, 0, 0, 49, 49, DesignationType::Mine);

    // Total area 2500. 10 already existed. So 2490 should be new.
    assert_eq!(count, 2490);
}
