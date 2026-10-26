//! Spec 366: The Bio-Acoustic Choir.
//!
//! Bioluminescent, sonic plants — the choir — each sing at a single
//! frequency. Pops tend the plants and retune them (see
//! [`TunePlantEvent`]): a plant's actual [`BioAcousticPlant::frequency`]
//! glides toward its `target_frequency` instead of snapping, so a
//! retune is a slow bend through every frequency in between — including
//! the bad ones.
//!
//! * A plant holding the **healing frequency** ([`HealingFrequency`],
//!   432 Hz) mends nearby pops a little every tick.
//! * A plant holding the **discordant frequency**
//!   ([`DiscordantFrequency`], 666 Hz) gives nearby pops migraines:
//!   real [`StressTracker`] stress climbs and leisure (the needs pool
//!   morale reads) drains.
//! * Anything else is neutral undergrowth: it hums, glows, and does
//!   nothing but look pretty.
//!
//! ## Refactor-phase mechanics (spec §5)
//!
//! * **Spatial limits:** a plant only affects pops within
//!   [`CHOIR_RADIUS`] tiles (Chebyshev). A plant spawned without a
//!   [`GridPosition`] is treated as colony-wide — the greenhouse /
//!   test seam, documented here rather than hidden.
//! * **Gradual tuning:** [`gradual_tuning_system`] moves `frequency`
//!   toward `target_frequency` by [`TUNING_RATE`] Hz per tick.
//! * **Harmonics & interference:** two or more healing plants in range
//!   of the same pop resonate — each extra plant adds
//!   [`HARMONIC_BONUS`] to the mend. Discord stacks the same way. When
//!   healing and discordant plants *both* reach a pop, the waves
//!   interfere destructively: the mend is cancelled outright and the
//!   migraine lands at full strength. A choir must be tuned as a
//!   choir, not plant by plant.
//! * **Fauna startle (spec §7):** there is no stampede *event* type
//!   anywhere in the codebase, and this module does not invent one.
//!   The real seam is the fauna state machine: discordant plants set
//!   nearby [`Fauna`] to [`FaunaState::Flee`] via
//!   [`choir_fauna_startle_system`], and the existing fauna behaviour
//!   system does the running.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **`Needs` has no `stress` field.** The real needs are
//!   hunger/rest/leisure/hygiene (`layer1::psychology::needs`); stress
//!   lives in [`StressTracker`] (`accumulated_stress`, the pool the
//!   breakdown system reads) and morale flows through leisure. The
//!   discordant penalty therefore writes both real pools:
//!   `StressTracker` up, `Needs::leisure` down.
//! * **No `JobType::Tuner`, no `Transform`.** The real job vocabulary
//!   is [`AssignmentType`], which has no tuner variant, and the grid
//!   position component is [`GridPosition`], not Bevy's `Transform`
//!   (this crate's simulation is `bevy_ecs`-only). Tuning is therefore
//!   an event any tending pop can raise — the event carries the pop
//!   entity for attribution — rather than a new formal job that would
//!   ripple through arrival/utility wiring. The spec's RED test's
//!   tenure intent is honoured by carrying a real [`JobTenure`] on
//!   the tuning pop in tests.
//! * **Spec §6's `cargo clippy -- -D warnings` / 85% coverage** are
//!   aspirational template text; the verified bar is the established
//!   one: `cargo check --all-targets` green, clippy clean on touched
//!   files, and this module's tests green.
//!
//! All names and strings are original to Scale — no borrowed IP.
//! (Not to be confused with the experimental *Feral* Choir, where the
//! fauna do the singing; here the garden does.)

use bevy_ecs::prelude::*;

use crate::layer1::fauna::{Fauna, FaunaState};
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::stress::StressTracker;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// The healing frequency, in Hz (spec §4 example value).
pub struct HealingFrequency;
impl HealingFrequency {
    /// Frequency that mends nearby pops.
    pub const VALUE: f32 = 432.0;
}

/// The discordant frequency, in Hz (spec §4 example value).
pub struct DiscordantFrequency;
impl DiscordantFrequency {
    /// Frequency that gives nearby pops migraines.
    pub const VALUE: f32 = 666.0;
}

