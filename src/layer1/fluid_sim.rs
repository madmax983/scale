//! Spec 412 — Fluid Simulation (Layer 1).
//!
//! Water and magma as a dynamic, conserved *volume* per tile — not a
//! static terrain paint. Fluid seeks its own level: every tick each
//! tile sheds a fraction of its level difference into lower adjacent
//! tiles, so a breached tank floods a room, a dug channel drains a
//! pond into a moat, and a punctured aquifer keeps coming until the
//! levels equalize.
//!
//! # Grounding (adaptations from the spec sketch)
//!
//! The spec's sketch types were mocks; the real seams are:
//!
//! - **Position**: the spec invented `TilePos`; the real one already
//!   exists at [`crate::layer1::map::TilePos`] (with
//!   [`GridPosition`](crate::layer1::map::GridPosition) for entities).
//!   Both are used as-is — no parallel coordinate type.
//! - **Volume store**: the spec §5 refactor note directs a grid
//!   resource instead of thousands of `Fluid` component entities.
//!   That is also the codebase pattern ([`WaterGrid`], `PressureGrid`,
//!   `BeautyGrid` are all resources), so the authoritative store is
//!   [`FluidGrid`]. The spec's per-tile `Fluid { volume, fluid_type }`
//!   record survives as [`Fluid`] — the value type a tile holds.
//! - **Not [`WaterGrid`]**: Spec 096's grid is *hydration* (0–100,
//!   recomputed from scratch every tick from `TerrainType::Water`
//!   and wells, for soil moisture). It holds no volume and conserves
//!   nothing, so extending it would corrupt both semantics. The two
//!   grids sit side by side: hydration feeds farms, volume floods.
//! - **"Lower tiles"**: the terrain has no elevation field —
//!   [`TerrainGrid`] is flat, and `ZLevel` exists only on entities
//!   (gravity chutes). "Lower" therefore means *lower fluid level*:
//!   flow is level-seeking equalization, exactly what the spec's RED
//!   tests assert. Pumps (below) are the against-the-gradient leg.
//! - **Dams**: no `Dam`/`Floodgate`/`Pump` `BuildingType` exists, and
//!   adding one touches ~15 exhaustive matches. The spec's `Dam`
//!   becomes [`Floodgate`], a component in the `WaterSource`
//!   precedent (extra components attached at a [`GridPosition`]).
//!   Real buildings still block flow through the *existing* seam:
//!   any building whose
//!   [`BuildingType::flow_transmissivity`](crate::layer1::building::BuildingType::flow_transmissivity)
//!   is `Some(0.0)` (Wall / Window / Airlock) is a fluid blocker, and
//!   solid terrain (Rock / DeepRock / MagmaRock / Void /
//!   IndestructibleStump / Artifact) never accepts fluid.
//! - **Pumps**: level/uphill movement needs power — the gravity
//!   logistics rule (Spec 1372). A [`FluidPump`] only moves volume
//!   while its real [`PowerConsumer`] is `active`.
//! - **Hazards**: no drowning system existed. [`fluid_hazard_system`]
//!   bridges onto the real [`Health`] component: deep water drowns,
//!   magma burns (half the MagmaRock geome rate — standing *in*
//!   magma fluid vs. on solid magma rock). A vanilla world holds
//!   zero fluid volume, so no existing balance moves.
//!
//! Conservation: flow and pumps only *move* volume. Volume enters
//! only when explicitly added (headless `fluid`, aquifer puncture)
//! and never leaves except through `FluidGrid::remove_fluid`.

