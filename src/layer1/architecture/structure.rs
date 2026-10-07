#![allow(clippy::float_cmp)]
use crate::layer1::building::Building;
use crate::layer1::building::BuildingType;
use crate::layer1::building::Material;
use crate::layer1::building::OccupiedTiles;
use crate::layer1::fire::Fire;
use crate::layer1::ruins::{Ruin, RuinHistory};
use crate::layer1::GridPosition;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Component representing the structural integrity of a building.
#[derive(Component, Debug, Clone, Copy)]
pub struct Structure {
    /// Current health points.
    pub current_hp: f32,
    /// Maximum health points.
    pub max_hp: f32,
}

impl Default for Structure {
    fn default() -> Self {
        Self {
            current_hp: 100.0,
            max_hp: 100.0,
        }
    }
}

/// Component preventing automatic utility AI repairs.
///
/// Players can manually designate repairs, but pops will not automatically
/// maintain this structure.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct DeferMaintenance;

/// Component added to buildings that have been jury-rigged.
/// They take increased damage from all sources.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Fragile {
    /// Number of times the building has been jury-rigged.
    pub stacks: u32,
}

/// Damage multiplier per stack of Fragility (50% increase per stack).
pub const FRAGILITY_DAMAGE_MULTIPLIER: f32 = 0.5;

/// System that applies fire damage to structures.
///
/// This runs every tick and reduces HP of structures standing in fire.
/// If HP reaches 0, the structure is destroyed.
#[allow(clippy::cast_precision_loss)]
pub fn fire_damage_structure_system(world: &mut World) {
    let mut destroyed = Vec::new();

    // Query for fires first to avoid borrowing conflict
    // ⚡ Bolt Optimization:
    // Use `HashMap` instead of a `Vec` for `fires` to provide O(1) lookups.
    // This eliminates the O(F * S) nested loop, dropping it to O(S), greatly reducing CPU overhead.
    // We sum intensities for overlapping fires on the same tile to preserve existing stacking behavior.
    let mut fires = bevy_utils::HashMap::default();
    let mut fire_query = world.query::<(&GridPosition, &Fire)>();
    for (pos, fire) in fire_query.iter(world) {
        *fires.entry(*pos).or_insert(0.0) += fire.intensity;
    }

    if fires.is_empty() {
        return;
    }

    let mut structure_query = world.query::<(
        Entity,
        &GridPosition,
        &mut Structure,
        Option<&Fragile>,
        Option<&Building>,
        Option<&Material>,
    )>();

    for (entity, pos, mut structure, fragile, building, material) in structure_query.iter_mut(world)
    {
        if let Some(&intensity) = fires.get(pos) {
            let base_damage = 5.0 * intensity; // 5.0 damage per tick per intensity unit
            let multiplier = fragile.map_or(1.0, |f| {
                (f.stacks as f32).mul_add(FRAGILITY_DAMAGE_MULTIPLIER, 1.0)
            });

            structure.current_hp -= base_damage * multiplier;

            if structure.current_hp <= 0.0 {
                let b_type = building.map(|b| b.building_type);
                let m_type = material.map(|m| m.0);
                destroyed.push((entity, *pos, b_type, m_type));
            }
        }
    }

    // Despawn destroyed structures
    for (entity, pos, b_type, m_type) in destroyed {
        // Double check it still exists (though unlikely to change within loop)
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);

            if let Some(building_type) = b_type {
                // Spawn Ruin
                let material = m_type.unwrap_or_default();
                let current_tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);

                world.spawn((
                    Ruin {
                        original_type: building_type,
                        material,
                    },
                    RuinHistory {
                        destruction_tick: current_tick,
                        reason: "Fire/Damage".to_string(),
                    },
                    pos, // GridPosition
                ));

                // Spec 1370 Civilizational Compost: destroyed buildings leave
                // a salvageable scrap heap alongside the ruin.
                world.spawn((
                    crate::layer1::agriculture::compost::ScrapHeap {
                        metal: 8.0,
                        wood: 8.0,
                    },
                    pos,
                ));

                // DO NOT remove from OccupiedTiles if Ruin spawns (it blocks construction)
            } else {
                // Clean up OccupiedTiles if it wasn't a building (or failed to spawn ruin)
                if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
                    occupied.0.remove(&(pos.x, pos.y));
                }
            }

            // Add log message
            if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
                log.add(format!(
                    "Structure destroyed by fire at ({}, {})",
                    pos.x, pos.y
                ));
            }
        }
    }
}

