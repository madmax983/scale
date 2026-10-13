use crate::layer1::construction::great_works::GreatWork;
use crate::layer1::construction::great_works::OperationalGreatWork;
use crate::layer1::energy::PowerConsumer;
use crate::layer3::resources::EmpireCredits;
use bevy_ecs::prelude::*;

#[derive(Component)]
pub struct PilgrimShip {
    pub parked: bool,
    pub fee_rate: f32,
}

pub fn tick_nostalgia_engine(
    mut commands: Commands,
    query: Query<(&GreatWork, &PowerConsumer), With<OperationalGreatWork>>,
    mut tick_counter: Local<u64>,
) {
    *tick_counter += 1;
    let has_active_engine = query
        .iter()
        .any(|(work, consumer)| work.name == "Nostalgia Engine" && consumer.active);

    if has_active_engine && (*tick_counter).is_multiple_of(100) {
        commands.spawn(PilgrimShip {
            parked: true,
            fee_rate: 100.0,
        });
    }
}

pub fn process_pilgrim_fees(query: Query<&PilgrimShip>, credits: Option<ResMut<EmpireCredits>>) {
    let total_fees: f32 = query
        .iter()
        .filter(|ship| ship.parked)
        .map(|ship| ship.fee_rate)
        .sum();

    if let Some(mut credits_res) = credits {
        credits_res.0 += total_fees;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::construction::great_works::GreatWork;
    use crate::layer1::energy::PowerConsumer;
    use crate::layer3::resources::EmpireCredits;
    use bevy::prelude::*;

    fn setup_app() -> App {
        let mut app = App::new();
        app.insert_resource(EmpireCredits(0.0));
        app.add_systems(Update, (tick_nostalgia_engine, process_pilgrim_fees));
        app
    }

    #[test]
    fn test_nostalgia_engine_spawns_pilgrim_ships() {
        let mut app = setup_app();

        // Spawn the engine
        app.world_mut().spawn((
            GreatWork {
                name: "Nostalgia Engine".to_string(),
                current_phase: 1,
                phase_costs: vec![],
                stages: vec![],
                stage_applied: 0.0,
                kind: None,
            },
            OperationalGreatWork,
            PowerConsumer {
                demand: 100.0,
                active: true,
            },
        ));

        // Tick 100 times to spawn a ship
        for _ in 0..100 {
            app.update();
        }

        let mut query = app.world_mut().query::<&PilgrimShip>();
        let ships: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(
            ships.len(),
            1,
            "Should spawn exactly one PilgrimShip after 100 ticks"
        );
        assert!(ships[0].parked);
        assert_eq!(ships[0].fee_rate, 100.0);
    }

    #[test]
    fn test_parked_pilgrim_ships_generate_credits() {
        let mut app = setup_app();
        app.world_mut().spawn(PilgrimShip {
            parked: true,
            fee_rate: 100.0,
        });

        app.update();

        let credits = app.world().resource::<EmpireCredits>();
        assert_eq!(credits.0, 100.0, "Parked pilgrims should pay fees");
    }
}
