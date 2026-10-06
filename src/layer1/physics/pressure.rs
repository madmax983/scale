//! The Atmospheric Pressure System.
//!
//! This module simulates the dispersion, containment, and loss of atmospheric pressure
//! within the colony. Because pops require oxygen to survive, maintaining a pressurized
//! environment is essential. If a room's pressure drops to a vacuum (0.0), occupants
//! will quickly take suffocation damage.
//!
//! Pressure is simulated as a cellular automata over a 2D grid (`PressureGrid`). Generators
//! (like `LifeSupport` buildings) inject pressure into the grid, which then diffuses to
//! adjacent cells. Blockers (like `Wall` or `Airlock` buildings) restrict or halt this
//! flow, allowing you to create sealed environments.
//!
//! # Examples
//!
//! ```
//! use bevy::prelude::*;
//! use scale::layer1::physics::pressure::{PressureGrid, update_pressure_system};
//! use scale::layer1::building::{Building, BuildingType};
//! use scale::layer1::Structure;
//! use scale::layer1::map::GridPosition;
//!
//! let mut app = App::new();
//! app.add_plugins(MinimalPlugins);
//!
//! // 1. Create a 5x5 grid (defaults to 0.0 vacuum)
//! let grid = PressureGrid::new(5, 5);
//! app.insert_resource(grid);
//!
//! // 2. Add a Life Support generator
//! app.world_mut().spawn((
//!     Building { building_type: BuildingType::LifeSupport },
//!     GridPosition { x: 2, y: 2 },
//!     Structure::default(),
//! ));
//!
//! // 3. Run the system to generate and diffuse pressure
//! app.add_systems(Update, update_pressure_system);
//! app.update();
//!
//! // The grid now has pressure near the generator!
//! let pressure = app.world().resource::<PressureGrid>().get(2, 2);
//! assert!(pressure > 0.0);
//! ```
//!
//! # Details
//! - **Diffusion Algorithm:** Pressure diffuses by averaging a cell with its 4 cardinal
//!   neighbors, weighted by their `transmissivity` (0.0 for walls, 1.0 for empty space).
//! - **Vacuum Decay:** If an area is exposed to the void (edges of the grid or breached
//!   walls), pressure will rapidly equalize with the vacuum, depleting the room.

use bevy::utils::HashMap;
use bevy_ecs::prelude::*;

/// Represents the atmospheric pressure layer.
/// Values range from 0.0 (Vacuum) to 1.0 (Standard Atmosphere).
#[derive(Resource)]
pub struct PressureGrid {
    /// Width of the grid.
    pub(crate) width: usize,
    /// Height of the grid.
    pub(crate) height: usize,
    /// Flattened grid values.
    pub(crate) values: Vec<f32>,
    /// Secondary buffer for diffusion calculation (double buffering).
    /// Used to avoid allocating a new vector every tick.
    pub(crate) scratch: Vec<f32>,
}

impl PressureGrid {
    /// Fills the entire grid with a value.
    pub fn fill(&mut self, value: f32) {
        self.values.fill(value);
    }

    /// Create a new empty pressure grid (initialized to 0.0).
    #[must_use]
    pub fn new(mut width: usize, mut height: usize) -> Self {
        let mut size = width.saturating_mul(height);
        if size > 10_000_000 {
            width = 1;
            height = 1;
            size = 1;
        }
        Self {
            width,
            height,
            values: vec![0.0; size],
            scratch: vec![0.0; size],
        }
    }

