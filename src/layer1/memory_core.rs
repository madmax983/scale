use crate::layer1::health::Dead;
use crate::layer1::inventory::{Inventory, InventoryItem};
use crate::layer1::items::ItemType;
use crate::layer1::economy::curio_trade::Curio;
use crate::layer1::pop::Pop;
use crate::layer1::skills::Skills;
use crate::layer1::stress::StressTracker;
use crate::layer1::traits::{Trait, Traits};
use bevy_ecs::prelude::*;

#[derive(Component, Clone)]
pub struct MemoryCoreData {
    pub skills: Skills,
    pub traits: Vec<Trait>,
}

#[derive(Event)]
pub struct ImplantMemoryCoreEvent {
    pub target: Entity,
    pub core_data: MemoryCoreData,
}

#[derive(Event)]
pub struct HarvestMemoryCoreEvent {
    pub target: Entity,
    pub harvester: Entity,
}

#[derive(Component)]
pub struct MemoryHarvester {
    pub processing: Option<Entity>,
}

pub fn implant_memory_core_system(
    mut events: EventReader<ImplantMemoryCoreEvent>,
    mut pops: Query<(&mut Skills, &mut Traits, &mut StressTracker), With<Pop>>,
) {
    for event in events.read() {
        if let Ok((mut skills, mut traits, mut stress)) = pops.get_mut(event.target) {
            // Overwrite skills
            *skills = event.core_data.skills.clone();

            // Overwrite traits
            let mut new_traits = Traits::default();
            for t in event.core_data.traits.clone() {
                new_traits.add(t);
            }
            *traits = new_traits;

            // Apply massive "Identity Rejection" stress
            stress.accumulated_stress += 80.0;
            stress.accumulated_stress = stress.accumulated_stress.min(100.0);
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn harvest_memory_core_system(
    mut events: EventReader<HarvestMemoryCoreEvent>,
    mut harvesters: Query<&mut Inventory>,
    targets: Query<(&Skills, &Traits), (With<Pop>, With<Dead>)>,
    mut commands: Commands,
) {
    for event in events.read() {
        if let Ok(mut inv) = harvesters.get_mut(event.harvester) {
            if let Ok((skills, traits)) = targets.get(event.target) {
                // Spawn a new entity to hold the MemoryCoreData
                // Spec 334: harvested memory cores are curios - tag them while
                // they remain local.
                let data_entity = commands
                    .spawn((
                        MemoryCoreData {
                            skills: skills.clone(),
                            traits: traits.iter().collect(),
                        },
                        Curio,
                    ))
                    .id();

                // Yield the item into inventory
                inv.add(InventoryItem {
                    item_type: ItemType::MemoryCore(data_entity),
                    entity: Some(data_entity),
                });

                // Despawn the processed body
                commands.entity(event.target).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Dead;
    use crate::layer1::inventory::Inventory;
    use crate::layer1::items::ItemType;
    use crate::layer1::memory::Memories;
    use crate::layer1::pop::Pop;
    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::stress::StressTracker;
    use crate::layer1::traits::{Trait, Traits};

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(Events::<ImplantMemoryCoreEvent>::default());
        world.insert_resource(Events::<HarvestMemoryCoreEvent>::default());
        world
    }

    #[test]
    fn test_implanting_memory_core_grants_skills_but_causes_stress_and_personality_overwrite() {
        let mut world = setup_world();

        // Create a rookie pop
        let pop_id = world
            .spawn((
                Pop,
                Skills::default(), // Starts with 0 skills
                StressTracker {
                    accumulated_stress: 0.0,
                },
                Traits::default(),
                Memories::default(),
                Inventory::default(),
            ))
            .id();

        // Create a memory core resource item with some stored data
        let mut core_data = MemoryCoreData {
            skills: Skills::default(),
            traits: vec![Trait::Volatile], // Example trait from the dead pop
        };
        core_data.skills.add_xp(SkillType::Mining, 10000.0); // Max skill

        world.send_event(ImplantMemoryCoreEvent {
            target: pop_id,
            core_data,
        });

        // Run systems manually
        let mut schedule = Schedule::default();
        schedule.add_systems(implant_memory_core_system);
        schedule.run(&mut world);

        // Check effects on Pop
        let skills = world.get::<Skills>(pop_id).unwrap();
        assert_eq!(skills.get_level(SkillType::Mining), 10); // Maxed out

        let traits = world.get::<Traits>(pop_id).unwrap();
        assert!(traits.has(Trait::Volatile));

        let stress = world.get::<StressTracker>(pop_id).unwrap();
        assert!(stress.accumulated_stress >= 80.0); // Severe "Identity Rejection" stress
    }

    #[test]
    fn test_harvesting_memory_core_from_dead_pop_with_high_skills() {
        let mut world = setup_world();

        // Create a dead pop with high skills
        let mut dead_skills = Skills::default();
        dead_skills.add_xp(SkillType::Crafting, 10000.0);

        let dead_pop_id = world
            .spawn((
                Pop,
                Dead,
                dead_skills,
                {
                    let mut t = Traits::default();
                    t.add(Trait::Volatile);
                    t
                }, // Use a real trait
            ))
            .id();

        let harvester_id = world
            .spawn((
                MemoryHarvester {
                    processing: Some(dead_pop_id),
                },
                Inventory::default(),
            ))
            .id();

        world.send_event(HarvestMemoryCoreEvent {
            target: dead_pop_id,
            harvester: harvester_id,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(harvest_memory_core_system);
        schedule.run(&mut world);

        // Check if the harvester's inventory now contains a Memory Core
        let harvester_inv = world.get::<Inventory>(harvester_id).unwrap();
        let has_core = harvester_inv
            .items
            .iter()
            .any(|i| matches!(i.item_type, ItemType::MemoryCore(_)));
        assert!(has_core);

        // Verify the extracted data
        let core_item = harvester_inv
            .items
            .iter()
            .find(|i| matches!(i.item_type, ItemType::MemoryCore(_)))
            .unwrap();
        if let ItemType::MemoryCore(data_entity) = core_item.item_type {
            let core_data = world.get::<MemoryCoreData>(data_entity).unwrap();
            assert_eq!(core_data.skills.get_level(SkillType::Crafting), 10);
            assert!(core_data.traits.contains(&Trait::Volatile));
        } else {
            panic!("Expected MemoryCore ItemType");
        }

        // Ensure the dead pop entity is despawned/processed
        assert!(world.get_entity(dead_pop_id).is_err());
    }
}
