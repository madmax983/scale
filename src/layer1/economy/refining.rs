// src/layer1/refining.rs
#![allow(clippy::too_many_lines, clippy::collapsible_if)]

//! Resource processing and refinement system.
//!
//! This module handles the conversion of raw materials (e.g., Wood, Ore) into
//! refined products (e.g., Planks, Metal) at industrial buildings.
//!
//! # The Refining Loop
//!
//! Refining is a continuous process that occurs tick-by-tick in [`process_refining_system`]:
//!
//! 1.  **Check Requirements**:
//!     *   Is the building active? (Powered, if applicable).
//!     *   Is there a worker AT the building doing `ActionType::Refine`?
//!     *   Are there input resources available? (e.g., Wood > 1).
//!     *   Is there storage space for output? (e.g., Planks < Max).
//! 2.  **Progress**:
//!     *   The worker's `Skills` (Crafting) determine efficiency.
//!     *   Progress accumulates in [`RefiningProgress`].
//! 3.  **Completion**:
//!     *   Inputs are consumed.
//!     *   Outputs are produced.
//!     *   Waste (Pollution) may be generated.
//!     *   Worker gains experience.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::crafting::byproducts::{bin_chokes, capture_byproducts, CraftingBuilding, Inventory};
use crate::layer1::eureka::check_for_eureka_world;
use crate::layer1::factions::{FactionMember, FactionState, Factions};
use crate::layer1::law::aesthetic_edict::Halted;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{ColonyResources, RefiningProgress, ResourceItem, ResourceType};
use crate::layer1::skills::{get_skill_efficiency, SkillType, Skills};
use crate::layer1::tech::Tech;
use crate::layer1::utility_ai::{ActionType, PopAction};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;
use rand::Rng;

/// System that processes refining at buildings like Lumber Mills and Stone Masons.
///
/// # Algorithm
///
/// 1.  **Filter Workers**: Collects all Pops with `ActionType::Refine` at a [`GridPosition`].
/// 2.  **Iterate Buildings**: Finds all entities with [`RefiningProgress`] and [`Building`].
/// 3.  **Power Check**: Skips buildings with inactive [`PowerConsumer`](crate::layer1::energy::PowerConsumer) components.
/// 4.  **Match Worker**: Checks if any worker is at the building's position.
/// 5.  **Check Recipe**: Calls [`get_refining_recipe`] to verify resource availability.
/// 6.  **Apply Work**:
///     *   Calculates efficiency based on worker's `Crafting` skill.
///     *   Increments progress.
/// 7.  **Finalize**:
///     *   On completion, atomically deducts input and adds output using [`ColonyResources::try_deduct`].
///     *   Spawns `Waste` with 50% probability.
///     *   Resets progress.
#[doc(alias = "crafting")]
pub fn process_refining_system(world: &mut World) {
    let workers = collect_active_refining_workers(world);

    let resources_snapshot = world
        .get_resource::<ColonyResources>()
        .cloned()
        .unwrap_or_default();

    let (finished_jobs, xp_gains) =
        process_active_refining_buildings(world, &workers, &resources_snapshot);

    // Apply XP gains
    for worker_entity in xp_gains {
        if let Some(mut skills) = world.get_mut::<Skills>(worker_entity) {
            skills.add_xp(SkillType::Crafting, 1.0);
        }
    }

    apply_finished_refining_jobs(world, finished_jobs);
}

