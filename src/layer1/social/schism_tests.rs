//! Spec 376 tests for The Schism, adapted from the spec mock-style
//! RED tests to the real codebase: factions are entries in the
//! Factions resource keyed by FactionId (not entities carrying a
//! FactionHappiness component), satisfaction runs 0.0..=1.0, and
//! Building lives at crate::layer1::building. See the module docs
//! in schism.rs for every adaptation.

use bevy_ecs::prelude::*;

use super::schism::*;
use crate::layer1::access_control::{check_access, AccessControl, AccessMode};
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::AddChronicleEvent;
use crate::layer1::factions::{FactionData, FactionId, FactionMember, FactionState, Factions};
use crate::layer1::map::GridPosition;
use crate::layer1::social::morale::Morale;
use crate::shared::log::MessageLog;

fn test_world() -> World {
    let mut world = World::new();
    world.insert_resource(Factions::default());
    world.insert_resource(SchismConfig::default());
    world.insert_resource(SchismState::default());
    world.init_resource::<Events<SchismEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<MessageLog>();
    world
}

fn set_satisfaction(world: &mut World, faction: FactionId, level: f32, members: usize) {
    let mut factions = world.resource_mut::<Factions>();
    factions.map.insert(
        faction,
        FactionData {
            name: "Test Faction".to_string(),
            satisfaction: level,
            members_count: members,
            state: FactionState::Loyal,
            active_demand: None,
        },
    );
}

fn run_process(world: &mut World) {
    let mut schedule = Schedule::default();
    schedule.add_systems(process_schism_system);
    schedule.run(world);
}

fn run_handle(world: &mut World) {
    let mut schedule = Schedule::default();
    schedule.add_systems(handle_schism_event_system);
    schedule.run(world);
}

fn run_both(world: &mut World) {
    // Sequential schedules: an unchained tuple has no ordering
    // guarantee, so process must complete before handle drains.
    run_process(world);
    run_handle(world);
}

fn schism_events(world: &mut World) -> Vec<SchismEvent> {
    world.resource_mut::<Events<SchismEvent>>().drain().collect()
}

fn spawn_building(world: &mut World, kind: BuildingType, x: i32, y: i32) -> Entity {
    world
        .spawn((Building { building_type: kind }, GridPosition { x, y }))
        .id()
}

fn spawn_member(world: &mut World, faction: FactionId, x: i32, y: i32, morale: f32) -> Entity {
    world
        .spawn((
            FactionMember {
                faction_id: Some(faction),
            },
            GridPosition { x, y },
            Morale {
                value: morale,
                modifiers: Vec::new(),
            },
        ))
        .id()
}

/// Spec RED test 1, adapted: low happiness triggers a schism event.
#[test]
fn test_low_happiness_triggers_schism() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.05, 3);
    run_process(&mut world);
    let events = schism_events(&mut world);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].faction, FactionId::MinersGuild);
}

#[test]
fn test_content_faction_does_not_secede() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.90, 3);
    spawn_member(&mut world, FactionId::MinersGuild, 10, 10, 0.85);
    run_process(&mut world);
    assert!(schism_events(&mut world).is_empty());
}

#[test]
fn test_threshold_is_configurable() {
    let mut world = test_world();
    world.insert_resource(SchismConfig { threshold: 0.50 });
    set_satisfaction(&mut world, FactionId::FarmersGuild, 0.40, 2);
    run_process(&mut world);
    assert_eq!(schism_events(&mut world).len(), 1);
}

#[test]
fn test_member_morale_collapse_triggers_even_when_satisfaction_resets() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 1.0, 2);
    spawn_member(&mut world, FactionId::MinersGuild, 10, 10, 0.0);
    spawn_member(&mut world, FactionId::MinersGuild, 11, 10, 0.05);
    run_process(&mut world);
    let events = schism_events(&mut world);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].faction, FactionId::MinersGuild);
}

#[test]
fn test_secession_latches_and_never_refires() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.0, 1);
    run_process(&mut world);
    assert_eq!(schism_events(&mut world).len(), 1);
    run_process(&mut world);
    assert!(schism_events(&mut world).is_empty());
    assert_eq!(seceded_faction_count(&world), 1);
}

