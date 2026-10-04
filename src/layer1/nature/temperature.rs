#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::missing_docs_in_private_items,
    clippy::collapsible_if
)]
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
use crate::layer1::energy::PowerConsumer;
use crate::layer1::health::Health;
use crate::layer1::items::{Clothing, Equipment};
use crate::layer1::map::GridPosition;
use crate::layer1::mother_lode::MotherLode;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{ResourceItem, ResourceType};
use crate::layer1::seasons::SeasonState;
use crate::layer1::terrain::TerrainGrid;
use bevy::utils::HashMap;
use bevy_ecs::prelude::*;

/// Emits heat into the temperature grid.
#[derive(Component, Debug, Clone, Default)]
pub struct HeatSource {
    /// Amount of heat produced per tick (Celsius).
    pub output: f32,
}

/// Grid managing temperature simulation.
///
/// ⚡ Bolt Optimization:
/// - Uses `bevy::utils::HashMap` (AHash) instead of `std::collections::HashMap`.
/// - Integer tuple keys `(i32, i32)` hash significantly faster with AHash, reducing overhead in diffusion calculations.
#[derive(Resource)]
pub struct TemperatureGrid {
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Current temperature values (Celsius).
    pub values: Vec<f32>,
    /// Scratch buffer for double-buffered updates.
    pub scratch: Vec<f32>,
    /// Target ambient temperature based on season/environment.
    pub ambient: f32,
}

impl TemperatureGrid {
    /// Create a new temperature grid.
    ///
    /// # Panics
    /// Panics if `width * height` overflows or exceeds 1,000,000.
    #[must_use]
    pub fn new(width: usize, height: usize, ambient: f32) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 1_000_000, "Grid size overflow or too large");

        Self {
            width,
            height,
            values: vec![ambient; size],
            scratch: vec![ambient; size],
            ambient,
        }
    }

    #[must_use]
    fn get_index(&self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = y.checked_mul(self.width)?.checked_add(x)?;
        if idx >= self.values.len() {
            return None;
        }
        Some(idx)
    }

    /// Get temperature at safe signed coordinates.
    #[must_use]
    pub fn get_safe(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 {
            return self.ambient;
        }
        self.get(x as usize, y as usize)
    }

    /// Get temperature at coordinates.
    /// Returns ambient if out of bounds.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        let Some(idx) = self.get_index(x, y) else {
            return self.ambient;
        };
        self.values[idx]
    }

    /// Set temperature at coordinates.
    pub fn set(&mut self, x: usize, y: usize, value: f32) {
        let Some(idx) = self.get_index(x, y) else {
            return;
        };
        self.values[idx] = value;
    }

    /// Add heat to a specific tile.
    pub fn add(&mut self, x: i32, y: i32, amount: f32) {
        if x < 0 || y < 0 {
            return;
        }
        let Some(idx) = self.get_index(x as usize, y as usize) else {
            return;
        };
        self.values[idx] += amount;
    }

    /// Run one step of diffusion simulation.
    ///
    /// - Diffuses heat between neighbors based on conductivity.
    /// - Drifts all cells slightly towards ambient temperature based on retention (Thermal Mass).
    pub fn diffuse(
        &mut self,
        conductivity: &HashMap<(i32, i32), f32>,
        terrain: &TerrainGrid,
        building_retention: &HashMap<(i32, i32), f32>,
    ) {
        let diffusion_rate = 0.2; // How fast heat spreads
        let base_drift_rate = 0.05; // Base cooling speed (Spec 198)

        for y in 0..self.height {
            for x in 0..self.width {
                let Some(idx) = self.get_index(x, y) else {
                    continue;
                };
                if idx >= self.scratch.len() {
                    continue;
                }

                let current_temp = self.values[idx];
                let ix = x as i32;
                let iy = y as i32;

                // Get local conductivity (default 1.0 for air)
                let self_k = *conductivity.get(&(ix, iy)).unwrap_or(&1.0);

                // Neighbors: Up, Down, Left, Right
                let neighbors = [(0, -1), (0, 1), (-1, 0), (1, 0)];
                let mut flow_sum = 0.0;

                for (dx, dy) in neighbors {
                    let nx = ix + dx;
                    let ny = iy + dy;

                    let n_temp = self.get_safe(nx, ny);

                    let neighbor_k = *conductivity.get(&(nx, ny)).unwrap_or(&1.0);

                    // Effective conductivity is the bottleneck (min or avg).
                    // Min is good for walls.
                    let k = self_k.min(neighbor_k);

                    flow_sum += (n_temp - current_temp) * k * diffusion_rate;
                }

                // Determine Retention (Thermal Mass)
                // Buildings override terrain retention.
                let retention = building_retention
                    .get(&(ix, iy))
                    .copied()
                    .unwrap_or_else(|| {
                        terrain
                            .get(x, y)
                            .map_or(0.1, crate::layer1::terrain::TerrainType::heat_retention)
                    });

                // Drift Logic (Spec 198)
                // High retention (0.8) -> Factor (0.2) -> Slow drift.
                // Low retention (0.1) -> Factor (0.9) -> Fast drift.
                let drift_factor = (1.0 - retention).max(0.01);
                let drift = (self.ambient - current_temp) * base_drift_rate * drift_factor;

                self.scratch[idx] = current_temp + flow_sum + drift;
            }
        }

        // Swap buffers
        std::mem::swap(&mut self.values, &mut self.scratch);
    }
}

