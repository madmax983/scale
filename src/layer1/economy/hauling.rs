#![allow(clippy::collapsible_if)]
use crate::layer1::drone::Drone;
use crate::layer1::execution::{AtTarget, MovementTarget};
use crate::layer1::factions::{FactionMember, FactionState, Factions};
use crate::layer1::gene_bank::GeneBank;
use crate::layer1::inventory::{Inventory, InventoryItem};
use crate::layer1::items::{CarryingItem, Item, ItemType};
use crate::layer1::permit::PermitRequired;
use crate::layer1::recycling::Recycler;
use crate::layer1::resources::{Carrying, ColonyResources, ResourceItem};
use crate::layer1::stockpile::Stockpile;
use crate::layer1::utility_ai::{manhattan_distance, ActionType, PopAction};
use crate::layer1::zone::{ZoneGrid, ZoneType};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;

/// Moves resources from the world to stockpiles.
/// ⚡ Bolt: Avoids cloning the Factions HashMap every tick. Reduces heap allocations by 1 per frame.
#[derive(QueryData)]
pub struct HaulerQuery {
    entity: Entity,
    action: &'static PopAction,
    pos: &'static GridPosition,
    carrying: Option<&'static Carrying>,
    carrying_item: Option<&'static CarryingItem>,
    at_target: Option<&'static AtTarget>,
    member: Option<&'static FactionMember>,
}

/// Executes the hauling action for pops and drones, moving items and resources.
pub fn haul_system(world: &mut World) {
    // Collect hauling pops
    let mut query = world.query::<HaulerQuery>();
    let factions_data = world.get_resource::<Factions>().map(|f| &f.map);

    let mut haulers = Vec::with_capacity(query.iter(world).len());
    for hauler in query.iter(world) {
        if hauler.action.current != ActionType::Haul {
            continue;
        }

        if let Some(fid) = hauler.member.and_then(|m| m.faction_id) {
            if let Some(map) = factions_data {
                if map
                    .get(&fid)
                    .is_some_and(|d| d.state == FactionState::Striking)
                {
                    continue;
                }
            }
        }

        haulers.push((
            hauler.entity,
            *hauler.pos,
            hauler.carrying.copied(),
            hauler.carrying_item.copied(),
            hauler.at_target.is_some(),
        ));
    }

    // Process each hauler
    for (entity, pos, carrying, carrying_item, at_target) in haulers {
        if at_target {
            process_hauler_at_target(world, entity, pos, carrying, carrying_item);
        } else if world.get::<MovementTarget>(entity).is_none() {
            process_hauler_seeking_target(world, entity, pos, carrying, carrying_item);
        }
    }
}

fn process_hauler_at_target(
    world: &mut World,
    entity: Entity,
    pos: GridPosition,
    carrying: Option<Carrying>,
    carrying_item: Option<CarryingItem>,
) {
    if let Some(carrying_data) = carrying {
        handle_drop_off(world, entity, carrying_data, pos);
        return;
    }
    if let Some(carrying_item_data) = carrying_item {
        handle_drop_off_item(world, entity, carrying_item_data, pos);
        return;
    }
    handle_pickup(world, entity, pos);
}

fn process_hauler_seeking_target(
    world: &mut World,
    entity: Entity,
    pos: GridPosition,
    carrying: Option<Carrying>,
    carrying_item: Option<CarryingItem>,
) {
    if let Some(carrying_data) = carrying {
        find_and_target_stockpile(world, entity, pos, carrying_data);
        return;
    }
    if let Some(carrying_item_data) = carrying_item {
        find_and_target_stockpile_item(world, entity, pos, carrying_item_data);
        return;
    }
    find_and_target_item(world, entity, pos);
}

fn handle_pickup(world: &mut World, pop_entity: Entity, pos: GridPosition) {
    // 1. Try to find ResourceItem
    let item_to_pickup = {
        let mut query = world.query::<(Entity, &GridPosition, &ResourceItem)>();
        query
            .iter(world)
            .find(|(_, p, _)| **p == pos)
            .map(|(e, _, i)| (e, *i))
    };

    if let Some((item_entity, item_data)) = item_to_pickup {
        // Pickup Resource
        if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
            pop.insert(Carrying {
                resource_type: item_data.resource_type,
                amount: item_data.amount,
            });
        }
        if world.get_entity(item_entity).is_ok() {
            world.despawn(item_entity);
        }
    } else {
        // 2. Try to find Generic Item
        let generic_item_to_pickup = {
            let mut query = world.query::<(Entity, &GridPosition, &Item)>();
            query
                .iter(world)
                .find(|(_, p, _)| **p == pos)
                .map(|(e, _, _)| e)
        };

        if let Some(item_entity) = generic_item_to_pickup {
            // Pickup Item
            if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
                pop.insert(CarryingItem(item_entity));
            }
            if let Ok(mut item) = world.get_entity_mut(item_entity) {
                item.remove::<GridPosition>();
            }
        }
    }

    // Clear movement state regardless of success (if item gone, we retry next tick)
    clear_hauler_target(world, pop_entity);
}

