// src/layer1/atmosphere.rs

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::structure::Structure;
use crate::layer1::weather::{WeatherState, WeatherType};
use crate::shared::time::SimulationTime;
use bevy::utils::HashMap;
use bevy_ecs::prelude::*;

/// Global corrosive atmosphere settings.
///
/// ⚡ Bolt Optimization:
/// - Uses `bevy::utils::HashMap` (AHash) instead of `std::collections::HashMap`.
/// - Integer tuple keys `(i32, i32)` hash significantly faster with AHash, reducing overhead in diffusion calculations.
#[derive(Resource, Default, Debug)]
pub struct CorrosiveAtmosphere {
    /// Intensity of corrosion (0.0 to 1.0 multiplier).
    pub intensity: f32,
}

/// Component that mitigates corrosion damage.
#[derive(Component, Default, Debug)]
pub struct CorrosionResistant {
    /// Resistance factor (0.0 = no resistance, 1.0 = immunity).
    pub factor: f32,
}

/// Marker component for entities protected from atmospheric effects (indoors).
#[derive(Component, Default, Debug)]
pub struct ProtectedFromAtmosphere;

/// System to apply corrosion damage to exposed structures.
pub fn corrosion_damage_system(
    atmosphere: Res<CorrosiveAtmosphere>,
    mut query: Query<(
        &mut Structure,
        Option<&CorrosionResistant>,
        Option<&ProtectedFromAtmosphere>,
    )>,
) {
    if atmosphere.intensity <= f32::EPSILON {
        return;
    }

    let base_damage = 1.0 * atmosphere.intensity;

    for (mut structure, resistance, protected) in &mut query {
        if protected.is_some() {
            continue;
        }

        let resist_factor = resistance.map_or(0.0, |r| r.factor);
        let effective_damage = base_damage * (1.0 - resist_factor).max(0.0);

        if effective_damage > 0.0 {
            structure.current_hp = (structure.current_hp - effective_damage).max(0.0);
        }
    }
}

/// Configuration for atmospheric diffusion.
#[derive(Resource, Debug, Clone)]
pub struct DiffusionConfig {
    /// Horizontal diffusion rate (0.0 to 1.0).
    pub rate: f32,
    /// Vertical escape rate (0.0 to 1.0).
    pub vertical_escape: f32,
}

impl Default for DiffusionConfig {
    fn default() -> Self {
        Self {
            rate: 0.1,
            vertical_escape: 0.05,
        }
    }
}

/// Represents the static "base" wind of the map, unaffected by tides.
#[derive(Resource)]
pub struct BaseGlobalWind {
    /// Normalized direction of the base wind.
    pub direction: crate::layer1::wind::Vec2,
    /// Base speed of the wind.
    pub speed: f32,
}

impl Default for BaseGlobalWind {
    fn default() -> Self {
        Self {
            direction: crate::layer1::wind::Vec2::X,
            speed: 1.0,
        }
    }
}

/// Tracks the global atmospheric pressure.
#[derive(Resource)]
pub struct AtmosphericTide {
    /// Multiplier centered at 1.0. Range usually 0.5 to 1.5.
    pub pressure: f32,
}

impl Default for AtmosphericTide {
    fn default() -> Self {
        Self { pressure: 1.0 }
    }
}

/// Cycle length in ticks for atmospheric tides.
const TIDE_CYCLE_TICKS: u64 = 1000;

/// Updates the atmospheric tide pressure based on simulation time.
pub fn update_atmospheric_tide_system(
    mut tide: ResMut<AtmosphericTide>,
    time: Res<SimulationTime>,
) {
    // Simple Sine Wave: 1.0 + 0.5 * sin(t)
    #[allow(clippy::cast_precision_loss)]
    let phase = (time.tick % TIDE_CYCLE_TICKS) as f32 / TIDE_CYCLE_TICKS as f32;
    let angle = phase * 2.0 * std::f32::consts::PI;
    tide.pressure = 0.5f32.mul_add(angle.sin(), 1.0);
}