    /// Get pressure at (x, y). Returns 0.0 if out of bounds.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 {
            return 0.0;
        }
        let ux = x as usize;
        let uy = y as usize;
        if ux >= self.width || uy >= self.height {
            return 0.0;
        }
        let idx = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux));

        if let Some(idx) = idx.filter(|&i| i < self.values.len()) {
            return self.values[idx];
        }
        0.0
    }

    /// Set pressure at (x, y). Clamped between 0.0 and 1.0.
    #[allow(clippy::cast_sign_loss)]
    pub fn set(&mut self, x: i32, y: i32, value: f32) {
        if x < 0 || y < 0 {
            return;
        }
        let ux = x as usize;
        let uy = y as usize;
        if ux >= self.width || uy >= self.height {
            return;
        }
        let idx = uy.checked_mul(self.width).and_then(|i| i.checked_add(ux));

        if let Some(idx) = idx.filter(|&i| i < self.values.len()) {
            self.values[idx] = value.clamp(0.0, 1.0);
        }
    }

    /// Add pressure at (x, y). Clamped to max 1.0.
    pub fn add(&mut self, x: i32, y: i32, amount: f32) {
        let current = self.get(x, y);
        self.set(x, y, current + amount);
    }

    /// Simulate diffusion of pressure.
    ///
    /// # Arguments
    ///
    /// * `blockers` - Map of (x, y) to transmissivity (0.0 = Wall, 1.0 = Open).
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss
    )]
    pub fn diffuse(&mut self, blockers: &HashMap<(i32, i32), f32>) {
        // Ensure scratch buffer size matches (in case of dynamic resizing, though rare)
        if self.scratch.len() != self.values.len() {
            self.scratch = vec![0.0; self.values.len()];
        }
        self.scratch.copy_from_slice(&self.values);

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y
                    .checked_mul(self.width)
                    .and_then(|i| i.checked_add(x))
                    .unwrap_or(usize::MAX);

                // If the cell itself is a solid blocker (Wall), it contains no pressure.
                if blockers
                    .get(&(x as i32, y as i32))
                    .is_some_and(|&trans| trans <= f32::EPSILON)
                {
                    if let Some(v) = self.scratch.get_mut(idx) {
                        *v = 0.0;
                    }
                    continue;
                }

                let current_val = self.values.get(idx);
                let Some(&current_val) = current_val else {
                    continue;
                };

                let mut sum = current_val;
                let mut total_weight = 1.0;

                // Check 4 neighbors
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if nx >= 0
                        && ny >= 0
                        && (nx as usize) < self.width
                        && (ny as usize) < self.height
                    {
                        let neighbor_val = self.get(nx, ny);
                        let neighbor_trans = *blockers.get(&(nx, ny)).unwrap_or(&1.0);

                        if neighbor_trans > f32::EPSILON {
                            sum += neighbor_val * neighbor_trans;
                            total_weight += neighbor_trans;
                        }
                    } else {
                        // Edge is vacuum (0.0 pressure, 1.0 transmissivity)
                        sum += 0.0;
                        total_weight += 1.0;
                    }
                }

                if total_weight > 0.0 {
                    if let Some(v) = self.scratch.get_mut(idx) {
                        *v = sum / total_weight;
                    }
                }
            }
        }
        std::mem::swap(&mut self.values, &mut self.scratch);
    }
}

/// System to update atmospheric pressure.
pub fn update_pressure_system(
    grid_opt: Option<ResMut<PressureGrid>>,
    blocker_query: Query<(
        &crate::layer1::building::Building,
        &crate::layer1::map::GridPosition,
        Option<&crate::layer1::control::DoorControl>,
    )>,
    generator_query: Query<(
        &crate::layer1::building::Building,
        &crate::layer1::map::GridPosition,
        Option<&crate::layer1::energy::PowerConsumer>,
    )>,
    vents: Query<(
        &crate::layer1::map::GridPosition,
        &crate::layer1::physics::vent::VentConnection,
    )>,
) {
    use crate::layer1::building::BuildingType;

    let Some(mut grid) = grid_opt else { return };

    // 1. Identify blockers
    // Pre-sizing hashmap using blocker_query size estimate, though not all buildings have flow_transmissivity.
    let mut blockers =
        HashMap::with_capacity_and_hasher(blocker_query.iter().len() / 2, Default::default());
    for (b, pos, control) in blocker_query.iter() {
        let mut transmissivity = b.building_type.flow_transmissivity();

        if let Some(ctrl) = control {
            match ctrl.state {
                crate::layer1::control::DoorState::Open => transmissivity = Some(1.0),
                crate::layer1::control::DoorState::Locked => transmissivity = Some(0.0),
                crate::layer1::control::DoorState::Auto => {}
            }
        }

        if let Some(t) = transmissivity {
            blockers.insert((pos.x, pos.y), t);
        }
    }

    for (pos, vent) in vents.iter() {
        let airflow = crate::layer1::physics::vent::calculate_vent_airflow(vent);
        blockers.insert((pos.x, pos.y), airflow);
    }

    // 2. Identify Generators and apply to Grid directly without intermediate Vec
    for (b, pos, power) in generator_query.iter() {
        if b.building_type == BuildingType::LifeSupport {
            let is_active = power.is_none_or(|p| p.active);
            if is_active {
                let current = grid.get(pos.x, pos.y);
                if current < 1.0 {
                    grid.set(pos.x, pos.y, (current + 0.2).min(1.0));
                }
            }
        }
    }

    // 3. Diffuse
    grid.diffuse(&blockers);
}