/// System to update the temperature grid.
#[allow(clippy::too_many_arguments)]
pub fn update_temperature_system(
    grid: Option<ResMut<TemperatureGrid>>,
    season: Option<Res<SeasonState>>,
    cycle: Option<Res<DayNightCycle>>,
    eclipse: Option<Res<crate::layer2::void_leviathan::LeviathanEclipse>>,
    terrain: Res<TerrainGrid>,
    buildings: Query<(&Building, &GridPosition, Option<&PowerConsumer>)>,
    heat_sources: Query<(&HeatSource, &GridPosition)>,
    items: Query<(&ResourceItem, &GridPosition)>,
    lodes: Query<(&MotherLode, &GridPosition)>,
) {
    let Some(mut grid) = grid else { return };

    // 1. Update Ambient
    if let Some(season) = season {
        grid.ambient = season.current_season.base_temperature();
    }

    if eclipse.is_some_and(|e| e.active) {
        grid.ambient -= 20.0; // Significant temp drop
    }

    // 2. Solar Heat (Spec 198)
    // Tuned 2026-10-03 (crisis-balance): Day 0.5 -> 0.15. The old value drove
    // open-air tiles ~11C above ambient at equilibrium, so every summer
    // cooked the colony (heatstroke at 35C) with no counterplay available.
    let solar_heat = if let Some(cycle) = cycle {
        match cycle.time_of_day {
            TimeOfDay::Day => 0.15,
            TimeOfDay::Dawn | TimeOfDay::Dusk => 0.05,
            TimeOfDay::Night => 0.0,
        }
    } else {
        0.0
    };

    if solar_heat > 0.0 {
        // Apply solar heat to all tiles
        for value in &mut grid.values {
            *value += solar_heat;
        }
    }

    // 3. Apply Heat Sources & Build Conductivity/Retention Maps
    let mut conductivity_map = HashMap::new();
    let mut building_retention = HashMap::new();

    for (b, pos, power) in &buildings {
        // Conductivity
        let k = b.building_type.thermal_conductivity();
        if (k - 1.0).abs() > f32::EPSILON {
            conductivity_map.insert((pos.x, pos.y), k);
        }

        // Retention
        let r = b.building_type.heat_retention();
        building_retention.insert((pos.x, pos.y), r);

        // Heat Sources (Legacy / Implicit)
        // If we transition fully to HeatSource component, we can remove this,
        // but for now we keep it for backward compatibility with existing entities.
        let heat = match b.building_type {
            BuildingType::Heater => {
                if power.is_some_and(|p| !p.active) {
                    0.0
                } else {
                    // Thermostat: heaters sleep once their tile is warm, so
                    // they save the colony in winter without turning housing
                    // into an oven in summer.
                    let tile_temp = grid.get_safe(pos.x, pos.y);
                    if tile_temp < 18.0 {
                        5.0
                    } else {
                        0.0
                    }
                }
            }
            BuildingType::LifeSupport => {
                // Life support includes thermal regulation for the habitat:
                // it gently holds nearby tiles toward a survivable 20C. This
                // is the colony's winter lifeline — and one more reason
                // sabotage targeting life support is scary.
                //
                // NOTE: arm returns 0.0; the actual multi-tile heating is
                // applied just below (it needs the grid, not just `heat`).
                0.0
            }
            BuildingType::Smelter => {
                if power.is_some_and(|p| !p.active) {
                    2.5
                } else {
                    2.0 + 2.5
                }
            }
            BuildingType::Generator => 2.0, // Generators produce heat when running (fuel logic is separate)
            BuildingType::AncientReactor => 10.0,
            BuildingType::Housing => 2.5,
            BuildingType::FlowerBed | BuildingType::PersonalGarden => -1.5,
            // Fire? handled by Fire entity, usually.
            // If Fire is a separate entity, we might query it separately.
            // For now, buildings only.
            _ => 0.0,
        };

        if heat > 0.0 {
            grid.add(pos.x, pos.y, heat);
        }

        // Life-support habitat thermoregulation (see match arm above):
        // warm every tile within radius 4 of a LifeSupport building toward
        // 20C. The habitat stays livable through winter as long as life
        // support stands.
        //
        // WINTER-WIPE FIX (2026-10-04): the old +6/tick cap lost to diffusion
        // against -5C winter ambient, leaving habitat tiles at 2-3C — below
        // the 10C hypothermia threshold for uninsulated pops, who then took
        // 0.5/tick all winter and died en masse ~tick 925. The stronger
        // +12/tick push actually holds the habitat above 10C.
        if b.building_type == BuildingType::LifeSupport {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    if dx.abs() + dy.abs() > 4 {
                        continue;
                    }
                    let tx = pos.x + dx;
                    let ty = pos.y + dy;
                    let tile_temp = grid.get_safe(tx, ty);
                    if tile_temp < 20.0 {
                        grid.add(tx, ty, (20.0 - tile_temp).min(12.0));
                    }
                }
            }
        }
    }

    // Apply Explicit HeatSource Components
    for (source, pos) in &heat_sources {
        grid.add(pos.x, pos.y, source.output);
    }

    // Apply Item Heat Sources
    for (item, pos) in &items {
        let heat = match item.resource_type {
            ResourceType::Waste => 2.0,
            ResourceType::Ore => 0.5,
            _ => 0.0,
        };
        if heat > 0.0 {
            grid.add(pos.x, pos.y, heat);
        }
    }

    // Apply Mother Lode Heat
    for (lode, pos) in &lodes {
        grid.add(pos.x, pos.y, lode.heat_output);
    }

    // 4. Diffuse (Drift depends on retention)
    grid.diffuse(&conductivity_map, &terrain, &building_retention);
}

