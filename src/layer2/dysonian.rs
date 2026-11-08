//! Spec 403: Dysonian Swarms.
//!
//! Layer 2 endgame energy: solar collectors in close stellar orbit
//! harvest the star's output, raising the civilization's global energy
//! availability — but the swarm physically blocks sunlight, reducing
//! Layer 1 insolation toward an artificial ice age.
//!
//! ## Grounding audit (spec §8 pattern: adapt to the real codebase)
//!
//! * **The spec sketch's `layer2::system::Star` does not exist.** The
//!   real Layer 2 bodies are [`SystemBody`][crate::layer2::system::
//!   SystemBody] marker + [`OrbitalBody`][crate::layer2::system::
//!   OrbitalBody] entities (stars are `OrbitalBody` radius ~5.0 at the
//!   system center); there is no luminosity anywhere. Following the
//!   spec-400/401 precedent, this module builds the spec's exact
//!   [`Star`] leg here and carries it on real `SystemBody` entities
//!   spawned by [`spawn_swarm_star`].
//! * **The spec sketch's `layer1::power::GlobalPowerNet` does not
//!   exist.** Spec 042's real power system is `layer1::energy`:
//!   per-grid [`PowerSource`][crate::layer1::energy::PowerSource] /
//!   `PowerConsumer` BFS grids plus the [`EnergyGrid`][crate::
//!   layer1::energy::EnergyGrid] aggregate component (the global
//!   generation/consumption ledger other systems sum — e.g.
//!   `predecessors` totals `EnergyGrid.total_generation` across the
//!   world). There is no single global power resource to add to, and
//!   a star has no colony-grid position to join a BFS grid with, so
//!   the swarm's per-collector output is written into the star's real
//!   `EnergyGrid.total_generation` and mirrored in this module's
//!   [`DysonianState`] ledger (the spec-400 `LagrangeState` pattern).
//!   No parallel power system is invented: `EnergyGrid` is the ledger,
//!   `DysonianState` is its swarm bookkeeping.
//! * **The spec sketch's `layer1::weather::Climate` does not exist.**
//!   The real Layer 1 climate seams are the [`TemperatureGrid`]
//!   [crate::layer1::nature::temperature::TemperatureGrid] (per-tile
//!   values + `ambient`, recomputed from `SeasonState` every tick by
//!   `update_temperature_system`) and that system's solar-heating leg
//!   (0.15/tick Day, 0.05 Dawn/Dusk — tuned 2026-10-03, untouched
//!   here). The swarm shadow therefore lives in
//!   `update_temperature_system` itself: when a [`DysonianState`] is
//!   present it subtracts [`TEMPERATURE_DROP_PER_SHADOW`] x
//!   `shadow_factor` from the freshly computed ambient and scales the
//!   solar heat by `insolation_factor`. With no swarm (shadow 0.0)
//!   both terms are exact no-ops, so no existing power/temperature
//!   constant or balance changes. Farms feel the shadow through the
//!   temperature grid they already live on (thermal damage, heater /
//!   LifeSupport counterplay) — no farm code is touched.
//! * **Launch logistics (spec 105).** The real launch seam prices
//!   fleet transfers (`layer2::logistics::launch`) and has no
//!   collector cargo; collectors are launched through the headless
//!   `swarm [n]` command onto the star entity via
//!   [`launch_collectors`] (the spec-400/401 headless-helper pattern).
//!   No fleet/fuel change is made.
//! * **Power formula (spec §4), kept verbatim:** 10 power per
//!   collector, scaled by `luminosity / 100`. One adaptation: the
//!   sketch *accumulates* into `available` every run, which inflates
//!   without bound if the system runs per tick; here
//!   `power_available` is recomputed idempotently each tick (the
//!   current total generation), matching how `EnergyGrid` works.
//! * **Shadow formula (spec §4), kept verbatim:**
//!   `shadow = (collectors / 1000).clamp(0.0, 0.9)`; insolation is
//!   `1 - shadow`; the temperature drop is the sketch's
//!   `50.0 * shadow` (max 45 degrees at full shadow), applied to the
//!   ambient baseline rather than a mock `Climate` component.
//! * **§5 solar-flare attrition, built cheaply:** every
//!   [`FLARE_CHECK_INTERVAL`] ticks each swarm rolls the spec-400
//!   deterministic splitmix64 roll (seed in [`DysonianState`], never
//!   `thread_rng`); a hit destroys [`flare_loss`] collectors (5%,
//!   minimum 1) and is chronicled. Reloading and re-running a tick
//!   cannot re-roll it (same-tick guard in the state).
//! * **§5 first-visibility chronicle, built:** the first time the
//!   shadow becomes non-zero, a Major chronicle entry — "The sky is
//!   caged" (the spec's own lore line) — is written once per run.
//! * **§5 sky-renderer dimming is NOT built:** the renderer reads the
//!   per-tile light map, and a global insolation dimming leg there
//!   would duplicate the temperature/solar bridge; left unbuilt and
//!   documented here per the brief.
//!
//! All names and strings are original and generic — "solar
//! collector", "swarm", and "insolation" are ordinary terms. See the
//! IP-guard test.