use crate::layer1::building::Building;
use crate::layer1::energy::PowerConsumer;
use crate::layer1::health::Health;
use crate::layer1::map::{GridPosition, TilePos};
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// Volume at which a tile reads as "full" (the spec's spawn value).
pub const FULL_VOLUME: f32 = 10.0;
/// Fraction of a level difference shed per tile pair per tick.
pub const FLOW_FRACTION: f32 = 0.2;
/// Water volume at or above which a standing pop drowns.
pub const DEEP_WATER_VOLUME: f32 = 7.0;
/// Drowning damage per tick in deep water.
pub const DROWN_DAMAGE: f32 = 1.0;
/// Magma volume at or above which a standing pop burns.
pub const MAGMA_BURN_VOLUME: f32 = 0.5;
/// Burn damage per tick standing in magma fluid.
pub const MAGMA_DAMAGE: f32 = 5.0;
/// Default volume a pump moves per tick while powered.
pub const PUMP_RATE: f32 = 2.0;
/// Water volume released by one aquifer puncture.
pub const AQUIFER_BURST: f32 = 10.0;

/// The kind of fluid a tile holds (spec §4 `FluidType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FluidType {
    /// Ordinary water: floods, fills moats, drowns at depth.
    #[default]
    Water,
    /// Molten rock: floods slowly the same way, burns on contact.
    Magma,
}

impl FluidType {
    /// Generic display name (IP guard: generic words only).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Water => "Water",
            Self::Magma => "Magma",
        }
    }
}

/// The fluid record of a single tile (spec §4 `Fluid`): how much of
/// what is pooled here. Stored inside [`FluidGrid`], per §5.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fluid {
    /// Volume units pooled on the tile (0.0 = dry).
    pub volume: f32,
    /// Which fluid is pooled (meaningful only while volume > 0).
    pub fluid_type: FluidType,
}

impl Default for Fluid {
    fn default() -> Self {
        Self {
            volume: 0.0,
            fluid_type: FluidType::Water,
        }
    }
}

/// A floodgate on a tile: while shut it is a fluid dam (the spec's
/// `Dam`); while open fluid passes as if it were not there. Attach
/// at a [`GridPosition`], `WaterSource`-style.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floodgate {
    /// Whether the gate is open (fluid may pass).
    pub open: bool,
}

impl Floodgate {
    /// A shut (blocking) floodgate.
    #[must_use]
    pub const fn shut() -> Self {
        Self { open: false }
    }
}

/// A powered pump: moves up to `rate` volume per tick from its own
/// tile to `target`, against the level gradient — but only while
/// its sibling [`PowerConsumer`] is active (Spec 1372's rule).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct FluidPump {
    /// Tile the pump discharges into.
    pub target: TilePos,
    /// Volume moved per tick while powered.
    pub rate: f32,
}

impl Default for FluidPump {
    fn default() -> Self {
        Self {
            target: TilePos::new(0, 0),
            rate: PUMP_RATE,
        }
    }
}

/// Registry of aquifer tiles: pressurized underground water waiting
/// under a tile. Puncturing one ([`puncture_aquifer`]) releases a
/// burst of water onto that tile — dig carefully.
#[derive(Resource, Default)]
pub struct AquiferMap {
    /// Tiles hiding an aquifer, as (x, y).
    pub tiles: HashSet<(i32, i32)>,
}

/// Authoritative per-tile fluid volumes for the whole map.
///
/// Double-buffered like `PressureGrid`: flow computes per-tile
/// deltas from a read snapshot, then applies them, so no tile's
/// update depends on iteration order. Total volume is conserved
/// by flow and pumps.
#[derive(Resource)]
pub struct FluidGrid {
    /// Width of the grid in cells.
    pub width: usize,
    /// Height of the grid in cells.
    pub height: usize,
    volumes: Vec<f32>,
    kinds: Vec<FluidType>,
    deltas: Vec<f32>,
    incoming_kinds: Vec<Option<FluidType>>,
}

impl Default for FluidGrid {
    fn default() -> Self {
        // The colony map is 80x50 (setup.rs inserts the same size
        // explicitly); Default exists for init_resource callers.
        Self::new(80, 50)
    }
}

