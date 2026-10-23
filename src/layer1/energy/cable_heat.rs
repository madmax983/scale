//! Cable heat-buildup pre-ignition telegraph (Item 125).
//!
//! Item 120 chronicled cable ignition the tick it happens, but the
//! per-tile cable lane in `load_limits::evaluate_grid_load_system` had
//! no warning *before* that tick: a cable whose local demand sits
//! between 1x and 2x its capacity silently adds heat
//! (`overload * 0.1` per tick to the `TemperatureGrid`) with no signal
//! at all, and the first Chronicle entry was the fire itself at >2x.
//!
//! This module is the telegraph for that lane. It tracks, per cable
//! tile, how long demand has stayed over capacity and produces
//! Chronicle warnings while there is still time to shed load. It
//! changes NO balance constants: cable capacity semantics, the
//! `overload * 0.1` heat rate, the 2.0x fire threshold, battery
//! buffering, and the fire pipeline in `load_limits` are byte-identical
//! in behaviour. Telegraphing only — no mercy, no retuning.
//!
//! Window choice mirrors the proven Item 117 grid-strain pattern
//! (`super::grid_strain`): first warning after
//! [`CABLE_HEAT_WARN_TICKS`] (10) consecutive overloaded ticks, so a
//! transient spike shorter than 10 ticks stays silent; escalated
//! warning after [`CABLE_HEAT_ESCALATE_TICKS`] (40) consecutive ticks.
//! Unlike the grid lane, cable ignition above 2x is instant rather than
//! a per-tick roll, so the sustained 1x–2x band tracked here is exactly
//! the band where a warning can land before any fire: heat is building
//! every tick, but nothing has ignited yet.
//!
//! Importance follows Item 117/120 precedent: the first warning is
//! `Standard` (nothing has happened yet — `Major` is reserved for the
//! ignition itself); the escalated warning is `Major` (the cable has
//! held an overload far past the first warning with no relief).
//!
//! Hysteresis: a warning fires once per sustained episode per tile.
//! The tracker re-arms only after [`CABLE_HEAT_CLEAR_TICKS`]
//! consecutive non-overloaded ticks for that tile, so a cable flickering
//! around its capacity cannot spam the Chronicle, and a brief dip does
//! not re-arm the warning.
//!
//! Scope: this covers the per-tile `PowerCable` path in
//! `load_limits::evaluate_grid_load_system` only. The colony-level grid
//! path is covered separately by `super::grid_strain` (Item 117).

use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Consecutive overloaded ticks before the first Chronicle warning.
pub const CABLE_HEAT_WARN_TICKS: u32 = 10;
/// Consecutive overloaded ticks before the escalated (Major) warning.
pub const CABLE_HEAT_ESCALATE_TICKS: u32 = 40;
/// Consecutive non-overloaded ticks required to re-arm after an episode.
pub const CABLE_HEAT_CLEAR_TICKS: u32 = 5;

/// Which warning an observation produced, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CableHeatWarning {
    /// First warning of an episode (Standard importance).
    First,
    /// Escalated warning for a long-unresolved episode (Major).
    Escalated,
}

/// Sustained-overload state for one cable tile.
#[derive(Debug, Clone, Default)]
pub struct CableTileHeat {
    /// Consecutive ticks this tile's demand has exceeded capacity.
    pub consecutive_overloaded_ticks: u32,
    /// Consecutive non-overloaded ticks since the last overloaded tick.
    pub consecutive_clear_ticks: u32,
    /// Demand seen on the most recent overloaded tick.
    pub last_demand: f32,
    /// Capacity seen on the most recent overloaded tick.
    pub last_capacity: f32,
    /// Peak demand/capacity ratio of the current episode.
    pub peak_ratio: f32,
    /// Whether the first warning already fired for this episode.
    pub warned: bool,
    /// Whether the escalated warning already fired for this episode.
    pub escalated: bool,
}

