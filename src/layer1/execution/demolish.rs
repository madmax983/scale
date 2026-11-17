use bevy_ecs::prelude::*;
use ratatui::style::Color;

use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::heirloom::{AncientStructure, RetrogradeEngineeringEvent};
use crate::layer1::map::{GridPosition, ScreenShake};
use crate::layer1::particles::spawn_particle;
use crate::layer1::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::ruins::{process_scavenge, Ruin};
use crate::layer1::structure::{process_jury_rig, Structure};
use crate::shared::log::MessageLog;

/// Executes the demolition of a building at the designation's location.
///
/// If the building is an Ancient Structure, this triggers "Retrograde Engineering",
/// awarding Knowledge instead of resources/debris.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::execution::demolish::execute_demolish;
/// use scale::layer1::map::GridPosition;
/// use scale::layer1::building::{Building, BuildingType};
///
/// let mut world = World::new();
/// let pos = GridPosition { x: 5, y: 5 };
///
/// let building = world.spawn((pos, Building { building_type: BuildingType::Housing })).id();
/// let designation = world.spawn(pos).id();
///
/// let success = execute_demolish(&mut world, designation);
/// assert!(success);
/// assert!(world.get_entity(designation).is_err());
/// assert!(world.get_entity(building).is_err());
/// ```
pub fn execute_demolish(world: &mut World, designation_entity: Entity) -> bool {
    let Some(designation_pos) = world.get::<GridPosition>(designation_entity).copied() else {
        return false;
    };

    if try_scavenge_ruin(world, designation_entity, designation_pos) {
        return true;
    }

    try_demolish_building(world, designation_pos);

    // Despawn the designation itself
    world.despawn(designation_entity);
    true
}

fn try_scavenge_ruin(
    world: &mut World,
    designation_entity: Entity,
    designation_pos: GridPosition,
) -> bool {
    let ruin_entity = world
        .query::<(Entity, &GridPosition, &Ruin)>()
        .iter(world)
        .find(|(_, pos, _)| **pos == designation_pos)
        .map(|(e, _, _)| e);

    let Some(ruin) = ruin_entity else {
        return false;
    };

    let yielded = process_scavenge(world, ruin);
    if !yielded.is_empty() {
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add_colored("Scavenged resources from Ruin.", Color::Green);
        }
    }
    // Despawn the designation itself
    world.despawn(designation_entity);
    true
}

fn try_demolish_building(world: &mut World, designation_pos: GridPosition) {
    let building_entity = world
        .query::<(Entity, &GridPosition, &Building)>()
        .iter(world)
        .find(|(_, pos, _)| pos.x == designation_pos.x && pos.y == designation_pos.y)
        .map(|(e, _, _)| e);

    let Some(entity) = building_entity else {
        return;
    };

    let building_type = world.get::<Building>(entity).map(|b| b.building_type);

    // Spec 426: indestructible buildings (the Archive of Regret) refuse
    // demolition — the designation is consumed, the building stands.
    if building_type.is_some_and(|t| t.is_indestructible()) {
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add("The Archive of Regret cannot be demolished.".to_string());
        }
        return;
    }

    let is_ancient = world.get::<AncientStructure>(entity).is_some();

    if is_ancient {
        handle_ancient_structure(world, designation_pos, building_type);
    } else {
        // Normal Debris
        spawn_particle(world, designation_pos, 'X', Color::Red, 10);
    }

    world.despawn(entity);

    // Trigger Screen Shake (Ludwig: "Juice")
    if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
        shake.trigger(0.5);
    }

    // Remove from OccupiedTiles
    if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
        occupied.0.remove(&(designation_pos.x, designation_pos.y));
    }

    // Send removal event (for Ghost Code, etc)
    if let Some(btype) = building_type {
        world.send_event(crate::layer1::core::events::BuildingRemovedEvent {
            entity,
            position: designation_pos,
            building_type: btype,
        });
    }
}