impl FluidGrid {
    /// Creates an empty (dry) fluid grid.
    ///
    /// # Panics
    /// Panics if `width * height` overflows or exceeds 10_000_000,
    /// matching the `WaterGrid` guard.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let size = width
            .checked_mul(height)
            .expect("Grid size overflow or too large");
        assert!(size <= 10_000_000, "Grid size overflow or too large");
        Self {
            width,
            height,
            volumes: vec![0.0; size],
            kinds: vec![FluidType::Water; size],
            deltas: vec![0.0; size],
            incoming_kinds: vec![None; size],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 {
            return None;
        }
        let (ux, uy) = (x as usize, y as usize);
        if ux >= self.width || uy >= self.height {
            return None;
        }
        uy.checked_mul(self.width)
            .and_then(|i| i.checked_add(ux))
            .filter(|&i| i < self.volumes.len())
    }

    /// The fluid record at (x, y); dry out of bounds.
    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> Fluid {
        match self.index(x, y) {
            Some(idx) => Fluid {
                volume: self.volumes[idx],
                fluid_type: self.kinds[idx],
            },
            None => Fluid::default(),
        }
    }

    /// Volume at (x, y); 0.0 out of bounds.
    #[must_use]
    pub fn volume(&self, x: i32, y: i32) -> f32 {
        self.get(x, y).volume
    }

    /// Sets the fluid on a tile outright (negative clamps to dry).
    pub fn set_fluid(&mut self, x: i32, y: i32, volume: f32, fluid_type: FluidType) {
        if let Some(idx) = self.index(x, y) {
            self.volumes[idx] = volume.max(0.0);
            if volume > 0.0 {
                self.kinds[idx] = fluid_type;
            }
        }
    }

    /// Adds volume of a kind to a tile. Adding a *different* kind to
    /// a wet tile is refused (fluids do not mix; returns 0.0).
    /// Returns the volume actually added.
    pub fn add_fluid(&mut self, x: i32, y: i32, volume: f32, fluid_type: FluidType) -> f32 {
        if volume <= 0.0 {
            return 0.0;
        }
        let Some(idx) = self.index(x, y) else {
            return 0.0;
        };
        if self.volumes[idx] > 0.0 && self.kinds[idx] != fluid_type {
            return 0.0;
        }
        self.kinds[idx] = fluid_type;
        self.volumes[idx] += volume;
        volume
    }

    /// Removes up to `volume` from a tile; returns what was removed.
    pub fn remove_fluid(&mut self, x: i32, y: i32, volume: f32) -> f32 {
        let Some(idx) = self.index(x, y) else {
            return 0.0;
        };
        let removed = volume.min(self.volumes[idx]).max(0.0);
        self.volumes[idx] -= removed;
        removed
    }

    /// Total volume of all fluids on the map.
    #[must_use]
    pub fn total_volume(&self) -> f32 {
        self.volumes.iter().sum()
    }

    /// Total volume of one fluid kind.
    #[must_use]
    pub fn total_of(&self, kind: FluidType) -> f32 {
        self.volumes
            .iter()
            .zip(self.kinds.iter())
            .filter(|(_, k)| **k == kind)
            .map(|(v, _)| v)
            .sum()
    }

    /// Number of tiles holding any fluid.
    #[must_use]
    pub fn flooded_tiles(&self) -> usize {
        self.volumes.iter().filter(|v| **v > 0.0).count()
    }

    /// The deepest pool on the map, if any tile is wet.
    #[must_use]
    pub fn deepest(&self) -> Option<(TilePos, Fluid)> {
        let mut best: Option<(usize, f32)> = None;
        for (idx, v) in self.volumes.iter().enumerate() {
            if *v > 0.0 && best.is_none_or(|(_, bv)| *v > bv) {
                best = Some((idx, *v));
            }
        }
        best.map(|(idx, v)| {
            let x = (idx % self.width) as i32;
            let y = (idx / self.width) as i32;
            (
                TilePos::new(x, y),
                Fluid {
                    volume: v,
                    fluid_type: self.kinds[idx],
                },
            )
        })
    }
}

/// Whether terrain is a hard solid that never accepts fluid.
/// Mirrors the unwalkable-solid family in `TerrainType::is_walkable`
/// minus water (a Water tile is a basin, not a blocker).
fn terrain_blocks_fluid(terrain: TerrainType) -> bool {
    matches!(
        terrain,
        TerrainType::Rock
            | TerrainType::DeepRock
            | TerrainType::MagmaRock
            | TerrainType::Void
            | TerrainType::IndestructibleStump
            | TerrainType::Artifact
    )
}

