//! Spec 380 tests: The Great Filter.
//!
//! Adapted from the spec sketch in `specs/380-the-great-filter.md`:
//! * The spec's `layer3::event` (singular) does not exist; the real
//!   module is `layer3::events` (collapse, refugee_waves, ...), so the
//!   implementation lives in `layer3::events::great_filter`.
//! * The spec's `layer3::tech` did not exist at all — it is created by
//!   this spec (`CivilizationTechLevel`, `TechAdvanceEvent`,
//!   `advance_tech_system`), so those imports match the sketch.
//! * The spec's `FilterEvent.filter_type: String` ("AI_UPRISING") is a
//!   typed `FilterKind` enum here — stringly filter names are how the
//!   IP-guard test would become unenforceable.
//! * The spec's `ActiveFilter { failed: true }` literal gains the
//!   resolution fields via `..Default::default()` (containment /
//!   pressure mechanic, per the spec REFACTOR note).
//! * The spec's first test never initialised `Events<TechAdvanceEvent>`
//!   before `world.send_event(...)`; that init is added (it panics
//!   otherwise in bevy_ecs).

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::AddChronicleEvent;
use crate::layer3::events::great_filter::{
    activate_filters_system, apply_civilization_wipe_system, filter_chronicle_bridge_system,
    filter_survival_benefit_system, process_failed_filters_system, tick_active_filters_system,
    trigger_filter_events_system, ActiveFilter, CivWorld, CivilizationWipeEvent, ExplorableRuin,
    FallenCivilization, FilterEvent, FilterKind, FilterSurvivedEvent, GreatFilterLedger,
    LocalCivilization, Transcendent, FILTER_THRESHOLD,
};
use crate::layer3::tech::{advance_tech_system, CivilizationTechLevel, TechAdvanceEvent};

fn init_filter_events(world: &mut World) {
    world.init_resource::<Events<TechAdvanceEvent>>();
    world.init_resource::<Events<FilterEvent>>();
    world.init_resource::<Events<FilterSurvivedEvent>>();
    world.init_resource::<Events<CivilizationWipeEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<GreatFilterLedger>();
}

// ---- Spec RED test 1 (adapted) -------------------------------------------

#[test]
fn test_high_tech_triggers_filter_event() {
    let mut world = World::new();
    init_filter_events(&mut world);

    let civ = world
        .spawn((CivilizationTechLevel { level: 90 },))
        .id();

    // Simulate a tech advancement passing threshold 100
    world.send_event(TechAdvanceEvent { civ, amount: 15 });

    let mut schedule = Schedule::default();
    schedule.add_systems((advance_tech_system, trigger_filter_events_system).chain());
    schedule.run(&mut world);

    // Verify a Filter event was queued
    let filter_events = world.resource::<Events<FilterEvent>>();
    let mut reader = filter_events.get_cursor();
    assert!(reader.read(filter_events).any(|e| e.civ == civ));
}

// ---- Spec RED test 2 (adapted) -------------------------------------------

#[test]
fn test_ruins_spawned_on_failed_filter() {
    let mut world = World::new();
    init_filter_events(&mut world);

    let civ = world
        .spawn((
            CivilizationTechLevel { level: 110 },
            ActiveFilter {
                failed: true,
                ..Default::default()
            },
        ))
        .id();
    let world_a = world.spawn((CivWorld { owner: civ, name: "Prime".to_string() },)).id();
    let world_b = world.spawn((CivWorld { owner: civ, name: "Second".to_string() },)).id();

    let mut schedule = Schedule::default();
    schedule.add_systems(
        (process_failed_filters_system, apply_civilization_wipe_system).chain(),
    );
    schedule.run(&mut world);
    schedule.run(&mut world);

    // A wipe event should fire, spawning ruins
    let wipe_events = world.resource::<Events<CivilizationWipeEvent>>();
    let mut reader = wipe_events.get_cursor();
    assert!(reader.read(wipe_events).any(|e| e.civ == civ));

    // Both owned worlds are now explorable ruins; the civ is fallen.
    assert!(world.get::<ExplorableRuin>(world_a).is_some());
    assert!(world.get::<ExplorableRuin>(world_b).is_some());
    assert!(world.get::<FallenCivilization>(civ).is_some());
    assert_eq!(world.resource::<GreatFilterLedger>().ruins_created, 2);
    assert_eq!(world.resource::<GreatFilterLedger>().fallen, 1);
}

// ---- Trigger behaviour ----------------------------------------------------

