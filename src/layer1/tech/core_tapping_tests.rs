//! Spec 375 tests for Planetary Core Tapping, adapted from the spec's
//! mock-style RED tests to the real codebase (see the module docs in
//! `core_tapping.rs` for every adaptation).

use bevy_ecs::prelude::*;

use super::core_tapping::*;
use crate::layer1::core::chronicle::AddChronicleEvent;
use crate::layer1::energy::PowerSource;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::geology::{GeologicalEvent, SeismicGrid};
use crate::layer1::map::GridPosition;
use crate::shared::log::MessageLog;

fn test_world(level: f32) -> World {
    let mut world = World::new();
    world.insert_resource(PlanetaryStability {
        level,
        ..PlanetaryStability::default()
    });
    world.init_resource::<Events<GeologicalEvent>>();
    world.init_resource::<Events<ExplosionEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<MessageLog>();
    world.insert_resource(SeismicGrid::new(100, 100));
    world
}

fn spawn_drill(world: &mut World, active: bool, x: i32, y: i32) -> Entity {
    world
        .spawn((
            CoreDrill { active },
            PowerSource {
                output: 0.0,
                active: false,
            },
            GridPosition { x, y },
        ))
        .id()
}

fn run_process(world: &mut World) {
    let mut schedule = Schedule::default();
    schedule.add_systems(process_core_drill_system);
    schedule.run(world);
}

fn run_trigger(world: &mut World) {
    let mut schedule = Schedule::default();
    schedule.add_systems(trigger_earthquake_system);
    schedule.run(world);
}

fn geological_events(world: &mut World) -> Vec<GeologicalEvent> {
    // Drain, don't cursor-read: a fresh EventCursor re-reads buffered
    // events (bare Schedule::run never ages them out), which made the
    // second-read assertions in this file read stale events.
    world
        .resource_mut::<Events<GeologicalEvent>>()
        .drain()
        .collect()
}

fn chronicle_texts(world: &mut World) -> Vec<String> {
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .drain()
        .map(|e| e.text.clone())
        .collect()
}

/// Spec RED test 1 (adapted): an active drill generates massive power
/// through the real PowerSource grid component.
#[test]
fn test_core_drill_generates_massive_power() {
    let mut world = test_world(1.0);
    let drill = spawn_drill(&mut world, true, 10, 10);
    run_process(&mut world);
    let source = world.get::<PowerSource>(drill).unwrap();
    assert!(source.output >= 10_000.0, "output was {}", source.output);
    assert!(source.active);
}

/// Spec RED test 2 (adapted): stability degrades while a drill taps.
#[test]
fn test_core_drill_degrades_stability() {
    let mut world = test_world(1.0);
    spawn_drill(&mut world, true, 10, 10);
    run_process(&mut world);
    let stability = world.resource::<PlanetaryStability>();
    assert!(stability.level < 1.0);
    assert!((stability.level - 0.999).abs() < 0.000_01);
}

/// An inactive drill produces nothing, and with every drill off the
/// planet recovers — slowly (+0.0002/tick, 20x slower than decay).
#[test]
fn test_inactive_drill_no_power_and_stability_recovers() {
    let mut world = test_world(0.5);
    let drill = spawn_drill(&mut world, false, 10, 10);
    run_process(&mut world);
    let source = world.get::<PowerSource>(drill).unwrap();
    assert_eq!(source.output, 0.0);
    assert!(!source.active);
    let stability = world.resource::<PlanetaryStability>();
    assert!((stability.level - 0.5002).abs() < 0.000_01);
}

/// Spec RED test 3 (adapted): at critical stability an earthquake
/// fires deterministically (the spec's random-only formula made its
/// own 0.1-stability test a coin flip — see module docs).
#[test]
fn test_low_stability_triggers_earthquakes() {
    let mut world = test_world(0.1);
    spawn_drill(&mut world, true, 12, 13);
    run_trigger(&mut world);
    let fired = geological_events(&mut world);
    assert_eq!(fired.len(), 1, "critical stability must quake");
    assert_eq!(fired[0].center, GridPosition { x: 12, y: 13 });
    // Magnitude formula: 5 + (1 - 0.1) * 5 = 9.5.
    assert!((fired[0].magnitude - 9.5).abs() < 0.001);
    // Cooldown throttles the next deterministic quake.
    run_trigger(&mut world);
    assert!(geological_events(&mut world).is_empty());
}

/// Active drills feed the real SeismicGrid (+2.0 stress/tick at the
/// drill tile), so `check_seismic_events` can fire on its own.
#[test]
fn test_active_drill_feeds_seismic_grid() {
    let mut world = test_world(1.0);
    spawn_drill(&mut world, true, 20, 21);
    run_process(&mut world);
    let grid = world.resource::<SeismicGrid>();
    assert!((grid.get_stress(20, 21) - 2.0).abs() < f32::EPSILON);
}

