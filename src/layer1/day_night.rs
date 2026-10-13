//! Day/Night Cycle System.
//!
//! This module implements the "Day/Night Cycle" mechanic, driving global changes in lighting,
//! scheduling, and physiological needs based on the progression of time.
//!
//! # Context
//! The [`DayNightCycle`] resource tracks the current [`TimeOfDay`] (Dawn, Day, Dusk, Night) and the
//! number of days elapsed. It is updated by the [`update_day_night_cycle_system`] based on the global
//! [`SimulationTime`](crate::shared::time::SimulationTime).
//!
//! Progression of the cycle has systemic effects:
//! *   [`update_ambient_light_from_cycle_system`] adjusts the global [`AmbientLight`](crate::layer1::lighting::AmbientLight) level.
//! *   [`circadian_rhythm_system`] increases the rate at which `Pop`s' `rest` need decays during `TimeOfDay::Night`.
//!
//! # Usage
//! ```
//! use bevy_ecs::prelude::*;
//! use scale::layer1::day_night::{DayNightCycle, TimeOfDay, update_day_night_cycle_system};
//! use scale::shared::time::SimulationTime;
//!
//! let mut world = World::new();
//!
//! // Setup time and cycle
//! world.insert_resource(SimulationTime { tick: 90, ..Default::default() });
//! world.insert_resource(DayNightCycle {
//!     time_of_day: TimeOfDay::Day,
//!     ticks_per_day: 100, // 0-9: Dawn, 10-74: Day, 75-84: Dusk, 85-99: Night
//!     day_count: 0,
//! });
//!
//! // Advance the cycle
//! let mut schedule = Schedule::default();
//! schedule.add_systems(update_day_night_cycle_system);
//! schedule.run(&mut world);
//!
//! // At tick 90, it is Night!
//! let cycle = world.resource::<DayNightCycle>();
//! assert_eq!(cycle.time_of_day, TimeOfDay::Night);
//! ```

use crate::layer1::lighting::AmbientLight;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Phases of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeOfDay {
    /// Sunrise phase.
    Dawn,
    /// Daylight phase.
    #[default]
    Day,
    /// Sunset phase.
    Dusk,
    /// Nighttime phase.
    Night,
}

/// Tracks the progress of the day.
#[derive(Resource, Clone, Copy)]
pub struct DayNightCycle {
    /// Current phase of the day.
    pub time_of_day: TimeOfDay,
    /// Number of days elapsed.
    pub day_count: u32,
    /// Total ticks in a full day cycle.
    pub ticks_per_day: u64,
}

impl Default for DayNightCycle {
    fn default() -> Self {
        Self {
            time_of_day: TimeOfDay::Day,
            day_count: 0,
            ticks_per_day: 250, // 4 days per year (1000 ticks)
        }
    }
}

const DAWN_THRESHOLD: f32 = 0.1;
const DAY_THRESHOLD: f32 = 0.75;
const DUSK_THRESHOLD: f32 = 0.85;

/// Updates the `DayNightCycle` based on `SimulationTime`.
///
/// Spec 345: on a Tidally Locked world there is no cycle to advance —
/// the colony heart sits on the permanent day side, so the phase is
/// frozen at Day (lighting zones across the grid are handled by the
/// solar path's `tidal_solar_factor`, not by a global phase).
pub fn update_day_night_cycle_system(
    time: Res<SimulationTime>,
    mut cycle: ResMut<DayNightCycle>,
    traits: Option<Res<crate::layer1::quirks::PlanetaryTraits>>,
) {
    if traits.as_deref().is_some_and(|t| {
        t.has(crate::layer1::quirks::PlanetaryTrait::TidallyLocked)
    }) {
        cycle.time_of_day = TimeOfDay::Day;
        return;
    }

    let tick = time.tick;
    let ticks_in_day = cycle.ticks_per_day;

    if ticks_in_day == 0 {
        return;
    }

    // Calculate current tick within the day
    let day_tick = tick % ticks_in_day;
    #[allow(clippy::cast_possible_truncation)]
    {
        cycle.day_count = (tick / ticks_in_day) as u32;
    }

    // Define phases (simple hardcoded thresholds for MVP)
    #[allow(clippy::cast_precision_loss)]
    let pct = day_tick as f32 / ticks_in_day as f32;

    cycle.time_of_day = if pct < DAWN_THRESHOLD {
        TimeOfDay::Dawn
    } else if pct < DAY_THRESHOLD {
        TimeOfDay::Day
    } else if pct < DUSK_THRESHOLD {
        TimeOfDay::Dusk
    } else {
        TimeOfDay::Night
    };
}

const DAWN_LIGHT: f32 = 0.6;
const DAY_LIGHT: f32 = 1.0;
const DUSK_LIGHT: f32 = 0.5;
const NIGHT_LIGHT: f32 = 0.2;

/// Updates `AmbientLight` based on `DayNightCycle`.
pub fn update_ambient_light_from_cycle_system(
    cycle: Res<DayNightCycle>,
    mut ambient: ResMut<AmbientLight>,
) {
    ambient.level = match cycle.time_of_day {
        TimeOfDay::Dawn => DAWN_LIGHT,
        TimeOfDay::Day => DAY_LIGHT,
        TimeOfDay::Dusk => DUSK_LIGHT,
        TimeOfDay::Night => NIGHT_LIGHT,
    };
}