impl CableTileHeat {
    /// Observe one tick for this tile. Overloaded means
    /// `demand > capacity` — a read of the existing overload condition
    /// in `evaluate_grid_load_system`, not a new tuning.
    /// Returns the warning to chronicle, if this tick produces one.
    pub fn observe(&mut self, demand: f32, capacity: f32) -> Option<CableHeatWarning> {
        if demand > capacity {
            self.consecutive_overloaded_ticks += 1;
            self.consecutive_clear_ticks = 0;
            self.last_demand = demand;
            self.last_capacity = capacity;
            if capacity > 0.0 {
                let ratio = demand / capacity;
                if ratio > self.peak_ratio {
                    self.peak_ratio = ratio;
                }
            }
            if !self.warned && self.consecutive_overloaded_ticks >= CABLE_HEAT_WARN_TICKS {
                self.warned = true;
                return Some(CableHeatWarning::First);
            }
            if self.warned
                && !self.escalated
                && self.consecutive_overloaded_ticks >= CABLE_HEAT_ESCALATE_TICKS
            {
                self.escalated = true;
                return Some(CableHeatWarning::Escalated);
            }
            None
        } else {
            self.consecutive_overloaded_ticks = 0;
            if self.warned || self.escalated {
                self.consecutive_clear_ticks += 1;
                if self.consecutive_clear_ticks >= CABLE_HEAT_CLEAR_TICKS {
                    self.warned = false;
                    self.escalated = false;
                    self.consecutive_clear_ticks = 0;
                    self.peak_ratio = 0.0;
                }
            } else {
                self.consecutive_clear_ticks = 0;
                self.peak_ratio = 0.0;
            }
            None
        }
    }
}

/// Per-tile tracker for sustained cable overloads.
///
/// Cables are individual tiles with their own capacity and local
/// demand, so — unlike the colony-level `GridStrainTracker` — state is
/// kept per tile: one overheating cable must not be masked by (or
/// blamed on) another.
#[derive(Resource, Default, Debug, Clone)]
pub struct CableHeatTracker {
    /// Sustained-overload state per cable tile.
    pub tiles: HashMap<(i32, i32), CableTileHeat>,
}

impl CableHeatTracker {
    /// Observe one tick for one cable tile.
    /// Returns the warning to chronicle, if this tick produces one.
    pub fn observe(
        &mut self,
        tile: (i32, i32),
        demand: f32,
        capacity: f32,
    ) -> Option<CableHeatWarning> {
        self.tiles
            .entry(tile)
            .or_default()
            .observe(demand, capacity)
    }

    /// Highest consecutive-overloaded count across all cable tiles.
    pub fn max_consecutive_overloaded_ticks(&self) -> u32 {
        self.tiles
            .values()
            .map(|t| t.consecutive_overloaded_ticks)
            .max()
            .unwrap_or(0)
    }

    /// Whether any cable tile is in a warned/escalated episode that
    /// has not yet cleared through the full re-arm window.
    pub fn any_episode_active(&self) -> bool {
        self.tiles.values().any(|t| t.warned || t.escalated)
    }
}

/// STATS helper: the highest consecutive-overloaded tick count of any
/// cable tile (0 when no cable is overloaded). Full context lives in
/// the Chronicle warnings.
pub fn cable_heat_ticks(world: &World) -> u32 {
    world
        .get_resource::<CableHeatTracker>()
        .map_or(0, |t| t.max_consecutive_overloaded_ticks())
}

/// STATS helper: whether a cable-heat episode is still armed.
/// `cable_heat_ticks` reads the consecutive-overloaded counter, which
/// resets to 0 on the first non-overloaded tick — so during a flicker
/// dip of up to [`CABLE_HEAT_CLEAR_TICKS`] ticks the counter reads 0
/// while the episode (and its already-fired warnings) is still live.
/// An episode is active while either warning flag is latched on any
/// tile; the flags clear only after the full clear window, exactly
/// when the tracker re-arms. Mirrors `grid_strain_episode_active`.
pub fn cable_heat_episode_active(world: &World) -> bool {
    world
        .get_resource::<CableHeatTracker>()
        .is_some_and(|t| t.any_episode_active())
}
