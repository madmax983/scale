use crate::layer1::items::Equipment;
use crate::layer1::map::GridPosition;
use crate::layer1::resources::ColonyResources;
use crate::layer1::utility_eval_types::*;
use crate::layer1::utility_types::UtilityWeights;
use bevy_ecs::prelude::*;

/// Evaluates if a pop should fetch a tool.
#[must_use]
pub(crate) fn evaluate_fetch_tool(
    pop_pos: GridPosition,
    equipment: &Equipment,
    resources: &ColonyResources,
    stockpiles: &[ScorableCandidate],
) -> Option<(f32, Entity)> {
    // If already has a tool, no need to fetch
    if equipment.tool.is_some() {
        return None;
    }

    // If no tools available in colony, can't fetch
    if resources.tools < 1.0 {
        return None;
    }

    let weights = UtilityWeights::default();
    // Spec 392: a pop with no tool (in production a broken tool has
    // already been removed, so missing == broken) strongly prioritizes
    // fetching a replacement from the Smithy-fed stockpile pool.
    evaluate_candidates(
        pop_pos,
        &weights,
        stockpiles,
        crate::layer1::tool_economy::REPLACEMENT_TOOL_UTILITY,
    )
}