fn handle_drop_off(world: &mut World, pop_entity: Entity, carrying: Carrying, pos: GridPosition) {
    if carrying.resource_type == crate::layer1::resources::ResourceType::BuildingPermit {
        if try_drop_off_permit(world, pop_entity, carrying, pos) {
            return;
        }
    }

    let is_stockpile = {
        let mut query = world.query::<(&GridPosition, &Stockpile)>();
        query.iter(world).any(|(p, _)| *p == pos)
    };

    if is_stockpile {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.add_resource(&carrying.resource_type, carrying.amount);
        remove_carrying(world, pop_entity);
    }

    clear_hauler_target(world, pop_entity);
}

/// Fail-safe hauler-target cleanup (Item 86): the pop snapshot in
/// `haul_system` can go stale if the pop is despawned mid-processing, so
/// never use `world.entity_mut` (which panics on a dead entity) here.
fn clear_hauler_target(world: &mut World, pop_entity: Entity) {
    if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
        pop.remove::<AtTarget>().remove::<MovementTarget>();
    }
}

/// Fail-safe removal of the resource-carrying state.
fn remove_carrying(world: &mut World, pop_entity: Entity) {
    if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
        pop.remove::<Carrying>();
    }
}

/// Fail-safe removal of the item-carrying state: used when a carried item
/// turns out to have been despawned by another system (spoilage/decay,
/// artifact sale, consumption, ...) while the hauler still referenced it.
/// The hauler simply drops the stale errand and can pick a new job.
fn clear_carried_item(world: &mut World, pop_entity: Entity) {
    clear_hauler_target(world, pop_entity);
    if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
        pop.remove::<CarryingItem>();
    }
}

fn try_drop_off_permit(
    world: &mut World,
    pop_entity: Entity,
    carrying: Carrying,
    pos: GridPosition,
) -> bool {
    let permit_target = {
        let mut query = world.query::<(Entity, &GridPosition, &PermitRequired, &mut Inventory)>();
        let mut target = None;
        for (e, p, _, _) in query.iter_mut(world) {
            if *p == pos {
                target = Some(e);
                break;
            }
        }
        target
    };

    let Some(target) = permit_target else {
        return false;
    };

    let delivered = if let Some(mut inventory) = world.get_mut::<Inventory>(target) {
        inventory.try_add(InventoryItem {
            item_type: ItemType::BuildingPermit,
            entity: None,
        })
    } else {
        false
    };

    if delivered {
        remove_carrying(world, pop_entity);
    } else {
        world.spawn((
            ResourceItem {
                resource_type: carrying.resource_type,
                amount: carrying.amount,
            },
            pos,
        ));
        remove_carrying(world, pop_entity);
    }

    clear_hauler_target(world, pop_entity);
    true
}

fn try_target_permit_building(world: &mut World, pop_entity: Entity, pos: GridPosition) -> bool {
    let mut query = world.query::<(Entity, &GridPosition, &PermitRequired, &Inventory)>();
    let mut best_permit = None;
    let mut min_dist = i32::MAX;

    for (e, p, _, inv) in query.iter(world) {
        let has_permit = inv
            .items
            .iter()
            .any(|i| i.item_type == ItemType::BuildingPermit);
        let has_space = inv.items.len() < inv.capacity;

        if !has_permit && has_space {
            let dist = manhattan_distance(&pos, p);
            if dist < min_dist {
                min_dist = dist;
                best_permit = Some((e, *p));
            }
        }
    }

    if let Some((target_entity, target_pos)) = best_permit {
        if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
            pop.insert(MovementTarget {
                target_entity,
                target_position: target_pos,
                for_action: ActionType::Haul,
            });
        }
        true
    } else {
        false
    }
}

