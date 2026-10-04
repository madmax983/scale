//! The Endless Draft
//!
//! Represents the grim reality of eternal conscription during protracted proxy wars.
//! Colonies are forced into continuous military tithes, devastating local labor pools.

use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer3::diplomacy::proxy_wars::Credits;
use bevy::prelude::*;

#[derive(Component)]
pub struct TotalWarEmpire {
    pub faction_id: Entity,
    pub next_draft_timer: f32, // Timer to prevent spamming drafts
}

#[derive(Event)]
pub struct DraftOrderEvent {
    pub sponsor: Entity,
    pub required_pops: usize,
    pub min_physical_stat: f32, // Simplified representation
}

#[derive(Event)]
pub struct DraftComplianceEvent {
    pub sponsor: Entity,
    pub pops_provided: Vec<Entity>,
}

#[derive(Event)]
pub struct DraftRefusalEvent {
    pub sponsor: Entity,
}

#[derive(Event)]
pub struct VeteranReturnEvent {
    pub sponsor: Entity,
}

#[derive(Component)]
pub struct DiplomaticCurrency(pub i32);

#[derive(Component)]
pub struct EmbargoedBy {
    pub faction: Entity,
}

pub fn draft_order_generation_system(
    time: Res<Time>,
    mut empires: Query<(Entity, &mut TotalWarEmpire)>,
    mut events: EventWriter<DraftOrderEvent>,
) {
    let dt = time.delta_secs();
    for (entity, mut empire) in empires.iter_mut() {
        empire.next_draft_timer -= dt;
        if empire.next_draft_timer <= 0.0 {
            events.send(DraftOrderEvent {
                sponsor: entity,
                required_pops: 5,
                min_physical_stat: 10.0,
            });
            // Reset timer (e.g. 10 minutes in real time between drafts)
            empire.next_draft_timer = 600.0;
        }
    }
}

pub fn process_draft_compliance_system(
    mut commands: Commands,
    mut events: EventReader<DraftComplianceEvent>,
    mut colony_query: Query<(&mut Credits, &mut DiplomaticCurrency)>,
) {
    for event in events.read() {
        // Despawn the drafted pops
        for pop_entity in event.pops_provided.iter() {
            if let Some(commands_entity) = commands.get_entity(*pop_entity) {
                commands_entity.despawn_recursive();
            }
        }

        // Grant rewards
        if let Ok((mut credits, mut diplomacy)) = colony_query.get_single_mut() {
            credits.0 += 1000;
            diplomacy.0 += 500;
        }
    }
}

pub fn process_draft_refusal_system(
    mut commands: Commands,
    mut events: EventReader<DraftRefusalEvent>,
    colony_query: Query<Entity, With<DiplomaticCurrency>>,
) {
    for event in events.read() {
        if let Ok(colony) = colony_query.get_single() {
            commands.entity(colony).insert(EmbargoedBy {
                faction: event.sponsor,
            });
        }
    }
}

pub fn spawn_veteran_system(mut commands: Commands, mut events: EventReader<VeteranReturnEvent>) {
    for _ in events.read() {
        let mut traits = Traits::default();
        traits.add(Trait::Veteran); // Mocking Veteran trait

        // GHOST-POP FIX (2026-10-04): returning veterans used to spawn as
        // bare Pop+Traits+Skills — no Health, no PopName, no Wallet. Spawn
        // a full PopBundle instead, keeping the Veteran trait.
        let mut rng = rand::thread_rng();
        let mut bundle = crate::layer1::PopBundle::random(0, 0, &mut rng);
        bundle.traits = traits;
        commands.spawn(bundle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_draft_order_generation() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.add_event::<DraftOrderEvent>();
        app.add_systems(Update, draft_order_generation_system);

        // Arrange: Mock Layer 3 Empire in Total War, ready to draft
        let empire = app
            .world_mut()
            .spawn(TotalWarEmpire {
                faction_id: Entity::from_raw(1),
                next_draft_timer: 0.0,
            })
            .id();

        // Act: Trigger draft order generation system
        app.world_mut()
            .resource_mut::<Time<()>>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();

        // Assert: Verify draft order demands Pops with high physical stats
        let draft_events = app.world().resource::<Events<DraftOrderEvent>>();
        let mut reader = draft_events.get_cursor();
        let events: Vec<&DraftOrderEvent> = reader.read(draft_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sponsor, empire);
        assert_eq!(events[0].required_pops, 5);
        assert!(events[0].min_physical_stat >= 10.0);
    }

    #[test]
    fn test_draft_compliance_rewards() {
        let mut app = App::new();
        app.add_event::<DraftComplianceEvent>();
        app.add_systems(Update, process_draft_compliance_system);

        // Arrange: Setup colony with required Pops
        let pop1 = app.world_mut().spawn(Pop).id();
        let pop2 = app.world_mut().spawn(Pop).id();
        let _colony = app
            .world_mut()
            .spawn((Credits(0), DiplomaticCurrency(0)))
            .id();

        // Act: Process draft compliance
        app.world_mut().send_event(DraftComplianceEvent {
            sponsor: Entity::from_raw(1),
            pops_provided: vec![pop1, pop2],
        });
        app.update();

        // Assert: Pops removed from Layer 1, Trade and Diplomatic currency increased
        assert!(app.world().get::<Pop>(pop1).is_none());
        assert!(app.world().get::<Pop>(pop2).is_none());

        let mut q = app.world_mut().query::<(&Credits, &DiplomaticCurrency)>();
        let (credits, diplo) = q.single(app.world());
        assert!(credits.0 > 0);
        assert!(diplo.0 > 0);
    }

    #[test]
    fn test_draft_refusal_penalties() {
        let mut app = App::new();
        app.add_event::<DraftRefusalEvent>();
        app.add_systems(Update, process_draft_refusal_system);

        // Arrange: Setup colony
        let sponsor = Entity::from_raw(1);
        let colony = app.world_mut().spawn(DiplomaticCurrency(0)).id();

        // Act: Process draft refusal
        app.world_mut().send_event(DraftRefusalEvent { sponsor });
        app.update();

        // Assert: Severe embargo modifier applied
        let embargo = app.world().get::<EmbargoedBy>(colony);
        assert!(embargo.is_some());
        assert_eq!(embargo.unwrap().faction, sponsor);
    }

    #[test]
    fn test_veteran_return_event() {
        let mut app = App::new();
        app.add_event::<VeteranReturnEvent>();
        app.add_systems(Update, spawn_veteran_system);

        // Arrange: Trigger veteran return
        app.world_mut().send_event(VeteranReturnEvent {
            sponsor: Entity::from_raw(1),
        });

        // Act: Spawn veteran Pop
        app.update();

        // Assert: Pop has Veteran trait and is a complete colonist
        // (GHOST-POP FIX 2026-10-04): Health, PopName, Wallet present.
        let mut q = app.world_mut().query::<(
            &Pop,
            &Traits,
            &crate::layer1::pop::PopName,
            &crate::layer1::health::Health,
            &crate::layer1::economy::Wallet,
        )>();
        let mut found = false;
        for (_, traits, _name, _health, _wallet) in q.iter(app.world()) {
            if traits.has(Trait::Veteran) {
                found = true;
            }
        }
        assert!(found, "A veteran Pop should be spawned");
    }
}