/// Returns the refining recipe for a building type.
///
/// Returns: (`CanRefine`, `InputCost`, `OutputGain`, `WasteChance`)
#[must_use]
pub fn get_refining_recipe(
    building_type: BuildingType,
    res: &ColonyResources,
) -> (bool, ColonyResources, ColonyResources, f64) {
    match building_type {
        BuildingType::Smokehouse => (
            res.food >= 5.0 && res.wood >= 1.0 && res.rations < res.max_rations,
            ColonyResources {
                food: 5.0,
                wood: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                rations: 5.0,
                ..ColonyResources::zeroed()
            },
            0.0, // Clean
        ),
        BuildingType::Recycler => (
            res.biological_waste >= 5.0 && res.nutrient_paste < res.max_nutrient_paste,
            ColonyResources {
                biological_waste: 5.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                nutrient_paste: 5.0,
                ..ColonyResources::zeroed()
            },
            0.5, // Unpleasant aura/waste byproduct
        ),
        BuildingType::LumberMill => (
            res.wood >= 1.0 && res.planks < res.max_planks,
            ColonyResources {
                wood: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                planks: 1.0,
                ..ColonyResources::zeroed()
            },
            0.2, // Sawdust
        ),
        BuildingType::StoneMason => (
            res.stone >= 1.0 && res.blocks < res.max_blocks,
            ColonyResources {
                stone: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                blocks: 1.0,
                ..ColonyResources::zeroed()
            },
            0.2, // Dust
        ),
        BuildingType::Smelter => (
            res.ore >= 1.0 && res.wood >= 1.0 && res.metal < res.max_metal,
            ColonyResources {
                ore: 1.0,
                wood: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                metal: 1.0,
                ..ColonyResources::zeroed()
            },
            0.5, // Slag
        ),
        BuildingType::Smithy => (
            res.metal >= 1.0 && res.wood >= 1.0 && res.tools < res.max_tools,
            ColonyResources {
                metal: 1.0,
                wood: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                tools: 1.0,
                ..ColonyResources::zeroed()
            },
            0.2, // Scraps
        ),
        BuildingType::Weaver => (
            res.fiber >= 1.0 && res.cloth < res.max_cloth,
            ColonyResources {
                fiber: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                cloth: 1.0,
                ..ColonyResources::zeroed()
            },
            0.1, // Fiber waste
        ),
        // Spec 1384 §4.5: the Tailor turns from clothing to EVA suits
        // whenever it can — suits are life-critical and expensive
        // (2 Cloth + 1 Metal each); when materials run short or the
        // suit pool is full it falls back to the clothing recipe.
        BuildingType::Tailor => (
            (res.cloth >= 2.0 && res.metal >= 1.0 && res.eva_suits < res.max_eva_suits)
                || (res.cloth >= 1.0 && res.clothing < res.max_clothing),
            if res.cloth >= 2.0 && res.metal >= 1.0 && res.eva_suits < res.max_eva_suits {
                ColonyResources {
                    cloth: 2.0,
                    metal: 1.0,
                    ..ColonyResources::zeroed()
                }
            } else {
                ColonyResources {
                    cloth: 1.0,
                    ..ColonyResources::zeroed()
                }
            },
            if res.cloth >= 2.0 && res.metal >= 1.0 && res.eva_suits < res.max_eva_suits {
                ColonyResources {
                    eva_suits: 1.0,
                    ..ColonyResources::zeroed()
                }
            } else {
                ColonyResources {
                    clothing: 1.0,
                    ..ColonyResources::zeroed()
                }
            },
            0.1, // Cloth scraps
        ),
        BuildingType::Refinery => (
            res.ore >= 2.0 && res.fuel < res.max_fuel,
            ColonyResources {
                ore: 2.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                fuel: 1.0,
                ..ColonyResources::zeroed()
            },
            0.8, // Toxic Sludge
        ),
        BuildingType::Office => (
            res.wood >= 1.0 && res.building_permits < res.max_building_permits,
            ColonyResources {
                wood: 1.0,
                ..ColonyResources::zeroed()
            },
            ColonyResources {
                building_permits: 1.0,
                ..ColonyResources::zeroed()
            },
            0.1, // Shredded paper / waste
        ),
        _ => (
            false,
            ColonyResources::zeroed(),
            ColonyResources::zeroed(),
            0.0,
        ),
    }
}

