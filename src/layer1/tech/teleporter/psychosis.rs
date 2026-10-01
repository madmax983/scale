#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;
    use crate::layer1::psychology::needs::Needs;
    use crate::layer1::psychology::traits::{Trait, Traits};
    use bevy::prelude::*;

    #[test]
    fn test_teleporter_use_adds_dissociation() {
        // Arrange
        let mut app = App::new();
        app.add_event::<TeleportEvent>();
        app.add_systems(Update, handle_teleport_system);

        let pop_id = app
            .world_mut()
            .spawn((Pop, Dissociation { level: 0.0 }))
            .id();

        // Act: Pop uses teleporter
        app.world_mut().send_event(TeleportEvent { entity: pop_id });
        app.update();

        // Assert
        let dissoc = app.world().get::<Dissociation>(pop_id).unwrap();
        assert!(dissoc.level > 0.0);
    }

    #[test]
    fn test_high_dissociation_grants_phantom_trait() {
        // Arrange
        let mut app = App::new();
        app.add_systems(Update, process_psychosis_system);

        let pop_id = app
            .world_mut()
            .spawn((
                Pop,
                Traits::default(),
                Dissociation { level: 90.0 }, // Above threshold
            ))
            .id();

        // Act
        app.update();

        // Assert
        let traits = app.world().get::<Traits>(pop_id).unwrap();
        assert!(traits.has(Trait::Phantom));
    }

    #[test]
    fn test_phantom_trait_ignores_hunger() {
        // Arrange
        let mut app = App::new();
        app.add_systems(Update, hunger_decay_system);

        let normal_pop = app
            .world_mut()
            .spawn((
                Pop,
                Needs {
                    hunger: 100.0,
                    ..Default::default()
                },
                Traits::default(),
            ))
            .id();

        let mut ghost_traits = Traits::default();
        ghost_traits.add(Trait::Phantom);

        let ghost_pop = app
            .world_mut()
            .spawn((
                Pop,
                Needs {
                    hunger: 100.0,
                    ..Default::default()
                },
                ghost_traits,
            ))
            .id();

        // Act
        app.update();

        // Assert: Normal pop gets hungry, Ghost pop doesn't
        assert!(app.world().get::<Needs>(normal_pop).unwrap().hunger < 100.0);
        assert_eq!(app.world().get::<Needs>(ghost_pop).unwrap().hunger, 100.0);
    }
}
use crate::layer1::pop::Pop;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::traits::{Trait, Traits};
use bevy::prelude::*;

#[derive(Component)]
pub struct Dissociation {
    pub level: f32,
}

#[derive(Event, Debug)]
pub struct TeleportEvent {
    pub entity: Entity,
}

pub fn handle_teleport_system(
    mut events: EventReader<TeleportEvent>,
    mut query: Query<&mut Dissociation>,
) {
    for event in events.read() {
        if let Ok(mut dissoc) = query.get_mut(event.entity) {
            dissoc.level += 5.0; // Magic number for minimal passing test
        }
    }
}

pub fn process_psychosis_system(mut query: Query<(&Dissociation, &mut Traits)>) {
    for (dissoc, mut traits) in query.iter_mut() {
        if dissoc.level >= 80.0 {
            traits.add(Trait::Phantom);
        }
    }
}

/// Teleport dissociation disrupts metabolism: pops suffering from dissociation
/// (not yet Phantom) burn hunger rapidly. Healthy pops are unaffected.
pub fn hunger_decay_system(
    mut query: Query<(&mut Needs, &Traits), (With<Pop>, With<Dissociation>)>,
) {
    for (mut needs, traits) in query.iter_mut() {
        if !traits.has(Trait::Phantom) {
            needs.hunger -= 1.0;
        }
    }
}
