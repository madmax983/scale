use super::*;
use crate::layer1::combat::AttackProperties;
use crate::layer1::psychology::traits::get_trait_work_speed_modifier;
use bevy_ecs::system::RunSystemOnce;

#[test]
fn test_infection_spreads_and_progresses() {
    let mut world = World::new();
    let infected = world.spawn((Pop, HiveSpore { stage: InfectionStage::Latent, spread_cooldown: 0.0 }, Traits::default())).id();
    let _healthy = world.spawn((Pop, Traits::default())).id();
    world.run_system_once(process_hive_infection_system).unwrap();
    let spore = world.get::<HiveSpore>(infected).unwrap();
    assert!(matches!(spore.stage, InfectionStage::Active) || spore.spread_cooldown > 0.0);
}

#[test]
fn test_infected_pops_gain_efficiency_buff() {
    let mut world = World::new();
    let infected = world.spawn((Pop, HiveSpore { stage: InfectionStage::Active, spread_cooldown: 0.0 }, Traits::default())).id();
    world.run_system_once(process_hive_infection_system).unwrap();
    let traits = world.get::<Traits>(infected).unwrap();
    assert!(traits.0.contains(&Trait::HiveCoordination));
    assert!(get_trait_work_speed_modifier(traits) > 1.0, "HiveCoordination must wire into the real work-speed consumer");
    assert!((hive_efficiency_multiplier(traits) - 1.25).abs() < f32::EPSILON);
}

#[test]
fn test_progression_reaches_critical() {
    let mut world = World::new();
    let e = world.spawn((Pop, HiveSpore { stage: InfectionStage::Latent, spread_cooldown: 0.0 }, Traits::default())).id();
    for _ in 0..210 { world.run_system_once(process_hive_infection_system).unwrap(); }
    assert_eq!(world.get::<HiveSpore>(e).unwrap().stage, InfectionStage::Critical);
}

#[test]
fn test_critical_mass_triggers_rebellion() {
    let mut world = World::new();
    world.init_resource::<Events<HiveMindRebellionEvent>>();
    for _ in 0..10 { world.spawn((Pop, HiveSpore { stage: InfectionStage::Critical, spread_cooldown: 0.0 })); }
    world.run_system_once(trigger_hive_rebellion_system).unwrap();
    let events = world.resource::<Events<HiveMindRebellionEvent>>();
    let mut reader = events.get_reader();
    assert!(reader.read(events).next().is_some());
}

#[test]
fn test_below_critical_mass_no_rebellion() {
    let mut world = World::new();
    world.init_resource::<Events<HiveMindRebellionEvent>>();
    for _ in 0..9 { world.spawn((Pop, HiveSpore { stage: InfectionStage::Critical, spread_cooldown: 0.0 })); }
    world.run_system_once(trigger_hive_rebellion_system).unwrap();
    let events = world.resource::<Events<HiveMindRebellionEvent>>();
    let mut reader = events.get_reader();
    assert!(reader.read(events).next().is_none());
}

#[test]
fn test_proximity_spread_and_range_limit() {
    let mut world = World::new();
    world.spawn((Pop, HiveSpore { stage: InfectionStage::Active, spread_cooldown: 5.0 }, GridPosition { x: 0, y: 0 }, Traits::default()));
    let near = world.spawn((Pop, GridPosition { x: 1, y: 0 }, Traits::default())).id();
    let far = world.spawn((Pop, GridPosition { x: 20, y: 20 }, Traits::default())).id();
    world.run_system_once(spread_hive_infection_system).unwrap();
    // Commands are deferred in a bare World; flush via a second empty run pattern:
    // run_system_once applies deferred, so assert directly.
    assert!(world.get::<HiveSpore>(near).is_some(), "near pop should be infected");
    assert_eq!(world.get::<HiveSpore>(near).unwrap().stage, InfectionStage::Latent);
    assert!(world.get::<HiveSpore>(far).is_none(), "far pop should stay clean");
}

#[test]
fn test_rebellion_transforms_to_hive_faction() {
    let mut world = World::new();
    world.init_resource::<Events<HiveMindRebellionEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.insert_resource(Factions::default());
    world.insert_resource(HiveMindState::default());
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(world.spawn((Pop, HiveSpore { stage: InfectionStage::Critical, spread_cooldown: 0.0 }, FactionMember { faction_id: Some(FactionId::Unaligned) }, Traits::default())).id());
    }
    world.run_system_once(trigger_hive_rebellion_system).unwrap();
    world.run_system_once(handle_hive_rebellion_system).unwrap();
    for id in &ids {
        assert_eq!(world.get::<FactionMember>(*id).unwrap().faction_id, Some(FactionId::HiveMind));
        assert!(world.get::<IntegratedCollective>(*id).is_some());
    }
    assert!(world.resource::<Factions>().map.contains_key(&FactionId::HiveMind));
    assert_eq!(world.resource::<Factions>().map[&FactionId::HiveMind].state, FactionState::Striking);
    assert!(world.resource::<HiveMindState>().rebellion_fired);
    let ev = world.resource::<Events<AddChronicleEvent>>();
    let mut r = ev.get_reader();
    let texts: Vec<String> = r.read(ev).map(|e| e.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("HIVE AWAKENS")), "chronicle: {texts:?}");
    // Second trigger must not refire (state gate): handler runs
    // again but the rebellion count stays at one.
    world.run_system_once(trigger_hive_rebellion_system).unwrap();
    world.run_system_once(handle_hive_rebellion_system).unwrap();
    assert_eq!(world.resource::<HiveMindState>().rebellions, 1);
}

#[test]
fn test_critical_sabotages_nearby_turret() {
    let mut world = World::new();
    world.spawn((Pop, HiveSpore { stage: InfectionStage::Critical, spread_cooldown: 5.0 }, GridPosition { x: 0, y: 0 }));
    let turret = world.spawn((Turret { attack: AttackProperties { damage: 10.0, range: 5.0, cooldown: 1, accuracy: 1.0 }, ammo_cost: 0.0, ammo_type: crate::layer1::resources::ResourceType::Waste }, GridPosition { x: 1, y: 0 }, Health { current: 50.0, max: 50.0, has_rust_lung: false })).id();
    let far_turret = world.spawn((Turret { attack: AttackProperties { damage: 10.0, range: 5.0, cooldown: 1, accuracy: 1.0 }, ammo_cost: 0.0, ammo_type: crate::layer1::resources::ResourceType::Waste }, GridPosition { x: 30, y: 30 })).id();
    world.run_system_once(hive_sabotage_system).unwrap();
    assert!(world.get::<Turret>(turret).unwrap().attack.damage < 10.0);
    assert!(world.get::<Health>(turret).unwrap().current < 50.0);
    assert_eq!(world.get::<Turret>(far_turret).unwrap().attack.damage, 10.0);
}

#[test]
fn test_stats_label_and_infect_helper() {
    let mut world = World::new();
    for _ in 0..3 { world.spawn(crate::layer1::entities::pop::PopBundle::random(0, 0, &mut rand::thread_rng())); }
    assert_eq!(infect_pops(&mut world, 2), 2);
    assert_eq!(hive_stats_label(&mut world), "2/0/0");
}

#[test]
fn test_ip_guard_original_names_only() {
    let banned = ["zerg", "tyranid", "genestealer", "flood", "borg", "warhammer", "starcraft", "halo", "warhammer 40k", "40k", "rimworld", "dwarf fortress", "stellaris"];
    for text in hive_public_strings() {
        let lower = text.to_lowercase();
        for word in banned { assert!(!lower.contains(word), "IP-guard: '{text}' contains '{word}'"); }
    }
}
