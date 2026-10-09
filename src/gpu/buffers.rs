//! GPU buffer types and marshalling functions.
//!
//! Defines `#[repr(C)]` structs with `bytemuck::Pod` + `Zeroable` derives for GPU buffers,
//! and marshal functions that extract ECS data into these structs.

use bevy_ecs::prelude::*;
use wgpu;

use crate::layer1::combat::Drafted;
use crate::layer1::designation::Designation;
use crate::layer1::farm::Farm;
use crate::layer1::fauna::Fauna;
use crate::layer1::funeral::{Corpse, Grave};
use crate::layer1::housing::Housing;
use crate::layer1::law::justice::{Inmate, Wanted};
use crate::layer1::map::GridPosition;
use crate::layer1::medical::Hospital;
use crate::layer1::needs::Needs;
use crate::layer1::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::social::Tavern;
use crate::layer1::stockpile::Stockpile;
use crate::layer1::tech::Library;
use crate::layer1::utility_ai::{PopAction, UtilityConfig, UtilityWeights};

/// Buffer storage for GPU data marshalling.
///
/// Stores reusable vectors to avoid heap allocations every frame.
#[derive(Resource, Default)]
pub struct GpuBuffers {
    /// Entities corresponding to `pop_inputs`.
    pub pop_entities: Vec<Entity>,
    /// Pop data to upload to GPU.
    pub pop_inputs: Vec<GpuPopInput>,
    /// Entities corresponding to `building_inputs`.
    pub building_entities: Vec<Entity>,
    /// Building data to upload to GPU.
    pub building_inputs: Vec<GpuBuildingInput>,
}

/// GPU-aligned pop input data. One per pop being evaluated.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(clippy::pub_underscore_fields)]
pub struct GpuPopInput {
    /// Grid X position.
    pub pos_x: i32,
    /// Grid Y position.
    pub pos_y: i32,
    /// Current hunger need level.
    pub hunger: f32,
    /// Current rest need level.
    pub rest: f32,
    /// Current leisure need level.
    pub leisure: f32,
    /// Current hygiene need level.
    pub hygiene: f32,
    /// Learned distance weight.
    pub distance_weight: f32,
    /// Learned availability weight.
    pub availability_weight: f32,
    /// Utility score of the current action.
    pub current_utility: f32,
    /// 1 if the pop is drafted for combat, 0 otherwise.
    pub drafted: u32,
}

/// GPU-aligned building/target input data. One per building.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(clippy::pub_underscore_fields)]
pub struct GpuBuildingInput {
    /// Grid X position.
    pub pos_x: i32,
    /// Grid Y position.
    pub pos_y: i32,
    /// Encoded building type (0=Farm,1=Housing,2=Tavern,3=Library,4=Designation,5=ResourceItem,9=Corpse).
    pub building_type: u32,
    /// Maximum capacity.
    pub capacity: u32,
    /// Current occupancy.
    pub occupied: u32,
    /// For Haul targets: 1 if the stockpile has room, 0 otherwise.
    /// For Corpse targets: 1 if ANY grave is available, 0 otherwise.
    pub resource_has_room: u32,
    /// Padding to 32-byte alignment.
    pub _padding: [u32; 2],
}

/// GPU-aligned per-tick global parameters.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(clippy::pub_underscore_fields)]
pub struct GpuGlobalState {
    /// Minimum utility difference to switch actions.
    pub switch_threshold: f32,
    /// 1 if knowledge is at max capacity, 0 otherwise.
    pub knowledge_full: u32,
    /// 1 if at least one stockpile exists, 0 otherwise.
    pub has_stockpile: u32,
    /// Total number of pops in this dispatch.
    pub pop_count: u32,
    /// Total number of buildings in this dispatch.
    pub building_count: u32,
    /// Padding to 32-byte alignment.
    pub _padding: [u32; 3],
}

/// GPU output: the decision for one pop.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuPopDecision {
    /// Index of the best action (maps to `ActionType`).
    pub best_action: u32,
    /// Utility score of the best action.
    pub best_utility: f32,
    /// Index into the building array for the chosen target.
    pub target_index: u32,
    /// 1 if this pop should switch from its current action, 0 otherwise.
    pub switched: u32,
}

// ---------------------------------------------------------------------------
// Marshal functions
// ---------------------------------------------------------------------------

