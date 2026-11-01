//! Spec 384 Orbital Synch — tests (written RED first).
//!
//! Adaptations from the spec sketch:
//! * The spec's `PlanetaryOrbit { angle }` resource does not exist. The
//!   real angle lives on the colony planet's `Orbit` component
//!   (`layer2::system`, radians, advanced by spec 379's
//!   `advance_orbits_system`); the colony planet is the entity carrying
//!   `ColonyLocation` (`layer2::generation`). Tests spawn exactly that.
//! * The spec's sketch buildings (Solar Array / Tidal Generator /
//!   Telescope) map to the real `BuildingType::SolarPanel` (a
//!   `PowerSource`) and `BuildingType::Observatory` (the telescope).
//!   The fuel `Generator` is the steady-power contrast and is never
//!   synch-gated.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::AddChronicleEvent;
use crate::layer1::energy::PowerSource;
use crate::layer2::generation::ColonyLocation;
use crate::layer2::logistics::orbital_synch::{
    angle_in_window, apply_orbital_synch_spawn_system, colony_angle_degrees,
    set_colony_angle_degrees, update_orbital_synch_system, OrbitalSynch, SynchChangedEvent,
    SynchKind, SynchLedger,
};
use crate::layer2::system::Orbit;
use bevy_ecs::prelude::*;
use std::f32::consts::TAU;

fn spawn_colony(world: &mut World, angle_degrees: f32) -> Entity {
    let star = world.spawn_empty().id();
    world
        .spawn((
            ColonyLocation,
            Orbit {
                parent: star,
                radius: 25.0,
                speed: 0.0,
                angle: angle_degrees.to_radians().rem_euclid(TAU),
            },
        ))
        .id()
}

fn synch_app(angle_degrees: f32) -> (World, Schedule, Entity) {
    let mut world = World::new();
    world.insert_resource(SynchLedger::default());
    world.init_resource::<Events<SynchChangedEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    let colony = spawn_colony(&mut world, angle_degrees);
    let mut schedule = Schedule::default();
    schedule.add_systems(update_orbital_synch_system);
    (world, schedule, colony)
}

fn set_angle(world: &mut World, colony: Entity, angle_degrees: f32) {
    world.get_mut::<Orbit>(colony).unwrap().angle = angle_degrees.to_radians().rem_euclid(TAU);
}

// --- Spec RED tests, adapted -------------------------------------------

#[test]
fn test_building_activates_in_synch_window() {
    let (mut world, mut schedule, _colony) = synch_app(90.0);
    let building = world
        .spawn(OrbitalSynch::new(SynchKind::Telescope, 80.0, 100.0))
        .id();
    assert!(!world.get::<OrbitalSynch>(building).unwrap().active);

    schedule.run(&mut world);

    assert!(
        world.get::<OrbitalSynch>(building).unwrap().active,
        "building should be active within its orbital window"
    );
}

#[test]
fn test_building_deactivates_outside_window() {
    let (mut world, mut schedule, _colony) = synch_app(150.0);
    let building = world
        .spawn(OrbitalSynch {
            kind: SynchKind::Telescope,
            active: true,
            required_angle_start: 80.0,
            required_angle_end: 100.0,
        })
        .id();

    schedule.run(&mut world);

    assert!(
        !world.get::<OrbitalSynch>(building).unwrap().active,
        "building should be inactive outside its orbital window"
    );
}

// --- Wraparound (spec REFACTOR: 350 -> 10) -------------------------------

#[test]
fn test_wraparound_window_active_on_both_sides_of_zero() {
    assert!(angle_in_window(355.0, 350.0, 10.0));
    assert!(angle_in_window(5.0, 350.0, 10.0));
    assert!(angle_in_window(350.0, 350.0, 10.0), "start is inclusive");
    assert!(angle_in_window(10.0, 350.0, 10.0), "end is inclusive");
    assert!(!angle_in_window(180.0, 350.0, 10.0));
    assert!(!angle_in_window(349.9, 350.0, 10.0));
}

#[test]
fn test_wraparound_window_gates_building_across_zero() {
    let (mut world, mut schedule, colony) = synch_app(355.0);
    let building = world
        .spawn(OrbitalSynch::new(SynchKind::Solar, 350.0, 10.0))
        .id();

    schedule.run(&mut world);
    assert!(world.get::<OrbitalSynch>(building).unwrap().active);

    set_angle(&mut world, colony, 180.0);
    schedule.run(&mut world);
    assert!(!world.get::<OrbitalSynch>(building).unwrap().active);
}

// --- Energy seam (spec Technical Guidance: out of synch = zero) ---------

#[test]
fn test_out_of_synch_power_source_is_switched_off() {
    let (mut world, mut schedule, _colony) = synch_app(180.0);
    let panel = world
        .spawn((
            OrbitalSynch::for_kind(SynchKind::Solar),
            PowerSource {
                output: 10.0,
                active: true,
            },
        ))
        .id();

    schedule.run(&mut world);

    let source = world.get::<PowerSource>(panel).unwrap();
    assert!(
        !source.active,
        "out-of-synch source must be inactive — the grid BFS only counts active sources"
    );
}

