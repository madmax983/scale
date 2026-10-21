use crate::layer1::architecture::building::BuildingType;
use crate::layer1::crafting::byproducts::{
    bin_chokes, capture_byproducts, CraftingBuilding, Inventory as ByproductInventory,
};
use crate::layer1::inventory::Inventory;
use crate::layer1::items::ItemType;
use crate::layer1::refining::get_refining_recipe;
use crate::layer1::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Component for the Recycler building.
#[derive(Component, Default, Debug)]
pub struct Recycler {
    /// Amount of waste stored in the recycler (waiting to be processed).
    pub waste_stored: f32,
    /// Entity ID of a corpse currently being processed.
    pub corpse_to_process: Option<Entity>,
}

/// System to process organic matter and waste into rations.
///
/// Item 116 (Spec 354): a Recycler carrying a byproduct bin generates
/// Waste byproducts as it works. The roll mirrors the Recycler entry in
/// the real refining table (`get_refining_recipe`, waste_chance 0.5) —
/// the bin changes where the waste goes, not how much is rolled. A
/// choked bin (cannot fit the next byproduct) halts processing: items
/// stay in the physical inventory until the bin is dumped. Recyclers
/// without a bin (hand-spawned) keep the historical behavior plus the
/// refining system's ground-spill fallback for rolled byproducts.
#[allow(clippy::type_complexity)]
pub fn recycle_processing_system(
    mut recycler_query: Query<
        (
            &mut Inventory,
            Option<&GridPosition>,
            Option<&CraftingBuilding>,
            Option<&mut ByproductInventory>,
        ),
        With<Recycler>,
    >,
    mut resources: ResMut<ColonyResources>,
    mut commands: Commands,
) {
    let waste_chance = get_refining_recipe(BuildingType::Recycler, &resources).3;
    let mut rng = rand::thread_rng();
    for (mut inventory, pos, crafting, mut bin) in &mut recycler_query {
        let mut indices_to_remove = Vec::new();

        for (i, item) in inventory.items.iter().enumerate() {
            if !matches!(item.item_type, ItemType::Corpse(_) | ItemType::Waste) {
                continue;
            }

            // Choke: a bin that cannot fit the next byproduct blocks
            // processing until it is dumped (Spec 354).
            if let (Some(crafting), Some(bin_ref)) = (crafting, bin.as_deref()) {
                if bin_chokes(crafting, bin_ref) {
                    break;
                }
            }

            match item.item_type {
                ItemType::Corpse(corpse_entity) => {
                    // Process Corpse
                    commands.entity(corpse_entity).despawn();
                    // Add Rations (e.g. 50.0)
                    resources.add_rations(50.0);
                }
                ItemType::Waste => {
                    // Process Waste: 1 Waste Item -> 0.5 Ration
                    // (2 items = 1 ration).
                    resources.add_rations(0.5);
                }
                _ => unreachable!(),
            }
            indices_to_remove.push(i);

            // Byproduct: capture into the bin; bin-less recyclers spill
            // to the ground, mirroring the refining seam's fallback.
            if rng.gen_bool(waste_chance) {
                let captured = match (crafting, bin.as_deref_mut()) {
                    (Some(crafting), Some(bin_mut)) => capture_byproducts(crafting, bin_mut),
                    _ => false,
                };
                if !captured {
                    if let Some(pos) = pos {
                        commands.spawn((
                            ResourceItem {
                                resource_type: ResourceType::Waste,
                                amount: 1.0,
                            },
                            *pos,
                        ));
                    }
                }
            }
        }

        // Remove processed items (in reverse order to keep indices valid)
        for index in indices_to_remove.into_iter().rev() {
            inventory.items.swap_remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::funeral::Corpse;
    use crate::layer1::inventory::InventoryItem;

    #[test]
    fn test_recycler_processes_corpse() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        let corpse = world
            .spawn(Corpse {
                name: "Bob".to_string(),
                decay: 0.0,
            })
            .id();

        // Spawn Recycler with Corpse in Inventory
        world.spawn((
            Recycler::default(),
            Inventory {
                items: vec![InventoryItem {
                    item_type: ItemType::Corpse(corpse),
                    entity: None,
                }],
                capacity: 10,
            },
        ));

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(recycle_processing_system);
        schedule.run(&mut world);

        // Assert
        let resources = world.resource::<ColonyResources>();
        assert!(resources.rations >= 50.0);

        // Inventory should be empty
        let mut inv_query = world.query::<&Inventory>();
        let inv = inv_query.single(&world);
        assert!(inv.items.is_empty());

        // Corpse entity should be despawned
        assert!(world.get_entity(corpse).is_err());
    }

    #[test]
    fn test_recycler_processes_waste() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        // Spawn Recycler with Waste in Inventory
        world.spawn((
            Recycler::default(),
            Inventory {
                items: vec![
                    InventoryItem {
                        item_type: ItemType::Waste,
                        entity: None,
                    },
                    InventoryItem {
                        item_type: ItemType::Waste,
                        entity: None,
                    },
                ],
                capacity: 10,
            },
        ));

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(recycle_processing_system);
        schedule.run(&mut world);

        // Assert
        let resources = world.resource::<ColonyResources>();
        // 2 Waste * 0.5 = 1.0 Ration
        assert!((resources.rations - 1.0).abs() < f32::EPSILON);

        let mut inv_query = world.query::<&Inventory>();
        let inv = inv_query.single(&world);
        assert!(inv.items.is_empty());
    }
}
