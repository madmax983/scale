#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use scale::layer1::clothing::clothing_wear_system;
    use scale::layer1::health::Health;
    use scale::layer1::items::{Clothing, ClothingType, Equipment, Item};
    use scale::layer1::map::GridPosition;
    use scale::layer1::pop::Pop;
    use scale::layer1::temperature::{thermal_damage_system, TemperatureGrid};

    #[test]
    fn test_clothing_temperature_integration() {
        let mut world = World::new();

        let mut grid = TemperatureGrid::new(10, 10, -50.0);
        grid.set(5, 5, -50.0);
        world.insert_resource(grid);
        world.init_resource::<Events<scale::layer1::nature::temperature::ThermalDamageEvent>>();

        let tunic = world
            .spawn((
                Item::default(),
                Clothing {
                    clothing_type: ClothingType::Tunic,
                    insulation: 2.0, // Grants +60 cold tolerance, making tolerance -50.0
                    durability: 0.05, // Will break after 1 wear tick
                    max_durability: 100.0,
                },
            ))
            .id();

        let pop = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Equipment {
                    body: Some(tunic),
                    ..Default::default()
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((clothing_wear_system, apply_deferred, thermal_damage_system).chain());

        schedule.run(&mut world);

        let h1 = world.get::<Health>(pop).unwrap();
        // Since tunic broke and commands applied, pop should take damage
        assert!(
            h1.current < 100.0,
            "Pop should take damage after clothing breaks"
        );
    }
    use scale::layer1::balance::TICKS_PER_YEAR;
    use scale::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
    use scale::layer1::core::integration::thermal_damage_chronicle_bridge;
    use scale::layer1::nature::temperature::ThermalDamageEvent;
    use scale::shared::time::SimulationTime;

    #[test]
    fn test_thermal_damage_chronicle_bridge() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let time = SimulationTime {
            tick: TICKS_PER_YEAR + 10,
            speed: scale::shared::time::SimSpeed::Normal,
        };
        app.insert_resource(time);

        app.add_event::<ThermalDamageEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(Update, thermal_damage_chronicle_bridge);

        // Send a thermal damage event (cold damage — this test's
        // "Deep Chill" expectation is the hypothermia telegraph)
        app.world_mut().send_event(ThermalDamageEvent {
            entity: Entity::from_raw(1),
            amount: 0.5,
            is_heatstroke: false,
        });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut cursor = chronicle_events.get_cursor();
        let emitted: Vec<&AddChronicleEvent> = cursor.read(chronicle_events).collect();

        assert_eq!(emitted.len(), 1, "Should emit one AddChronicleEvent");
        assert_eq!(emitted[0].importance, EventImportance::Standard);
        assert!(emitted[0].text.contains("Deep Chill"));
    }
}