/// Extracts pop data for GPU evaluation.
///
/// Only includes pops whose `ticks_committed >= config.evaluation_interval`.
/// Populates `entities` and `inputs` vectors, clearing them first.
#[allow(clippy::cast_possible_truncation)]
pub fn extract_pop_inputs(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuPopInput>,
) {
    let evaluation_interval = world
        .get_resource::<UtilityConfig>()
        .map_or(1, |c| c.evaluation_interval);

    entities.clear();
    inputs.clear();

    let mut query = world.query::<(
        Entity,
        &GridPosition,
        &Needs,
        &UtilityWeights,
        &PopAction,
        Option<&Inmate>,
        Option<&Drafted>,
    )>();

    for (entity, pos, needs, weights, action, inmate, drafted) in query.iter(world) {
        if action.ticks_committed < evaluation_interval || inmate.is_some() {
            continue;
        }

        entities.push(entity);
        inputs.push(GpuPopInput {
            pos_x: pos.x,
            pos_y: pos.y,
            hunger: needs.hunger,
            rest: needs.rest,
            leisure: needs.leisure,
            hygiene: needs.hygiene,
            distance_weight: weights.distance_weight,
            availability_weight: weights.availability_weight,
            current_utility: action.current_utility,
            drafted: u32::from(drafted.is_some()),
        });
    }
}

/// Extracts building/target data for GPU evaluation.
///
/// Combines multiple building types into a single array.
/// Populates `entities` and `inputs` vectors, clearing them first.
#[allow(clippy::cast_possible_truncation)]
pub fn extract_building_inputs(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    entities.clear();
    inputs.clear();

    extract_farms(world, entities, inputs);
    extract_housing(world, entities, inputs);
    extract_taverns(world, entities, inputs);
    extract_libraries(world, entities, inputs);
    extract_hospitals(world, entities, inputs);
    extract_designations(world, entities, inputs);
    extract_resource_items(world, entities, inputs);
    extract_corpses(world, entities, inputs);
    extract_wanted(world, entities, inputs);
    extract_suspects(world, entities, inputs);
    extract_fauna(world, entities, inputs);
}

fn extract_farms(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Farm)>();
    for (entity, pos, farm) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 0,
            capacity: farm.capacity as u32,
            occupied: farm.workers.len() as u32,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_housing(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Housing)>();
    for (entity, pos, housing) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 1,
            capacity: housing.capacity as u32,
            occupied: housing.residents.len() as u32,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_taverns(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Tavern)>();
    for (entity, pos, tavern) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 2,
            capacity: tavern.capacity as u32,
            occupied: tavern.visitors.len() as u32,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_libraries(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Library)>();
    for (entity, pos, _library) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 3,
            capacity: 5,
            occupied: 0,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_hospitals(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Hospital)>();
    for (entity, pos, _hospital) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 7,
            capacity: 10,
            occupied: 0,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_designations(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    use crate::layer1::designation::DesignationType;
    let mut query = world.query::<(Entity, &GridPosition, &Designation)>();
    for (entity, pos, designation) in query.iter(world) {
        let building_type = match designation.designation_type {
            DesignationType::Repair => 6,
            DesignationType::Tame => 11,
            _ => 4,
        };
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type,
            capacity: 1,
            occupied: 0,
            resource_has_room: 0,
            _padding: [0; 2],
        });
    }
}

fn extract_resource_items(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let resources = world
        .get_resource::<ColonyResources>()
        .cloned()
        .unwrap_or_default();

    let mut query = world.query::<(Entity, &GridPosition, &ResourceItem)>();
    for (entity, pos, item) in query.iter(world) {
        let has_room = match item.resource_type {
            ResourceType::Food => resources.food < resources.max_food,
            ResourceType::Wood => resources.wood < resources.max_wood,
            ResourceType::Stone => resources.stone < resources.max_stone,
            ResourceType::Ore => resources.ore < resources.max_ore,
            ResourceType::Metal => resources.metal < resources.max_metal,
            ResourceType::Planks => resources.planks < resources.max_planks,
            ResourceType::Blocks => resources.blocks < resources.max_blocks,
            ResourceType::Waste => resources.waste < resources.max_waste,
            ResourceType::Rations => resources.rations < resources.max_rations,
            ResourceType::Fuel => resources.fuel < resources.max_fuel,
            ResourceType::Alcohol => resources.alcohol < resources.max_alcohol,
            ResourceType::Scrap => resources.scrap < resources.max_scrap,
            ResourceType::Tools => resources.tools < resources.max_tools,
            ResourceType::BuildingPermit => {
                resources.building_permits < resources.max_building_permits
            }
            ResourceType::MemoryCore => resources.memory_cores < resources.max_memory_cores,
            ResourceType::VoidAle => resources.void_ale < resources.max_void_ale,
            ResourceType::HyperValuable => true,
            ResourceType::BiologicalWaste | ResourceType::NutrientPaste => false,
            ResourceType::Organs => resources.organs < resources.max_organs,
        };

        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 5,
            capacity: 1,
            occupied: 0,
            resource_has_room: u32::from(has_room),
            _padding: [0; 2],
        });
    }
}