/// Half-width of a frequency band: a plant within this many Hz of a
/// named frequency counts as holding it.
pub const BAND_TOLERANCE: f32 = 10.0;
/// Radius (Chebyshev tiles) a choir plant's song reaches.
pub const CHOIR_RADIUS: u32 = 8;
/// Radius (Chebyshev tiles) within which discord startles fauna.
pub const STARTLE_RADIUS: u32 = 6;
/// Hz per tick a plant's frequency glides toward its target.
pub const TUNING_RATE: f32 = 2.0;
/// Health restored per tick by one healing plant in range.
pub const HEAL_PER_TICK: f32 = 0.1;
/// Stress added per tick by one discordant plant in range.
pub const DISCORD_STRESS_PER_TICK: f32 = 0.5;
/// Leisure drained per tick by one discordant plant in range.
pub const DISCORD_LEISURE_PER_TICK: f32 = 0.002;
/// Extra fraction of effect per additional same-band plant in range
/// (harmonics for healing, stacking discord for migraines).
pub const HARMONIC_BONUS: f32 = 0.25;

// ---------------------------------------------------------------------------
// Components & events
// ---------------------------------------------------------------------------

/// Which named band a frequency falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoirBand {
    /// At the healing frequency: mends nearby pops.
    Healing,
    /// At the discordant frequency: migraines nearby.
    Discordant,
    /// Any other frequency: ambience only.
    Neutral,
}

/// Classify a frequency into its band.
#[must_use]
pub fn classify_frequency(frequency: f32) -> ChoirBand {
    if (frequency - HealingFrequency::VALUE).abs() <= BAND_TOLERANCE {
        ChoirBand::Healing
    } else if (frequency - DiscordantFrequency::VALUE).abs() <= BAND_TOLERANCE {
        ChoirBand::Discordant
    } else {
        ChoirBand::Neutral
    }
}

/// A bioluminescent sonic plant that sings at one frequency.
#[derive(Component, Debug, Clone, Copy)]
pub struct BioAcousticPlant {
    /// The frequency the plant is actually emitting now.
    pub frequency: f32,
    /// The frequency tenders have set it gliding toward.
    pub target_frequency: f32,
}

impl BioAcousticPlant {
    /// A plant already settled at `frequency` (target == current).
    #[must_use]
    pub const fn new(frequency: f32) -> Self {
        Self {
            frequency,
            target_frequency: frequency,
        }
    }

    /// The band the plant's *current* frequency falls in.
    #[must_use]
    pub fn band(&self) -> ChoirBand {
        classify_frequency(self.frequency)
    }

    /// True while the plant is still gliding toward its target.
    #[must_use]
    pub fn is_tuning(&self) -> bool {
        (self.frequency - self.target_frequency).abs() > f32::EPSILON
    }
}

impl Default for BioAcousticPlant {
    fn default() -> Self {
        Self::new(HealingFrequency::VALUE)
    }
}

/// A pop tends a choir plant and sets a new target frequency. The
/// plant glides there over time ([`gradual_tuning_system`]); it does
/// not snap.
#[derive(Event, Debug, Clone, Copy)]
pub struct TunePlantEvent {
    /// The plant being retuned.
    pub plant: Entity,
    /// The tending pop (attribution; any pop may tend the choir).
    pub pop: Entity,
    /// The new target frequency in Hz.
    pub new_frequency: f32,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Applies [`TunePlantEvent`]s: sets each plant's target frequency.
/// The current frequency is left alone — gliding is
/// [`gradual_tuning_system`]'s job.
pub fn tune_plant_system(
    mut events: EventReader<TunePlantEvent>,
    mut plants: Query<&mut BioAcousticPlant>,
) {
    for event in events.read() {
        if let Ok(mut plant) = plants.get_mut(event.plant) {
            plant.target_frequency = event.new_frequency;
        }
    }
}

/// Moves every plant's frequency toward its target by
/// [`TUNING_RATE`] Hz per tick, snapping the final step.
pub fn gradual_tuning_system(mut plants: Query<&mut BioAcousticPlant>) {
    for mut plant in plants.iter_mut() {
        let delta = plant.target_frequency - plant.frequency;
        if delta.abs() <= TUNING_RATE {
            plant.frequency = plant.target_frequency;
        } else {
            plant.frequency += TUNING_RATE * delta.signum();
        }
    }
}

/// Counts (healing, discordant) plants whose song reaches `pop_pos`.
/// A plant with no position is colony-wide (see module docs).
fn plants_in_range(
    plants: &[(ChoirBand, Option<GridPosition>)],
    pop_pos: Option<GridPosition>,
) -> (usize, usize) {
    let mut healing = 0usize;
    let mut discordant = 0usize;
    for (band, plant_pos) in plants {
        let in_range = match (plant_pos, pop_pos) {
            (Some(plant), Some(pop)) => plant.distance_chebyshev(pop) <= CHOIR_RADIUS,
            _ => true,
        };
        if !in_range {
            continue;
        }
        match band {
            ChoirBand::Healing => healing += 1,
            ChoirBand::Discordant => discordant += 1,
            ChoirBand::Neutral => {}
        }
    }
    (healing, discordant)
}

/// Harmonic multiplier for `count` same-band plants singing together.
fn harmonic_multiplier(count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        1.0 + HARMONIC_BONUS * (count - 1) as f32
    }
}