#[test]
fn test_in_synch_power_source_stays_on() {
    // Solar default window is 300 -> 60 (wraps); 0 degrees is inside it.
    let (mut world, mut schedule, _colony) = synch_app(0.0);
    let panel = world
        .spawn((
            OrbitalSynch::for_kind(SynchKind::Solar),
            PowerSource {
                output: 10.0,
                active: true,
            },
        ))
        .id();

    schedule.run(&mut world);

    assert!(world.get::<PowerSource>(panel).unwrap().active);
    assert!(world.get::<OrbitalSynch>(panel).unwrap().active);
}

// --- Observability: events fire on edges only, chronicle follows --------

#[test]
fn test_synch_change_event_fires_on_edges_only() {
    let (mut world, mut schedule, colony) = synch_app(90.0);
    let _building = world
        .spawn(OrbitalSynch::new(SynchKind::Telescope, 80.0, 100.0))
        .id();

    schedule.run(&mut world);
    schedule.run(&mut world);
    schedule.run(&mut world);

    let events = world.resource::<Events<SynchChangedEvent>>();
    let mut reader = events.get_cursor();
    let fired: Vec<_> = reader.read(events).collect();
    assert_eq!(fired.len(), 1, "one rising edge, no flip-flop spam");
    assert!(fired[0].active);

    let chronicle = world.resource::<Events<AddChronicleEvent>>();
    let mut chron_reader = chronicle.get_cursor();
    assert_eq!(
        chron_reader.read(chronicle).count(),
        1,
        "rising edge writes exactly one chronicle entry"
    );

    set_angle(&mut world, colony, 200.0);
    schedule.run(&mut world);
    let events = world.resource::<Events<SynchChangedEvent>>();
    let fired: Vec<_> = reader.read(events).collect();
    assert_eq!(fired.len(), 1, "one falling edge");
    assert!(!fired[0].active);
    assert_eq!(world.resource::<SynchLedger>().transitions, 2);
}

// --- Spawn tagging: SolarPanel + Observatory only ------------------------

#[test]
fn test_spawn_system_tags_solar_and_telescope_only() {
    let mut world = World::new();
    let mut schedule = Schedule::default();
    schedule.add_systems(apply_orbital_synch_spawn_system);

    let panel = world
        .spawn(Building {
            building_type: BuildingType::SolarPanel,
        })
        .id();
    let scope = world
        .spawn(Building {
            building_type: BuildingType::Observatory,
        })
        .id();
    let generator = world
        .spawn(Building {
            building_type: BuildingType::Generator,
        })
        .id();

    schedule.run(&mut world);

    let panel_synch = world.get::<OrbitalSynch>(panel).expect("solar tagged");
    assert_eq!(panel_synch.kind, SynchKind::Solar);
    let scope_synch = world.get::<OrbitalSynch>(scope).expect("telescope tagged");
    assert_eq!(scope_synch.kind, SynchKind::Telescope);
    assert!(
        world.get::<OrbitalSynch>(generator).is_none(),
        "fuel Generator is the steady-power contrast — never synch-gated"
    );
}

// --- Debug seam: move the colony orbit (headless has no DayNightCycle) --

#[test]
fn test_set_colony_angle_wraps_degrees() {
    let mut world = World::new();
    let colony = spawn_colony(&mut world, 10.0);
    let _ = colony;

    assert!(set_colony_angle_degrees(&mut world, 370.0));
    assert!((colony_angle_degrees(&mut world).unwrap() - 10.0).abs() < 0.01);
    assert!(set_colony_angle_degrees(&mut world, -10.0));
    assert!((colony_angle_degrees(&mut world).unwrap() - 350.0).abs() < 0.01);
}

#[test]
fn test_colony_angle_absent_without_colony_body() {
    let mut world = World::new();
    assert!(colony_angle_degrees(&mut world).is_none());
    assert!(!set_colony_angle_degrees(&mut world, 90.0));
}

// --- IP guard -------------------------------------------------------------

#[test]
fn test_ip_guard_public_strings_are_original() {
    const BANNED: &[&str] = &[
        "stellaris",
        "dwarf fortress",
        "star wars",
        "star trek",
        "dune",
        "foundation",
        "kerbal",
        "rimworld",
    ];
    let mut public_strings = vec![
        SynchKind::Solar.label().to_string(),
        SynchKind::Telescope.label().to_string(),
        SynchKind::Solar.window_label(),
        SynchKind::Telescope.window_label(),
    ];
    public_strings.push(crate::layer2::logistics::orbital_synch::synch_stats_label(
        &mut World::new(),
    ));
    for text in &public_strings {
        let lower = text.to_lowercase();
        for banned in BANNED {
            assert!(
                !lower.contains(banned),
                "public string {text:?} contains banned IP term {banned:?}"
            );
        }
    }
}