/// System that slowly degrades building HP over time (Entropy).
pub fn entropy_system(world: &mut World) {
    let mut query = world.query::<(&mut Structure, Option<&Building>)>();

    for (mut structure, building) in query.iter_mut(world) {
        // Base decay rate
        let decay = 0.01; // 0.01 HP per tick.

        // Modifiers based on building type (Walls decay slower?)
        let modifier = building.map_or(1.0, |b| match b.building_type {
            BuildingType::Wall | BuildingType::Gate => 0.1,
            _ => 1.0,
        });

        structure.current_hp = (structure.current_hp - decay * modifier).max(0.0);
    }
}

/// Calculates the probability of malfunction based on current HP percentage.
#[must_use]
pub fn calculate_malfunction_risk(current: f32, max: f32) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    let percent = current / max;
    if percent < 0.3 {
        (0.3 - percent) * 0.1
    } else {
        0.0
    }
}

/// System that triggers malfunctions in poorly maintained buildings.
pub fn malfunction_system(world: &mut World) {
    let mut events = Vec::new();
    let mut query = world.query::<(
        Entity,
        &Structure,
        &GridPosition,
        Option<&Building>,
        Option<&crate::layer1::prototyping::Prototype>,
    )>();

    for (entity, structure, pos, _building, prototype) in query.iter(world) {
        let base_risk = calculate_malfunction_risk(structure.current_hp, structure.max_hp);

        // Apply Prototype modifier
        let modifier = prototype.map_or(1.0, |p| p.breakdown_chance_modifier);
        let risk = base_risk * modifier;

        if risk > 0.0 && rand::random::<f32>() < risk {
            events.push((entity, *pos));
        }
    }

    for (_entity, pos) in events {
        // Trigger malfunction: Spawn Fire
        world.spawn((
            crate::layer1::fire::Fire {
                intensity: 1.0,
                lifetime: 10,
            },
            pos,
        ));

        // Notification
        if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
            log.add(format!(
                "Malfunction at ({}, {}) due to lack of maintenance!",
                pos.x, pos.y
            ));
        }
    }
}

/// Logic to process repair work.
///
/// Increases structure HP. Removes target if it is a fully completed designation.
/// Does NOT remove target if it is the structure itself.
///
/// Returns `true` if repair is complete (max HP reached).
pub fn process_repair(world: &mut World, target_entity: Entity, amount: f32) -> bool {
    // 1. Determine target type and position
    let is_designation = world
        .get::<crate::layer1::designation::Designation>(target_entity)
        .is_some();
    let is_structure = world.get::<Structure>(target_entity).is_some();

    let Some(pos) = world.get::<GridPosition>(target_entity).copied() else {
        return false;
    };

    // 2. Identify the structure entity to repair
    let structure_to_repair = if is_structure {
        Some(target_entity)
    } else {
        // Search for structure at this position
        // Use a scope to borrow world for query
        let mut query = world.query_filtered::<(Entity, &GridPosition), With<Structure>>();
        query
            .iter(world)
            .find_map(|(entity, p)| if *p == pos { Some(entity) } else { None })
    };

    let Some(structure_entity) = structure_to_repair else {
        // Structure missing? If designation, remove it.
        if is_designation {
            world.despawn(target_entity);
        }
        return true; // Technically complete since target is gone
    };

    // 3. Apply repair
    let mut new_hp = 0.0;
    let mut max_hp = 0.0;
    let mut old_hp = 0.0;
    let mut ancient = false;

    // Check for AncientStructure prevention
    if world
        .get::<crate::layer1::heirloom::AncientStructure>(structure_entity)
        .is_some()
    {
        ancient = true;
    } else if let Some(mut s) = world.get_mut::<Structure>(structure_entity) {
        old_hp = s.current_hp;
        s.current_hp = (s.current_hp + amount).min(s.max_hp);
        new_hp = s.current_hp;
        max_hp = s.max_hp;
    }

    if ancient {
        return false; // Cannot repair
    }

    // Spec 901: Reveal hidden quirks on repair
    if new_hp > old_hp {
        if let Some(hidden_quirk) = world
            .get::<crate::layer2::derelict_stations::HiddenQuirk>(structure_entity)
            .map(|q| q.0)
        {
            world
                .entity_mut(structure_entity)
                .remove::<crate::layer2::derelict_stations::HiddenQuirk>();
            world
                .entity_mut(structure_entity)
                .insert(crate::layer1::social::rituals::Quirk {
                    quirk_type: hidden_quirk,
                });
        }
    }

    let fully_repaired = (new_hp - max_hp).abs() < f32::EPSILON;

    // 4. Cleanup designation if fully repaired
    if is_designation && fully_repaired {
        world.despawn(target_entity);
    }

    fully_repaired
}

