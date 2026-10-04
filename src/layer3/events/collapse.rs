use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::morale::Morale;
use crate::layer1::pop::Pop;
use bevy::prelude::*;

#[derive(Event, Debug, Clone)]
pub struct CivilizationCollapseEvent {
    pub civ_id: Entity,
    pub population_lost: u32,
    pub tech_level: u32,
}

#[derive(Event, Debug, Clone)]
pub struct RefugeeFleetArrivalEvent {
    pub incoming_population: u32,
    pub tech_fragments: u32,
}

pub fn process_civilization_collapse_system(
    mut collapse_events: EventReader<CivilizationCollapseEvent>,
    mut refugee_events: EventWriter<RefugeeFleetArrivalEvent>,
    q_morale: Query<&Morale, With<Pop>>,
) {
    // Stability calculation: average morale of the colony
    let mut total_morale = 0.0;
    let mut count = 0;
    for morale in q_morale.iter() {
        total_morale += morale.value;
        count += 1;
    }

    let stability = if count > 0 {
        total_morale / count as f32
    } else {
        0.0 // No stability if no pops
    };

    for event in collapse_events.read() {
        if stability >= 0.5 {
            refugee_events.send(RefugeeFleetArrivalEvent {
                incoming_population: event.population_lost / 100,
                tech_fragments: event.tech_level * 10,
            });
        }
    }
}

pub fn process_refugee_arrival_system(
    mut commands: Commands,
    mut arrival_events: EventReader<RefugeeFleetArrivalEvent>,
    mut resources: Option<ResMut<ColonyResources>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in arrival_events.read() {
        // Spawn pops for incoming_population
        // GHOST-POP FIX (2026-10-04): refugees used to spawn as bare
        // Pop+Morale — no Health, no PopName, no Wallet. Full PopBundle
        // instead (it already includes Morale::default()).
        let mut rng = rand::thread_rng();
        for _ in 0..event.incoming_population {
            let bundle = crate::layer1::PopBundle::random(0, 0, &mut rng);
            commands.spawn(bundle);
        }

        if let Some(res) = resources.as_deref_mut() {
            res.add_knowledge(event.tech_fragments as f32);
        }

        chronicle.send(AddChronicleEvent {
            text: format!(
                "A massive refugee fleet has arrived, bringing {} new souls and fragments of lost technology.",
                event.incoming_population
            ),
            importance: EventImportance::Major,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_civilization_collapse_emits_refugee_event() {
        let mut app = App::new();
        app.add_event::<CivilizationCollapseEvent>();
        app.add_event::<RefugeeFleetArrivalEvent>();

        // Spawn a pop with high morale to create >0.5 stability
        app.world_mut().spawn((
            Pop,
            Morale {
                value: 0.8,
                ..default()
            },
        ));
        app.add_systems(Update, process_civilization_collapse_system);

        // Act: A neighboring civ collapses
        app.world_mut()
            .resource_mut::<Events<CivilizationCollapseEvent>>()
            .send(CivilizationCollapseEvent {
                civ_id: Entity::PLACEHOLDER,
                population_lost: 1_000_000,
                tech_level: 5,
            });

        app.update();

        // Assert: A refugee wave is generated targeted at our colony
        let refugee_events = app.world().resource::<Events<RefugeeFleetArrivalEvent>>();
        let mut reader = refugee_events.get_cursor();
        let events: Vec<_> = reader.read(refugee_events).collect();

        assert_eq!(events.len(), 1);
        assert!(events[0].incoming_population > 0);
        assert!(events[0].tech_fragments > 0);
    }

    #[test]
    fn test_low_stability_ignores_refugees() {
        let mut app = App::new();
        app.add_event::<CivilizationCollapseEvent>();
        app.add_event::<RefugeeFleetArrivalEvent>();

        // Low stability means they go elsewhere
        app.world_mut().spawn((
            Pop,
            Morale {
                value: 0.3,
                ..default()
            },
        ));
        app.add_systems(Update, process_civilization_collapse_system);

        app.world_mut()
            .resource_mut::<Events<CivilizationCollapseEvent>>()
            .send(CivilizationCollapseEvent {
                civ_id: Entity::PLACEHOLDER,
                population_lost: 1_000_000,
                tech_level: 5,
            });

        app.update();

        let refugee_events = app.world().resource::<Events<RefugeeFleetArrivalEvent>>();
        assert!(refugee_events.is_empty());
    }

    #[test]
    fn test_refugee_wave_arrival_impacts_resources() {
        let mut app = App::new();
        app.add_event::<RefugeeFleetArrivalEvent>();
        app.add_event::<AddChronicleEvent>();

        app.insert_resource(ColonyResources {
            knowledge: 100.0,
            max_knowledge: 1000.0,
            ..default()
        });
        app.add_systems(Update, process_refugee_arrival_system);

        app.world_mut()
            .resource_mut::<Events<RefugeeFleetArrivalEvent>>()
            .send(RefugeeFleetArrivalEvent {
                incoming_population: 50,
                tech_fragments: 50,
            });

        app.update();

        let res = app.world().resource::<ColonyResources>();
        assert_eq!(res.knowledge, 150.0);

        // Verify pops spawned as complete colonists
        // (GHOST-POP FIX 2026-10-04): not bare Pop+Morale ghosts.
        let mut pop_query = app.world_mut().query::<&Pop>();
        let pop_count = pop_query.iter(app.world()).count();
        assert_eq!(pop_count, 50);
        let mut name_query = app.world_mut().query::<&crate::layer1::pop::PopName>();
        assert_eq!(name_query.iter(app.world()).count(), 50);
        let mut health_query = app.world_mut().query::<&crate::layer1::health::Health>();
        assert_eq!(health_query.iter(app.world()).count(), 50);
        let mut wallet_query = app.world_mut().query::<&crate::layer1::economy::Wallet>();
        assert_eq!(wallet_query.iter(app.world()).count(), 50);
    }
}