fn find_and_target_stockpile(
    world: &mut World,
    pop_entity: Entity,
    pos: GridPosition,
    carrying: Carrying,
) {
    // Check if drone
    let is_drone = world.get::<Drone>(pop_entity).is_some();

    // Permit Logic: Prioritize PermitRequired buildings
    if carrying.resource_type == crate::layer1::resources::ResourceType::BuildingPermit {
        if try_target_permit_building(world, pop_entity, pos) {
            return;
        }
        // If we have a permit but no target building, do NOT haul to stockpile.
        // This prevents permits from disappearing into the global resource void.
        return;
    }

    if try_target_stockpile_for_resource(world, pop_entity, pos, is_drone) {
        return;
    }

    // Fallback: Find Generic Item
    find_and_target_generic_item(world, pop_entity, pos);
}

fn try_target_stockpile_for_resource(
    world: &mut World,
    pop_entity: Entity,
    pos: GridPosition,
    is_drone: bool,
) -> bool {
    let mut query = world.query::<(Entity, &GridPosition, &Stockpile)>();
    let mut best = None;
    let mut min_dist = i32::MAX;

    let is_sanctuary = |world: &World, p: &GridPosition| -> bool {
        if let Some(grid) = world.get_resource::<ZoneGrid>() {
            grid.get(p.x, p.y) == ZoneType::Sanctuary
        } else {
            false
        }
    };

    for (e, p, _) in query.iter(world) {
        if is_drone && is_sanctuary(world, p) {
            continue;
        }

        let dist = manhattan_distance(&pos, p);
        if dist < min_dist {
            min_dist = dist;
            best = Some((e, *p));
        }
    }

    set_hauler_target(world, pop_entity, best)
}

fn find_and_target_generic_item(world: &mut World, pop_entity: Entity, pos: GridPosition) {
    let is_drone = world.get::<Drone>(pop_entity).is_some();

    // Collect stockpile positions to avoid hauling items already stored
    // ⚡ Bolt Optimization: Use a HashSet for O(1) lookups inside the loop
    //    instead of a Vec to prevent O(N * M) time complexity.
    let stockpiles: bevy::utils::HashSet<GridPosition> = world
        .query::<(&GridPosition, &Stockpile)>()
        .iter(world)
        .map(|(p, _)| *p)
        .collect();

    let mut query = world.query::<(Entity, &GridPosition, &Item)>();

    let mut best = None;
    let mut min_dist = i32::MAX;

    for (e, p, _) in query.iter(world) {
        if is_drone {
            if let Some(grid) = world.get_resource::<ZoneGrid>() {
                if grid.get(p.x, p.y) == ZoneType::Sanctuary {
                    continue;
                }
            }
        }

        if stockpiles.contains(p) {
            continue;
        }

        let dist = manhattan_distance(&pos, p);
        if dist < min_dist {
            min_dist = dist;
            best = Some((e, *p));
        }
    }

    set_hauler_target(world, pop_entity, best);
}

fn try_drop_off_gene_bank(
    world: &mut World,
    pop_entity: Entity,
    item_entity: CarryingItem,
    pos: GridPosition,
) -> bool {
    let gene_bank_target = {
        let mut query = world.query::<(Entity, &GridPosition, &mut GeneBank)>();
        let mut target = None;
        for (e, p, _) in query.iter_mut(world) {
            if *p == pos {
                target = Some(e);
                break;
            }
        }
        target
    };

    let Some(gene_bank_entity) = gene_bank_target else {
        return false;
    };

    let data = {
        let mut item_entity_mut = match world.get_entity_mut(item_entity.0) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let sample = match item_entity_mut.take::<crate::layer1::gene_bank::GeneticSample>() {
            Some(s) => s,
            None => return false,
        };
        sample.data
    };
    if let Some(mut bank) = world.get_mut::<GeneBank>(gene_bank_entity) {
        bank.store_sample(data);
    }

    if world.get_entity(item_entity.0).is_ok() {
        world.despawn(item_entity.0);
    }
    clear_carried_item(world, pop_entity);

    true
}