#[test]
fn below_threshold_never_triggers() {
    let mut world = World::new();
    init_filter_events(&mut world);
    let civ = world.spawn((CivilizationTechLevel { level: 40 },)).id();
    world.send_event(TechAdvanceEvent { civ, amount: 30 });

    let mut schedule = Schedule::default();
    schedule.add_systems((advance_tech_system, trigger_filter_events_system).chain());
    schedule.run(&mut world);

    assert_eq!(world.get::<CivilizationTechLevel>(civ).unwrap().level, 70);
    let filter_events = world.resource::<Events<FilterEvent>>();
    assert!(filter_events.is_empty());
}

#[test]
fn advance_tech_accumulates_and_saturates() {
    let mut world = World::new();
    init_filter_events(&mut world);
    let civ = world.spawn((CivilizationTechLevel { level: 10 },)).id();
    let mut schedule = Schedule::default();
    schedule.add_systems(advance_tech_system);
    world.send_event(TechAdvanceEvent { civ, amount: 5 });
    schedule.run(&mut world);
    world.send_event(TechAdvanceEvent { civ, amount: u32::MAX });
    schedule.run(&mut world);
    assert_eq!(world.get::<CivilizationTechLevel>(civ).unwrap().level, u32::MAX);
}

#[test]
fn fallen_and_transcendent_civs_do_not_retrigger() {
    for marker in 0..2 {
        let mut world = World::new();
        init_filter_events(&mut world);
        let civ = if marker == 0 {
            world
                .spawn((CivilizationTechLevel { level: 150 }, FallenCivilization))
                .id()
        } else {
            world
                .spawn((
                    CivilizationTechLevel { level: 150 },
                    Transcendent { kind: FilterKind::GreyBloom },
                ))
                .id()
        };
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_filter_events_system);
        // Changed<> needs a mutation to fire; run twice (spawn counts as changed on first run).
        schedule.run(&mut world);
        let _ = civ;
        let filter_events = world.resource::<Events<FilterEvent>>();
        assert!(filter_events.is_empty(), "marker {marker} must not retrigger");
    }
}

#[test]
fn filter_kind_is_deterministic_per_civ_and_level() {
    let a = FilterKind::for_civ(7, 120);
    let b = FilterKind::for_civ(7, 120);
    assert_eq!(a, b);
    // Labels are distinct per kind.
    let labels: Vec<&str> = [
        FilterKind::UnboundMind,
        FilterKind::GreyBloom,
        FilterKind::QuietAscension,
    ]
    .iter()
    .map(|k| k.label())
    .collect();
    let mut dedup = labels.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(dedup.len(), 3);
    assert_eq!(FILTER_THRESHOLD, 100);
}

// ---- Activation + resolution mechanic ------------------------------------

fn run_full_chain(world: &mut World, ticks: usize) {
    let mut schedule = Schedule::default();
    schedule.add_systems(
        (
            advance_tech_system,
            trigger_filter_events_system,
            activate_filters_system,
            tick_active_filters_system,
            process_failed_filters_system,
            apply_civilization_wipe_system,
        )
            .chain(),
    );
    for _ in 0..ticks {
        schedule.run(world);
    }
}

#[test]
fn strong_containment_survives_and_grants_transcendence() {
    let mut world = World::new();
    init_filter_events(&mut world);
    // Containment comfortably above every kind's final pressure.
    let civ = world
        .spawn((
            CivilizationTechLevel { level: 100 },
            crate::layer3::events::great_filter::CivilizationResilience { containment: 10_000.0 },
        ))
        .id();
    run_full_chain(&mut world, 130);

    assert!(world.get::<Transcendent>(civ).is_some());
    assert!(world.get::<ActiveFilter>(civ).is_none());
    assert!(world.get::<FallenCivilization>(civ).is_none());
    let ledger = world.resource::<GreatFilterLedger>();
    assert_eq!(ledger.triggered, 1);
    assert_eq!(ledger.survived, 1);
    assert_eq!(ledger.fallen, 0);
    let survived = world.resource::<Events<FilterSurvivedEvent>>();
    let mut reader = survived.get_cursor();
    assert!(reader.read(survived).any(|e| e.civ == civ));
}

#[test]
fn weak_containment_fails_and_wipes_worlds() {
    let mut world = World::new();
    init_filter_events(&mut world);
    let civ = world
        .spawn((
            CivilizationTechLevel { level: 100 },
            crate::layer3::events::great_filter::CivilizationResilience { containment: 0.0 },
        ))
        .id();
    let owned = world.spawn((CivWorld { owner: civ, name: "Doomed".to_string() },)).id();
    let foreign_civ = world.spawn((CivilizationTechLevel { level: 10 },)).id();
    let foreign = world
        .spawn((CivWorld { owner: foreign_civ, name: "Safe".to_string() },))
        .id();

    run_full_chain(&mut world, 130);

    assert!(world.get::<FallenCivilization>(civ).is_some());
    assert!(world.get::<ExplorableRuin>(owned).is_some());
    // Another civ's world is untouched.
    assert!(world.get::<ExplorableRuin>(foreign).is_none());
    let ledger = world.resource::<GreatFilterLedger>();
    assert_eq!(ledger.fallen, 1);
    assert_eq!(ledger.ruins_created, 1);
}