const EXTRA_REST_DECAY_PER_TICK: f32 = 0.0005;

/// Applies increased rest decay during night.
pub fn circadian_rhythm_system(cycle: Res<DayNightCycle>, mut query: Query<&mut Needs, With<Pop>>) {
    // Only apply extra decay at night
    if cycle.time_of_day != TimeOfDay::Night {
        return;
    }

    for mut needs in &mut query {
        needs.rest = (needs.rest - EXTRA_REST_DECAY_PER_TICK).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::lighting::AmbientLight;
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;
    use bevy_ecs::world::World;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(DayNightCycle::default());
        world.insert_resource(AmbientLight::default());
        world
    }

    #[test]
    fn test_initialization() {
        let world = setup_world();
        let cycle = world.resource::<DayNightCycle>();
        assert_eq!(cycle.time_of_day, TimeOfDay::Day); // Default start
        assert_eq!(cycle.day_count, 0);
    }

    #[test]
    fn test_cycle_progression() {
        let mut world = setup_world();

        // Advance time to Night
        // Assuming Day Length = 100 ticks for test simplicity (configurable)
        // Dawn: 0-25, Day: 25-75, Dusk: 75-85, Night: 85-100
        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.ticks_per_day = 100;
        }

        {
            let mut time = world.resource_mut::<SimulationTime>();
            time.tick = 90;
        }

        world
            .run_system_once(update_day_night_cycle_system)
            .unwrap();

        let cycle = world.resource::<DayNightCycle>();
        assert_eq!(cycle.time_of_day, TimeOfDay::Night);
    }

    #[test]
    fn test_day_increment() {
        let mut world = setup_world();

        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.ticks_per_day = 100;
        }

        // Cross day boundary (tick 99 -> 100)
        {
            let mut time = world.resource_mut::<SimulationTime>();
            time.tick = 101;
        }

        world
            .run_system_once(update_day_night_cycle_system)
            .unwrap();

        let cycle = world.resource::<DayNightCycle>();
        assert_eq!(cycle.day_count, 1);
        assert_eq!(cycle.time_of_day, TimeOfDay::Dawn); // Start of new day
    }

    #[test]
    fn test_ambient_light_update() {
        let mut world = setup_world();

        // Set to Night
        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.time_of_day = TimeOfDay::Night;
        }

        world
            .run_system_once(update_ambient_light_from_cycle_system)
            .unwrap();

        let ambient = world.resource::<AmbientLight>();
        assert!(ambient.level < 0.3); // Should be dark
    }

    #[test]
    fn test_circadian_rhythm_rest_decay() {
        let mut world = setup_world();

        // Spawn Pop
        let pop = world
            .spawn((
                Pop,
                Needs {
                    rest: 1.0,
                    ..Default::default()
                },
            ))
            .id();

        // Set to Day
        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.time_of_day = TimeOfDay::Day;
        }

        // Run system
        world.run_system_once(circadian_rhythm_system).unwrap();
        let rest_day = world.get::<Needs>(pop).unwrap().rest;

        // Reset
        world.get_mut::<Needs>(pop).unwrap().rest = 1.0;

        // Set to Night
        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.time_of_day = TimeOfDay::Night;
        }

        // Run system
        world.run_system_once(circadian_rhythm_system).unwrap();
        let rest_night = world.get::<Needs>(pop).unwrap().rest;

        // Verify decay was stronger at night (lower resulting rest)
        assert!(rest_night < rest_day, "Pops should tire faster at night");
    }

    #[test]
    fn test_all_phases_and_light_levels() {
        let mut world = setup_world();

        // 100 ticks per day for easy math
        // Dawn: 0-10 (0-9)
        // Day: 10-75 (10-74)
        // Dusk: 75-85 (75-84)
        // Night: 85-100 (85-99)
        {
            let mut cycle = world.resource_mut::<DayNightCycle>();
            cycle.ticks_per_day = 100;
        }

        let cases = vec![
            (5, TimeOfDay::Dawn, DAWN_LIGHT),
            (50, TimeOfDay::Day, DAY_LIGHT),
            (80, TimeOfDay::Dusk, DUSK_LIGHT),
            (90, TimeOfDay::Night, NIGHT_LIGHT),
        ];

        for (tick, expected_phase, expected_light) in cases {
            {
                let mut time = world.resource_mut::<SimulationTime>();
                time.tick = tick;
            }

            // Update Cycle
            world
                .run_system_once(update_day_night_cycle_system)
                .unwrap();

            // Verify Phase
            let cycle = world.resource::<DayNightCycle>();
            assert_eq!(
                cycle.time_of_day, expected_phase,
                "Tick {} should be {:?}",
                tick, expected_phase
            );

            // Update Light
            world
                .run_system_once(update_ambient_light_from_cycle_system)
                .unwrap();

            // Verify Light
            let ambient = world.resource::<AmbientLight>();
            assert!(
                (ambient.level - expected_light).abs() < f32::EPSILON,
                "Tick {} light should be {}, got {}",
                tick,
                expected_light,
                ambient.level
            );
        }
    }
}
