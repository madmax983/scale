//! Weather system for the colony.
//!
//! Handles:
//! - Weather state (Clear, Rain, Storm, etc.)
//! - Weather transitions based on Seasons.
//! - Effects on pop speed.

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::pop::Speed;
use crate::layer1::seasons::{Season, SeasonState};
use bevy_ecs::prelude::*;
use rand::Rng;

/// Types of weather conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeatherType {
    /// No adverse weather.
    #[default]
    Clear,
    /// Light rain, minor slowdown.
    Rain,
    /// Heavy storm, major slowdown.
    Storm,
    /// Fog, reduces visibility (future) and speed.
    Fog,
    /// Extreme heat, minor slowdown.
    Heatwave,
    /// Snow, moderate slowdown.
    Snow,
    /// Thermal Inversion, traps smog.
    ThermalInversion,
    /// Magnetic Storm, high energy interference.
    MagneticStorm,
    /// Toxic glowing rain that causes mutations.
    MutagenicRain,
    BlissStorm,
    SporeStorm,
}

impl WeatherType {
    /// Returns the display name of the weather.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Clear => "Clear Skies",
            Self::Rain => "Rain",
            Self::Storm => "Storm",
            Self::Fog => "Fog",
            Self::Heatwave => "Heatwave",
            Self::Snow => "Snow",
            Self::ThermalInversion => "Thermal Inversion",
            Self::MagneticStorm => "Magnetic Storm",
            Self::MutagenicRain => "Mutagenic Rain",
            Self::BlissStorm => "Bliss Storm",
            Self::SporeStorm => "Spore Storm",
        }
    }

    /// Returns the speed modifier for this weather (1.0 = normal).
    #[must_use]
    pub const fn speed_modifier(&self) -> f32 {
        match self {
            Self::Clear | Self::ThermalInversion => 1.0,
            Self::MutagenicRain | Self::BlissStorm => 1.0,
            Self::SporeStorm => 0.8,
            Self::Rain => 0.8,
            Self::Storm => 0.5,
            Self::Fog | Self::MagneticStorm => 0.7,
            Self::Heatwave => 0.9,
            Self::Snow => 0.6,
        }
    }
}

/// Resource tracking the current weather state.
#[derive(Resource)]
pub struct WeatherState {
    /// The current active weather.
    pub current_weather: WeatherType,
    /// Ticks remaining for the current weather.
    pub duration_remaining: u32,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self {
            current_weather: WeatherType::Clear,
            duration_remaining: 100, // Initial buffer
        }
    }
}

/// System to update weather duration and change weather when it expires.
pub fn update_weather_system(world: &mut World) {
    // 1. Check/update duration (scope the mutable borrow)
    let time_to_change = {
        let mut state = world.resource_mut::<WeatherState>();
        if state.duration_remaining > 0 {
            state.duration_remaining -= 1;
            false
        } else {
            true
        }
    };

    if !time_to_change {
        return;
    }

    // 2. Determine new weather
    let season = world.resource::<SeasonState>().current_season;
    let mut rng = rand::thread_rng();
    let new_weather = pick_weather_for_season(season, &mut rng);
    let new_duration = rng.gen_range(50..200);

    // 3. Apply changes and log
    {
        let mut state = world.resource_mut::<WeatherState>();
        state.current_weather = new_weather;
        state.duration_remaining = new_duration;
    }

    if new_weather != WeatherType::Clear {
        // We can't access SimulationTime inside the exclusive system easily if we also borrow World?
        // Wait, World has everything.
        // But to send event we need EventWriter logic or direct world access.
        // We can use world.send_event if we have the event type.

        let event = AddChronicleEvent {
            text: format!("Weather changed to {}.", new_weather.name()),
            importance: EventImportance::Minor,
        };
        world.send_event(event);
    }
}