use bevy_ecs::prelude::*;
use ratatui::style::Color;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::EnergyGrid;
use crate::layer2::lagrange::deterministic_roll;
use crate::layer2::system::{OrbitalBody, SystemBody};
use crate::shared::time::SimulationTime;

/// Power generated per collector at luminosity 100 (spec §4).
pub const POWER_PER_COLLECTOR: f32 = 10.0;
/// Collectors at which the shadow reaches its 0.9 cap (spec §4).
pub const SHADOW_FULL_COLLECTORS: f32 = 1000.0;
/// Maximum shadow factor: 90% of sunlight blocked (spec §4).
pub const MAX_SHADOW_FACTOR: f32 = 0.9;
/// Ambient temperature drop (Celsius) at shadow factor 1.0 — the
/// spec sketch's `50.0 * shadow_factor` (max 45 degrees at the cap).
pub const TEMPERATURE_DROP_PER_SHADOW: f32 = 50.0;
/// Default game seed for the deterministic flare rolls.
pub const DEFAULT_DYSON_SEED: u64 = 0x0403_5EED_0000_0403;
/// Ticks between solar-flare attrition checks (§5).
pub const FLARE_CHECK_INTERVAL: u64 = 500;
/// Chance one swarm loses collectors at one flare check (§5).
pub const FLARE_CHANCE: f32 = 0.25;
/// Collectors launched by a bare `swarm` headless command.
pub const DEFAULT_LAUNCH_BATCH: u32 = 50;

/// The star (spec type, built here — see the module grounding
/// audit). Carried by real [`SystemBody`] entities.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Star {
    /// Stellar luminosity; 100.0 is the reference star of spec §4.
    pub luminosity: f32,
}

impl Default for Star {
    fn default() -> Self {
        Self { luminosity: 100.0 }
    }
}

/// Solar collectors in close stellar orbit around a [`Star`]
/// (spec type, exact sketch shape).
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DysonianSwarm {
    /// Collectors currently active (flare attrition reduces this).
    pub collectors_active: u32,
}

/// Global swarm ledger: the bookkeeping mirror of the real
/// `EnergyGrid` generation the swarm writes (see the audit).
#[derive(Resource, Debug, Clone)]
pub struct DysonianState {
    /// Game seed for the deterministic flare rolls.
    pub seed: u64,
    /// Total active collectors across all swarms.
    pub collectors_total: u32,
    /// Current global power available from all swarms (recomputed
    /// idempotently each tick — see the audit).
    pub power_available: f32,
    /// Fraction of sunlight blocked, 0.0..=[`MAX_SHADOW_FACTOR`].
    pub shadow_factor: f32,
    /// Fraction of sunlight reaching Layer 1 (`1 - shadow_factor`).
    pub insolation_factor: f32,
    /// Collectors destroyed by solar flares so far (§5).
    pub flare_losses: u64,
    /// Last tick the flare check ran (same-tick guard).
    pub last_flare_tick: u64,
    /// One-shot: the "sky is caged" chronicle was written.
    pub sky_caged_chronicled: bool,
}

impl Default for DysonianState {
    fn default() -> Self {
        Self {
            seed: DEFAULT_DYSON_SEED,
            collectors_total: 0,
            power_available: 0.0,
            shadow_factor: 0.0,
            insolation_factor: 1.0,
            flare_losses: 0,
            last_flare_tick: 0,
            sky_caged_chronicled: false,
        }
    }
}

/// Power one swarm generates: the spec §4 formula, verbatim.
#[must_use]
pub fn swarm_power(collectors: u32, luminosity: f32) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let collectors = collectors as f32;
    collectors * POWER_PER_COLLECTOR * (luminosity / 100.0)
}

