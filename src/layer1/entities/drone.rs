//! Drone entities and automation logic.
//!
//! This module defines the `Drone` entity and its associated states and systems.
//! Drones are automated workers that operate without the complex needs of normal `Pop`s,
//! requiring only a connection to a `CommandCenter` and access to a `DroneHub` for recharging.
//!
//! If a drone loses its connection to a command center, it becomes feral, attacking
//! nearby pops and hoarding resources.

use crate::layer1::building::Building;
use crate::layer1::energy::PowerConsumer;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::map::GridPosition;
use crate::layer1::physics::pressure::SUFFOCATION_PRESSURE_THRESHOLD;
use crate::layer1::pressure::PressureGrid;
use crate::layer1::structure::{DeferMaintenance, Structure};
use crate::layer1::utility_ai::{manhattan_distance, ActionType, PopAction, StartPlan};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// State of a drone.
///
/// Indicates what the drone is currently doing, such as hauling or if it has gone feral.
///
/// # Examples
///
/// ```
/// use scale::layer1::entities::drone::{Drone, DroneState};
///
/// let my_drone = Drone {
///     state: DroneState::Idle,
/// };
///
/// assert_eq!(my_drone.state, DroneState::Idle);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DroneState {
    /// Drone is idle and waiting for tasks.
    #[default]
    Idle,
    /// Drone is currently hauling resources.
    Hauling,
    /// Drone is travelling to or repairing a damaged building (Spec 1385).
    Repairing,
    /// Drone has lost connection to the Command Center and gone feral.
    Feral,
}

/// HP of repair work a drone applies per tick (Spec 1385 §4.1).
///
/// Deliberately the pop repair rate: a drone is a tireless pop-rate
/// welder, not a better one — its advantage is that it can stand in
/// vacuum forever, not that it welds faster.
pub const DRONE_REPAIR_PER_TICK: f32 = 10.0;

/// One damaged-building candidate for drone repair dispatch (Spec 1385 §4.2).
#[derive(Debug, Clone, Copy)]
pub struct RepairCandidate {
    /// The building entity.
    pub entity: Entity,
    /// The building's tile.
    pub pos: GridPosition,
    /// `1.0 - current_hp / max_hp`, in `(0.0, 1.0]` for real candidates.
    pub damage_fraction: f32,
    /// Tile pressure is below the suffocation threshold: the building
    /// is exposed to vacuum, where pops pay 1 HP/tick to work.
    pub exterior: bool,
}

/// Picks the drone repair target (Spec 1385 §4.2).
///
/// Ordering is lexicographic: **exterior first**, then **damage
/// fraction descending**, then **Manhattan distance ascending**.
/// Deterministic on full ties (the earlier candidate wins). Candidates
/// with no damage are ignored; `None` when nothing needs repair.
pub fn pick_repair_target(candidates: &[RepairCandidate], from: GridPosition) -> Option<Entity> {
    let mut best: Option<&RepairCandidate> = None;
    for candidate in candidates {
        if candidate.damage_fraction <= 0.0 {
            continue;
        }
        let replace = match best {
            None => true,
            Some(current) => {
                if candidate.exterior != current.exterior {
                    candidate.exterior
                } else if (candidate.damage_fraction - current.damage_fraction).abs() > f32::EPSILON
                {
                    candidate.damage_fraction > current.damage_fraction
                } else {
                    manhattan_distance(&from, &candidate.pos)
                        < manhattan_distance(&from, &current.pos)
                }
            }
        };
        if replace {
            best = Some(candidate);
        }
    }
    best.map(|candidate| candidate.entity)
}

/// Marker component for Drone entities.
///
/// Drones are automated workers that require power (Battery) but have no needs.
#[derive(Component, Default)]
pub struct Drone {
    /// The current operational state of the drone.
    pub state: DroneState,
}

/// Component indicating a Feral Drone.
#[derive(Component, Default)]
pub struct FeralDrone {
    /// Resources hoarded by the feral drone.
    pub hoard: Vec<crate::layer1::resources::ResourceItem>,
}

/// Component indicating a drone's connection state to a Command Center.
#[derive(Component, Default)]
pub struct GridConnection {
    /// True if the drone is currently connected to power
    pub is_connected: bool,
    /// Number of ticks the drone has been disconnected
    pub time_disconnected: u64,
}

/// Component linking a drone to a Command Center.
#[derive(Component)]
pub struct ConnectedTo(pub Entity);

/// Event fired when a drone loses connection to a Command Center.
#[derive(Event)]
pub struct DroneDisconnectedEvent {
    /// The drone entity that disconnected.
    pub drone: Entity,
}

