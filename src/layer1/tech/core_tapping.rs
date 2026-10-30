//! Spec 375: Planetary Core Tapping.
//!
//! "Infinite energy at the cost of the world."
//!
//! A [`CoreDrill`] building taps the planet's mantle for limitless
//! geothermal power: while active it drives a real
//! [`PowerSource`][crate::layer1::energy::PowerSource] to
//! [`CORE_DRILL_OUTPUT`] on the Layer 1 grid. The price is
//! [`PlanetaryStability`]: every tick any drill taps, stability bleeds
//! by [`STABILITY_DECAY_PER_TICK`]; with every drill off it recovers
//! twenty times slower ([`STABILITY_RECOVERY_PER_TICK`]). Active drills
//! also feed [`SEISMIC_STRESS_PER_TICK`] of stress per tick into the real
//! [`SeismicGrid`][crate::layer1::geology::SeismicGrid] at the drill
//! tile, so the Spec 153 geology pipeline (`check_seismic_events`)
//! fires from drill-fed stress on its own threshold.
//!
//! As stability falls, earthquakes (Spec 153
//! [`GeologicalEvent`][crate::layer1::geology::GeologicalEvent]s, which
//! the already-live `apply_geological_event_system` turns into damage
//! and screen shake) grow likelier and stronger following the spec's
//! exact formulas: chance `(1 - level) * 0.05`, magnitude
//! `5 + (1 - level) * 5`. Threshold chronicles fire at 75% / 50% / 25%
//! (Major, once per downward crossing — band hysteresis, re-armed by
//! recovery). At 0.0 the core breaches, once and permanently: the drills
//! are wrecked (inactive, output 0, restart refused), a real
//! [`ExplosionEvent`][crate::layer1::environment::volatile::ExplosionEvent]
//! (600 damage, radius 8) and a magnitude-10 geological event fire at
//! each drill site, and a Legendary "PLANETARY CORE BREACH" chronicle
//! is written. There is no recovery after a breach.
//!
//! ## Adaptations (spec sketch -> real codebase)
//!
//! * `PowerGenerator` does not exist -> the real
//!   `layer1::energy::PowerSource` (`output` + `active`; the drill
//!   writes both).
//! * `layer1::hazards::EarthquakeEvent` does not exist -> Spec 153
//!   `layer1::geology::GeologicalEvent` + `SeismicGrid` (plus
//!   `environment::seismic::SeismicSource` on the building, added at
//!   placement in `architecture/building.rs`). Quakes damage through
//!   the live geology pipeline.
//! * The spec's quake test at stability 0.1 was a coin flip under its
//!   own random-only formula -> at critical stability
//!   ([`CRITICAL_STABILITY`] and below) quakes fire deterministically,
//!   throttled by a [`QUAKE_COOLDOWN_TICKS`]-tick cooldown; mid-range
//!   stability keeps the spec's exact chance formula via the pure
//!   helpers [`quake_chance`] / [`quake_magnitude`].
//! * [`process_core_drill_system`] is an exclusive `&mut World` system
//!   (the codebase's disaster pattern, cf. Spec 277 deorbit) instead of
//!   the sketch's Query/ResMut signature — it tolerates missing event
//!   resources and avoids the Bevy 0.15 tuple-arity / `Option<Res<T>>`
//!   traps.
//! * Tests live in a sibling `core_tapping_tests.rs` module (codebase
//!   pattern) rather than an inner `mod tests`.
//! * Layer-2 export (spec REFACTOR): satisfied structurally — drill
//!   output is an ordinary PowerSource on the Layer 1 grid ledger; no
//!   separate export pipe exists in the codebase.
//!
//! All names and strings are original and generic (see the IP-guard
//! test in `core_tapping_tests.rs`).

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerSource;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::geology::{GeologicalEvent, SeismicGrid};
use crate::layer1::map::GridPosition;
use crate::shared::log::MessageLog;

