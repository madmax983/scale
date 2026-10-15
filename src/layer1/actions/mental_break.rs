use crate::layer1::stress::BreakdownType;
use crate::layer1::unrest::{MentalBreakType, MentalState};
use crate::layer1::utility_eval_types::*;
use crate::layer1::utility_types::manhattan_distance;
use crate::layer1::utility_types::ActionType;
use bevy_ecs::prelude::*;

/// Evaluates actions for a pop undergoing a mental break.
pub(crate) fn evaluate_mental_break(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
) -> Option<(ActionType, f32, Option<Entity>)> {
    if let Some(breakdown) = data.breakdown {
        let best_utility = 100.0;
        let mut best_target = None;

        let best_action = match breakdown.breakdown_type {
            BreakdownType::Dazing => ActionType::Daze,
            BreakdownType::SadWander => ActionType::SadWander,
            BreakdownType::HideInRoom => ActionType::HideInRoom,
            BreakdownType::BingeEating => {
                find_food_target(data, buffer, &mut best_target);
                ActionType::Binge
            }
            BreakdownType::FireStarting => {
                find_structure_target(data, buffer, &mut best_target);
                ActionType::FireStarting
            }
        };
        return Some((best_action, best_utility, best_target));
    }

    if let Some(MentalState::Broken(break_type)) = data.mental_state {
        let best_utility = 100.0;
        let mut best_target = None;

        let best_action = match break_type {
            MentalBreakType::Vandalize => {
                find_structure_target(data, buffer, &mut best_target);
                ActionType::Vandalize
            }
            MentalBreakType::Binge => {
                find_food_target(data, buffer, &mut best_target);
                ActionType::Binge
            }
            MentalBreakType::Daze => ActionType::Daze,
            MentalBreakType::Sleepwalking => {
                // Sleepwalkers just wander. Target is assigned by assign_sleepwalk_target_system.
                best_target = None;
                ActionType::Sleepwalking
            }
            MentalBreakType::RealityCollapse => {
                find_structure_target(data, buffer, &mut best_target);
                ActionType::RealityCollapse
            }
            MentalBreakType::Violent => {
                find_structure_target(data, buffer, &mut best_target);
                ActionType::Vandalize
            }
        };

        return Some((best_action, best_utility, best_target));
    }

    None
}

fn find_structure_target(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
    best_target: &mut Option<Entity>,
) {
    let mut closest_dist = i32::MAX;
    let mut closest_target = None;

    for structure in &buffer.all_structures {
        if structure.entity == data.entity {
            continue;
        }
        let dist = manhattan_distance(&data.pos, &structure.pos);
        if dist < closest_dist {
            closest_dist = dist;
            closest_target = Some(structure.entity);
        }
    }
    *best_target = closest_target;
}