/// Syncs the effective global wind based on base wind and atmospheric pressure.
pub fn sync_global_wind_system(
    base: Res<BaseGlobalWind>,
    tide: Res<AtmosphericTide>,
    mut effective: ResMut<crate::layer1::wind::GlobalWind>,
) {
    effective.direction = base.direction;
    effective.speed = base.speed * tide.pressure;
}

/// Calculates the movement cost modifier based on atmospheric pressure.
#[must_use]
pub fn calculate_atmospheric_movement_cost(pressure: f32) -> f32 {
    // High pressure = High Drag = High Cost
    // Low pressure = Low Drag = Low Cost
    // Mapping: 0.5 -> 0.8 cost, 1.5 -> 1.2 cost
    // Formula: 0.6 + 0.4 * pressure
    0.4f32.mul_add(pressure, 0.6)
}

/// Represents the atmospheric pollution layer.
/// Values range from 0.0 (Clean) to 1.0 (Toxic).
#[derive(Resource)]
pub struct AtmosphereGrid {
    /// Width of the grid.
    pub(crate) width: usize,
    /// Height of the grid.
    pub(crate) height: usize,
    /// Flattened grid values.
    pub(crate) values: Vec<f32>,
    /// Secondary buffer for diffusion calculation (double buffering).
    /// Used to avoid allocating a new vector every tick.
    pub(crate) scratch: Vec<f32>,
    /// Retention rate of pollution per tick (0.0 to 1.0).
    /// Higher values mean pollution stays longer.
    /// Default: 0.99.
    pub diffusion_rate: f32,
}