/// Marker component for Drone Hub buildings.
///
/// Drone Hubs consume power and allow Drones to recharge.
#[derive(Component, Default)]
pub struct DroneHub;

/// Component tracking internal battery charge for Drones.
#[derive(Component, Clone, Copy, Debug)]
pub struct DroneBattery {
    /// Current charge level.
    pub current: f32,
    /// Maximum charge capacity.
    pub max: f32,
}

/// Evaluates drone actions (Spec 1385 §4.3).
///
/// Priority law: **Charge > Repair > Haul**. Charging if battery is
/// low (< 20%) is absolute and recalls even a repairing drone. Repair
/// dispatch targets the colony's most urgent damaged building —
/// exterior (vacuum-exposed) buildings first, then by damage
/// fraction, then distance — via [`pick_repair_target`]. Hauling is
/// downstream filler, assigned by `drone_work_bridge_system`.
#[allow(clippy::type_complexity)]
pub fn evaluate_drone_actions_system(
    mut commands: Commands,
    mut drone_query: Query<
        (
            Entity,
            &mut Drone,
            &DroneBattery,
            &mut PopAction,
            &GridPosition,
            Option<&GridConnection>,
            Option<&FeralDrone>,
            Option<&MovementTarget>,
        ),
        With<Drone>,
    >,
    hub_query: Query<(Entity, &GridPosition, &PowerConsumer), (With<DroneHub>, With<Building>)>,
    building_query: Query<
        (
            Entity,
            &GridPosition,
            &Structure,
            Has<DeferMaintenance>,
            Has<crate::layer1::heirloom::AncientStructure>,
        ),
        With<Building>,
    >,
    pressure_grid: Option<Res<PressureGrid>>,
) {
    // Candidate set is colony state, not drone state: build it once.
    let mut candidates: Vec<RepairCandidate> = Vec::new();
    for (entity, pos, structure, deferred, ancient) in building_query.iter() {
        if deferred || ancient || structure.max_hp <= 0.0 {
            continue;
        }
        let damage_fraction = 1.0 - (structure.current_hp / structure.max_hp);
        if damage_fraction <= 0.0 {
            continue;
        }
        let exterior = pressure_grid
            .as_deref()
            .is_some_and(|grid| grid.get(pos.x, pos.y) < SUFFOCATION_PRESSURE_THRESHOLD);
        candidates.push(RepairCandidate {
            entity,
            pos: *pos,
            damage_fraction,
            exterior,
        });
    }

    for (entity, mut drone, battery, mut action, pos, connection, feral, movement_target) in
        &mut drone_query
    {
        let battery_pct = battery.current / battery.max;

        // 1. Charge if low — battery law outranks repair (Spec 1385 §4.3).
        if battery_pct < 0.2 {
            if action.current != ActionType::Charge {
                // Find nearest active Hub
                let mut best_hub = None;
                let mut min_dist = i32::MAX;

                for (hub_entity, hub_pos, power) in hub_query.iter() {
                    if !power.active {
                        continue;
                    }
                    let dist = manhattan_distance(pos, hub_pos);
                    if dist < min_dist {
                        min_dist = dist;
                        best_hub = Some(hub_entity);
                    }
                }

                if let Some(hub) = best_hub {
                    if action.current == ActionType::Repair {
                        drone.state = DroneState::Idle;
                    }
                    action.current = ActionType::Charge;
                    action.current_utility = 1.0;
                    action.ticks_committed = 0;

                    // Trigger movement via StartPlan
                    commands.entity(entity).insert(StartPlan {
                        action: ActionType::Charge,
                        target: Some(hub),
                    });
                }
            }
            continue;
        }

        // 2. Resume Idle if fully charged (and was charging)
        if action.current == ActionType::Charge && battery_pct >= 0.99 {
            action.current = ActionType::Idle;
            action.current_utility = 0.5;
            action.ticks_committed = 0;
            commands.entity(entity).remove::<StartPlan>(); // Stop moving to charger
        }

        // 3. Repair dispatch (Spec 1385). Feral drones repair nothing;
        // disconnected drones hold position instead of freelancing. A
        // drone with no GridConnection is treated as connected (the
        // pre-existing spawn shape; disconnection only ever resolves
        // into Feral, which is checked first).
        if feral.is_some() || drone.state == DroneState::Feral {
            continue;
        }
        if connection.is_some_and(|c| !c.is_connected) {
            continue;
        }
        if !matches!(
            action.current,
            ActionType::Idle | ActionType::Haul | ActionType::Repair
        ) {
            continue;
        }

        match pick_repair_target(&candidates, *pos) {
            Some(target) => {
                let already_en_route = action.current == ActionType::Repair
                    && movement_target.is_some_and(|mt| {
                        mt.for_action == ActionType::Repair && mt.target_entity == target
                    });
                if !already_en_route {
                    action.current = ActionType::Repair;
                    action.current_utility = 0.9;
                    action.ticks_committed = 0;
                    drone.state = DroneState::Repairing;
                    commands.entity(entity).insert(StartPlan {
                        action: ActionType::Repair,
                        target: Some(target),
                    });
                }
            }
            None => {
                // Nothing damaged: release a stale repair assignment.
                // (Idle/Haul drones are untouched; the bridge owns Haul.)
                if action.current == ActionType::Repair {
                    action.current = ActionType::Idle;
                    action.current_utility = 0.0;
                    action.ticks_committed = 0;
                    drone.state = DroneState::Idle;
                    commands
                        .entity(entity)
                        .remove::<StartPlan>()
                        .remove::<MovementTarget>()
                        .remove::<AtTarget>();
                }
            }
        }

        // 4. Haul is assigned downstream by drone_work_bridge_system.
    }
}

