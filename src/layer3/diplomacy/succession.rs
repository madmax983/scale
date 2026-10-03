//! Dynastic Succession
//!
//! Handles faction leadership changes when the current leader dies, promoting the heir apparent,
//! or triggering a succession crisis if no heir is present.
use bevy::prelude::*;

use super::god_mind::GodMind;

#[derive(Component)]
pub struct Faction {
    pub name: String,
}

#[derive(Component)]
pub struct CurrentLeader(pub Entity);

#[derive(Component)]
pub struct HeirApparent(pub Entity);

#[derive(Component)]
pub struct Leader {
    pub name: String,
}

#[derive(Component)]
pub struct Heir {
    pub name: String,
}

#[derive(Component)]
pub struct Age {
    pub current: u32,
    pub max: u32,
}

#[derive(Component)]
pub struct Dead;

#[derive(Component)]
pub struct SuccessionCrisis;

#[derive(Event, Debug, Clone)]
pub struct SuccessionEvent {
    pub faction_name: String,
    pub old_leader_name: String,
    pub new_leader_name: String,
}

#[derive(Event, Debug, Clone)]
pub struct SuccessionCrisisEvent {
    pub faction_name: String,
    pub old_leader_name: String,
}

#[allow(clippy::type_complexity)]
pub fn process_succession_system(
    mut commands: Commands,
    leader_query: Query<(Entity, &Age, &Leader), Without<Dead>>,
    mut faction_query: Query<(
        Entity,
        &mut CurrentLeader,
        Option<&HeirApparent>,
        Option<&SuccessionCrisis>,
        &Faction,
        Option<&GodMind>,
    )>,
    heir_query: Query<&Heir>,
    mut succession_events: EventWriter<SuccessionEvent>,
    mut crisis_events: EventWriter<SuccessionCrisisEvent>,
) {
    // Reverse the iteration order to avoid nesting: Iterate over Factions first
    for (faction_entity, mut current_leader, heir_apparent, crisis, faction, god_mind) in
        faction_query.iter_mut()
    {
        // The Eternal Ruler never dies and never triggers succession crises.
        if god_mind.is_some() {
            continue;
        }

        // If they already have a crisis, we don't process them again
        if crisis.is_some() {
            continue;
        }

        // Check if the current leader is dead/dying
        if let Ok((leader_entity, age, leader)) = leader_query.get(current_leader.0) {
            if age.current >= age.max {
                // Leader died
                commands.entity(leader_entity).insert(Dead);
                commands.entity(leader_entity).remove::<CurrentLeader>(); // Remove from leadership position

                if let Some(heir) = heir_apparent {
                    // Heir takes over
                    current_leader.0 = heir.0;
                    commands.entity(heir.0).remove::<Heir>();

                    let heir_name = if let Ok(h) = heir_query.get(heir.0) {
                        h.name.clone()
                    } else {
                        "New King".to_string()
                    };

                    commands.entity(heir.0).insert(Leader {
                        name: heir_name.clone(),
                    });
                    commands.entity(faction_entity).remove::<HeirApparent>();

                    succession_events.send(SuccessionEvent {
                        faction_name: faction.name.clone(),
                        old_leader_name: leader.name.clone(),
                        new_leader_name: heir_name,
                    });
                } else {
                    // No heir, crisis
                    commands.entity(faction_entity).insert(SuccessionCrisis);
                    crisis_events.send(SuccessionCrisisEvent {
                        faction_name: faction.name.clone(),
                        old_leader_name: leader.name.clone(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynastic_succession_applies_new_modifiers() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<SuccessionEvent>();
        app.add_event::<SuccessionCrisisEvent>();
        app.add_systems(Update, process_succession_system);

        // Current leader with a trait
        let leader_entity = app
            .world_mut()
            .spawn((
                Leader {
                    name: "Queen A".to_string(),
                },
                Age {
                    current: 90,
                    max: 90,
                }, // About to die
            ))
            .id();

        // The heir with a different trait
        let heir_entity = app
            .world_mut()
            .spawn((Heir {
                name: "Prince B".to_string(),
            },))
            .id();

        let faction_entity = app
            .world_mut()
            .spawn((
                Faction {
                    name: "Empire".to_string(),
                },
                CurrentLeader(leader_entity),
                HeirApparent(heir_entity),
            ))
            .id();

        // Run the succession logic
        app.update();

        let faction = app.world().get::<CurrentLeader>(faction_entity).unwrap();
        // The old leader died and was replaced by the heir
        assert_eq!(faction.0, heir_entity);

        // The heir is now the leader
        assert!(app.world().get::<Leader>(heir_entity).is_some());
        assert_eq!(
            app.world().get::<Leader>(heir_entity).unwrap().name,
            "Prince B"
        );
        assert!(app.world().get::<Heir>(heir_entity).is_none());

        // Old leader is dead
        assert!(
            app.world().get_entity(leader_entity).is_err()
                || app.world().get::<Dead>(leader_entity).is_some()
        );
    }

    #[test]
    fn test_leader_death_without_heir_causes_crisis() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<SuccessionEvent>();
        app.add_event::<SuccessionCrisisEvent>();
        app.add_systems(Update, process_succession_system);

        let leader_entity = app
            .world_mut()
            .spawn((
                Leader {
                    name: "King C".to_string(),
                },
                Age {
                    current: 100,
                    max: 100,
                },
            ))
            .id();

        let faction_entity = app
            .world_mut()
            .spawn((
                Faction {
                    name: "Kingdom".to_string(),
                },
                CurrentLeader(leader_entity),
                // No HeirApparent
            ))
            .id();

        app.update();

        // Check for succession crisis
        assert!(app
            .world()
            .get::<SuccessionCrisis>(faction_entity)
            .is_some());
    }
}
