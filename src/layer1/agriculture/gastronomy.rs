#![allow(clippy::doc_markdown, clippy::option_if_let_else)]
//! Xeno-Gastronomy system (Spec 166).
//!
//! Handles the creation of "Mystery Meals" from alien ingredients, with randomized effects.

use crate::layer1::items::ItemType;
use bevy_ecs::prelude::*;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::hash::{Hash, Hasher};

/// Possible effects of consuming a Mystery Meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MealEffect {
    /// No effect.
    None,
    /// Increases Morale.
    MoodBoost,
    /// Decreases Morale.
    MoodDebuff,
    /// Increases work speed.
    HighEnergy,
    /// Decreases work speed.
    Lethargy,
    /// Damages health.
    Poison,
    /// Causes visual glitches (client-side only).
    Hallucination,
}

/// Tracks discovered recipes (Ingredient -> Effect).
#[derive(Resource, Default)]
pub struct Cookbook {
    /// Known associations between ingredients and effects.
    pub known_recipes: bevy::utils::HashMap<ItemType, MealEffect>,
}

impl Cookbook {
    /// Records a discovery.
    pub fn discover(&mut self, ingredient: ItemType, effect: MealEffect) {
        self.known_recipes.insert(ingredient, effect);
    }

    /// Checks if an ingredient's effect is known.
    #[must_use]
    pub fn is_known(&self, ingredient: &ItemType) -> bool {
        self.known_recipes.contains_key(ingredient)
    }

    /// Gets the known effect of an ingredient.
    #[must_use]
    pub fn get_effect(&self, ingredient: &ItemType) -> Option<MealEffect> {
        self.known_recipes.get(ingredient).copied()
    }
}

/// Component storing the hidden effect of a cooked meal.
#[derive(Component)]
pub struct MealHiddenEffect {
    /// The effect that will trigger when eaten.
    pub effect: MealEffect,
}

/// Component applied to a Pop when they consume a HighEnergy or Lethargy meal.
#[derive(Component, Debug, Clone, Copy)]
pub struct WorkSpeedBuff {
    /// Multiplier to work speed (e.g. 1.5 for HighEnergy, 0.5 for Lethargy).
    pub multiplier: f32,
    /// Duration in ticks.
    pub duration: u32,
}

/// Component applied to a Pop when they consume a Hallucinogenic meal.
#[derive(Component, Debug, Clone, Copy)]
pub struct Hallucinating {
    /// Duration in ticks.
    pub duration: u32,
}

/// Generates a deterministic effect for an item type based on a seed.
#[must_use]
pub fn generate_meal_effect(seed: u64, item: &ItemType) -> MealEffect {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    item.hash(&mut hasher);
    let item_hash = hasher.finish();

    let combined_seed = seed.wrapping_add(item_hash);
    let mut rng = StdRng::seed_from_u64(combined_seed);

    match rng.gen_range(0..6) {
        0 => MealEffect::MoodBoost,
        1 => MealEffect::MoodDebuff,
        2 => MealEffect::HighEnergy,
        3 => MealEffect::Lethargy,
        4 => MealEffect::Poison,
        _ => MealEffect::None,
    }
}

/// Performs a cooking experiment, consuming an ingredient to create a Mystery Meal.
pub fn perform_experiment(world: &mut World, chef: Entity, ingredient_entity: Entity) -> bool {
    // 1. Get Ingredient Type
    let item_type = if let Some(item) = world.get::<crate::layer1::items::Item>(ingredient_entity) {
        item.item_type
    } else {
        return false;
    };

    // 2. Consume Ingredient
    world.despawn(ingredient_entity);

    // 3. Determine Effect
    // Try to get WorldSeed resource, otherwise fallback to default seed
    // Using explicit path to WorldSeed if possible.
    // Assuming we can access crate::shared::random::WorldSeed.
    // If not, we fall back to a hardcoded seed for safety, but we should try to use the resource.
    let seed = if let Some(world_seed) = world.get_resource::<crate::shared::random::WorldSeed>() {
        world_seed.0
    } else {
        12345
    };

    let effect = generate_meal_effect(seed, &item_type);

    // 4. Determine Spawn Position (Chef's position)
    let pos = world
        .get::<crate::layer1::map::GridPosition>(chef)
        .copied()
        .unwrap_or_default();

    // 5. Produce Mystery Meal with Effect Component
    world.spawn((
        crate::layer1::items::Item {
            item_type: crate::layer1::items::ItemType::MysteryMeal,
        },
        MealHiddenEffect { effect },
        pos,
    ));

    // 6. Update Cookbook
    // For GREEN phase, we assume the chef learns immediately upon cooking.
    if let Some(mut cookbook) = world.get_resource_mut::<Cookbook>() {
        cookbook.discover(item_type, effect);
    }

    true
}

