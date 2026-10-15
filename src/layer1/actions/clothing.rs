use crate::layer1::map::GridPosition;
use crate::layer1::resources::ColonyResources;
use crate::layer1::utility_eval_types::*;
use crate::layer1::utility_types::UtilityWeights;
use bevy_ecs::prelude::*;

/// Evaluates if a pop should fetch clothing.
///
/// # The "Shiver Factor"
///
/// Pops don't just upgrade clothes for fashion; they do it to survive.
/// This evaluator checks:
///
/// 1.  **Nakedness:** If `insulation == 0.0`, urgency is high (0.95).
/// 2.  **Hypothermia Risk:** If the local temperature is below what the current
///     clothes can handle (`insulation * -30 + 10`), urgency becomes critical (0.99),
///     prompting an immediate upgrade even if they are already dressed.
#[must_use]
pub(crate) fn evaluate_fetch_clothing(
    pop_pos: GridPosition,
    current_insulation: f32,
    resources: &ColonyResources,
    stockpiles: &[ScorableCandidate],
    temperature_grid: Option<&crate::layer1::temperature::TemperatureGrid>,
) -> Option<(f32, Entity)> {
    let mut score = 0.0;

    if current_insulation == 0.0 {
        score = 0.95; // Need clothes!
    } else if let Some(grid) = temperature_grid {
        if pop_pos.x >= 0 && pop_pos.y >= 0 {
            // Check if freezing despite clothes
            #[allow(clippy::cast_sign_loss)]
            let temp = grid.get(pop_pos.x as usize, pop_pos.y as usize);
            let safe_temp = current_insulation.mul_add(-30.0, 10.0);
            if temp < safe_temp {
                // If insulation is already high (e.g. >= 2.0), fetching won't help unless we have super-parka.
                if current_insulation < 2.0 {
                    score = 0.99; // Upgrade needed immediately!
                }
            }
        }
    }

    if score == 0.0 {
        return None;
    }

    // If no clothing available in colony, can't fetch
    if resources.clothing < 1.0 {
        return None;
    }

    // Use evaluate_candidates with default weights (clothing is basic need, traits matter less)
    let weights = UtilityWeights::default();
    evaluate_candidates(pop_pos, &weights, stockpiles, score)
}

