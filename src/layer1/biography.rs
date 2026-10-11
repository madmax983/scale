//! # Biography: The Life Story of a Pop
//!
//! This module implements the "Biography" system, which gives every Pop a persistent history.
//! It works by monitoring the [`AssignedTo`](crate::layer1::actions::AssignedTo) component
//! and recording significant changes (like getting a new job or moving house) as text events.
//!
//! ## How it Works
//!
//! 1.  **Initialization**: When a Pop is spawned, it receives a [`Biography`] component with an "Arrival" event.
//! 2.  **Monitoring**: The [`biography_monitor_system`] runs every tick.
//! 3.  **Translation**: It reads the Pop's current [`AssignedTo`] component.
//! 4.  **Recording**: If the assignment has changed (and isn't just a repeat of the last event),
//!     a new [`BiographyEvent`] is appended.
//!
//! ## Example Output
//!
//! > *Tick 105*: Arrived via Cryopod malfunction. Joined the colony.
//! > *Tick 120*: Started working at Hydroponics Bay.
//! > *Tick 500*: Moved into Habitation Module A.
//! > *Tick 600*: Visited The Rusty Sprocket to socialize.

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::building::Building;
use crate::layer1::pop::Pop;
use crate::shared::narrative::NarrativeGenerator;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// A single significant event in a Pop's life.
///
/// Events are immutable records of past actions or milestones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiographyEvent {
    /// The simulation tick when the event occurred.
    pub tick: u64,
    /// The human-readable description of the event.
    pub text: String,
}

/// Component storing the history of a Pop.
///
/// This component acts as the "memory" of the Pop's life. It can be queried by the UI
/// to display a timeline of the Pop's existence.
///
/// # Examples
///
/// ```
/// use scale::layer1::biography::Biography;
///
/// let mut bio = Biography::default();
/// bio.add_event(100, "Became the colony's first Governor.".to_string());
///
/// assert_eq!(bio.events.len(), 1);
/// assert_eq!(bio.events[0].tick, 100);
/// ```
pub const MAX_BIOGRAPHY_EVENTS: usize = 50;

#[derive(Component, Default, Debug, Clone)]
pub struct Biography {
    /// Chronological list of life events.
    pub events: Vec<BiographyEvent>,
}

impl Biography {
    /// Appends a new event to the biography.
    pub fn add_event(&mut self, tick: u64, text: String) {
        self.events.push(BiographyEvent { tick, text });
        if self.events.len() > MAX_BIOGRAPHY_EVENTS {
            self.events.remove(0);
        }
    }
}

/// System to monitor and record biography events based on assignments.
///
/// This system polls all Pops with an [`AssignedTo`] component. It compares the current assignment
/// description against the last recorded event in the [`Biography`]. If they differ, it records
/// the new assignment.
///
/// # Logic
///
/// *   **New Pops**: Assigns a random "Arrival" story via [`NarrativeGenerator`].
/// *   **Assignments**: Translates [`AssignmentType`] into a sentence (e.g., `FarmWorker` -> "Started working at Farm").
/// *   **Deduplication**: Prevents spamming the same event every tick by checking `bio.events.last()`.
pub fn biography_monitor_system(
    time: Res<SimulationTime>,
    new_pops: Query<Entity, (With<Pop>, Without<Biography>)>,
    mut bio_pops: Query<(Entity, &mut Biography, &AssignedTo)>,
    buildings: Query<&Building>,
    mut commands: Commands,
    generator: Res<NarrativeGenerator>,
) {
    let current_tick = time.tick;

    // 1. Initialize new Pops
    for entity in &new_pops {
        let arrival = generator
            .get_random_fragment("POP_ARRIVAL_METHOD")
            .map_or_else(
                || "Joined the colony.".to_string(),
                |method| format!("Arrived {method}. Joined the colony."),
            );
        commands.entity(entity).insert(Biography {
            events: vec![BiographyEvent {
                tick: current_tick,
                text: arrival,
            }],
        });
    }

    // 2. Monitor Assignments
    for (_, mut bio, assigned) in &mut bio_pops {
        let target_name = buildings
            .get(assigned.entity)
            .map_or("Unknown Building", |building| {
                building.building_type.label()
            });

        let event_text = match assigned.assignment_type {
            AssignmentType::FarmWorker => format!("Started working at {target_name}."),
            AssignmentType::HousingResident => format!("Moved into {target_name}."),
            AssignmentType::TavernVisitor => format!("Visited {target_name} to socialize."),
            AssignmentType::LibraryWorker => format!("Started research at {target_name}."),
            AssignmentType::Patient => format!("Admitted to {target_name} for treatment."),
            AssignmentType::Funeral => format!("Attending funeral at {target_name}."),
            AssignmentType::ObservatoryWorker => format!("Observing the cosmos at {target_name}."),
            AssignmentType::Administrator => format!("Started administration at {target_name}."),
            AssignmentType::Surgery => format!("Undergoing surgery at {target_name}."),
            AssignmentType::Sheriff => format!("Enforcing the law at {target_name}."),
            AssignmentType::DeepMining => format!("Digging deep at {target_name}."),
            AssignmentType::RationingBureaucrat => format!("Rationing supplies at {target_name}."),
            AssignmentType::Doctor => format!("Tending patients at {target_name}."),
        };

        // Avoid duplicate consecutive events
        if let Some(last) = bio.events.last() {
            if last.text == event_text {
                continue;
            }
        }

        bio.add_event(current_tick, event_text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::BuildingType;
    use crate::shared::narrative::NarrativeGenerator;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_biography_initialization() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());

        let pop = world.spawn(Pop).id();

        world.run_system_once(biography_monitor_system).unwrap();

        let bio = world
            .get::<Biography>(pop)
            .expect("Biography should be added");
        assert_eq!(bio.events.len(), 1);
        assert!(
            bio.events[0].text.contains("colony"),
            "Should mention the colony"
        );
    }

    #[test]
    fn test_biography_records_assignment() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());

        // Create a farm
        let farm = world
            .spawn(Building {
                building_type: BuildingType::Farm,
            })
            .id();

        // Create a pop with Biography already (simulating existing pop)
        let pop = world
            .spawn((
                Pop,
                Biography::default(),
                AssignedTo {
                    entity: farm,
                    assignment_type: AssignmentType::FarmWorker,
                },
            ))
            .id();

        world.run_system_once(biography_monitor_system).unwrap();

        let bio = world.get::<Biography>(pop).unwrap();
        assert!(bio
            .events
            .iter()
            .any(|e| e.text.contains("Started working at Farm")));
    }

    #[test]
    fn test_biography_prevents_duplicate_events() {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());

        let farm = world
            .spawn(Building {
                building_type: BuildingType::Farm,
            })
            .id();

        let mut bio = Biography::default();
        bio.add_event(0, "Started working at Farm.".to_string());

        let pop = world
            .spawn((
                Pop,
                bio,
                AssignedTo {
                    entity: farm,
                    assignment_type: AssignmentType::FarmWorker,
                },
            ))
            .id();

        world.run_system_once(biography_monitor_system).unwrap();

        let bio_after = world.get::<Biography>(pop).unwrap();
        assert_eq!(bio_after.events.len(), 1, "Should not add duplicate event");
    }
}
