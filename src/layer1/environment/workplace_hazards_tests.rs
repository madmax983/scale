#[cfg(test)]
mod tests {
    use crate::layer1::administration::edicts::{
        get_hazard_modifier, get_work_speed_modifier, ColonyPolicies, Policy,
    };
    use crate::layer1::architecture::building::{Building, BuildingType};
    use crate::layer1::chronicle::AddChronicleEvent;
    use crate::layer1::environment::hazards::{AccidentSeverity, AmputationEvent};
    use crate::layer1::environment::workplace_hazards::{
        building_accident_risk, process_workplace_hazards_system, safety_risk_multiplier,
        should_trigger_accident, workplace_accident_injury_system, workplace_accident_text,
        workplace_label, AccidentLedger, AccidentRiskOverride, WorkplaceAccidentEvent,
        SAFETY_PROTOCOLS_RISK_MULTIPLIER,
    };
    use crate::layer1::gastronomy::WorkSpeedBuff;
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::structure::Structure;
    use bevy_ecs::prelude::*;

    fn run_process(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_workplace_hazards_system);
        schedule.run(world);
    }

    fn spawn_building(world: &mut World, risk: Option<f32>) -> Entity {
        let mut e = world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
            Structure {
                current_hp: 100.0,
                max_hp: 100.0,
            },
        ));
        if let Some(r) = risk {
            e.insert(AccidentRiskOverride { risk: r });
        }
        e.id()
    }

    fn accident_events(world: &World) -> usize {
        let events = world.resource::<Events<WorkplaceAccidentEvent>>();
        let mut reader = events.get_cursor();
        reader.read(events).count()
    }

    #[test]
    fn test_risk_zero_pristine_idle() {
        assert_eq!(building_accident_risk(1.0, 0, 1.0), 0.0);
        // Damaged but idle: nobody to injure.
        assert_eq!(building_accident_risk(0.0, 0, 1.0), 0.0);
    }

    #[test]
    fn test_risk_rises_with_damage_and_workload() {
        let pristine = building_accident_risk(1.0, 1, 1.0);
        let worn = building_accident_risk(0.5, 1, 1.0);
        let ruined = building_accident_risk(0.0, 1, 1.0);
        assert!(pristine > 0.0, "staffed pristine building has base risk");
        assert!(worn > pristine, "damage raises risk: {worn} vs {pristine}");
        assert!(ruined > worn, "more damage raises risk: {ruined} vs {worn}");
        let crowded = building_accident_risk(0.5, 4, 1.0);
        assert!(crowded > worn, "more workers raise risk: {crowded} vs {worn}");
    }

    #[test]
    fn test_safety_protocols_halve_risk_and_slow_work() {
        let mut policies = ColonyPolicies::default();
        assert_eq!(safety_risk_multiplier(&policies), 1.0);
        assert_eq!(get_hazard_modifier(&policies), 1.0);
        assert!((get_work_speed_modifier(&policies) - 1.0).abs() < f32::EPSILON);

        policies.toggle(Policy::SafetyProtocols);
        assert_eq!(
            safety_risk_multiplier(&policies),
            SAFETY_PROTOCOLS_RISK_MULTIPLIER
        );
        assert_eq!(SAFETY_PROTOCOLS_RISK_MULTIPLIER, 0.5);
        assert_eq!(get_hazard_modifier(&policies), 0.5);
        let risk_on = building_accident_risk(0.0, 2, safety_risk_multiplier(&policies));
        let risk_off = building_accident_risk(0.0, 2, 1.0);
        assert!((risk_on - risk_off * 0.5).abs() < f32::EPSILON);
        let speed = get_work_speed_modifier(&policies);
        assert!(
            (speed - 0.90).abs() < f32::EPSILON,
            "Safety Protocols must slow work by 0.10, got {speed}"
        );
    }

    #[test]
    fn test_trigger_extremes_are_deterministic() {
        assert!(should_trigger_accident(1.0, 0.999));
        assert!(should_trigger_accident(1.0, 0.0));
        assert!(!should_trigger_accident(0.0, 0.0));
        assert!(!should_trigger_accident(0.0, 0.5));
    }

    #[test]
    fn test_high_risk_triggers_accident() {
        // Adapted spec RED test: a workplace at 100% risk fires.
        let mut world = World::new();
        world.init_resource::<Events<WorkplaceAccidentEvent>>();
        world.insert_resource(ColonyPolicies::default());
        let building = spawn_building(&mut world, Some(1.0));
        let worker = world
            .spawn((Pop, GridPosition { x: 5, y: 5 }, Health::default()))
            .id();
        let _ = building;
        run_process(&mut world);
        let events = world.resource::<Events<WorkplaceAccidentEvent>>();
        let mut reader = events.get_cursor();
        let fired: Vec<_> = reader.read(events).collect();
        assert_eq!(fired.len(), 1, "Accident should have triggered");
        assert_eq!(fired[0].worker, worker);
    }

    #[test]
    fn test_zero_risk_no_accident() {
        // Adapted spec RED test: a workplace at 0% risk never fires.
        let mut world = World::new();
        world.init_resource::<Events<WorkplaceAccidentEvent>>();
        world.insert_resource(ColonyPolicies::default());
        spawn_building(&mut world, Some(0.0));
        world.spawn((Pop, GridPosition { x: 5, y: 5 }, Health::default()));
        for _ in 0..5 {
            run_process(&mut world);
        }
        assert_eq!(accident_events(&world), 0, "Accident should not trigger");
    }

    #[test]
    fn test_cooldown_prevents_multi_trigger() {
        let mut world = World::new();
        world.init_resource::<Events<WorkplaceAccidentEvent>>();
        world.insert_resource(ColonyPolicies::default());
        spawn_building(&mut world, Some(1.0));
        world.spawn((Pop, GridPosition { x: 5, y: 5 }, Health::default()));
        for _ in 0..3 {
            run_process(&mut world);
        }
        assert_eq!(
            accident_events(&world),
            1,
            "Cooldown must stop a building multi-triggering"
        );
    }

    #[test]
    fn test_assigned_worker_counts_from_elsewhere() {
        // AssignedTo seam: a pop assigned to the building counts even
        // when standing far from its tile.
        let mut world = World::new();
        world.init_resource::<Events<WorkplaceAccidentEvent>>();
        world.insert_resource(ColonyPolicies::default());
        let building = spawn_building(&mut world, Some(1.0));
        world.spawn((
            Pop,
            GridPosition { x: 50, y: 50 },
            Health::default(),
            crate::layer1::actions::AssignedTo {
                entity: building,
                assignment_type: crate::layer1::utility_types::AssignmentType::FarmWorker,
            },
        ));
        run_process(&mut world);
        assert_eq!(accident_events(&world), 1);
    }

    fn run_injury(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(workplace_accident_injury_system);
        schedule.run(world);
    }

    fn setup_injury_world() -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<Events<WorkplaceAccidentEvent>>();
        world.init_resource::<Events<AmputationEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(AccidentLedger::default());
        let worker = world.spawn((Pop, Health::default())).id();
        (world, worker)
    }

    #[test]
    fn test_injury_applies_minor_damage_debuff_ledger_chronicle() {
        let (mut world, worker) = setup_injury_world();
        let workplace = world.spawn_empty().id();
        world.send_event(WorkplaceAccidentEvent {
            worker,
            workplace,
            severity_roll: 0.5, // Minor band
        });
        run_injury(&mut world);
        assert_eq!(world.get::<Health>(worker).unwrap().current, 90.0);
        let buff = world.get::<WorkSpeedBuff>(worker).unwrap();
        assert!((buff.multiplier - 0.75).abs() < f32::EPSILON);
        let ledger = world.resource::<AccidentLedger>();
        assert_eq!((ledger.total, ledger.minor), (1, 1));
        let chron = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chron.get_cursor();
        let entries: Vec<_> = reader.read(chron).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("Workplace accident"));
    }

    #[test]
    fn test_injury_critical_fires_amputation() {
        let (mut world, worker) = setup_injury_world();
        let workplace = world.spawn_empty().id();
        world.send_event(WorkplaceAccidentEvent {
            worker,
            workplace,
            severity_roll: 0.99, // Critical band
        });
        run_injury(&mut world);
        assert_eq!(world.get::<Health>(worker).unwrap().current, 20.0);
        let ledger = world.resource::<AccidentLedger>();
        assert_eq!((ledger.total, ledger.critical), (1, 1));
        let amps = world.resource::<Events<AmputationEvent>>();
        let mut reader = amps.get_cursor();
        assert_eq!(reader.read(amps).count(), 1);
    }

    #[test]
    fn test_workplace_label_variants() {
        // Item 162: pure label rules — type label alone when unique,
        // coordinates only as a disambiguator, generic when unknown.
        assert_eq!(
            workplace_label(Some((BuildingType::Smelter, GridPosition { x: 7, y: 9 })), 1),
            "the Smelter"
        );
        assert_eq!(
            workplace_label(Some((BuildingType::Smelter, GridPosition { x: 7, y: 9 })), 2),
            "the Smelter at (7, 9)"
        );
        assert_eq!(workplace_label(None, 0), "a workplace");
        assert_eq!(
            workplace_accident_text("the Smelter", AccidentSeverity::Major, 40.0),
            "Workplace accident at the Smelter: a worker was injured (Major, -40 HP)."
        );
    }

    fn chronicle_texts(world: &World) -> Vec<String> {
        let chron = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = chron.get_cursor();
        reader.read(chron).map(|e| e.text.clone()).collect()
    }

    #[test]
    fn test_chronicle_names_building_type_not_entity() {
        // Item 162 regression (Mark's playtest report): the chronicle
        // read `building 34v1#4294967330`. It must name the building.
        let (mut world, worker) = setup_injury_world();
        let workplace = world
            .spawn((
                Building {
                    building_type: BuildingType::Smelter,
                },
                GridPosition { x: 7, y: 9 },
            ))
            .id();
        world.send_event(WorkplaceAccidentEvent {
            worker,
            workplace,
            severity_roll: 0.85, // Major band, -40 HP (Mark's example)
        });
        run_injury(&mut world);
        let texts = chronicle_texts(&world);
        assert_eq!(texts.len(), 1);
        let text = &texts[0];
        assert_eq!(
            text,
            "Workplace accident at the Smelter: a worker was injured (Major, -40 HP)."
        );
        assert!(text.contains("Smelter"), "chronicle must name the type: {text}");
        assert!(!text.contains('#'), "chronicle leaked Entity Debug: {text}");
        assert!(
            !text.contains(&format!("{workplace:?}")),
            "chronicle leaked Entity Debug: {text}"
        );
    }

    #[test]
    fn test_chronicle_disambiguates_duplicate_building_types() {
        // Two Smelters: the accident site gets its coordinates.
        let (mut world, worker) = setup_injury_world();
        let workplace = world
            .spawn((
                Building {
                    building_type: BuildingType::Smelter,
                },
                GridPosition { x: 7, y: 9 },
            ))
            .id();
        world.spawn((
            Building {
                building_type: BuildingType::Smelter,
            },
            GridPosition { x: 12, y: 3 },
        ));
        world.send_event(WorkplaceAccidentEvent {
            worker,
            workplace,
            severity_roll: 0.85,
        });
        run_injury(&mut world);
        let texts = chronicle_texts(&world);
        assert_eq!(texts.len(), 1);
        assert!(
            texts[0].contains("the Smelter at (7, 9)"),
            "duplicate types need coordinates: {}",
            texts[0]
        );
        assert!(!texts[0].contains('#'), "chronicle leaked Entity Debug: {}", texts[0]);
    }

    #[test]
    fn test_chronicle_unknown_workplace_stays_generic() {
        // A bare workplace entity (the pre-fix test shape) must still
        // chronicle — generically, with no entity dump.
        let (mut world, worker) = setup_injury_world();
        let workplace = world.spawn_empty().id();
        world.send_event(WorkplaceAccidentEvent {
            worker,
            workplace,
            severity_roll: 0.5,
        });
        run_injury(&mut world);
        let texts = chronicle_texts(&world);
        assert_eq!(texts.len(), 1);
        assert!(
            texts[0].contains("Workplace accident at a workplace:"),
            "unknown workplace should be generic: {}",
            texts[0]
        );
        assert!(!texts[0].contains('#'), "chronicle leaked Entity Debug: {}", texts[0]);
    }

    #[test]
    fn test_ip_guard_original_names_only() {
        // Public-facing strings of this spec must be original: no
        // named characters/places or distinctive IP from any source.
        let mut policies = ColonyPolicies::default();
        policies.toggle(Policy::SafetyProtocols);
        let labels = format!(
            "{:?} {:.2}",
            Policy::SafetyProtocols,
            safety_risk_multiplier(&policies)
        )
        .to_lowercase();
        for banned in [
            "sad king billy",
            "windsor",
            "hitchhiker",
            "foundation",
            "area 51",
            "oscorp",
        ] {
            assert!(!labels.contains(banned), "IP leak: {banned}");
        }
    }
}