fn handle_ancient_structure(
    world: &mut World,
    designation_pos: GridPosition,
    building_type: Option<BuildingType>,
) {
    // Retrograde Engineering: Award Knowledge
    let amount = calculate_knowledge_reward(building_type);

    if let Some(mut res) = world.get_resource_mut::<ColonyResources>() {
        res.add_knowledge(amount);
    }

    let label = building_type.map_or_else(
        || "Ancient Structure".to_string(),
        |b| b.label().to_string(),
    );

    // Fire integration event
    world.send_event(RetrogradeEngineeringEvent {
        building_label: label.clone(),
        knowledge_gained: amount,
    });

    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add_colored(
            format!("Retrograde Engineering: Deconstructed {label} for {amount} Knowledge."),
            Color::Cyan,
        );
    }

    // Cyan 'data' sparks
    spawn_particle(world, designation_pos, '?', Color::Cyan, 15);
}

/// Executes the cannibalization of the Lander.
///
/// This destroys the Lander and spawns a large amount of resources.
pub fn execute_cannibalize(world: &mut World, designation_entity: Entity) -> bool {
    let Some(designation_pos) = world.get::<GridPosition>(designation_entity).copied() else {
        return false;
    };

    // Find building at this position
    let building_entity = world
        .query::<(Entity, &GridPosition, &Building)>()
        .iter(world)
        .find(|(_, pos, b)| {
            pos.x == designation_pos.x
                && pos.y == designation_pos.y
                && b.building_type == BuildingType::Lander
        })
        .map(|(e, _, _)| e);

    if let Some(entity) = building_entity {
        // Spawn Resources
        spawn_resource_pile(world, designation_pos, ResourceType::Metal, 100.0);
        spawn_resource_pile(world, designation_pos, ResourceType::Fuel, 50.0);
        spawn_resource_pile(world, designation_pos, ResourceType::Rations, 50.0);

        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add_colored(
                "Lander cannibalized! Massive resources gained.",
                Color::Yellow,
            );
        }

        // VFX
        spawn_particle(world, designation_pos, 'X', Color::Red, 20);
        if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
            shake.trigger(0.8);
        }

        // Cleanup
        world.despawn(entity);
        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&(designation_pos.x, designation_pos.y));
        }

        world.despawn(designation_entity);
        return true;
    }

    // If we are here, we didn't find a Lander (maybe destroyed already)
    // Clean up designation anyway
    world.despawn(designation_entity);
    false
}

/// Executes the total destruction of a building (Vacuum Welded or otherwise).
///
/// Unlike Demolish, this yields NO resources.
pub fn execute_destroy(world: &mut World, designation_entity: Entity) -> bool {
    // Find designation position
    let designation_pos = if let Some(pos) = world.get::<GridPosition>(designation_entity) {
        *pos
    } else {
        return false;
    };

    // Find Building at position
    let building_entity = world
        .query::<(Entity, &GridPosition, &Building)>()
        .iter(world)
        .find(|(_, pos, _)| **pos == designation_pos)
        .map(|(e, _, _)| e);

    // Spec 426: indestructible buildings refuse total destruction too.
    if let Some(entity) = building_entity {
        let building_type = world.get::<Building>(entity).map(|b| b.building_type);
        if building_type.is_some_and(|t| t.is_indestructible()) {
            if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
                log.add("The Archive of Regret cannot be demolished.".to_string());
            }
            world.despawn(designation_entity);
            return true;
        }
    }

    if let Some(entity) = building_entity {
        // VFX: Red explosion
        spawn_particle(world, designation_pos, 'X', Color::Red, 20);

        // Trigger Screen Shake
        if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
            shake.trigger(0.8);
        }

        // Despawn building
        world.despawn(entity);

        // Remove from OccupiedTiles
        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&(designation_pos.x, designation_pos.y));
        }

        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add_colored("Building destroyed (Total Loss).", Color::Red);
        }
    }

    // Despawn the designation itself
    world.despawn(designation_entity);
    true
}

/// Executes a jury-rigging operation on a structure.
///
/// This attempts to temporarily repair or bypass issues in a structure
/// at the designation's location.
pub fn execute_jury_rig(world: &mut World, designation_entity: Entity) -> bool {
    // Find designation position
    world
        .get::<GridPosition>(designation_entity)
        .copied()
        .is_some_and(|designation_pos| {
            // Find structure at this position
            let structure_entity = world
                .query::<(Entity, &GridPosition, &Structure)>()
                .iter(world)
                .find(|(_, pos, _)| **pos == designation_pos)
                .map(|(e, _, _)| e);

            if let Some(entity) = structure_entity {
                process_jury_rig(world, entity);
            }

            // Despawn the designation itself (Jury-Rig is one-shot)
            world.despawn(designation_entity);
            true
        })
}

