use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::layer1::energy::{Battery, PowerConsumer};
use crate::layer1::fire::Fire;
use crate::layer1::map::GridPosition;
use crate::layer1::temperature::TemperatureGrid;

#[derive(Component)]
pub struct PowerCable {
    pub capacity: f32,
    pub current_load: f32,
}

#[allow(clippy::too_many_arguments)] // Item 125 adds the cable-heat tracker param (8th).
pub fn evaluate_grid_load_system(
    mut cables: Query<(&mut PowerCable, &GridPosition)>,
    consumers: Query<(&PowerConsumer, &GridPosition)>,
    mut batteries: Query<(&mut Battery, &GridPosition)>,
    existing_fires: Query<&GridPosition, With<Fire>>,
    mut temperature_grid: ResMut<TemperatureGrid>,
    mut commands: Commands,
    // Item 125: optional so minimal worlds that never inserted a
    // CableHeatTracker keep working exactly as before (telegraph absent).
    mut cable_heat: Option<ResMut<crate::layer1::energy::cable_heat::CableHeatTracker>>,
    mut chronicle: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    // Group consumers by tile to find local demand
    let mut tile_demand = HashMap::new();
    for (consumer, pos) in consumers.iter() {
        if consumer.active {
            *tile_demand.entry((pos.x, pos.y)).or_insert(0.0) += consumer.demand;
        }
    }

    // Attempt to buffer surges using local batteries
    let mut buffered_demand = HashMap::new();
    for (pos, demand) in tile_demand.iter() {
        let mut remaining_demand = *demand;
        // Find batteries on this tile
        for (mut battery, bat_pos) in batteries.iter_mut() {
            if bat_pos.x == pos.0 && bat_pos.y == pos.1 {
                let drained = battery.discharge(remaining_demand);
                remaining_demand -= drained;
                if remaining_demand <= 0.0 {
                    break;
                }
            }
        }
        buffered_demand.insert(*pos, remaining_demand);
    }

    // Check each cable
    for (mut cable, pos) in cables.iter_mut() {
        let demand = *buffered_demand.get(&(pos.x, pos.y)).unwrap_or(&0.0);
        cable.current_load = demand;

        // Item 125: cable heat-buildup telegraph. Observe EVERY cable
        // every tick (overloaded or not) so the per-tile sustained
        // counters and the clear/re-arm hysteresis advance on the same
        // clock as the heat below. This only reads demand/capacity and
        // chronicles warnings — the overload branch below (heat rate,
        // 2.0x fire threshold, fire spawn) is untouched.
        let heat_warning = cable_heat
            .as_deref_mut()
            .and_then(|tracker| tracker.observe((pos.x, pos.y), demand, cable.capacity));
        if let Some(kind) = heat_warning {
            use crate::layer1::energy::cable_heat::CableHeatWarning;
            let (text, importance) = match kind {
                CableHeatWarning::First => (
                    format!(
                        "Warning: the power cable at ({}, {}) is overheating — load {:.0} against capacity {:.0} for {} ticks. Shed load or add capacity before it catches fire.",
                        pos.x,
                        pos.y,
                        demand,
                        cable.capacity,
                        crate::layer1::energy::cable_heat::CABLE_HEAT_WARN_TICKS,
                    ),
                    crate::layer1::chronicle::EventImportance::Standard,
                ),
                CableHeatWarning::Escalated => (
                    format!(
                        "The power cable at ({}, {}) is about to ignite — load {:.0} against capacity {:.0} has held for {} ticks and heat is still building. Shed load at once.",
                        pos.x,
                        pos.y,
                        demand,
                        cable.capacity,
                        crate::layer1::energy::cable_heat::CABLE_HEAT_ESCALATE_TICKS,
                    ),
                    crate::layer1::chronicle::EventImportance::Major,
                ),
            };
            chronicle.send(crate::layer1::chronicle::AddChronicleEvent { text, importance });
        }

        if demand > cable.capacity {
            // Overloaded
            // Heat generation scales with overload amount
            let overload_amount = demand - cable.capacity;
            temperature_grid.add(pos.x, pos.y, overload_amount * 0.1); // Add some heat

            if demand > cable.capacity * 2.0 {
                // Extreme overload -> Fire
                // Prevent spawning dozens of fires per frame on the same tile
                let has_fire = existing_fires.iter().any(|p| p.x == pos.x && p.y == pos.y);
                if !has_fire {
                    commands.spawn((Fire::default(), *pos));

                    // Item 120 / balance Bug #4 (ignition telegraph): this
                    // cable-lane ignition used to spawn Fire with no
                    // Chronicle entry at all, and no downstream fire
                    // system chronicles new fires (fire_spread /
                    // fire_damage / fire_pressure_check are all silent),
                    // so the first signal was a pop panicking at the
                    // sight of it. Chronicle the ignition the tick it
                    // happens, naming the cable-overload cause and the
                    // tile, at Major importance — the Item 114 pattern
                    // used by the malfunction and grid-overload lanes.
                    // The has_fire guard above is also the spam guard:
                    // an already-burning tile never re-chronicles.
                    chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
                        text: format!(
                            "Fire! A cable overload has set the power cable at ({}, {}) ablaze!",
                            pos.x, pos.y
                        ),
                        importance: crate::layer1::chronicle::EventImportance::Major,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::energy::{Battery, PowerConsumer};
    use crate::layer1::fire::Fire;
    use crate::layer1::map::GridPosition;
    use crate::layer1::temperature::TemperatureGrid;

    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_cable_overload_creates_heat() {
        // Arrange
        let mut world = World::new();

        // Spawn a cable with max capacity 100
        let cable_pos = GridPosition { x: 5, y: 5 };
        let cable_entity = world
            .spawn((
                PowerCable {
                    capacity: 100.0,
                    current_load: 0.0,
                },
                cable_pos,
            ))
            .id();

        // Connect a consumer drawing 150 power
        world.spawn((
            PowerConsumer {
                demand: 150.0,
                active: true,
            },
            cable_pos,
        ));

        // Add a temperature grid
        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>(
            );
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));

        // Act: Evaluate grid
        let _ = world.run_system_once(evaluate_grid_load_system);

        // Assert: Cable is overloaded
        let cable = world.get::<PowerCable>(cable_entity).unwrap();
        assert!(
            cable.current_load > cable.capacity,
            "Cable load should exceed capacity"
        );

        // Assert: Heat is generated on the tile
        let temperature_grid = world.resource::<TemperatureGrid>();
        let tile_heat = if cable_pos.x >= 0 && cable_pos.y >= 0 {
            temperature_grid.get(cable_pos.x as usize, cable_pos.y as usize)
        } else {
            0.0
        };
        assert!(tile_heat > 0.0, "Overloaded cable must generate heat");
    }

    #[test]
    fn test_extreme_overload_causes_fire() {
        let mut world = World::new();

        let pos = GridPosition { x: 3, y: 3 };
        let _cable = world
            .spawn((
                PowerCable {
                    capacity: 50.0,
                    current_load: 0.0,
                },
                pos,
            ))
            .id();

        // Draw 300 power (massive overload)
        world.spawn((
            PowerConsumer {
                demand: 300.0,
                active: true,
            },
            pos,
        ));

        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>(
            );
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));

        let _ = world.run_system_once(evaluate_grid_load_system);

        // Assert: Fire entity spawned on the cable position
        let mut fire_query = world.query_filtered::<&GridPosition, With<Fire>>();
        let fire_exists = fire_query.iter(&world).any(|p| p.x == 3 && p.y == 3);
        assert!(fire_exists, "Massive overload must start a fire");
    }

    #[test]
    fn test_extreme_overload_chronicles_cable_fire() {
        // Item 120 / balance Bug #4: the per-tile cable lane spawns Fire
        // directly at demand > capacity * 2.0. Unlike the malfunction
        // lane (structure.rs) and the grid-overload bridge
        // (integration.rs), it sent no Chronicle entry, and no downstream
        // fire system chronicles new fires — so this ignition was silent.
        // The spawn must chronicle a Major cable-overload ignition naming
        // the tile, exactly once per new fire (the has_fire guard is the
        // spam guard).
        let mut world = World::new();
        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>(
            );

        let pos = GridPosition { x: 3, y: 3 };
        world.spawn((
            PowerCable {
                capacity: 50.0,
                current_load: 0.0,
            },
            pos,
        ));
        world.spawn((
            PowerConsumer {
                demand: 300.0,
                active: true,
            },
            pos,
        ));
        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));

        let _ = world.run_system_once(evaluate_grid_load_system);

        let mut fire_query = world.query_filtered::<&GridPosition, With<Fire>>();
        assert!(
            fire_query.iter(&world).any(|p| p.x == 3 && p.y == 3),
            "Massive overload must start a fire"
        );

        let events = world
            .resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let sent: Vec<_> = reader.read(events).collect();
        assert_eq!(
            sent.len(),
            1,
            "cable ignition must chronicle exactly once, got: {:?}",
            sent.iter().map(|e| e.text.clone()).collect::<Vec<_>>()
        );
        assert!(
            sent[0].text.contains("cable overload") && sent[0].text.contains("(3, 3)"),
            "chronicle must name the cable-overload cause and tile, got: {}",
            sent[0].text
        );
        assert!(
            matches!(
                sent[0].importance,
                crate::layer1::chronicle::EventImportance::Major
            ),
            "cable ignition chronicle must be Major, following item 114"
        );

        // Second tick on the already-burning tile: no duplicate entry.
        let _ = world.run_system_once(evaluate_grid_load_system);
        let events = world
            .resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let total = reader.read(events).count();
        assert_eq!(total, 1, "burning-tile re-run must not re-chronicle");
    }

    #[test]
    fn test_batteries_buffer_surges() {
        let mut world = World::new();
        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>(
            );

        let pos = GridPosition { x: 2, y: 2 };
        world.spawn((
            PowerCable {
                capacity: 100.0,
                current_load: 0.0,
            },
            pos,
        ));

        // Add a battery that can absorb the surge
        world.spawn((
            Battery {
                capacity: 500.0,
                charge: 500.0,
                max_throughput: 100.0,
            },
            pos,
        ));

        world.spawn((
            PowerConsumer {
                demand: 150.0,
                active: true,
            },
            pos,
        ));

        world.insert_resource(TemperatureGrid::new(10, 10, 0.0));

        let _ = world.run_system_once(evaluate_grid_load_system);

        // Assert: Cable is not overloaded because battery handles the local draw
        let temperature_grid = world.resource::<TemperatureGrid>();
        let tile_heat = if pos.x >= 0 && pos.y >= 0 {
            temperature_grid.get(pos.x as usize, pos.y as usize)
        } else {
            0.0
        };
        assert_eq!(
            tile_heat, 0.0,
            "Battery should buffer surge and prevent heat"
        );
    }
}