/// Power (per tick) one active core drill feeds the grid.
pub const CORE_DRILL_OUTPUT: f32 = 10_000.0;
/// Stability lost per tick while any drill is tapping.
pub const STABILITY_DECAY_PER_TICK: f32 = 0.001;
/// Stability regained per tick while every drill is off (20x slower
/// than the decay — the temptation is easy to start, slow to undo).
pub const STABILITY_RECOVERY_PER_TICK: f32 = 0.0002;
/// Seismic stress one active drill feeds its tile per tick.
pub const SEISMIC_STRESS_PER_TICK: f32 = 2.0;
/// At or below this stability, quakes fire deterministically.
pub const CRITICAL_STABILITY: f32 = 0.25;
/// Minimum ticks between drill-driven quakes at critical stability.
pub const QUAKE_COOLDOWN_TICKS: u32 = 20;
/// Explosion damage of the catastrophic core breach.
pub const BREACH_DAMAGE: f32 = 600.0;
/// Explosion radius (Chebyshev tiles) of the catastrophic breach.
pub const BREACH_RADIUS: u32 = 8;
/// Magnitude of the geological event fired by the breach.
pub const BREACH_MAGNITUDE: f32 = 10.0;
/// Stability thresholds that earn a Major chronicle on the way down.
pub const STABILITY_THRESHOLDS: [f32; 3] = [0.75, 0.50, 0.25];
/// Fallback quake center when no drill stands (map heart).
pub const FALLBACK_CENTER: GridPosition = GridPosition { x: 40, y: 25 };

/// Marker + state for a planetary core drill building. A fresh drill
/// taps on placement (active by default): the temptation is opt-out.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct CoreDrill {
    /// Whether the drill is currently tapping the core.
    pub active: bool,
}

impl CoreDrill {
    /// A freshly constructed drill, tapping.
    #[must_use]
    pub const fn new() -> Self {
        Self { active: true }
    }
}

/// How many stability thresholds `level` has crossed downward:
/// 0 above 75%, 1 at/below 75%, 2 at/below 50%, 3 at/below 25%.
#[must_use]
pub fn stability_band(level: f32) -> u8 {
    let mut band = 0u8;
    for threshold in STABILITY_THRESHOLDS {
        if level <= threshold {
            band += 1;
        }
    }
    band
}

/// The planet's structural stability (1.0 pristine -> 0.0 breached).
#[derive(Resource, Debug, Clone)]
pub struct PlanetaryStability {
    /// Current stability level, clamped 0.0..=1.0.
    pub level: f32,
    /// Latched catastrophic failure: once true, never recovers and
    /// wrecked drills refuse to restart.
    pub failed: bool,
    /// Ticks until the next deterministic critical quake may fire.
    pub quake_cooldown: u32,
    /// Highest threshold band already chronicled (hysteresis: recovery
    /// to a lower band re-arms the warning).
    pub last_band: u8,
}

impl Default for PlanetaryStability {
    fn default() -> Self {
        Self {
            level: 1.0,
            failed: false,
            quake_cooldown: 0,
            last_band: 0,
        }
    }
}

/// Spec formula: quake chance per tick at a given stability level.
#[must_use]
pub fn quake_chance(level: f32) -> f32 {
    (1.0 - level.clamp(0.0, 1.0)) * 0.05
}

/// Spec formula: quake magnitude at a given stability level.
#[must_use]
pub fn quake_magnitude(level: f32) -> f32 {
    5.0 + (1.0 - level.clamp(0.0, 1.0)) * 5.0
}

/// Number of core drills standing, and how many are tapping.
#[must_use]
pub fn core_drill_counts(world: &mut World) -> (usize, usize) {
    let mut query = world.query::<&CoreDrill>();
    let mut total = 0usize;
    let mut active = 0usize;
    for drill in query.iter(world) {
        total += 1;
        if drill.active {
            active += 1;
        }
    }
    (total, active)
}

/// Current planetary stability level (1.0 if the resource is absent).
#[must_use]
pub fn planetary_stability_level(world: &World) -> f32 {
    world
        .get_resource::<PlanetaryStability>()
        .map_or(1.0, |s| s.level)
}

