#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::*;

    use crate::layer1::core::chronicle::AddChronicleEvent;
    use crate::layer1::resources::ResourceType;
    use crate::layer2::combat::resolve_combat;
    use crate::layer2::entities::nomad_fleet::{
        handle_nomad_arrival_system, nomad_fleet_composition, nomad_public_strings,
        nomad_stats_label, process_nomad_mining_system, process_nomad_orbit_mining_system,
        spawn_nomad_fleet, trigger_nomad_arrival_system, NodeResources, NodeType, NomadArrivalEvent,
        NomadArrivalState, NomadCombatStats, NomadFleet, NomadTradeMarket, SystemNode,
        SystemPosition, NOMAD_ATTACK_POWER,
    };
    use crate::layer2::fleet::{FleetComposition, FleetFaction, InOrbit};
    use crate::layer2::mining::MiningTarget;
    use crate::layer2::ship::{Ship, ShipType};

    // Spec RED test 1, adapted: the spec ran the trigger system
    // bare and expected an event from a 1% roll; the adapted
    // deterministic state pins the chance to 1.0 (see the module
    // docs in nomad_fleet.rs).
    #[test]
    fn test_nomad_fleet_arrives() {
        let mut world = World::new();
        world.init_resource::<Events<NomadArrivalEvent>>();
        world.insert_resource(NomadArrivalState {
            chance_per_tick: 1.0,
            ..NomadArrivalState::default()
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_nomad_arrival_system);
        schedule.run(&mut world);

        let events = world.resource::<Events<NomadArrivalEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).next().is_some());
    }

    #[test]
    fn test_nomad_fleet_does_not_arrive_at_zero_chance() {
        let mut world = World::new();
        world.init_resource::<Events<NomadArrivalEvent>>();
        world.insert_resource(NomadArrivalState {
            chance_per_tick: 0.0,
            ..NomadArrivalState::default()
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_nomad_arrival_system);
        for _ in 0..10 {
            schedule.run(&mut world);
        }

        let events = world.resource::<Events<NomadArrivalEvent>>();
        let mut reader = events.get_cursor();
        assert!(reader.read(events).next().is_none());
    }

    // Spec RED test 2, near-verbatim (imports adapted to the real
    // module path — see nomad_fleet.rs module docs).
    #[test]
    fn test_nomads_strip_mine_asteroids() {
        let mut world = World::new();

        let pos = SystemPosition { x: 5, y: 5 };
        let asteroid = world
            .spawn((
                SystemNode {
                    node_type: NodeType::Asteroid,
                },
                pos,
                NodeResources {
                    metal: 1000.0,
                    volatiles: 500.0,
                },
            ))
            .id();

        let _fleet = world
            .spawn((NomadFleet { mining_rate: 100.0 }, pos))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(process_nomad_mining_system);
        schedule.run(&mut world);

        let resources = world.get::<NodeResources>(asteroid).unwrap();
        assert!(resources.metal < 1000.0);
        // Spec rates: metal at mining_rate, volatiles at half.
        assert!((resources.metal - 900.0).abs() < f32::EPSILON);
        assert!((resources.volatiles - 450.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mining_clamps_at_zero_and_ignores_non_asteroids() {
        let mut world = World::new();
        let pos = SystemPosition { x: 1, y: 1 };
        let thin = world
            .spawn((
                SystemNode {
                    node_type: NodeType::Asteroid,
                },
                pos,
                NodeResources {
                    metal: 30.0,
                    volatiles: 10.0,
                },
            ))
            .id();
        let planet = world
            .spawn((
                SystemNode {
                    node_type: NodeType::Planet,
                },
                pos,
                NodeResources {
                    metal: 1000.0,
                    volatiles: 1000.0,
                },
            ))
            .id();
        world.spawn((NomadFleet { mining_rate: 100.0 }, pos));

        let mut schedule = Schedule::default();
        schedule.add_systems(process_nomad_mining_system);
        schedule.run(&mut world);

        let thin = world.get::<NodeResources>(thin).unwrap();
        assert_eq!(thin.metal, 0.0);
        assert_eq!(thin.volatiles, 0.0);
        let planet = world.get::<NodeResources>(planet).unwrap();
        assert_eq!(planet.metal, 1000.0, "Planets are not strip-mined");
    }

    #[test]
    fn test_arrival_spawns_fleet_with_market_and_overwhelming_stats() {
        let mut world = World::new();
        world.init_resource::<Events<NomadArrivalEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(NomadArrivalState::default());
        world
            .resource_mut::<Events<NomadArrivalEvent>>()
            .send(NomadArrivalEvent {
                entry_point: SystemPosition { x: -12, y: 3 },
            });

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_nomad_arrival_system);
        schedule.run(&mut world);

        let mut query = world.query::<(
            &NomadFleet,
            &SystemPosition,
            &NomadCombatStats,
            &NomadTradeMarket,
        )>();
        let fleets: Vec<_> = query.iter(&world).collect();
        assert_eq!(fleets.len(), 1);
        let (_, pos, combat, market) = fleets[0];
        assert_eq!(*pos, SystemPosition { x: -12, y: 3 });
        assert_eq!(combat.attack_power, NOMAD_ATTACK_POWER);
        assert!(market.open, "Arrival opens the temporary market");
        assert!(
            market
                .goods
                .iter()
                .any(|g| g.resource == ResourceType::AncientTech),
            "Market carries high-tier goods"
        );
        assert_eq!(world.resource::<NomadArrivalState>().fleets_spawned, 1);
        assert_eq!(world.resource::<NomadArrivalState>().markets_opened, 1);
    }

    #[test]
    fn test_nomad_moves_toward_asteroid_belt_and_holds_to_mine() {
        let mut world = World::new();
        let asteroid_pos = SystemPosition { x: 5, y: 5 };
        world.spawn((
            SystemNode {
                node_type: NodeType::Asteroid,
            },
            asteroid_pos,
            NodeResources {
                metal: 100.0,
                volatiles: 50.0,
            },
        ));
        let fleet = world
            .spawn((
                NomadFleet { mining_rate: 10.0 },
                SystemPosition { x: 0, y: 0 },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(crate::layer2::entities::nomad_fleet::nomad_movement_system);
        schedule.run(&mut world);
        assert_eq!(
            *world.get::<SystemPosition>(fleet).unwrap(),
            SystemPosition { x: 1, y: 1 },
            "Fleet steps toward the belt, not a random walk"
        );

        // Teleport onto the asteroid: movement holds (mining posture).
        world
            .entity_mut(fleet)
            .insert(asteroid_pos);
        schedule.run(&mut world);
        assert_eq!(*world.get::<SystemPosition>(fleet).unwrap(), asteroid_pos);
    }

    #[test]
    fn test_orbit_mining_depletes_real_mining_target() {
        let mut world = World::new();
        let asteroid = world
            .spawn(MiningTarget {
                resource_type: ResourceType::Ore,
                amount: 100.0,
                mining_difficulty: 1.0,
            })
            .id();
        world.spawn((
            NomadFleet { mining_rate: 25.0 },
            InOrbit { parent: asteroid },
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(process_nomad_orbit_mining_system);
        schedule.run(&mut world);

        let target = world.get::<MiningTarget>(asteroid).unwrap();
        assert!((target.amount - 75.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_attacking_nomads_is_overwhelming() {
        // Two player frigates pick a fight with the nomad escort
        // through the real combat resolver.
        let mut player = FleetComposition::default();
        player.add_ship(Ship::new(ShipType::Frigate));
        player.add_ship(Ship::new(ShipType::Frigate));

        let result = resolve_combat(
            FleetFaction::Player,
            &player,
            FleetFaction::Merchant,
            &nomad_fleet_composition(),
        );
        assert_eq!(result.winner, FleetFaction::Merchant);
        assert!(
            result.loser_survivors.ships.is_empty(),
            "The attackers are wiped out"
        );
        assert_eq!(
            result.winner_survivors.ships.len(),
            nomad_fleet_composition().ships.len(),
            "The nomads shrug it off"
        );
    }

    #[test]
    fn test_stats_label() {
        let mut world = World::new();
        assert_eq!(nomad_stats_label(&mut world), "none");
        spawn_nomad_fleet(&mut world, SystemPosition { x: 0, y: 0 });
        assert_eq!(nomad_stats_label(&mut world), "fleets1/mined0");
    }

    #[test]
    fn test_ip_guard_no_borrowed_terms() {
        let banned = [
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "sandworm",
            "bene gesserit",
            " hitchhiker",
            "vogon",
        ];
        for text in nomad_public_strings() {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(
                    !lower.contains(term),
                    "public string contains banned term {term:?}: {text}"
                );
            }
        }
    }
}