fn try_drop_off_inventory(
    world: &mut World,
    pop_entity: Entity,
    item_entity: CarryingItem,
    pos: GridPosition,
) {
    let container = {
        let mut query = world.query::<(Entity, &GridPosition, &mut Inventory)>();
        let mut target = None;
        for (e, p, _) in query.iter(world) {
            if *p == pos {
                target = Some(e);
                break;
            }
        }
        target
    };

    let Some(container_entity) = container else {
        drop_item_on_ground(world, pop_entity, item_entity, pos);
        return;
    };

    let Some(item) = world.get::<Item>(item_entity.0) else {
        // Stale carried item (Item 86): it was despawned while carried.
        // Clear the hauler's state and move on — never panic.
        clear_carried_item(world, pop_entity);
        return;
    };

    let item_type = item.item_type;
    let is_photophobic = world
        .get_entity(item_entity.0)
        .is_ok_and(|e| e.contains::<crate::layer1::environment::photophobic::Photophobic>());

    let inv_item = InventoryItem {
        item_type,
        entity: if is_photophobic {
            Some(item_entity.0)
        } else {
            None
        },
    };

    let success = if let Some(mut inv) = world.get_mut::<Inventory>(container_entity) {
        inv.try_add(inv_item)
    } else {
        false
    };

    if success {
        if is_photophobic {
            if let Ok(mut item) = world.get_entity_mut(item_entity.0) {
                item.remove::<GridPosition>();
                item.insert(crate::layer1::environment::photophobic::Parent(
                    container_entity,
                ));
            }
        } else if world.get_entity(item_entity.0).is_ok() {
            world.despawn(item_entity.0);
        }
        clear_carried_item(world, pop_entity);
    } else {
        drop_item_on_ground(world, pop_entity, item_entity, pos);
    }
}

fn drop_item_on_ground(
    world: &mut World,
    pop_entity: Entity,
    item_entity: CarryingItem,
    pos: GridPosition,
) {
    // Item 86: the carried item may have been despawned by another system
    // (spoilage/decay, artifact sale, consumption, ...) while the hauler
    // still referenced it. `world.entity_mut` panics on a dead entity —
    // that was the stochastic long-run haul panic at this site. Fail safe:
    // re-ground the item only if it still exists, and always clear the
    // hauler's carrying/target state so it can pick a new job.
    if let Ok(mut item) = world.get_entity_mut(item_entity.0) {
        item.insert(pos);
        item.remove::<crate::layer1::environment::photophobic::Parent>();
    }

    clear_carried_item(world, pop_entity);
}

fn handle_drop_off_item(
    world: &mut World,
    pop_entity: Entity,
    item_entity: CarryingItem,
    pos: GridPosition,
) {
    if try_drop_off_gene_bank(world, pop_entity, item_entity, pos) {
        return;
    }

    try_drop_off_inventory(world, pop_entity, item_entity, pos);
}

fn find_and_target_stockpile_item(
    world: &mut World,
    pop_entity: Entity,
    pos: GridPosition,
    carrying_item: CarryingItem,
) {
    // Item 86: a carried item despawned by another system leaves a stale
    // reference — clear it instead of targeting a stockpile for a ghost
    // (which would then panic on arrival in `drop_item_on_ground`).
    if world.get_entity(carrying_item.0).is_err() {
        clear_carried_item(world, pop_entity);
        return;
    }

    let item_type = world.get::<Item>(carrying_item.0).map(|i| i.item_type);

    if matches!(item_type, Some(ItemType::Waste)) {
        if try_target_recycler(world, pop_entity, pos) {
            return;
        }
    } else if matches!(item_type, Some(ItemType::GeneticSample)) {
        if try_target_gene_bank(world, pop_entity, pos) {
            return;
        }
    }

    try_target_stockpile_for_item(world, pop_entity, pos);
}

fn try_target_recycler(world: &mut World, pop_entity: Entity, pos: GridPosition) -> bool {
    let mut query = world.query::<(Entity, &GridPosition, &Recycler, &Inventory)>();
    let mut best = None;
    let mut min_dist = i32::MAX;

    for (e, p, _, inv) in query.iter(world) {
        if inv.items.len() < inv.capacity {
            let dist = manhattan_distance(&pos, p);
            if dist < min_dist {
                min_dist = dist;
                best = Some((e, *p));
            }
        }
    }

    set_hauler_target(world, pop_entity, best)
}

fn try_target_gene_bank(world: &mut World, pop_entity: Entity, pos: GridPosition) -> bool {
    let mut query = world.query::<(Entity, &GridPosition, &GeneBank)>();
    let mut best = None;
    let mut min_dist = i32::MAX;

    for (e, p, _) in query.iter(world) {
        let dist = manhattan_distance(&pos, p);
        if dist < min_dist {
            min_dist = dist;
            best = Some((e, *p));
        }
    }

    set_hauler_target(world, pop_entity, best)
}