#[allow(clippy::type_complexity)]
fn process_active_refining_buildings(
    world: &mut World,
    workers: &[(Entity, GridPosition)],
    resources_snapshot: &ColonyResources,
) -> (
    Vec<(Entity, ColonyResources, ColonyResources, GridPosition, f64)>,
    Vec<Entity>,
) {
    let mut finished_jobs = Vec::new();
    let mut xp_gains = Vec::new();

    let buildings: Vec<(
        Entity,
        BuildingType,
        GridPosition,
        f32,
        f32,
        bool,
        f32,
        Option<CraftingBuilding>,
        Option<Inventory>,
    )> = world
        .query_filtered::<(
            Entity,
            &Building,
            &GridPosition,
            &RefiningProgress,
            Option<&crate::layer1::energy::PowerConsumer>,
            Option<&crate::layer1::rituals::Quirk>,
            Option<&crate::layer1::prototyping::Prototype>,
            Option<&CraftingBuilding>,
            Option<&Inventory>,
        ), Without<Halted>>()
        .iter(world)
        .map(|(e, b, p, prog, power, quirk, prototype, crafting, inventory)| {
            let active = power.is_none_or(|c| c.active);
            let quirk_stops = quirk.is_some_and(crate::layer1::rituals::Quirk::stops_production);
            let efficiency_mod = prototype.map_or(1.0, |pr| pr.efficiency_modifier);
            (
                e,
                b.building_type,
                *p,
                prog.current,
                prog.max,
                active && !quirk_stops,
                efficiency_mod,
                crafting.cloned(),
                inventory.cloned(),
            )
        })
        .collect();

    for (
        building_entity,
        building_type,
        pos,
        _current_prog,
        _max_prog,
        is_active,
        efficiency_mod,
        crafting,
        inventory,
    ) in buildings
    {
        if !is_active {
            continue;
        }

        // Tech Corruption Check
        // ⚡ Bolt Optimization: Defers `TechState` lookup to only the specific `tech` needed
        // to avoid cloning the entire `HashMap` O(N) on every frame.
        if let Some(tech) = building_type.required_tech() {
            if let Some(ts) = world.get_resource::<crate::layer1::tech::TechState>() {
                if ts.techs.get(&tech) != Some(&crate::layer1::tech::TechStatus::Active) {
                    continue;
                }
            }
        }

        // Find worker AT the building
        let worker_at_building = workers.iter().find(|(_, p)| *p == pos);

        let Some((worker_entity, _)) = worker_at_building else {
            continue;
        };

        // Get skill efficiency
        let efficiency = {
            let skills = world.get::<Skills>(*worker_entity);
            get_skill_efficiency(skills, SkillType::Crafting)
        };

        let (can_refine, input_cost, output_gain, waste_chance) =
            get_refining_recipe(building_type, resources_snapshot);

        if !can_refine {
            continue;
        }

        // Item 108 (Spec 354 live wiring): a waste-choked byproduct bin
        // blocks completion — the building stalls at full progress until
        // its bin is dumped.
        let bin_choked = match (&crafting, &inventory) {
            (Some(crafting), Some(inventory)) => bin_chokes(crafting, inventory),
            _ => false,
        };
        if let Some(mut progress) = world.get_mut::<RefiningProgress>(building_entity) {
            progress.current += 1.0 * efficiency * efficiency_mod;
            if progress.is_complete() {
                if bin_choked {
                    progress.current = progress.max;
                    continue;
                }
                finished_jobs.push((building_entity, input_cost, output_gain, pos, waste_chance));

                // Rhythm update
                let tick = world
                    .get_resource::<crate::shared::time::SimulationTime>()
                    .map(|t| t.tick)
                    .unwrap_or(0);
                if let Some(mut rhythm) =
                    world.get_mut::<crate::layer1::tech::rhythm::MachineRhythm>(building_entity)
                {
                    rhythm.cycle_end_tick = tick;
                }
            }
        }

        xp_gains.push(*worker_entity);

        // Eureka Check
        let related_tech = match building_type {
            BuildingType::Smelter => Some(Tech::MetalWorking),
            // Add others
            _ => None,
        };

        check_for_eureka_world(
            world,
            ActionType::Refine,
            related_tech,
            Some(*worker_entity),
        );
    }

    (finished_jobs, xp_gains)
}

fn apply_finished_refining_jobs(
    world: &mut World,
    finished_jobs: Vec<(Entity, ColonyResources, ColonyResources, GridPosition, f64)>,
) {
    for (entity, input, output, pos, waste_chance) in finished_jobs {
        let success = world
            .get_resource_mut::<ColonyResources>()
            .is_some_and(|mut resources| {
                if resources.try_deduct(&input) {
                    resources.add_rations(output.rations);
                    resources.add_planks(output.planks);
                    resources.add_blocks(output.blocks);
                    resources.add_metal(output.metal);
                    resources.add_tools(output.tools);
                    resources.add_cloth(output.cloth);
                    resources.add_clothing(output.clothing);
                resources.add_eva_suits(output.eva_suits);
                    resources.add_fuel(output.fuel);
                    true
                } else {
                    false
                }
            });

        if success {
            // Spawn Waste (Spec 354, Item 108 live wiring): a building
            // with a byproduct bin captures its byproduct in the bin;
            // bin-less buildings keep the historical ground spill.
            let mut rng = rand::thread_rng();
            if rng.gen_bool(waste_chance) {
                let captured = match (
                    world.get::<CraftingBuilding>(entity).cloned(),
                    world.get_mut::<Inventory>(entity),
                ) {
                    (Some(crafting), Some(mut inv)) => capture_byproducts(&crafting, &mut inv),
                    _ => false,
                };
                if !captured {
                    world.spawn((
                        ResourceItem {
                            resource_type: ResourceType::Waste,
                            amount: 1.0,
                        },
                        pos,
                    ));
                }
            }

            // Paperwork Physicality: Spawn BuildingPermit as item instead of adding to global storage
            if output.building_permits > 0.0 {
                world.spawn((
                    ResourceItem {
                        resource_type: ResourceType::BuildingPermit,
                        amount: output.building_permits,
                    },
                    pos,
                ));
            }

            if let Some(mut progress) = world.get_mut::<RefiningProgress>(entity) {
                progress.current = 0.0;
            }
        } else if let Some(mut progress) = world.get_mut::<RefiningProgress>(entity) {
            progress.current = progress.max;
        }
    }
}

