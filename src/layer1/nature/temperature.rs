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
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use bevy::utils::HashMap;
use bevy_ecs::prelude::*;

/// Emits heat into the temperature grid.
#[derive(Component, Debug, Clone, Default)]
pub struct HeatSource {
    /// Amount of heat produced per tick (Celsius).
    pub output: f32,
}

/// Spec 404: a standalone thermal emitter — a heater, vent, or cooler
/// that adds heat to (positive `heat_output`) or removes heat from
/// (negative) its own tile every tick.
///
/// ADAPTATION (spec 404 grounding): the spec sketch assumed no
/// temperature system existed. Trunk already has [`HeatSource`] and a
/// full [`TemperatureGrid`]; `ThermalEmitter` is the spec's named,
/// building-independent seam (any entity with a [`GridPosition`] can
/// carry one — no `Building` required), applied by
/// [`update_temperature_system`] alongside `HeatSource`, and standalone
/// via [`thermal_emission_system`].
#[derive(Component, Debug, Clone, Default)]
pub struct ThermalEmitter {
    /// Heat added per tick (Celsius). Negative values cool.
    pub heat_output: f32,
}

/// Spec 404: a standalone thermal insulator on a tile.
///
/// `insulation_value` is 0.0 (fully conductive) to 1.0 (perfect
/// insulator). During diffusion the tile's effective conductivity is
/// `1.0 - insulation_value`, folded into the same conductivity map the
/// building system already builds (Walls sit at 0.05) — the diffuse
/// step's bottleneck (`min`) rule does the rest, so an insulator is an
/// O(1) hash entry per tile, never a per-neighbor query scan (spec §5).
#[derive(Component, Debug, Clone, Default)]
pub struct ThermalInsulator {
    /// Fraction of heat transfer blocked, 0.0..=1.0 (clamped on use).
    pub insulation_value: f32,
}

impl ThermalInsulator {
    /// Effective thermal conductivity of an insulated tile.
    #[must_use]
    pub fn conductivity(&self) -> f32 {
        1.0 - self.insulation_value.clamp(0.0, 1.0)
    }
}

/// Fold one insulator tile into a conductivity map (Spec 404).
///
/// The map is keyed by tile and read once per tile per diffuse step, so
/// this is the spec §5 REFACTOR spatial index: insulators become map
/// entries instead of an O(N) component scan inside the diffusion
/// loops. An existing lower conductivity (e.g. a Wall's 0.05) wins —
/// insulation can only ever reduce transfer, never increase it.
pub fn merge_insulation_into_conductivity(
    conductivity: &mut HashMap<(i32, i32), f32>,
    pos: (i32, i32),
    insulation_value: f32,
) {
    let k = 1.0 - insulation_value.clamp(0.0, 1.0);
    let entry = conductivity.entry(pos).or_insert(1.0);
    *entry = entry.min(k);
}

/// Census of the Spec 404 thermal layer for STATS / headless panels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThermalCensus {
    /// Live [`ThermalEmitter`] entities.
    pub emitters: usize,
    /// Live [`ThermalInsulator`] entities.
    pub insulators: usize,
    /// Mean tile temperature across the grid (0.0 with no grid).
    pub avg_temp: f32,
    /// Hottest tile temperature (ambient with no grid).
    pub max_temp: f32,
}

/// Count emitters/insulators and summarize the grid (Spec 404).
pub fn thermal_census(world: &mut World) -> ThermalCensus {
    let emitters = world
        .query::<&ThermalEmitter>()
        .iter(world)
        .count();
    let insulators = world
        .query::<&ThermalInsulator>()
        .iter(world)
        .count();
    let (avg_temp, max_temp) = world.get_resource::<TemperatureGrid>().map_or(
        (0.0, 0.0),
        |grid| {
            if grid.values.is_empty() {
                (grid.ambient, grid.ambient)
            } else {
                let sum: f32 = grid.values.iter().sum();
                #[allow(clippy::cast_precision_loss)]
                let avg = sum / grid.values.len() as f32;
                let max = grid
                    .values
                    .iter()
                    .copied()
                    .fold(f32::MIN, f32::max);
                (avg, max)
            }
        },
    );
    ThermalCensus {
        emitters,
        insulators,
        avg_temp,
        max_temp,
    }
}