fn try_target_stockpile_for_item(world: &mut World, pop_entity: Entity, pos: GridPosition) -> bool {
    let is_drone = world.get::<Drone>(pop_entity).is_some();

    let mut query = world.query::<(Entity, &GridPosition, &Stockpile)>();
    let mut best = None;
    let mut min_dist = i32::MAX;

    for (e, p, _) in query.iter(world) {
        if is_drone {
            if let Some(grid) = world.get_resource::<ZoneGrid>() {
                if grid.get(p.x, p.y) == ZoneType::Sanctuary {
                    continue;
                }
            }
        }

        let dist = manhattan_distance(&pos, p);
        if dist < min_dist {
            min_dist = dist;
            best = Some((e, *p));
        }
    }

    set_hauler_target(world, pop_entity, best)
}

fn set_hauler_target(
    world: &mut World,
    pop_entity: Entity,
    target: Option<(Entity, GridPosition)>,
) -> bool {
    if let Some((target_entity, target_pos)) = target {
        if let Ok(mut pop) = world.get_entity_mut(pop_entity) {
            pop.insert(MovementTarget {
                target_entity,
                target_position: target_pos,
                for_action: ActionType::Haul,
            });
        }
        true
    } else {
        false
    }
}

fn find_and_target_item(world: &mut World, pop_entity: Entity, pos: GridPosition) {
    // Check if drone
    let is_drone = world.get::<Drone>(pop_entity).is_some();

    // 1. Create query state first
    let mut query = world.query::<(Entity, &GridPosition, &ResourceItem)>();

    // 2. Get resources
    let resources = *world.resource::<ColonyResources>(); // Clone small struct
    let zone_grid = world.get_resource::<ZoneGrid>();

    let target = {
        let mut best = None;
        let mut min_dist = i32::MAX;

        for (e, p, item) in query.iter(world) {
            // Drone Check: Drones cannot pickup items in Sanctuary
            if is_drone {
                if let Some(grid) = zone_grid {
                    if grid.get(p.x, p.y) == ZoneType::Sanctuary {
                        continue;
                    }
                }
            }

            let has_room = resources.has_room_for(&item.resource_type);

            if has_room {
                let dist = manhattan_distance(&pos, p);
                if dist < min_dist {
                    min_dist = dist;
                    best = Some((e, *p));
                }
            }
        }
        best
    };

    if let Some((target_entity, target_pos)) = target {
        set_hauler_target(world, pop_entity, Some((target_entity, target_pos)));
    }
}

