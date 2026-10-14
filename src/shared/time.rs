//! Simulation time management.
//!
//! This module handles the passage of time in the game world, distinguishing between
//! "Simulation Time" (game state updates) and "Wall Time" (real-world seconds).
//!
//! # The Time Model
//!
//! 1.  **Ticks ([`crate::shared::time::SimulationTime`])**: The fundamental quantum of game logic.
//!     *   Simulation systems run once per tick.
//!     *   1 tick is roughly 100ms of "game time" (at 1x speed).
//!     *   The game is deterministic based on ticks, not real time.
//!
//! 2.  **Speed ([`crate::shared::time::SimSpeed`])**: Controls how many ticks occur per second.
//!     *   **Paused**: 0 ticks/sec.
//!     *   **1x**: 10 ticks/sec.
//!     *   **3x**: 30 ticks/sec.
//!     *   **5x**: 50 ticks/sec.
//!
//! 3.  **Wall Time ([`crate::shared::time::WallTime`])**: Measures real-world seconds since app start.
//!     *   Used for UI animations (pulsing cursors, fading notifications) that must
//!         continue even when the game is paused.

use bevy_ecs::prelude::*;

/// Tracks the global simulation time and speed.
///
/// This resource is the "clock" of the colony.
///
/// # Examples
///
/// Reading the current tick:
/// ```
/// use scale::shared::time::SimulationTime;
///
/// let time = SimulationTime::default();
/// println!("Current tick: {}", time.tick);
/// ```
#[derive(Resource, Default)]
pub struct SimulationTime {
    /// The current simulation tick (update count).
    ///
    /// Monotonically increasing. Only increments when the game is unpaused.
    pub tick: u64,
    /// The current simulation speed target.
    pub speed: SimSpeed,
}

/// Tracks the wall-clock time for UI animations (e.g. pulsing cursors).
///
/// This is updated every frame (render loop), independent of simulation speed or pausing.
/// Use this for visual effects that shouldn't freeze when the game is paused.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct WallTime(pub f32);

/// Defines the speed at which the simulation runs.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SimSpeed {
    /// The simulation is paused (0 ticks/sec).
    Paused,
    /// The simulation runs at normal speed (10 ticks/sec).
    #[default]
    Normal,
    /// The simulation runs at fast speed (30 ticks/sec).
    Fast,
    /// The simulation runs at maximum speed (50 ticks/sec).
    Faster,
}

impl SimSpeed {
    /// Returns a user-friendly label for the UI.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::shared::time::SimSpeed;
    ///
    /// assert!(SimSpeed::Paused.label().contains("Paused"));
    /// assert!(SimSpeed::Normal.label().contains("1x"));
    /// ```
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Paused => "⏸ Paused",
            Self::Normal => "▶ 1x",
            Self::Fast => "▶▶ 3x",
            Self::Faster => "▶▶▶ 5x",
        }
    }

    /// Simulation ticks issued per real second at this speed.
    ///
    /// This is the rate the frame loops must realize — the enum's
    /// documented contract (0 / 10 / 30 / 50) that [`TickAccumulator`]
    /// turns into per-frame tick counts.
    #[must_use]
    pub const fn ticks_per_second(self) -> u32 {
        match self {
            Self::Paused => 0,
            Self::Normal => 10,
            Self::Fast => 30,
            Self::Faster => 50,
        }
    }

    /// The next rung of the speed ladder, as bound to the `.` key:
    /// Normal → Fast → Faster → Normal.
    ///
    /// Paused is not a rung — pausing is `GameState`'s job, toggled
    /// with Space — so cycling from `Paused` lands on Normal.
    #[must_use]
    pub const fn cycle(self) -> Self {
        match self {
            Self::Normal => Self::Fast,
            Self::Fast => Self::Faster,
            Self::Faster | Self::Paused => Self::Normal,
        }
    }
}