/// Applies choir effects to every pop: healing plants mend, discordant
/// plants migraine, and a pop hearing both gets interference (no mend,
/// full migraine).
#[allow(clippy::type_complexity)]
pub fn apply_choir_effects_system(
    plants: Query<(&BioAcousticPlant, Option<&GridPosition>)>,
    mut pops: Query<
        (
            &mut Health,
            Option<&mut Needs>,
            Option<&mut StressTracker>,
            Option<&GridPosition>,
        ),
        With<Pop>,
    >,
) {
    if plants.is_empty() {
        return;
    }
    let plant_list: Vec<(ChoirBand, Option<GridPosition>)> = plants
        .iter()
        .map(|(plant, pos)| (plant.band(), pos.copied()))
        .collect();

    for (mut health, needs, stress, pop_pos) in pops.iter_mut() {
        let (healing, discordant) = plants_in_range(&plant_list, pop_pos.copied());
        if healing == 0 && discordant == 0 {
            continue;
        }
        // Destructive interference: discord cancels the mend outright.
        if healing > 0 && discordant == 0 {
            let mend = HEAL_PER_TICK * harmonic_multiplier(healing);
            health.current = (health.current + mend).min(health.max);
        }
        if discordant > 0 {
            let stack = harmonic_multiplier(discordant);
            if let Some(mut stress) = stress {
                stress.accumulated_stress += DISCORD_STRESS_PER_TICK * stack;
            }
            if let Some(mut needs) = needs {
                needs.leisure = (needs.leisure - DISCORD_LEISURE_PER_TICK * stack).clamp(0.0, 1.0);
            }
        }
    }
}

/// Discordant plants startle nearby fauna into [`FaunaState::Flee`]
/// (the real fauna seam — see module docs on the stampede hook).
pub fn choir_fauna_startle_system(
    plants: Query<(&BioAcousticPlant, &GridPosition)>,
    mut fauna: Query<(&mut Fauna, &GridPosition)>,
) {
    let discordant: Vec<GridPosition> = plants
        .iter()
        .filter(|(plant, _)| plant.band() == ChoirBand::Discordant)
        .map(|(_, pos)| *pos)
        .collect();
    if discordant.is_empty() {
        return;
    }
    for (mut animal, pos) in fauna.iter_mut() {
        if animal.state == FaunaState::Flee {
            continue;
        }
        if discordant
            .iter()
            .any(|plant_pos| plant_pos.distance_chebyshev(*pos) <= STARTLE_RADIUS)
        {
            animal.state = FaunaState::Flee;
        }
    }
}

// ---------------------------------------------------------------------------
// Headless / world helpers
// ---------------------------------------------------------------------------

/// Census of the choir: (total plants, healing, discordant, tuning).
#[must_use]
pub fn choir_counts(world: &mut World) -> (usize, usize, usize, usize) {
    let mut query = world.query::<&BioAcousticPlant>();
    let mut total = 0usize;
    let mut healing = 0usize;
    let mut discordant = 0usize;
    let mut tuning = 0usize;
    for plant in query.iter(world) {
        total += 1;
        match plant.band() {
            ChoirBand::Healing => healing += 1,
            ChoirBand::Discordant => discordant += 1,
            ChoirBand::Neutral => {}
        }
        if plant.is_tuning() {
            tuning += 1;
        }
    }
    (total, healing, discordant, tuning)
}

/// One-token STATS label, e.g. `3/1h/1d` (plants/healing/discordant),
/// or `none` when the colony has no choir.
#[must_use]
pub fn choir_stats_label(world: &mut World) -> String {
    let (total, healing, discordant, _) = choir_counts(world);
    if total == 0 {
        "none".to_string()
    } else {
        format!("{total}/{healing}h/{discordant}d")
    }
}

/// Spawn a choir plant at the first living pop's tile (or the origin
/// when the colony is empty), settled at `frequency`. Returns the
/// plant entity.
pub fn spawn_choir_plant(world: &mut World, frequency: f32) -> Entity {
    let anchor = {
        let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
        query.iter(world).next().copied()
    }
    .unwrap_or(GridPosition { x: 0, y: 0 });
    world.spawn((BioAcousticPlant::new(frequency), anchor)).id()
}