/// Builds the per-tile blocked map for one flow step: solid terrain,
/// fully-sealed buildings (Wall/Window/Airlock via the real
/// `flow_transmissivity` seam), and shut floodgates.
fn build_blocked(
    grid: &FluidGrid,
    terrain: Option<&TerrainGrid>,
    floodgates: &Query<(&GridPosition, &Floodgate)>,
    buildings: &Query<(&Building, &GridPosition)>,
) -> Vec<bool> {
    let mut blocked = vec![false; grid.volumes.len()];
    if let Some(terrain) = terrain {
        for y in 0..grid.height {
            for x in 0..grid.width {
                if let Some(t) = terrain.get(x, y) {
                    if terrain_blocks_fluid(t) {
                        if let Some(idx) = grid.index(x as i32, y as i32) {
                            blocked[idx] = true;
                        }
                    }
                }
            }
        }
    }
    for (building, pos) in buildings {
        if building.building_type.flow_transmissivity() == Some(0.0) {
            if let Some(idx) = grid.index(pos.x, pos.y) {
                blocked[idx] = true;
            }
        }
    }
    for (pos, gate) in floodgates {
        if !gate.open {
            if let Some(idx) = grid.index(pos.x, pos.y) {
                blocked[idx] = true;
            }
        }
    }
    blocked
}

/// Level-seeking flow: each wet tile sheds [`FLOW_FRACTION`] of its
/// level difference into each lower, unblocked, kind-compatible
/// neighbor. Double-buffered via per-tile deltas; volume conserved.
pub fn fluid_flow_system(
    mut grid: ResMut<FluidGrid>,
    terrain: Option<Res<TerrainGrid>>,
    floodgates: Query<(&GridPosition, &Floodgate)>,
    buildings: Query<(&Building, &GridPosition)>,
) {
    let blocked = build_blocked(&grid, terrain.as_deref(), &floodgates, &buildings);
    grid.deltas.fill(0.0);
    grid.incoming_kinds.fill(None);
    let (width, height) = (grid.width, grid.height);
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let volume = grid.volumes[idx];
            if volume <= 0.0 || blocked[idx] {
                continue;
            }
            let kind = grid.kinds[idx];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                let Some(nidx) = grid.index(nx, ny) else {
                    continue;
                };
                if blocked[nidx] {
                    continue;
                }
                let nvolume = grid.volumes[nidx];
                if nvolume > 0.0 && grid.kinds[nidx] != kind {
                    continue; // fluids do not mix
                }
                let diff = volume - nvolume;
                if diff <= 0.0 {
                    continue;
                }
                let flow = diff * FLOW_FRACTION;
                grid.deltas[idx] -= flow;
                grid.deltas[nidx] += flow;
                if nvolume <= 0.0 {
                    grid.incoming_kinds[nidx] = Some(kind);
                }
            }
        }
    }
    for idx in 0..grid.volumes.len() {
        if grid.deltas[idx] != 0.0 {
            grid.volumes[idx] = (grid.volumes[idx] + grid.deltas[idx]).max(0.0);
            if grid.volumes[idx] > 0.0 {
                if let Some(kind) = grid.incoming_kinds[idx] {
                    grid.kinds[idx] = kind;
                }
            }
        }
    }
}

