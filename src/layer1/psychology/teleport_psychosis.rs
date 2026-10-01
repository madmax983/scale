use crate::layer1::entities::pop::Pop;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::traits::{Trait, Traits};
use bevy::prelude::*;

pub const TELEPORT_DISSOCIATION_COST: f32 = 5.0;
pub const DISSOCIATION_PHANTOM_THRESHOLD: f32 = 80.0;
pub const DISSOCIATION_DEATH_THRESHOLD: f32 = 100.0;

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
            dissoc.level += TELEPORT_DISSOCIATION_COST;
        }
    }
}

pub fn process_psychosis_system(
    mut commands: Commands,
    mut query: Query<(Entity, &Dissociation, &mut Traits)>,
) {
    for (entity, dissoc, mut traits) in query.iter_mut() {
        if dissoc.level >= DISSOCIATION_DEATH_THRESHOLD {
            commands.entity(entity).despawn_recursive();
        } else if dissoc.level >= DISSOCIATION_PHANTOM_THRESHOLD {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_event::<TeleportEvent>();
        app.add_systems(
            Update,
            (
                handle_teleport_system,
                process_psychosis_system,
                hunger_decay_system,
            ),
        );
        app
    }

    #[test]
    fn should_increase_dissociation_when_teleport_event_received() {
        let mut app = setup_app();

        let pop = app.world_mut().spawn(Dissociation { level: 0.0 }).id();

        app.world_mut()
            .resource_mut::<Events<TeleportEvent>>()
            .send(TeleportEvent { entity: pop });

        app.update();

        let dissoc = app.world().get::<Dissociation>(pop).unwrap();
        assert_eq!(
            dissoc.level, TELEPORT_DISSOCIATION_COST,
            "Teleporting should increase dissociation by TELEPORT_DISSOCIATION_COST"
        );
    }

    #[test]
    fn should_add_phantom_trait_when_dissociation_reaches_phantom_threshold() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Dissociation {
                    level: DISSOCIATION_PHANTOM_THRESHOLD,
                },
                Traits::default(),
            ))
            .id();

        app.update();

        let traits = app.world().get::<Traits>(pop).unwrap();
        assert!(
            traits.has(Trait::Phantom),
            "Reaching DISSOCIATION_PHANTOM_THRESHOLD should add Trait::Phantom"
        );
    }

    #[test]
    fn should_despawn_entity_when_dissociation_reaches_death_threshold() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Dissociation {
                    level: DISSOCIATION_DEATH_THRESHOLD,
                },
                Traits::default(),
            ))
            .id();

        app.update();

        assert!(
            app.world().get_entity(pop).is_err(),
            "Reaching DISSOCIATION_DEATH_THRESHOLD should despawn the entity"
        );
    }

    #[test]
    fn should_decay_hunger_when_no_phantom_trait() {
        let mut app = setup_app();

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Needs {
                    hunger: 10.0,
                    ..Default::default()
                },
                Traits::default(),
            ))
            .id();

        app.update();

        let needs = app.world().get::<Needs>(pop).unwrap();
        assert_eq!(
            needs.hunger, 9.0,
            "Hunger should decay by 1.0 when not a Phantom"
        );
    }

    #[test]
    fn should_not_decay_hunger_when_phantom_trait_present() {
        let mut app = setup_app();

        let mut traits = Traits::default();
        traits.add(Trait::Phantom);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Needs {
                    hunger: 10.0,
                    ..Default::default()
                },
                traits,
            ))
            .id();

        app.update();

        let needs = app.world().get::<Needs>(pop).unwrap();
        assert_eq!(
            needs.hunger, 10.0,
            "Hunger should NOT decay when Trait::Phantom is present"
        );
    }
}
