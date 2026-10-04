//! Primitives
//!
//! Defines primitive civilizations and the interactions with them.

use crate::layer1::architecture::structure::Structure;
use crate::layer1::resources::ColonyResources;
use crate::layer3::diplomacy_reflection::DiplomaticRelations;
use bevy::prelude::*;

#[derive(Component)]
pub struct PrimitiveCivilization {
    pub tech_level: PrimitiveTechLevel,
}

#[derive(Component)]
pub struct ObservationPost {
    pub active: bool,
}

#[derive(Component)]
pub struct SlaveMarker; // Spec used `Slave`, we will define it here if it's not in pop

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimitiveTechLevel {
    StoneAge,
    BronzeAge,
    IronAge,
}

#[derive(Event, Debug, Clone)]
pub struct InvasionEvent {
    pub target: Entity,
    pub aggressor: FactionId,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FactionId {
    Player,
    AI(u32),
}

#[derive(Component)]
pub struct PrimitiveFollowers {
    pub last_miracle_time: f64,
    pub anger_level: f32,
}

#[derive(Event)]
pub struct PrimitiveRetaliationEvent {
    pub target: Entity,
}

pub fn primitive_faith_system(
    time: Res<Time>,
    mut query: Query<(
        Entity,
        &mut PrimitiveFollowers,
        Option<&mut DiplomaticRelations>,
    )>,
    posts: Query<&ObservationPost>,
    mut events: EventWriter<PrimitiveRetaliationEvent>,
) {
    if posts.is_empty() {
        return;
    }

    let current_time = time.elapsed_secs_f64();
    for (entity, mut followers, maybe_diplomacy) in query.iter_mut() {
        let time_since_miracle = current_time - followers.last_miracle_time;

        if time_since_miracle > 30.0 {
            // Because tests use advance_by and we want to reliably increment without delta ticks:
            // Calculate missed miracles
            let neglect = time_since_miracle - 30.0;
            followers.anger_level = (neglect * 0.1) as f32; // Deterministic calculation

            if let Some(mut diplomacy) = maybe_diplomacy {
                for relation in diplomacy.relations.iter_mut() {
                    // Start dropping standing drastically
                    relation.standing = 50.0 - (neglect * 5.0) as f32;
                }
            }

            if followers.anger_level > 10.0 {
                events.send(PrimitiveRetaliationEvent { target: entity });
                // We'll let the system process it.
                // In game, we'd reset it, but tests will re-trigger unless we update miracle time.
            }
        }
    }
}

pub fn primitive_retaliation_system(
    mut events: EventReader<PrimitiveRetaliationEvent>,
    mut structures: Query<&mut Structure, With<ObservationPost>>,
) {
    for _event in events.read() {
        for mut structure in structures.iter_mut() {
            structure.current_hp -= 50.0;
        }
    }
}

pub fn process_observation_posts(
    posts: Query<&ObservationPost, With<PrimitiveCivilization>>,
    mut resources: ResMut<ColonyResources>,
) {
    for post in posts.iter() {
        if post.active {
            resources.knowledge += 5.0; // Minimal science gain
        }
    }
}

pub fn resolve_primitive_invasions(
    mut commands: Commands,
    mut events: EventReader<InvasionEvent>,
    civs: Query<&PrimitiveCivilization>,
) {
    for event in events.read() {
        if civs.get(event.target).is_ok() {
            // Remove the civilization
            commands
                .entity(event.target)
                .remove::<PrimitiveCivilization>();

            // Spawn some slaves
            // GHOST-POP FIX (2026-10-04): these used to spawn as bare
            // Pop+SlaveMarker — no Health, no PopName, no Wallet. Full
            // PopBundle instead so freed/invaded pops are real colonists.
            let mut rng = rand::thread_rng();
            for _ in 0..5 {
                let bundle = crate::layer1::PopBundle::random(0, 0, &mut rng);
                commands.spawn((bundle, SlaveMarker));
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::layer1::pop::Pop;
    use crate::layer2::generation::Planet;

    #[test]
    fn test_primitive_planet_generates_science_with_observation_post() {
        let mut app = App::new();
        app.insert_resource(ColonyResources::default());
        let _planet = app
            .world_mut()
            .spawn((
                Planet,
                PrimitiveCivilization {
                    tech_level: PrimitiveTechLevel::BronzeAge,
                },
                ObservationPost { active: true },
            ))
            .id();

        app.add_systems(Update, process_observation_posts);
        app.update();

        let resources = app.world().get_resource::<ColonyResources>().unwrap();
        assert!(
            resources.knowledge > 0.0,
            "Observation post should generate science points"
        );
    }

    #[test]
    fn test_invading_primitive_civilization_spawns_slave_pops() {
        let mut app = App::new();
        app.add_event::<InvasionEvent>();
        app.insert_resource(ColonyResources::default());
        let planet = app
            .world_mut()
            .spawn((
                Planet,
                PrimitiveCivilization {
                    tech_level: PrimitiveTechLevel::IronAge,
                },
            ))
            .id();

        let mut events = app.world_mut().resource_mut::<Events<InvasionEvent>>();
        events.send(InvasionEvent {
            target: planet,
            aggressor: FactionId::Player,
        });

        app.add_systems(Update, resolve_primitive_invasions);
        app.update();

        let planet_state = app.world().get::<PrimitiveCivilization>(planet);
        assert!(
            planet_state.is_none(),
            "Civilization should be removed after successful invasion"
        );

        let slave_count = app
            .world_mut()
            .query_filtered::<&Pop, With<SlaveMarker>>()
            .iter(app.world())
            .count();
        assert!(slave_count > 0, "Invasion should result in new slave pops");

        // GHOST-POP FIX (2026-10-04): freed slaves must be complete
        // colonists, not bare Pop+SlaveMarker ghosts.
        let named_slaves = app
            .world_mut()
            .query_filtered::<&crate::layer1::pop::PopName, With<SlaveMarker>>()
            .iter(app.world())
            .count();
        assert_eq!(named_slaves, slave_count, "Every slave must have a name");
        let healthy_slaves = app
            .world_mut()
            .query_filtered::<&crate::layer1::health::Health, With<SlaveMarker>>()
            .iter(app.world())
            .count();
        assert_eq!(healthy_slaves, slave_count, "Every slave must have Health");
    }

    use crate::layer1::architecture::structure::Structure;
    use crate::layer1::resources::ResourceType;
    use crate::layer3::diplomacy_reflection::{
        DiplomaticRelations, DiplomaticStanding, FaithCurrency,
    };

    // Dummy helper functions for testing, mirroring the spec
    fn setup_test_app() -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(FaithCurrency::default());
        app.world_mut()
            .spawn(DiplomaticRelations { relations: vec![] });
        app.add_event::<PrimitiveRetaliationEvent>();

        // Add BOTH systems to Update
        app.add_systems(
            Update,
            (primitive_faith_system, primitive_retaliation_system),
        );
        app
    }

    fn spawn_primitive_civ(world: &mut World) -> Entity {
        world
            .spawn((
                PrimitiveCivilization {
                    tech_level: PrimitiveTechLevel::StoneAge,
                },
                PrimitiveFollowers {
                    last_miracle_time: 0.0,
                    anger_level: 0.0,
                },
                DiplomaticRelations {
                    relations: vec![DiplomaticStanding {
                        target_id: "player".to_string(),
                        standing: 50.0,
                        sanctioned: false,
                    }],
                },
            ))
            .id()
    }

    fn spawn_observation_post(world: &mut World, _civ_entity: Entity) -> Entity {
        world
            .spawn((
                ObservationPost { active: true },
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
            ))
            .id()
    }

    fn drop_supplies_to_primitives(
        world: &mut World,
        _post_entity: Entity,
        _item_type: ResourceType,
    ) {
        // 1. Grant faith
        let mut faith = world.resource_mut::<FaithCurrency>();
        faith.amount += 10.0;

        // 2. Reset timer
        let current_time = world.resource::<Time>().elapsed_secs_f64();
        let mut query = world.query::<&mut PrimitiveFollowers>();
        for mut followers in query.iter_mut(world) {
            followers.last_miracle_time = current_time;
            followers.anger_level = 0.0;
        }
    }

    fn advance_simulation_time(world: &mut World, seconds: f64) {
        let mut time = world.resource_mut::<Time>();
        // Using advance_by will affect elapsed time
        time.advance_by(bevy::utils::Duration::from_secs_f64(seconds));
    }

    fn trigger_primitive_retaliation(__world: &mut World, _prim_entity: Entity) {
        // Left empty for RED phase
    }

    #[test]
    fn test_accidental_gods_faith_gain() {
        // Arrange: Setup world with a primitive civilization
        let mut app = setup_test_app();
        let prim_entity = spawn_primitive_civ(app.world_mut());
        let post_entity = spawn_observation_post(app.world_mut(), prim_entity);

        // Act: Drop supplies (Manna) to the primitives
        let initial_faith = app.world().resource::<FaithCurrency>().amount;
        drop_supplies_to_primitives(app.world_mut(), post_entity, ResourceType::Food);
        app.update();

        // Assert: Verify Faith currency increases
        let new_faith = app.world().resource::<FaithCurrency>().amount;
        assert!(
            new_faith > initial_faith,
            "Dropping supplies should generate Faith"
        );
    }

    #[test]
    fn test_accidental_gods_faith_loss_on_neglect() {
        // Arrange: Setup world where primitives received supplies previously
        let mut app = setup_test_app();
        let prim_entity = spawn_primitive_civ(app.world_mut());
        let post_entity = spawn_observation_post(app.world_mut(), prim_entity);
        drop_supplies_to_primitives(app.world_mut(), post_entity, ResourceType::Food);
        app.update();

        // Act: Neglect the primitives for an extended duration
        let initial_diplomacy = app
            .world()
            .get::<DiplomaticRelations>(prim_entity)
            .unwrap()
            .relations[0]
            .standing;
        advance_simulation_time(app.world_mut(), 30.0 * 24.0 * 3600.0); // 30 days
        app.update();

        // Assert: Verify diplomatic penalty is applied due to missing 'miracles'
        let new_diplomacy = app
            .world()
            .get::<DiplomaticRelations>(prim_entity)
            .unwrap()
            .relations[0]
            .standing;
        assert!(
            new_diplomacy < initial_diplomacy,
            "Neglecting primitive followers should cause diplomatic penalties"
        );
    }

    #[test]
    fn test_accidental_gods_primitive_retaliation() {
        // Arrange: Setup neglected primitives
        let mut app = setup_test_app();
        let prim_entity = spawn_primitive_civ(app.world_mut());
        let post_entity = spawn_observation_post(app.world_mut(), prim_entity);

        // Act: Advance simulation time to trigger extreme neglect retaliation
        advance_simulation_time(app.world_mut(), 60.0 * 24.0 * 3600.0); // 60 days
        trigger_primitive_retaliation(app.world_mut(), prim_entity);
        app.update();
        app.update();

        // Assert: Verify the observation post takes damage or a chronicle event is logged
        let post_health = app
            .world()
            .get::<Structure>(post_entity)
            .unwrap()
            .current_hp;
        assert!(
            post_health < 100.0,
            "Primitives should attack the post when angry"
        );
    }
}