fn collect_active_refining_workers(world: &mut World) -> Vec<(Entity, GridPosition)> {
    // ⚡ Bolt Optimization: Avoid cloning the Factions HashMap every tick.
    // We also avoid intermediate heap allocations by creating the query state first,
    // getting the read-only Factions resource, and then iterating directly.
    let mut query = world
        .query_filtered::<(Entity, &GridPosition, &PopAction, Option<&FactionMember>), With<Pop>>();

    let factions_data = world.get_resource::<Factions>().map(|f| &f.map);

    query
        .iter(world)
        .filter_map(|(entity, pos, action, member)| {
            if action.current != ActionType::Refine {
                return None;
            }

            let is_striking = member
                .and_then(|m| m.faction_id)
                .and_then(|fid| factions_data.and_then(|map| map.get(&fid)))
                .is_some_and(|d| d.state == FactionState::Striking);

            if is_striking {
                None
            } else {
                Some((entity, *pos))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::pop::Pop;
    use crate::layer1::refining::process_refining_system;
    use crate::layer1::resources::{ColonyResources, RefiningProgress};
    use crate::layer1::skills::{SkillType, Skills};
    use crate::layer1::utility_ai::{ActionType, PopAction};
    use crate::layer1::GridPosition;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_colony_resources_refined_fields() {
        let resources = ColonyResources::default();
        assert!((resources.planks - 0.0).abs() < f32::EPSILON);
        assert!((resources.blocks - 0.0).abs() < f32::EPSILON);
        assert!((resources.max_planks - 50.0).abs() < f32::EPSILON);
        assert!((resources.max_blocks - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_process_refining_lumber_mill() {
        let mut world = World::new();

        let resources = ColonyResources {
            wood: 10.0,
            planks: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 0.0,
                max: 10.0,
            },
        ));

        // Spawn Worker AT (5, 5) with correct action
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Refine,
                current_utility: 0.5,
                ticks_committed: 1,
            },
        ));

        process_refining_system(&mut world);

        let progress = world.query::<&RefiningProgress>().single(&world);
        assert!(progress.current > 0.0);
    }

    #[test]
    fn test_process_refining_skills_efficiency() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            wood: 10.0,
            ..Default::default()
        });

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 0.0,
                max: 10.0,
            },
        ));

        let mut skills = Skills::default();
        skills.add_xp(SkillType::Crafting, 100.0);
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            skills,
            PopAction {
                current: ActionType::Refine,
                ..Default::default()
            },
        ));

        process_refining_system(&mut world);

        let progress = world.query::<&RefiningProgress>().single(&world);
        assert!((progress.current - 1.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_process_refining_skills_xp() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            wood: 10.0,
            ..Default::default()
        });

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 0.0,
                max: 10.0,
            },
        ));

        let worker = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                Skills::default(),
                PopAction {
                    current: ActionType::Refine,
                    ..Default::default()
                },
            ))
            .id();

        process_refining_system(&mut world);

        let skills = world.get::<Skills>(worker).unwrap();
        assert_eq!(skills.get_xp(SkillType::Crafting), 1.0);
    }

    #[test]
    fn test_process_refining_consumes_input_on_complete() {
        let mut world = World::new();

        let resources = ColonyResources {
            wood: 10.0,
            planks: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 9.9,
                max: 10.0,
            },
        ));

        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Refine,
                ..Default::default()
            },
        ));

        process_refining_system(&mut world);

        let res = world.resource::<ColonyResources>();
        assert!((res.wood - 9.0).abs() < f32::EPSILON);
        assert!((res.planks - 1.0).abs() < f32::EPSILON);

        let progress = world.query::<&RefiningProgress>().single(&world);
        assert!(progress.current < 1.0);
    }

    #[test]
    fn test_refining_stops_if_no_input() {
        let mut world = World::new();
        let resources = ColonyResources {
            wood: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress::default(),
        ));
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Refine,
                ..Default::default()
            },
        ));

        process_refining_system(&mut world);

        let progress = world.query::<&RefiningProgress>().single(&world);
        assert!((progress.current - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_office_produces_physical_permit() {
        use crate::layer1::resources::{ResourceItem, ResourceType};
        let mut world = World::new();

        let resources = ColonyResources {
            wood: 10.0,
            building_permits: 0.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        // Office
        world.spawn((
            Building {
                building_type: BuildingType::Office,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 9.9, // Almost done
                max: 10.0,
            },
        ));

        // Worker
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Refine,
                current_utility: 0.5,
                ticks_committed: 1,
            },
        ));

        process_refining_system(&mut world);

        // Verify Permit Entity Spawned
        let item = world
            .query::<(&ResourceItem, &GridPosition)>()
            .iter(&world)
            .find(|(i, _)| i.resource_type == ResourceType::BuildingPermit);

        assert!(item.is_some(), "Should spawn BuildingPermit item");
        let (item_data, pos) = item.unwrap();
        assert_eq!(item_data.amount, 1.0);
        assert_eq!(pos.x, 5);

        // Verify NOT added to global resources
        let res = world.resource::<ColonyResources>();
        assert_eq!(
            res.building_permits, 0.0,
            "Permit should be physical, not global"
        );
        assert!(
            (res.wood - 9.0).abs() < f32::EPSILON,
            "Wood should be consumed"
        );
    }

    #[test]
    fn test_quirk_stops_refining() {
        use crate::layer1::rituals::{Quirk, QuirkType};
        let mut world = World::new();

        world.insert_resource(ColonyResources {
            wood: 10.0,
            ..Default::default()
        });

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 5, y: 5 },
            RefiningProgress {
                current: 0.0,
                max: 10.0,
            },
            Quirk {
                quirk_type: QuirkType::Glitchy,
            },
        ));

        // Worker
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Refine,
                current_utility: 0.5,
                ticks_committed: 1,
            },
        ));

        process_refining_system(&mut world);

        let progress = world.query::<&RefiningProgress>().single(&world);
        assert_eq!(progress.current, 0.0, "Glitchy mill should not progress");
    }
}

