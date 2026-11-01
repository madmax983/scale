//! Spec 385: Terraforming Backlash — the planet pushes back.
//!
//! Aggressive terraforming (rapid heating / rapid detox through the
//! colony's Atmospheric Processors) triggers Planetary Defense
//! backlash: super-storms, seismic unrest, or dormant mega-fauna
//! waking beneath the colony.
//!
//! Adaptations from the spec sketch (§8 grounding, following the
//! 336/354/356/384 pattern):
//! * The spec's `TerraformingState { temperature, last_temperature }`
//!   does not exist and no parallel state is invented. The real,
//!   live-written terraforming state is
//!   [`PlanetaryAtmosphere`](crate::layer1::environment::terraforming::PlanetaryAtmosphere)
//!   (`temperature`, `toxicity`), driven every tick by
//!   `update_planetary_atmosphere_system` from powered
//!   `BuildingType::AtmosphericProcessor` buildings (+0.001 C and
//!   -0.0001 toxicity per processor per tick). This module snapshots
//!   that resource on a throttled interval — the spec §5 REFACTOR's
//!   `TerraformingVelocity` over a time window, not a per-tick
//!   `last_temperature` field.
//! * There is no global pressure resource to add as a third vector:
//!   pressure in this codebase is the local `PressureGrid` /
//!   `ColonyAtmosphere` habitat system, which terraforming never
//!   writes. The generalized stress therefore spans the two vectors
//!   terraforming actually moves — temperature and toxicity — each
//!   normalized against its own backlash threshold.
//! * Hazards dispatch through real systems, never bespoke damage:
//!   super-storms set the real [`WeatherState`] to
//!   [`WeatherType::Storm`] (speed penalties flow through
//!   `apply_weather_effects_system`), seismic backlash sends the real
//!   [`GeologicalEvent`] (damage + screen shake flow through
//!   `apply_geological_event_system`), and mega-fauna backlash spawns
//!   a real [`Fauna`] entity the fauna AI hunts with.
//! * Chronicle feedback goes through [`AddChronicleEvent`], and
//!   [`BacklashLedger`] counts events for the headless panel / STATS.

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::environment::terraforming::PlanetaryAtmosphere;
use crate::layer1::fauna::{Fauna, FaunaType};
use crate::layer1::geology::GeologicalEvent;
use crate::layer1::health::Health;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::weather::{WeatherState, WeatherType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Ticks between backlash checks. Terraforming is slow work; checking
/// every tick would measure noise, and the spec §5 REFACTOR explicitly
/// asks for a slow interval. At the real processor rate
/// (+0.001 C/tick each), even a hundred processors move only 5.0 C per
/// interval — backlash needs a genuinely aggressive build-out.
pub const CHECK_INTERVAL_TICKS: u64 = 50;

/// Temperature change per check interval (degrees C) at which the
/// planet starts pushing back. The spec's RED threshold, kept.
pub const TEMP_BACKLASH_THRESHOLD: f32 = 5.0;

/// Toxicity change per check interval at which the planet starts
/// pushing back. Normal detox is -0.0001/tick per processor
/// (-0.005/interval), so this is fifty processors' worth of violence.
pub const TOXICITY_BACKLASH_THRESHOLD: f32 = 0.25;

/// Which Planetary Defense answers a backlash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BacklashKind {
    /// A super-storm: the real weather state slams to Storm.
    SuperStorm,
    /// Seismic unrest: a real geological event under the colony.
    SeismicUnrest,
    /// Dormant mega-fauna wakes: a real hostile fauna spawns.
    DormantMegafauna,
}

impl BacklashKind {
    /// Short display label (original name, IP-guard tested).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::SuperStorm => "Super-Storm",
            Self::SeismicUnrest => "Seismic Unrest",
            Self::DormantMegafauna => "Dormant Mega-Fauna",
        }
    }

    /// Parse a headless kind name (case-insensitive, aliases ok).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "storm" | "superstorm" | "super-storm" => Some(Self::SuperStorm),
            "seismic" | "quake" | "earthquake" | "unrest" => Some(Self::SeismicUnrest),
            "fauna" | "megafauna" | "mega-fauna" | "beast" => Some(Self::DormantMegafauna),
            _ => None,
        }
    }
}