impl AtmosphereGrid {
    /// Create a new empty atmosphere grid.
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
            width,
            height,
            values: vec![0.0; size],
            scratch: vec![0.0; size],
            diffusion_rate: 0.99,
        }
    }

    /// Get pollution level at (x, y). Returns 0.0 if out of bounds.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::collapsible_if)]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 {
            return 0.0;
        }
        let ux = x as usize;
        let uy = y as usize;
        if ux >= self.width || uy >= self.height {
            return 0.0;
        }
        if let Some(idx) = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux)) {
            if idx < self.values.len() {
                return self.values[idx];
            }
        }
        0.0
    }

    /// Set pollution level at (x, y). Clamped between 0.0 and `1_000_000.0`.
    /// Also sanitizes NaN and Infinity to prevent logic errors.
    #[allow(clippy::cast_sign_loss, clippy::collapsible_if)]
    pub fn set(&mut self, x: i32, y: i32, value: f32) {
        if x < 0 || y < 0 {
            return;
        }
        let ux = x as usize;
        let uy = y as usize;
        if ux >= self.width || uy >= self.height {
            return;
        }
        if let Some(idx) = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux)) {
            if idx < self.values.len() {
                // Sanitize input: NaN -> 0.0, Inf -> Max, Negative -> 0.0
                let safe_value = if value.is_finite() {
                    value.clamp(0.0, 1_000_000.0)
                } else if value.is_infinite() && value.is_sign_positive() {
                    1_000_000.0
                } else {
                    0.0
                };
                self.values[idx] = safe_value;
            }
        }
    }

    /// Add pollution at (x, y). Clamped to max 1.0.
    pub fn add(&mut self, x: i32, y: i32, amount: f32) {
        let current = self.get(x, y);
        self.set(x, y, current + amount);
    }

    /// Get interpolated pollution value at float coordinates.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::suboptimal_flops
    )]
    pub fn get_interpolated(&self, x: f32, y: f32) -> f32 {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = x0 + 1;
        let y1 = y0 + 1;

        let wx = x - x0 as f32;
        let wy = y - y0 as f32;

        let v00 = self.get(x0, y0);
        let v10 = self.get(x1, y0);
        let v01 = self.get(x0, y1);
        let v11 = self.get(x1, y1);

        // Bilinear interpolation
        let top = v00.mul_add(1.0 - wx, v10 * wx);
        let bottom = v01.mul_add(1.0 - wx, v11 * wx);

        top.mul_add(1.0 - wy, bottom * wy)
    }

    /// Advect pollution using the wind grid.
    pub fn advect(&mut self, wind_grid: &crate::layer1::wind::WindGrid) {
        if self.width != wind_grid.width || self.height != wind_grid.height {
            return;
        }

        // Ensure scratch buffer size
        if self.scratch.len() != self.values.len() {
            self.scratch = vec![0.0; self.values.len()];
        }

        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap
        )]
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y
                    .checked_mul(self.width)
                    .and_then(|i| i.checked_add(x))
                    .unwrap_or(usize::MAX);

                let wind = wind_grid.get_wind(x as i32, y as i32);
                if wind.length() < f32::EPSILON {
                    let val = self.values.get(idx).copied().unwrap_or(0.0);
                    if let Some(v) = self.scratch.get_mut(idx) {
                        *v = val;
                    }
                    continue;
                }

                // Trace back
                let src_x = x as f32 - wind.x;
                let src_y = y as f32 - wind.y;

                let interpolated_val = self.get_interpolated(src_x, src_y);
                if let Some(v) = self.scratch.get_mut(idx) {
                    *v = interpolated_val;
                }
            }
        }

        std::mem::swap(&mut self.values, &mut self.scratch);
    }

    /// Simulate diffusion and natural decay of pollution.
    /// Uses a simple box blur weighted by `horizontal_rate`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss
    )]
    pub fn diffuse(&mut self, blockers: &HashMap<(i32, i32), f32>, horizontal_rate: f32) {
        // Ensure scratch buffer size matches (in case of dynamic resizing, though rare)
        if self.scratch.len() != self.values.len() {
            self.scratch = vec![0.0; self.values.len()];
        }

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y
                    .checked_mul(self.width)
                    .and_then(|i| i.checked_add(x))
                    .unwrap_or(usize::MAX);

                // If the cell itself is a solid blocker (Wall), it contains no pollution.
                // However, we only consider it a full blocker if its transmissivity is 0.0 (EPSILON).
                if blockers
                    .get(&(x as i32, y as i32))
                    .is_some_and(|&trans| trans <= f32::EPSILON)
                {
                    if let Some(v) = self.scratch.get_mut(idx) {
                        *v = 0.0;
                    }
                    continue;
                }

                let current_val = self.values.get(idx);
                let Some(&current_val) = current_val else {
                    continue;
                };
                let mut sum = current_val;
                let mut total_weight = 1.0;

                // Check 4 neighbors
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0
                        && ny >= 0
                        && (nx as usize) < self.width
                        && (ny as usize) < self.height
                    {
                        let neighbor_val = self.get(nx, ny);
                        let neighbor_trans = *blockers.get(&(nx, ny)).unwrap_or(&1.0);

                        if neighbor_trans > f32::EPSILON {
                            let weight = neighbor_trans * horizontal_rate;
                            sum += neighbor_val * weight;
                            total_weight += weight;
                        }
                    } else {
                        // Vacuum edge sucks pollution away
                        // Edge logic: If horizontal_rate is 0.0, vacuum shouldn't apply?
                        // Or vacuum is a property of the map edge?
                        // Assuming vacuum is a "neighbor with value 0.0".
                        let weight = 1.0 * horizontal_rate;
                        sum += 0.0;
                        total_weight += weight;
                    }
                }

                // Average
                let final_val = if total_weight > 0.0 {
                    sum / total_weight
                } else {
                    0.0
                };

                if let Some(v) = self.scratch.get_mut(idx) {
                    *v = final_val * self.diffusion_rate;
                }
            }
        }
        // Swap buffers
        std::mem::swap(&mut self.values, &mut self.scratch);
    }
}

/// System to update atmospheric advection and emission (Part 1).
///
/// ⚡ Bolt Optimization:
/// We apply emissions directly inside the loop instead of collecting emitters into a `Vec`.
/// This avoids unnecessary per-frame heap allocations.
pub fn update_atmosphere_system(
    mut grid: ResMut<AtmosphereGrid>,
    wind_grid: Option<Res<crate::layer1::wind::WindGrid>>,
    query: Query<(&Building, &GridPosition)>,
) {
    // 1. Advect with Wind
    if let Some(wind) = wind_grid {
        grid.advect(&wind);
    }

    // 2. Emitters
    // Note: We don't collect blockers here anymore, they are used in diffusion system.
    for (b, pos) in query.iter() {
        let emission = match b.building_type {
            BuildingType::Refinery | BuildingType::AncientReactor => 0.08,
            BuildingType::Smelter | BuildingType::Generator => 0.05,
            BuildingType::Smithy => 0.02,
            _ => 0.0,
        };
        if emission > 0.0 {
            grid.add(pos.x, pos.y, emission);
        }
    }
}