/// Converts elapsed real time into whole simulation ticks at a
/// [`SimSpeed`]'s rate.
///
/// Frame loops own one accumulator and call [`advance_frame`] once
/// per rendered frame (or [`advance`] with the measured frame delta,
/// as the native loop does). Fractional ticks are banked between
/// calls, so the long-run rate is exact: over any horizon the total
/// ticks issued differ from `elapsed × ticks_per_second` by less than
/// one tick — no drift, no rounding loss.
///
/// One call never issues more than [`MAX_TICKS_PER_ADVANCE`] ticks.
/// After a long stall (window drag, debugger breakpoint, suspended
/// tab) the backlog beyond the cap is dropped instead of
/// fast-forwarding through it in a spiral of death.
///
/// [`advance_frame`]: TickAccumulator::advance_frame
/// [`advance`]: TickAccumulator::advance
/// [`MAX_TICKS_PER_ADVANCE`]: TickAccumulator::MAX_TICKS_PER_ADVANCE
#[derive(Debug, Default, Clone, Copy)]
pub struct TickAccumulator {
    /// Banked fractional ticks, kept in `[0.0, 1.0)` between calls.
    credit: f64,
}

impl TickAccumulator {
    /// Upper bound on ticks issued by a single `advance` call.
    ///
    /// At the fastest speed (50 ticks/sec) an ordinary 60 fps frame
    /// owes at most one tick; the cap only bites on multi-second
    /// stalls, where dropping time is the correct behavior.
    pub const MAX_TICKS_PER_ADVANCE: u32 = 10;

    /// A fresh accumulator with no banked credit.
    #[must_use]
    pub const fn new() -> Self {
        Self { credit: 0.0 }
    }

    /// Advance by one rendered frame at a nominal 60 fps and return
    /// how many simulation ticks are due for it.
    pub fn advance_frame(&mut self, speed: SimSpeed) -> u32 {
        self.advance(speed, 1.0 / 60.0)
    }

    /// Advance by `elapsed_secs` of real time and return how many
    /// simulation ticks are due.
    ///
    /// Non-positive elapsed times (clock hiccups) issue no ticks and
    /// bank nothing.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn advance(&mut self, speed: SimSpeed, elapsed_secs: f64) -> u32 {
        if elapsed_secs.is_nan() || elapsed_secs <= 0.0 {
            return 0;
        }
        self.credit += elapsed_secs * f64::from(speed.ticks_per_second());
        let due = self.credit.floor() as u32;
        if due > Self::MAX_TICKS_PER_ADVANCE {
            // Stall: drop the backlog beyond the cap rather than
            // fast-forwarding through it.
            self.credit = 0.0;
            return Self::MAX_TICKS_PER_ADVANCE;
        }
        self.credit -= f64::from(due);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sim_speed_labels() {
        assert!(SimSpeed::Paused.label().contains("Paused"));
        assert!(SimSpeed::Normal.label().contains("1x"));
        assert!(SimSpeed::Fast.label().contains("3x"));
        assert!(SimSpeed::Faster.label().contains("5x"));
    }

    #[test]
    fn test_simulation_time_default() {
        let sim_time = SimulationTime::default();
        assert_eq!(sim_time.tick, 0);
        assert_eq!(sim_time.speed, SimSpeed::Normal);
    }