/// Executes drone repair work (Spec 1385 §4.4).
///
/// Exclusive system in the shape of `work_execution_system`, because it
/// calls the same world-shaped repair pipeline: a drone that has
/// arrived (`AtTarget`) at its repair building applies
/// [`DRONE_REPAIR_PER_TICK`] through `structure::process_repair`, so
/// quirks reveal, ancient refusal, and HP capping behave identically
/// whoever holds the welder. On completion the drone returns to Idle,
/// mirroring pop work cleanup.
pub fn drone_repair_system(world: &mut World) {
    let mut workers: Vec<(Entity, Entity)> = Vec::new();
    {
        let mut query = world
            .query_filtered::<(Entity, &PopAction, &MovementTarget), (With<Drone>, With<AtTarget>)>(
            );
        for (entity, action, movement_target) in query.iter(world) {
            if action.current == ActionType::Repair
                && movement_target.for_action == ActionType::Repair
            {
                workers.push((entity, movement_target.target_entity));
            }
        }
    }

    for (drone_entity, target) in workers {
        let complete = if world.get_entity(target).is_err() {
            true
        } else {
            crate::layer1::structure::process_repair(world, target, DRONE_REPAIR_PER_TICK)
        };

        if complete {
            world
                .entity_mut(drone_entity)
                .remove::<MovementTarget>()
                .remove::<AtTarget>();
            if let Some(mut action) = world.get_mut::<PopAction>(drone_entity) {
                action.current = ActionType::Idle;
                action.current_utility = 0.0;
                action.ticks_committed = 0;
            }
            if let Some(mut drone) = world.get_mut::<Drone>(drone_entity) {
                if drone.state == DroneState::Repairing {
                    drone.state = DroneState::Idle;
                }
            }
        }
    }
}

/// Checks if a drone has lost connection to its Command Center.
pub fn check_drone_connection(
    mut commands: Commands,
    mut drones: Query<(Entity, &mut Drone, &ConnectedTo, &mut GridConnection)>,
    command_centers: Query<&PowerConsumer, With<Building>>,
    mut disconnect_events: EventWriter<DroneDisconnectedEvent>,
) {
    const FERAL_THRESHOLD: u64 = 5000;

    for (entity, mut drone, connection, mut grid_connection) in drones.iter_mut() {
        let mut disconnected = false;
        // Find the command center entity
        if let Ok(power) = command_centers.get(connection.0) {
            // Check if it is powered
            if !power.active {
                disconnected = true;
            }
        } else {
            // Command center is destroyed or missing PowerConsumer
            disconnected = true;
        }

        if disconnected {
            grid_connection.is_connected = false;
            grid_connection.time_disconnected += 1;
        } else {
            grid_connection.is_connected = true;
            grid_connection.time_disconnected = 0;
        }

        if !grid_connection.is_connected && grid_connection.time_disconnected >= FERAL_THRESHOLD {
            commands.entity(entity).remove::<ConnectedTo>();
            commands.entity(entity).insert(FeralDrone::default());
            drone.state = DroneState::Feral;
            disconnect_events.send(DroneDisconnectedEvent { drone: entity });
        }
    }
}

