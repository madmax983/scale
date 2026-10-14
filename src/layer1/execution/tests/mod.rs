//! Execution System Tests.
//!
//! Shared test utilities and setups for the layer1 execution mechanics.
//!
//! Layer 1 Execution Tests.
//!
//! Layer 1 Execution Tests.
//!
use crate::layer1::building::OccupiedTiles;
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::erosion::ErosionGrid;
use crate::layer1::items::UnequipEvent;
use crate::layer1::resources::ColonyResources;
use crate::layer1::structural_integrity::RoofGrid;
use crate::layer1::taboo::TabooState;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

pub fn setup_world() -> World {
    crate::setup::init_task_pools();
    let mut world = World::new();
    let tiles = vec![TerrainType::Grass; 100];
    world.insert_resource(TerrainGrid {
        width: 10,
        height: 10,
        tiles,
    });
    world.insert_resource(ErosionGrid::new(10, 10));
    world.insert_resource(ColonyResources::default());
    world.insert_resource(RoofGrid::new(10, 10));
    world.insert_resource(SimulationTime::default());
    world.insert_resource(DayNightCycle::default());
    world.insert_resource(OccupiedTiles::default());
    world.insert_resource(TabooState::default());
    world.init_resource::<bevy_ecs::event::Events<UnequipEvent>>();
    world
}

pub mod arrival_tests;
pub mod combat_tests;
pub mod movement_tests;
pub mod schedule_work_tests;
pub mod work_tests;
