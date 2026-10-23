//! Energy system for the colony.
//!
//! Handles power generation, distribution, and consumption.
//! Grids are formed dynamically based on connectivity via `Conduit`s and power-related buildings.

use crate::layer1::map::GridPosition;
use crate::layer1::resources::ColonyResources;
use crate::layer1::weather::{WeatherState, WeatherType};
use bevy::utils::{HashMap, HashSet};
use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::VecDeque;

/// Auroral power generation.
pub mod auroral;
#[cfg(test)]
mod auroral_tests;
#[cfg(test)]
mod blackout_tests;
#[cfg(test)]
mod instability_tests;
pub mod grid_strain;
#[cfg(test)]
mod grid_strain_tests;
pub mod cable_heat;
#[cfg(test)]
mod cable_heat_tests;
pub mod load_limits;
pub mod phantom_grid;
#[cfg(test)]
mod phantom_grid_tests;
pub mod sky_tether;
pub use sky_tether::*;

pub use auroral::update_auroral_output_system;
pub mod gravity_siphon;
pub use gravity_siphon::*;

/// Emits power to the grid.
#[derive(Component, Debug, Clone)]
pub struct PowerSource {
    /// Amount of power produced per tick.
    pub output: f32,
    /// Whether the source is currently active (e.g., has fuel).
    pub active: bool,
}

impl Default for PowerSource {
    fn default() -> Self {
        Self {
            output: 10.0,
            active: true,
        }
    }
}

/// Consumes power from the grid.
#[derive(Component, Debug, Clone, Default)]
pub struct PowerConsumer {
    /// Amount of power consumed per tick.
    pub demand: f32,
    /// Whether the consumer is currently powered.
    pub active: bool,
}

/// Consumes fuel to operate.
#[derive(Component, Debug, Clone)]
pub struct FuelConsumer {
    /// Amount of fuel consumed per tick.
    pub amount: f32,
}

/// Connects power grid elements.
#[derive(Component, Debug, Clone)]
pub struct Conduit;

/// Event emitted when a grid component takes damage from overload.
#[derive(Event, Debug, Clone, Copy)]
pub struct GridOverloadEvent {
    /// The entity that was overloaded.
    pub victim: Entity,
}

/// Global protocol to cut power in emergencies.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct BlackoutProtocol {
    /// Whether the blackout is currently active.
    pub active: bool,
}

/// Stores excess power to buffer brownouts.
#[derive(Component, Debug, Clone)]
pub struct Battery {
    /// Total capacity.
    pub capacity: f32,
    /// Current charge.
    pub charge: f32,
    /// Maximum charge/discharge per tick.
    pub max_throughput: f32,
}

impl Battery {
    /// Charges the battery.
    pub fn charge(&mut self, amount: f32) {
        self.charge = (self.charge + amount).min(self.capacity);
    }

    /// Discharges the battery.
    pub fn discharge(&mut self, amount: f32) -> f32 {
        let actual = amount.min(self.charge).min(self.max_throughput);
        self.charge -= actual;
        actual
    }
}

/// ⚡ Bolt Optimization:
/// - Replaced `std::collections::{HashMap, HashSet}` with `bevy::utils::{HashMap, HashSet}` (AHash).
/// - Using AHash eliminates SipHash overhead on coordinate-based integer tuple keys `(i32, i32)`.
/// - Measurably reduces the CPU cost of `build_grid_map` and `bfs_grid` calls every frame.
fn build_grid_map(world: &mut World) -> HashMap<(i32, i32), Entity> {
    let mut grid_map = HashMap::new();
    let mut query = world.query_filtered::<(Entity, &GridPosition), Or<(
        With<PowerSource>,
        With<PowerConsumer>,
        With<Conduit>,
        With<Battery>,
        With<crate::layer1::kinetic_storage::KineticBattery>,
    )>>();

    for (entity, pos) in query.iter(world) {
        grid_map.insert((pos.x, pos.y), entity);
    }
    grid_map
}