/// Powered pumps move fluid against the gradient (see [`FluidPump`]).
/// Unpowered pumps — or pumps with no [`PowerConsumer`] at all — sit
/// idle, exactly the gravity-logistics rule.
pub fn fluid_pump_system(
    mut grid: ResMut<FluidGrid>,
    pumps: Query<(&GridPosition, &FluidPump, Option<&PowerConsumer>)>,
    terrain: Option<Res<TerrainGrid>>,
) {
    let mut moves: Vec<(i32, i32, i32, i32, f32, FluidType)> = Vec::new();
    for (pos, pump, power) in &pumps {
        if !power.is_some_and(|p| p.active) {
            continue;
        }
        let source = grid.get(pos.x, pos.y);
        if source.volume <= 0.0 {
            continue;
        }
        if let Some(terrain) = terrain.as_deref() {
            if let (Ok(ux), Ok(uy)) = (
                usize::try_from(pump.target.x),
                usize::try_from(pump.target.y),
            ) {
                if terrain.get(ux, uy).is_some_and(terrain_blocks_fluid) {
                    continue;
                }
            }
        }
        let amount = pump.rate.min(source.volume);
        if amount > 0.0 {
            moves.push((
                pos.x,
                pos.y,
                pump.target.x,
                pump.target.y,
                amount,
                source.fluid_type,
            ));
        }
    }
    for (sx, sy, tx, ty, amount, kind) in moves {
        // Re-check the target accepts this kind before draining.
        let target = grid.get(tx, ty);
        if target.volume > 0.0 && target.fluid_type != kind {
            continue;
        }
        let removed = grid.remove_fluid(sx, sy, amount);
        grid.add_fluid(tx, ty, removed, kind);
    }
}

/// Punctures the aquifer under (x, y), if one is registered: a burst
/// of water floods the tile and the aquifer is spent. Returns true
/// when a burst happened. Emergence leg of the spec: dig into the
/// wrong cavern and the mine floods — volume then obeys flow.
pub fn puncture_aquifer(
    grid: &mut FluidGrid,
    aquifers: &mut AquiferMap,
    x: i32,
    y: i32,
) -> bool {
    if !aquifers.tiles.remove(&(x, y)) {
        return false;
    }
    grid.add_fluid(x, y, AQUIFER_BURST, FluidType::Water);
    true
}

/// Standing in fluid is dangerous: deep water drowns, magma burns.
/// Bridges onto the real [`Health`] component; death itself is left
/// to the existing death pipeline (this only deals damage).
pub fn fluid_hazard_system(
    grid: Res<FluidGrid>,
    mut pops: Query<(&GridPosition, &mut Health)>,
) {
    for (pos, mut health) in &mut pops {
        let fluid = grid.get(pos.x, pos.y);
        if fluid.volume <= 0.0 {
            continue;
        }
        let damage = match fluid.fluid_type {
            FluidType::Magma if fluid.volume >= MAGMA_BURN_VOLUME => MAGMA_DAMAGE,
            FluidType::Water if fluid.volume >= DEEP_WATER_VOLUME => DROWN_DAMAGE,
            _ => 0.0,
        };
        if damage > 0.0 {
            health.current = (health.current - damage).max(0.0);
        }
    }
}