/// Event fired when terraforming velocity crosses the backlash
/// threshold. `severity` is the dominant raw delta (the spec sketch's
/// field); `stress` is the generalized normalized stress (>= 1.0).
#[derive(Event, Debug, Clone, Copy)]
pub struct BacklashEvent {
    /// The dominant raw change that triggered the backlash.
    pub severity: f32,
    /// Generalized stress across the temperature + toxicity vectors.
    pub stress: f32,
    /// Which defense the planet answers with.
    pub kind: BacklashKind,
}

/// Velocity snapshot of the real [`PlanetaryAtmosphere`]: the values
/// seen at the previous check, plus when that check happened. This is
/// a measurement window over live state, not a second terraforming
/// state — nothing writes terraforming values here.
#[derive(Resource, Debug, Clone)]
pub struct BacklashTracker {
    /// Temperature at the previous check.
    pub last_temperature: f32,
    /// Toxicity at the previous check.
    pub last_toxicity: f32,
    /// Tick of the previous check.
    pub last_check_tick: u64,
    /// Whether the snapshot has been seeded from live state.
    pub initialized: bool,
}

impl Default for BacklashTracker {
    fn default() -> Self {
        Self {
            last_temperature: 0.0,
            last_toxicity: 0.0,
            last_check_tick: 0,
            initialized: false,
        }
    }
}

/// Ledger of backlash events for the headless panel and STATS.
#[derive(Resource, Debug, Clone, Default)]
pub struct BacklashLedger {
    /// Total backlash events fired.
    pub total: u32,
    /// Super-storms among them.
    pub storms: u32,
    /// Seismic events among them.
    pub quakes: u32,
    /// Mega-fauna awakenings among them.
    pub fauna_awakenings: u32,
    /// Tick of the most recent backlash (0 = none yet).
    pub last_tick: u64,
    /// Peak generalized stress observed.
    pub peak_stress: f32,
}

/// Generalized terraforming stress across the temperature and
/// toxicity vectors: each delta normalized by its own threshold.
/// Stress >= 1.0 means at least one vector moved violently.
#[must_use]
pub fn terraforming_stress(temp_delta: f32, toxicity_delta: f32) -> f32 {
    let temp_stress = temp_delta.abs() / TEMP_BACKLASH_THRESHOLD;
    let toxicity_stress = toxicity_delta.abs() / TOXICITY_BACKLASH_THRESHOLD;
    temp_stress.max(toxicity_stress)
}

/// Picks the planet's answer from which vector dominates. A pure
/// function so the mapping is unit-testable without a world.
#[must_use]
pub fn kind_for_stress(temp_delta: f32, toxicity_delta: f32, prior_events: u32) -> BacklashKind {
    let temp_stress = temp_delta.abs() / TEMP_BACKLASH_THRESHOLD;
    let toxicity_stress = toxicity_delta.abs() / TOXICITY_BACKLASH_THRESHOLD;
    if toxicity_stress > temp_stress * 1.5 {
        // Violent detox wakes what slept in the old atmosphere.
        BacklashKind::DormantMegafauna
    } else if temp_stress >= 1.0 && toxicity_stress >= 0.5 {
        // Both vectors screaming at once: the ground itself answers.
        BacklashKind::SeismicUnrest
    } else if prior_events % 3 == 2 {
        // Every third heat-driven answer comes from below instead.
        BacklashKind::SeismicUnrest
    } else {
        BacklashKind::SuperStorm
    }
}