/// Processes feral drones: hoarding nearby resources and attacking pops.
pub fn process_feral_drones(
    mut commands: Commands,
    mut feral_drones: Query<(&mut FeralDrone, &GridPosition)>,
    resources: Query<(
        Entity,
        &crate::layer1::resources::ResourceItem,
        &GridPosition,
    )>,
    mut pops: Query<
        (&mut crate::layer1::health::Health, &GridPosition),
        With<crate::layer1::pop::Pop>,
    >,
) {
    for (mut feral_drone, drone_pos) in feral_drones.iter_mut() {
        // Reproduce if we have enough hoarded resources
        if feral_drone.hoard.len() >= 5 {
            feral_drone.hoard.clear();
            commands.spawn((
                Drone {
                    state: DroneState::Feral,
                },
                FeralDrone::default(),
                *drone_pos,
                PopAction::default(),
                DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
            ));
            // Skip gathering this tick since we just reproduced
            continue;
        }

        // Hoard nearby resources (within 2 tiles distance)
        for (res_entity, resource, res_pos) in resources.iter() {
            if manhattan_distance(drone_pos, res_pos) < 2 {
                feral_drone.hoard.push(*resource);
                commands.entity(res_entity).despawn();
                break; // Only pick up one per tick
            }
        }

        // Attack nearby pops (within 2 tiles distance)
        for (mut health, pop_pos) in pops.iter_mut() {
            if manhattan_distance(drone_pos, pop_pos) < 2 {
                health.current -= 10.0;
            }
        }
    }
}

/// Charges drones when they are at a Hub and performing `ActionType::Charge`.
pub fn process_charge_system(
    mut drone_query: Query<(Entity, &mut DroneBattery, &PopAction, &GridPosition), With<Drone>>,
    hub_query: Query<(&GridPosition, &PowerConsumer), With<DroneHub>>,
) {
    for (_entity, mut battery, action, pos) in &mut drone_query {
        if action.current == ActionType::Charge {
            // Check if at any ACTIVE hub location
            let at_active_hub = hub_query
                .iter()
                .any(|(hub_pos, power)| hub_pos == pos && power.active);

            if at_active_hub {
                battery.current = (battery.current + 1.0).min(battery.max);
            }
        }
    }
}

/// Drains drone battery over time.
pub fn drone_battery_system(mut query: Query<(&mut DroneBattery, &PopAction), With<Drone>>) {
    for (mut battery, action) in &mut query {
        let drain = match action.current {
            ActionType::Idle => 0.05,
            ActionType::Charge => 0.0, // Don't drain while charging logic runs (it net gains)
            _ => 0.1,                  // Work harder
        };

        battery.current = (battery.current - drain).max(0.0);
    }
}

pub fn drone_spawner_bridge_system(
    mut commands: Commands,
    hubs: Query<
        (Entity, &GridPosition, &crate::layer1::energy::PowerConsumer),
        With<crate::layer1::drone::DroneHub>,
    >,
    drones: Query<&crate::layer1::drone::Drone>,
    _time: Res<SimulationTime>,
) {
    // Limit total drones to 3 * Hubs
    let hub_count = hubs.iter().count();
    if hub_count == 0 {
        return;
    }

    let drone_count = drones.iter().count();
    let max_drones = hub_count * 3;

    if drone_count >= max_drones {
        return;
    }

    // Spawn 1 drone per tick max
    for (hub_entity, pos, power) in hubs.iter() {
        if power.active {
            // Spawn drone
            commands.spawn((
                crate::layer1::drone::Drone {
                    state: crate::layer1::drone::DroneState::Idle,
                },
                crate::layer1::drone::ConnectedTo(hub_entity),
                crate::layer1::drone::GridConnection::default(),
                *pos,
                crate::layer1::utility_ai::PopAction::default(),
                crate::layer1::drone::DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
                crate::layer1::utility_ai::UtilityWeights::default(),
            ));
            break; // Only one per tick
        }
    }
}

pub fn drone_work_bridge_system(
    mut query: Query<
        (
            &mut crate::layer1::utility_ai::PopAction,
            &crate::layer1::drone::DroneBattery,
        ),
        With<crate::layer1::drone::Drone>,
    >,
) {
    for (mut action, battery) in &mut query {
        // If idle and battery > 20%, start hauling
        // Drones handle charging logic in evaluate_drone_actions_system which sets action to Charge.
        // We only override Idle.
        if action.current == crate::layer1::utility_ai::ActionType::Idle && battery.current > 20.0 {
            action.current = crate::layer1::utility_ai::ActionType::Haul;
            action.current_utility = 0.8; // High utility to persist
            action.ticks_committed = 0;
        }
    }
}
