use bevy_ecs::prelude::*;
use scale::layer1::building::{Building, BuildingType};
use scale::layer1::farm::{consume_food_system, produce_food_system, Farm};
use scale::layer1::needs::Needs;
use scale::layer1::pop::Pop;
use scale::layer1::resources::ColonyResources;
use scale::layer1::utility_types::{ActionType, PopAction};
use scale::layer1::GridPosition;
use scale::shared::time::SimulationTime;

#[test]
fn test_food_production_satisfies_hunger() {
    scale::setup::init_task_pools();
    let mut world = World::new();

    // 1. Init Resources
    world.insert_resource(ColonyResources {
        food: 0.0,
        max_food: 100.0,
        ..Default::default()
    });
    world.insert_resource(SimulationTime::default());
    world.init_resource::<Events<scale::layer1::eureka::EurekaEvent>>();

    // add empty config
    world.insert_resource(scale::layer1::eureka::EurekaConfig {
        base_chance: 0.0,
        knowledge_reward: 0.0,
        breakthrough_points: 0.0,
    });

    // 2. Spawn a working Farmer to produce food
    world.spawn((
        Pop,
        GridPosition { x: 5, y: 5 },
        PopAction {
            current: ActionType::Farm,
            ..Default::default()
        },
    ));

    // Spawn the Farm at the same location
    world.spawn((
        Building {
            building_type: BuildingType::Farm,
        },
        GridPosition { x: 5, y: 5 },
        Farm::default(),
    ));

    // 3. Spawn a hungry pop at a different location
    let hungry_pop = world
        .spawn((
            Pop,
            Needs {
                hunger: 0.1, // Starving
                ..Default::default()
            },
        ))
        .id();

    // 4. Run the systems!
    // produce_food_system -> consume_food_system
    let mut schedule = Schedule::default();
    schedule.add_systems((produce_food_system, consume_food_system).chain());

    // run the schedule enough times to produce enough food
    for _ in 0..30 {
        schedule.run(&mut world);
    }

    // 5. Verify the seam is connected!
    // The farmer should have produced food, and the hungry pop should have consumed it immediately
    let needs = world.get::<Needs>(hungry_pop).unwrap();
    assert!(
        needs.hunger > 0.1,
        "Pop's hunger should have been satisfied by the food produced (hunger is {})",
        needs.hunger
    );

    // Let's verify the farmer ALSO consumed some if they were hungry (they spawned with 0.8 hunger by default, so they might have eaten too!)
}