/// Evaluates if a pop should fetch an EVA suit (Spec 1384 §4.4).
///
/// Scores 0.97 (the FetchClothing family, above Work's 0.5) when all
/// hold: the pop wears no suit; a suit is obtainable (colony pool > 0,
/// targeted at a stockpile, or a loose physical suit exists — loose
/// suits are targeted first, they are strictly better and already
/// physical); and an exterior commitment pends (a Work/Repair/Explore
/// target below the suffocation threshold, supplied by the caller as
/// `exterior_pending`) or the pop is already standing in vacuum
/// unsuited (`pop_pressure` below threshold — the emergency dash).
///
/// Panicked pops never reach this evaluator: `collect_pop_data`
/// excludes `Panic` pops from utility evaluation entirely, so a
/// panicking pop will not stop to dress. That is intended.
#[must_use]
pub(crate) fn evaluate_fetch_suit(
    pop_pos: GridPosition,
    wears_suit: bool,
    resources: &ColonyResources,
    stockpiles: &[ScorableCandidate],
    loose_suits: &[ScorableCandidate],
    exterior_pending: bool,
    pop_pressure: f32,
) -> Option<(f32, Entity)> {
    if wears_suit {
        return None;
    }
    let emergency = pop_pressure < crate::layer1::pressure::SUFFOCATION_PRESSURE_THRESHOLD;
    if !exterior_pending && !emergency {
        return None;
    }
    // Loose physical suits first (strictly better, already materialized).
    if !loose_suits.is_empty() {
        let weights = UtilityWeights::default();
        if let Some(found) = evaluate_candidates(pop_pos, &weights, loose_suits, 0.97) {
            return Some(found);
        }
    }
    if resources.eva_suits < 1.0 {
        return None;
    }
    let weights = UtilityWeights::default();
    evaluate_candidates(pop_pos, &weights, stockpiles, 0.97)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::resources::ColonyResources;
    use crate::layer1::temperature::TemperatureGrid;
    use crate::layer1::utility_eval_types::ScorableCandidate;
    use bevy_ecs::prelude::Entity;

    #[test]
    fn should_return_none_when_resources_clothing_is_zero() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let resources = ColonyResources {
            clothing: 0.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        let result = evaluate_fetch_clothing(pop_pos, 0.0, &resources, &candidates, None);
        assert_eq!(result, None, "Expected None when no clothing is available");
    }

    #[test]
    fn should_return_some_when_naked_and_clothing_available() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let resources = ColonyResources {
            clothing: 5.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        let result = evaluate_fetch_clothing(pop_pos, 0.0, &resources, &candidates, None);
        assert!(
            result.is_some(),
            "Expected Some when naked and clothing is available"
        );
        let (score, _) = result.unwrap();
        assert!(score > 0.0);
    }

    #[test]
    fn should_return_none_when_dressed_and_no_temperature_grid() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let resources = ColonyResources {
            clothing: 5.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        // Insulation > 0.0 and no temperature_grid
        let result = evaluate_fetch_clothing(pop_pos, 1.0, &resources, &candidates, None);
        assert_eq!(result, None, "Expected None when dressed and no temp grid");
    }

    #[test]
    fn should_return_some_when_freezing_despite_clothes() {
        let pop_pos = GridPosition { x: 5, y: 5 };
        let resources = ColonyResources {
            clothing: 5.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        let mut grid = TemperatureGrid::new(10, 10, 0.0);
        // Current insulation is 1.0, safe temp is 1.0 * -30.0 + 10.0 = -20.0
        // Set temp to -25.0 (below safe temp)
        grid.set(5, 5, -25.0);

        let result = evaluate_fetch_clothing(pop_pos, 1.0, &resources, &candidates, Some(&grid));
        assert!(
            result.is_some(),
            "Expected Some when freezing despite clothes"
        );
        let (score, _) = result.unwrap();
        assert!(score > 0.0);
    }

    #[test]
    fn should_return_none_when_freezing_but_insulation_already_maxed() {
        let pop_pos = GridPosition { x: 5, y: 5 };
        let resources = ColonyResources {
            clothing: 5.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        let mut grid = TemperatureGrid::new(10, 10, 0.0);
        // Current insulation is 2.5 (>= 2.0 max).
        // Safe temp is 2.5 * -30.0 + 10.0 = -65.0
        // Set temp to -70.0 (below safe temp, but insulation is maxed out)
        grid.set(5, 5, -70.0);

        let result = evaluate_fetch_clothing(pop_pos, 2.5, &resources, &candidates, Some(&grid));
        assert_eq!(
            result, None,
            "Expected None when insulation is already high (>= 2.0)"
        );
    }

    #[test]
    fn should_return_none_when_temperature_is_safe() {
        let pop_pos = GridPosition { x: 5, y: 5 };
        let resources = ColonyResources {
            clothing: 5.0,
            ..Default::default()
        };
        let candidates = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 1, y: 0 },
        )];

        let mut grid = TemperatureGrid::new(10, 10, 0.0);
        // Current insulation is 1.0, safe temp is 1.0 * -30.0 + 10.0 = -20.0
        // Set temp to 0.0 (safe)
        grid.set(5, 5, 0.0);

        let result = evaluate_fetch_clothing(pop_pos, 1.0, &resources, &candidates, Some(&grid));
        assert_eq!(result, None, "Expected None when temperature is safe");
    }

    // --- Spec 1384: FetchSuit evaluator ---

    fn suit_resources(pool: f32) -> ColonyResources {
        ColonyResources {
            eva_suits: pool,
            ..Default::default()
        }
    }

    #[test]
    fn fetch_suit_scores_when_exterior_work_pends() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let stockpiles = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 0, y: 0 },
        )];
        let result = evaluate_fetch_suit(
            pop_pos,
            false,
            &suit_resources(2.0),
            &stockpiles,
            &[],
            true,
            1.0,
        );
        let (score, _) = result.expect("exterior work + pool suit must score");
        assert!(score >= 0.9, "FetchSuit score {score} below 0.9 at hand");
    }

    #[test]
    fn fetch_suit_none_when_all_interior() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let stockpiles = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 0, y: 0 },
        )];
        assert_eq!(
            evaluate_fetch_suit(pop_pos, false, &suit_resources(2.0), &stockpiles, &[], false, 1.0),
            None
        );
    }

    #[test]
    fn fetch_suit_none_when_already_suited_or_no_suit_anywhere() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let stockpiles = vec![ScorableCandidate::new(
            Entity::from_raw(1),
            GridPosition { x: 0, y: 0 },
        )];
        assert_eq!(
            evaluate_fetch_suit(pop_pos, true, &suit_resources(2.0), &stockpiles, &[], true, 1.0),
            None,
            "already suited"
        );
        assert_eq!(
            evaluate_fetch_suit(pop_pos, false, &suit_resources(0.0), &stockpiles, &[], true, 1.0),
            None,
            "no suit exists anywhere"
        );
    }

    #[test]
    fn fetch_suit_targets_loose_suit_first_and_emergency_dash() {
        let pop_pos = GridPosition { x: 0, y: 0 };
        let loose = vec![ScorableCandidate::new(
            Entity::from_raw(9),
            GridPosition { x: 0, y: 0 },
        )];
        // No pool, no pending work — but the pop stands in vacuum:
        // the emergency dash targets the loose suit.
        let result = evaluate_fetch_suit(pop_pos, false, &suit_resources(0.0), &[], &loose, false, 0.0);
        let (_, target) = result.expect("loose suit must be targeted");
        assert_eq!(target, Entity::from_raw(9));
    }
}