/// Instantly repairs a structure but adds fragility.
///
/// This is a "god power" or instant action that fully heals the building
/// but makes it susceptible to future damage.
pub fn process_jury_rig(world: &mut World, structure_entity: Entity) {
    // 1. Fully heal
    if let Some(mut structure) = world.get_mut::<Structure>(structure_entity) {
        structure.current_hp = structure.max_hp;
    }

    // 2. Add/Increment Fragile
    if let Some(mut fragile) = world.get_mut::<Fragile>(structure_entity) {
        fragile.stacks += 1;
    } else {
        world
            .entity_mut(structure_entity)
            .insert(Fragile { stacks: 1 });
    }

    // Note: The designation cleanup is handled by the caller (work execution system)
    // unlike process_repair which handles it internally because it's incremental.
    // If process_jury_rig is called directly, ensure designation is removed if applicable.
}

/// System that slowly damages Fragile buildings over time.
#[allow(clippy::cast_precision_loss)]
pub fn fragile_decay_system(world: &mut World) {
    let mut query = world.query::<(&mut Structure, &Fragile)>();
    for (mut structure, fragile) in query.iter_mut(world) {
        // Base chance of decay per tick (0.1% per stack)
        // At 60 ticks/sec, this is ~6% chance per second per stack.
        // Over a "day" (many ticks), it will accumulate.
        let base_chance = 0.001;
        let chance = base_chance * (fragile.stacks as f32);

        if rand::random::<f32>() < chance {
            structure.current_hp -= 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::designation::{Designation, DesignationType};
    use crate::layer1::fire::{Fire, Flammable};
    use crate::layer1::structure::{fire_damage_structure_system, Structure};
    use crate::layer1::GridPosition;
    use bevy_ecs::prelude::*;
    // use crate::layer1::utility_types::ActionType;

    #[test]
    fn test_structure_component_defaults() {
        let s = Structure::default();
        assert!(s.max_hp > 0.0);
        assert_eq!(s.current_hp, s.max_hp);
    }

    #[test]
    fn test_fire_damages_structure() {
        let mut world = World::new();

        // Spawn a building with Structure and Flammable
        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                Flammable::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Spawn Fire on top of it
        world.spawn((
            Fire {
                intensity: 1.0,
                lifetime: 10,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Run damage system
        fire_damage_structure_system(&mut world);

        // Check HP reduced
        let structure = world.get::<Structure>(building).unwrap();
        assert!(structure.current_hp < 100.0, "HP should be reduced by fire");
        assert!(structure.current_hp > 0.0, "Should not be instant kill");
    }

    #[test]
    fn test_structure_destruction_at_zero_hp() {
        let mut world = World::new();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 1.0,
                    max_hp: 100.0,
                },
                Flammable::default(),
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Fire deals damage 5.0 * 10.0 = 50.0 > 1.0
        world.spawn((
            Fire {
                intensity: 10.0,
                lifetime: 10,
            },
            GridPosition { x: 0, y: 0 },
        ));

        fire_damage_structure_system(&mut world);

        assert!(
            world.get_entity(building).is_err(),
            "Building should be destroyed at 0 HP"
        );
    }

    #[test]
    fn test_repair_restores_hp() {
        let mut world = World::new();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 50.0,
                    max_hp: 100.0,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Designation for Repair
        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Repair,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Worker performing Repair
        crate::layer1::structure::process_repair(&mut world, designation, 10.0);

        let structure = world.get::<Structure>(building).unwrap();
        assert_eq!(structure.current_hp, 60.0, "HP should be restored");
    }

    #[test]
    fn test_repair_removes_designation_at_max_hp() {
        let mut world = World::new();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 95.0,
                    max_hp: 100.0,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Repair,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        crate::layer1::structure::process_repair(&mut world, designation, 10.0);

        // HP capped at max
        let structure = world.get::<Structure>(building).unwrap();
        assert_eq!(structure.current_hp, 100.0, "HP should be capped at max");

        // Designation should be despawned
        assert!(
            world.get_entity(designation).is_err(),
            "Designation should be removed when fully repaired"
        );
    }

    #[test]
    fn test_process_repair_on_direct_structure_does_not_despawn() {
        let mut world = World::new();

        let building = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 50.0,
                    max_hp: 100.0,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        // Repair building directly (no designation)
        crate::layer1::structure::process_repair(&mut world, building, 60.0); // Full heal + extra

        // Building should still exist
        assert!(
            world.get_entity(building).is_ok(),
            "Building should not be despawned"
        );
        let s = world.get::<Structure>(building).unwrap();
        assert_eq!(s.current_hp, 100.0);
    }
}
