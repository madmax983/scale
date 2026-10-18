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

/// Marker: this pop has already been alerted for its current vacuum
/// excursion (Item 77 Phase 2 telegraphing).
///
/// Inserted by [`pressure_damage_system`] on the first tick a pop takes
/// suffocation damage, removed when the pop is back at or above the
/// suffocation threshold — so each new excursion alerts exactly once and
/// a pop standing in vacuum does not spam the chronicle every tick.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SuffocationAlerted;

/// Pressure below which a pop takes suffocation damage (and alerts fire).
pub const SUFFOCATION_PRESSURE_THRESHOLD: f32 = 0.2;

/// System to apply suffocation damage.
///
/// Item 77 Phase 2: the first signal of a suffocation death used to be the
/// death notice itself. Now the first tick a pop is below
/// [`SUFFOCATION_PRESSURE_THRESHOLD`] and losing HP chronicles
/// "{name} is suffocating — no air at their position", throttled per pop
/// per excursion via [`SuffocationAlerted`]. The damage itself is
/// unchanged (1 HP/tick — Classic stays brutal; this is awareness).
#[allow(clippy::type_complexity)]
pub fn pressure_damage_system(
    mut commands: Commands,
    mut pop_query: Query<
        (
            Entity,
            &crate::layer1::map::GridPosition,
            &mut crate::layer1::health::Health,
            Option<&crate::layer1::pop::PopName>,
            Has<SuffocationAlerted>,
            Option<&crate::layer1::items::Equipment>,
            Option<&crate::layer1::eva_suit::SuitAirAlerted>,
        ),
        (
            With<crate::layer1::pop::Pop>,
            // The phased Chronostalker is out of the time stream:
            // vacuum cannot touch what is not, momentarily, there.
            Without<crate::layer1::culture::chronostalker::Phased>,
            // Item 100: dormant origin pops wait in stasis at their
            // outdoor sites — the automaton does not breathe, the chrome
            // stranger is between moments, the changed scout is half
            // weather. Suffocation cannot touch them until they wake
            // (the dormant marker is removed on taking the path, and
            // from then on they suffocate like anyone else).
            Without<crate::layer1::culture::lawbound::DormantAutomaton>,
            Without<crate::layer1::culture::bloomtouched::DormantTouched>,
            Without<crate::layer1::culture::chronostalker::DormantStalker>,
        ),
    >,
    mut suit_query: Query<&mut crate::layer1::eva_suit::EvaSuit>,
    grid_opt: Option<Res<PressureGrid>>,
    mut events: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    use crate::layer1::chronicle::EventImportance;

    let Some(grid) = grid_opt else { return };
    let tick = time.map_or(0, |t| t.tick);
    let mut killed = 0;

    for (entity, pos, mut health, name, alerted, equipment, suit_alerted) in pop_query.iter_mut() {
        if health.current <= 0.0 {
            continue;
        }

        let pressure = grid.get(pos.x, pos.y);
        if pressure < SUFFOCATION_PRESSURE_THRESHOLD {
            // Spec 1384: a worn suit with air covers this tick — the
            // bottle drains instead of the pop's HP, and the suit-air
            // chronicle warnings fire here (once per excursion) so
            // "covered by bottle" and "takes damage" can never drift.
            let mut covered = false;
            if let Some(body) = equipment.and_then(|eq| eq.body) {
                if let Ok(mut suit) = suit_query.get_mut(body) {
                    if suit.air > 0.0 {
                        covered = true;
                        suit.air = (suit.air
                            - crate::layer1::eva_suit::EVA_AIR_DRAIN_PER_TICK)
                            .max(0.0);
                        let label =
                            name.map_or_else(|| "A colonist".to_string(), |n| n.0.clone());
                        let mut state = suit_alerted.copied().unwrap_or_default();
                        let mut changed = false;
                        if suit.air <= 0.0 && !state.dry_warned {
                            events.send(crate::layer1::chronicle::AddChronicleEvent {
                                text: format!("{label}'s suit has run dry."),
                                importance: EventImportance::Major,
                            });
                            state.dry_warned = true;
                            state.low_warned = true;
                            changed = true;
                        } else if suit.air_fraction()
                            < crate::layer1::eva_suit::EVA_AIR_LOW_FRACTION
                            && !state.low_warned
                        {
                            events.send(crate::layer1::chronicle::AddChronicleEvent {
                                text: format!(
                                    "{label}'s suit air is low — {} days of air left.",
                                    suit.air.floor() as i64
                                ),
                                importance: EventImportance::Major,
                            });
                            state.low_warned = true;
                            changed = true;
                        }
                        if changed {
                            commands
                                .entity(entity)
                                .try_insert(crate::layer1::eva_suit::SuitAirAlerted {
                                    low_warned: state.low_warned,
                                    dry_warned: state.dry_warned,
                                });
                        }
                    }
                }
            }
            if covered {
                continue;
            }
            if !alerted {
                let label = name.map_or_else(|| "A colonist".to_string(), |n| n.0.clone());
                events.send(crate::layer1::chronicle::AddChronicleEvent {
                    text: format!("{label} is suffocating — no air at their position."),
                    importance: EventImportance::Major,
                });
                // try_insert: the pop may be despawned by another system
                // in the same tick batch before commands apply.
                commands.entity(entity).try_insert(SuffocationAlerted);
            }
            health.take_damage(1.0);
            // Item 84: record the source on every suffocation tick, so a
            // death finished by another system in the same tick window
            // still attributes to the breach; the explicit DeathCause
            // below remains authoritative when this system lands the kill.
            crate::layer1::biology::death_attribution::record_damage(
                &mut commands,
                entity,
                crate::layer1::biology::death_attribution::CAUSE_ATMOSPHERIC_BREACH,
                tick,
            );
            if health.current <= 0.0 {
                commands
                    .entity(entity)
                    .try_insert(crate::layer1::health::DeathCause(
                        "Atmospheric breach".to_string(),
                    ));
                killed += 1;
            }
        } else if alerted {
            // Back in air: re-arm the alert for the next excursion.
            commands.entity(entity).remove::<SuffocationAlerted>();
        }
    }

    if killed > 0 {
        events.send(crate::layer1::chronicle::AddChronicleEvent {
            text: format!("{} Pops suffocated due to atmospheric breach.", killed),
            importance: EventImportance::Major,
        });
    }
}

