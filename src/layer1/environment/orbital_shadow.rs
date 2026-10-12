//! Spec 335: Orbital Shadow.
//!
//! Fantasy: the space infrastructure is so massive it blocks the sun.
//! Large Layer-2 orbital entities (fleets, stations) cast dynamic shadows
//! on the Layer-1 colony map, reducing solar power and temperature on
//! shaded tiles. Park your dreadnought squadron in orbit for repairs and
//! accidentally freeze your crops: defense positioning vs. planetary
//! agriculture.
//!
//! ## Mechanics
//! * Each [`OrbitalShadowCaster`] records a Layer-2 vessel's projected
//!   footprint: a center [`GridPosition`] on the colony map (where the
//!   vessel hovers), a pre-projection Layer-2 size, a peak block percent,
//!   and an orbital drift vector.
//! * [`project_orbital_shadows_system`] (Layer1SystemSet::Environment)
//!   drifts the casters and rebuilds the [`OrbitalShadowMap`] tile map
//!   with a gradient edge (100% of peak at the center, 50% at the edge).
//! * [`apply_orbital_shadows_system`] (Execution, after
//!   `update_lighting_system`, which resets the map to ambient each tick)
//!   multiplies shaded [`LightMap`][crate::layer1::lighting::LightMap]
//!   tiles and subtracts from
//!   [`TemperatureGrid`][crate::layer1::nature::temperature::TemperatureGrid].
//! * [`dim_shadowed_solar_system`] (Economy, after
//!   `update_solar_output_system`, before `power_grid_system`) dims
//!   [`SolarPower`][crate::layer1::nature::solar::SolarPower] panels
//!   standing under a shadow, since the real solar calc reads
//!   [`DayNightCycle`][crate::layer1::day_night::DayNightCycle], not the
//!   per-tile light map. Environment runs after Economy in the real
//!   schedule, so the shadow systems sit in the sets where their ordering
//!   edges can actually fire.
//! * The first time a major shadow falls on farmland, a Major chronicle
//!   entry warns the colony.
//!
//! ## Spec adaptations
//! * The sketch's mock `GridPosition` / `LightGrid` / `TemperatureGrid` /
//!   `OrbitalShadowMap` are adapted to the real types:
//!   [`GridPosition`][crate::layer1::core::map::GridPosition],
//!   [`LightMap`][crate::layer1::lighting::LightMap] (per-tile, reset to
//!   ambient each tick by `update_lighting_system`),
//!   [`TemperatureGrid`][crate::layer1::nature::temperature::TemperatureGrid]
//!   (per-tile, `values`), and a real `OrbitalShadowMap` resource holding
//!   the same `shadows: HashMap<GridPosition, f32>` block-percent shape.
//! * The sketch's `OrbitalEntity` with `Mass`/`Size` does not exist as a
//!   Layer-1-positioned type (real [`Fleet`][crate::layer2::fleet::Fleet]
//!   / [`InOrbit`][crate::layer2::fleet::InOrbit] live in Layer-2's
//!   abstract system view with no colony-grid position). Casters are
//!   anchored on the colony grid directly (the `shadow` console command
//!   parks one over the colony heart), reusing the colony-heart pattern
//!   from the deorbit spec.
//! * Shadow movement: casters drift by their `drift` vector each tick
//!   (wrapping around the map), shifting the projected shadow.
//! * §7's requested `Layer1SystemSet::Environment`-only placement is
//!   impossible verbatim — the real schedule runs Environment *after*
//!   Economy — so the three systems sit in the sets where their order
//!   constraints fire: projection in Environment, light application in
//!   Execution (after the lighting reset), solar dimming in Economy.
//! * All names are original and generic (see the IP-guard test).

use bevy::utils::HashMap;
use bevy_ecs::prelude::*;

use crate::layer1::agriculture::farm::Farm;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::energy::PowerSource;
use crate::layer1::lighting::LightMap;
use crate::layer1::nature::solar::SolarPower;
use crate::layer1::nature::temperature::TemperatureGrid;

/// Layer-2 vessel size (tiles) to Layer-1 shadow footprint scaling.
/// A 5-wide vessel throws a ~50-wide shadow (radius = size * scale / 2).
pub const ORBITAL_PROJECTION_SCALE: f32 = 10.0;
/// Block percent at or above which a tile counts as "shadowed" for STATS.
pub const SHADOWED_TILE_THRESHOLD: f32 = 50.0;
/// Temperature drop (Celsius) per percent blocked, per tick.
pub const TEMP_DROP_PER_BLOCK_PERCENT: f32 = 0.1;
/// Block percent at or above which a farm tile triggers the farmland chronicle.
pub const FARMLAND_CHRONICLE_BLOCK: f32 = 50.0;