/// HP per tick that a damaged LifeSupport unit restores on its own.
///
/// LifeSupport units carry self-diagnostic maintenance systems that slowly
/// knit sabotage damage closed. This is the colony's *reliable* counterplay
/// layer: unlike crew repair (which depends on the utility AI noticing),
/// self-repair always runs. At 0.25 HP/tick it loses to active sabotage
/// (1.5–3.0 HP/tick average), so sabotage still hurts — but the window for
/// the crew to respond stretches, and lone saboteurs can't kill a unit
/// faster than it heals between strikes.
pub const LIFESUPPORT_SELF_REPAIR_PER_TICK: f32 = 0.25;

/// Self-repair system: damaged LifeSupport units slowly restore integrity.
///
/// Runs every tick. Only repairs upward toward max HP; never resurrects a
/// destroyed (0 HP) unit — once it's gone, it's gone.
pub fn lifesupport_self_repair_system(
    mut query: Query<(
        &crate::layer1::building::Building,
        &mut crate::layer1::structure::Structure,
    )>,
) {
    use crate::layer1::building::BuildingType;

    for (building, mut structure) in query.iter_mut() {
        if building.building_type != BuildingType::LifeSupport {
            continue;
        }
        if structure.current_hp > 0.0 && structure.current_hp < structure.max_hp {
            structure.current_hp =
                (structure.current_hp + LIFESUPPORT_SELF_REPAIR_PER_TICK).min(structure.max_hp);
        }
    }
}

/// Per-entity alert state for LifeSupport damage warnings.
///
/// Hysteresis bands keep the chronicle from spamming while a unit sits below
/// a threshold: each band warns once, and both reset only after repairs bring
/// integrity back above [`LIFESUPPORT_WARNING_RESET`].
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct LifeSupportAlertState {
    warned_half: bool,
    warned_quarter: bool,
}

/// Integrity fraction that triggers the "LifeSupport at 50%" warning.
pub const LIFESUPPORT_WARNING_HALF: f32 = 0.5;
/// Integrity fraction that triggers the "LifeSupport failing" warning.
pub const LIFESUPPORT_WARNING_QUARTER: f32 = 0.25;
/// Integrity fraction that resets the warning bands after repair.
pub const LIFESUPPORT_WARNING_RESET: f32 = 0.65;

