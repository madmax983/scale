// src/layer3/events/refugee_waves.rs

//! Refugee Waves
//!
//! Handles mass migration events caused by galactic instability. Players face the dilemma of
//! accepting desperate, traumatized pops (burdening local resources) or rejecting them
//! (suffering severe diplomatic penalties with the origin faction).
use crate::layer1::health::Health;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::traits::Traits;
use crate::layer3::diplomacy_reflection::DiplomaticRelations;
use bevy::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject,
}

#[derive(Event)]
pub struct RefugeeWaveEvent {
    pub decision: Decision,
    pub population_count: usize,
    pub origin_faction: String,
    pub health_penalty: f32,
}

/// Processes the player's decision to accept or reject an incoming wave of refugees.
///
/// # Examples
/// ```
/// use bevy::prelude::*;
/// use scale::layer3::events::refugee_waves::{process_refugee_decision, RefugeeWaveEvent, Decision};
/// let mut app = App::new();
/// app.add_event::<RefugeeWaveEvent>();
/// app.add_systems(Update, process_refugee_decision);
/// app.world_mut().send_event(RefugeeWaveEvent { decision: Decision::Accept, population_count: 10, origin_faction: "Rebels".to_string(), health_penalty: 20.0 });
/// app.update();
/// // 10 traumatized pops are spawned in the world
/// ```
pub fn process_refugee_decision(
    mut events: EventReader<RefugeeWaveEvent>,
    mut commands: Commands,
    mut diplomacy_query: Query<&mut DiplomaticRelations>,
) {
    for event in events.read() {
        match event.decision {
            Decision::Accept => {
                // Spawn massive influx of low-health, desperate pops
                // GHOST-POP FIX (2026-10-04): these used to spawn as bare
                // Pop+Health+Needs+Traits — no PopName, no Wallet, no
                // GridPosition — invisible to STATS. Full PopBundle instead,
                // keeping the refugee/traumatized traits, the health
                // penalty, and the desperate hunger.
                let mut rng = rand::thread_rng();
                for _ in 0..event.population_count {
                    let mut traits = Traits::default();
                    traits.add(crate::layer1::psychology::traits::Trait::Refugee);
                    traits.add(crate::layer1::psychology::traits::Trait::Traumatized);

                    let mut bundle =
                        crate::layer1::PopBundle::random(0, 0, &mut rng);
                    bundle.traits = traits;
                    bundle.health = Health {
                        current: 100.0 - event.health_penalty,
                        max: 100.0,
                        has_rust_lung: false,
                    };
                    bundle.needs = Needs {
                        hunger: 10.0,
                        rest: 10.0,
                        ..default()
                    };
                    commands.spawn(bundle);
                }
            }
            Decision::Reject => {
                // Apply a severe diplomatic penalty with the origin faction (or their enemies)
                for mut dip in diplomacy_query.iter_mut() {
                    for relation in dip.relations.iter_mut() {
                        if relation.target_id == event.origin_faction {
                            relation.standing -= 50.0;
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::pop::Pop;
    use crate::layer3::diplomacy_reflection::DiplomaticRelations;
    use crate::layer3::diplomacy_reflection::DiplomaticStanding;

    #[test]
    fn test_accepting_refugees_spawns_pops_with_low_health() {
        let mut app = App::new();
        app.add_event::<RefugeeWaveEvent>();
        app.add_systems(Update, process_refugee_decision);

        // Fire decision event to ACCEPT
        app.world_mut().send_event(RefugeeWaveEvent {
            decision: Decision::Accept,
            population_count: 50,
            origin_faction: "Rebel Alliance".to_string(),
            health_penalty: 50.0,
        });

        app.update();

        // Verify 50 new pops spawned with poor health
        let mut pop_query = app.world_mut().query::<(&Pop, &Health)>();
        let new_pops: Vec<_> = pop_query.iter(app.world()).collect();

        assert_eq!(new_pops.len(), 50);
        // Assuming max health is 100
        assert!(new_pops[0].1.current <= 50.0);

        // GHOST-POP FIX (2026-10-04): refugees must be complete colonists —
        // named, with Wallets, on the grid — not bare Pop+Health+Needs ghosts.
        let mut name_query = app.world_mut().query::<&crate::layer1::pop::PopName>();
        assert_eq!(name_query.iter(app.world()).count(), 50);
        let mut wallet_query = app.world_mut().query::<&crate::layer1::economy::Wallet>();
        assert_eq!(wallet_query.iter(app.world()).count(), 50);
        let mut pos_query = app.world_mut().query::<&crate::layer1::map::GridPosition>();
        assert_eq!(pos_query.iter(app.world()).count(), 50);
    }

    #[test]
    fn test_rejecting_refugees_causes_diplomatic_penalty() {
        let mut app = App::new();
        app.add_event::<RefugeeWaveEvent>();
        app.add_systems(Update, process_refugee_decision);

        let entity = app
            .world_mut()
            .spawn(DiplomaticRelations {
                relations: vec![DiplomaticStanding {
                    target_id: "Galactic Senate".to_string(),
                    standing: 0.0,
                    sanctioned: false,
                }],
            })
            .id();

        // Fire decision event to REJECT
        app.world_mut().send_event(RefugeeWaveEvent {
            decision: Decision::Reject,
            population_count: 50,
            origin_faction: "Galactic Senate".to_string(),
            health_penalty: 0.0,
        });

        app.update();

        let diplomacy = app.world().get::<DiplomaticRelations>(entity).unwrap();
        // The player's standing with "Galactic Senate" should decrease significantly
        assert!(diplomacy.relations[0].standing < 0.0);
    }
}
