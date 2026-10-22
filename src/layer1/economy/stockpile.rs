use crate::layer1::resources::{
    ColonyResources, BASE_MAX_BIOLOGICAL_WASTE, BASE_MAX_FOOD, BASE_MAX_NUTRIENT_PASTE,
    BASE_MAX_STONE, BASE_MAX_WOOD,
};
use bevy_ecs::prelude::*;

/// Component that increases the maximum resource capacity of the colony.
#[derive(Component)]
pub struct Stockpile {
    /// Bonus to max food capacity.
    pub food_bonus: f32,
    /// Bonus to max wood capacity.
    pub wood_bonus: f32,
    /// Bonus to max stone capacity.
    pub stone_bonus: f32,
    /// Bonus to max waste capacity.
    pub waste_bonus: f32,
}

impl Default for Stockpile {
    fn default() -> Self {
        Self {
            food_bonus: 0.0,
            wood_bonus: 100.0,
            stone_bonus: 100.0,
            waste_bonus: 0.0,
        }
    }
}

/// System that calculates and updates the colony's resource caps based on existing stockpiles.
pub fn update_resource_caps_system(
    query: Query<&Stockpile>,
    mut resources: ResMut<ColonyResources>,
) {
    let mut total_food_bonus = 0.0;
    let mut total_wood_bonus = 0.0;
    let mut total_stone_bonus = 0.0;
    let mut total_waste_bonus = 0.0;

    for stockpile in &query {
        total_food_bonus += stockpile.food_bonus;
        total_wood_bonus += stockpile.wood_bonus;
        total_stone_bonus += stockpile.stone_bonus;
        total_waste_bonus += stockpile.waste_bonus;
    }

    resources.max_food = BASE_MAX_FOOD + total_food_bonus;
    resources.max_wood = BASE_MAX_WOOD + total_wood_bonus;
    resources.max_stone = BASE_MAX_STONE + total_stone_bonus;
    resources.max_waste = total_waste_bonus; // No base capacity for waste
    // Item 118: wire the Recycler ledger recipe's caps, which no system
    // ever raised from 0.0 before. Biological waste is stored with the
    // waste stream, so the waste bonus (Landfill: 100) raises its cap
    // over a base capacity; nutrient paste is a foodstuff (it counts in
    // `total_food`), so the food bonus raises its cap over a base.
    resources.max_biological_waste = BASE_MAX_BIOLOGICAL_WASTE + total_waste_bonus;
    resources.max_nutrient_paste = BASE_MAX_NUTRIENT_PASTE + total_food_bonus;

    resources.food = resources.food.min(resources.max_food);
    resources.wood = resources.wood.min(resources.max_wood);
    resources.stone = resources.stone.min(resources.max_stone);
    resources.waste = resources.waste.min(resources.max_waste);
    resources.biological_waste = resources.biological_waste.min(resources.max_biological_waste);
    resources.nutrient_paste = resources.nutrient_paste.min(resources.max_nutrient_paste);
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::resources::ColonyResources;
    use crate::layer1::stockpile::{update_resource_caps_system, Stockpile};
    use bevy_ecs::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_colony_resources_caps_default() {
        let resources = ColonyResources::default();
        // Default small capacity for survival
        assert_eq!(resources.max_food, 50.0);
        assert_eq!(resources.max_wood, 50.0);
        assert_eq!(resources.max_stone, 20.0);
    }

    #[test]
    fn test_add_resource_clamped_to_max() {
        let mut resources = ColonyResources {
            wood: 40.0,
            max_wood: 50.0,
            ..Default::default()
        };

        // Add 20, should cap at 50
        resources.add_wood(20.0);
        assert_eq!(resources.wood, 50.0);
    }

    #[test]
    fn test_add_resource_no_overflow() {
        let mut resources = ColonyResources {
            stone: 20.0,
            max_stone: 20.0,
            ..Default::default()
        };

        resources.add_stone(10.0);
        assert_eq!(resources.stone, 20.0);
    }

    #[test]
    fn test_stockpile_component_defaults() {
        let stockpile = Stockpile::default();
        assert_eq!(stockpile.food_bonus, 0.0);
        assert_eq!(stockpile.wood_bonus, 100.0);
        assert_eq!(stockpile.stone_bonus, 100.0);
    }

    #[test]
    fn test_update_resource_caps_system() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        // Spawn 2 Stockpiles
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile {
                food_bonus: 0.0,
                wood_bonus: 100.0,
                stone_bonus: 50.0,
                waste_bonus: 0.0,
            },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile {
                food_bonus: 0.0,
                wood_bonus: 100.0,
                stone_bonus: 50.0,
                waste_bonus: 0.0,
            },
        ));

        // Run system
        world.run_system_once(update_resource_caps_system).unwrap();

        let resources = world.resource::<ColonyResources>();

        // Base (50) + 2 * 100 = 250
        assert_eq!(resources.max_wood, 250.0);
        // Base (20) + 2 * 50 = 120
        assert_eq!(resources.max_stone, 120.0);
    }

    #[test]
    fn test_building_type_stockpile_exists() {
        let bt = BuildingType::Stockpile;
        assert_eq!(bt.label(), "Stockpile");
    }
}