/// A Layer-2 vessel parked over the colony, casting a shadow.
///
/// The real Layer-2 [`Fleet`][crate::layer2::fleet::Fleet] /
/// [`InOrbit`][crate::layer2::fleet::InOrbit] entities live in the abstract
/// system view with no colony-grid position, so the caster records the
/// projected Layer-1 footprint directly.
#[derive(Debug, Clone, Copy)]
pub struct OrbitalShadowCaster {
    /// Vessel class name (original flavor, e.g. "dreadnought squadron").
    pub name: &'static str,
    /// Layer-1 tile the vessel hovers above (the shadow center).
    pub center: GridPosition,
    /// Layer-2 vessel footprint in tiles (pre-projection).
    pub layer2_size: u32,
    /// Peak light block at the shadow center (percent, 0.0-100.0).
    pub peak_block: f32,
    /// Orbital drift per tick (tiles); the shadow crawls across the map.
    pub drift: (i32, i32),
    /// One-shot: the farmland chronicle was already sent for this caster.
    pub farmland_chronicled: bool,
}

impl OrbitalShadowCaster {
    /// Projected shadow radius in Layer-1 tiles.
    #[must_use]
    pub fn shadow_radius(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let size = self.layer2_size as f32;
        size * ORBITAL_PROJECTION_SCALE / 2.0
    }

    /// Gradient block percent at `tile`: 100% of peak at the center,
    /// 50% of peak at the edge, 0 beyond the radius.
    #[must_use]
    pub fn block_at(&self, tile: GridPosition) -> f32 {
        let radius = self.shadow_radius();
        if radius <= 0.0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let dist = ((tile.x - self.center.x) as f32).hypot((tile.y - self.center.y) as f32);
        if dist > radius {
            return 0.0;
        }
        self.peak_block * (1.0 - 0.5 * dist / radius)
    }
}

/// Per-tile shadow block percent (0.0-100.0), rebuilt from casters each tick.
#[derive(Resource, Default)]
pub struct OrbitalShadowMap {
    /// Maps a tile to its light-block percent (0.0 = clear, 100.0 = total).
    pub shadows: HashMap<GridPosition, f32>,
}

/// The live shadow casters (drifted and recomputed each tick).
#[derive(Resource, Default)]
pub struct OrbitalShadowCasters {
    /// Active casters.
    pub casters: Vec<OrbitalShadowCaster>,
}

/// Recompute the shadow map: drift casters, rebuild the gradient tile map,
/// and chronicle the first major shadow over farmland.
pub fn project_orbital_shadows_system(
    casters: Option<ResMut<OrbitalShadowCasters>>,
    shadow_map: Option<ResMut<OrbitalShadowMap>>,
    light_map: Res<LightMap>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    farms: Query<&GridPosition, With<Farm>>,
) {
    // Optional resources: test worlds and minimal schedules that never
    // parked a shadow simply have no casters to project.
    let (Some(mut casters), Some(mut shadow_map)) = (casters, shadow_map) else {
        return;
    };
    // Map bounds for drift wrapping (origin at 0,0).
    let width = light_map.width.max(1);
    let height = light_map.height.max(1);
    let farm_positions: Vec<GridPosition> = farms.iter().copied().collect();

    shadow_map.shadows.clear();
    for caster in casters.casters.iter_mut() {
        // Orbital drift: the shadow crawls across the grid, wrapping around.
        #[allow(clippy::cast_possible_wrap)]
        let (w, h) = (width as i32, height as i32);
        caster.center = GridPosition {
            x: (caster.center.x + caster.drift.0).rem_euclid(w),
            y: (caster.center.y + caster.drift.1).rem_euclid(h),
        };

        let radius = caster.shadow_radius();
        #[allow(clippy::cast_possible_truncation)]
        let reach = radius.ceil() as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let tile = GridPosition {
                    x: caster.center.x + dx,
                    y: caster.center.y + dy,
                };
                if tile.x < 0 || tile.y < 0 {
                    continue;
                }
                let block = caster.block_at(tile);
                if block <= 0.0 {
                    continue;
                }
                shadow_map
                    .shadows
                    .entry(tile)
                    .and_modify(|b| *b = b.max(block).min(100.0))
                    .or_insert(block.min(100.0));
            }
        }

        // One-shot Major chronicle when a major shadow first falls on farmland.
        if !caster.farmland_chronicled
            && farm_positions
                .iter()
                .any(|farm_pos| caster.block_at(*farm_pos) >= FARMLAND_CHRONICLE_BLOCK)
        {
            caster.farmland_chronicled = true;
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "ORBITAL SHADOW: the {} has blotted out the sun over the farmlands. \
                     Crops shiver in the sudden cold — move the fleet or watch the harvest die.",
                    caster.name
                ),
                importance: EventImportance::Major,
            });
        }
    }
}

