pub mod deorbit;
pub mod mega_event;

use crate::layer1::core::map::GridPosition;
use bevy_ecs::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisasterType {
    Fissure,
    MegaHurricane,
}

#[derive(Event, Debug, Clone)]
pub struct DisasterEvent {
    pub position: GridPosition,
    pub disaster_type: DisasterType,
}