/// Colony-level atmosphere snapshot, refreshed every tick by
/// [`update_colony_atmosphere_system`] (Item 77 Phase 2).
///
/// This is the data the shell status bar and the headless STATS line read:
/// how many pops are standing in vacuum right now, the worst pop-tile
/// pressure, and whether any LifeSupport unit is sitting unpowered. Before
/// Phase 2 the only surface for any of this was the building inspector.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ColonyAtmosphere {
    /// Pops currently on tiles below the suffocation threshold.
    pub pops_in_vacuum: usize,
    /// Lowest pressure on any living pop's tile (1.0 when there are no pops).
    pub min_pop_pressure: f32,
    /// LifeSupport units whose power consumer is inactive.
    pub unpowered_life_support: usize,
    /// Total LifeSupport units in the colony.
    pub life_support_total: usize,
}

impl ColonyAtmosphere {
    /// True when at least one LifeSupport exists and none of them is powered.
    #[must_use]
    pub fn life_support_unpowered(&self) -> bool {
        self.life_support_total > 0 && self.unpowered_life_support == self.life_support_total
    }

    /// True when at least one LifeSupport unit is unpowered (Item 99).
    /// The all-or-nothing check above left partial outages — one of two
    /// units dark, the survivor carrying the colony — invisible in the
    /// shell status bar even though STATS counted them.
    #[must_use]
    pub fn any_life_support_unpowered(&self) -> bool {
        self.unpowered_life_support > 0
    }
}

