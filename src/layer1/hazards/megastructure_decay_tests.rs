//! Spec 372 RED tests, adapted to the real codebase — see the
//! adaptation notes in `megastructure_decay.rs` (TileType -> the
//! TerrainGrid resource with Rock as the rubble equivalent per the
//! spec's own Technical Guidance; Health is the real
//! `layer1::health::Health`; the random trigger is deterministic,
//! so "eventually triggers" is asserted over a bounded run).

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::*;

    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::hazards::megastructure_decay::{
        decay_roll, event_chance, process_debris_fall_system, trigger_decay_event_system,
        DecayEvent, MegastructureRuins, RuinsDecayLedger, DECAY_EVENT_DAMAGE,
        RUBBLE_DAMAGE_THRESHOLD,
    };
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::terrain::{TerrainGrid, TerrainType};

    fn base_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<DecayEvent>>();
        world.init_resource::<Events<crate::layer1::core::chronicle::AddChronicleEvent>>();
        world.insert_resource(RuinsDecayLedger::default());
        world
    }

    fn drain_decay_events(world: &mut World) -> Vec<DecayEvent> {
        let events: Vec<DecayEvent> = world
            .resource::<Events<DecayEvent>>()
            .iter_current_update_events()
            .copied()
            .collect();
        world.resource_mut::<Events<DecayEvent>>().clear();
        events
    }

    /// Spec RED test 1, adapted: the sketch rolled `thread_rng`
    /// once (a 9% coin flip at stability 0.1); its own comment says
    /// an event should *eventually* trigger. Deterministic version:
    /// at stability 0.1 at least one event fires within 200 ticks,
    /// every fired event carries the spec damage, and at stability
    /// 1.0 (sound ruins) no event ever fires.
    #[test]
    fn test_decay_event_triggers_randomly() {
        let mut world = base_world();
        world.insert_resource(MegastructureRuins { stability: 0.1 });

        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_decay_event_system);
        let mut fired = Vec::new();
        for _ in 0..200 {
            schedule.run(&mut world);
            fired.extend(drain_decay_events(&mut world));
        }
        assert!(
            !fired.is_empty(),
            "low-stability ruins should eventually drop debris"
        );
        assert!(fired
            .iter()
            .all(|e| (e.damage - DECAY_EVENT_DAMAGE).abs() < f32::EPSILON));
        assert!(world.resource::<RuinsDecayLedger>().events_fired >= 1);

        // Sound ruins are silent forever (chance is exactly zero).
        let mut world = base_world();
        world.insert_resource(MegastructureRuins { stability: 1.0 });
        let mut schedule = Schedule::default();
        schedule.add_systems(trigger_decay_event_system);
        for _ in 0..200 {
            schedule.run(&mut world);
        }
        assert!(drain_decay_events(&mut world).is_empty());
        assert_eq!(world.resource::<RuinsDecayLedger>().events_fired, 0);
    }

    /// The spec's probability law, verbatim: (1 - stability) * 0.1.
    #[test]
    fn test_event_chance_law() {
        assert_eq!(event_chance(1.0), 0.0);
        assert!((event_chance(0.1) - 0.09).abs() < 1e-6);
        assert!((event_chance(0.0) - 0.1).abs() < 1e-6);
        // Rolls are deterministic and always inside [0, 1).
        for i in 0..512 {
            let roll = decay_roll(i);
            assert!((0.0..1.0).contains(&roll), "roll {i} = {roll}");
            assert_eq!(roll, decay_roll(i));
        }
    }

    /// Spec RED test 2, near-verbatim: a decay event damages the
    /// building standing on its target tile (the real Health has
    /// one extra field, `has_rust_lung`).
    #[test]
    fn test_debris_damages_buildings() {
        let mut world = base_world();

        let pos = GridPosition { x: 5, y: 5 };
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                pos,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        // A bystander one tile away is untouched.
        let bystander = world
            .spawn((
                GridPosition { x: 6, y: 5 },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        world
            .resource_mut::<Events<DecayEvent>>()
            .send(DecayEvent {
                target: pos,
                damage: 50.0,
            });

        let mut schedule = Schedule::default();
        schedule.add_systems(process_debris_fall_system);
        schedule.run(&mut world);

        let health = world.get::<Health>(building).unwrap();
        assert_eq!(health.current, 50.0);
        assert_eq!(world.get::<Health>(bystander).unwrap().current, 100.0);
        assert_eq!(world.resource::<RuinsDecayLedger>().entities_damaged, 1);
    }

    /// Spec RED test 3, adapted: the sketch spawned a
    /// `(GridPosition, TileType)` tile entity; no TileType exists —
    /// terrain is the TerrainGrid resource, and the rubble
    /// equivalent is TerrainType::Rock (spec Technical Guidance +
    /// structural-integrity precedent; see module docs). Massive
    /// damage buries the tile; ordinary damage does not.
    #[test]
    fn test_debris_alters_terrain() {
        let mut world = base_world();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        let pos = GridPosition { x: 2, y: 2 };
        world
            .resource_mut::<Events<DecayEvent>>()
            .send(DecayEvent {
                target: pos,
                damage: 100.0,
            });

        let mut schedule = Schedule::default();
        schedule.add_systems(process_debris_fall_system);
        schedule.run(&mut world);

        let grid = world.resource::<TerrainGrid>();
        assert_eq!(grid.get(2, 2), Some(TerrainType::Rock));
        assert_eq!(world.resource::<RuinsDecayLedger>().tiles_rubbled, 1);
        assert_eq!(RUBBLE_DAMAGE_THRESHOLD, 100.0);

        // Ordinary debris (50 damage) dents but does not bury.
        let mut world = base_world();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });
        world
            .resource_mut::<Events<DecayEvent>>()
            .send(DecayEvent {
                target: pos,
                damage: 50.0,
            });
        let mut schedule = Schedule::default();
        schedule.add_systems(process_debris_fall_system);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<TerrainGrid>().get(2, 2),
            Some(TerrainType::Grass)
        );
    }

    /// IP guard: every public-facing string uses original names only.
    #[test]
    fn test_ip_guard_original_names_only() {
        let banned = [
            "galactus",
            "marvel",
            "mass effect",
            "reaper",
            "rimworld",
            "dwarf fortress",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "pokemon",
            "zelda",
            "skyrim",
            "halo",
            "forerunner",
            "promethean",
            "death star",
            "ringworld",
            "dyson sphere",
        ];
        for text in crate::layer1::hazards::megastructure_decay::ruins_public_strings() {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "IP-guard: '{text}' contains banned term '{word}'"
                );
            }
        }
    }
}