/// Compact STATS label for the thermal layer (Spec 404), mirroring the
/// `dysonian_stats_label` pattern: `avg<avg>/max<max>/<n>e/<n>i`.
pub fn thermal_stats_label(world: &mut World) -> String {
    let census = thermal_census(world);
    format!(
        "avg{:.1}/max{:.1}/{}e/{}i",
        census.avg_temp, census.max_temp, census.emitters, census.insulators
    )
}

/// Spawn a bare Spec 404 emitter entity at a tile (headless/debug seam).
pub fn spawn_thermal_emitter(
    world: &mut World,
    x: i32,
    y: i32,
    heat_output: f32,
) -> Entity {
    world
        .spawn((
            GridPosition { x, y },
            ThermalEmitter { heat_output },
        ))
        .id()
}

/// Spawn a bare Spec 404 insulator entity at a tile (headless/debug seam).
pub fn spawn_thermal_insulator(
    world: &mut World,
    x: i32,
    y: i32,
    insulation_value: f32,
) -> Entity {
    world
        .spawn((
            GridPosition { x, y },
            ThermalInsulator { insulation_value },
        ))
        .id()
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
    quirks: Option<Res<crate::layer1::quirks::PlanetaryTraits>>,
    cycle: Option<Res<DayNightCycle>>,
    dysonian: Option<Res<crate::layer2::dysonian::DysonianState>>,
    rings: Option<Res<crate::layer2::planetary_rings::PlanetaryRingState>>,
    eclipse: Option<Res<crate::layer2::void_leviathan::LeviathanEclipse>>,
    terrain: Res<TerrainGrid>,
    buildings: Query<(&Building, &GridPosition, Option<&PowerConsumer>)>,
    heat_sources: Query<(&HeatSource, &GridPosition)>,
    items: Query<(&ResourceItem, &GridPosition)>,
    lodes: Query<(&MotherLode, &GridPosition)>,
    thermal_emitters: Query<(&ThermalEmitter, &GridPosition)>,
    thermal_insulators: Query<(&ThermalInsulator, &GridPosition)>,
) {
    let Some(mut grid) = grid else { return };

    // 1. Update Ambient
    if let Some(season) = season {
        grid.ambient = season.current_season.base_temperature();
    }

    if eclipse.is_some_and(|e| e.active) {
        grid.ambient -= 20.0; // Significant temp drop
    }

    // Spec 345: planetary quirks shift the ambient baseline (Volcanic +5C).
    if let Some(traits) = quirks.as_deref() {
        grid.ambient +=
            crate::layer1::quirks::GlobalModifiers::from_traits(&traits.0).temperature_offset;
    }

    // Spec 403 Dysonian Swarms: the collector swarm's shadow drops
    // the ambient baseline. Ambient is recomputed from season each
    // tick above, so this is absolute, not cumulative; shadow 0.0
    // (or no state) is an exact no-op.
    let dyson_shadow = dysonian.as_deref().map_or(0.0, |s| s.shadow_factor);
    if dyson_shadow > 0.0 {
        grid.ambient -= crate::layer2::dysonian::TEMPERATURE_DROP_PER_SHADOW * dyson_shadow;
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

    // Spec 403: insolation scales the solar heat that gets through.
    let solar_heat = solar_heat * (1.0 - dyson_shadow);

    // Spec 407 Planetary Rings: the colony planet's ring blocks
    // sunlight in one band of rows. With no ring shadow both terms
    // below are exact no-ops (band tiles get the full solar heat,
    // and the post-diffusion band relaxation is skipped).
    let (ring_shadow, ring_band_drop) = rings.as_deref().map_or((0.0, 0.0), |s| {
        (s.shadow_factor, s.band_temp_drop)
    });
    if solar_heat > 0.0 {
        // Apply solar heat to all tiles; band tiles receive only
        // the fraction of insolation the ring lets through.
        if ring_shadow > 0.0 {
            let width = grid.width.max(1);
            let band = crate::layer2::planetary_rings::ring_shadow_band(grid.height);
            for (idx, value) in grid.values.iter_mut().enumerate() {
                let y = idx / width;
                *value += if band.contains(&y) {
                    solar_heat * (1.0 - ring_shadow)
                } else {
                    solar_heat
                };
            }
        } else {
            for value in &mut grid.values {
                *value += solar_heat;
            }
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
                // it gently holds nearby tiles toward a survivable 20C in
                // both directions — heating in winter, cooling in summer.
                // This is the colony's year-round lifeline — and one more
                // reason sabotage targeting life support is scary.
                //
                // NOTE: arm returns 0.0; the actual multi-tile
                // thermoregulation is applied just below (it needs the
                // grid, not just `heat`).
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
        // push every tile within radius 4 of a LifeSupport building toward
        // 20C — heating cold tiles, cooling hot ones. The habitat stays
        // livable year-round as long as life support stands.
        //
        // WINTER-WIPE FIX (2026-10-04): the old +6/tick cap lost to diffusion
        // against -5C winter ambient, leaving habitat tiles at 2-3C — below
        // the 10C hypothermia threshold for uninsulated pops, who then took
        // 0.5/tick all winter and died en masse ~tick 925. The stronger
        // +12/tick push actually holds the habitat above 10C.
        //
        // SUMMER-COOLING FIX (item 96, 2026-10-17): the arm above only ever
        // heated (`tile_temp < 20.0`), so there was no cooling verb at all —
        // building heat (+2.5 Housing, +2.0 Generator, ...) and summer solar
        // gain cooked habitat interiors to 35–47C. The 2026-10-17 balance
        // re-baseline found heatstroke was 42% of pooled deaths, 97% of
        // them on pressurized tiles with LifeSupport intact. The cooling
        // branch mirrors the winter fix exactly: a symmetric capped push
        // toward 20C, so heating behavior is unchanged below 20C.
        // POWER GATE (item 121, balance-loop Bug #1): thermoregulation is
        // powered work — an inactive PowerConsumer means the unit is off,
        // exactly like the Heater/Smelter arms above and the pressure
        // system (`power.is_none_or(|p| p.active)`). A LifeSupport with NO
        // PowerConsumer at all still runs unconditionally (legacy spawns
        // and the thermoregulation regression tests rely on that).
        if b.building_type == BuildingType::LifeSupport && !power.is_some_and(|p| !p.active) {
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
                    } else if tile_temp > 20.0 {
                        grid.add(tx, ty, -(tile_temp - 20.0).min(12.0));
                    }
                }
            }
        }
    }

    // Apply Explicit HeatSource Components
    for (source, pos) in &heat_sources {
        grid.add(pos.x, pos.y, source.output);
    }

    // Spec 404: Apply ThermalEmitter Components (positive heats,
    // negative cools — the cooler half of the spec's acceptance list).
    for (emitter, pos) in &thermal_emitters {
        grid.add(pos.x, pos.y, emitter.heat_output);
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

    // Spec 404: fold ThermalInsulator tiles into the conductivity map
    // built above, so the existing double-buffered diffuse() step sees
    // them through its bottleneck rule. No insulators -> the map is
    // exactly what trunk built before this spec (byte-compatible).
    for (insulator, pos) in &thermal_insulators {
        merge_insulation_into_conductivity(
            &mut conductivity_map,
            (pos.x, pos.y),
            insulator.insulation_value,
        );
    }

    // 4. Diffuse (Drift depends on retention)
    grid.diffuse(&conductivity_map, &terrain, &building_retention);

    // Spec 407: after diffusion, shadow-band tiles relax toward the
    // band's colder target (ambient minus the ring's band drop).
    // This is a bounded relaxation, not a per-tick subtraction, so
    // the band equilibrates colder instead of freezing without
    // limit; heaters / LifeSupport in the band still fight back
    // through the normal heat-source legs above.
    if ring_band_drop > 0.0 {
        let target = grid.ambient - ring_band_drop;
        let band = crate::layer2::planetary_rings::ring_shadow_band(grid.height);
        for y in band {
            for x in 0..grid.width {
                let current = grid.get(x, y);
                grid.set(x, y, current + (target - current) * 0.2);
            }
        }
    }
}