fn spawn_resource_pile(world: &mut World, pos: GridPosition, res_type: ResourceType, amount: f32) {
    world.spawn((
        ResourceItem {
            resource_type: res_type,
            amount,
        },
        pos,
    ));
}

const fn calculate_knowledge_reward(building_type: Option<BuildingType>) -> f32 {
    match building_type {
        Some(BuildingType::AncientReactor) => 500.0,
        Some(BuildingType::AncientFabricator) => 300.0,
        _ => 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::resources::{ResourceItem, ResourceType};
    use crate::layer1::structure::Structure;

    #[test]
    fn test_execute_cannibalize_success() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let building = world
            .spawn((
                pos,
                Building {
                    building_type: BuildingType::Lander,
                },
            ))
            .id();

        let designation = world.spawn(pos).id();

        assert!(execute_cannibalize(&mut world, designation));
        assert!(world.get_entity(building).is_err());
        assert!(world.get_entity(designation).is_err());

        let mut found_metal = false;
        let mut found_fuel = false;
        let mut found_rations = false;

        for (_, item) in world.query::<(&GridPosition, &ResourceItem)>().iter(&world) {
            match item.resource_type {
                ResourceType::Metal => found_metal = true,
                ResourceType::Fuel => found_fuel = true,
                ResourceType::Rations => found_rations = true,
                _ => {}
            }
        }

        assert!(found_metal);
        assert!(found_fuel);
        assert!(found_rations);
    }

    #[test]
    fn test_execute_cannibalize_invalid() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };
        let designation = world.spawn(pos).id();

        assert!(!execute_cannibalize(&mut world, designation));
        assert!(world.get_entity(designation).is_err());
    }

    #[test]
    fn test_execute_destroy_success() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let building = world
            .spawn((
                pos,
                Building {
                    building_type: BuildingType::Housing,
                },
            ))
            .id();

        let designation = world.spawn(pos).id();

        assert!(execute_destroy(&mut world, designation));
        assert!(world.get_entity(building).is_err());
        assert!(world.get_entity(designation).is_err());
    }

    #[test]
    fn test_execute_destroy_invalid() {
        let mut world = World::new();
        let designation = world.spawn_empty().id();

        assert!(!execute_destroy(&mut world, designation));
    }

    #[test]
    fn test_execute_demolish_refuses_archive_of_regret() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let building = world
            .spawn((
                pos,
                Building {
                    building_type: BuildingType::ArchiveOfRegret,
                },
            ))
            .id();
        let designation = world.spawn(pos).id();

        assert!(execute_demolish(&mut world, designation));
        assert!(
            world.get_entity(building).is_ok(),
            "the Archive of Regret must survive demolition"
        );
        assert!(world.get_entity(designation).is_err());
    }

    #[test]
    fn test_execute_destroy_refuses_archive_of_regret() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let building = world
            .spawn((
                pos,
                Building {
                    building_type: BuildingType::ArchiveOfRegret,
                },
            ))
            .id();
        let designation = world.spawn(pos).id();

        assert!(execute_destroy(&mut world, designation));
        assert!(
            world.get_entity(building).is_ok(),
            "the Archive of Regret must survive total destruction"
        );
        assert!(world.get_entity(designation).is_err());
    }

    #[test]
    fn test_execute_jury_rig_success() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let structure = world.spawn((pos, Structure::default())).id();

        let designation = world.spawn(pos).id();

        assert!(execute_jury_rig(&mut world, designation));
        assert!(world.get_entity(designation).is_err());
        // Structure should still exist
        assert!(world.get_entity(structure).is_ok());
    }

    #[test]
    fn test_execute_jury_rig_invalid() {
        let mut world = World::new();
        let designation = world.spawn_empty().id();

        assert!(!execute_jury_rig(&mut world, designation));
    }
}