/// Refresh the [`ColonyAtmosphere`] snapshot from the live world.
pub fn update_colony_atmosphere_system(
    mut snapshot: ResMut<ColonyAtmosphere>,
    grid_opt: Option<Res<PressureGrid>>,
    pops: Query<&crate::layer1::map::GridPosition, With<crate::layer1::pop::Pop>>,
    life_support: Query<(
        &crate::layer1::building::Building,
        Option<&crate::layer1::energy::PowerConsumer>,
    )>,
) {
    use crate::layer1::building::BuildingType;

    let mut min_pressure = 1.0f32;
    let mut in_vacuum = 0usize;
    if let Some(grid) = grid_opt.as_deref() {
        for pos in pops.iter() {
            let pressure = grid.get(pos.x, pos.y);
            min_pressure = min_pressure.min(pressure);
            if pressure < SUFFOCATION_PRESSURE_THRESHOLD {
                in_vacuum += 1;
            }
        }
    }
    let mut total = 0usize;
    let mut unpowered = 0usize;
    for (building, consumer) in life_support.iter() {
        if building.building_type != BuildingType::LifeSupport {
            continue;
        }
        total += 1;
        // A LifeSupport with no PowerConsumer at all runs unconditionally
        // (update_pressure_system treats missing consumer as active).
        if consumer.is_some_and(|c| !c.active) {
            unpowered += 1;
        }
    }
    *snapshot = ColonyAtmosphere {
        pops_in_vacuum: in_vacuum,
        min_pop_pressure: if pops.is_empty() { 1.0 } else { min_pressure },
        unpowered_life_support: unpowered,
        life_support_total: total,
    };
}

/// Per-entity alert state for the unpowered-LifeSupport warning.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct LifeSupportPowerAlertState {
    /// Tick the current continuous unpowered stretch began.
    unpowered_since: Option<u64>,
    /// Whether the warning has fired for the current stretch.
    warned_unpowered: bool,
    /// Whether the sustained-outage escalation has fired (Item 99).
    escalated_unpowered: bool,
}

/// How long a LifeSupport must stay continuously unpowered before the
/// colony warning fires.
///
/// The power grid's brownout mechanic flickers consumers on undersized
/// grids (the starter grid runs at half supply by design), so a warning
/// on the first dark tick would spam the chronicle every few ticks and
/// cry wolf. A unit on a grid with no production at all — the forensics
/// bug this alert exists for — stays dark forever and crosses this
/// threshold; a brownout flicker never comes close.
pub const UNPOWERED_WARNING_DELAY_TICKS: u64 = 100;

/// How long a LifeSupport must stay continuously unpowered before the
/// warning escalates (Item 99).
///
/// The first warning cannot tell a recovering brownout from an outage
/// that is still in force hours later — before Item 99 a unit dark for
/// 101 ticks and a unit dark for 10,000 produced exactly one identical
/// chronicle line, so the balance data's endemic sustained outages
/// (final `ls_unpowered >= 1` in roughly half of colonies) read as
/// already-answered news. At this duration the outage is the colony's
/// condition, not an incident, and the chronicle says so — once.
pub const UNPOWERED_ESCALATION_TICKS: u64 = 600;