/// Early-warning system: chronicles LifeSupport damage before it becomes
/// unrecoverable.
///
/// Mind-spore sabotage chews through LifeSupport HP in silence; once a unit
/// hits 0 HP, malfunction fires finish it and the colony depressurizes. This
/// gives the repair counterplay a visible trigger: at 50% integrity the crew
/// is alerted, at 25% it's an emergency.
pub fn lifesupport_damage_warning_system(    mut commands: Commands,
    query: Query<(
        Entity,
        &crate::layer1::building::Building,
        &crate::layer1::structure::Structure,
        Option<&LifeSupportAlertState>,
    )>,
    mut chronicle: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    use crate::layer1::building::BuildingType;
    use crate::layer1::chronicle::EventImportance;

    for (entity, building, structure, alert) in query.iter() {
        if building.building_type != BuildingType::LifeSupport || structure.max_hp <= 0.0 {
            continue;
        }
        let fraction = (structure.current_hp / structure.max_hp).clamp(0.0, 1.0);
        let mut state = alert.copied().unwrap_or_default();
        let mut changed = false;

        if fraction >= LIFESUPPORT_WARNING_RESET {
            // Repaired past the reset band: re-arm warnings.
            if state.warned_half || state.warned_quarter {
                state = LifeSupportAlertState::default();
                changed = true;
            }
        } else if fraction < LIFESUPPORT_WARNING_QUARTER && !state.warned_quarter {
            chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
                text: "EMERGENCY: Life support failing! Repair crews must respond or the air goes with it.".to_string(),
                importance: EventImportance::Major,
            });
            state.warned_quarter = true;
            state.warned_half = true;
            changed = true;
        } else if fraction < LIFESUPPORT_WARNING_HALF && !state.warned_half {
            chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
                text: "Life support integrity at 50% — sabotage damage needs repair soon."
                    .to_string(),
                importance: EventImportance::Standard,
            });
            state.warned_half = true;
            changed = true;
        }

        if changed {
            commands.entity(entity).insert(state);
        }
    }
}

/// System to apply suffocation damage.
#[allow(clippy::type_complexity)]
pub fn pressure_damage_system(
    mut commands: Commands,
    mut pop_query: Query<
        (
            Entity,
            &crate::layer1::map::GridPosition,
            &mut crate::layer1::health::Health,
        ),
        (
            With<crate::layer1::pop::Pop>,
            // The phased Chronostalker is out of the time stream:
            // vacuum cannot touch what is not, momentarily, there.
            Without<crate::layer1::culture::chronostalker::Phased>,
        ),
    >,
    grid_opt: Option<Res<PressureGrid>>,
    mut events: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    use crate::layer1::chronicle::EventImportance;

    let Some(grid) = grid_opt else { return };
    let mut killed = 0;

    for (entity, pos, mut health) in pop_query.iter_mut() {
        if health.current <= 0.0 {
            continue;
        }

        let pressure = grid.get(pos.x, pos.y);
        if pressure < 0.2 {
            health.take_damage(1.0);
            if health.current <= 0.0 {
                commands
                    .entity(entity)
                    .insert(crate::layer1::health::DeathCause(
                        "Atmospheric breach".to_string(),
                    ));
                killed += 1;
            }
        }
    }

    if killed > 0 {
        events.send(crate::layer1::chronicle::AddChronicleEvent {
            text: format!("{} Pops suffocated due to atmospheric breach.", killed),
            importance: EventImportance::Major,
        });
    }
}

pub fn process_door_venting_system(
    mut grid: ResMut<PressureGrid>,
    query: Query<(
        &crate::layer1::building::Building,
        &crate::layer1::control::DoorControl,
        &crate::layer1::map::GridPosition,
    )>,
) {
    for (building, control, pos) in query.iter() {
        if control.state == crate::layer1::control::DoorState::Open {
            let vent_rate =
                if building.building_type == crate::layer1::building::BuildingType::Airlock {
                    0.04
                } else {
                    0.5
                };

            // Diffuse X and Y neighbors across the door
            for (dx, dy) in [(-1, 0), (0, -1)] {
                let p1 = grid.get(pos.x + dx, pos.y + dy);
                let p2 = grid.get(pos.x - dx, pos.y - dy);
                let diff = (p1 - p2) * vent_rate;
                grid.set(pos.x + dx, pos.y + dy, p1 - diff);
                grid.set(pos.x - dx, pos.y - dy, p2 + diff);
            }
        }
    }
}