/// Spec 404 standalone emission leg: add every [`ThermalEmitter`]'s
/// output to its tile.
///
/// This is the spec's named system, kept as a composable piece for
/// isolated schedules/tests. PRODUCTION does not schedule it
/// separately — [`update_temperature_system`] applies emitters itself
/// (between the `HeatSource` leg and diffusion), so scheduling both
/// would double-count every emitter.
pub fn thermal_emission_system(
    mut grid: ResMut<TemperatureGrid>,
    emitters: Query<(&GridPosition, &ThermalEmitter)>,
) {
    for (pos, emitter) in emitters.iter() {
        grid.add(pos.x, pos.y, emitter.heat_output);
    }
}

/// Spec 404 standalone diffusion leg: one double-buffered diffusion
/// step over the existing [`TemperatureGrid`], honoring
/// [`ThermalInsulator`] tiles.
///
/// Delegates to the grid's own `diffuse()` — the same code production
/// runs inside [`update_temperature_system`] — so conductivity
/// bottleneck, terrain/building retention, and the ambient drift
/// (the grid trending back toward ambient, spec §5) are identical,
/// never a parallel reimplementation. When no `TerrainGrid` resource
/// exists (the spec's isolated test setup), a flat grass grid stands
/// in so retention falls back to the terrain defaults.
pub fn thermal_diffusion_system(
    mut grid: ResMut<TemperatureGrid>,
    terrain: Option<Res<TerrainGrid>>,
    insulators: Query<(&GridPosition, &ThermalInsulator)>,
) {
    let mut conductivity_map = HashMap::new();
    for (pos, insulator) in insulators.iter() {
        merge_insulation_into_conductivity(
            &mut conductivity_map,
            (pos.x, pos.y),
            insulator.insulation_value,
        );
    }
    let building_retention = HashMap::new();
    match terrain {
        Some(terrain) => grid.diffuse(&conductivity_map, &terrain, &building_retention),
        None => {
            let flat = TerrainGrid {
                width: grid.width,
                height: grid.height,
                tiles: vec![TerrainType::Grass; grid.width * grid.height],
            };
            grid.diffuse(&conductivity_map, &flat, &building_retention);
        }
    }
}