/// Apply shadows to the light and temperature grids.
///
/// Must run after `update_lighting_system` (which resets the map to
/// ambient each tick). Sparse/missing entries are skipped.
pub fn apply_orbital_shadows_system(
    shadow_map: Option<Res<OrbitalShadowMap>>,
    mut light_map: ResMut<LightMap>,
    mut temp_grid: ResMut<TemperatureGrid>,
) {
    // Optional resource: worlds without an OrbitalShadowMap get no shadows.
    let Some(shadow_map) = shadow_map else {
        return;
    };
    if shadow_map.shadows.is_empty() {
        return;
    }
    for (pos, block) in shadow_map.shadows.iter() {
        let block = block.clamp(0.0, 100.0);
        if block <= 0.0 || pos.x < 0 || pos.y < 0 {
            continue;
        }
        // Sparse entries outside the real grids are skipped, never created.
        if (pos.x as u32) < light_map.width && (pos.y as u32) < light_map.height {
            let light = light_map.get(pos.x as u32, pos.y as u32);
            light_map.set(pos.x as u32, pos.y as u32, light * (1.0 - block / 100.0));
        }
        let temp = temp_grid.get(pos.x as usize, pos.y as usize);
        temp_grid.set(
            pos.x as usize,
            pos.y as usize,
            temp - block * TEMP_DROP_PER_BLOCK_PERCENT,
        );
    }
}