fn pick_weather_for_season(season: Season, rng: &mut impl Rng) -> WeatherType {
    let roll = rng.gen_range(0.0..1.0);
    match season {
        Season::Spring => {
            // Spec 427: MutagenicRain takes a rare 3% slice carved from
            // Rain's tail (Rain 30% -> 27%); Clear's 60% share is
            // untouched so spring stays mostly fair.
            if roll < 0.6 {
                WeatherType::Clear
            } else if roll < 0.87 {
                WeatherType::Rain
            } else if roll < 0.90 {
                WeatherType::MutagenicRain
            } else {
                WeatherType::Fog
            }
        }
        Season::Summer => {
            // Spec 427: MutagenicRain takes a rare 2% slice carved from
            // Storm's tail (Storm 10% -> 8%); Clear's 70% share is
            // untouched so summer stays mostly fair.
            if roll < 0.7 {
                WeatherType::Clear
            } else if roll < 0.85 {
                WeatherType::Heatwave
            } else if roll < 0.93 {
                WeatherType::Storm
            } else if roll < 0.95 {
                WeatherType::MutagenicRain
            } else {
                WeatherType::MagneticStorm
            }
        }
        Season::Autumn => {
            if roll < 0.5 {
                WeatherType::Clear
            } else if roll < 0.8 {
                WeatherType::Rain
            } else {
                WeatherType::Fog
            }
        }
        Season::Winter => {
            if roll < 0.4 {
                WeatherType::Clear
            } else if roll < 0.8 {
                WeatherType::Snow
            } else if roll < 0.9 {
                WeatherType::ThermalInversion
            } else if roll < 0.95 {
                WeatherType::Storm
            } else {
                WeatherType::MagneticStorm
            } // Blizzard
        }
    }
}

/// Applies weather effects (speed penalties) to pops.
///
/// Multiplies `Speed.current` by the weather modifier.
/// Must run AFTER `apply_lighting_penalties_system` (which resets/sets base speed)
/// and BEFORE `movement_system`.
pub fn apply_weather_effects_system(mut pops: Query<&mut Speed>, weather: Res<WeatherState>) {
    let modifier = weather.current_weather.speed_modifier();
    // Optimization: Don't iterate if modifier is 1.0 (Clear)
    // However, if we skip 1.0, we rely on previous systems to have set the "clean" state.
    // Since this system is multiplicative, multiplying by 1.0 is safe.
    // If we skip, we just don't change anything, which is also correct (x * 1.0 = x).
    if (modifier - 1.0).abs() < f32::EPSILON {
        return;
    }

    for mut speed in &mut pops {
        speed.current *= modifier;
    }
}

#[cfg(test)]
mod tests {