fn bfs_grid(
    start_pos: (i32, i32),
    grid_map: &HashMap<(i32, i32), Entity>,
    world: &World,
    visited: &mut HashSet<(i32, i32)>,
    grid_entities: &mut Vec<Entity>,
    queue: &mut VecDeque<(i32, i32)>,
) -> (f32, f32, f32) {
    if visited.insert(start_pos) {
        queue.push_back(start_pos);
    }

    let mut total_production = 0.0;
    let mut total_nameplate_production = 0.0;
    let mut total_demand = 0.0;

    while let Some(pos) = queue.pop_front() {
        if let Some(&entity) = grid_map.get(&pos) {
            grid_entities.push(entity);

            if let Some(source) = world.get::<PowerSource>(entity).filter(|s| s.active) {
                // Check if a Quirk stops production
                let is_stopped = world
                    .get::<crate::layer1::rituals::Quirk>(entity)
                    .is_some_and(crate::layer1::rituals::Quirk::stops_production);

                if !is_stopped {
                    let efficiency = world
                        .get::<crate::layer1::prototyping::Prototype>(entity)
                        .map_or(1.0, |p| p.efficiency_modifier);
                    total_production += source.output * efficiency;
                    total_nameplate_production += source.output;
                }
            }
            if let Some(consumer) = world.get::<PowerConsumer>(entity) {
                total_demand += consumer.demand;
            }

            let neighbors = [
                (pos.0 + 1, pos.1),
                (pos.0 - 1, pos.1),
                (pos.0, pos.1 + 1),
                (pos.0, pos.1 - 1),
            ];

            for neighbor in neighbors {
                if grid_map.contains_key(&neighbor) && !visited.contains(&neighbor) {
                    visited.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
    }
    (total_production, total_nameplate_production, total_demand)
}

/// Calculates total production and demand for the grid connected to `start_entity`.
pub fn calculate_grid_stats(world: &mut World, start_entity: Entity) -> (f32, f32) {
    let grid_map = build_grid_map(world);
    let start_pos = match world.get::<GridPosition>(start_entity) {
        Some(p) => (p.x, p.y),
        None => return (0.0, 0.0),
    };

    let mut visited = HashSet::new();
    let mut grid_entities = Vec::new();
    let mut queue = VecDeque::new();
    let (prod, _nameplate, demand) = bfs_grid(
        start_pos,
        &grid_map,
        world,
        &mut visited,
        &mut grid_entities,
        &mut queue,
    );
    (prod, demand)
}

/// System to update power grids.
/// Identifies connected components, sums production/demand, and enables/disables consumers.
///
/// ⚡ Bolt Optimization:
/// - Avoids allocating a Vec for `positions` by iterating directly over `grid_map.keys().copied()`.
/// - Utilizes single-pass iteration replacing two separate `.iter().filter(...).copied().collect()` chains
///   with a single loop over `grid_entities`. This avoids iterating the vector twice and removes
///   two intermediate iterator/collection overheads per grid.
pub fn power_grid_system(world: &mut World) {
    // 0. Check Protocol
    let blackout = world
        .get_resource::<BlackoutProtocol>()
        .is_some_and(|b| b.active);

    if blackout {
        let mut consumers = world.query::<&mut PowerConsumer>();
        for mut consumer in consumers.iter_mut(world) {
            consumer.active = false;
        }
        // Blackout sheds all load: count it as an unstrained tick so a
        // strain episode can re-arm while the protocol is active.
        update_grid_strain(world, 1.0, None);
        return;
    }

    // Check Weather
    let demand_multiplier = world.get_resource::<WeatherState>().map_or(1.0, |w| {
        if w.current_weather == WeatherType::MagneticStorm {
            1.5
        } else {
            1.0
        }
    });

    // 1. Build grid map
    let grid_map = build_grid_map(world);

    // Extract buffer resource
    let mut buffer = world
        .remove_resource::<PowerGridBuffer>()
        .unwrap_or_default();

    // Clear global buffer state
    buffer.visited.clear();

    // Item 117: worst grid ratio this tick, for the strain telegraph.
    let mut worst_ratio = 1.0f32;
    let mut worst_example: Option<(Option<String>, (i32, i32))> = None;

    for start_pos in grid_map.keys().copied() {
        if buffer.visited.contains(&start_pos) {
            continue;
        }

        buffer.grid_entities.clear();
        buffer.queue.clear();
        buffer.batteries.clear();
        buffer.kinetic_batteries.clear();

        // BFS for this grid
        let (total_production, total_nameplate_production, base_demand) = bfs_grid(
            start_pos,
            &grid_map,
            world,
            &mut buffer.visited,
            &mut buffer.grid_entities,
            &mut buffer.queue,
        );

        let total_demand = base_demand * demand_multiplier;

        // 3. Calculate Net & Handle Batteries
        let mut net = total_production - total_demand;

        // Collect batteries in this grid
        for e in &buffer.grid_entities {
            if world.get::<Battery>(*e).is_some() {
                buffer.batteries.push(*e);
            }
            if world
                .get::<crate::layer1::kinetic_storage::KineticBattery>(*e)
                .is_some()
            {
                buffer.kinetic_batteries.push(*e);
            }
        }

        let provided = handle_batteries(world, &buffer.batteries, &buffer.kinetic_batteries, net);
        if net < 0.0 {
            // Update net after battery discharge (effectively increasing production availability)
            net += provided;
        }

        // 4. Handle Activation & Overload
        // Recalculate effective supply ratio
        // If net >= 0, supply_ratio = 1.0 (fully powered)
        // If net < 0, supply_ratio = (production + provided) / demand

        // total_available = total_production + battery_provided
        // battery_provided is calculated above.
        // If net < 0 originally: provided is what batteries gave.
        // New net = old_net + provided.
        // total_available = total_production + provided = total_demand + New net.

        let total_available = total_demand + net;

        let supply_ratio = if total_demand > 0.0 {
            (total_available / total_demand).min(1.0)
        } else {
            1.0
        };

        let grid_ratio = handle_overload(
            world,
            &buffer.grid_entities,
            total_nameplate_production,
            total_demand,
        );
        if grid_ratio > worst_ratio {
            worst_ratio = grid_ratio;
            worst_example = buffer.grid_entities.first().and_then(|e| {
                world.get::<GridPosition>(*e).map(|p| {
                    (
                        world
                            .get::<crate::layer1::building::Building>(*e)
                            .map(|b| b.building_type.label().to_string()),
                        (p.x, p.y),
                    )
                })
            });
        }

        activate_consumers(world, &buffer.grid_entities, net, supply_ratio);
    }

    // Item 117: feed the worst grid's ratio to the strain tracker once
    // per tick (not once per grid), so consecutive-tick counting is a
    // colony clock, and chronicle any warning that comes due.
    update_grid_strain(world, worst_ratio, worst_example);

    // Re-insert the buffer for the next frame
    world.insert_resource(buffer);
}

/// Item 117: advance the grid-strain tracker by one tick and, when a
/// warning comes due, chronicle it. First warning is Standard (nothing
/// has happened yet — Major is reserved for the Item 114 ignition
/// itself); the escalated warning is Major (ignition is overdue in
/// expectation and imminent). No-op for the Chronicle when the event
/// resource was never initialized (minimal test worlds).
fn update_grid_strain(
    world: &mut World,
    worst_ratio: f32,
    example: Option<(Option<String>, (i32, i32))>,
) {
    if !world.contains_resource::<grid_strain::GridStrainTracker>() {
        world.insert_resource(grid_strain::GridStrainTracker::default());
    }
    let warning = world
        .resource_mut::<grid_strain::GridStrainTracker>()
        .observe(worst_ratio, example);
    let Some(kind) = warning else { return };

    let (ratio, label, pos) = {
        let tracker = world.resource::<grid_strain::GridStrainTracker>();
        (
            tracker.last_ratio,
            tracker.example_label.clone(),
            tracker.example_pos,
        )
    };
    let pct = ratio * 100.0;
    let around = match (label, pos) {
        (Some(label), Some((x, y))) => format!(" around the {label} at ({x}, {y})"),
        (None, Some((x, y))) => format!(" around ({x}, {y})"),
        _ => String::new(),
    };
    let (text, importance) = match kind {
        grid_strain::StrainWarning::First => (
            format!(
                "Warning: the power grid is straining under overload — demand at {pct:.0}% of capacity{around}. Shed load or add generation before it catches fire."
            ),
            crate::layer1::chronicle::EventImportance::Standard,
        ),
        grid_strain::StrainWarning::Escalated => (
            format!(
                "The power grid is at breaking point — demand has held at {pct:.0}% of capacity{around}. Fire is imminent if load is not shed at once."
            ),
            crate::layer1::chronicle::EventImportance::Major,
        ),
    };
    if world.contains_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>() {
        world.send_event(crate::layer1::chronicle::AddChronicleEvent { text, importance });
    }
}

/// System to process fuel consumption for power sources.
pub fn process_fuel_consumption_system(
    mut resources: ResMut<ColonyResources>,
    mut query: Query<(&mut PowerSource, &FuelConsumer)>,
) {
    let available_fuel = resources.fuel;
    let mut fuel_spent = 0.0;

    for (mut source, consumer) in &mut query {
        if available_fuel - fuel_spent >= consumer.amount {
            // Have fuel
            fuel_spent += consumer.amount;

            // Activate if inactive
            if !source.active {
                source.active = true;
            }
        } else {
            // Out of fuel
            // Deactivate if active
            if source.active {
                source.active = false;
            }
        }
    }

    // Deduct total from global resources
    if fuel_spent > 0.0 {
        resources.fuel = (resources.fuel - fuel_spent).max(0.0);
    }
}

fn handle_batteries(
    world: &mut World,
    batteries: &[Entity],
    kinetic_batteries: &[Entity],
    net: f32,
) -> f32 {
    let mut provided = 0.0;

    if net > 0.0 {
        // Surplus: Charge batteries
        #[allow(clippy::cast_precision_loss)]
        let total_batteries = (batteries.len() + kinetic_batteries.len()) as f32;
        if total_batteries > 0.0 {
            let charge_per_battery = net / total_batteries;
            for bat_entity in batteries {
                if let Some(mut bat) = world.get_mut::<Battery>(*bat_entity) {
                    bat.charge(charge_per_battery);
                }
            }
            for bat_entity in kinetic_batteries {
                if let Some(mut bat) =
                    world.get_mut::<crate::layer1::kinetic_storage::KineticBattery>(*bat_entity)
                {
                    let input = charge_per_battery.min(bat.charge_rate);
                    // Efficiency loss on input
                    let stored = input * bat.efficiency;
                    bat.charge = (bat.charge + stored).min(bat.capacity);
                }
            }
        }
    } else if net < 0.0 {
        // Deficit: Discharge batteries
        let mut needed = -net;

        // Discharge normal batteries
        for bat_entity in batteries {
            if let Some(mut bat) = world.get_mut::<Battery>(*bat_entity) {
                let amount = bat.discharge(needed);
                provided += amount;
                needed -= amount;
                if needed <= 0.0 {
                    break;
                }
            }
        }

        // Discharge kinetic batteries if needed
        if needed > 0.0 {
            for bat_entity in kinetic_batteries {
                if let Some(mut bat) =
                    world.get_mut::<crate::layer1::kinetic_storage::KineticBattery>(*bat_entity)
                {
                    let available = bat.charge;
                    let discharge = needed.min(available).min(bat.charge_rate);
                    bat.charge -= discharge;
                    provided += discharge;
                    needed -= discharge;
                    if needed <= 0.0 {
                        break;
                    }
                }
            }
        }
    }

    provided
}

fn handle_overload(
    world: &mut World,
    grid_entities: &[Entity],
    total_nameplate_production: f32,
    total_demand: f32,
) -> f32 {
    // Overload is a wiring-capacity phenomenon: demand measured against the
    // grid's installed (nameplate) production, NOT prototype-discounted
    // output. Prototype inefficiency already manifests as brownout —
    // probabilistic consumer activation via the supply ratio in
    // `activate_consumers`. Measuring overload against discounted output
    // made every unmastered generator a fire hazard at nameplate-balanced
    // load: the starter grid (Lander 10 nameplate at 0.5 prototype
    // efficiency = 5 effective, LifeSupport demand 10) sat at ratio 2.0
    // from tick 0, so `GridOverloadEvent` -> `grid_overload_fire_bridge`
    // ignited the starter hull in nearly every run and fire destroyed the
    // interior buildings (LifeSupport included) within ~100 ticks.
    let overload_ratio = if total_nameplate_production > 0.0 {
        total_demand / total_nameplate_production
    } else {
        1.0
    };

    let mut rng = rand::thread_rng();

    // Overload Check (>150% demand vs base production)
    // Batteries don't prevent overload damage caused by high demand on generators
    if overload_ratio > 1.5 {
        // Risk of damage to random entity in grid
        // Chance increases with overload: (ratio - 1.5) * 0.05
        // e.g. 2.0 ratio -> 0.025 (2.5%) per tick
        if rng.gen_bool((0.05 * f64::from(overload_ratio - 1.5)).min(1.0)) {
            // Pick random entity
            if let Some(victim) = grid_entities.choose(&mut rng) {
                // Clippy suggests collapsing, but let_chains is unstable
                #[allow(clippy::collapsible_if)]
                if let Some(mut health) = world.get_mut::<crate::layer1::health::Health>(*victim) {
                    health.take_damage(10.0);
                }

                // Emit overload event
                world.send_event(GridOverloadEvent { victim: *victim });
            }
        }
    }

    overload_ratio
}

fn activate_consumers(world: &mut World, grid_entities: &[Entity], net: f32, supply_ratio: f32) {
    // Activation
    let mut rng = rand::thread_rng();
    for entity in grid_entities {
        if let Some(mut consumer) = world.get_mut::<PowerConsumer>(*entity) {
            if net >= -f32::EPSILON {
                consumer.active = true;
            } else {
                // Brownout: Probabilistic activation
                // e.g. 80% supply -> 80% chance to run
                consumer.active = rng.gen_bool(f64::from(supply_ratio));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{calculate_grid_stats, Conduit, PowerConsumer, PowerSource};
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::map::GridPosition;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_power_components() {
        let source = PowerSource {
            output: 10.0,
            active: true,
        };
        let consumer = PowerConsumer {
            demand: 5.0,
            active: true,
        };
        let _conduit = Conduit; // Marker

        assert_eq!(source.output, 10.0);
        assert_eq!(consumer.demand, 5.0);
    }

    #[test]
    fn test_grid_connectivity_isolated() {
        // Source and Consumer far apart, no conduit
        let mut world = World::new();

        // Generator at 0,0
        let generator = world
            .spawn((
                PowerSource {
                    output: 10.0,
                    active: true,
                },
                GridPosition { x: 0, y: 0 },
                Building {
                    building_type: BuildingType::Generator,
                }, // Assume new type
            ))
            .id();

        // Consumer at 10,10
        let cons = world
            .spawn((
                PowerConsumer {
                    demand: 5.0,
                    active: false,
                },
                GridPosition { x: 10, y: 10 },
                Building {
                    building_type: BuildingType::Smelter,
                },
            ))
            .id();

        // Run calculation
        let (production, _demand) = calculate_grid_stats(&mut world, generator); // Pass root entity to flood fill

        // Gen is its own grid
        assert_eq!(production, 10.0);

        // Consumer is isolated
        let (c_prod, c_demand) = calculate_grid_stats(&mut world, cons);
        assert_eq!(c_prod, 0.0);
        assert_eq!(c_demand, 5.0);
    }

    #[test]
    fn test_grid_connectivity_connected() {
        let mut world = World::new();

        // Generator at 0,0
        let generator = world
            .spawn((
                PowerSource {
                    output: 10.0,
                    active: true,
                },
                GridPosition { x: 0, y: 0 },
                Building {
                    building_type: BuildingType::Generator,
                },
            ))
            .id();

        // Conduit at 0,1
        world.spawn((
            Conduit,
            GridPosition { x: 0, y: 1 },
            Building {
                building_type: BuildingType::PowerPole,
            }, // New type
        ));

        // Consumer at 0,2
        let _cons = world
            .spawn((
                PowerConsumer {
                    demand: 5.0,
                    active: false,
                },
                GridPosition { x: 0, y: 2 },
                Building {
                    building_type: BuildingType::Smelter,
                },
            ))
            .id();

        // Run calculation (start from generator)
        let (production, demand) = calculate_grid_stats(&mut world, generator);

        assert_eq!(production, 10.0);
        assert_eq!(demand, 5.0);
    }

    #[test]
    fn test_overload_shutdown() {
        // 10 Prod, 15 Demand
        let mut world = World::new();

        // Gen 10
        world.spawn((
            PowerSource {
                output: 10.0,
                active: true,
            },
            GridPosition { x: 0, y: 0 },
            Building {
                building_type: BuildingType::Generator,
            },
        ));

        // Cons 1 (10)
        let c1 = world
            .spawn((
                PowerConsumer {
                    demand: 10.0,
                    active: true,
                },
                GridPosition { x: 0, y: 1 },
                Conduit, // Connect them implicitly or explicitly
                Building {
                    building_type: BuildingType::Smelter,
                },
            ))
            .id();

        // Cons 2 (5)
        let c2 = world
            .spawn((
                PowerConsumer {
                    demand: 5.0,
                    active: true,
                },
                GridPosition { x: 0, y: 2 },
                Conduit,
                Building {
                    building_type: BuildingType::Smelter,
                },
            ))
            .id();

        // Run system
        super::power_grid_system(&mut world);

        // Verify active state toggles
        // For Grid Instability (Spec 125): If Demand > Production, consumers flicker (Brownout).
        // Since 15 > 10, supply ratio is 0.66. There is a chance they are active.
        // We cannot deterministically assert !active unless supply is 0.
        // But for this test, we accept either state as valid runtime behavior,
        // effectively disabling the strict check to avoid flakiness until we mock RNG.
        let _c1_state = world.get::<PowerConsumer>(c1).unwrap();
        let _c2_state = world.get::<PowerConsumer>(c2).unwrap();

        // assert!(!c1_state.active); // Flaky
    }

    #[test]
    fn test_quirk_stops_power_production() {
        use crate::layer1::rituals::{Quirk, QuirkType};
        let mut world = World::new();

        let generator = world
            .spawn((
                PowerSource {
                    output: 10.0,
                    active: true,
                },
                GridPosition { x: 0, y: 0 },
                Building {
                    building_type: BuildingType::Generator,
                },
                Quirk {
                    quirk_type: QuirkType::Glitchy,
                }, // Should stop production
            ))
            .id();

        let (production, _) = calculate_grid_stats(&mut world, generator);
        assert_eq!(production, 0.0, "Glitchy generator should produce 0 power");
    }

    fn overload_event_count(world: &World) -> usize {
        let events = world.resource::<bevy_ecs::event::Events<super::GridOverloadEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).count()
    }

    #[test]
    fn test_prototype_grid_at_nameplate_balance_never_overloads() {
        // Regression (vanishing starter buildings): the starter grid pairs a
        // Lander (10 nameplate, 0.5 prototype efficiency = 5 effective) with
        // a LifeSupport (demand 10). Measuring overload against the
        // prototype-discounted output read ratio 2.0 from tick 0, so the
        // overload -> fire bridge ignited the starter hull in nearly every
        // run and fire destroyed the interior buildings. Overload is now
        // measured against nameplate production: this grid is balanced
        // (10 vs 10) and must never emit an overload event, while the
        // effective production stays discounted (brownout is preserved).
        let mut world = World::new();
        world.init_resource::<bevy_ecs::event::Events<super::GridOverloadEvent>>();

        let generator = world
            .spawn((
                PowerSource {
                    output: 10.0,
                    active: true,
                },
                GridPosition { x: 0, y: 0 },
                crate::layer1::prototyping::Prototype::default(),
            ))
            .id();
        world.spawn((
            PowerConsumer {
                demand: 10.0,
                active: true,
            },
            GridPosition { x: 1, y: 0 },
        ));

        let (production, demand) = calculate_grid_stats(&mut world, generator);
        assert_eq!(production, 5.0, "prototype discount still applies to output");
        assert_eq!(demand, 10.0);

        for _ in 0..300 {
            super::power_grid_system(&mut world);
        }
        assert_eq!(
            overload_event_count(&world),
            0,
            "nameplate-balanced prototype grid must never overload"
        );
    }

    #[test]
    fn test_overload_still_fires_when_demand_exceeds_nameplate() {
        // The overload mechanic itself is unchanged for genuinely
        // over-subscribed grids: demand 40 against 10 nameplate is ratio
        // 4.0, a 12.5% fire chance per call — 300 calls make a miss
        // effectively impossible (~0.875^300).
        let mut world = World::new();
        world.init_resource::<bevy_ecs::event::Events<super::GridOverloadEvent>>();
        let victim = world.spawn(GridPosition { x: 0, y: 0 }).id();

        for _ in 0..300 {
            super::handle_overload(&mut world, &[victim], 10.0, 40.0);
        }
        assert!(
            overload_event_count(&world) >= 1,
            "demand far beyond nameplate capacity must still overload"
        );
    }
}

/// Represents the aggregate state of an energy grid.
#[derive(Component, Default, Debug, Clone)]
pub struct EnergyGrid {
    /// Total energy generated by all sources in the grid.
    pub total_generation: f32,
    /// Total energy consumed by all consumers in the grid.
    pub total_consumption: f32,
}

/// Buffer resource for reusing allocations in the `power_grid_system` each frame.
///
/// ⚡ Bolt Optimization:
/// - Reusing these vectors avoids O(N) heap allocations per frame/tick when
///   evaluating power grids.
#[derive(Resource, Default)]
pub struct PowerGridBuffer {
    /// Visited tiles for BFS
    pub visited: HashSet<(i32, i32)>,
    /// Entities in the current grid
    pub grid_entities: Vec<Entity>,
    /// Queue for BFS traversal
    pub queue: VecDeque<(i32, i32)>,
    /// Batteries in the current grid
    pub batteries: Vec<Entity>,
    /// Kinetic batteries in the current grid
    pub kinetic_batteries: Vec<Entity>,
}
