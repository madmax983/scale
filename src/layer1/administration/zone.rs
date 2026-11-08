#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::map::GridPosition;
use bevy_ecs::prelude::*;

/// Types of zones that can be designated on the map.
///
/// Zones provide efficiency bonuses to buildings within them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZoneType {
    /// No zone designated.
    #[default]
    None,
    /// Bedroom zone (bonus to sleeping recovery in Housing).
    Bedroom,
    /// Dining zone (bonus to leisure recovery in Taverns).
    Dining,
    /// Hospital zone (bonus to healing in Hospitals).
    Hospital,
    /// Office zone (placeholder).
    Office,
    /// Storage zone (placeholder).
    Storage,
    /// Jail zone (for housing Inmates).
    Jail,
    /// Pasture for tamed animals.
    Pasture,
    /// Penal zone for forced labor.
    Penal,
    /// Sanctuary district where laws are ignored.
    Sanctuary,
    /// Embassy district where foreign laws apply.
    Extraterritorial,
    /// Customs checkpoint for vetting new arrivals.
    Customs,
    /// Redlined zone where pops become Stateless.
    Dezoned,
    /// Quarantine zone sealed by Spec 406 protocols: pathfinding
    /// edges across its boundary are severed and doors inside lock.
    Quarantine,
}

/// Resource storing the grid of zones.
#[derive(Resource, Clone)]
pub struct ZoneGrid {
    /// Flattened grid data.
    pub grid: Vec<ZoneType>,
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
}

impl ZoneGrid {
    /// Creates a new `ZoneGrid` with the given dimensions.
    ///
    /// # Panics
    /// Panics if `width * height` overflows or exceeds 10,000,000.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 10_000_000, "Grid size overflow or too large");

        Self {
            grid: vec![ZoneType::None; size],
            width,
            height,
        }
    }

    /// Gets the zone type at the given coordinates.
    #[must_use]
    #[allow(clippy::collapsible_if)]
    pub fn get(&self, x: i32, y: i32) -> ZoneType {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return ZoneType::None;
        }
        let ux = x as usize;
        let uy = y as usize;
        if let Some(idx) = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux)) {
            if idx < self.grid.len() {
                return self.grid[idx];
            }
        }
        ZoneType::None
    }

    /// Sets the zone type at the given coordinates.
    #[allow(clippy::collapsible_if)]
    pub fn set(&mut self, x: i32, y: i32, zone: ZoneType) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let ux = x as usize;
        let uy = y as usize;
        if let Some(idx) = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux)) {
            if idx < self.grid.len() {
                self.grid[idx] = zone;
            }
        }
    }
}

/// Helper to get the zone at a specific position from the world.
#[must_use]
pub fn get_zone_at(world: &World, pos: GridPosition) -> ZoneType {
    world
        .get_resource::<ZoneGrid>()
        .map_or(ZoneType::None, |grid| grid.get(pos.x, pos.y))
}

/// Calculates the efficiency bonus for a building in a zone.
///
/// Returns a multiplier fraction (e.g., 0.2 for +20%).
#[must_use]
pub const fn calculate_zone_bonus(zone: ZoneType, building_type: BuildingType) -> f32 {
    match (building_type, zone) {
        (BuildingType::Housing, ZoneType::Bedroom) => 0.2, // 20% better sleep
        (BuildingType::Tavern, ZoneType::Dining) => 0.1,   // 10% better social
        (BuildingType::Hospital, ZoneType::Hospital) => 0.5, // 50% better healing
        // Stockpiles might just affect organization, no direct float bonus yet
        _ => 0.0,
    }
}

/// Helper to get the zone bonus for a specific building entity.
///
/// Requires `&World` access.
#[must_use]
pub fn get_zone_bonus(world: &World, building_entity: Entity) -> f32 {
    let (Some(building), Some(pos)) = (
        world.get::<Building>(building_entity),
        world.get::<GridPosition>(building_entity),
    ) else {
        return 0.0;
    };

    let zone = get_zone_at(world, *pos);
    calculate_zone_bonus(zone, building.building_type)
}

/// System to apply zone designations immediately.
pub fn apply_zone_designation_system(
    mut commands: Commands,
    mut zone_grid: ResMut<ZoneGrid>,
    query: Query<(
        Entity,
        &crate::layer1::designation::Designation,
        &GridPosition,
    )>,
) {
    for (entity, designation, pos) in &query {
        if let crate::layer1::designation::DesignationType::SetZone(zone_type) =
            designation.designation_type
        {
            zone_grid.set(pos.x, pos.y, zone_type);
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;

    #[test]
    fn test_zone_grid_default() {
        let grid = ZoneGrid::new(10, 10);
        assert_eq!(grid.get(0, 0), ZoneType::None);
    }

    #[test]
    fn test_set_zone() {
        let mut grid = ZoneGrid::new(10, 10);
        grid.set(5, 5, ZoneType::Bedroom);
        assert_eq!(grid.get(5, 5), ZoneType::Bedroom);
    }

    #[test]
    fn test_remove_zone() {
        let mut grid = ZoneGrid::new(10, 10);
        grid.set(5, 5, ZoneType::Dining);
        grid.set(5, 5, ZoneType::None);
        assert_eq!(grid.get(5, 5), ZoneType::None);
    }

    #[test]
    fn test_building_inherits_zone_buff() {
        let mut world = World::new();

        // Setup Grid
        let mut zone_grid = ZoneGrid::new(10, 10);
        zone_grid.set(2, 2, ZoneType::Bedroom);
        world.insert_resource(zone_grid);

        // Spawn Bed at (2,2)
        let bed = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();

        // Spawn Pop using the Bed (e.g. Sleeping state)
        // For test, we just check if helper function identifies the zone bonus
        let bonus = crate::layer1::zone::get_zone_bonus(&world, bed);
        assert!(bonus > 0.0); // Should get Bedroom bonus
    }

    #[test]
    fn test_wrong_zone_no_buff() {
        let mut world = World::new();

        let mut zone_grid = ZoneGrid::new(10, 10);
        zone_grid.set(2, 2, ZoneType::Dining); // Wrong zone for a Bed
        world.insert_resource(zone_grid);

        let bed = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();

        let bonus = crate::layer1::zone::get_zone_bonus(&world, bed);
        assert_eq!(bonus, 0.0);
    }
}