/// Shadow factor for a collector count: the spec §4 formula.
#[must_use]
pub fn shadow_factor_for(collectors: u32) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let collectors = collectors as f32;
    (collectors / SHADOW_FULL_COLLECTORS).clamp(0.0, MAX_SHADOW_FACTOR)
}

/// Collectors destroyed by one flare hit: 5% of the swarm, minimum 1.
#[must_use]
pub fn flare_loss(collectors: u32) -> u32 {
    #[allow(clippy::cast_precision_loss)]
    let five_percent = (collectors as f32 * 0.05).ceil() as u32;
    five_percent.max(1).min(collectors)
}

/// Spec system (adapted): recompute each swarm's contribution into
/// the real global power ledger — the star's [`EnergyGrid`]
/// generation — and mirror the totals in [`DysonianState`].
pub fn calculate_swarm_power_system(
    mut commands: Commands,
    mut swarms: Query<(Entity, &Star, &DysonianSwarm, Option<&mut EnergyGrid>)>,
    state: Option<ResMut<DysonianState>>,
) {
    let mut total_collectors = 0u32;
    let mut total_power = 0.0f32;
    for (entity, star, swarm, grid) in swarms.iter_mut() {
        let power = swarm_power(swarm.collectors_active, star.luminosity);
        total_collectors += swarm.collectors_active;
        total_power += power;
        match grid {
            Some(mut grid) => {
                grid.total_generation = power;
            }
            None => {
                commands.entity(entity).insert(EnergyGrid {
                    total_generation: power,
                    total_consumption: 0.0,
                });
            }
        }
    }
    if let Some(mut state) = state {
        state.collectors_total = total_collectors;
        state.power_available = total_power;
    }
}

