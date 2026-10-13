use crate::layer1::building::{Building, BuildingType};
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::energy::PowerSource;
use crate::layer1::pop::{Pop, Speed};
use crate::layer1::quirks::{
    add_quirk_by_name, apply_quirk_modifiers_system, carnivorous_flora_system, quirks_summary,
    tidal_solar_factor, tidal_zone, GlobalModifiers, PlanetaryTrait, PlanetaryTraits, TidalZone,
};
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;

fn setup_world() -> World {
    let mut world = World::new();
    world.insert_resource(PlanetaryTraits::default());
    world.insert_resource(DayNightCycle::default());
    world
}

#[test]
fn test_traits_initialization() {
    let world = setup_world();
    let traits = world.resource::<PlanetaryTraits>();
    assert!(traits.0.is_empty()); // Default is Earth-like (no quirks)
}

#[test]
fn test_high_gravity_slows_movement() {
    let mut world = setup_world();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::HighGravity]));

    // Spawn pop with default speed
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

    // Run modifier system
    world.run_system_once(apply_quirk_modifiers_system).unwrap();

    // Check speed
    let speed = world.get::<Speed>(pop).unwrap();
    // High Gravity = 0.8x speed
    assert!(
        (speed.current - 0.8).abs() < f32::EPSILON,
        "Expected 0.8, got {}",
        speed.current
    );
}

#[test]
fn test_low_gravity_speeds_movement() {
    let mut world = setup_world();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::LowGravity]));

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

    world.run_system_once(apply_quirk_modifiers_system).unwrap();

    let speed = world.get::<Speed>(pop).unwrap();
    // Low Gravity = 1.2x speed
    assert!(
        (speed.current - 1.2).abs() < f32::EPSILON,
        "Expected 1.2, got {}",
        speed.current
    );
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn test_rapid_orbit_shortens_day() {
    let mut world = setup_world();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::RapidOrbit]));

    // Setup DayNightCycle with default
    let default_ticks = 250;
    world.insert_resource(DayNightCycle {
        ticks_per_day: default_ticks,
        ..Default::default()
    });

    // Run modifier system
    world.run_system_once(apply_quirk_modifiers_system).unwrap();

    let cycle = world.resource::<DayNightCycle>();
    // Rapid Orbit = 0.5x day length
    assert_eq!(cycle.ticks_per_day, (default_ticks as f32 * 0.5) as u64);
}

#[test]
fn test_dense_atmosphere_reduces_solar_power() {
    let mut world = setup_world();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::DenseAtmosphere]));

    // Spawn Generator (Solar/Wind implied for this test context, or generic "Generator")
    // If 042 uses "Generator" for all power, we assume atmosphere affects efficiency generally
    // or specifically adds a penalty.
    let generator = world
        .spawn((
            Building {
                building_type: BuildingType::Generator,
            },
            PowerSource {
                output: 10.0,
                ..Default::default()
            },
        ))
        .id();

    world.run_system_once(apply_quirk_modifiers_system).unwrap();

    let source = world.get::<PowerSource>(generator).unwrap();
    // Dense Atmosphere = 0.8x output
    assert!(
        (source.output - 8.0).abs() < f32::EPSILON,
        "Expected 8.0, got {}",
        source.output
    );
}

#[test]
fn test_modifiers_stack() {
    let mut world = setup_world();
    // High Gravity (0.8 speed) + Low Gravity (1.2 speed) = 0.96 speed
    world.insert_resource(PlanetaryTraits(vec![
        PlanetaryTrait::HighGravity,
        PlanetaryTrait::LowGravity,
    ]));

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

    world.run_system_once(apply_quirk_modifiers_system).unwrap();

    let speed = world.get::<Speed>(pop).unwrap();
    assert!(
        (speed.current - 0.96).abs() < f32::EPSILON,
        "Expected 0.96, got {}",
        speed.current
    );
}

// --- Spec 345 RED tests (adapted from the spec's mock sketch to real types) ---
// Spec sketch used PlanetaryQuirks{active_quirks: Vec<Quirk>}, PopMovementCost,
// GlobalTime and a mock SolarPanel. The real codebase already has
// PlanetaryTraits(Vec<PlanetaryTrait>), Speed + movement_cost in
// execution::movement, DayNightCycle, and SolarPower/PowerSource — these
// tests exercise those real types.

#[test]
fn test_345_high_gravity_increases_movement_cost() {
    // Spec RED 1: "High gravity should significantly increase the energy
    // cost of moving." Real movement cost is the per-tile threshold the
    // speed accumulator must reach in execution::movement.
    assert!((PlanetaryTrait::HighGravity.movement_cost_modifier() - 1.5).abs() < f32::EPSILON);
    let mods = GlobalModifiers::from_traits(&[PlanetaryTrait::HighGravity]);
    assert!(mods.movement_cost > 1.0, "cost must rise, got {}", mods.movement_cost);
    assert!((mods.movement_cost - 1.5).abs() < f32::EPSILON);
    // ...and the pre-existing speed effect still applies (0.8x).
    assert!((mods.movement_speed - 0.8).abs() < f32::EPSILON);
}