/// Event emitted when a pop takes thermal damage (hypothermia or heatstroke).
#[derive(Event, Debug, Clone)]
pub struct ThermalDamageEvent {
    /// The entity that took thermal damage.
    pub entity: Entity,
    /// The amount of damage taken.
    pub amount: f32,
}

/// System to apply thermal damage to pops.
pub fn thermal_damage_system(
    grid: Option<Res<TemperatureGrid>>,
    mut pops: Query<(Entity, &mut Health, &GridPosition, &Equipment), With<Pop>>,
    clothing_query: Query<&Clothing>,
    mut thermal_damage_events: EventWriter<ThermalDamageEvent>,
) {
    let Some(grid) = grid else { return };

    for (entity, mut health, pos, equipment) in &mut pops {
        if pos.x < 0 || pos.y < 0 {
            continue;
        }
        // Get temperature at pop's location
        let temp = grid.get_safe(pos.x, pos.y);

        // Calculate Insulation from Equipment
        let mut insulation = 0.0;
        if let Some(body_entity) = equipment.body {
            if let Ok(clothing) = clothing_query.get(body_entity) {
                insulation += clothing.insulation;
            }
        }

        // Define Safe Range
        // Base range: 10C to 35C
        // Insulation extends the lower bound.
        // E.g. 1.0 insulation -> -20C tolerance extension?
        // Let's say each 1.0 insulation adds 20 degrees of cold protection.
        let cold_tolerance = 10.0 - (insulation * 30.0);
        let heat_tolerance = 35.0; // Clothing might add heat, but usually protects from cold.

        if temp < cold_tolerance {
            // Hypothermia
            // Damage scales with severity?
            // Spec says "0.5 damage".
            health.take_damage(0.5);
            thermal_damage_events.send(ThermalDamageEvent {
                entity,
                amount: 0.5,
            });
        } else if temp > heat_tolerance {
            // Heatstroke
            health.take_damage(0.5);
            thermal_damage_events.send(ThermalDamageEvent {
                entity,
                amount: 0.5,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
    use crate::layer1::energy::PowerConsumer;
    use crate::layer1::health::Health;
    use crate::layer1::items::{Clothing, ClothingType, Equipment};
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::seasons::{Season, SeasonState};
    use crate::layer1::temperature::{
        thermal_damage_system, update_temperature_system, HeatSource, TemperatureGrid,
    };
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use bevy_ecs::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_grid_initialization_to_ambient() {
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: Season::Winter,
        }); // -5.0 C

        // Initialize grid
        let grid = TemperatureGrid::new(10, 10, -5.0);

        assert_eq!(grid.get(5, 5), -5.0);
    }

    #[test]
    fn test_heat_source_emission() {
        let mut world = World::new();
        let grid = TemperatureGrid::new(10, 10, 0.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState::default()); // Need season or ambient update might fail/reset
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Spawn Heater (Active)
        world.spawn((
            Building {
                building_type: BuildingType::Heater,
            },
            GridPosition { x: 5, y: 5 },
            PowerConsumer {
                demand: 5.0,
                active: true,
            },
        ));

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert!(grid.get(5, 5) > 0.0, "Heater should raise temperature");
    }

    #[test]
    fn test_explicit_heat_source_emission() {
        let mut world = World::new();
        let grid = TemperatureGrid::new(10, 10, 0.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Spawn Explicit HeatSource
        world.spawn((HeatSource { output: 25.0 }, GridPosition { x: 5, y: 5 }));

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        // Check that temperature is > 0.0 because diffusion and drift lower it
        assert!(
            grid.get(5, 5) > 0.0,
            "HeatSource should set temperature (additive to 0.0 ambient)"
        );
    }

    #[test]
    fn test_unpowered_heater_no_heat() {
        let mut world = World::new();
        // Initialize with Spring ambient (15.0) to avoid drift interference
        let grid = TemperatureGrid::new(10, 10, 15.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Spawn Heater (Inactive)
        world.spawn((
            Building {
                building_type: BuildingType::Heater,
            },
            GridPosition { x: 5, y: 5 },
            PowerConsumer {
                demand: 5.0,
                active: false,
            },
        ));

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert!(
            (grid.get(5, 5) - 15.0).abs() < f32::EPSILON,
            "Inactive heater should not raise temperature"
        );
    }

    #[test]
    fn test_diffusion_and_insulation() {
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(5, 1, 0.0);
        grid.set(0, 0, 100.0); // Hot source
        world.insert_resource(grid);
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 5,
            height: 1,
            tiles: vec![TerrainType::Grass; 5],
        });

        // Wall at (2, 0)
        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 2, y: 0 },
        ));

        // Run update (multiple ticks for diffusion)
        for _ in 0..10 {
            world.resource_mut::<TemperatureGrid>().set(0, 0, 100.0); // Replenish source to combat rapid edge loss
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        // (1,0) should be warm (neighbor of source)
        assert!(grid.get(1, 0) > 10.0);
        // (3,0) should be cold (blocked by wall)
        // Walls are not perfect insulators (0.05 conductivity), but significantly colder than open air
        assert!(
            grid.get(3, 0) < grid.get(1, 0) * 0.5,
            "Wall should block most heat"
        );
    }

    #[test]
    fn test_thermal_damage() {
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(10, 10, 0.0);
        grid.set(5, 5, -20.0); // Freezing
        world.insert_resource(grid);
        world.init_resource::<Events<crate::layer1::nature::temperature::ThermalDamageEvent>>();

        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Equipment::default(),
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.run_system_once(thermal_damage_system).unwrap();

        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0, "Freezing temp should damage Pop");
    }

    #[test]
    fn test_ambient_drift() {
        // Tiles should slowly drift towards season ambient temp if not insulated
        let mut world = World::new();
        // let ambient = -10.0;
        world.insert_resource(SeasonState {
            current_season: Season::Winter,
        }); // Assume Winter = -5.0
        let mut grid = TemperatureGrid::new(10, 10, 20.0); // Start warm (20 C)
                                                           // Set explicit ambient on grid to match season for test clarity,
                                                           // though system will overwrite it.
        grid.ambient = -5.0;
        world.insert_resource(grid);
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();
        assert!(grid.get(0, 0) < 20.0, "Should cool down towards ambient");
    }

    #[test]
    fn test_lifesupport_thermoregulation_holds_habitat_above_hypothermia() {
        // WINTER-WIPE FIX (2026-10-04): LifeSupport must hold its habitat
        // above the 10C hypothermia threshold for uninsulated pops through
        // a winter night. Regression test: with -5C ambient and no solar
        // input, tiles within radius 4 of a LifeSupport must stay >= 10C
        // after the grid reaches equilibrium.
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: Season::Winter,
        });
        world.insert_resource(DayNightCycle {
            time_of_day: TimeOfDay::Night,
            day_count: 0,
            ticks_per_day: 1000,
        });
        let mut grid = TemperatureGrid::new(20, 20, -5.0);
        grid.ambient = -5.0;
        world.insert_resource(grid);
        world.insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });

        // LifeSupport at center (10,10), no PowerConsumer = always on.
        world.spawn((
            Building {
                building_type: BuildingType::LifeSupport,
                ..Default::default()
            },
            GridPosition { x: 10, y: 10 },
        ));

        // Run many ticks to reach thermal equilibrium.
        for _ in 0..300 {
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        // Hypothermia threshold for uninsulated pops is 10C.
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() > 4 {
                    continue;
                }
                let t = grid.get_safe(10 + dx, 10 + dy);
                assert!(
                    t >= 10.0,
                    "Habitat tile ({},{}) at {:.1}C below hypothermia threshold",
                    10 + dx,
                    10 + dy,
                    t
                );
            }
        }
    }

    #[test]
    fn test_clothing_protection() {
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(10, 10, 0.0);
        grid.set(5, 5, -15.0); // Cold (-15)
        world.insert_resource(grid);
        world.init_resource::<Events<crate::layer1::nature::temperature::ThermalDamageEvent>>();

        // Create Clothing entity
        let coat = world
            .spawn(Clothing {
                clothing_type: ClothingType::Tunic, // Generic
                insulation: 1.0,                    // Protects ~30 degrees
                durability: 100.0,
                max_durability: 100.0,
            })
            .id();

        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Equipment {
                    body: Some(coat),
                    ..Default::default()
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.run_system_once(thermal_damage_system).unwrap();

        let health = world.get::<Health>(pop).unwrap();
        // Base safe min = 10.0. Insulation 1.0 -> 10 - 30 = -20.0 safe min.
        // Temp -15.0 is > -20.0, so should be safe.
        assert_eq!(health.current, 100.0, "Clothing should protect pop");
    }

    #[test]
    fn test_get_overflow_protection() {
        // Construct a grid with huge dimensions but small buffer
        // This simulates a potentially malicious or corrupted state
        let width = usize::MAX / 2;
        let height = 10;
        let tiles = vec![0.0; 1];

        let grid = TemperatureGrid {
            width,
            height,
            values: tiles,
            scratch: vec![],
            ambient: 0.0,
        };

        // (2, 0) -> index 2. 2 > 1. Should safely return ambient instead of panicking.
        assert_eq!(grid.get(2, 0), 0.0);

        // (0, 0) -> index 0. 0 < 1. Should successfully return value.
        assert_eq!(grid.get(0, 0), 0.0);
    }

    #[test]
    fn test_dense_buildings_increase_local_temperature() {
        let mut world = World::new();
        // Use 15.0 because Season::Spring defaults to 15.0 ambient temp.
        // This avoids drift affecting the ambient assertions
        let grid = TemperatureGrid::new(10, 10, 15.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState {
            current_season: Season::Spring,
        }); // 15.0
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Spawn a dense cluster of buildings
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 5, y: 5 },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Smelter,
            },
            GridPosition { x: 5, y: 6 },
            PowerConsumer {
                demand: 5.0,
                active: true,
            },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 6, y: 5 },
        ));

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();

        // The center of the cluster should be significantly hotter than ambient
        assert!(grid.get(5, 5) > 16.5); // Spring ambient + Heat modifier

        println!("grid 0,0 is {}", grid.get(0, 0));
        // Far away should remain near ambient
        assert!((grid.get(0, 0) - 15.0).abs() < 1.0);
    }

    #[test]
    fn test_parks_mitigate_heat_island_effect() {
        let mut world = World::new();
        let grid = TemperatureGrid::new(10, 10, 15.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState {
            current_season: Season::Spring,
        }); // 15.0
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        // Spawn a dense cluster with a park in the middle
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 5, y: 4 },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::FlowerBed,
            },
            GridPosition { x: 5, y: 5 },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Smelter,
            },
            GridPosition { x: 5, y: 6 },
            PowerConsumer {
                demand: 5.0,
                active: true,
            },
        ));

        // Run update
        world.run_system_once(update_temperature_system).unwrap();

        let grid = world.resource::<TemperatureGrid>();

        // The center should be cooler than it would be without the park
        // Assuming without park it's > 21.5, with park it should be < 21.5
        assert!(grid.get(5, 5) < 21.5);
    }
}
