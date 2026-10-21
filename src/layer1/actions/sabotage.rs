use crate::layer1::utility_eval_types::*;
use crate::layer1::utility_types::{manhattan_distance, ActionType};
use bevy_ecs::prelude::*;

pub(crate) fn evaluate_sabotage(
    data: &PopEvalData,
    buffer: &UtilityAIBuffer,
) -> Option<(ActionType, f32, Option<Entity>)> {
    // Lane 1 (unchanged): nostalgia at full heat.
    // Lane 2 (Spec 1387 §4.5.3): a Bitter pop cold-sabotages advanced
    // tech at 60 + 40×total — below the 100.0 of an active breakdown;
    // sabotage is cold, breaks are hot.
    let best_utility = if data.is_nostalgic {
        100.0
    } else if data.dissatisfaction_total
        >= crate::layer1::social::dissatisfaction::BAND_BITTER
    {
        60.0 + 40.0 * data.dissatisfaction_total
    } else {
        return None;
    };
    let mut best_target = None;
    let mut closest_dist = i32::MAX;

    for structure in &buffer.all_structures {
        if structure.entity == data.entity {
            continue;
        }

        if structure.is_advanced_tech {
            let dist = manhattan_distance(&data.pos, &structure.pos);
            if dist < closest_dist {
                closest_dist = dist;
                best_target = Some(structure.entity);
            }
        }
    }

    if best_target.is_some() {
        return Some((ActionType::Sabotage, best_utility, best_target));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;

    use crate::layer1::utility_eval_types::{PopEvalData, ScorableCandidate, UtilityAIBuffer};
    use crate::layer1::utility_types::ActionType;

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
            stockpile_positions: bevy_utils::HashSet::default(),
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
    fn test_evaluate_sabotage_no_nostalgia() {
        let data = default_pop_eval_data();
        let buffer = default_buffer();

        let result = evaluate_sabotage(&data, &buffer);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_sabotage_with_nostalgia() {
        let mut data = default_pop_eval_data();
        data.is_nostalgic = true;

        let mut buffer = default_buffer();

        let structure1 = Entity::from_raw(10);
        let mut cand1 = ScorableCandidate::new(structure1, GridPosition { x: 5, y: 0 });
        cand1.is_advanced_tech = false;

        let structure2 = Entity::from_raw(20);
        let mut cand2 = ScorableCandidate::new(structure2, GridPosition { x: 10, y: 0 });
        cand2.is_advanced_tech = true;

        let structure3 = Entity::from_raw(30);
        let mut cand3 = ScorableCandidate::new(structure3, GridPosition { x: 15, y: 0 });
        cand3.is_advanced_tech = true;

        buffer.all_structures.push(cand1);
        buffer.all_structures.push(cand2);
        buffer.all_structures.push(cand3);

        let result = evaluate_sabotage(&data, &buffer);
        assert!(result.is_some());
        let (action, utility, target) = result.unwrap();

        assert_eq!(action, ActionType::Sabotage);
        assert_eq!(utility, 100.0);
        assert_eq!(target, Some(structure2)); // Closer advanced tech
    }
}