    // StepRng generates values by starting at `initial` and adding `increment` each time.
    // For f32 mapping (0.0..1.0), it uses next_u32.
    // If we want a specific f32 `x` between 0 and 1, we set next_u32 to `x * 2^32`.
    // Or we can just use `rand_pcg::Pcg32` with different seeds to find the ones that map to our targets,
    // but building our own `RngCore` is easiest if we do it right.

    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_pick_weather_for_season_table_driven() {
        // We dynamically find seeds that produce exact float ranges to avoid
        // hardcoding brittle seeds or trying to reverse engineer rand math.

        let mut bucket_seeds = [0u64; 11];
        let targets = [
            (0.0, 0.35),    // < 0.4
            (0.4, 0.48),    // < 0.5
            (0.5, 0.58),    // < 0.6
            (0.6, 0.68),    // < 0.7
            (0.7, 0.78),    // < 0.8
            (0.81, 0.84),   // < 0.85
            (0.86, 0.865),  // Spring mutagenic slice is 0.87..0.90; 0.86..0.87 is Rain
            (0.91, 0.94),   // < 0.95
            (0.96, 0.99),   // >= 0.95
            (0.875, 0.895), // Spring MutagenicRain (0.87..0.90)
            (0.935, 0.945), // Summer MutagenicRain (0.93..0.95)
        ];

        for (i, (min, max)) in targets.iter().enumerate() {
            for seed in 0..100000 {
                let mut rng = StdRng::seed_from_u64(seed);
                // pick_weather_for_season calls `rng.gen_range(0.0..1.0)` but Rust infers
                // it is `f64` since the literal is `0.0..1.0` unless specified! Let's check `pick_weather_for_season`.
                // Wait, it says: `let roll = rng.gen_range(0.0..1.0);`
                // Rust infers `roll` type from usage: `roll < 0.6`. `0.6` is f64 by default.
                // Ah, the constants in the function are `0.6`, not `0.6_f32`.
                // Let's just generate using the exact same generic call.
                let val: f64 = rng.gen_range(0.0..1.0);
                if val >= *min && val <= *max {
                    bucket_seeds[i] = seed;
                    break;
                }
            }
        }

        let cases = vec![
            // Spring (Roll < 0.6, < 0.9)
            (Season::Spring, bucket_seeds[2], WeatherType::Clear), // 0.5 - 0.58
            (Season::Spring, bucket_seeds[4], WeatherType::Rain),  // 0.7 - 0.78
            (Season::Spring, bucket_seeds[8], WeatherType::Fog),   // 0.96 - 0.99
            // Spring MutagenicRain slice (0.87..0.90)
            (Season::Spring, bucket_seeds[9], WeatherType::MutagenicRain),
            // Summer (Roll < 0.7, < 0.85, < 0.93 Storm, < 0.95 MutagenicRain)
            (Season::Summer, bucket_seeds[3], WeatherType::Clear), // 0.6 - 0.68
            (Season::Summer, bucket_seeds[4], WeatherType::Heatwave), // 0.7 - 0.78
            (Season::Summer, bucket_seeds[6], WeatherType::Storm), // 0.86 - 0.865
            (Season::Summer, bucket_seeds[10], WeatherType::MutagenicRain), // 0.935 - 0.945
            (Season::Summer, bucket_seeds[8], WeatherType::MagneticStorm), // 0.96 - 0.99
            // Autumn (Roll < 0.5, < 0.8)
            (Season::Autumn, bucket_seeds[1], WeatherType::Clear), // 0.4 - 0.48
            (Season::Autumn, bucket_seeds[4], WeatherType::Rain),  // 0.7 - 0.78
            (Season::Autumn, bucket_seeds[5], WeatherType::Fog),   // 0.81 - 0.84
            // Winter (Roll < 0.4, < 0.8, < 0.9, < 0.95)
            (Season::Winter, bucket_seeds[0], WeatherType::Clear), // 0.0 - 0.35
            (Season::Winter, bucket_seeds[4], WeatherType::Snow),  // 0.7 - 0.78
            (
                Season::Winter,
                bucket_seeds[6],
                WeatherType::ThermalInversion,
            ), // 0.86 - 0.89
            (Season::Winter, bucket_seeds[7], WeatherType::Storm), // 0.91 - 0.94
            (Season::Winter, bucket_seeds[8], WeatherType::MagneticStorm), // 0.96 - 0.99
        ];

        for (season, seed, expected) in cases {
            let mut rng = StdRng::seed_from_u64(seed);
            let result = pick_weather_for_season(season, &mut rng);
            assert_eq!(
                result, expected,
                "Failed for Season: {:?}, Seed: {}",
                season, seed
            );
        }
    }

    use super::*;
    use crate::layer1::chronicle::Chronicle;
    use crate::layer1::pop::{Pop, Speed};
    use crate::layer1::seasons::{Season, SeasonState};
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_weather_state_default() {
        let state = WeatherState::default();
        assert_eq!(state.current_weather, WeatherType::Clear);
        assert!(state.duration_remaining > 0);
    }

    #[test]
    fn test_update_weather_system_decrements_duration() {
        let mut world = World::new();
        let initial_duration = 100;
        world.insert_resource(WeatherState {
            current_weather: WeatherType::Clear,
            duration_remaining: initial_duration,
        });
        world.insert_resource(SeasonState::default());
        world.insert_resource(Chronicle::default());
        world.insert_resource(SimulationTime::default());
        // Need to add events resource for Chronicle to work
        world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();

        world
            .run_system_once(update_weather_system)
            .expect("Failed to run update_weather_system");

        let state = world.resource::<WeatherState>();
        assert_eq!(state.duration_remaining, initial_duration - 1);
    }