fn find_food_target(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
    best_target: &mut Option<Entity>,
) {
    let mut closest_dist = i32::MAX;
    let mut closest_target = None;

    for stockpile in &buffer.stockpiles {
        let dist = manhattan_distance(&data.pos, &stockpile.pos);
        if dist < closest_dist {
            closest_dist = dist;
            closest_target = Some(stockpile.entity);
        }
    }

    for farm in &buffer.farms {
        let dist = manhattan_distance(&data.pos, &farm.pos);
        if dist < closest_dist {
            closest_dist = dist;
            closest_target = Some(farm.entity);
        }
    }
    *best_target = closest_target;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;

    use crate::layer1::stress::{Breakdown, BreakdownType};
    use crate::layer1::unrest::{MentalBreakType, MentalState};
    use crate::layer1::utility_eval_types::{PopEvalData, ScorableCandidate, UtilityAIBuffer};
    use crate::layer1::utility_types::ActionType;
    use bevy_ecs::prelude::Entity;

    fn default_pop_eval_data() -> PopEvalData {
        PopEvalData::test_instance()
    }

    fn default_buffer() -> UtilityAIBuffer {
        UtilityAIBuffer {
            pop_data: vec![],
            results: vec![],
            farms: vec![],
            housing: vec![],
            taverns: vec![],
            libraries: vec![],
            refining: vec![],
            work_designations: vec![],
            repair_designations: vec![],
            tame_designations: vec![],
            items: vec![],
            item_entities: vec![],
            stockpiles: vec![],
            stockpile_positions: bevy_utils::HashSet::new(),
            anomalies: vec![],
            hospitals: vec![],
            corpses: vec![],
            graves: vec![],
            unpollinated_crops: vec![],
            repair_structures: vec![],
            wanted_criminals: vec![],
            suspects: vec![],
            offices: vec![],
            walls: vec![],
            enemies: vec![],
            all_structures: vec![],
            showers: vec![],
            hum_sources: vec![],
            gene_banks: vec![],
            residues: vec![],
            cleaning_targets: vec![],
            sanctuaries: vec![],
            mobs: vec![],
            loose_eva_suits: vec![],
            exterior_suit_pending: false,
            pop_pressures: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_evaluate_mental_break_no_break() {
        let data = default_pop_eval_data();
        let buffer = default_buffer();

        let result = evaluate_mental_break(&data, &buffer);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_mental_break_with_breakdown_variants() {
        let mut buffer = default_buffer();
        let structure = Entity::from_raw(10);
        buffer.all_structures.push(ScorableCandidate::new(
            structure,
            GridPosition { x: 5, y: 0 },
        ));

        let stockpile = Entity::from_raw(20);
        buffer.stockpiles.push(ScorableCandidate::new(
            stockpile,
            GridPosition { x: 10, y: 0 },
        ));

        let cases = vec![
            (BreakdownType::Dazing, ActionType::Daze, None),
            (BreakdownType::SadWander, ActionType::SadWander, None),
            (BreakdownType::HideInRoom, ActionType::HideInRoom, None),
            (
                BreakdownType::BingeEating,
                ActionType::Binge,
                Some(stockpile),
            ),
            (
                BreakdownType::FireStarting,
                ActionType::FireStarting,
                Some(structure),
            ),
        ];

        for (break_type, expected_action, expected_target) in cases {
            let mut data = default_pop_eval_data();
            data.breakdown = Some(Breakdown {
                breakdown_type: break_type,
                duration_remaining: 100,
            });

            let result = evaluate_mental_break(&data, &buffer);
            assert!(result.is_some());
            let (action, utility, target) = result.unwrap();

            assert_eq!(action, expected_action);
            assert_eq!(utility, 100.0);
            assert_eq!(target, expected_target);
        }
    }

    #[test]
    fn test_evaluate_mental_break_with_mental_state_variants() {
        let mut buffer = default_buffer();
        let structure = Entity::from_raw(10);
        buffer.all_structures.push(ScorableCandidate::new(
            structure,
            GridPosition { x: 5, y: 0 },
        ));

        let farm = Entity::from_raw(30);
        buffer
            .farms
            .push(ScorableCandidate::new(farm, GridPosition { x: 15, y: 0 }));

        let cases = vec![
            (
                MentalBreakType::Vandalize,
                ActionType::Vandalize,
                Some(structure),
            ),
            (MentalBreakType::Binge, ActionType::Binge, Some(farm)),
            (MentalBreakType::Daze, ActionType::Daze, None),
            (
                MentalBreakType::Sleepwalking,
                ActionType::Sleepwalking,
                None,
            ),
            (
                MentalBreakType::RealityCollapse,
                ActionType::RealityCollapse,
                Some(structure),
            ),
            (
                MentalBreakType::Violent,
                ActionType::Vandalize,
                Some(structure),
            ),
        ];

        for (break_type, expected_action, expected_target) in cases {
            let mut data = default_pop_eval_data();
            data.mental_state = Some(MentalState::Broken(break_type));

            let result = evaluate_mental_break(&data, &buffer);
            assert!(result.is_some());
            let (action, utility, target) = result.unwrap();

            assert_eq!(action, expected_action);
            assert_eq!(utility, 100.0);
            assert_eq!(target, expected_target);
        }
    }

    #[test]
    fn test_target_selection_logic_structure() {
        let mut data = default_pop_eval_data();
        let mut buffer = default_buffer();

        data.breakdown = Some(Breakdown {
            breakdown_type: BreakdownType::FireStarting,
            duration_remaining: 100,
        });

        // Far structure
        let far_struct = Entity::from_raw(10);
        buffer.all_structures.push(ScorableCandidate::new(
            far_struct,
            GridPosition { x: 10, y: 10 },
        ));

        // Close structure
        let close_struct = Entity::from_raw(11);
        buffer.all_structures.push(ScorableCandidate::new(
            close_struct,
            GridPosition { x: 1, y: 1 },
        ));

        // Structure is pop itself (should be ignored)
        let self_struct = data.entity;
        buffer.all_structures.push(ScorableCandidate::new(
            self_struct,
            GridPosition { x: 0, y: 0 },
        ));

        let result = evaluate_mental_break(&data, &buffer);
        assert!(result.is_some());
        let (action, _, target) = result.unwrap();

        assert_eq!(action, ActionType::FireStarting);
        assert_eq!(target, Some(close_struct));
    }

    #[test]
    fn test_target_selection_logic_food() {
        let mut data = default_pop_eval_data();
        let mut buffer = default_buffer();

        data.mental_state = Some(MentalState::Broken(MentalBreakType::Binge));

        // Far stockpile
        let far_stockpile = Entity::from_raw(20);
        buffer.stockpiles.push(ScorableCandidate::new(
            far_stockpile,
            GridPosition { x: 20, y: 0 },
        ));

        // Close farm
        let close_farm = Entity::from_raw(30);
        buffer.farms.push(ScorableCandidate::new(
            close_farm,
            GridPosition { x: 0, y: 5 },
        ));

        let result = evaluate_mental_break(&data, &buffer);
        assert!(result.is_some());
        let (action, _, target) = result.unwrap();

        assert_eq!(action, ActionType::Binge);
        assert_eq!(target, Some(close_farm));
    }

    #[test]
    fn test_edge_case_no_targets_exist() {
        let mut data = default_pop_eval_data();
        let buffer = default_buffer(); // Empty buffers

        data.mental_state = Some(MentalState::Broken(MentalBreakType::Vandalize));

        let result = evaluate_mental_break(&data, &buffer);
        assert!(result.is_some());
        let (action, _, target) = result.unwrap();

        assert_eq!(action, ActionType::Vandalize);
        assert_eq!(target, None); // Should default to None since no structures exist
    }
}
