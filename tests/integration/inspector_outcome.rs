use bevy_ecs::prelude::*;
use scale::layer1::inspector::{inspector_report_system, Inspector};
use scale::layer1::integration::inspector_outcome_bridge_system;
use scale::layer1::map::GridPosition;
use scale::layer1::memory::Memories;
use scale::layer1::notifications::NotificationQueue;
use scale::layer1::pop::Pop;
use scale::layer1::resources::ColonyResources;
use scale::layer1::visitor::{Visitor, VisitorState};
use scale::shared::time::SimulationTime;

#[test]
fn inspector_outcome_affects_colony() {
    // 1. Setup World
    scale::setup::init_task_pools(); // Ensure task pools for commands
    let mut world = World::new();

    // Resources needed
    world.insert_resource(NotificationQueue::default());
    world.insert_resource(SimulationTime::default());
    world.insert_resource(ColonyResources::default());

    // 2. Setup Schedule
    let mut schedule = Schedule::default();
    // Register systems: Report -> Outcome
    schedule.add_systems((
        inspector_report_system,
        inspector_outcome_bridge_system.after(inspector_report_system),
    ));

    // 3. Spawn Entities

    // Spawn a Pop (to receive memories)
    let pop = world.spawn((Pop, Memories::default())).id();

    // Spawn an Inspector who is departing with a Great Score (S Grade)
    // S Grade requires Avg Score > 5.0
    // Let's say 5 samples, total 30.0 (Avg 6.0)
    world.spawn((
        Inspector {
            beauty_score: 30.0,
            samples_taken: 5,
            next_sample_tick: 0,
            ..Default::default()
        },
        Visitor {
            state: VisitorState::Departing,
            arrival_tick: 0,
            departure_tick: 100,
        },
        GridPosition { x: 0, y: 0 },
    ));

    // 4. Run the schedule once
    schedule.run(&mut world);

    // 5. Assertions

    // Check Notification (should exist)
    let notifications = world.resource::<NotificationQueue>();
    assert!(
        !notifications.active.is_empty(),
        "Notification should be generated"
    );
    assert!(notifications.active[0].text.contains("Grade S"));

    // Check Pop Memories (should have InspectorImpressed)
    let memories = world.get::<Memories>(pop).unwrap();
    let has_memory = memories
        .items
        .iter()
        .any(|m| format!("{:?}", m.memory_type) == "InspectorImpressed");
    assert!(has_memory, "Pop should remember the great inspection!");

    // Check Resources (should have gained Knowledge)
    let resources = world.resource::<ColonyResources>();
    assert!(
        resources.knowledge > 0.0,
        "Colony should gain Knowledge from S-grade inspection"
    );
}