/// System to apply health effects from pollution.
///
/// ⚡ Bolt Optimization:
/// We use `world.resource_scope` to extract the `AtmosphereGrid` resource and then query for `&mut Health` directly.
/// This allows applying damage directly in the loop without needing an intermediate `Vec` to store the damages.
pub fn pollution_effects_system(world: &mut World) {
    let mut query = world.query_filtered::<(&GridPosition, &mut Health), With<Pop>>();

    world.resource_scope(|world, grid: Mut<AtmosphereGrid>| {
        for (pos, mut health) in query.iter_mut(world) {
            let pollution = grid.get(pos.x, pos.y);
            // Threshold 0.3
            if pollution > 0.3 {
                let damage = (pollution - 0.3) * 0.1;
                health.take_damage(damage);
            }
        }
    });
}

/// Updates diffusion rate based on weather conditions.
pub fn update_weather_diffusion_system(
    mut grid: ResMut<AtmosphereGrid>,
    config: Res<DiffusionConfig>,
    weather: Res<WeatherState>,
) {
    // 1. Determine effective escape rate
    let vertical_escape = if weather.current_weather == WeatherType::ThermalInversion {
        0.0
    } else {
        config.vertical_escape
    };

    // Update grid diffusion rate (retention = 1.0 - escape)
    grid.diffusion_rate = 1.0 - vertical_escape;
}

/// Simulates gas diffusion with weather effects (e.g. Thermal Inversion).
pub fn simulate_diffusion_system(
    mut grid: ResMut<AtmosphereGrid>,
    config: Res<DiffusionConfig>,
    query: Query<(&Building, &GridPosition)>,
    vents_query: Query<(&GridPosition, &crate::layer1::physics::vent::VentConnection)>,
) {
    // Diffusion rate is now set by update_weather_diffusion_system (and potentially modified by terraforming)

    // 2. Identify Blockers
    let mut blockers = HashMap::new();
    for (b, pos) in query.iter() {
        if let Some(transmissivity) = b.building_type.flow_transmissivity() {
            blockers.insert((pos.x, pos.y), transmissivity);
        }
    }

    for (pos, vent) in vents_query.iter() {
        let airflow = crate::layer1::physics::vent::calculate_vent_airflow(vent);
        blockers.insert((pos.x, pos.y), airflow);
    }

    // 3. Diffuse
    grid.diffuse(&blockers, config.rate);
}