/// Colony-level alert: a LifeSupport unit with no power (Item 77 Phase 2).
///
/// Before Phase 2 this fact lived only in the inspector of the exact
/// building ("Consumer Demand: 10 (Inactive)"), so a colony could coast on
/// its one-time seeded air charge with a dead generator and never be told.
/// The warning fires once per sustained power loss (see
/// [`UNPOWERED_WARNING_DELAY_TICKS`]), escalates once if the outage is
/// still in force at [`UNPOWERED_ESCALATION_TICKS`] (Item 99), and both
/// re-arm on repowering.
pub fn lifesupport_power_warning_system(
    mut commands: Commands,
    time: Res<crate::shared::time::SimulationTime>,
    query: Query<(
        Entity,
        &crate::layer1::building::Building,
        Option<&crate::layer1::energy::PowerConsumer>,
        Option<&LifeSupportPowerAlertState>,
    )>,
    mut chronicle: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    use crate::layer1::building::BuildingType;
    use crate::layer1::chronicle::EventImportance;

    for (entity, building, consumer, alert) in query.iter() {
        if building.building_type != BuildingType::LifeSupport {
            continue;
        }
        let powered = consumer.is_none_or(|c| c.active);
        let state = alert.copied().unwrap_or_default();
        if powered {
            if state.unpowered_since.is_some() || state.warned_unpowered {
                commands.entity(entity).try_insert(LifeSupportPowerAlertState {
                    unpowered_since: None,
                    warned_unpowered: false,
                    escalated_unpowered: false,
                });
            }
            continue;
        }
        let since = state.unpowered_since.unwrap_or(time.tick);
        let dark_for = time.tick.saturating_sub(since);
        if state.unpowered_since.is_none() {
            commands.entity(entity).try_insert(LifeSupportPowerAlertState {
                unpowered_since: Some(time.tick),
                warned_unpowered: false,
                escalated_unpowered: false,
            });
        } else if !state.warned_unpowered && dark_for >= UNPOWERED_WARNING_DELAY_TICKS {
            chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
                text: "LifeSupport has no power — the air is only what the hull already holds. Bridge it to a power source with a Power Pole or build a Generator.".to_string(),
                importance: EventImportance::Major,
            });
            // try_insert: a LifeSupport can be destroyed (ruined) by
            // another system in the same tick batch; warning about a
            // building that no longer exists must not panic the schedule.
            commands.entity(entity).try_insert(LifeSupportPowerAlertState {
                unpowered_since: Some(since),
                warned_unpowered: true,
                escalated_unpowered: false,
            });
        } else if state.warned_unpowered
            && !state.escalated_unpowered
            && dark_for >= UNPOWERED_ESCALATION_TICKS
        {
            // Item 99: a sustained outage escalates once. The wording
            // deliberately avoids the first warning's "no power" phrasing
            // so chronicle readers can tell the two apart at a glance.
            chronicle.send(crate::layer1::chronicle::AddChronicleEvent {
                text: format!(
                    "LifeSupport is STILL without power after {dark_for} ticks — no new air is being made, and the colony is living on what the hull trapped. This is no longer a flicker. Restore power now: bridge it with a Power Pole or build a Generator."
                ),
                importance: EventImportance::Major,
            });
            commands.entity(entity).try_insert(LifeSupportPowerAlertState {
                unpowered_since: Some(since),
                warned_unpowered: true,
                escalated_unpowered: true,
            });
        }
    }
}

/// The shelter / recall order (Item 77 Phase 2 counterplay).
///
/// While active, every non-panicked, non-possessed pop standing in vacuum
/// is ordered to the nearest pressurized tile. This is the missing verb
/// against exposure deaths and panic-cascade aftermaths: the player can
/// call everyone home instead of watching them linger outside. It does
/// NOT soften panic itself — panicked pops keep fleeing (their movement
/// is panic's, not the order's), and vacuum damage is unchanged.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ShelterOrder {
    /// Whether the order is currently in force.
    pub active: bool,
    /// Tick the order was last issued (0 = never / lifted).
    pub issued_tick: u64,
}

/// Pressure a tile must hold for the shelter order to count it as safe.
pub const SHELTER_SAFE_PRESSURE: f32 = 0.5;

