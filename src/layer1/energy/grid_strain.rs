//! Grid-straining pre-ignition telegraph (Item 117).
//!
//! Item 114 chronicled grid-overload ignition the tick it happens, but
//! there was no warning *before* that tick: `handle_overload` rolls
//! `(ratio - 1.5) * 0.05` per tick once demand exceeds 150% of nameplate
//! production, and the first visible signal was the fire itself.
//!
//! This module is the telegraph. It tracks how long the colony's most
//! strained grid has stayed over the 1.5 overload threshold and writes
//! Chronicle warnings while there is still time to shed load or add
//! generation. It changes NO balance constants: the 1.5 threshold, the
//! `(ratio - 1.5) * 0.05` ignition chance, and the 10 HP overload damage
//! in `super::handle_overload` are byte-identical. Telegraphing and
//! counterplay only — no mercy, no retuning.
//!
//! Window justification (from the ignition math): at ratio 2.0 the
//! per-tick ignition chance is 0.025, so expected time-to-ignition is
//! ~40 ticks. A warning after [`STRAIN_WARN_TICKS`] (10) consecutive
//! strained ticks lands ~30 ticks ahead of that expectation, while a
//! transient spike shorter than 10 ticks stays silent. At ratio 3.0 the
//! expectation is ~13 ticks, so no fixed window can always win that
//! race — that is inherent to the existing ignition roll, not something
//! a telegraph may "fix" by retuning. The escalated warning at
//! [`STRAIN_ESCALATE_TICKS`] (40) consecutive ticks marks the point
//! where, at ratio 2.0, ignition is already overdue in expectation.
//!
//! Importance: the first warning is `Standard` — it is a warning about
//! something that has not happened yet, and `Major` is reserved (per
//! the Item 114 precedent) for the ignition itself. The escalated
//! warning is `Major`: by then the grid has strained past the expected
//! ignition horizon and the fire is imminent.
//!
//! Hysteresis: a warning fires once per sustained episode. The tracker
//! re-arms only after [`STRAIN_CLEAR_TICKS`] consecutive unstrained
//! ticks, so a grid flickering around the threshold cannot spam the
//! Chronicle.
//!
//! Scope: this covers the main grid path (`power_grid_system` ->
//! `handle_overload` -> `GridOverloadEvent` -> ignition bridge). The
//! per-tile `PowerCable` path in `load_limits::evaluate_grid_load_system`
//! is a separate, also-live ignition path (local cable capacity vs.
//! local demand, fire at >2x); it has no grid-level ratio to track, so
//! it is deliberately NOT folded in here — see the Item 117 report.

use bevy_ecs::prelude::*;

/// Consecutive strained ticks before the first Chronicle warning.
pub const STRAIN_WARN_TICKS: u32 = 10;
/// Consecutive strained ticks before the escalated (Major) warning.
pub const STRAIN_ESCALATE_TICKS: u32 = 40;
/// Consecutive unstrained ticks required to re-arm after an episode.
pub const STRAIN_CLEAR_TICKS: u32 = 5;
/// Overload ratio threshold — mirrors `handle_overload`'s 1.5 exactly.
/// This is a read of the existing balance constant, not a new tuning.
pub const STRAIN_RATIO_THRESHOLD: f32 = 1.5;

/// Which warning an observation produced, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrainWarning {
    /// First warning of an episode (Standard importance).
    First,
    /// Escalated warning for a long-unresolved episode (Major).
    Escalated,
}

/// Colony-level tracker for the most strained grid each tick.
///
/// Grids are dynamic connected components with no stable identity, so
/// the tracker follows the worst grid's ratio per tick — the Chronicle
/// is a colony narrative, and "the grid is straining" is the actionable
/// signal regardless of which component is worst.
#[derive(Resource, Default, Debug, Clone)]
pub struct GridStrainTracker {
    /// Consecutive ticks the worst grid has been over the threshold.
    pub consecutive_strained_ticks: u32,
    /// Consecutive unstrained ticks since the last strained tick.
    pub consecutive_clear_ticks: u32,
    /// Worst overload ratio seen on the most recent tick (1.0 if none).
    pub last_ratio: f32,
    /// Peak overload ratio of the current episode.
    pub peak_ratio: f32,
    /// Whether the first warning already fired for this episode.
    pub warned: bool,
    /// Whether the escalated warning already fired for this episode.
    pub escalated: bool,
    /// Position of a representative entity on the worst grid.
    pub example_pos: Option<(i32, i32)>,
    /// Building label of that representative entity, if it has one.
    pub example_label: Option<String>,
}

impl GridStrainTracker {
    /// Observe one tick. `worst_ratio` is the highest overload ratio of
    /// any grid this tick (pass 1.0 or less when no grid is strained);
    /// `example` describes a representative entity on that worst grid.
    /// Returns the warning to chronicle, if this tick produces one.
    pub fn observe(
        &mut self,
        worst_ratio: f32,
        example: Option<(Option<String>, (i32, i32))>,
    ) -> Option<StrainWarning> {
        self.last_ratio = worst_ratio;
        if worst_ratio > STRAIN_RATIO_THRESHOLD {
            self.consecutive_strained_ticks += 1;
            self.consecutive_clear_ticks = 0;
            if worst_ratio > self.peak_ratio {
                self.peak_ratio = worst_ratio;
            }
            if let Some((label, pos)) = example {
                self.example_label = label;
                self.example_pos = Some(pos);
            }
            if !self.warned && self.consecutive_strained_ticks >= STRAIN_WARN_TICKS {
                self.warned = true;
                return Some(StrainWarning::First);
            }
            if self.warned
                && !self.escalated
                && self.consecutive_strained_ticks >= STRAIN_ESCALATE_TICKS
            {
                self.escalated = true;
                return Some(StrainWarning::Escalated);
            }
            None
        } else {
            self.consecutive_strained_ticks = 0;
            if self.warned || self.escalated {
                self.consecutive_clear_ticks += 1;
                if self.consecutive_clear_ticks >= STRAIN_CLEAR_TICKS {
                    self.warned = false;
                    self.escalated = false;
                    self.consecutive_clear_ticks = 0;
                    self.peak_ratio = 0.0;
                    self.example_pos = None;
                    self.example_label = None;
                }
            } else {
                self.consecutive_clear_ticks = 0;
                self.peak_ratio = 0.0;
            }
            None
        }
    }

    /// Whether any grid is currently in a sustained strain episode.
    pub fn is_strained(&self) -> bool {
        self.consecutive_strained_ticks > 0
    }
}

/// STATS helper: consecutive strained ticks of the worst grid (0 when
/// no grid is strained). Full context lives in the Chronicle warnings.
pub fn grid_strain_ticks(world: &World) -> u32 {
    world
        .get_resource::<GridStrainTracker>()
        .map_or(0, |t| t.consecutive_strained_ticks)
}