#[test]
fn test_345_tidally_locked_creates_permanent_day() {
    // Spec RED 2 adapted: at "night" a solar device on the day side of a
    // tidally locked world still produces power (real SolarPower system).
    use crate::layer1::day_night::TimeOfDay;
    use crate::layer1::solar::{update_solar_output_system, SolarCycleState, SolarPower};

    let mut world = World::new();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::TidallyLocked]));
    world.insert_resource(SolarCycleState::default()); // Minimum = 0.8x
    world.insert_resource(DayNightCycle {
        time_of_day: TimeOfDay::Night,
        ..Default::default()
    });
    let panel = world
        .spawn((
            PowerSource {
                output: 0.0,
                ..Default::default()
            },
            SolarPower { base_output: 10.0 },
        ))
        .id();

    world.run_system_once(update_solar_output_system).unwrap();

    let source = world.get::<PowerSource>(panel).unwrap();
    assert!(
        source.output > 0.0,
        "tidally locked day side must keep producing at night, got {}",
        source.output
    );
    assert!((source.output - 8.0).abs() < 0.001);
}

#[test]
fn test_345_tidal_zones_split_the_grid() {
    // REFACTOR phase: permanent day side / terminator / night side thirds.
    assert_eq!(tidal_zone(0, 90), TidalZone::DaySide);
    assert_eq!(tidal_zone(29, 90), TidalZone::DaySide);
    assert_eq!(tidal_zone(30, 90), TidalZone::Terminator);
    assert_eq!(tidal_zone(89, 90), TidalZone::NightSide);
    assert!((tidal_solar_factor(0, 90) - 1.0).abs() < f32::EPSILON);
    assert!((tidal_solar_factor(45, 90) - 0.5).abs() < f32::EPSILON);
    assert!(tidal_solar_factor(89, 90).abs() < f32::EPSILON);
}

#[test]
fn test_345_tidal_night_side_panel_gets_no_sun_even_at_day() {
    use crate::layer1::day_night::TimeOfDay;
    use crate::layer1::map::GridPosition;
    use crate::layer1::solar::{update_solar_output_system, SolarCycleState, SolarPower};

    let mut world = World::new();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::TidallyLocked]));
    world.insert_resource(SolarCycleState::default());
    world.insert_resource(DayNightCycle {
        time_of_day: TimeOfDay::Day,
        ..Default::default()
    });
    // Colony grid is 80 wide: x=70 sits in the night-side third.
    let panel = world
        .spawn((
            PowerSource {
                output: 5.0,
                ..Default::default()
            },
            SolarPower { base_output: 10.0 },
            GridPosition { x: 70, y: 10 },
        ))
        .id();

    world.run_system_once(update_solar_output_system).unwrap();
    let source = world.get::<PowerSource>(panel).unwrap();
    assert!(source.output.abs() < f32::EPSILON, "night side must stay dark, got {}", source.output);
}

#[test]
fn test_345_day_night_cycle_freezes_on_tidally_locked() {
    use crate::layer1::day_night::{update_day_night_cycle_system, TimeOfDay};
    use crate::shared::time::SimulationTime;

    let mut world = World::new();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::TidallyLocked]));
    world.insert_resource(SimulationTime {
        tick: 240, // 96% through a 250-tick day = deep night, unlocked
        ..Default::default()
    });
    world.insert_resource(DayNightCycle::default());

    world.run_system_once(update_day_night_cycle_system).unwrap();
    assert_eq!(world.resource::<DayNightCycle>().time_of_day, TimeOfDay::Day);
}

#[test]
fn test_345_dense_atmosphere_wind_double_solar_half() {
    // Spec §7 guidance: DenseAtmosphere = wind power x2, solar power / 2.
    assert!((PlanetaryTrait::DenseAtmosphere.solar_modifier() - 0.5).abs() < f32::EPSILON);
    assert!((PlanetaryTrait::DenseAtmosphere.wind_modifier() - 2.0).abs() < f32::EPSILON);
    let mods = GlobalModifiers::from_traits(&[PlanetaryTrait::DenseAtmosphere]);
    assert!((mods.solar - 0.5).abs() < f32::EPSILON);
    assert!((mods.wind - 2.0).abs() < f32::EPSILON);
}