    #[test]
    fn test_wall_time_default() {
        let wall_time = WallTime::default();
        assert!((wall_time.0 - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sim_speed_transitions() {
        let mut speed = SimSpeed::Normal;
        assert_eq!(speed, SimSpeed::Normal);

        speed = SimSpeed::Fast;
        assert_eq!(speed, SimSpeed::Fast);

        speed = SimSpeed::Paused;
        assert_eq!(speed, SimSpeed::Paused);

        speed = SimSpeed::Faster;
        assert_eq!(speed, SimSpeed::Faster);
    }

    #[test]
    fn test_tick_increment() {
        let mut sim_time = SimulationTime::default();
        assert_eq!(sim_time.tick, 0);

        sim_time.tick += 1;
        assert_eq!(sim_time.tick, 1);

        sim_time.tick += 10;
        assert_eq!(sim_time.tick, 11);
    }

    #[test]
    fn test_speed_is_copy() {
        let speed1 = SimSpeed::Fast;
        let speed2 = speed1; // Should copy, not move
        assert_eq!(speed1, speed2);
    }

    #[test]
    fn test_speed_debug_format() {
        let speed = SimSpeed::Normal;
        let debug_str = format!("{speed:?}");
        assert!(debug_str.contains("Normal"));
    }

    #[test]
    fn test_simulation_time_speed_changes() {
        let mut sim_time = SimulationTime::default();
        assert_eq!(sim_time.speed, SimSpeed::Normal);

        sim_time.speed = SimSpeed::Fast;
        assert_eq!(sim_time.speed, SimSpeed::Fast);
        assert_eq!(sim_time.tick, 0); // Speed change doesn't affect tick

        sim_time.tick = 42;
        sim_time.speed = SimSpeed::Paused;
        assert_eq!(sim_time.tick, 42); // Tick preserved across speed change
    }

    #[test]
    fn test_ticks_per_second_matches_documented_rates() {
        assert_eq!(SimSpeed::Paused.ticks_per_second(), 0);
        assert_eq!(SimSpeed::Normal.ticks_per_second(), 10);
        assert_eq!(SimSpeed::Fast.ticks_per_second(), 30);
        assert_eq!(SimSpeed::Faster.ticks_per_second(), 50);
    }

    #[test]
    fn test_speed_cycle_rotates_and_wraps() {
        assert_eq!(SimSpeed::Normal.cycle(), SimSpeed::Fast);
        assert_eq!(SimSpeed::Fast.cycle(), SimSpeed::Faster);
        assert_eq!(SimSpeed::Faster.cycle(), SimSpeed::Normal);
        // Paused sits outside the cycle (Space owns pause); cycling
        // from it lands on Normal.
        assert_eq!(SimSpeed::Paused.cycle(), SimSpeed::Normal);
        // Three cycles from any rung return to the start.
        let mut speed = SimSpeed::Normal;
        for _ in 0..3 {
            speed = speed.cycle();
        }
        assert_eq!(speed, SimSpeed::Normal);
    }

    /// Drive the accumulator one rendered frame at a time, exactly as
    /// the frame loops do, and total the ticks issued.
    fn ticks_over_frames(speed: SimSpeed, frames: u32) -> u32 {
        let mut acc = TickAccumulator::new();
        (0..frames).map(|_| acc.advance_frame(speed)).sum()
    }

    #[test]
    fn test_accumulator_exact_rates_over_600_frames() {
        // 600 frames at 60 fps == 10 seconds of wall time. The demo
        // from the spec: 600 frames at Faster yields ~500 ticks, at
        // Normal ~100.
        assert!((99..=100).contains(&ticks_over_frames(SimSpeed::Normal, 600)));
        assert!((299..=300).contains(&ticks_over_frames(SimSpeed::Fast, 600)));
        assert!((499..=500).contains(&ticks_over_frames(SimSpeed::Faster, 600)));
        assert_eq!(ticks_over_frames(SimSpeed::Paused, 600), 0);
    }

    #[test]
    fn test_accumulator_no_long_run_drift() {
        // 36,000 frames == 10 minutes. Remainder banking keeps the
        // total within one tick of exact at any horizon.
        let total = ticks_over_frames(SimSpeed::Fast, 36_000);
        assert!((17_999..=18_000).contains(&total), "got {total}");
    }

    #[test]
    fn test_accumulator_frame_bursts_are_small() {
        // Even the fastest speed averages under one tick per frame, so
        // no single frame ever owes more than one tick.
        let mut acc = TickAccumulator::new();
        for _ in 0..600 {
            assert!(acc.advance_frame(SimSpeed::Faster) <= 1);
        }
    }

    #[test]
    fn test_accumulator_cap_drops_stall_backlog() {
        let mut acc = TickAccumulator::new();
        // A 60-second stall at 5x would owe 3000 ticks; the call caps
        // and drops the backlog instead of fast-forwarding through it.
        assert_eq!(
            acc.advance(SimSpeed::Faster, 60.0),
            TickAccumulator::MAX_TICKS_PER_ADVANCE
        );
        // The next ordinary frame owes at most one tick again.
        assert!(acc.advance_frame(SimSpeed::Faster) <= 1);
    }

    #[test]
    fn test_accumulator_ignores_nonpositive_elapsed() {
        let mut acc = TickAccumulator::new();
        assert_eq!(acc.advance(SimSpeed::Fast, 0.0), 0);
        assert_eq!(acc.advance(SimSpeed::Fast, -1.0), 0);
    }
}