/// One-line census for the headless STATS line (`fluids=` field).
#[must_use]
pub fn fluid_stats_label(world: &mut World) -> String {
    let Some(grid) = world.get_resource::<FluidGrid>() else {
        return "dry".to_string();
    };
    let water = grid.total_of(FluidType::Water);
    let magma = grid.total_of(FluidType::Magma);
    let tiles = grid.flooded_tiles();
    if tiles == 0 {
        return "dry".to_string();
    }
    let gates = {
        let mut q = world.query::<&Floodgate>();
        q.iter(world).count()
    };
    let pumps = {
        let mut q = world.query::<&FluidPump>();
        q.iter(world).count()
    };
    let aquifers = world.get_resource::<AquiferMap>().map_or(0, |a| a.tiles.len());
    format!("water={water:.1} magma={magma:.1} tiles={tiles} gates={gates} pumps={pumps} aquifers={aquifers}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::BuildingType;
    use bevy_ecs::system::RunSystemOnce;

    fn world_with_grid(width: usize, height: usize) -> World {
        let mut world = World::new();
        world.insert_resource(FluidGrid::new(width, height));
        world.insert_resource(TerrainGrid {
            width,
            height,
            tiles: vec![TerrainType::Grass; width * height],
        });
        world
    }

    // Spec RED test 1, adapted: Fluid component -> FluidGrid tile
    // record (spec §5 grid refactor); TilePos is the real map type.
    #[test]
    fn test_fluid_volume_exists_on_tile() {
        let mut world = world_with_grid(10, 10);
        let pos = TilePos::new(3, 4);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(pos.x, pos.y, 10.0, FluidType::Water);
        // No flow system registered (mirrors the spec test's App,
        // which updates without flow): the record simply holds.
        let fluid = world.resource::<FluidGrid>().get(pos.x, pos.y);
        assert_eq!(fluid.volume, 10.0, "Fluid should hold volume on its tile");
        assert_eq!(fluid.fluid_type, FluidType::Water);
    }

    // Spec RED test 2, adapted: flow equalizes adjacent tiles and
    // conserves total volume.
    #[test]
    fn test_fluid_flows_to_adjacent_empty_tiles() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        let volume_a = grid.volume(0, 0);
        let volume_b = grid.volume(1, 0);
        assert!(volume_a < 10.0, "Fluid should leave origin tile");
        assert!(volume_b > 0.0, "Fluid should flow into adjacent empty tile");
        assert!(
            (grid.total_volume() - 10.0).abs() < 0.001,
            "Flow must conserve volume, got {}",
            grid.total_volume()
        );
    }

    // Spec RED test 3, adapted: the spec's Dam -> a shut Floodgate.
    #[test]
    fn test_dams_block_fluid_flow() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        world.spawn((GridPosition { x: 1, y: 0 }, Floodgate::shut()));
        // The spec's world is 1D (two tiles); here the source's only
        // other neighbor must be dammed too, or fluid honestly flows
        // around the gate — which is correct sim behavior.
        world.spawn((GridPosition { x: 0, y: 1 }, Floodgate::shut()));
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.volume(0, 0), 10.0, "Fluid should NOT flow past a shut floodgate");
        assert_eq!(grid.volume(1, 0), 0.0, "Dam tile should not receive fluid");
    }

    #[test]
    fn test_open_floodgate_lets_fluid_pass() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        world.spawn((GridPosition { x: 1, y: 0 }, Floodgate { open: true }));
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert!(grid.volume(1, 0) > 0.0, "Open gate tile should receive fluid");
    }

    #[test]
    fn test_wall_building_blocks_fluid_flow() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        world.spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 1, y: 0 },
        ));
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.volume(1, 0), 0.0, "A real Wall building must block fluid");
    }

    #[test]
    fn test_rock_terrain_blocks_fluid_flow() {
        let mut world = world_with_grid(10, 10);
        world.resource_mut::<TerrainGrid>().tiles[1] = TerrainType::Rock; // (1,0)
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.volume(1, 0), 0.0, "Rock should not receive fluid");
    }

    #[test]
    fn test_fluid_equalizes_and_conserves_over_many_ticks() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(0, 0, 10.0, FluidType::Water);
        for _ in 0..200 {
            world.run_system_once(fluid_flow_system).unwrap();
        }
        let grid = world.resource::<FluidGrid>();
        assert!(
            (grid.total_volume() - 10.0).abs() < 0.01,
            "Volume must be conserved over 200 ticks, got {}",
            grid.total_volume()
        );
        assert!(
            (grid.volume(0, 0) - grid.volume(1, 0)).abs() < 0.5,
            "Adjacent tiles should near-equalize: {} vs {}",
            grid.volume(0, 0),
            grid.volume(1, 0)
        );
    }

    #[test]
    fn test_water_and_magma_do_not_mix() {
        let mut world = world_with_grid(10, 10);
        {
            let mut grid = world.resource_mut::<FluidGrid>();
            grid.set_fluid(0, 0, 10.0, FluidType::Water);
            grid.set_fluid(1, 0, 10.0, FluidType::Magma);
        }
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.total_of(FluidType::Water), 10.0);
        assert_eq!(grid.total_of(FluidType::Magma), 10.0);
        assert_eq!(grid.get(0, 0).fluid_type, FluidType::Water);
        assert_eq!(grid.get(1, 0).fluid_type, FluidType::Magma);
    }

    #[test]
    fn test_magma_flows_like_water() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(5, 5, 10.0, FluidType::Magma);
        world.run_system_once(fluid_flow_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert!(grid.volume(5, 5) < 10.0);
        assert!(grid.volume(6, 5) > 0.0);
        assert_eq!(grid.get(6, 5).fluid_type, FluidType::Magma);
    }

    #[test]
    fn test_powered_pump_moves_fluid_against_gradient() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(2, 2, 4.0, FluidType::Water);
        world.spawn((
            GridPosition { x: 2, y: 2 },
            FluidPump {
                target: TilePos::new(2, 5),
                rate: 2.0,
            },
            PowerConsumer {
                demand: 1.0,
                active: true,
            },
        ));
        world.run_system_once(fluid_pump_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.volume(2, 2), 2.0, "Pump should drain its tile");
        assert_eq!(grid.volume(2, 5), 2.0, "Pump should fill its target");
    }

    #[test]
    fn test_unpowered_pump_sits_idle() {
        let mut world = world_with_grid(10, 10);
        world
            .resource_mut::<FluidGrid>()
            .set_fluid(2, 2, 4.0, FluidType::Water);
        world.spawn((
            GridPosition { x: 2, y: 2 },
            FluidPump {
                target: TilePos::new(2, 5),
                rate: 2.0,
            },
            PowerConsumer {
                demand: 1.0,
                active: false,
            },
        ));
        world.run_system_once(fluid_pump_system).unwrap();
        let grid = world.resource::<FluidGrid>();
        assert_eq!(grid.volume(2, 2), 4.0);
        assert_eq!(grid.volume(2, 5), 0.0);
    }

    #[test]
    fn test_aquifer_puncture_releases_water_once() {
        let mut grid = FluidGrid::new(10, 10);
        let mut aquifers = AquiferMap::default();
        assert!(!puncture_aquifer(&mut grid, &mut aquifers, 4, 4), "No aquifer, no burst");
        aquifers.tiles.insert((4, 4));
        assert!(puncture_aquifer(&mut grid, &mut aquifers, 4, 4));
        assert_eq!(grid.volume(4, 4), AQUIFER_BURST);
        assert_eq!(grid.get(4, 4).fluid_type, FluidType::Water);
        assert!(!puncture_aquifer(&mut grid, &mut aquifers, 4, 4), "Aquifer is spent");
    }

    #[test]
    fn test_deep_water_drowns_and_magma_burns() {
        let mut world = World::new();
        let mut grid = FluidGrid::new(10, 10);
        grid.set_fluid(1, 1, DEEP_WATER_VOLUME, FluidType::Water);
        grid.set_fluid(2, 2, 1.0, FluidType::Magma);
        grid.set_fluid(3, 3, 2.0, FluidType::Water); // shallow: safe
        world.insert_resource(grid);
        let mk = |x: i32, y: i32| {
            (
                GridPosition { x, y },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            )
        };
        let deep = world.spawn(mk(1, 1)).id();
        let magma = world.spawn(mk(2, 2)).id();
        let shallow = world.spawn(mk(3, 3)).id();
        world.run_system_once(fluid_hazard_system).unwrap();
        assert_eq!(world.get::<Health>(deep).unwrap().current, 100.0 - DROWN_DAMAGE);
        assert_eq!(world.get::<Health>(magma).unwrap().current, 100.0 - MAGMA_DAMAGE);
        assert_eq!(world.get::<Health>(shallow).unwrap().current, 100.0);
    }

    #[test]
    fn test_ip_guard_fluid_names_are_generic() {
        let banned = [
            "dwarf fortress",
            "rimworld",
            "minecraft",
            "terraria",
            "oxygen not included",
            "valve",
            "nintendo",
        ];
        let public_strings = [
            FluidType::Water.name(),
            FluidType::Magma.name(),
            "Floodgate",
            "FluidPump",
            "Aquifer",
            "Fluid Simulation",
        ];
        for s in public_strings {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(!lower.contains(b), "public string {s:?} contains banned term {b:?}");
            }
        }
    }
}
