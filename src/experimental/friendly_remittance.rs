use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::Wallet;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::PopName;
use crate::layer1::social::Relationships;
use crate::layer1::utility_types::manhattan_distance;
use bevy_ecs::prelude::*;

pub fn friendly_remittance_system(
    mut wallets: Query<&mut Wallet>,
    pops: Query<(Entity, &Relationships, &GridPosition)>,
    names: Query<&PopName>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    let mut transactions = Vec::new();

    // Find valid transactions
    for (entity1, rels, pos1) in pops.iter() {
        if let Ok(wallet1) = wallets.get(entity1) {
            if wallet1.credits > 20.0 {
                for (entity2, _, pos2) in pops.iter() {
                    if entity1 == entity2 {
                        continue;
                    }
                    if rels.get_affinity(entity2) > 50.0 && manhattan_distance(pos1, pos2) <= 2 {
                        if let Ok(wallet2) = wallets.get(entity2) {
                            if wallet2.credits < 5.0 {
                                transactions.push((entity1, entity2));
                                break; // One transfer per pop per tick
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply transactions
    for (rich, poor) in transactions {
        let amount = 5.0;

        if let Ok([mut w_rich, mut w_poor]) = wallets.get_many_mut([rich, poor]) {
            if w_rich.credits >= amount {
                w_rich.credits -= amount;
                w_poor.credits += amount;

                // Item 162 audit: name the pops, never Entity Debug.
                let rich_label = names
                    .get(rich)
                    .map_or_else(|_| "A colonist".to_string(), |name| name.0.clone());
                let poor_label = names
                    .get(poor)
                    .map_or_else(|_| "a friend".to_string(), |name| name.0.clone());
                let text = format!("{rich_label} generously transferred {:.1} credits to their good friend {poor_label} to help them through hard times.", amount);
                chronicle_events.send(AddChronicleEvent {
                    text,
                    importance: EventImportance::Minor,
                });
            }
        }
    }
}

pub fn register(schedule: &mut Schedule) {
    schedule.add_systems(friendly_remittance_system);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_friendly_remittance_system() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();

        let poor = world
            .spawn((
                GridPosition { x: 5, y: 6 },
                Wallet { credits: 2.0 },
                Relationships::default(),
            ))
            .id();

        let mut rich_rels = Relationships::default();
        rich_rels.set_affinity(poor, 80.0);
        let rich = world
            .spawn((
                GridPosition { x: 5, y: 5 },
                Wallet { credits: 25.0 },
                rich_rels,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(friendly_remittance_system);
        schedule.run(&mut world);

        let w_rich = world.get::<Wallet>(rich).unwrap();
        assert_eq!(w_rich.credits, 20.0);

        let w_poor = world.get::<Wallet>(poor).unwrap();
        assert_eq!(w_poor.credits, 7.0);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let evt = reader.read(events).next().unwrap();
        assert!(evt.text.contains("5.0 credits"));
    }
}