/// Applies the meal effect to a pop.
pub fn apply_meal_effect(world: &mut World, pop: Entity, effect: MealEffect) {
    match effect {
        MealEffect::MoodBoost => {
            if let Some(mut morale) = world.get_mut::<crate::layer1::morale::Morale>(pop) {
                morale.modifiers.push(crate::layer1::morale::MoodModifier {
                    label: "Delicious Meal".to_string(),
                    value: 0.2,
                    duration: 100,
                });
            }
        }
        MealEffect::MoodDebuff => {
            if let Some(mut morale) = world.get_mut::<crate::layer1::morale::Morale>(pop) {
                morale.modifiers.push(crate::layer1::morale::MoodModifier {
                    label: "Disgusting Meal".to_string(),
                    value: -0.2,
                    duration: 100,
                });
            }
        }
        MealEffect::HighEnergy => {
            world.entity_mut(pop).insert(WorkSpeedBuff {
                multiplier: 1.5,
                duration: 200,
            });
        }
        MealEffect::Lethargy => {
            world.entity_mut(pop).insert(WorkSpeedBuff {
                multiplier: 0.5,
                duration: 200,
            });
        }
        MealEffect::Poison => {
            if let Some(mut health) = world.get_mut::<crate::layer1::health::Health>(pop) {
                health.take_damage(10.0);
            }
            // Item 84: attribute the damage source at the moment of damage.
            crate::layer1::biology::death_attribution::note_damage_world(
                world,
                pop,
                crate::layer1::biology::death_attribution::CAUSE_FOOD_POISONING,
            );
        }
        MealEffect::Hallucination => {
            world
                .entity_mut(pop)
                .insert(Hallucinating { duration: 100 });
        }
        MealEffect::None => {}
    }
}

/// System to decay WorkSpeedBuff.
pub fn handle_work_speed_buff_decay(
    mut commands: Commands,
    mut query: Query<(Entity, &mut WorkSpeedBuff)>,
) {
    for (entity, mut buff) in &mut query {
        if buff.duration > 0 {
            buff.duration -= 1;
        }

        if buff.duration == 0 {
            commands.entity(entity).remove::<WorkSpeedBuff>();
        }
    }
}

/// System to decay Hallucinating component.
pub fn handle_hallucination_decay(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Hallucinating)>,
) {
    for (entity, mut h) in &mut query {
        if h.duration > 0 {
            h.duration -= 1;
        }

        if h.duration == 0 {
            commands.entity(entity).remove::<Hallucinating>();
        }
    }
}