/// Throttled backlash check: every [`CHECK_INTERVAL_TICKS`] ticks,
/// compare the real [`PlanetaryAtmosphere`] against the velocity
/// snapshot and fire a [`BacklashEvent`] when the generalized stress
/// crosses 1.0. The snapshot always advances on a check, so one
/// violent interval fires once, not every tick until it decays.
pub fn check_terraforming_backlash_system(
    time: Res<SimulationTime>,
    atmosphere: Res<PlanetaryAtmosphere>,
    mut tracker: ResMut<BacklashTracker>,
    ledger: Res<BacklashLedger>,
    mut events: EventWriter<BacklashEvent>,
) {
    if !tracker.initialized {
        tracker.last_temperature = atmosphere.temperature;
        tracker.last_toxicity = atmosphere.toxicity;
        tracker.last_check_tick = time.tick;
        tracker.initialized = true;
        return;
    }
    if time.tick == 0 || !time.tick.is_multiple_of(CHECK_INTERVAL_TICKS) {
        return;
    }
    if time.tick <= tracker.last_check_tick {
        return;
    }

    let temp_delta = atmosphere.temperature - tracker.last_temperature;
    let toxicity_delta = atmosphere.toxicity - tracker.last_toxicity;
    tracker.last_temperature = atmosphere.temperature;
    tracker.last_toxicity = atmosphere.toxicity;
    tracker.last_check_tick = time.tick;

    let stress = terraforming_stress(temp_delta, toxicity_delta);
    if stress >= 1.0 {
        let severity = temp_delta.abs().max(toxicity_delta.abs() * 20.0);
        events.send(BacklashEvent {
            severity,
            stress,
            kind: kind_for_stress(temp_delta, toxicity_delta, ledger.total),
        });
    }
}