/// Event emitted when a pop takes thermal damage (hypothermia or heatstroke).
#[derive(Event, Debug, Clone)]
pub struct ThermalDamageEvent {
    /// The entity that took thermal damage.
    pub entity: Entity,
    /// The amount of damage taken.
    pub amount: f32,
    /// True when the damage is heatstroke (tile above the heat
    /// tolerance); false for hypothermia. Lets downstream consumers
    /// (e.g. the INT-1335 chronicle bridge) telegraph the honest cause.
    pub is_heatstroke: bool,
}

/// System to apply thermal damage to pops.
pub fn thermal_damage_system(
    grid: Option<Res<TemperatureGrid>>,
    mut pops: Query<
        (Entity, &mut Health, &GridPosition, &Equipment),
        (
            With<Pop>,
            // Item 100: dormant origin pops are in stasis at their
            // sites (see pressure_damage_system) — they are excluded
            // from utility AI and tethered in place, so they can never
            // seek warmth; thermal damage cannot touch them until
            // they wake and the dormant marker is removed.
            Without<crate::layer1::culture::lawbound::DormantAutomaton>,
            Without<crate::layer1::culture::bloomtouched::DormantTouched>,
            Without<crate::layer1::culture::chronostalker::DormantStalker>,
        ),
    >,
    clothing_query: Query<&Clothing>,
    mut thermal_damage_events: EventWriter<ThermalDamageEvent>,
    mut commands: Commands,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let Some(grid) = grid else { return };
    let tick = time.map_or(0, |t| t.tick);

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
            // Item 84: attribute the damage source at the moment of damage.
            crate::layer1::biology::death_attribution::record_damage(
                &mut commands,
                entity,
                crate::layer1::biology::death_attribution::CAUSE_HYPOTHERMIA,
                tick,
            );
            thermal_damage_events.send(ThermalDamageEvent {
                entity,
                amount: 0.5,
                is_heatstroke: false,
            });
        } else if temp > heat_tolerance {
            // Heatstroke
            health.take_damage(0.5);
            crate::layer1::biology::death_attribution::record_damage(
                &mut commands,
                entity,
                crate::layer1::biology::death_attribution::CAUSE_HEATSTROKE,
                tick,
            );
            thermal_damage_events.send(ThermalDamageEvent {
                entity,
                amount: 0.5,
                is_heatstroke: true,
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
        thermal_damage_system, thermal_diffusion_system, thermal_emission_system,
        update_temperature_system, HeatSource, TemperatureGrid, ThermalEmitter,
        ThermalInsulator,
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
    fn test_dormant_origin_pops_take_no_thermal_damage() {
        // Item 100: the same stasis that exempts dormant origin pops from
        // suffocation covers thermal damage — balance data shows dormant
        // Vigils/Quills/Tocks whose sites were pressurized surviving the
        // vacuum, then freezing in place over winter (they are excluded
        // from utility AI and tethered to their sites, so they can never
        // seek warmth). An ordinary pop on the same tiles still freezes.
        let mut world = World::new();
        let grid = TemperatureGrid::new(10, 10, -20.0);
        world.insert_resource(grid);
        world.init_resource::<Events<crate::layer1::nature::temperature::ThermalDamageEvent>>();

        let mk_health = || Health {
            current: 100.0,
            max: 100.0,
            has_rust_lung: false,
        };
        let vigil = world
            .spawn((
                Pop,
                mk_health(),
                Equipment::default(),
                GridPosition { x: 5, y: 5 },
                crate::layer1::culture::lawbound::DormantAutomaton,
            ))
            .id();
        let quill = world
            .spawn((
                Pop,
                mk_health(),
                Equipment::default(),
                GridPosition { x: 6, y: 5 },
                crate::layer1::culture::bloomtouched::DormantTouched,
            ))
            .id();
        let tock = world
            .spawn((
                Pop,
                mk_health(),
                Equipment::default(),
                GridPosition { x: 7, y: 5 },
                crate::layer1::culture::chronostalker::DormantStalker,
            ))
            .id();
        let control = world
            .spawn((
                Pop,
                mk_health(),
                Equipment::default(),
                GridPosition { x: 8, y: 5 },
            ))
            .id();

        world.run_system_once(thermal_damage_system).unwrap();

        for dormant in [vigil, quill, tock] {
            let health = world.get::<Health>(dormant).unwrap();
            assert_eq!(
                health.current, 100.0,
                "dormant origin pops are in stasis — cold cannot touch them"
            );
        }
        let health = world.get::<Health>(control).unwrap();
        assert!(
            health.current < 100.0,
            "an ordinary pop on the same freezing tiles still takes damage"
        );
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
    fn test_lifesupport_thermoregulation_holds_habitat_below_heatstroke() {
        // SUMMER TWIN of the winter-wipe regression test above (item 96,
        // 2026-10-17): LifeSupport thermoregulation was heating-only, so
        // habitat interiors cooked all summer — the 2026-10-17 balance
        // re-baseline found heatstroke = 42% of pooled deaths, 97% of
        // them on pressurized tiles at 35–47C with LifeSupport intact.
        // With 30C summer ambient, daytime solar gain, and building heat
        // from a housing cluster inside the habitat, tiles within
        // radius 4 of a LifeSupport must stay BELOW the 35C heatstroke
        // threshold after the grid reaches equilibrium.
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: Season::Summer,
        });
        world.insert_resource(DayNightCycle {
            time_of_day: TimeOfDay::Day,
            day_count: 0,
            ticks_per_day: 1000,
        });
        let mut grid = TemperatureGrid::new(20, 20, 30.0);
        grid.ambient = 30.0;
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
            },
            GridPosition { x: 10, y: 10 },
        ));

        // Building heat inside the habitat: a housing ring (+2.5/tick
        // each) plus a generator (+2.0/tick) — the heat load a real
        // habitat carries through summer.
        for (x, y) in [
            (9, 9),
            (10, 9),
            (11, 9),
            (9, 10),
            (11, 10),
            (9, 11),
            (10, 11),
            (11, 11),
        ] {
            world.spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x, y },
            ));
        }
        world.spawn((
            Building {
                building_type: BuildingType::Generator,
            },
            GridPosition { x: 12, y: 10 },
        ));

        // Run many ticks to reach thermal equilibrium.
        for _ in 0..300 {
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        // Heatstroke threshold for pops is 35C.
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() > 4 {
                    continue;
                }
                let t = grid.get_safe(10 + dx, 10 + dy);
                assert!(
                    t < 35.0,
                    "Habitat tile ({},{}) at {:.1}C above heatstroke threshold",
                    10 + dx,
                    10 + dy,
                    t
                );
            }
        }
    }

    #[test]
    fn test_unpowered_lifesupport_does_not_heat_habitat() {
        // Item 121 (balance-loop Bug #1): a LifeSupport whose PowerConsumer
        // is inactive must NOT thermoregulate — the Heater and Smelter arms
        // already gate on `power.is_some_and(|p| !p.active)`, and the
        // pressure system treats an inactive consumer as off. Before the
        // fix, the habitat block ran unconditionally, so an unpowered
        // LifeSupport heated its habitat for free. Cold-grid twin of the
        // winter-wipe regression test: same -5C winter night, but the
        // LifeSupport carries an inactive PowerConsumer, so habitat tiles
        // must stay BELOW the 10C hypothermia threshold (near ambient).
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

        // LifeSupport at center (10,10) with an INACTIVE PowerConsumer.
        world.spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 10, y: 10 },
            PowerConsumer {
                demand: 10.0,
                active: false,
            },
        ));

        for _ in 0..300 {
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() > 4 {
                    continue;
                }
                let t = grid.get_safe(10 + dx, 10 + dy);
                assert!(
                    t < 10.0,
                    "Unpowered LifeSupport heated habitat tile ({},{}) to {:.1}C — thermoregulation must be power-gated",
                    10 + dx,
                    10 + dy,
                    t
                );
            }
        }
    }

    #[test]
    fn test_unpowered_lifesupport_does_not_cool_habitat() {
        // Item 121, hot-grid twin: same summer heat load as the heatstroke
        // regression test (30C ambient, daytime solar, housing ring +
        // generator inside the habitat), but the LifeSupport's
        // PowerConsumer is inactive, so nothing cools the habitat and at
        // least one habitat tile must reach the 35C heatstroke threshold.
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: Season::Summer,
        });
        world.insert_resource(DayNightCycle {
            time_of_day: TimeOfDay::Day,
            day_count: 0,
            ticks_per_day: 1000,
        });
        let mut grid = TemperatureGrid::new(20, 20, 30.0);
        grid.ambient = 30.0;
        world.insert_resource(grid);
        world.insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });

        // LifeSupport at center (10,10) with an INACTIVE PowerConsumer.
        world.spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 10, y: 10 },
            PowerConsumer {
                demand: 10.0,
                active: false,
            },
        ));

        for (x, y) in [
            (9, 9),
            (10, 9),
            (11, 9),
            (9, 10),
            (11, 10),
            (9, 11),
            (10, 11),
            (11, 11),
        ] {
            world.spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x, y },
            ));
        }
        world.spawn((
            Building {
                building_type: BuildingType::Generator,
            },
            GridPosition { x: 12, y: 10 },
        ));

        for _ in 0..300 {
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        let mut max_habitat = f32::MIN;
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() > 4 {
                    continue;
                }
                max_habitat = max_habitat.max(grid.get_safe(10 + dx, 10 + dy));
            }
        }
        assert!(
            max_habitat >= 35.0,
            "Unpowered LifeSupport cooled the habitat — hottest habitat tile only {max_habitat:.1}C, expected >= 35C heatstroke threshold with no cooling"
        );
    }

    #[test]
    fn test_powered_lifesupport_still_thermoregulates() {
        // Item 121, positive control: a LifeSupport with an ACTIVE
        // PowerConsumer must thermoregulate exactly like the no-consumer
        // case in the winter-wipe regression test — habitat held at or
        // above the 10C hypothermia threshold through a -5C winter night.
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

        // LifeSupport at center (10,10) with an ACTIVE PowerConsumer.
        world.spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 10, y: 10 },
            PowerConsumer {
                demand: 10.0,
                active: true,
            },
        ));

        for _ in 0..300 {
            world.run_system_once(update_temperature_system).unwrap();
        }

        let grid = world.resource::<TemperatureGrid>();
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() > 4 {
                    continue;
                }
                let t = grid.get_safe(10 + dx, 10 + dy);
                assert!(
                    t >= 10.0,
                    "Powered LifeSupport failed to hold habitat tile ({},{}) above hypothermia threshold: {:.1}C",
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

    // ---- Spec 404 Thermal Management (adapted RED tests) ----
    // The spec sketch used an App + MapDimensions; trunk's temperature
    // tests use World + run_system_once and there is no MapDimensions
    // resource — the tests below keep the spec's three scenarios (plus
    // cooler / production-integration / ambient-trend coverage) in the
    // file's real idiom.

    #[test]
    fn test_thermal_emitter_increases_local_temperature() {
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 20.0));
        world.spawn((
            GridPosition { x: 5, y: 5 },
            ThermalEmitter { heat_output: 10.0 },
        ));
        world.run_system_once(thermal_emission_system).unwrap();
        let grid = world.resource::<TemperatureGrid>();
        assert!(
            grid.get(5, 5) > 20.0,
            "Heater should raise the local tile temperature"
        );
    }

    #[test]
    fn test_thermal_emitter_cooler_decreases_local_temperature() {
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 20.0));
        world.spawn((
            GridPosition { x: 5, y: 5 },
            ThermalEmitter { heat_output: -10.0 },
        ));
        world.run_system_once(thermal_emission_system).unwrap();
        let grid = world.resource::<TemperatureGrid>();
        assert!(
            grid.get(5, 5) < 20.0,
            "Cooler (negative output) should lower the local tile temperature"
        );
    }

    #[test]
    fn test_heat_diffuses_to_adjacent_tiles() {
        // No TerrainGrid inserted on purpose: the spec's isolated setup
        // has none, and the standalone diffusion leg must still run.
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(10, 10, 20.0);
        grid.set(5, 5, 100.0);
        world.insert_resource(grid);
        world
            .run_system_once(thermal_diffusion_system)
            .unwrap();
        let grid = world.resource::<TemperatureGrid>();
        assert!(grid.get(5, 5) < 100.0, "Center should shed heat");
        assert!(
            grid.get(5, 4) > 20.0,
            "Heat should diffuse to neighbors"
        );
    }

    #[test]
    fn test_insulation_blocks_diffusion() {
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(10, 10, 20.0);
        grid.set(5, 5, 100.0);
        world.insert_resource(grid);
        world.spawn((
            GridPosition { x: 5, y: 4 },
            ThermalInsulator {
                insulation_value: 0.9,
            },
        ));
        world
            .run_system_once(thermal_diffusion_system)
            .unwrap();
        let grid = world.resource::<TemperatureGrid>();
        let insulated_temp = grid.get(5, 4);
        let open_temp = grid.get(5, 6);
        assert!(
            insulated_temp < open_temp,
            "Insulator should resist temperature changes ({insulated_temp} vs {open_temp})"
        );
    }

    #[test]
    fn test_thermal_emitter_flows_through_production_temperature_system() {
        // The production seam: update_temperature_system itself must
        // apply ThermalEmitters (no separate schedule needed).
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 20.0));
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        // SeasonState::default is Spring (15C ambient): the system
        // recomputes ambient, so start the grid there to isolate the
        // emitter's contribution.
        world.resource_mut::<TemperatureGrid>().ambient = 15.0;
        world.resource_mut::<TemperatureGrid>().values.fill(15.0);
        world.spawn((
            GridPosition { x: 5, y: 5 },
            ThermalEmitter { heat_output: 10.0 },
        ));
        world.run_system_once(update_temperature_system).unwrap();
        let grid = world.resource::<TemperatureGrid>();
        assert!(
            grid.get(5, 5) > 15.0,
            "Production system should apply the emitter, got {}",
            grid.get(5, 5)
        );
    }

    #[test]
    fn test_thermal_insulator_flows_through_production_temperature_system() {
        let mut world = World::new();
        let mut grid = TemperatureGrid::new(10, 10, 20.0);
        grid.set(5, 5, 100.0);
        world.insert_resource(grid);
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.spawn((
            GridPosition { x: 5, y: 4 },
            ThermalInsulator {
                insulation_value: 0.9,
            },
        ));
        world.run_system_once(update_temperature_system).unwrap();
        let grid = world.resource::<TemperatureGrid>();
        assert!(
            grid.get(5, 4) < grid.get(5, 6),
            "Production diffusion should honor the insulator ({} vs {})",
            grid.get(5, 4),
            grid.get(5, 6)
        );
    }

    #[test]
    fn test_thermal_grid_trends_toward_ambient() {
        // Spec §5 ambient equilibrium: a hot grid with no sources must
        // cool toward ambient through the diffusion leg's drift.
        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 20.0));
        {
            let mut grid = world.resource_mut::<TemperatureGrid>();
            grid.values.fill(40.0);
        }
        for _ in 0..10 {
            world
                .run_system_once(thermal_diffusion_system)
                .unwrap();
        }
        let grid = world.resource::<TemperatureGrid>();
        assert!(
            grid.get(5, 5) < 40.0 && grid.get(5, 5) > 20.0,
            "Grid should trend toward ambient without overshooting, got {}",
            grid.get(5, 5)
        );
    }

    #[test]
    fn test_insulator_conductivity_clamps() {
        assert!((ThermalInsulator { insulation_value: 0.9 }.conductivity() - 0.1).abs() < 1e-6);
        assert_eq!(
            ThermalInsulator { insulation_value: 2.0 }.conductivity(),
            0.0,
            "Over-1.0 insulation clamps to a perfect insulator"
        );
        assert_eq!(
            ThermalInsulator { insulation_value: -1.0 }.conductivity(),
            1.0,
            "Negative insulation clamps to fully conductive"
        );
    }
}