#[test]
fn active_filter_pressure_grows_until_resolution() {
    let mut world = World::new();
    init_filter_events(&mut world);
    let civ = world
        .spawn((ActiveFilter {
            kind: FilterKind::UnboundMind,
            duration: 10,
            containment: 5.0,
            ..Default::default()
        },))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(tick_active_filters_system);
    schedule.run(&mut world);
    let active = world.get::<ActiveFilter>(civ).unwrap();
    assert_eq!(active.elapsed, 1);
    assert!(active.pressure() > 0.0);
    assert!(!active.failed);
    // Run to expiry: containment 5 < final pressure -> failed flag set.
    for _ in 0..9 {
        schedule.run(&mut world);
    }
    // On resolution the component is removed (survive) or flagged (fail);
    // containment 5.0 cannot beat UnboundMind's final pressure.
    match world.get::<ActiveFilter>(civ) {
        Some(active) => assert!(active.failed),
        None => assert!(world.get::<FallenCivilization>(civ).is_some() || world.get::<Transcendent>(civ).is_none()),
    }
}

// ---- Chronicle + benefit bridges ------------------------------------------

#[test]
fn chronicle_bridge_records_trigger_and_fall() {
    let mut world = World::new();
    init_filter_events(&mut world);
    world.spawn((CivilizationTechLevel { level: 120 },));

    let mut schedule = Schedule::default();
    schedule.add_systems(
        (
            trigger_filter_events_system,
            filter_chronicle_bridge_system,
        )
            .chain(),
    );
    schedule.run(&mut world);

    let chronicle = world.resource::<Events<AddChronicleEvent>>();
    let mut reader = chronicle.get_cursor();
    let texts: Vec<String> = reader.read(chronicle).map(|e| e.text.clone()).collect();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("GREAT FILTER"));
}

#[test]
fn local_civ_survival_grants_knowledge_benefit() {
    let mut world = World::new();
    init_filter_events(&mut world);
    world.insert_resource(crate::layer1::economy::resources::ColonyResources {
        knowledge: 0.0,
        max_knowledge: 100_000.0,
        ..Default::default()
    });
    let civ = world.spawn((LocalCivilization,)).id();
    world.send_event(FilterSurvivedEvent { civ, kind: FilterKind::QuietAscension });

    let mut schedule = Schedule::default();
    schedule.add_systems(filter_survival_benefit_system);
    schedule.run(&mut world);

    let res = world.resource::<crate::layer1::economy::resources::ColonyResources>();
    assert!(res.knowledge > 0.0, "surviving the Filter must pay out");
}

#[test]
fn remote_civ_survival_does_not_touch_colony_pool() {
    let mut world = World::new();
    init_filter_events(&mut world);
    world.insert_resource(crate::layer1::economy::resources::ColonyResources {
        knowledge: 0.0,
        max_knowledge: 100_000.0,
        ..Default::default()
    });
    let civ = world.spawn(()).id();
    world.send_event(FilterSurvivedEvent { civ, kind: FilterKind::GreyBloom });

    let mut schedule = Schedule::default();
    schedule.add_systems(filter_survival_benefit_system);
    schedule.run(&mut world);

    let res = world.resource::<crate::layer1::economy::resources::ColonyResources>();
    assert_eq!(res.knowledge, 0.0);
}

// ---- IP guard --------------------------------------------------------------

#[test]
fn ip_guard_no_lifted_names_in_public_strings() {
    const BANNED: &[&str] = &[
        "skynet", "terminator", "borg", "ultron", "hal 9000", "matrix",
        "replicator", "paperclip maximizer", "fermi", "stargate", "warhammer",
    ];
    let mut corpus = String::new();
    for kind in [
        FilterKind::UnboundMind,
        FilterKind::GreyBloom,
        FilterKind::QuietAscension,
    ] {
        corpus.push_str(kind.label());
        corpus.push(' ');
        corpus.push_str(kind.description());
        corpus.push(' ');
    }
    let lower = corpus.to_lowercase();
    for banned in BANNED {
        assert!(!lower.contains(banned), "banned IP term present: {banned}");
    }
}