/// Spec RED test 2, adapted: handling the event claims territory.
#[test]
fn test_schism_claims_territory() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::ScholarsGuild, 0.0, 1);
    let building = spawn_building(&mut world, BuildingType::Library, 10, 10);
    world.send_event(SchismEvent {
        faction: FactionId::ScholarsGuild,
    });
    run_handle(&mut world);
    let territory = world.get::<SecededTerritory>(building).unwrap();
    assert_eq!(territory.faction, FactionId::ScholarsGuild);
}

#[test]
fn test_cluster_prefers_housing_and_workplaces_near_members() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.0, 2);
    spawn_member(&mut world, FactionId::MinersGuild, 10, 10, 0.0);
    let housing = spawn_building(&mut world, BuildingType::Housing, 11, 10);
    let farm = spawn_building(&mut world, BuildingType::Farm, 12, 10);
    let housing2 = spawn_building(&mut world, BuildingType::Housing, 11, 11);
    let wall_near = spawn_building(&mut world, BuildingType::Wall, 10, 11);
    let far_housing = spawn_building(&mut world, BuildingType::Housing, 60, 60);
    world.send_event(SchismEvent {
        faction: FactionId::MinersGuild,
    });
    run_handle(&mut world);
    assert!(world.get::<SecededTerritory>(housing).is_some());
    assert!(world.get::<SecededTerritory>(farm).is_some());
    assert!(world.get::<SecededTerritory>(housing2).is_some());
    assert!(world.get::<SecededTerritory>(wall_near).is_none());
    assert!(world.get::<SecededTerritory>(far_housing).is_none());
    assert_eq!(seceded_territory_count(&mut world), 3);
}

#[test]
fn test_fallback_claims_one_building_when_nothing_preferred() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::Cartel, 0.0, 0);
    let wall = spawn_building(&mut world, BuildingType::Wall, 5, 5);
    let wall2 = spawn_building(&mut world, BuildingType::Wall, 6, 5);
    world.send_event(SchismEvent {
        faction: FactionId::Cartel,
    });
    run_handle(&mut world);
    assert_eq!(seceded_territory_count(&mut world), 1);
    assert!(
        world.get::<SecededTerritory>(wall).is_some()
            || world.get::<SecededTerritory>(wall2).is_some()
    );
}

#[test]
fn test_doors_bar_outsiders_and_admit_members() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.0, 1);
    let member = spawn_member(&mut world, FactionId::MinersGuild, 10, 10, 0.0);
    let outsider = spawn_member(&mut world, FactionId::FarmersGuild, 20, 20, 0.9);
    let building = spawn_building(&mut world, BuildingType::Housing, 11, 10);
    world.send_event(SchismEvent {
        faction: FactionId::MinersGuild,
    });
    run_handle(&mut world);
    let access = world.get::<AccessControl>(building).unwrap();
    assert_eq!(access.mode, AccessMode::Restricted);
    assert!(access.allowed_pops.contains(&member));
    assert!(check_access(&world, building, member));
    assert!(!check_access(&world, building, outsider));
    assert!(outsider_can_enter(&world, building, member));
    assert!(!outsider_can_enter(&world, building, outsider));
}

#[test]
fn test_schism_writes_chronicle_and_marks_faction_striking() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::MinersGuild, 0.0, 1);
    spawn_building(&mut world, BuildingType::Housing, 10, 10);
    run_both(&mut world);
    let chronicle: Vec<AddChronicleEvent> = world
        .resource_mut::<Events<AddChronicleEvent>>()
        .drain()
        .collect();
    assert_eq!(chronicle.len(), 1);
    assert!(chronicle[0].text.contains("THE SCHISM"));
    assert!(chronicle[0].text.contains("Test Faction"));
    let factions = world.resource::<Factions>();
    assert_eq!(
        factions.get(FactionId::MinersGuild).unwrap().state,
        FactionState::Striking
    );
}

#[test]
fn test_full_pipeline_from_unhappiness_to_territory() {
    let mut world = test_world();
    set_satisfaction(&mut world, FactionId::FarmersGuild, 0.02, 1);
    spawn_member(&mut world, FactionId::FarmersGuild, 8, 8, 0.02);
    let housing = spawn_building(&mut world, BuildingType::Housing, 9, 8);
    run_both(&mut world);
    assert!(world.get::<SecededTerritory>(housing).is_some());
    assert_eq!(seceded_faction_count(&world), 1);
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
    ];
    for text in schism_public_strings() {
        let lower = text.to_lowercase();
        for word in banned {
            assert!(
                !lower.contains(word),
                "IP-guard: banned term found in public string"
            );
        }
    }
}