#[cfg(test)]
mod recycler_production_tests {
    use super::*;
    use crate::layer1::architecture::building::BuildingType;
    use crate::layer1::economy::resources::ColonyResources;

    #[test]
    fn test_recycler_recipe() {
        let res = ColonyResources {
            biological_waste: 10.0,
            max_nutrient_paste: 50.0,
            ..ColonyResources::default()
        };

        // Spec 1384 §3/§4.5: Tailor prefers suits when it can afford
        // them, falls back to clothing otherwise.
        let tailor_res = ColonyResources {
            cloth: 5.0,
            metal: 2.0,
            eva_suits: 0.0,
            max_eva_suits: 50.0,
            ..ColonyResources::zeroed()
        };
        let (can, input, output, _w) = get_refining_recipe(BuildingType::Tailor, &tailor_res);
        assert!(can);
        assert_eq!(input.cloth, 2.0);
        assert_eq!(input.metal, 1.0);
        assert_eq!(output.eva_suits, 1.0);
        let poor_res = ColonyResources {
            cloth: 1.0,
            metal: 0.0,
            max_clothing: 50.0,
            ..ColonyResources::zeroed()
        };
        let (can2, _i2, output2, _w2) = get_refining_recipe(BuildingType::Tailor, &poor_res);
        assert!(can2, "clothing fallback still refines");
        assert_eq!(output2.clothing, 1.0);

        let (can_refine, input, output, _waste) = get_refining_recipe(BuildingType::Recycler, &res);
        assert!(can_refine);
        assert_eq!(input.biological_waste, 5.0);
        assert_eq!(output.nutrient_paste, 5.0);
    }
}
