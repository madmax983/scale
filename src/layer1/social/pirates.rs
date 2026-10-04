use crate::layer1::core::integration::PirateAmnestyEvent;
use crate::layer1::economy::Wallet;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::traits::{Trait, Traits};
use bevy::prelude::*;

use crate::layer1::law::justice::{CrimeCommittedEvent, CrimeType};
use crate::layer3::physics::relativity::SimulationTime;
use rand::Rng;

/// Spawns Layer 1 Pirate pops in response to a Layer 3 pirate fleet accepting amnesty.
pub fn process_pirate_amnesty_system(
    mut commands: Commands,
    mut amnesty_events: EventReader<PirateAmnestyEvent>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    // Amnestied pirates join the colony proper: land on a living pop's
    // tile so they arrive inside the pressurized habitat, not at (0, 0).
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();
    for _ev in amnesty_events.read() {
        // Spawn multiple pirate pops per fleet
        for _ in 0..5 {
            let mut pirate_traits = Traits::default();
            pirate_traits.add(Trait::Pirate);

            // GHOST-POP FIX (2026-10-04): amnestied pirates used to spawn
            // as bare Pop+Traits+Wallet — no Health, no PopName, no
            // GridPosition — invisible to STATS and immune to damage.
            // Spawn a full PopBundle instead, keeping the Pirate trait and
            // the amnesty credit boost.
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });
            let mut bundle =
                crate::layer1::PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            bundle.traits = pirate_traits;
            bundle.wallet = Wallet { credits: 1000.0 }; // Massive credit boost
            commands.spawn(bundle);
        }
    }
}

/// Pirates frequently shirk work and engage in brawls/crime randomly.
pub fn pirate_crime_system(
    mut crime_events: EventWriter<CrimeCommittedEvent>,
    pirates: Query<(Entity, &Traits), With<Pop>>,
    _sim_time: Res<SimulationTime>,
) {
    let mut rng = rand::thread_rng();

    for (entity, traits) in pirates.iter() {
        if traits.has(Trait::Pirate) {
            // Determine base on arbitrary periodic check
            // e.g. 5% chance per tick to commit vandalism
            if rng.gen_bool(0.05) {
                crime_events.send(CrimeCommittedEvent {
                    perpetrator: entity,
                    crime_type: CrimeType::Vandalism,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pirate_crime() {
        let mut app = App::new();
        app.add_event::<CrimeCommittedEvent>();
        app.init_resource::<SimulationTime>();
        // Required so events clear each frame if added directly.
        // We will just read the cursor without clearing since we loop update.
        // Wait, Bevy clears events every frame if not explicitly updated in schedule via event buffer systems.
        // Actually, App::update() runs all schedules, including clearing events.
        // Let's read events directly instead of relying on `App::update()` loop OR don't run `App::update()`.
        // Better: Just manually run the system!

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);

        // A pirate pop
        let pirate_pop = app.world_mut().spawn((Pop, pirate_traits)).id();

        // A normal pop
        let normal_pop = app.world_mut().spawn((Pop, Traits::default())).id();

        let mut pirate_crimes = 0;
        let mut normal_crimes = 0;

        // Run the system manually many times
        for _ in 0..1000 {
            let mut system_state = bevy::ecs::system::IntoSystem::into_system(pirate_crime_system);
            system_state.initialize(app.world_mut());
            system_state.run((), app.world_mut());
            system_state.apply_deferred(app.world_mut());

            let events = app.world().resource::<Events<CrimeCommittedEvent>>();
            let mut cursor = events.get_cursor();
            for ev in cursor.read(events) {
                if ev.perpetrator == pirate_pop {
                    pirate_crimes += 1;
                } else if ev.perpetrator == normal_pop {
                    normal_crimes += 1;
                }
            }
            app.world_mut()
                .resource_mut::<Events<CrimeCommittedEvent>>()
                .clear();
        }

        // Ensure pirate committed a crime and normal pop did not
        assert!(
            pirate_crimes > 0,
            "Pirate pop should have committed at least one crime"
        );
        assert_eq!(
            normal_crimes, 0,
            "Normal pop should not have committed a crime"
        );
    }

    #[test]
    fn test_process_pirate_amnesty() {
        let mut app = App::new();
        app.add_event::<PirateAmnestyEvent>();

        app.world_mut().send_event(PirateAmnestyEvent {
            fleet: Entity::PLACEHOLDER,
        });

        app.add_systems(Update, process_pirate_amnesty_system);
        app.update();

        // Verify that pirate pops were spawned as COMPLETE colonists
        // (GHOST-POP FIX 2026-10-04): Health, PopName, GridPosition —
        // not bare Pop+Traits+Wallet ghosts.
        let mut query = app.world_mut().query::<(
            &Pop,
            &Traits,
            &Wallet,
            &crate::layer1::pop::PopName,
            &crate::layer1::health::Health,
            &GridPosition,
        )>();
        let mut count = 0;
        for (_pop, traits, wallet, _name, _health, _pos) in query.iter(app.world()) {
            assert!(traits.has(Trait::Pirate));
            assert_eq!(wallet.credits, 1000.0);
            count += 1;
        }

        assert_eq!(count, 5); // Spawns 5 per fleet
    }
}