/// Crossing the 75% threshold writes exactly one Major chronicle;
/// staying in the band does not spam.
#[test]
fn test_threshold_chronicle_fires_once_per_crossing() {
    let mut world = test_world(0.751);
    spawn_drill(&mut world, true, 10, 10);
    run_process(&mut world); // 0.751 -> 0.750: crosses 75%
    let texts = chronicle_texts(&mut world);
    assert_eq!(texts.len(), 1, "one threshold chronicle, got {texts:?}");
    assert!(texts[0].contains("75%"));
    run_process(&mut world); // 0.750 -> 0.749: same band, no spam
    assert!(chronicle_texts(&mut world).is_empty());
}

/// At 0.0 the breach latches: drills wrecked, real ExplosionEvent +
/// magnitude-10 GeologicalEvent at the drill site, Legendary chronicle,
/// and restart refused forever.
#[test]
fn test_catastrophic_failure_at_zero_stability() {
    let mut world = test_world(0.0005);
    let drill = spawn_drill(&mut world, true, 10, 10);
    run_process(&mut world);

    let stability = world.resource::<PlanetaryStability>();
    assert!(stability.failed);
    assert_eq!(stability.level, 0.0);
    assert!(!world.get::<CoreDrill>(drill).unwrap().active);
    assert_eq!(world.get::<PowerSource>(drill).unwrap().output, 0.0);

    let fired: Vec<_> = world
        .resource_mut::<Events<ExplosionEvent>>()
        .drain()
        .collect();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].damage, 600.0);
    assert_eq!(fired[0].radius, 8);

    let quakes = geological_events(&mut world);
    assert_eq!(quakes.len(), 1);
    assert!((quakes[0].magnitude - 10.0).abs() < f32::EPSILON);

    let texts = chronicle_texts(&mut world);
    assert!(
        texts.iter().any(|t| t.contains("PLANETARY CORE BREACH")),
        "breach chronicle, got {texts:?}"
    );

    // Restart refused, and further ticks change nothing (latched).
    assert!(!set_all_drills_active(&mut world, true));
    assert!(!world.get::<CoreDrill>(drill).unwrap().active);
    run_process(&mut world);
    assert_eq!(world.resource::<PlanetaryStability>().level, 0.0);
}

/// The toggle helper drives every drill (and its power) on and off.
#[test]
fn test_toggle_helper_switches_all_drills() {
    let mut world = test_world(1.0);
    let a = spawn_drill(&mut world, true, 10, 10);
    let b = spawn_drill(&mut world, true, 11, 10);
    assert!(set_all_drills_active(&mut world, false));
    for e in [a, b] {
        assert!(!world.get::<CoreDrill>(e).unwrap().active);
        assert_eq!(world.get::<PowerSource>(e).unwrap().output, 0.0);
    }
    assert!(toggle_all_drills(&mut world));
    for e in [a, b] {
        assert!(world.get::<CoreDrill>(e).unwrap().active);
        assert_eq!(
            world.get::<PowerSource>(e).unwrap().output,
            CORE_DRILL_OUTPUT
        );
    }
    let (total, active) = core_drill_counts(&mut world);
    assert_eq!((total, active), (2, 2));
}

/// The spec's pure scaling formulas, exactly.
#[test]
fn test_quake_scaling_helpers_match_spec_formulas() {
    assert!((quake_chance(1.0) - 0.0).abs() < f32::EPSILON);
    assert!((quake_chance(0.0) - 0.05).abs() < f32::EPSILON);
    assert!((quake_chance(0.5) - 0.025).abs() < f32::EPSILON);
    assert!((quake_magnitude(1.0) - 5.0).abs() < f32::EPSILON);
    assert!((quake_magnitude(0.0) - 10.0).abs() < f32::EPSILON);
    assert!((quake_magnitude(0.5) - 7.5).abs() < f32::EPSILON);
    // High stability never quakes through the random path.
    let mut world = test_world(1.0);
    run_trigger(&mut world);
    assert!(geological_events(&mut world).is_empty());
}

/// IP guard: every public-facing string uses original names only.
#[test]
fn test_ip_guard_original_names_only() {
    let banned = [
        "rimworld",
        "dwarf fortress",
        "stellaris",
        "star wars",
        "star trek",
        "warhammer",
        "dune",
        "mass effect",
        "factorio",
        "oxygen not included",
        "deep rock",
        "helldivers",
    ];
    for text in core_tapping_public_strings() {
        let lower = text.to_lowercase();
        for word in banned {
            assert!(
                !lower.contains(word),
                "IP-guard: '{text}' contains banned term '{word}'"
            );
        }
    }
    assert_eq!(crate::layer1::building::BuildingType::CoreDrill.label(), "Core Drill");
}