/// Dim solar panels standing under a shadow.
///
/// The real `update_solar_output_system` reads `DayNightCycle`, not the
/// per-tile light map, so this bridges shadows into solar generation. Runs
/// after it and before `power_grid_system`.
pub fn dim_shadowed_solar_system(
    shadow_map: Option<Res<OrbitalShadowMap>>,
    mut panels: Query<(&mut PowerSource, &SolarPower, &GridPosition)>,
) {
    let Some(shadow_map) = shadow_map else {
        return;
    };
    if shadow_map.shadows.is_empty() {
        return;
    }
    for (mut source, _solar, pos) in &mut panels {
        if let Some(block) = shadow_map.shadows.get(pos) {
            let block = block.clamp(0.0, 100.0);
            if block > 0.0 {
                source.output *= 1.0 - block / 100.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn test_world() -> World {
        let mut world = World::new();
        let mut light = LightMap::new(80, 50);
        light.tiles.fill(1.0);
        world.insert_resource(light);
        world.insert_resource(TemperatureGrid::new(80, 50, 20.0));
        world.insert_resource(OrbitalShadowMap::default());
        world.insert_resource(OrbitalShadowCasters::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    #[test]
    fn test_orbital_shadow_blocks_light() {
        let mut world = test_world();
        world
            .resource_mut::<OrbitalShadowMap>()
            .shadows
            .insert(GridPosition { x: 10, y: 10 }, 50.0); // 50% block
        world.run_system_once(apply_orbital_shadows_system).unwrap();
        let light = world.resource::<LightMap>().get(10, 10);
        assert!(light < 1.0, "Shadow should reduce light, got {light}");
        assert!((light - 0.5).abs() < 1e-6, "50% block of 1.0 -> 0.5, got {light}");
        // Untouched tiles stay lit.
        assert!((world.resource::<LightMap>().get(11, 11) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_orbital_shadow_drops_temperature() {
        let mut world = test_world();
        world
            .resource_mut::<OrbitalShadowMap>()
            .shadows
            .insert(GridPosition { x: 10, y: 10 }, 50.0); // 50% block
        world.run_system_once(apply_orbital_shadows_system).unwrap();
        let temp = world.resource::<TemperatureGrid>().get(10, 10);
        assert!(temp < 20.0, "Shadow should drop temperature, got {temp}");
        assert!((temp - 15.0).abs() < 1e-4, "50% block -> -5C, got {temp}");
        // Untouched tiles keep ambient.
        assert!((world.resource::<TemperatureGrid>().get(11, 11) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_shadow_gradient_center_and_edge() {
        let caster = OrbitalShadowCaster {
            name: "test",
            center: GridPosition { x: 0, y: 0 },
            layer2_size: 2, // radius = 2 * 10 / 2 = 10
            peak_block: 100.0,
            drift: (0, 0),
            farmland_chronicled: false,
        };
        assert!((caster.shadow_radius() - 10.0).abs() < 1e-6);
        assert!((caster.block_at(GridPosition { x: 0, y: 0 }) - 100.0).abs() < 1e-6);
        // Edge: 50% of peak.
        assert!((caster.block_at(GridPosition { x: 10, y: 0 }) - 50.0).abs() < 1e-6);
        // Beyond the radius: nothing.
        assert_eq!(caster.block_at(GridPosition { x: 11, y: 0 }), 0.0);
    }

    #[test]
    fn test_orbital_projection_scale() {
        // §7: a 5-wide vessel throws a ~50-wide shadow.
        let caster = OrbitalShadowCaster {
            name: "test",
            center: GridPosition { x: 0, y: 0 },
            layer2_size: 5,
            peak_block: 80.0,
            drift: (0, 0),
            farmland_chronicled: false,
        };
        assert!((caster.shadow_radius() * 2.0 - 5.0 * ORBITAL_PROJECTION_SCALE).abs() < 1e-6);
    }

    #[test]
    fn test_shadow_drift_moves_center() {
        let mut world = test_world();
        world
            .resource_mut::<OrbitalShadowCasters>()
            .casters
            .push(OrbitalShadowCaster {
                name: "test",
                center: GridPosition { x: 10, y: 10 },
                layer2_size: 2,
                peak_block: 100.0,
                drift: (1, 0),
                farmland_chronicled: false,
            });
        world.run_system_once(project_orbital_shadows_system).unwrap();
        let casters = world.resource::<OrbitalShadowCasters>();
        assert_eq!(casters.casters[0].center, GridPosition { x: 11, y: 10 });
        let map = world.resource::<OrbitalShadowMap>();
        // The tile map was rebuilt around the drifted center.
        assert!(map.shadows[&GridPosition { x: 11, y: 10 }] > 99.0);
    }

    #[test]
    fn test_shadowed_solar_panels_dim() {
        let mut world = test_world();
        let panel = world
            .spawn((
                crate::layer1::energy::PowerSource {
                    output: 10.0,
                    active: true,
                },
                SolarPower { base_output: 10.0 },
                GridPosition { x: 10, y: 10 },
            ))
            .id();
        world
            .resource_mut::<OrbitalShadowMap>()
            .shadows
            .insert(GridPosition { x: 10, y: 10 }, 50.0);
        world.run_system_once(dim_shadowed_solar_system).unwrap();
        let output = world.get::<PowerSource>(panel).unwrap().output;
        assert!((output - 5.0).abs() < 1e-6, "50% shadow -> half output, got {output}");
    }

    #[test]
    fn test_ip_guard_orbital_shadow() {
        let src = include_str!("orbital_shadow.rs");
        // concat! keeps the full terms out of this file's own text, so the
        // include_str! scan never self-matches the ban list.
        for banned in [
            concat!("Vo", "gon"),
            concat!("Heart of ", "Gold"),
            concat!("Sh", "rike"),
            concat!("Shi", "mmer"),
            concat!("Bene Gesse", "rit"),
            concat!("Mel", "lon"),
        ] {
            assert!(
                !src.contains(banned),
                "banned franchise term {banned:?} in orbital_shadow.rs"
            );
        }
    }

    #[test]
    fn test_farmland_shadow_chronicled_once() {
        let mut world = test_world();
        // A farm right under where the shadow will fall.
        world.spawn((Farm::default(), GridPosition { x: 20, y: 20 }));
        world
            .resource_mut::<OrbitalShadowCasters>()
            .casters
            .push(OrbitalShadowCaster {
                name: "dreadnought squadron",
                center: GridPosition { x: 20, y: 20 },
                layer2_size: 2,
                peak_block: 100.0,
                drift: (0, 0),
                farmland_chronicled: false,
            });
        world.run_system_once(project_orbital_shadows_system).unwrap();
        let sent: Vec<AddChronicleEvent> = {
            let mut events = world.resource_mut::<Events<AddChronicleEvent>>();
            events.drain().collect()
        };
        assert!(
            sent.iter()
                .any(|e| e.importance == EventImportance::Major && e.text.contains("farmland")),
            "major shadow on farmland should chronicle a Major event, got {sent:?}"
        );
        assert!(world.resource::<OrbitalShadowCasters>().casters[0].farmland_chronicled);
        // Second run: no repeat chronicle.
        world.run_system_once(project_orbital_shadows_system).unwrap();
        let sent2: Vec<AddChronicleEvent> = {
            let mut events = world.resource_mut::<Events<AddChronicleEvent>>();
            events.drain().collect()
        };
        assert!(sent2.is_empty(), "farmland chronicle must be one-shot");
    }
}