/// Spec system (adapted): recompute the insolation shadow from the
/// live collector count and write the one-shot first-visibility
/// chronicle (§5). The temperature/solar effect itself is applied
/// inside the real `update_temperature_system`, which reads
/// [`DysonianState`] — see the module audit.
pub fn apply_swarm_shadow_system(
    swarms: Query<&DysonianSwarm>,
    state: Option<ResMut<DysonianState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let total: u32 = swarms.iter().map(|s| s.collectors_active).sum();
    let shadow = shadow_factor_for(total);
    let Some(mut state) = state else { return };
    state.collectors_total = total;
    state.shadow_factor = shadow;
    state.insolation_factor = 1.0 - shadow;
    if shadow > 0.0 && !state.sky_caged_chronicled {
        state.sky_caged_chronicled = true;
        chronicle.send(AddChronicleEvent {
            text: "The sky is caged — the collector swarm has grown dense enough to see from the surface, a dark lattice drinking the star's light. The power is real, and so is the shadow it casts.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// §5 maintenance: solar flares occasionally destroy collectors.
/// Deterministic (seed, tick, ordinal) rolls — see the audit.
pub fn swarm_flare_attrition_system(
    time: Res<SimulationTime>,
    state: Option<ResMut<DysonianState>>,
    mut swarms: Query<&mut DysonianSwarm>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let tick = time.tick;
    if tick == 0 || !tick.is_multiple_of(FLARE_CHECK_INTERVAL) {
        return;
    }
    let seed = state.as_ref().map_or(DEFAULT_DYSON_SEED, |s| s.seed);
    if let Some(state) = &state {
        if state.last_flare_tick == tick {
            return;
        }
    }
    // Snapshot first in query order, then apply in the same order
    // — one query only, no B0001 conflict.
    let snapshot: Vec<u32> = swarms.iter().map(|s| s.collectors_active).collect();
    let mut losses: Vec<u32> = Vec::new();
    for (ordinal, collectors) in snapshot.iter().enumerate() {
        if *collectors == 0 {
            losses.push(0);
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        let roll = deterministic_roll(seed, tick, ordinal as u64);
        losses.push(if roll < FLARE_CHANCE {
            flare_loss(*collectors)
        } else {
            0
        });
    }
    let total_lost: u64 = losses.iter().map(|l| u64::from(*l)).sum();
    if total_lost > 0 {
        for (mut swarm, loss) in swarms.iter_mut().zip(losses.iter()) {
            swarm.collectors_active -= loss;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A solar flare has torn through the collector swarm — {total_lost} collector(s) burned out in close orbit. Launch replacements to hold the swarm's output."
            ),
            importance: EventImportance::Standard,
        });
    }
    if let Some(mut state) = state {
        state.last_flare_tick = tick;
        state.flare_losses += total_lost;
    }
}

// ---------------------------------------------------------------------------
// World helpers (headless / census)
// ---------------------------------------------------------------------------

/// Census of the swarm layer for STATS / panels.
#[derive(Debug, Clone, Copy, Default)]
pub struct DysonianCensus {
    /// Stars carrying a swarm.
    pub stars: usize,
    /// Total active collectors.
    pub collectors: u32,
    /// Current global power from all swarms.
    pub power: f32,
    /// Current shadow factor.
    pub shadow: f32,
    /// Current insolation factor.
    pub insolation: f32,
    /// Collectors lost to flares so far.
    pub flare_losses: u64,
}

/// Census of stars, collectors, power, and shadow.
#[must_use]
pub fn dysonian_census(world: &mut World) -> DysonianCensus {
    let mut stars = 0usize;
    let mut collectors = 0u32;
    let mut power = 0.0f32;
    {
        let mut q = world.query::<(&Star, &DysonianSwarm)>();
        for (star, swarm) in q.iter(world) {
            stars += 1;
            collectors += swarm.collectors_active;
            power += swarm_power(swarm.collectors_active, star.luminosity);
        }
    }
    let (flare_losses, shadow, insolation) =
        world.get_resource::<DysonianState>().map_or((0, shadow_factor_for(collectors), 1.0 - shadow_factor_for(collectors)), |s| {
            (s.flare_losses, s.shadow_factor, s.insolation_factor)
        });
    DysonianCensus {
        stars,
        collectors,
        power,
        shadow,
        insolation,
        flare_losses,
    }
}

/// Compact STATS label, e.g. `50col/500pow/0.05shadow`. No spaces.
#[must_use]
pub fn dysonian_stats_label(world: &mut World) -> String {
    let census = dysonian_census(world);
    format!(
        "{}col/{:.0}pow/{:.2}shadow",
        census.collectors, census.power, census.shadow
    )
}

/// Spawn the system's star carrying an (initially empty) swarm: a
/// real [`SystemBody`] + [`OrbitalBody`] with the spec [`Star`] leg,
/// a [`DysonianSwarm`], and the real [`EnergyGrid`] ledger the power
/// system writes into.
pub fn spawn_swarm_star(world: &mut World, collectors: u32) -> Entity {
    world
        .spawn((
            SystemBody,
            OrbitalBody {
                name: "Primary Star".to_string(),
                radius: 5.0,
                color: Color::Yellow,
                char: '*',
            },
            Star::default(),
            DysonianSwarm {
                collectors_active: collectors,
            },
            EnergyGrid {
                total_generation: swarm_power(collectors, 100.0),
                total_consumption: 0.0,
            },
        ))
        .id()
}

/// Launch `count` collectors (spec 105 headless leg): adds them to
/// the first swarm found, spawning the star + swarm first if the
/// system has none. Returns the star entity.
pub fn launch_collectors(world: &mut World, count: u32) -> Entity {
    let existing = {
        let mut q = world.query::<(Entity, &DysonianSwarm)>();
        q.iter(world).map(|(e, _)| e).next()
    };
    if let Some(star) = existing {
        if let Some(mut swarm) = world.get_mut::<DysonianSwarm>(star) {
            swarm.collectors_active += count;
        }
        star
    } else {
        spawn_swarm_star(world, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    fn swarm_app() -> App {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(DysonianState::default());
        app.add_systems(
            Update,
            (calculate_swarm_power_system, apply_swarm_shadow_system),
        );
        app
    }

    #[test]
    fn test_swarm_increases_global_power() {
        // Spec RED test 1, adapted: GlobalPowerNet does not exist —
        // the real global ledger is the star's EnergyGrid plus the
        // DysonianState mirror (see the module audit).
        let mut app = swarm_app();
        let star_ent = app
            .world_mut()
            .spawn((
                SystemBody,
                Star { luminosity: 100.0 },
                DysonianSwarm {
                    collectors_active: 50,
                },
            ))
            .id();
        app.update();
        // 50 collectors x 10 power x (100/100) = 500 into the ledger.
        let grid = app.world().get::<EnergyGrid>(star_ent).unwrap();
        assert!(
            grid.total_generation > 100.0,
            "Swarm should add power to the global net"
        );
        assert!((grid.total_generation - 500.0).abs() < f32::EPSILON);
        let state = app.world().resource::<DysonianState>();
        assert!((state.power_available - 500.0).abs() < f32::EPSILON);
        assert_eq!(state.collectors_total, 50);
        // Idempotent: a second tick must not accumulate (audit note).
        app.update();
        assert!(
            (app.world().resource::<DysonianState>().power_available - 500.0).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn test_swarm_power_scales_with_luminosity() {
        assert!((swarm_power(50, 100.0) - 500.0).abs() < f32::EPSILON);
        assert!((swarm_power(50, 200.0) - 1000.0).abs() < f32::EPSILON);
        assert!((swarm_power(0, 100.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_swarm_reduces_planetary_insolation_and_temp() {
        // Spec RED test 2, adapted: Climate does not exist — the
        // shadow lands in DysonianState and is applied by the real
        // update_temperature_system (integration test below).
        let mut app = swarm_app();
        app.world_mut().spawn((
            SystemBody,
            Star { luminosity: 100.0 },
            DysonianSwarm {
                collectors_active: 100,
            },
        ));
        app.update();
        let state = app.world().resource::<DysonianState>();
        assert!(
            state.insolation_factor < 1.0,
            "Insolation should be reduced"
        );
        assert!((state.shadow_factor - 0.1).abs() < f32::EPSILON);
        assert!((state.insolation_factor - 0.9).abs() < f32::EPSILON);
        // The sketch's temperature leg: 50 x shadow = 5 degrees.
        assert!(
            (TEMPERATURE_DROP_PER_SHADOW * state.shadow_factor - 5.0).abs() < f32::EPSILON
        );
    }

    #[test]
    fn test_shadow_factor_clamps_at_ninety_percent() {
        assert!((shadow_factor_for(0)).abs() < f32::EPSILON);
        assert!((shadow_factor_for(500) - 0.5).abs() < f32::EPSILON);
        assert!((shadow_factor_for(1000) - 0.9).abs() < f32::EPSILON);
        assert!((shadow_factor_for(5000) - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_shadow_cools_the_real_temperature_grid() {
        // Integration with the REAL Layer 1 seam: the same world run
        // through update_temperature_system with and without a swarm
        // shadow in DysonianState. Spring ambient is 15C; 100
        // collectors (shadow 0.1) must drop the ambient by 5C and
        // leave tiles colder (solar heat scaled by insolation 0.9).
        use crate::layer1::nature::temperature::{update_temperature_system, TemperatureGrid};
        use crate::layer1::seasons::SeasonState;
        use crate::layer1::terrain::{TerrainGrid, TerrainType};
        use bevy_ecs::system::RunSystemOnce;

        fn run(shadow: f32) -> (f32, f32) {
            let mut world = World::new();
            world.insert_resource(TemperatureGrid::new(10, 10, 15.0));
            world.insert_resource(SeasonState::default());
            world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
            world.insert_resource(TerrainGrid {
                width: 10,
                height: 10,
                tiles: vec![TerrainType::Grass; 100],
            });
            world.insert_resource(DysonianState {
                shadow_factor: shadow,
                insolation_factor: 1.0 - shadow,
                ..Default::default()
            });
            world.run_system_once(update_temperature_system).unwrap();
            let grid = world.resource::<TemperatureGrid>();
            (grid.ambient, grid.get(5, 5))
        }

        let (clear_ambient, clear_tile) = run(0.0);
        let (shaded_ambient, shaded_tile) = run(0.1);
        assert!(
            (clear_ambient - shaded_ambient - 5.0).abs() < 0.01,
            "shadow 0.1 must drop ambient by 5C (clear {clear_ambient}, shaded {shaded_ambient})"
        );
        assert!(
            shaded_tile < clear_tile,
            "shaded tiles must be colder (clear {clear_tile}, shaded {shaded_tile})"
        );
    }

    #[test]
    fn test_no_swarm_leaves_temperature_untouched() {
        // Zero-balance guard: shadow 0.0 is an exact no-op, and a
        // missing DysonianState resource is too (minimal worlds).
        use crate::layer1::nature::temperature::{update_temperature_system, TemperatureGrid};
        use crate::layer1::seasons::SeasonState;
        use crate::layer1::terrain::{TerrainGrid, TerrainType};
        use bevy_ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(TemperatureGrid::new(10, 10, 15.0));
        world.insert_resource(SeasonState::default());
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world.run_system_once(update_temperature_system).unwrap();
        assert!((world.resource::<TemperatureGrid>().ambient - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sky_is_caged_chronicle_fires_once() {
        let mut app = swarm_app();
        app.world_mut().spawn(DysonianSwarm {
            collectors_active: 10,
        });
        app.update();
        app.update();
        let mut events = app.world_mut().resource_mut::<Events<AddChronicleEvent>>();
        let texts: Vec<String> = events.drain().map(|e| e.text).collect();
        assert_eq!(texts.len(), 1, "first-visibility chronicle is one-shot");
        assert!(texts[0].contains("The sky is caged"));
    }

    #[test]
    fn test_flare_attrition_is_deterministic() {
        // Default seed at tick 10000, ordinal 0 rolls ~0.093 — under
        // FLARE_CHANCE (0.25), so the swarm loses 5% (min 1).
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(SimulationTime {
            tick: 10_000,
            speed: Default::default(),
        });
        app.insert_resource(DysonianState::default());
        app.add_systems(Update, swarm_flare_attrition_system);
        let star = app
            .world_mut()
            .spawn(DysonianSwarm {
                collectors_active: 100,
            })
            .id();
        app.update();
        assert_eq!(
            app.world().get::<DysonianSwarm>(star).unwrap().collectors_active,
            95
        );
        assert_eq!(app.world().resource::<DysonianState>().flare_losses, 5);
        // Same tick re-run cannot re-roll (same-tick guard).
        app.update();
        assert_eq!(
            app.world().get::<DysonianSwarm>(star).unwrap().collectors_active,
            95
        );
    }

    #[test]
    fn test_flare_misses_on_a_quiet_tick() {
        // Default seed at tick 500, ordinal 0 rolls ~0.447 — over
        // FLARE_CHANCE, so nothing is lost.
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(SimulationTime {
            tick: 500,
            speed: Default::default(),
        });
        app.insert_resource(DysonianState::default());
        app.add_systems(Update, swarm_flare_attrition_system);
        let star = app
            .world_mut()
            .spawn(DysonianSwarm {
                collectors_active: 100,
            })
            .id();
        app.update();
        assert_eq!(
            app.world().get::<DysonianSwarm>(star).unwrap().collectors_active,
            100
        );
        assert_eq!(app.world().resource::<DysonianState>().flare_losses, 0);
    }

    #[test]
    fn test_flare_loss_rounding() {
        assert_eq!(flare_loss(100), 5);
        assert_eq!(flare_loss(10), 1);
        assert_eq!(flare_loss(1), 1);
        assert_eq!(flare_loss(0), 0);
    }

    #[test]
    fn test_launch_collectors_and_census() {
        let mut world = World::new();
        world.insert_resource(DysonianState::default());
        assert_eq!(dysonian_stats_label(&mut world), "0col/0pow/0.00shadow");
        let star = launch_collectors(&mut world, 50);
        assert_eq!(
            world.get::<DysonianSwarm>(star).unwrap().collectors_active,
            50
        );
        // The star carries the real seams: SystemBody + EnergyGrid.
        assert!(world.get::<SystemBody>(star).is_some());
        assert!(
            (world.get::<EnergyGrid>(star).unwrap().total_generation - 500.0).abs()
                < f32::EPSILON
        );
        launch_collectors(&mut world, 50);
        let census = dysonian_census(&mut world);
        assert_eq!(census.stars, 1);
        assert_eq!(census.collectors, 100);
        assert!((census.power - 1000.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_ip_guard_no_borrowed_names() {
        const BANNED: [&str; 10] = [
            "dyson sphere",
            "star wars",
            "star trek",
            "the expanse",
            "mass effect",
            "ringworld",
            "culture",
            "foundation",
            "stellar engine",
            "shkadov",
        ];
        let mut haystack = String::new();
        haystack.push_str("Primary Star solar collector swarm insolation ");
        haystack.push_str("The sky is caged — the collector swarm has grown dense enough to see from the surface, a dark lattice drinking the star's light. The power is real, and so is the shadow it casts. ");
        haystack.push_str("A solar flare has torn through the collector swarm — collector(s) burned out in close orbit. Launch replacements to hold the swarm's output.");
        let haystack = haystack.to_lowercase();
        for banned in BANNED {
            assert!(!haystack.contains(banned), "banned term present: {banned}");
        }
    }
}