/// Applies damage from high smog levels.
pub fn apply_smog_damage_system(
    grid: Res<AtmosphereGrid>,
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &GridPosition,
            &mut Health,
            Option<&crate::layer1::economy::items::Equipment>,
        ),
        With<Pop>,
    >,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    for (entity, pos, mut health, equipment) in &mut query {
        let smog_level = grid.get(pos.x, pos.y);
        if smog_level > 150.0 {
            let has_mask = equipment.is_some_and(|e| e.mask.is_some());
            if !has_mask && !health.has_rust_lung {
                // Suffocation / Toxicity
                health.take_damage(1.0);
                // Item 84: attribute the damage source at the moment of damage.
                crate::layer1::biology::death_attribution::record_damage(
                    &mut commands,
                    entity,
                    crate::layer1::biology::death_attribution::CAUSE_TOXIC_SMOG,
                    tick,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_atmosphere_grid_initialization() {
        let grid = AtmosphereGrid::new(10, 10);
        assert_eq!(grid.width, 10);
        assert_eq!(grid.height, 10);
        assert_eq!(grid.get(0, 0), 0.0);
    }

    #[test]
    fn test_pollution_emission() {
        let mut world = World::new();
        let grid = AtmosphereGrid::new(10, 10);
        world.insert_resource(grid);

        // Spawn a Smelter (dirty building)
        world.spawn((
            Building {
                building_type: BuildingType::Smelter,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Run update
        let mut schedule = Schedule::default();
        schedule.add_systems(update_atmosphere_system);
        schedule.run(&mut world);

        // Check pollution at source
        let grid = world.resource::<AtmosphereGrid>();
        assert!(grid.get(5, 5) > 0.0, "Smelter should emit pollution");
    }

    #[test]
    fn test_pollution_diffusion() {
        let mut grid = AtmosphereGrid::new(3, 3);
        grid.set(1, 1, 10.0); // High pollution in center

        // Simulate one step of diffusion with 1.0 rate (full mix)
        grid.diffuse(&HashMap::new(), 1.0);

        // Center should decrease, neighbors should increase
        assert!(
            grid.get(1, 1) < 10.0,
            "Pollution should diffuse away from center"
        );
        assert!(
            grid.get(0, 1) > 0.0,
            "Pollution should diffuse to neighbors"
        );
        assert!(
            grid.get(1, 0) > 0.0,
            "Pollution should diffuse to neighbors"
        );
    }

    #[test]
    fn test_pollution_health_impact() {
        let mut world = World::new();
        let mut grid = AtmosphereGrid::new(10, 10);
        grid.set(5, 5, 1.0); // Max pollution
        world.insert_resource(grid);

        // Spawn Pop in pollution
        let pop = world
            .spawn((Pop, Health::default(), GridPosition { x: 5, y: 5 }))
            .id();

        // Run effects
        pollution_effects_system(&mut world);

        // Health should drop
        let health = world.get::<Health>(pop).expect("Component should exist");
        assert!(
            health.current < 100.0,
            "Health should drop due to pollution"
        );
    }

    #[test]
    fn test_pollution_blocked_by_wall() {
        let mut world = World::new();
        world.insert_resource(DiffusionConfig {
            rate: 1.0,
            vertical_escape: 0.05,
        });
        world.insert_resource(WeatherState::default());

        let mut grid = AtmosphereGrid::new(5, 1);
        grid.set(0, 0, 1.0); // Source
        world.insert_resource(grid);

        // Wall at (1, 0)
        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 1, y: 0 },
        ));

        // Run atmosphere update AND diffusion
        let mut schedule = Schedule::default();
        schedule.add_systems((update_atmosphere_system, simulate_diffusion_system).chain());
        for _ in 0..5 {
            schedule.run(&mut world);
        }

        let grid = world.resource::<AtmosphereGrid>();
        assert!(
            grid.get(2, 0) < 0.01,
            "Pollution should NOT pass through Wall"
        );
    }

    #[test]
    fn test_pollution_passes_through_vent() {
        let mut world = World::new();
        world.insert_resource(DiffusionConfig {
            rate: 1.0,
            vertical_escape: 0.05,
        });
        world.insert_resource(WeatherState::default());

        let mut grid = AtmosphereGrid::new(5, 1);
        grid.set(0, 0, 1.0); // Source
        world.insert_resource(grid);

        // Smelter at (0, 0) to maintain source
        world.spawn((
            Building {
                building_type: BuildingType::Smelter,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Vent at (1, 0)
        world.spawn((
            Building {
                building_type: BuildingType::Vent,
            },
            GridPosition { x: 1, y: 0 },
        ));

        // Run atmosphere update AND diffusion
        let mut schedule = Schedule::default();
        schedule.add_systems((update_atmosphere_system, simulate_diffusion_system).chain());
        for _ in 0..20 {
            // Manually refill source
            world.resource_mut::<AtmosphereGrid>().set(0, 0, 1.0);
            schedule.run(&mut world);
        }

        let grid = world.resource::<AtmosphereGrid>();
        assert!(grid.get(2, 0) > 0.05, "Pollution SHOULD pass through Vent");
    }
}

#[derive(Resource, Default)]
pub struct SmogGrid {
    pub levels: bevy::utils::HashMap<GridPosition, f32>,
}

impl SmogGrid {
    pub fn get_smog(&self, pos: GridPosition) -> f32 {
        self.levels.get(&pos).copied().unwrap_or(0.0)
    }
    pub fn set_smog(&mut self, pos: GridPosition, amount: f32) {
        self.levels.insert(pos, amount);
    }
}
