#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss
)]
//! Radioactive system handling radiation grid and sickness.

use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{ResourceItem, ResourceType};
use bevy_ecs::prelude::*;

/// Grid managing radiation levels.
#[derive(Resource)]
pub struct RadiationGrid {
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Radiation values.
    pub values: Vec<f32>,
}

impl RadiationGrid {
    /// Create a new radiation grid.
    ///
    /// # Panics
    /// Panics if `width * height` overflows or exceeds 1,000,000.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 1_000_000, "Grid size overflow or too large");

        Self {
            width,
            height,
            values: vec![0.0; size],
        }
    }

    /// Get radiation level at coordinates.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        if x < self.width && y < self.height {
            if let Some(idx) = y.checked_mul(self.width).and_then(|i| i.checked_add(x)) {
                if idx < self.values.len() {
                    return self.values[idx];
                }
            }
        }
        0.0
    }

    /// Set radiation level at coordinates.
    pub fn set(&mut self, x: usize, y: usize, val: f32) {
        if x < self.width && y < self.height {
            if let Some(idx) = y.checked_mul(self.width).and_then(|i| i.checked_add(x)) {
                if idx < self.values.len() {
                    self.values[idx] = val;
                }
            }
        }
    }

    /// Clear the grid.
    pub fn clear(&mut self) {
        self.values.fill(0.0);
    }

    /// Add a radiation source.
    pub fn add_source(&mut self, x: i32, y: i32, intensity: f32, radius: f32) {
        let r_int = radius.ceil() as i32;
        for dy in -r_int..=r_int {
            for dx in -r_int..=r_int {
                let dist = (dx as f32 * dx as f32 + dy as f32 * dy as f32).sqrt();
                if dist > radius {
                    continue;
                }

                let falloff = 1.0 - (dist / (radius + 0.1));
                if falloff <= 0.0 {
                    continue;
                }

                let nx = x + dx;
                let ny = y + dy;
                if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                    continue;
                }

                let nx_u = nx as usize;
                let ny_u = ny as usize;
                if let Some(idx) = ny_u
                    .checked_mul(self.width)
                    .and_then(|i| i.checked_add(nx_u))
                {
                    if idx < self.values.len() {
                        self.values[idx] += intensity * falloff;
                    }
                }
            }
        }
    }
}

/// Component tracking radiation sickness.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct RadiationSickness {
    /// Severity of sickness (0.0 - 100.0).
    pub severity: f32,
}

/// System to update radiation grid and apply sickness.
pub fn radiation_system(
    grid: Option<ResMut<RadiationGrid>>,
    items: Query<(&ResourceItem, &GridPosition)>,
    mut pops: Query<(Entity, &GridPosition, Option<&mut RadiationSickness>), With<Pop>>,
    mut commands: Commands,
) {
    let Some(mut grid) = grid else { return };

    // 1. Reset Grid
    grid.clear();

    // 2. Add Sources
    for (item, pos) in &items {
        let (rads, radius) = match item.resource_type {
            ResourceType::Waste => (5.0, 3.0),
            ResourceType::Ore => (1.0, 1.0),
            _ => (0.0, 0.0),
        };
        if rads > 0.0 {
            grid.add_source(pos.x, pos.y, rads, radius);
        }
    }

    // 3. Apply to Pops
    for (entity, pos, sickness_opt) in &mut pops {
        if pos.x < 0 || pos.y < 0 {
            continue;
        }
        let exposure = grid.get(pos.x as usize, pos.y as usize);
        if exposure > 0.0 {
            if let Some(mut sick) = sickness_opt {
                sick.severity += exposure * 0.1;
                sick.severity = sick.severity.min(100.0);
            } else {
                commands.entity(entity).insert(RadiationSickness {
                    severity: exposure * 0.1,
                });
            }
        } else if let Some(mut sick) = sickness_opt {
            // Recovery
            sick.severity = (sick.severity - 0.1).max(0.0);
            if sick.severity <= 0.0 {
                commands.entity(entity).remove::<RadiationSickness>();
            }
        }
    }
}