fn extract_corpses(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let any_empty_grave = world.query::<&Grave>().iter(world).any(|g| !g.occupied);

    let mut query = world.query::<(Entity, &GridPosition, &Corpse)>();
    for (entity, pos, _corpse) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 9,
            capacity: 1,
            occupied: 0,
            resource_has_room: u32::from(any_empty_grave),
            _padding: [0; 2],
        });
    }
}

fn extract_wanted(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Wanted)>();
    for (entity, pos, _wanted) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 10,
            capacity: 1,
            occupied: 0,
            resource_has_room: 1,
            _padding: [0; 2],
        });
    }
}

fn extract_suspects(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    use crate::layer1::law::predictive_policing::Suspect;
    let mut query = world.query::<(Entity, &GridPosition, &Suspect)>();
    for (entity, pos, _suspect) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 13,
            capacity: 1,
            occupied: 0,
            resource_has_room: 1,
            _padding: [0; 2],
        });
    }
}

fn extract_fauna(
    world: &mut World,
    entities: &mut Vec<Entity>,
    inputs: &mut Vec<GpuBuildingInput>,
) {
    let mut query = world.query::<(Entity, &GridPosition, &Fauna)>();
    for (entity, pos, _fauna) in query.iter(world) {
        entities.push(entity);
        inputs.push(GpuBuildingInput {
            pos_x: pos.x,
            pos_y: pos.y,
            building_type: 12,
            capacity: 1,
            occupied: 0,
            resource_has_room: 1,
            _padding: [0; 2],
        });
    }
}