/// Whether the core has breached (latched catastrophic failure).
#[must_use]
pub fn core_failed(world: &World) -> bool {
    world
        .get_resource::<PlanetaryStability>()
        .is_some_and(|s| s.failed)
}

/// Toggle helper (headless `coredrill on|off|toggle` seam): set every
/// drill's active flag. Returns `false` without changing anything when
/// a restart is refused because the core has breached.
pub fn set_all_drills_active(world: &mut World, active: bool) -> bool {
    if active && core_failed(world) {
        return false;
    }
    let drills: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<CoreDrill>>();
        query.iter(world).collect()
    };
    for entity in drills {
        if let Some(mut drill) = world.get_mut::<CoreDrill>(entity) {
            drill.active = active;
        }
        if let Some(mut source) = world.get_mut::<PowerSource>(entity) {
            source.output = if active { CORE_DRILL_OUTPUT } else { 0.0 };
            source.active = active;
        }
    }
    true
}

/// Flip every drill's active flag. Refused (returns `false`) when the
/// core has breached and any drill would be turned back on.
pub fn toggle_all_drills(world: &mut World) -> bool {
    let (_, active) = core_drill_counts(world);
    set_all_drills_active(world, active == 0)
}

fn send_chronicle(world: &mut World, text: String, importance: EventImportance) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent { text, importance });
    }
}

/// The catastrophic breach: wreck every drill, fire the real explosion
/// and geology pipelines at each drill site, log the end of the world.
/// Latched by `PlanetaryStability.failed` — runs exactly once.
fn catastrophic_failure(world: &mut World, sites: &[GridPosition]) {
    {
        let mut stability = world.resource_mut::<PlanetaryStability>();
        stability.level = 0.0;
        stability.failed = true;
    }
    // Wreck the drills: inactive, no output, restart refused (the
    // `failed` latch in set_all_drills_active enforces the refusal).
    let drills: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<CoreDrill>>();
        query.iter(world).collect()
    };
    for entity in drills {
        if let Some(mut drill) = world.get_mut::<CoreDrill>(entity) {
            drill.active = false;
        }
        if let Some(mut source) = world.get_mut::<PowerSource>(entity) {
            source.output = 0.0;
            source.active = false;
        }
    }
    for site in sites {
        if let Some(mut events) = world.get_resource_mut::<Events<ExplosionEvent>>() {
            events.send(ExplosionEvent {
                center: *site,
                damage: BREACH_DAMAGE,
                radius: BREACH_RADIUS,
            });
        }
        if let Some(mut events) = world.get_resource_mut::<Events<GeologicalEvent>>() {
            events.send(GeologicalEvent {
                center: *site,
                magnitude: BREACH_MAGNITUDE,
            });
        }
    }
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(
            "PLANETARY CORE BREACH: the mantle has ruptured. The drills are wrecked and the wound will not close."
                .to_string(),
        );
    }
    send_chronicle(
        world,
        "PLANETARY CORE BREACH: the core drills pushed the mantle past its limit. \
         The ground split, the drills were wrecked where they stood, and the planet \
         will carry this wound forever."
            .to_string(),
        EventImportance::Legendary,
    );
}