    #[test]
    fn test_update_weather_system_changes_weather() {
        let mut world = World::new();
        world.insert_resource(WeatherState {
            current_weather: WeatherType::Clear,
            duration_remaining: 0, // Force update immediately
        });
        world.insert_resource(SeasonState {
            current_season: Season::Winter,
        });
        world.insert_resource(Chronicle::default());
        world.insert_resource(SimulationTime::default());
        world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();

        // Run multiple times to statistically likely hit a non-Clear weather if random
        // Or mock RNG. Here we just check *if* it changes, or if logic runs.
        // For RED phase, just running it and checking state change or event is enough.
        // Since we can't mock RNG easily without dependency injection, we'll check logic flow.

        world
            .run_system_once(update_weather_system)
            .expect("Failed to run update_weather_system");

        let state = world.resource::<WeatherState>();
        // Duration should be reset to > 0
        assert!(state.duration_remaining > 0);

        // We can't guarantee weather changed from Clear (it might pick Clear again),
        // but if it picked non-Clear, we should see an event.
        // Let's just check that duration reset, which implies logic ran.
    }

    #[test]
    fn test_apply_weather_effects_system_slows_movement() {
        let mut world = World::new();
        world.insert_resource(WeatherState {
            current_weather: WeatherType::Storm,
            duration_remaining: 100,
        });

        let pop = world
            .spawn((
                Pop,
                Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        world
            .run_system_once(apply_weather_effects_system)
            .expect("Failed to run apply_weather_effects_system");

        let speed = world
            .get::<Speed>(pop)
            .expect("Pop should have Speed component");
        // Storm modifier is 0.5. 1.0 * 0.5 = 0.5.
        assert!((speed.current - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mutagenic_rain_is_reachable_in_spring_and_summer_only() {
        // Spec 427 Leg 1: MutagenicRain must be rollable organically in
        // Spring/Summer (rare, ~2-3%) and never in Autumn/Winter.
        for season in [Season::Spring, Season::Summer] {
            let mut count = 0usize;
            for seed in 0..2000u64 {
                let mut rng = StdRng::seed_from_u64(seed);
                if pick_weather_for_season(season, &mut rng) == WeatherType::MutagenicRain {
                    count += 1;
                }
            }
            assert!(
                (10..=200).contains(&count),
                "{season:?} should roll MutagenicRain rarely (got {count}/2000)"
            );
        }
        for season in [Season::Autumn, Season::Winter] {
            for seed in 0..2000u64 {
                let mut rng = StdRng::seed_from_u64(seed);
                assert_ne!(
                    pick_weather_for_season(season, &mut rng),
                    WeatherType::MutagenicRain,
                    "{season:?} must never roll MutagenicRain"
                );
            }
        }
    }

    #[test]
    fn test_apply_weather_effects_system_multiplies_speed() {
        // This test replaces `test_apply_weather_effects_system_resets_speed`
        // ensuring compatibility with lighting penalties.
        let mut world = World::new();
        world.insert_resource(WeatherState {
            current_weather: WeatherType::Storm, // 0.5 modifier
            duration_remaining: 100,
        });

        let pop = world
            .spawn((
                Pop,
                Speed {
                    base: 1.0,
                    current: 0.5, // Already slowed by darkness/lighting
                    accumulator: 0.0,
                },
            ))
            .id();

        world
            .run_system_once(apply_weather_effects_system)
            .expect("Failed to run apply_weather_effects_system");

        let speed = world
            .get::<Speed>(pop)
            .expect("Pop should have Speed component");
        // Should be 0.5 (current) * 0.5 (weather) = 0.25
        assert!(
            (speed.current - 0.25).abs() < f32::EPSILON,
            "Expected 0.25, got {}",
            speed.current
        );
    }
}
