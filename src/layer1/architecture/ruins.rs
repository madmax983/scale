use crate::layer1::building::{BuildingType, MaterialType, OccupiedTiles};
use crate::layer1::resources::{ResourceItem, ResourceType};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;

/// Component representing a destroyed building.
#[derive(Component, Debug, Clone, Copy)]
pub struct Ruin {
    /// The original type of the building.
    pub original_type: BuildingType,
    /// The material of the original building.
    pub material: MaterialType,
}

/// Component storing historical data about a ruin.
#[derive(Component, Debug, Clone)]
pub struct RuinHistory {
    /// The tick when the building was destroyed.
    pub destruction_tick: u64,
    /// The reason for destruction (e.g., "Fire", "Decay").
    pub reason: String,
}

/// Processes the scavenging of a ruin.
///
/// This calculates the resources to return (approx 20% of construction cost),
/// spawns them on the ground as `ResourceItem`s, clears the `OccupiedTiles` entry,
/// and despawns the ruin entity.
pub fn process_scavenge(world: &mut World, ruin_entity: Entity) -> Vec<ResourceType> {
    let mut loot = Vec::new();
    let mut pos = None;

    if let Some(ruin) = world.get::<Ruin>(ruin_entity) {
        let cost = ruin.original_type.cost(ruin.material);

        // 20% yield logic
        let yield_percent = 0.2;

        if cost.wood > 0.0 {
            loot.push((ResourceType::Wood, cost.wood * yield_percent));
        }
        if cost.stone > 0.0 {
            loot.push((ResourceType::Stone, cost.stone * yield_percent));
        }
        if cost.metal > 0.0 {
            loot.push((ResourceType::Metal, cost.metal * yield_percent));
        }
        // Simplified for MVP: focusing on main construction materials
    }

    if let Some(p) = world.get::<GridPosition>(ruin_entity) {
        pos = Some(*p);
    }

    if let Some(p) = pos {
        for (res_type, amount) in &loot {
            if *amount >= 1.0 {
                // Minimum yield of 1.0 to avoid clutter? Or just spawn fractional.
                // ResourceItem supports f32, so let's allow it.
                world.spawn((
                    ResourceItem {
                        resource_type: *res_type,
                        amount: *amount,
                    },
                    p,
                ));
            }
        }

        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&(p.x, p.y));
        }
    }

    world.despawn(ruin_entity);
    loot.into_iter().map(|(t, _)| t).collect()
}

/// Chronicle promotion for structure destruction (Item 77 Phase 2).
///
/// Every path that reduces a building to a ruin (fire damage, sabotage,
/// deorbit impact, decay) ends by spawning a `Ruin`, so watching for
/// `Added<Ruin>` is the one central place where destruction becomes
/// history. Before this, fire-driven destruction only reached the
/// ephemeral message log while a colony's LifeSupport silently became
/// a ruin. Losing a LifeSupport is Major; anything else is Standard.
pub fn ruin_chronicle_system(
    new_ruins: Query<(&Ruin, Option<&RuinHistory>, &GridPosition), Added<Ruin>>,
    mut chronicle: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    use crate::layer1::chronicle::EventImportance;

    for (ruin, history, pos) in &new_ruins {
        let reason = history.map_or("destroyed", |h| h.reason.as_str());
        let importance = if ruin.original_type == BuildingType::LifeSupport {
            EventImportance::Major
        } else {
            EventImportance::Standard
        };
        chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
            text: format!(
                "The {:?} at ({}, {}) has been destroyed ({reason}). Only a ruin remains.",
                ruin.original_type, pos.x, pos.y
            ),
            importance,
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType, Material, MaterialType, OccupiedTiles};
    use crate::layer1::fire::{Fire, Flammable};
    use crate::layer1::ruins::Ruin;
    use crate::layer1::structure::{fire_damage_structure_system, Structure};
    use crate::layer1::GridPosition;
    use crate::shared::time::SimulationTime;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_destruction_spawns_ruin() {
        let mut world = World::new();
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(crate::shared::log::MessageLog::default());

        // Spawn a building with 1 HP
        let pos = GridPosition { x: 5, y: 5 };
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Material(MaterialType::Wood),
                Structure {
                    current_hp: 1.0,
                    max_hp: 100.0,
                },
                Flammable::default(),
                pos,
            ))
            .id();

        // Register occupation
        world.resource_mut::<OccupiedTiles>().0.insert((5, 5));

        // Spawn fire to destroy it
        world.spawn((
            Fire {
                intensity: 10.0,
                lifetime: 10,
            },
            pos,
        ));

        // Run damage system
        fire_damage_structure_system(&mut world);

        // Assert Building is gone
        assert!(
            world.get_entity(building).is_err(),
            "Building should be despawned"
        );

        // Assert Ruin exists at same pos
        let mut ruin_query = world.query::<(&Ruin, &GridPosition)>();
        let result = ruin_query.get_single(&world);

        assert!(result.is_ok(), "Ruin should be spawned");

        let (ruin, ruin_pos) = result.unwrap();

        assert_eq!(ruin.original_type, BuildingType::Housing);
        assert_eq!(ruin.material, MaterialType::Wood);
        assert_eq!(*ruin_pos, pos);

        // Assert Tile is still Occupied (Ruins block construction)
        assert!(world.resource::<OccupiedTiles>().0.contains(&(5, 5)));
    }

    #[test]
    fn test_scavenge_ruin_yields_resources() {
        let mut world = World::new();
        world.insert_resource(OccupiedTiles::default());
        let _pos = GridPosition { x: 2, y: 2 };

        // Spawn a Ruin
        let ruin = world
            .spawn((
                Ruin {
                    original_type: BuildingType::Wall,
                    material: MaterialType::Stone,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();

        world.resource_mut::<OccupiedTiles>().0.insert((2, 2));

        // Mock Scavenge execution
        let yielded = crate::layer1::ruins::process_scavenge(&mut world, ruin);

        // Wall (Stone) cost: 5 Stone. Yield 20% = 1.0 Stone.
        assert!(!yielded.is_empty(), "Scavenging should yield resources");
        assert!(yielded.contains(&crate::layer1::resources::ResourceType::Stone));

        // Ruin should be despawned
        assert!(world.get_entity(ruin).is_err());

        // OccupiedTiles should be cleared
        assert!(!world.resource::<OccupiedTiles>().0.contains(&(2, 2)));
    }

    #[test]
    fn test_ruin_chronicle_system_reports_destruction() {
        use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
        use crate::layer1::ruins::RuinHistory;
        use bevy_ecs::event::Events;
        use bevy_ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.spawn((
            Ruin {
                original_type: BuildingType::LifeSupport,
                material: MaterialType::Metal,
            },
            RuinHistory {
                destruction_tick: 42,
                reason: "Fire".to_string(),
            },
            GridPosition { x: 3, y: 4 },
        ));

        world
            .run_system_once(crate::layer1::ruins::ruin_chronicle_system)
            .unwrap();

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let sent: Vec<_> = reader.read(events).collect();
        assert_eq!(sent.len(), 1, "one chronicle event per new ruin");
        assert!(
            sent[0].text.contains("LifeSupport") && sent[0].text.contains("(3, 4)"),
            "chronicle entry names the structure and place: {}",
            sent[0].text
        );
        assert_eq!(
            sent[0].importance,
            EventImportance::Major,
            "losing a LifeSupport is Major"
        );
    }
}