/// Find the nearest tile at or above [`SHELTER_SAFE_PRESSURE`] to `from`,
/// scanning expanding Chebyshev rings. Deterministic: nearest distance,
/// then lowest (x, y).
#[must_use]
pub fn nearest_pressurized_tile(
    grid: &PressureGrid,
    from: crate::layer1::map::GridPosition,
) -> Option<crate::layer1::map::GridPosition> {
    use crate::layer1::map::GridPosition;
    if grid.get(from.x, from.y) >= SHELTER_SAFE_PRESSURE {
        return Some(from);
    }
    // The grid is at most a few hundred tiles across; a bounded ring scan
    // is more than fast enough for a per-tick order check.
    for radius in 1..=64i32 {
        let mut best: Option<GridPosition> = None;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate = GridPosition {
                    x: from.x + dx,
                    y: from.y + dy,
                };
                if grid.get(candidate.x, candidate.y) >= SHELTER_SAFE_PRESSURE
                    && best.is_none_or(|b| (candidate.x, candidate.y) < (b.x, b.y))
                {
                    best = Some(candidate);
                }
            }
        }
        if best.is_some() {
            return best;
        }
    }
    None
}

/// Drive the shelter order: pops in vacuum get a movement target on the
/// nearest pressurized tile (self-targeted, the same pattern visitor
/// departure and panic flight use to move without a building target).
#[allow(clippy::type_complexity)]
pub fn shelter_order_system(
    order: Res<ShelterOrder>,
    grid_opt: Option<Res<PressureGrid>>,
    mut commands: Commands,
    pops: Query<
        (
            Entity,
            &crate::layer1::map::GridPosition,
            Option<&crate::layer1::execution::components::MovementTarget>,
        ),
        (
            With<crate::layer1::pop::Pop>,
            Without<crate::layer1::psychology::panic_spirals::Panic>,
            Without<crate::layer1::culture::chronostalker::Phased>,
            // The possessed pop is the player's own hands; the order
            // never overrides direct control.
            Without<crate::layer1::direct_link::Possessed>,
        ),
    >,
) {
    if !order.active {
        return;
    }
    let Some(grid) = grid_opt.as_deref() else {
        return;
    };
    for (entity, pos, movement) in pops.iter() {
        if grid.get(pos.x, pos.y) >= SUFFOCATION_PRESSURE_THRESHOLD {
            // Safe: release any shelter movement target we issued so the
            // utility AI owns the pop again.
            if movement.is_some_and(|mt| mt.target_entity == entity) {
                commands
                    .entity(entity)
                    .remove::<crate::layer1::execution::components::MovementTarget>()
                    .remove::<crate::layer1::execution::components::AtTarget>();
            }
            continue;
        }
        if let Some(target) = nearest_pressurized_tile(grid, *pos) {
            if target == *pos {
                continue;
            }
            let already_heading = movement
                .is_some_and(|mt| mt.target_entity == entity && mt.target_position == target);
            if !already_heading {
                commands
                    .entity(entity)
                    .remove::<crate::layer1::execution::components::AtTarget>()
                    .try_insert(crate::layer1::execution::components::MovementTarget {
                        target_entity: entity,
                        target_position: target,
                        for_action: crate::layer1::utility_types::ActionType::Idle,
                    });
            }
        }
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
    fn test_dormant_origin_pops_do_not_suffocate() {
        // Item 100: dormant origin pops (Vigil the automaton, Quill the
        // changed scout, Tock the chrome stranger) wait in stasis at
        // outdoor sites beyond the habitat's pressure. Suffocation must
        // not touch them before a player can wake them — in the
        // 2026-10-18 balance matrix every one of the 30 fatal vacuum
        // episodes was a dormant origin pop dying ~98 ticks after spawn.
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        app.insert_resource(PressureGrid::new(10, 10));

        let mk_health = || Health {
            current: 100.0,
            max: 100.0,
            has_rust_lung: false,
        };
        let vigil = app
            .world_mut()
            .spawn((
                Pop,
                mk_health(),
                GridPosition { x: 5, y: 5 },
                crate::layer1::culture::lawbound::DormantAutomaton,
            ))
            .id();
        let quill = app
            .world_mut()
            .spawn((
                Pop,
                mk_health(),
                GridPosition { x: 6, y: 5 },
                crate::layer1::culture::bloomtouched::DormantTouched,
            ))
            .id();
        let tock = app
            .world_mut()
            .spawn((
                Pop,
                mk_health(),
                GridPosition { x: 7, y: 5 },
                crate::layer1::culture::chronostalker::DormantStalker,
            ))
            .id();
        let control = app
            .world_mut()
            .spawn((Pop, mk_health(), GridPosition { x: 8, y: 5 }))
            .id();

        app.add_systems(bevy::prelude::Update, pressure_damage_system);
        app.update();

        for dormant in [vigil, quill, tock] {
            let health = app.world().get::<Health>(dormant).unwrap();
            assert_eq!(
                health.current, 100.0,
                "dormant origin pops are in stasis — vacuum cannot touch them"
            );
        }
        let health = app.world().get::<Health>(control).unwrap();
        assert!(
            health.current < 100.0,
            "an ordinary pop on the same vacuum tiles still suffocates"
        );
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

    // -----------------------------------------------------------------
    // Item 77 Phase 2: suffocation alert, atmosphere snapshot, power
    // warning, shelter order.
    // -----------------------------------------------------------------

    fn chronicle_texts(app: &bevy::prelude::App) -> Vec<String> {
        use crate::layer1::chronicle::AddChronicleEvent;
        let events = app
            .world()
            .resource::<bevy::prelude::Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        reader.read(events).map(|e| e.text.clone()).collect()
    }

    fn suffocation_app() -> bevy::prelude::App {
        use crate::layer1::chronicle::AddChronicleEvent;
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(PressureGrid::new(10, 10));
        app
    }

    fn spawn_named_pop(app: &mut bevy::prelude::App, x: i32, y: i32) -> Entity {
        app.world_mut()
            .spawn((
                Pop,
                crate::layer1::pop::PopName("Ada".to_string()),
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x, y },
            ))
            .id()
    }

    #[test]
    fn test_suffocation_alert_fires_once_per_excursion() {
        let mut app = suffocation_app();
        let pop = spawn_named_pop(&mut app, 5, 5);
        app.add_systems(bevy::prelude::Update, pressure_damage_system);

        // First tick in vacuum: the alert fires immediately (event buffers
        // only retain two frames, so read before running further ticks).
        app.update();
        let texts = chronicle_texts(&app);
        let alerts: Vec<_> = texts.iter().filter(|t| t.contains("suffocating")).collect();
        assert_eq!(alerts.len(), 1, "one alert per excursion, got {texts:?}");
        assert!(alerts[0].contains("Ada"), "alert names the pop: {alerts:?}");
        // Two more ticks in vacuum: no re-alert, damage every tick.
        app.update();
        app.update();
        assert!(
            app.world().get::<SuffocationAlerted>(pop).is_some(),
            "alert marker should persist for the whole excursion"
        );
        let health = app.world().get::<Health>(pop).unwrap();
        assert_eq!(health.current, 97.0, "damage unchanged at 1 HP/tick");

        // Back in air: marker clears (re-arm). Then a new excursion alerts again.
        app.world_mut().resource_mut::<PressureGrid>().set(5, 5, 1.0);
        app.update();
        assert!(
            app.world().get::<SuffocationAlerted>(pop).is_none(),
            "alert marker should clear once the pop breathes again"
        );
        app.world_mut().resource_mut::<PressureGrid>().set(5, 5, 0.0);
        app.update();
        let texts = chronicle_texts(&app);
        // A fresh event cursor reads the whole retained buffer, so counts
        // are cumulative: first excursion + second excursion = 2.
        let alerts = texts.iter().filter(|t| t.contains("suffocating")).count();
        assert_eq!(alerts, 2, "second excursion should alert again");
    }

    #[test]
    fn test_colony_atmosphere_snapshot_counts_vacuum_and_unpowered() {
        let mut app = suffocation_app();
        app.init_resource::<ColonyAtmosphere>();
        spawn_named_pop(&mut app, 5, 5); // vacuum tile
        let safe = spawn_named_pop(&mut app, 6, 6);
        app.world_mut().resource_mut::<PressureGrid>().set(6, 6, 1.0);
        let _ = safe;
        app.world_mut().spawn((
            Building {
                building_type: BuildingType::LifeSupport,
            },
            GridPosition { x: 1, y: 1 },
            crate::layer1::energy::PowerConsumer {
                demand: 10.0,
                active: false,
            },
        ));
        app.add_systems(bevy::prelude::Update, update_colony_atmosphere_system);
        app.update();
        let snapshot = *app.world().resource::<ColonyAtmosphere>();
        assert_eq!(snapshot.pops_in_vacuum, 1);
        assert_eq!(snapshot.min_pop_pressure, 0.0);
        assert_eq!(snapshot.life_support_total, 1);
        assert_eq!(snapshot.unpowered_life_support, 1);
        assert!(snapshot.life_support_unpowered());
    }

    #[test]
    fn test_lifesupport_power_warning_fires_once_and_rearms() {
        use crate::layer1::chronicle::AddChronicleEvent;
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(crate::shared::time::SimulationTime::default());
        let ls = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 5, y: 5 },
                crate::layer1::energy::PowerConsumer {
                    demand: 10.0,
                    active: false,
                },
            ))
            .id();
        app.add_systems(bevy::prelude::Update, lifesupport_power_warning_system);
        let set_tick = |app: &mut bevy::prelude::App, tick: u64| {
            app.world_mut()
                .resource_mut::<crate::shared::time::SimulationTime>()
                .tick = tick;
        };
        // Brownout-length flickers must NOT warn: the delay is what keeps
        // the chronicle honest on undersized grids.
        app.update();
        set_tick(&mut app, UNPOWERED_WARNING_DELAY_TICKS - 1);
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("no power")).count(),
            0,
            "no warning before the sustained-loss delay, got {texts:?}"
        );
        // Sustained loss: fires exactly once.
        set_tick(&mut app, UNPOWERED_WARNING_DELAY_TICKS);
        app.update();
        app.update();
        let texts = chronicle_texts(&app);
        assert_eq!(
            texts.iter().filter(|t| t.contains("no power")).count(),
            1,
            "unpowered warning should fire once after the delay, got {texts:?}"
        );
        // Repower (re-arms), then a new sustained loss refires.
        app.world_mut()
            .get_mut::<crate::layer1::energy::PowerConsumer>(ls)
            .unwrap()
            .active = true;
        app.update();
        app.world_mut()
            .get_mut::<crate::layer1::energy::PowerConsumer>(ls)
            .unwrap()
            .active = false;
        set_tick(&mut app, UNPOWERED_WARNING_DELAY_TICKS + 10);
        app.update();
        set_tick(&mut app, UNPOWERED_WARNING_DELAY_TICKS * 2 + 10);
        app.update();
        let texts = chronicle_texts(&app);
        // Cumulative count (fresh cursor reads the retained buffer).
        assert_eq!(
            texts.iter().filter(|t| t.contains("no power")).count(),
            2,
            "warning should refire after a repower/re-loss cycle"
        );
    }

    #[test]
    fn test_lifesupport_power_warning_escalates_once_on_sustained_outage() {
        // Item 99: the first warning (100 ticks) cannot distinguish a
        // brownout that recovered from an outage that is still killing
        // the colony hours later. A sustained outage must escalate
        // exactly once, and re-arm on repowering like the first warning.
        use crate::layer1::chronicle::AddChronicleEvent;
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::prelude::MinimalPlugins);
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(crate::shared::time::SimulationTime::default());
        let ls = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                GridPosition { x: 5, y: 5 },
                crate::layer1::energy::PowerConsumer {
                    demand: 10.0,
                    active: false,
                },
            ))
            .id();
        app.add_systems(bevy::prelude::Update, lifesupport_power_warning_system);
        let set_tick = |app: &mut bevy::prelude::App, tick: u64| {
            app.world_mut()
                .resource_mut::<crate::shared::time::SimulationTime>()
                .tick = tick;
        };
        let escalations = |app: &bevy::prelude::App| {
            chronicle_texts(app)
                .iter()
                .filter(|t| t.contains("STILL"))
                .count()
        };
        app.update();
        set_tick(&mut app, UNPOWERED_WARNING_DELAY_TICKS);
        app.update();
        assert_eq!(escalations(&app), 0, "no escalation with the first warning");
        set_tick(&mut app, UNPOWERED_ESCALATION_TICKS - 1);
        app.update();
        assert_eq!(escalations(&app), 0, "no escalation before the sustained threshold");
        set_tick(&mut app, UNPOWERED_ESCALATION_TICKS);
        app.update();
        app.update();
        assert_eq!(escalations(&app), 1, "sustained outage escalates exactly once");
        set_tick(&mut app, UNPOWERED_ESCALATION_TICKS * 4);
        app.update();
        assert_eq!(escalations(&app), 1, "escalation does not spam on longer outages");
        // Repower re-arms: a fresh sustained outage escalates again.
        app.world_mut()
            .get_mut::<crate::layer1::energy::PowerConsumer>(ls)
            .unwrap()
            .active = true;
        app.update();
        app.world_mut()
            .get_mut::<crate::layer1::energy::PowerConsumer>(ls)
            .unwrap()
            .active = false;
        set_tick(&mut app, UNPOWERED_ESCALATION_TICKS * 4 + 10);
        app.update();
        set_tick(&mut app, UNPOWERED_ESCALATION_TICKS * 5 + 10);
        app.update();
        // The re-loss first re-fires the plain warning (previous update);
        // one more tick at the same sustained duration escalates again.
        app.update();
        assert_eq!(escalations(&app), 2, "escalation re-arms after repower/re-loss");
    }

    #[test]
    fn test_shelter_order_targets_nearest_pressurized_tile() {
        let mut app = suffocation_app();
        app.init_resource::<ShelterOrder>();
        app.world_mut().resource_mut::<ShelterOrder>().active = true;
        // One safe tile at (8, 5); the pop stands in vacuum at (5, 5).
        app.world_mut().resource_mut::<PressureGrid>().set(8, 5, 1.0);
        let pop = spawn_named_pop(&mut app, 5, 5);
        app.add_systems(bevy::prelude::Update, shelter_order_system);
        app.update();
        let target = app
            .world()
            .get::<crate::layer1::execution::components::MovementTarget>(pop)
            .expect("shelter order should give the pop a movement target");
        assert_eq!(target.target_position, GridPosition { x: 8, y: 5 });
        assert_eq!(target.target_entity, pop, "shelter targets are self-targeted");

        // Inactive order: no target issued.
        let mut app2 = suffocation_app();
        app2.init_resource::<ShelterOrder>();
        app2.world_mut().resource_mut::<PressureGrid>().set(8, 5, 1.0);
        let pop2 = spawn_named_pop(&mut app2, 5, 5);
        app2.add_systems(bevy::prelude::Update, shelter_order_system);
        app2.update();
        assert!(
            app2.world()
                .get::<crate::layer1::execution::components::MovementTarget>(pop2)
                .is_none(),
            "no shelter target while the order is lifted"
        );
    }

    #[test]
    fn test_nearest_pressurized_tile_prefers_closest() {
        let mut grid = PressureGrid::new(20, 20);
        grid.set(12, 10, 1.0);
        grid.set(10, 14, 1.0);
        let found = nearest_pressurized_tile(&grid, GridPosition { x: 10, y: 10 });
        assert_eq!(found, Some(GridPosition { x: 12, y: 10 }));
        // Already safe: returns own tile.
        let found = nearest_pressurized_tile(&grid, GridPosition { x: 12, y: 10 });
        assert_eq!(found, Some(GridPosition { x: 12, y: 10 }));
        // No safe tile anywhere: None.
        let empty = PressureGrid::new(5, 5);
        assert_eq!(
            nearest_pressurized_tile(&empty, GridPosition { x: 2, y: 2 }),
            None
        );
    }
}