/// Exclusive system: drive every drill's power output, bleed or recover
/// planetary stability, feed the seismic grid, chronicle threshold
/// crossings, and fire the catastrophic breach at 0.0.
pub fn process_core_drill_system(world: &mut World) {
    if world.get_resource::<PlanetaryStability>().is_none() {
        return;
    }
    if core_failed(world) {
        // Latched: wrecked drills stay wrecked, stability stays 0.
        let drills: Vec<Entity> = {
            let mut query = world.query_filtered::<Entity, With<CoreDrill>>();
            query.iter(world).collect()
        };
        for entity in drills {
            if let Some(mut source) = world.get_mut::<PowerSource>(entity) {
                source.output = 0.0;
                source.active = false;
            }
        }
        return;
    }

    // Drive power + collect active drill sites.
    let mut active_sites: Vec<GridPosition> = Vec::new();
    {
        let mut query =
            world.query::<(&CoreDrill, &mut PowerSource, Option<&GridPosition>)>();
        for (drill, mut source, pos) in query.iter_mut(world) {
            if drill.active {
                source.output = CORE_DRILL_OUTPUT;
                source.active = true;
                active_sites.push(pos.copied().unwrap_or(FALLBACK_CENTER));
            } else {
                source.output = 0.0;
                source.active = false;
            }
        }
    }
    let any_active = !active_sites.is_empty();

    // Feed the real seismic grid at each tapping drill's tile.
    if any_active {
        if let Some(mut grid) = world.get_resource_mut::<SeismicGrid>() {
            for site in &active_sites {
                grid.add_stress(site.x, site.y, SEISMIC_STRESS_PER_TICK);
            }
        }
    }

    // Bleed or recover stability.
    let (level_before, band_before) = {
        let stability = world.resource::<PlanetaryStability>();
        (stability.level, stability.last_band)
    };
    let level_after = if any_active {
        (level_before - STABILITY_DECAY_PER_TICK).max(0.0)
    } else {
        (level_before + STABILITY_RECOVERY_PER_TICK).min(1.0)
    };
    {
        let mut stability = world.resource_mut::<PlanetaryStability>();
        stability.level = level_after;
    }

    // The breach fires the moment stability hits 0.0.
    if level_after <= 0.0 {
        catastrophic_failure(world, &active_sites);
        return;
    }

    // Threshold chronicles: once per downward band crossing; recovery
    // to a lower band re-arms the warning (hysteresis).
    let band_after = stability_band(level_after);
    if band_after > band_before {
        for band in (band_before + 1)..=band_after {
            let threshold = STABILITY_THRESHOLDS[usize::from(band - 1)];
            #[allow(clippy::cast_possible_truncation)]
            let percent = (threshold * 100.0) as u32;
            send_chronicle(
                world,
                format!(
                    "The planet groans: planetary stability has fallen to {percent}%. \
                     Every tick the core drills tap, the mantle weakens. \
                     Shut them down and let the world knit itself back together."
                ),
                EventImportance::Major,
            );
        }
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(format!(
                "Planetary stability at {:.0}% — the core drills are wounding the world.",
                level_after * 100.0
            ));
        }
    }
    {
        let mut stability = world.resource_mut::<PlanetaryStability>();
        stability.last_band = band_after;
    }
}

/// System: earthquakes grow likelier and stronger as stability falls.
/// At critical stability they fire deterministically (cooldown
/// throttled); above it, the spec's exact chance formula rolls.
pub fn trigger_earthquake_system(
    mut stability: ResMut<PlanetaryStability>,
    mut events: EventWriter<GeologicalEvent>,
    drills: Query<&GridPosition, With<CoreDrill>>,
) {
    if stability.failed {
        return;
    }
    if stability.quake_cooldown > 0 {
        stability.quake_cooldown -= 1;
        return;
    }
    let level = stability.level;
    let fire = if level <= CRITICAL_STABILITY {
        true
    } else {
        let mut rng = rand::thread_rng();
        rng.gen::<f32>() < quake_chance(level)
    };
    if fire {
        let center = drills.iter().next().copied().unwrap_or(FALLBACK_CENTER);
        events.send(GeologicalEvent {
            center,
            magnitude: quake_magnitude(level),
        });
        stability.quake_cooldown = QUAKE_COOLDOWN_TICKS;
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn core_tapping_public_strings() -> Vec<String> {
    let mut strings = vec![
        "Planetary Core Tapping".to_string(),
        "Core Drill".to_string(),
        "Planetary Stability".to_string(),
        "PLANETARY CORE BREACH".to_string(),
        "The planet groans: planetary stability has fallen.".to_string(),
        "The core is breached. There is no recovery.".to_string(),
        "The wrecked drills cannot be restarted.".to_string(),
    ];
    for threshold in STABILITY_THRESHOLDS {
        #[allow(clippy::cast_possible_truncation)]
        let percent = (threshold * 100.0) as u32;
        strings.push(format!("planetary stability {percent}%"));
    }
    strings
}