/// System to apply damage from radiation sickness.
pub fn sickness_damage_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Health, &RadiationSickness)>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for (entity, mut health, sick) in &mut query {
        let mut damaged = false;
        if sick.severity > 50.0 {
            health.take_damage(0.1); // Slow death
            damaged = true;
        }
        if sick.severity > 90.0 {
            health.take_damage(0.5); // Fast death
            damaged = true;
        }
        if damaged {
            // Item 84: attribute the damage source at the moment of damage.
            crate::layer1::biology::death_attribution::record_damage(
                &mut commands,
                entity,
                crate::layer1::biology::death_attribution::CAUSE_RADIATION_SICKNESS,
                tick,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::radioactive::{radiation_system, RadiationGrid, RadiationSickness};
    use crate::layer1::resources::{ResourceItem, ResourceType};
    use crate::layer1::temperature::{update_temperature_system, TemperatureGrid};
    use bevy_ecs::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_waste_emits_heat() {
        use crate::layer1::terrain::{TerrainGrid, TerrainType};
        let mut world = World::new();
        // Setup TemperatureGrid
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        // No SeasonState, so ambient stays 0.0

        // Spawn Waste Item
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Waste,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Run temperature update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert!(grid.get(5, 5) > 0.0, "Waste should emit heat");
    }

    #[test]
    fn test_ore_emits_heat() {
        use crate::layer1::terrain::{TerrainGrid, TerrainType};
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Ore,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert!(grid.get(5, 5) > 0.0, "Ore should emit heat");
    }

    #[test]
    fn test_food_does_not_emit_heat() {
        use crate::layer1::terrain::{TerrainGrid, TerrainType};
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Food,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert_eq!(grid.get(5, 5), 0.0, "Food should not emit heat");
    }

    #[test]
    fn test_radiation_grid_accumulates() {
        let mut world = World::new();
        world.insert_resource(RadiationGrid::new(10, 10));

        // Spawn Waste
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Waste,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Run radiation system
        world.run_system_once(radiation_system).unwrap();

        let grid = world.resource::<RadiationGrid>();
        assert!(grid.get(5, 5) > 0.0);
        assert!(grid.get(6, 5) > 0.0); // Spread
    }

    #[test]
    fn test_radiation_sickness_application() {
        let mut world = World::new();
        world.insert_resource(RadiationGrid::new(10, 10));

        // Spawn Waste Source
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Waste,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 5 },
        ));

        let pop = world
            .spawn((
                Pop,
                Health::default(),
                GridPosition { x: 5, y: 5 },
                // Sickness component added by system? Or exists with 0 severity?
            ))
            .id();

        world.run_system_once(radiation_system).unwrap();
        // apply deferred commands
        world.flush();

        // Pop should now have RadiationSickness component
        let sickness = world.get::<RadiationSickness>(pop);
        assert!(sickness.is_some());
        assert!(sickness.unwrap().severity > 0.0);
    }

    #[test]
    fn test_sickness_damages_health() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                RadiationSickness { severity: 60.0 }, // Threshold is usually 50
            ))
            .id();

        // Run health/damage system logic for sickness
        world
            .run_system_once(crate::layer1::radioactive::sickness_damage_system)
            .unwrap();

        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0);
    }

    #[test]
    fn test_get_overflow_protection() {
        // Construct a grid with huge dimensions but small buffer
        // This simulates a potentially malicious or corrupted state
        let width = usize::MAX / 2;
        let height = 10;
        let tiles = vec![0.0; 1];

        let grid = RadiationGrid {
            width,
            height,
            values: tiles,
        };

        // (2, 0) -> index 2. 2 > 1. Should safely return 0.0 instead of panicking.
        assert_eq!(grid.get(2, 0), 0.0);

        // (0, 0) -> index 0. 0 < 1. Should successfully return value.
        assert_eq!(grid.get(0, 0), 0.0);
    }
}