/// System for analyzing ingredients (placeholder).
pub const fn analyze_ingredient_system() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::items::{Item, ItemType};
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::Pop;
    use crate::shared::random::WorldSeed;

    #[test]
    fn test_generate_consistent_effect() {
        // Effect should be deterministic based on seed + item type
        let effect1 = generate_meal_effect(12345, &ItemType::AlienMeatA);
        let effect2 = generate_meal_effect(12345, &ItemType::AlienMeatA);

        assert_eq!(effect1, effect2, "Same seed/item should yield same effect");

        // Try to find a seed that yields a different effect
        let mut found_different = false;
        for i in 0..100 {
            let effect3 = generate_meal_effect(12346 + i, &ItemType::AlienMeatA);
            if effect1 != effect3 {
                found_different = true;
                break;
            }
        }
        assert!(
            found_different,
            "Different seed should eventually yield different effect"
        );
    }

    #[test]
    fn test_cookbook_recording() {
        let mut world = World::new();
        world.insert_resource(Cookbook::default());

        // Record a discovery
        let mut cookbook = world.resource_mut::<Cookbook>();
        cookbook.discover(ItemType::AlienMeatA, MealEffect::HighEnergy);

        assert!(cookbook.is_known(&ItemType::AlienMeatA));
        assert_eq!(
            cookbook.get_effect(&ItemType::AlienMeatA),
            Some(MealEffect::HighEnergy)
        );
    }

    #[test]
    fn test_cooking_experiment_consumes_ingredients() {
        let mut world = World::new();
        world.insert_resource(Cookbook::default()); // Need resource to record
        world.insert_resource(WorldSeed(12345)); // Mock seed

        // Setup Chef and Ingredients
        let chef = world.spawn(GridPosition { x: 5, y: 5 }).id();
        let ingredient = world
            .spawn(Item {
                item_type: ItemType::AlienMeatA,
            })
            .id();

        // Add Cooking Experiment component to Chef or Workstation
        // For simplicity, let's say Chef performs action
        let success = crate::layer1::gastronomy::perform_experiment(&mut world, chef, ingredient);

        assert!(success);
        assert!(
            world.get_entity(ingredient).is_err(),
            "Ingredient should be consumed"
        );

        // Check for Output Meal
        let mut query = world.query::<(&Item, &GridPosition)>();
        let (item, pos) = query.single(&world);
        assert_eq!(item.item_type, ItemType::MysteryMeal);
        assert_eq!(pos.x, 5);
        assert_eq!(pos.y, 5);
    }

    #[test]
    fn test_eating_mystery_meal_applies_mood_boost() {
        let mut world = World::new();
        let pop = world
            .spawn((
                Pop,
                Morale {
                    value: 0.5,
                    ..Default::default()
                },
            ))
            .id();

        crate::layer1::gastronomy::apply_meal_effect(&mut world, pop, MealEffect::MoodBoost);

        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale.modifiers.iter().any(|m| m.label == "Delicious Meal"));
    }

    #[test]
    fn test_eating_mystery_meal_applies_work_speed_buff() {
        let mut world = World::new();
        let pop = world.spawn(Pop).id();

        crate::layer1::gastronomy::apply_meal_effect(&mut world, pop, MealEffect::HighEnergy);

        let buff = world.get::<WorkSpeedBuff>(pop).unwrap();
        assert_eq!(buff.multiplier, 1.5);
        assert_eq!(buff.duration, 200);
    }

    #[test]
    fn test_eating_mystery_meal_applies_poison() {
        let mut world = World::new();
        let pop = world.spawn((Pop, Health::default())).id();

        crate::layer1::gastronomy::apply_meal_effect(&mut world, pop, MealEffect::Poison);

        let health = world.get::<Health>(pop).unwrap();
        assert!(health.current < 100.0);
    }

    #[test]
    fn test_eating_mystery_meal_applies_hallucination() {
        let mut world = World::new();
        let pop = world.spawn(Pop).id();

        crate::layer1::gastronomy::apply_meal_effect(&mut world, pop, MealEffect::Hallucination);

        let h = world.get::<Hallucinating>(pop).unwrap();
        assert_eq!(h.duration, 100);
    }

    #[test]
    fn test_buff_decay() {
        let mut world = World::new();
        let pop = world
            .spawn(WorkSpeedBuff {
                multiplier: 1.5,
                duration: 1,
            })
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_work_speed_buff_decay);

        schedule.run(&mut world);

        assert!(world.get::<WorkSpeedBuff>(pop).is_none());
    }

    #[test]
    fn test_hallucination_decay() {
        let mut world = World::new();
        let pop = world.spawn(Hallucinating { duration: 1 }).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_hallucination_decay);

        schedule.run(&mut world);

        assert!(world.get::<Hallucinating>(pop).is_none());
    }

    #[test]
    fn test_apply_meal_effect_none() {
        let mut world = World::new();
        let pop = world.spawn(Pop).id();

        crate::layer1::gastronomy::apply_meal_effect(&mut world, pop, MealEffect::None);

        // Ensure no components added
        assert!(world.get::<WorkSpeedBuff>(pop).is_none());
        assert!(world.get::<Hallucinating>(pop).is_none());
        assert!(world.get::<Morale>(pop).is_none());
    }
}