/// Extracts global state for GPU evaluation.
pub fn extract_global_state(
    world: &mut World,
    pop_count: u32,
    building_count: u32,
) -> GpuGlobalState {
    let switch_threshold = world
        .get_resource::<UtilityConfig>()
        .map_or(0.15, |c| c.switch_threshold);

    let resources = world
        .get_resource::<ColonyResources>()
        .cloned()
        .unwrap_or_default();

    let knowledge_full = u32::from(resources.knowledge >= resources.max_knowledge);

    let mut has_stockpile_query = world.query::<&Stockpile>();
    let has_stockpile = u32::from(has_stockpile_query.iter(world).next().is_some());

    GpuGlobalState {
        switch_threshold,
        knowledge_full,
        has_stockpile,
        pop_count,
        building_count,
        _padding: [0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::combat::Drafted;
    use crate::layer1::designation::{Designation, DesignationType};
    use crate::layer1::fauna::Fauna;
    use crate::layer1::utility_types::ActionType;
    use crate::setup::init_task_pools;

    #[test]
    fn test_extract_fauna_as_targets() {
        init_task_pools();
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        let fauna_entity = world
            .spawn((GridPosition { x: 10, y: 10 }, Fauna::default()))
            .id();

        let mut entities = Vec::new();
        let mut inputs = Vec::new();
        extract_building_inputs(&mut world, &mut entities, &mut inputs);

        // Should find the fauna
        let fauna_idx = entities.iter().position(|&e| e == fauna_entity);
        assert!(fauna_idx.is_some(), "Fauna should be extracted as target");

        let idx = fauna_idx.unwrap();
        assert_eq!(
            inputs[idx].building_type, 12,
            "Fauna should map to building_type 12"
        );
        assert_eq!(
            inputs[idx].resource_has_room, 1,
            "Fauna should be available target"
        );
    }

    #[test]
    fn test_extract_drafted_status() {
        init_task_pools();
        let mut world = World::new();
        world.insert_resource(UtilityConfig::default());

        // Drafted pop
        let _pop = world
            .spawn((
                GridPosition { x: 0, y: 0 },
                Needs::default(),
                UtilityWeights::default(),
                PopAction {
                    ticks_committed: 10, // Eligible for evaluation
                    ..Default::default()
                },
                Drafted,
            ))
            .id();

        let mut entities = Vec::new();
        let mut inputs = Vec::new();
        extract_pop_inputs(&mut world, &mut entities, &mut inputs);

        assert_eq!(entities.len(), 1);
        assert_eq!(
            inputs[0].drafted, 1,
            "Drafted pop should have drafted flag set to 1"
        );
    }

    #[test]
    fn test_gpu_pop_input_size() {
        // 2*i32 + 3*f32 + 3*f32 + 1*f32 + 1*u32
        // = 8 + 12 + 12 + 4 + 4 = 40 bytes
        assert_eq!(std::mem::size_of::<GpuPopInput>(), 40);
    }

    #[test]
    fn test_gpu_building_input_size() {
        // 8 fields * 4 bytes = 32
        assert_eq!(std::mem::size_of::<GpuBuildingInput>(), 32);
    }

    #[test]
    fn test_gpu_global_state_size() {
        // 8 fields * 4 bytes = 32
        assert_eq!(std::mem::size_of::<GpuGlobalState>(), 32);
    }

    #[test]
    fn test_gpu_pop_decision_size() {
        // 4 fields * 4 bytes = 16
        assert_eq!(std::mem::size_of::<GpuPopDecision>(), 16);
    }

    #[test]
    fn test_extract_pop_inputs() {
        init_task_pools();
        let mut world = World::new();
        world.insert_resource(UtilityConfig::default());

        // Pop that should be extracted (ticks_committed >= evaluation_interval=1)
        let pop1 = world
            .spawn((
                GridPosition { x: 5, y: 10 },
                Needs {
                    hunger: 0.7,
                    rest: 0.5,
                    leisure: 0.9,
                    hygiene: 0.8,
                },
                UtilityWeights {
                    distance_weight: 1.2,
                    availability_weight: 0.8,
                },
                PopAction {
                    current: ActionType::SatisfyHunger,
                    current_utility: 0.75,
                    ticks_committed: 3,
                },
            ))
            .id();

        // Pop that should NOT be extracted (ticks_committed < evaluation_interval)
        let _pop2 = world
            .spawn((
                GridPosition { x: 1, y: 1 },
                Needs::default(),
                UtilityWeights::default(),
                PopAction {
                    current: ActionType::Idle,
                    current_utility: 0.0,
                    ticks_committed: 0,
                },
            ))
            .id();

        let mut entities = Vec::new();
        let mut inputs = Vec::new();
        extract_pop_inputs(&mut world, &mut entities, &mut inputs);

        assert_eq!(entities.len(), 1);
        assert_eq!(inputs.len(), 1);
        assert_eq!(entities[0], pop1);

        let input = &inputs[0];
        assert_eq!(input.pos_x, 5);
        assert_eq!(input.pos_y, 10);
        assert!((input.hunger - 0.7).abs() < f32::EPSILON);
        assert!((input.rest - 0.5).abs() < f32::EPSILON);
        assert!((input.leisure - 0.9).abs() < f32::EPSILON);
        assert!((input.distance_weight - 1.2).abs() < f32::EPSILON);
        assert!((input.availability_weight - 0.8).abs() < f32::EPSILON);
        assert!((input.current_utility - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_extract_building_inputs() {
        init_task_pools();
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        // Farm
        let farm_entity = world
            .spawn((
                GridPosition { x: 1, y: 2 },
                Farm {
                    capacity: 3,
                    workers: vec![],
                    ..Default::default()
                },
            ))
            .id();

        // Housing
        let housing_entity = world
            .spawn((
                GridPosition { x: 3, y: 4 },
                Housing {
                    capacity: 4,
                    residents: vec![],
                },
            ))
            .id();

        // Tavern
        let tavern_entity = world
            .spawn((
                GridPosition { x: 5, y: 6 },
                Tavern {
                    capacity: 5,
                    visitors: vec![],
                },
            ))
            .id();

        // Library
        let library_entity = world.spawn((GridPosition { x: 7, y: 8 }, Library)).id();

        // Designation
        let designation_entity = world
            .spawn((
                GridPosition { x: 9, y: 10 },
                Designation {
                    designation_type: DesignationType::Mine,
                },
            ))
            .id();

        // ResourceItem with room
        let resource_entity = world
            .spawn((
                GridPosition { x: 11, y: 12 },
                ResourceItem {
                    resource_type: ResourceType::Stone,
                    amount: 1.0,
                },
            ))
            .id();

        // Corpse with grave available
        let corpse_entity = world
            .spawn((
                GridPosition { x: 13, y: 14 },
                Corpse {
                    name: "Dead".into(),
                    decay: 0.0,
                },
            ))
            .id();

        // Spawn an empty grave
        world.spawn(Grave {
            occupied: false,
            corpse_name: None,
        });

        let mut entities = Vec::new();
        let mut inputs = Vec::new();
        extract_building_inputs(&mut world, &mut entities, &mut inputs);

        assert_eq!(entities.len(), 7);
        assert_eq!(inputs.len(), 7);

        // Verify farm
        let farm_idx = entities.iter().position(|&e| e == farm_entity).unwrap();
        assert_eq!(inputs[farm_idx].building_type, 0);

        // Verify housing
        let housing_idx = entities.iter().position(|&e| e == housing_entity).unwrap();
        assert_eq!(inputs[housing_idx].building_type, 1);

        // Verify tavern
        let tavern_idx = entities.iter().position(|&e| e == tavern_entity).unwrap();
        assert_eq!(inputs[tavern_idx].building_type, 2);

        // Verify library
        let library_idx = entities.iter().position(|&e| e == library_entity).unwrap();
        assert_eq!(inputs[library_idx].building_type, 3);

        // Verify designation
        let designation_idx = entities
            .iter()
            .position(|&e| e == designation_entity)
            .unwrap();
        assert_eq!(inputs[designation_idx].building_type, 4);

        // Verify resource item
        let resource_idx = entities.iter().position(|&e| e == resource_entity).unwrap();
        assert_eq!(inputs[resource_idx].building_type, 5);

        // Verify corpse
        let corpse_idx = entities.iter().position(|&e| e == corpse_entity).unwrap();
        assert_eq!(inputs[corpse_idx].building_type, 9);
        assert_eq!(inputs[corpse_idx].resource_has_room, 1);
    }

    #[test]
    fn test_extract_global_state() {
        init_task_pools();
        let mut world = World::new();

        world.insert_resource(UtilityConfig {
            switch_threshold: 0.2,
            ..Default::default()
        });

        world.insert_resource(ColonyResources {
            knowledge: 100.0,
            max_knowledge: 100.0,
            ..Default::default()
        });

        // Spawn a stockpile
        world.spawn(Stockpile::default());

        let state = extract_global_state(&mut world, 10, 25);

        assert!((state.switch_threshold - 0.2).abs() < f32::EPSILON);
        assert_eq!(state.knowledge_full, 1);
        assert_eq!(state.has_stockpile, 1);
        assert_eq!(state.pop_count, 10);
        assert_eq!(state.building_count, 25);
    }
}

/// Persistent buffers for GPU reuse.
#[derive(Resource, Default)]
pub struct GpuPersistentBuffers {
    /// Persistent storage buffer for pop inputs.
    pub pop_buffer: Option<wgpu::Buffer>,
    /// Capacity of the pop buffer in bytes.
    pub pop_capacity: u64,
    /// Persistent storage buffer for building inputs.
    pub building_buffer: Option<wgpu::Buffer>,
    /// Capacity of the building buffer in bytes.
    pub building_capacity: u64,
    /// Persistent uniform buffer for global state.
    pub global_buffer: Option<wgpu::Buffer>,
    /// Capacity of the global buffer in bytes.
    pub global_capacity: u64,
    /// Persistent storage buffer for decisions.
    pub decision_buffer: Option<wgpu::Buffer>,
    /// Capacity of the decision buffer in bytes.
    pub decision_capacity: u64,
    /// Persistent staging buffer for readback.
    pub staging_buffer: Option<wgpu::Buffer>,
    /// Capacity of the staging buffer in bytes.
    pub staging_capacity: u64,
    /// Cached bind group, recreated only when buffers change.
    pub bind_group: Option<wgpu::BindGroup>,
}