#[cfg(test)]
mod tests {
    use super::haul_system;
    use crate::layer1::actions::evaluate_haul;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::items::{CarryingItem, Item, ItemType};
    use crate::layer1::resources::{Carrying, ColonyResources, ResourceItem, ResourceType};
    use crate::layer1::stockpile::Stockpile;
    use crate::layer1::utility_ai::{ActionType, PopAction, UtilityWeights};
    use crate::layer1::utility_eval_types::ScorableCandidate;
    use crate::layer1::{GridPosition, Pop};
    use crate::shared::time::SimulationTime;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_resource_item_component() {
        let item = ResourceItem {
            resource_type: ResourceType::Wood,
            amount: 1.0,
        };
        assert_eq!(item.resource_type, ResourceType::Wood);
        assert!((item.amount - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_evaluate_haul_finds_item_and_stockpile() {
        let mut world = World::new();
        let pop_pos = GridPosition { x: 0, y: 0 };
        let weights = UtilityWeights::default();
        world.insert_resource(ColonyResources::default());

        // Spawn ResourceItem
        let item_entity = world
            .spawn((
                ResourceItem {
                    resource_type: ResourceType::Stone,
                    amount: 1.0,
                },
                GridPosition { x: 5, y: 0 },
            ))
            .id();

        // Spawn Stockpile with capacity
        let _stockpile_entity = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile::default(), // Assume default has space
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        // Manual Proxy Creation for Test
        let items: Vec<ScorableCandidate> = world
            .query::<(Entity, &GridPosition, &ResourceItem)>()
            .iter(&world)
            .map(|(e, p, i)| {
                let mut c = ScorableCandidate::new(e, *p);
                c.resource_type = Some(i.resource_type);
                c
            })
            .collect();

        let stockpiles: Vec<ScorableCandidate> = world
            .query::<(Entity, &GridPosition, &Stockpile)>()
            .iter(&world)
            .map(|(e, p, _)| ScorableCandidate::new(e, *p))
            .collect();

        let resources = world.resource::<ColonyResources>();
        let result = evaluate_haul(
            pop_pos,
            &weights,
            &items,
            &[],
            &stockpiles,
            &[],
            resources,
            None,
            None,
            None,
        );

        assert!(result.is_some());
        let (utility, target) = result.unwrap();
        assert_eq!(target, item_entity); // Should target the item first
        assert!(utility > 0.0);
    }

    #[test]
    fn test_evaluate_haul_ignores_full_stockpiles() {
        let mut world = World::new();
        let pop_pos = GridPosition { x: 0, y: 0 };
        let weights = UtilityWeights::default();

        // Fill global resources to max
        let mut resources = ColonyResources::default();
        resources.stone = resources.max_stone;
        world.insert_resource(resources);

        // Spawn Item
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Stone,
                amount: 1.0,
            },
            GridPosition { x: 5, y: 0 },
        ));

        // Spawn Stockpile
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile::default(),
            GridPosition { x: 10, y: 0 },
        ));

        let items: Vec<ScorableCandidate> = world
            .query::<(Entity, &GridPosition, &ResourceItem)>()
            .iter(&world)
            .map(|(e, p, i)| {
                let mut c = ScorableCandidate::new(e, *p);
                c.resource_type = Some(i.resource_type);
                c
            })
            .collect();

        let stockpiles: Vec<ScorableCandidate> = world
            .query::<(Entity, &GridPosition, &Stockpile)>()
            .iter(&world)
            .map(|(e, p, _)| ScorableCandidate::new(e, *p))
            .collect();

        // Should return None because global storage is full
        let resources = world.resource::<ColonyResources>();
        let result = evaluate_haul(
            pop_pos,
            &weights,
            &items,
            &[],
            &stockpiles,
            &[],
            resources,
            None,
            None,
            None,
        );

        // Since we filled global resources, we expect None.
        assert!(result.is_none());
    }

    #[test]
    fn test_haul_action_lifecycle() {
        // This integration test simulates the full loop
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());

        // 1. Spawn Pop, Item, Stockpile
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
                // Need a "Carrying" component? No, starts empty.
            ))
            .id();

        let item = world
            .spawn((
                ResourceItem {
                    resource_type: ResourceType::Wood,
                    amount: 10.0,
                },
                GridPosition { x: 2, y: 0 },
            ))
            .id();

        let _stockpile = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile::default(),
                GridPosition { x: 4, y: 0 },
            ))
            .id();

        // 2. Run haul_system (pickup phase)
        // Assume pop moves to item (simulated here by teleport)
        *world.get_mut::<GridPosition>(pop).unwrap() = GridPosition { x: 2, y: 0 };
        // Manually add AtTarget to simulate arrival
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);

        // System should detect overlap, pick up item (add Carrying, despawn Item)
        haul_system(&mut world);

        assert!(
            world.get::<Carrying>(pop).is_some(),
            "Pop should be carrying after pickup"
        );
        assert!(
            world.get_entity(item).is_err(),
            "Item should be despawned after pickup"
        );
        assert!(
            world
                .get::<crate::layer1::execution::AtTarget>(pop)
                .is_none(),
            "AtTarget should be removed"
        );

        // 3. Run haul_system (drop phase)
        // Teleport to stockpile
        *world.get_mut::<GridPosition>(pop).unwrap() = GridPosition { x: 4, y: 0 };
        // Manually add AtTarget
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);

        haul_system(&mut world);

        // 4. Verify resources added (starting wood is 15.0, hauled 10.0)
        let res = world.resource::<ColonyResources>();
        assert!(
            (res.wood - 25.0).abs() < f32::EPSILON,
            "Resources should be credited"
        );
        assert!(
            world.get::<Carrying>(pop).is_none(),
            "Pop should no longer be carrying"
        );
    }

    #[test]
    fn test_evaluate_haul_generic_item() {
        let mut world = World::new();
        let pop_pos = GridPosition { x: 0, y: 0 };
        let weights = UtilityWeights::default();
        let resources = ColonyResources::default();

        // Spawn Manual
        let manual_entity = world
            .spawn((
                Item {
                    item_type: ItemType::Manual,
                },
                GridPosition { x: 5, y: 0 },
            ))
            .id();

        // Spawn Stockpile
        let stockpile_entity = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile::default(),
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        let item_entities = vec![{
            let mut c = ScorableCandidate::new(manual_entity, GridPosition { x: 5, y: 0 });
            c.item_type = Some(ItemType::Manual);
            c
        }];

        let stockpiles = vec![ScorableCandidate::new(
            stockpile_entity,
            GridPosition { x: 10, y: 0 },
        )];

        let result = evaluate_haul(
            pop_pos,
            &weights,
            &[],
            &item_entities,
            &stockpiles,
            &[],
            &resources,
            None,
            None,
            None,
        );

        assert!(result.is_some());
        let (_, target) = result.unwrap();
        assert_eq!(target, manual_entity);
    }

    #[test]
    fn test_haul_permit_to_required_building() {
        use crate::layer1::inventory::Inventory;
        use crate::layer1::permit::PermitRequired;

        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources::default());

        // Permit Item
        let _permit_item = world
            .spawn((
                ResourceItem {
                    resource_type: ResourceType::BuildingPermit,
                    amount: 1.0,
                },
                GridPosition { x: 2, y: 0 },
            ))
            .id();

        // PermitRequired Building
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Smelter,
                },
                PermitRequired,
                Inventory::default(),
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        // Stockpile (Distraction)
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile::default(),
            GridPosition { x: 5, y: 0 }, // Closer than building
        ));

        // Hauler
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 2, y: 0 },
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
            ))
            .id();

        // 1. Pickup
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);
        haul_system(&mut world);

        // Verify carrying permit
        let carrying = world.get::<Carrying>(pop).unwrap();
        assert_eq!(carrying.resource_type, ResourceType::BuildingPermit);

        // 2. Find Target (Should pick Building over Stockpile despite distance)
        haul_system(&mut world);

        let target = world
            .get::<crate::layer1::execution::MovementTarget>(pop)
            .unwrap();
        assert_eq!(
            target.target_entity, building,
            "Should target PermitRequired building"
        );

        // 3. Dropoff
        *world.get_mut::<GridPosition>(pop).unwrap() = GridPosition { x: 10, y: 0 };
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);
        haul_system(&mut world);

        // Verify dropped in inventory
        let inv = world.get::<Inventory>(building).unwrap();
        assert_eq!(inv.items.len(), 1);
        assert_eq!(inv.items[0].item_type, ItemType::BuildingPermit);

        // Verify pop empty
        assert!(world.get::<Carrying>(pop).is_none());
    }

    // --- Item 86 regression: stale carried item (despawned while carried) ---
    //
    // A carried generic item can be despawned by another system (spoilage /
    // decay, artifact sale, consumption) while the hauler's `CarryingItem`
    // still references it. Every haul path must fail safe: no panic, and the
    // hauler's carrying/target state is cleared so it can pick a new job.

    fn spawn_stale_carrier(
        world: &mut World,
        pos: GridPosition,
        at_target: bool,
    ) -> (Entity, Entity) {
        let item = world
            .spawn((
                Item {
                    item_type: ItemType::Manual,
                },
                GridPosition { x: pos.x, y: pos.y },
            ))
            .id();
        let pop = world
            .spawn((
                Pop,
                pos,
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
                CarryingItem(item),
            ))
            .id();
        if at_target {
            world
                .entity_mut(pop)
                .insert(crate::layer1::execution::AtTarget);
        }
        // The carried item is despawned by some other system while the
        // hauler still references it.
        world.despawn(item);
        (pop, item)
    }

    fn stale_carrier_world() -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(crate::layer1::factions::Factions::default());
        world.insert_resource(ColonyResources::default());
        world
    }

    #[test]
    fn test_drop_item_on_ground_stale_item_clears_hauler() {
        let mut world = stale_carrier_world();
        let pos = GridPosition { x: 3, y: 3 };
        let (pop, item) = spawn_stale_carrier(&mut world, pos, true);

        // Direct call into the panicking helper (trunk hauling.rs:479).
        super::drop_item_on_ground(&mut world, pop, CarryingItem(item), pos);

        assert!(
            world.get::<CarryingItem>(pop).is_none(),
            "Stale CarryingItem should be cleared"
        );
        assert!(
            world
                .get::<crate::layer1::execution::AtTarget>(pop)
                .is_none(),
            "AtTarget should be cleared"
        );
        assert!(
            world
                .get::<crate::layer1::execution::MovementTarget>(pop)
                .is_none(),
            "MovementTarget should be cleared"
        );
    }

    #[test]
    fn test_haul_system_stale_carried_item_at_target_no_container() {
        let mut world = stale_carrier_world();
        let pos = GridPosition { x: 3, y: 3 };
        // No container / gene bank / stockpile at the drop tile: the drop
        // falls through to drop_item_on_ground.
        let (pop, _item) = spawn_stale_carrier(&mut world, pos, true);

        haul_system(&mut world);

        assert!(world.get::<CarryingItem>(pop).is_none());
        assert!(world
            .get::<crate::layer1::execution::AtTarget>(pop)
            .is_none());
    }

    #[test]
    fn test_haul_system_stale_carried_item_at_gene_bank() {
        let mut world = stale_carrier_world();
        let pos = GridPosition { x: 3, y: 3 };
        // Gene bank (but no Inventory) at the drop tile: try_drop_off_gene_bank
        // must fail safe on the stale item and the fall-through must not panic.
        world.spawn((
            Building {
                building_type: BuildingType::GeneBank,
            },
            crate::layer1::gene_bank::GeneBank::default(),
            pos,
        ));
        let (pop, _item) = spawn_stale_carrier(&mut world, pos, true);

        haul_system(&mut world);

        assert!(world.get::<CarryingItem>(pop).is_none());
        assert!(world
            .get::<crate::layer1::execution::AtTarget>(pop)
            .is_none());
    }

    #[test]
    fn test_haul_system_stale_carried_item_with_container_clears() {
        use crate::layer1::inventory::Inventory;

        let mut world = stale_carrier_world();
        let pos = GridPosition { x: 3, y: 3 };
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Inventory::default(),
            pos,
        ));
        let (pop, _item) = spawn_stale_carrier(&mut world, pos, true);

        haul_system(&mut world);

        assert!(world.get::<CarryingItem>(pop).is_none());
        assert!(world
            .get::<crate::layer1::execution::AtTarget>(pop)
            .is_none());
    }

    #[test]
    fn test_haul_system_stale_carried_item_seeking_target_clears() {
        let mut world = stale_carrier_world();
        let pos = GridPosition { x: 0, y: 0 };
        // A stockpile exists to target: a live carried item would target it,
        // but a stale one must be dropped from the hauler's state instead of
        // being hauled forever (or panicking on arrival).
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile::default(),
            GridPosition { x: 10, y: 0 },
        ));
        let (pop, _item) = spawn_stale_carrier(&mut world, pos, false);

        haul_system(&mut world);

        assert!(
            world.get::<CarryingItem>(pop).is_none(),
            "Stale CarryingItem should be cleared while seeking a target"
        );
        assert!(
            world
                .get::<crate::layer1::execution::MovementTarget>(pop)
                .is_none(),
            "Hauler should not target a stockpile for a stale item"
        );
    }

    #[test]
    fn test_haul_manual_system_lifecycle() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(crate::layer1::factions::Factions::default());
        world.insert_resource(ColonyResources::default());

        // 1. Setup
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 2, y: 0 }, // At Item location
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
            ))
            .id();

        let manual = world
            .spawn((
                Item {
                    item_type: ItemType::Manual,
                },
                GridPosition { x: 2, y: 0 },
            ))
            .id();

        let _stockpile = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile::default(),
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        // 2. Pickup
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);
        haul_system(&mut world);

        assert!(world.get::<CarryingItem>(pop).is_some());
        assert_eq!(world.get::<CarryingItem>(pop).unwrap().0, manual);
        assert!(world.get::<GridPosition>(manual).is_none()); // Picked up

        // 3. Find Dropoff Target
        haul_system(&mut world);

        let target = world.get::<crate::layer1::execution::MovementTarget>(pop);
        assert!(target.is_some());
        assert_eq!(
            target.unwrap().target_position,
            GridPosition { x: 10, y: 0 }
        );

        // 4. Dropoff
        *world.get_mut::<GridPosition>(pop).unwrap() = GridPosition { x: 10, y: 0 };
        world
            .entity_mut(pop)
            .insert(crate::layer1::execution::AtTarget);

        haul_system(&mut world);

        assert!(world.get::<CarryingItem>(pop).is_none());
        assert!(world.get::<GridPosition>(manual).is_some());
        assert_eq!(
            *world.get::<GridPosition>(manual).unwrap(),
            GridPosition { x: 10, y: 0 }
        );
    }
}