/// Answers each [`BacklashEvent`] through the real hazard systems and
/// records it in the ledger + chronicle.
#[allow(clippy::too_many_arguments)]
pub fn handle_backlash_system(
    mut events: EventReader<BacklashEvent>,
    time: Res<SimulationTime>,
    mut ledger: ResMut<BacklashLedger>,
    mut weather: ResMut<WeatherState>,
    mut quakes: EventWriter<GeologicalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut commands: Commands,
    pops: Query<&GridPosition, With<Pop>>,
) {
    for event in events.read() {
        ledger.total += 1;
        ledger.last_tick = time.tick;
        ledger.peak_stress = ledger.peak_stress.max(event.stress);

        // The colony heart: the first living pop's tile, with a
        // fixed fallback for an empty world (debug spawns).
        let heart = pops.iter().next().copied().unwrap_or(GridPosition { x: 32, y: 32 });

        let description = match event.kind {
            BacklashKind::SuperStorm => {
                ledger.storms += 1;
                weather.current_weather = WeatherType::Storm;
                weather.duration_remaining =
                    weather.duration_remaining.max(100 + (event.severity * 10.0) as u32);
                "a super-storm rolls over the colony".to_string()
            }
            BacklashKind::SeismicUnrest => {
                ledger.quakes += 1;
                quakes.send(GeologicalEvent {
                    center: heart,
                    magnitude: (event.severity / 5.0).clamp(1.0, 3.0),
                });
                "the ground heaves beneath the colony".to_string()
            }
            BacklashKind::DormantMegafauna => {
                ledger.fauna_awakenings += 1;
                let den = GridPosition {
                    x: heart.x + 6,
                    y: heart.y + 6,
                };
                commands.spawn((
                    Fauna {
                        fauna_type: FaunaType::Wolf,
                        detection_range: 12.0,
                        ..Default::default()
                    },
                    den,
                    Health {
                        current: 200.0,
                        max: 200.0,
                        has_rust_lung: false,
                    },
                ));
                "something vast wakes beneath the colony and climbs toward the light"
                    .to_string()
            }
        };

        chronicle.send(AddChronicleEvent {
            text: format!(
                "PLANETARY DEFENSE: {} — {} (terraforming stress {:.1}x). The planet has an immune system, and the colony is the fever.",
                event.kind.label(),
                description,
                event.stress
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Census for STATS / the headless panel: `(total, storms, quakes,
/// fauna_awakenings)` from the ledger, zeroes when absent.
#[must_use]
pub fn backlash_stats_label(world: &World) -> String {
    world.get_resource::<BacklashLedger>().map_or_else(
        || "0/0/0/0".to_string(),
        |ledger| {
            format!(
                "{}/{}/{}/{}",
                ledger.total, ledger.storms, ledger.quakes, ledger.fauna_awakenings
            )
        },
    )
}

/// Headless/debug seam: force the tracker snapshot so the next check
/// measures from the given baseline, then optionally jump the real
/// atmosphere. Used by the `backlash` console trigger; returns false
/// when the world has no planetary atmosphere yet.
pub fn prime_backlash_snapshot(world: &mut World, temperature: f32, toxicity: f32) -> bool {
    if !world.contains_resource::<PlanetaryAtmosphere>() {
        return false;
    }
    let tick = world.resource::<SimulationTime>().tick;
    world.insert_resource(BacklashTracker {
        last_temperature: temperature,
        last_toxicity: toxicity,
        last_check_tick: tick,
        initialized: true,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backlash_world(temperature: f32, toxicity: f32, baseline_temp: f32, baseline_tox: f32) -> World {
        let mut world = World::new();
        world.insert_resource(PlanetaryAtmosphere {
            toxicity,
            temperature,
        });
        world.insert_resource(SimulationTime {
            tick: CHECK_INTERVAL_TICKS,
            ..Default::default()
        });
        world.insert_resource(BacklashTracker {
            last_temperature: baseline_temp,
            last_toxicity: baseline_tox,
            last_check_tick: 0,
            initialized: true,
        });
        world.init_resource::<BacklashLedger>();
        world.init_resource::<Events<BacklashEvent>>();
        world
    }

    fn count_backlash(world: &World) -> usize {
        let events = world.resource::<Events<BacklashEvent>>();
        let mut reader = events.get_cursor();
        reader.read(events).count()
    }

    // Spec RED test 1, grounded: a 10-degree jump in one check cycle
    // against the real PlanetaryAtmosphere triggers one backlash.
    #[test]
    fn test_rapid_temperature_change_triggers_backlash() {
        let mut world = backlash_world(20.0, 0.8, 10.0, 0.8);
        let mut schedule = Schedule::default();
        schedule.add_systems(check_terraforming_backlash_system);
        schedule.run(&mut world);
        assert_eq!(
            count_backlash(&world),
            1,
            "Backlash event should be triggered for rapid heating"
        );
    }

    // Spec RED test 2, grounded: a 1-degree drift is safe.
    #[test]
    fn test_slow_temperature_change_is_safe() {
        let mut world = backlash_world(11.0, 0.8, 10.0, 0.8);
        let mut schedule = Schedule::default();
        schedule.add_systems(check_terraforming_backlash_system);
        schedule.run(&mut world);
        assert_eq!(
            count_backlash(&world),
            0,
            "No backlash for slow changes"
        );
    }

    #[test]
    fn test_rapid_toxicity_change_triggers_fauna_backlash() {
        let mut world = backlash_world(-20.0, 0.2, -20.0, 0.8);
        let mut schedule = Schedule::default();
        schedule.add_systems(check_terraforming_backlash_system);
        schedule.run(&mut world);
        let events = world.resource::<Events<BacklashEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].kind, BacklashKind::DormantMegafauna);
    }

    #[test]
    fn test_check_is_throttled_between_intervals() {
        // Same violent delta, but the tick is not a check boundary:
        // nothing fires and the snapshot does not advance.
        let mut world = backlash_world(20.0, 0.8, 10.0, 0.8);
        world.resource_mut::<SimulationTime>().tick = CHECK_INTERVAL_TICKS + 1;
        let mut schedule = Schedule::default();
        schedule.add_systems(check_terraforming_backlash_system);
        schedule.run(&mut world);
        assert_eq!(count_backlash(&world), 0);
        assert_eq!(world.resource::<BacklashTracker>().last_check_tick, 0);
    }

    #[test]
    fn test_first_run_only_seeds_snapshot() {
        let mut world = World::new();
        world.insert_resource(PlanetaryAtmosphere {
            toxicity: 0.0,
            temperature: 90.0,
        });
        world.insert_resource(SimulationTime {
            tick: CHECK_INTERVAL_TICKS,
            ..Default::default()
        });
        world.init_resource::<BacklashTracker>();
        world.init_resource::<BacklashLedger>();
        world.init_resource::<Events<BacklashEvent>>();
        let mut schedule = Schedule::default();
        schedule.add_systems(check_terraforming_backlash_system);
        schedule.run(&mut world);
        assert_eq!(count_backlash(&world), 0);
        assert!(world.resource::<BacklashTracker>().initialized);
    }

    #[test]
    fn test_stress_generalizes_across_vectors() {
        assert!(terraforming_stress(5.0, 0.0) >= 1.0);
        assert!(terraforming_stress(0.0, 0.25) >= 1.0);
        assert!(terraforming_stress(2.5, 0.1) < 1.0);
        assert!(terraforming_stress(-10.0, 0.0) >= 1.0, "rapid cooling counts too");
    }

    fn handler_world() -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime {
            tick: 500,
            ..Default::default()
        });
        world.init_resource::<BacklashLedger>();
        world.init_resource::<WeatherState>();
        world.init_resource::<Events<BacklashEvent>>();
        world.init_resource::<Events<GeologicalEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    #[test]
    fn test_storm_backlash_sets_real_weather() {
        let mut world = handler_world();
        world.send_event(BacklashEvent {
            severity: 10.0,
            stress: 2.0,
            kind: BacklashKind::SuperStorm,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_backlash_system);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Storm
        );
        assert_eq!(world.resource::<BacklashLedger>().storms, 1);
    }

    #[test]
    fn test_seismic_backlash_sends_real_geological_event() {
        let mut world = handler_world();
        world.spawn((Pop, GridPosition { x: 7, y: 9 }));
        world.send_event(BacklashEvent {
            severity: 10.0,
            stress: 2.0,
            kind: BacklashKind::SeismicUnrest,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_backlash_system);
        schedule.run(&mut world);
        let quakes = world.resource::<Events<GeologicalEvent>>();
        let mut reader = quakes.get_cursor();
        let fired: Vec<_> = reader.read(quakes).collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].center, GridPosition { x: 7, y: 9 });
        assert_eq!(world.resource::<BacklashLedger>().quakes, 1);
    }

    #[test]
    fn test_fauna_backlash_spawns_real_fauna() {
        let mut world = handler_world();
        world.spawn((Pop, GridPosition { x: 7, y: 9 }));
        world.send_event(BacklashEvent {
            severity: 10.0,
            stress: 2.0,
            kind: BacklashKind::DormantMegafauna,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_backlash_system);
        schedule.run(&mut world);
        let mut query = world.query::<(&Fauna, &GridPosition, &Health)>();
        let fauna: Vec<_> = query.iter(&world).collect();
        assert_eq!(fauna.len(), 1);
        assert_eq!(fauna[0].1, &GridPosition { x: 13, y: 15 });
        assert_eq!(fauna[0].2.current, 200.0);
        assert_eq!(world.resource::<BacklashLedger>().fauna_awakenings, 1);
    }

    #[test]
    fn test_backlash_writes_major_chronicle() {
        let mut world = handler_world();
        world.send_event(BacklashEvent {
            severity: 10.0,
            stress: 2.0,
            kind: BacklashKind::SuperStorm,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_backlash_system);
        schedule.run(&mut world);
        let chronicle = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle.get_cursor();
        let fired: Vec<_> = reader.read(chronicle).collect();
        assert_eq!(fired.len(), 1);
        assert!(fired[0].text.contains("PLANETARY DEFENSE"));
        assert_eq!(fired[0].importance, EventImportance::Major);
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        // Concept-only: the public strings must not lift named IP.
        let banned = [
            "gaia's vengeance",
            "captain planet",
            "godzilla",
            "mothra",
            "kaiju",
            "avatar",
            "pandora",
        ];
        let mut corpus = String::new();
        for kind in [
            BacklashKind::SuperStorm,
            BacklashKind::SeismicUnrest,
            BacklashKind::DormantMegafauna,
        ] {
            corpus.push_str(kind.label());
            corpus.push(' ');
        }
        let lowered = corpus.to_lowercase();
        for term in banned {
            assert!(!lowered.contains(term), "banned IP term present: {term}");
        }
    }
}
