use crate::layer1::map::GridPosition;
use crate::layer1::utility_types::ActionType;
use bevy_ecs::prelude::*;
use std::collections::VecDeque;

/// Component indicating a pop is moving toward a target.
#[derive(Component, Debug)]
pub struct MovementTarget {
    /// The entity being targeted (farm, housing, or designation).
    pub target_entity: Entity,
    /// The grid position of the target.
    pub target_position: GridPosition,
    /// The action type this movement is for.
    pub for_action: ActionType,
}

/// Marker component indicating a pop has arrived at its target.
#[derive(Component, Debug)]
pub struct AtTarget;

/// Item 94: a committed detour around an obstacle that greedy
/// movement cannot pass.
///
/// When both greedy candidate steps are unwalkable (the aligned-axis
/// wedge: `calculate_next_positions` offers only ONE candidate when
/// dx == 0 or dy == 0), movement plans a full shortest walkable path
/// and stores the remaining steps here. While this component is
/// present and its target still matches the pop's `MovementTarget`,
/// movement follows the stored steps instead of re-deciding greedily
/// each tick — a bare sidestep without this commitment oscillates,
/// because greedy horizontal preference steps straight back into the
/// blocked column/row from the sidestep tile. The component is
/// dropped on arrival, on a new plan, when its target no longer
/// matches, or when a stored step becomes unwalkable (movement then
/// replans).
#[derive(Component, Debug)]
pub struct DetourPath {
    /// The `MovementTarget` position this detour was planned for.
    pub target: GridPosition,
    /// Remaining steps, front = next tile to enter. Excludes the
    /// pop's position at plan time; last step is the goal tile.
    pub steps: VecDeque<GridPosition>,
}

#[derive(Component, Debug, Clone)]
pub struct HabituatedRoute {
    pub path: Vec<GridPosition>,
    pub urgency: f32,
    pub frustration: u32,
    pub last_pos: Option<GridPosition>,
}