pub fn apply_door_movement_penalties_system(
    mut pop_query: Query<
        (
            &crate::layer1::map::GridPosition,
            &mut crate::layer1::pop::Speed,
        ),
        With<crate::layer1::pop::Pop>,
    >,
    door_query: Query<(
        &crate::layer1::map::GridPosition,
        &crate::layer1::building::Building,
    )>,
) {
    // ⚡ Bolt Optimization: Uses bevy::utils::HashSet (AHash) to eliminate SipHash overhead on integer keys.
    let mut airlock_positions = bevy::utils::HashSet::with_capacity(door_query.iter().len());
    for (door_pos, building) in door_query.iter() {
        if building.building_type == crate::layer1::building::BuildingType::Airlock {
            airlock_positions.insert((door_pos.x, door_pos.y));
        }
    }

    for (pop_pos, mut speed) in pop_query.iter_mut() {
        if airlock_positions.contains(&(pop_pos.x, pop_pos.y)) {
            speed.current *= 0.5; // Slow down
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::health::Health;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::structure::Structure;

    #[test]
    fn test_pressure_grid_defaults_to_vacuum() {
        let grid = PressureGrid::new(10, 10);
        assert_eq!(grid.get(5, 5), 0.0);
    }

    #[test]
    fn test_life_support_generates_pressure() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        let grid = PressureGrid::new(10, 10);
        app.insert_resource(grid);

        app.world_mut().spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 5, y: 5 },
        ));

        app.add_systems(bevy::prelude::Update, update_pressure_system);
        app.update();

        let grid = app.world().resource::<PressureGrid>();
        assert!(
            grid.get(5, 5) > 0.0,
            "Life Support should generate pressure"
        );
    }

    #[test]
    fn test_walls_block_diffusion() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        let mut grid = PressureGrid::new(5, 1);
        grid.set(1, 0, 1.0); // Source
        app.insert_resource(grid);

        // Wall at (2, 0)
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x: 2, y: 0 },
            Structure::default(),
        ));

        app.add_systems(bevy::prelude::Update, update_pressure_system);

        for _ in 0..5 {
            app.update();
        }

        let grid = app.world().resource::<PressureGrid>();
        // (1,0) source. (2,0) wall. (3,0) target.
        assert!(
            grid.get(3, 0) < 0.01,
            "Pressure should not pass through wall"
        );
    }

    #[test]
    fn test_gate_leaks_pressure() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        let mut grid = PressureGrid::new(5, 1);
        grid.set(1, 0, 1.0);
        app.insert_resource(grid);

        // Gate at (2, 0)
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Gate,
            },
            GridPosition { x: 2, y: 0 },
            Structure::default(),
        ));

        app.add_systems(bevy::prelude::Update, update_pressure_system);

        // Run multiple ticks
        for _ in 0..5 {
            app.update();
        }

        let grid = app.world().resource::<PressureGrid>();
        assert!(grid.get(3, 0) > 0.0, "Pressure SHOULD leak through Gate");
    }

    #[test]
    fn test_airlock_blocks_pressure() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        let mut grid = PressureGrid::new(5, 1);
        grid.set(1, 0, 1.0);
        app.insert_resource(grid);

        // Airlock at (2, 0)
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Airlock,
            },
            GridPosition { x: 2, y: 0 },
            Structure::default(),
        ));

        app.add_systems(bevy::prelude::Update, update_pressure_system);

        for _ in 0..5 {
            app.update();
        }

        let grid = app.world().resource::<PressureGrid>();
        assert!(
            grid.get(3, 0) < 0.01,
            "Pressure should NOT leak through Airlock"
        );
    }

    #[test]
    fn test_suffocation_damage() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        let grid = PressureGrid::new(10, 10);
        app.insert_resource(grid);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, pressure_damage_system);
        app.update();

        let health = app.world().get::<Health>(pop).unwrap();
        assert!(health.current < 100.0, "Pop in vacuum should take damage");
    }

    #[test]
    fn test_pressure_passes_through_vent() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        let mut grid = PressureGrid::new(5, 1);
        grid.set(0, 0, 1.0); // Source
        app.insert_resource(grid);

        // Source generator at (0, 0) to maintain pressure against vacuum
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Vent at (1, 0)
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::Vent,
            },
            GridPosition { x: 1, y: 0 },
        ));

        app.add_systems(bevy::prelude::Update, update_pressure_system);

        // Run pressure update multiple times to allow diffusion
        for _ in 0..20 {
            // Manually refill source to fight vacuum decay for test purposes
            app.world_mut()
                .resource_mut::<PressureGrid>()
                .set(0, 0, 1.0);
            app.update();
        }

        let grid = app.world().resource::<PressureGrid>();
        assert!(grid.get(2, 0) > 0.05, "Pressure SHOULD pass through Vent");
    }

    #[test]
    fn test_door_vents_atmosphere_on_open() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let _door_entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Gate,
                },
                crate::layer1::control::DoorControl {
                    state: crate::layer1::control::DoorState::Open,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        app.world_mut().insert_resource(PressureGrid::new(20, 20));
        app.world_mut()
            .resource_mut::<PressureGrid>()
            .set(9, 10, 1.0);
        app.world_mut()
            .resource_mut::<PressureGrid>()
            .set(11, 10, 0.0);

        app.add_systems(bevy::prelude::Update, process_door_venting_system);
        app.update();

        let grid = app.world().resource::<PressureGrid>();
        assert!(grid.get(9, 10) < 1.0, "Interior pressure should drop");
        assert!(grid.get(11, 10) > 0.0, "Exterior pressure should rise");
    }

    #[test]
    fn test_airlock_minimizes_venting() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let _airlock_entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Airlock,
                },
                crate::layer1::control::DoorControl {
                    state: crate::layer1::control::DoorState::Open,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        app.world_mut().insert_resource(PressureGrid::new(20, 20));
        app.world_mut()
            .resource_mut::<PressureGrid>()
            .set(9, 10, 1.0);
        app.world_mut()
            .resource_mut::<PressureGrid>()
            .set(11, 10, 0.0);

        app.add_systems(bevy::prelude::Update, process_door_venting_system);
        app.update();

        let grid = app.world().resource::<PressureGrid>();
        assert!(grid.get(9, 10) < 1.0, "Airlock should vent slightly");
        assert!(
            grid.get(9, 10) > 0.95,
            "Airlock should preserve most interior pressure"
        );
        assert!(
            grid.get(11, 10) < 0.05,
            "Airlock should leak minimal pressure"
        );
    }

    #[test]
    fn test_airlock_slows_movement() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let _airlock_entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Airlock,
                },
                crate::layer1::control::DoorControl {
                    state: crate::layer1::control::DoorState::Auto,
                }, // Even if auto, passing through slows down
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        let pop_entity = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, apply_door_movement_penalties_system);
        app.update();

        let speed = app
            .world()
            .get::<crate::layer1::pop::Speed>(pop_entity)
            .unwrap();
        assert!(speed.current < speed.base, "Airlock should slow movement");
    }

    #[test]
    fn test_pressure_grid_new_clamps_size() {
        let grid_large = PressureGrid::new(10000, 10000);
        assert_eq!(
            grid_large.values.len(),
            1,
            "Grid size should be clamped to 1x1 on overflow"
        );
        let grid_overflow = PressureGrid::new(usize::MAX, 2);
        assert_eq!(
            grid_overflow.values.len(),
            1,
            "Overflow size should be clamped to 1x1 on overflow"
        );
    }

    #[test]
    fn test_pressure_systems_without_grid_resource() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        // No PressureGrid inserted

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        app.add_systems(
            bevy::prelude::Update,
            (pressure_damage_system, update_pressure_system),
        );
        app.update(); // Should early-return without panic

        let Some(health) = app.world().get::<Health>(pop) else {
            panic!("missing Health");
        };
        assert_eq!(
            health.current, 100.0,
            "Health should not change without PressureGrid"
        );
    }

    #[test]
    fn test_pressure_grid_get_and_set_out_of_bounds() {
        let mut grid = PressureGrid::new(10, 10);

        // Negative coordinates
        grid.set(-1, -1, 0.5);
        assert_eq!(grid.get(-1, -1), 0.0, "Out of bounds should return 0.0");

        // Very large coordinates (preventing integer wrapping bugs)
        grid.set(i32::MAX, i32::MAX, 0.5);
        assert_eq!(grid.get(i32::MAX, i32::MAX), 0.0);

        // Just outside bounds
        grid.set(10, 10, 0.5);
        assert_eq!(grid.get(10, 10), 0.0);
    }

    #[test]
    fn test_lifesupport_self_repair_restores_hp() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let ls = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 20.0,
                    max_hp: 50.0,
                },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, lifesupport_self_repair_system);
        app.update();

        let structure = app.world().get::<Structure>(ls).unwrap();
        assert!(
            (structure.current_hp - (20.0 + LIFESUPPORT_SELF_REPAIR_PER_TICK)).abs() < 0.001,
            "LifeSupport should self-repair, got {}",
            structure.current_hp
        );
    }

    #[test]
    fn test_lifesupport_self_repair_caps_at_max() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let ls = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 49.9,
                    max_hp: 50.0,
                },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, lifesupport_self_repair_system);
        app.update();

        let structure = app.world().get::<Structure>(ls).unwrap();
        assert_eq!(
            structure.current_hp, 50.0,
            "Self-repair should cap at max HP"
        );
    }

    #[test]
    fn test_lifesupport_self_repair_ignores_destroyed() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let ls = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 0.0,
                    max_hp: 50.0,
                },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, lifesupport_self_repair_system);
        app.update();

        let structure = app.world().get::<Structure>(ls).unwrap();
        assert_eq!(
            structure.current_hp, 0.0,
            "Destroyed LifeSupport should not resurrect"
        );
    }

    #[test]
    fn test_lifesupport_self_repair_ignores_other_buildings() {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);

        let housing = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 5, y: 5 },
                Structure {
                    current_hp: 20.0,
                    max_hp: 50.0,
                },
            ))
            .id();

        app.add_systems(bevy::prelude::Update, lifesupport_self_repair_system);
        app.update();

        let structure = app.world().get::<Structure>(housing).unwrap();
        assert_eq!(
            structure.current_hp, 20.0,
            "Non-LifeSupport buildings should not self-repair"
        );
    }

    #[test]
    fn test_lifesupport_damage_warning_fires_at_half() {
        use crate::layer1::chronicle::AddChronicleEvent;

        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_event::<AddChronicleEvent>();

        app.world_mut().spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 5, y: 5 },
            Structure {
                current_hp: 20.0, // 40% < 50% threshold
                max_hp: 50.0,
            },
        ));

        app.add_systems(bevy::prelude::Update, lifesupport_damage_warning_system);
        app.update();

        let events = app.world().resource::<bevy::prelude::Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let texts: Vec<_> = reader.read(events).map(|e| e.text.clone()).collect();
        assert!(
            texts.iter().any(|t| t.contains("50%")),
            "Should warn at 50% integrity, got {:?}",
            texts
        );
    }

    #[test]
    fn test_lifesupport_damage_warning_no_spam() {
        use crate::layer1::chronicle::AddChronicleEvent;

        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_event::<AddChronicleEvent>();

        app.world_mut().spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 5, y: 5 },
            Structure {
                current_hp: 20.0,
                max_hp: 50.0,
            },
        ));

        app.add_systems(bevy::prelude::Update, lifesupport_damage_warning_system);
        // Run twice: second run should NOT re-warn (hysteresis).
        app.update();
        app.update();

        let events = app.world().resource::<bevy::prelude::Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let count = reader.read(events).count();
        assert_eq!(count, 1, "Warning should fire once, not every tick");
    }
}