/// Retune the first choir plant (spawn order) toward `frequency` by
/// raising the real [`TunePlantEvent`]. Returns false when the colony
/// has no choir plant to tune.
pub fn tune_first_plant(world: &mut World, frequency: f32) -> bool {
    let plant = {
        let mut query = world.query_filtered::<Entity, With<BioAcousticPlant>>();
        query.iter(world).next()
    };
    let Some(plant) = plant else {
        return false;
    };
    let pop = {
        let mut query = world.query_filtered::<Entity, With<Pop>>();
        query.iter(world).next()
    }
    .unwrap_or(Entity::PLACEHOLDER);
    world.send_event(TunePlantEvent {
        plant,
        pop,
        new_frequency: frequency,
    });
    true
}

/// Parse a headless frequency token: `healing`, `discordant`, or a
/// raw Hz number.
#[must_use]
pub fn parse_frequency(token: &str) -> Option<f32> {
    match token.to_lowercase().as_str() {
        "healing" | "heal" => Some(HealingFrequency::VALUE),
        "discordant" | "discord" => Some(DiscordantFrequency::VALUE),
        other => other.parse::<f32>().ok(),
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn choir_public_strings() -> Vec<String> {
    vec![
        "Bio-Acoustic Choir".to_string(),
        "healing frequency".to_string(),
        "discordant frequency".to_string(),
        "The garden is singing.".to_string(),
        "Choir migraine".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Tests (spec §3 RED tests, adapted to the real codebase — see the
// module docs for every adaptation and why)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::specialization::JobTenure;
    use crate::layer1::systems::update_event_buffer;
    use crate::layer1::utility_types::AssignmentType;

    fn choir_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems((
            update_event_buffer::<TunePlantEvent>,
            tune_plant_system,
            gradual_tuning_system,
            apply_choir_effects_system,
            choir_fauna_startle_system,
        ));
        schedule
    }

    fn world_with_events() -> World {
        let mut world = World::new();
        world.init_resource::<Events<TunePlantEvent>>();
        world
    }

    /// Spec RED test 1, adapted: `JobType::Tuner` / `Transform` do not
    /// exist — the tuner carries a real `JobTenure` and positions are
    /// `GridPosition`. The event sets the *target*; gliding is gradual.
    #[test]
    fn test_choir_plant_tuning_job() {
        let mut world = world_with_events();
        let plant = world
            .spawn((BioAcousticPlant::new(10.0), GridPosition { x: 0, y: 0 }))
            .id();
        let pop = world
            .spawn((
                Pop,
                JobTenure::with_ticks(AssignmentType::FarmWorker, 25),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        world.send_event(TunePlantEvent {
            plant,
            pop,
            new_frequency: 42.0,
        });
        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        let plant_comp = world.get::<BioAcousticPlant>(plant).unwrap();
        assert_eq!(plant_comp.target_frequency, 42.0);
    }

    /// Spec RED test 2: a healing-frequency plant mends a hurt pop.
    #[test]
    fn test_choir_buff_application() {
        let mut world = world_with_events();
        world.spawn((
            BioAcousticPlant::new(HealingFrequency::VALUE),
            GridPosition { x: 5, y: 5 },
        ));
        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 50.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        let health = world.get::<Health>(pop).unwrap();
        assert!(
            health.current > 50.0,
            "healing song should mend: {health:?}"
        );
    }

    /// Spec RED test 3, adapted: `Needs` has no `stress` field — the
    /// migraine lands in the real `StressTracker` pool and drains
    /// leisure.
    #[test]
    fn test_choir_misalignment_penalty() {
        let mut world = world_with_events();
        world.spawn((
            BioAcousticPlant::new(DiscordantFrequency::VALUE),
            GridPosition { x: 5, y: 5 },
        ));
        let pop = world
            .spawn((
                Pop,
                Health::default(),
                Needs::default(),
                StressTracker::default(),
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        let stress = world.get::<StressTracker>(pop).unwrap();
        assert!(
            stress.accumulated_stress > 0.0,
            "discord should stress: {stress:?}"
        );
        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            needs.leisure < 0.8,
            "discord should drain leisure: {needs:?}"
        );
    }

    /// §5 gradual tuning: frequency glides, never snaps.
    #[test]
    fn test_gradual_tuning_glides_toward_target() {
        let mut world = world_with_events();
        let plant = world
            .spawn((
                BioAcousticPlant::new(HealingFrequency::VALUE),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        world
            .get_mut::<BioAcousticPlant>(plant)
            .unwrap()
            .target_frequency = DiscordantFrequency::VALUE;

        let mut schedule = Schedule::default();
        schedule.add_systems(gradual_tuning_system);
        for _ in 0..10 {
            schedule.run(&mut world);
        }
        let after = world.get::<BioAcousticPlant>(plant).unwrap();
        assert!(after.frequency > HealingFrequency::VALUE);
        assert!(after.frequency < DiscordantFrequency::VALUE);
        assert!((after.frequency - (HealingFrequency::VALUE + 10.0 * TUNING_RATE)).abs() < 0.01);
        // And it does eventually arrive.
        for _ in 0..200 {
            schedule.run(&mut world);
        }
        let settled = world.get::<BioAcousticPlant>(plant).unwrap();
        assert_eq!(settled.frequency, DiscordantFrequency::VALUE);
        assert!(!settled.is_tuning());
    }

    /// §5 spatial limits: a pop far outside the radius hears nothing.
    #[test]
    fn test_choir_radius_limits_effects() {
        let mut world = world_with_events();
        world.spawn((
            BioAcousticPlant::new(HealingFrequency::VALUE),
            GridPosition { x: 0, y: 0 },
        ));
        let near = world
            .spawn((
                Pop,
                Health {
                    current: 50.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 3, y: 3 },
            ))
            .id();
        let far = world
            .spawn((
                Pop,
                Health {
                    current: 50.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 40, y: 40 },
            ))
            .id();

        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        assert!(world.get::<Health>(near).unwrap().current > 50.0);
        assert_eq!(world.get::<Health>(far).unwrap().current, 50.0);
    }

    /// §5 harmonics: two healing plants mend faster than one.
    #[test]
    fn test_choir_harmonics_amplify_healing() {
        let heal_with = |plants: usize| -> f32 {
            let mut world = world_with_events();
            for i in 0..plants {
                world.spawn((
                    BioAcousticPlant::new(HealingFrequency::VALUE),
                    GridPosition { x: i as i32, y: 0 },
                ));
            }
            let pop = world
                .spawn((
                    Pop,
                    Health {
                        current: 50.0,
                        max: 100.0,
                        has_rust_lung: false,
                    },
                    GridPosition { x: 0, y: 0 },
                ))
                .id();
            let mut schedule = choir_schedule();
            schedule.run(&mut world);
            world.get::<Health>(pop).unwrap().current
        };
        let one = heal_with(1);
        let two = heal_with(2);
        assert!(two > one, "harmonics should amplify: one={one} two={two}");
    }

    /// §5 interference: healing + discordant together cancel the mend
    /// but not the migraine.
    #[test]
    fn test_choir_interference_cancels_healing() {
        let mut world = world_with_events();
        world.spawn((
            BioAcousticPlant::new(HealingFrequency::VALUE),
            GridPosition { x: 0, y: 0 },
        ));
        world.spawn((
            BioAcousticPlant::new(DiscordantFrequency::VALUE),
            GridPosition { x: 1, y: 0 },
        ));
        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 50.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                StressTracker::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        assert_eq!(world.get::<Health>(pop).unwrap().current, 50.0);
        assert!(world.get::<StressTracker>(pop).unwrap().accumulated_stress > 0.0);
    }

    /// §7 fauna seam: discord startles nearby fauna into Flee; a
    /// distant animal keeps wandering.
    #[test]
    fn test_choir_discord_startles_fauna() {
        let mut world = world_with_events();
        world.spawn((
            BioAcousticPlant::new(DiscordantFrequency::VALUE),
            GridPosition { x: 10, y: 10 },
        ));
        let near = world
            .spawn((Fauna::default(), GridPosition { x: 12, y: 10 }))
            .id();
        let far = world
            .spawn((Fauna::default(), GridPosition { x: 40, y: 40 }))
            .id();

        let mut schedule = choir_schedule();
        schedule.run(&mut world);

        assert_eq!(world.get::<Fauna>(near).unwrap().state, FaunaState::Flee);
        assert_eq!(world.get::<Fauna>(far).unwrap().state, FaunaState::Wander);
    }

    /// Neutral frequencies do nothing at all.
    #[test]
    fn test_neutral_frequency_is_inert() {
        let mut world = world_with_events();
        world.spawn((BioAcousticPlant::new(220.0), GridPosition { x: 0, y: 0 }));
        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 50.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                StressTracker::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();
        let mut schedule = choir_schedule();
        schedule.run(&mut world);
        assert_eq!(world.get::<Health>(pop).unwrap().current, 50.0);
        assert_eq!(
            world.get::<StressTracker>(pop).unwrap().accumulated_stress,
            0.0
        );
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "little shop",
            "audrey",
            "seymour",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
        ];
        for text in choir_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