#[test]
fn test_345_volcanic_geothermal_and_heat() {
    // Spec §7 guidance: Volcanic = geothermal power x2, higher temperature.
    assert!((PlanetaryTrait::Volcanic.geothermal_modifier() - 2.0).abs() < f32::EPSILON);
    assert!((PlanetaryTrait::Volcanic.temperature_offset() - 5.0).abs() < f32::EPSILON);
    let mods = GlobalModifiers::from_traits(&[PlanetaryTrait::Volcanic]);
    assert!((mods.geothermal - 2.0).abs() < f32::EPSILON);
    assert!((mods.temperature_offset - 5.0).abs() < f32::EPSILON);
}

#[test]
fn test_345_carnivorous_flora_bites_neighbouring_pop() {
    use crate::layer1::flora::{Flora, FloraType};
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;

    let mut world = World::new();
    world.insert_resource(PlanetaryTraits(vec![PlanetaryTrait::CarnivorousFlora]));
    world.spawn((
        Flora {
            flora_type: FloraType::StrangleVines,
            ..Default::default()
        },
        GridPosition { x: 5, y: 5 },
    ));
    let pop = world
        .spawn((
            Pop,
            Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            },
            GridPosition { x: 6, y: 5 },
        ))
        .id();

    world.run_system_once(carnivorous_flora_system).unwrap();
    let health = world.get::<Health>(pop).unwrap();
    assert!(health.current < 100.0, "vines should bite, health {}", health.current);
    assert!((health.current - 98.5).abs() < f32::EPSILON);
}

#[test]
fn test_345_flora_is_harmless_without_the_quirk() {
    use crate::layer1::flora::{Flora, FloraType};
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;

    let mut world = World::new();
    world.insert_resource(PlanetaryTraits::default());
    world.spawn((
        Flora {
            flora_type: FloraType::StrangleVines,
            ..Default::default()
        },
        GridPosition { x: 5, y: 5 },
    ));
    let pop = world
        .spawn((
            Pop,
            Health {
                current: 100.0,
                max: 100.0,
                has_rust_lung: false,
            },
            GridPosition { x: 5, y: 5 },
        ))
        .id();

    world.run_system_once(carnivorous_flora_system).unwrap();
    assert!((world.get::<Health>(pop).unwrap().current - 100.0).abs() < f32::EPSILON);
}

#[test]
fn test_345_global_modifiers_neutral_default() {
    let mods = GlobalModifiers::default();
    assert!((mods.movement_speed - 1.0).abs() < f32::EPSILON);
    assert!((mods.movement_cost - 1.0).abs() < f32::EPSILON);
    assert!((mods.solar - 1.0).abs() < f32::EPSILON);
    assert!((mods.wind - 1.0).abs() < f32::EPSILON);
    assert!((mods.geothermal - 1.0).abs() < f32::EPSILON);
    assert!(mods.temperature_offset.abs() < f32::EPSILON);
    assert!(!mods.tidally_locked);
    assert!(!mods.carnivorous_flora);
    assert_eq!(GlobalModifiers::from_traits(&[]), GlobalModifiers::default());
}

#[test]
fn test_345_quirk_parse_names_and_ip_guard() {
    assert_eq!(PlanetaryTrait::parse("high_gravity"), Some(PlanetaryTrait::HighGravity));
    assert_eq!(PlanetaryTrait::parse("HighGravity"), Some(PlanetaryTrait::HighGravity));
    assert_eq!(PlanetaryTrait::parse("tidally-locked"), Some(PlanetaryTrait::TidallyLocked));
    assert_eq!(PlanetaryTrait::parse("carnivorous flora"), Some(PlanetaryTrait::CarnivorousFlora));
    assert_eq!(PlanetaryTrait::parse("volcanic"), Some(PlanetaryTrait::Volcanic));
    assert_eq!(PlanetaryTrait::parse("bogus"), None);
    // IP guard: public labels are plain generic words, no borrowed IP and
    // none of the spec sketch's mock type names leak into player text.
    for quirk in PlanetaryTrait::ALL {
        let label = quirk.label();
        for banned in ["PopMovementCost", "GlobalTime", "SolarPanel"] {
            assert!(!label.contains(banned), "label leaked {banned}: {label}");
        }
    }
}

#[test]
fn test_345_add_quirk_by_name_updates_world() {
    let mut world = World::new();
    world.insert_resource(PlanetaryTraits::default());
    world.insert_resource(GlobalModifiers::default());
    assert_eq!(quirks_summary(&world), "none");

    let added = add_quirk_by_name(&mut world, "volcanic").expect("volcanic parses");
    assert_eq!(added, PlanetaryTrait::Volcanic);
    assert_eq!(quirks_summary(&world), "Volcanic");
    // Duplicate adds do not stack the same quirk twice.
    add_quirk_by_name(&mut world, "volcanic").expect("idempotent");
    assert_eq!(world.resource::<PlanetaryTraits>().0.len(), 1);
    assert!(add_quirk_by_name(&mut world, "bogus").is_err());
}
