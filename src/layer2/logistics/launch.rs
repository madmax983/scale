//! Spec 379: Launch Windows — transfer windows from orbital positions.
//!
//! "The planets align. Space travel isn't just about distance, it's
//! about timing."
//!
//! A [`LaunchCostCalculator`] prices a transfer between two celestial
//! bodies from their real [`Orbit`] components (`layer2::system`, spec
//! 094): when the bodies are aligned (smallest angular separation at
//! most [`WINDOW_HALF_ANGLE`], pi/4 in the spec MVP) the transfer costs
//! the baseline [`BASELINE_FUEL_COST`]; out of window it costs 10x
//! ([`OUT_OF_WINDOW_FUEL_COST`]).
//!
//! Adaptations from the spec sketch:
//! * The spec's `layer2::celestial::Orbit { angle, radius }` does not
//!   exist; the real `Orbit` also carries `parent` and `speed`, and
//!   lives in `layer2::system`. The calculator reads the real type.
//! * The spec's raw absolute angle difference is replaced by the
//!   smallest signed separation wrapped into `[0, pi]` — real orbit
//!   angles wrap at a full turn, so 350 degrees vs 10 degrees is a
//!   20-degree separation, not 340.
//! * Fuel is the real colony pool, `ColonyResources.fuel` (the
//!   spec-117 stockpile that generators also burn). A launched fleet
//!   pays from that pool; per-ship `DeltaV` (`layer2::ship::logistics`)
//!   still governs in-flight burn once a ship is moving.
//! * A successful [`LaunchCommand`] reuses the spec-099 fleet seam:
//!   it swaps `InOrbit` for `InTransit` (fixed 100-tick MVP duration,
//!   exactly like `fleet_order_system`) and stamps a [`TransferOrbit`]
//!   on the fleet. The legacy `FleetOrder::MoveTo` path is deliberately
//!   left ungated — gating it would change every existing caller; the
//!   fuel gate lives on this launch path, per the spec's Technical
//!   Guidance ("The `LaunchCommand` should check this calculator").
//! * `HohmannTransfer` is an alias for [`TransferOrbit`]: the transfer
//!   is tracked for the duration of the `InTransit` leg and cleared on
//!   arrival. Full two-burn Hohmann phasing (departure/arrival burns,
//!   phasing orbits) is left alone — the fleet seam has no burn model
//!   to hang it on yet.
//! * Orbits actually drift: [`advance_orbits_system`] implements the
//!   behaviour `layer2::system::Orbit` always documented ("Every tick,
//!   `angle` is incremented by `speed`") but no system performed.

use crate::layer1::economy::resources::ColonyResources;
use crate::layer2::fleet::{Fleet, InOrbit, InTransit};
use crate::layer2::system::Orbit;
use bevy_ecs::prelude::*;
use std::f32::consts::{PI, TAU};

/// Fuel cost of a transfer launched inside the window.
pub const BASELINE_FUEL_COST: u32 = 100;
/// Fuel cost of a transfer launched out of window (10x baseline).
pub const OUT_OF_WINDOW_FUEL_COST: u32 = BASELINE_FUEL_COST * 10;
/// Half-angle of the launch window, radians (spec MVP: pi/4).
pub const WINDOW_HALF_ANGLE: f32 = PI / 4.0;
/// Fixed transfer duration in ticks, matching the spec-099 MVP
/// `InTransit` duration in `fleet_order_system`.
pub const TRANSFER_DURATION_TICKS: u64 = 100;
/// Predictor scan horizon: beyond this many ticks a window is reported
/// as "not foreseeable" rather than scanned forever.
pub const MAX_PREDICTION_TICKS: u64 = 200_000;

/// The priced cost of one transfer: cheap in window, 10x out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchCost {
    /// Bodies aligned: baseline fuel.
    InWindow(u32),
    /// Bodies misaligned: 10x fuel.
    OutOfWindow(u32),
}

// Spec-style variant imports: `use launch::{InWindow, OutOfWindow}`.
pub use LaunchCost::{InWindow, OutOfWindow};

impl LaunchCost {
    /// The fuel this transfer costs, whichever variant it is.
    #[must_use]
    pub const fn fuel(self) -> u32 {
        match self {
            Self::InWindow(fuel) | Self::OutOfWindow(fuel) => fuel,
        }
    }

    /// Whether this transfer is priced at the in-window baseline.
    #[must_use]
    pub const fn is_in_window(self) -> bool {
        matches!(self, Self::InWindow(_))
    }
}

/// Smallest angular separation between two bodies, wrapped into
/// `[0, pi]`: the raw difference is reduced modulo a full turn first,
/// so bodies just either side of angle 0 read as close together.
#[must_use]
pub fn angular_separation(a: &Orbit, b: &Orbit) -> f32 {
    let raw = (b.angle - a.angle).rem_euclid(TAU);
    raw.min(TAU - raw)
}

/// The angular separation the pair will have `ticks_ahead` ticks from
/// now, once each body has drifted by its own orbital `speed`.
#[must_use]
pub fn separation_at(a: &Orbit, b: &Orbit, ticks_ahead: u64) -> f32 {
    let t = ticks_ahead as f32;
    let future_a = Orbit {
        parent: a.parent,
        radius: a.radius,
        speed: a.speed,
        angle: a.angle + a.speed * t,
    };
    let future_b = Orbit {
        parent: b.parent,
        radius: b.radius,
        speed: b.speed,
        angle: b.angle + b.speed * t,
    };
    angular_separation(&future_a, &future_b)
}

/// Whether the pair will be inside the launch window `ticks_ahead`
/// ticks from now.
#[must_use]
pub fn in_window_at(a: &Orbit, b: &Orbit, ticks_ahead: u64) -> bool {
    separation_at(a, b, ticks_ahead) <= WINDOW_HALF_ANGLE
}

/// Prices transfers between orbiting bodies. Stateless; all inputs
/// come from the real `Orbit` components.
pub struct LaunchCostCalculator;

impl LaunchCostCalculator {
    /// Spec entry point: price the transfer `origin` -> `destination`
    /// from the bodies' current `Orbit`s. `None` when either entity
    /// has no `Orbit`.
    pub fn calculate(
        origin: Entity,
        destination: Entity,
        orbits: &Query<&Orbit>,
    ) -> Option<LaunchCost> {
        let origin_orbit = orbits.get(origin).ok()?;
        let dest_orbit = orbits.get(destination).ok()?;
        Some(Self::calculate_from_orbits(origin_orbit, dest_orbit))
    }

    /// Pure pricing from two orbits: in window at or below
    /// [`WINDOW_HALF_ANGLE`] costs the baseline, anything wider
    /// costs 10x.
    #[must_use]
    pub fn calculate_from_orbits(origin: &Orbit, destination: &Orbit) -> LaunchCost {
        if angular_separation(origin, destination) <= WINDOW_HALF_ANGLE {
            LaunchCost::InWindow(BASELINE_FUEL_COST)
        } else {
            LaunchCost::OutOfWindow(OUT_OF_WINDOW_FUEL_COST)
        }
    }
}

/// Launch Window Predictor: ticks from now until the pair's next
/// low-fuel window opens. `Some(0)` when the pair is in window now;
/// `None` when no window opens within [`MAX_PREDICTION_TICKS`] (for
/// example a phase-locked pair whose separation never changes).
#[must_use]
pub fn next_window_start(origin: &Orbit, destination: &Orbit) -> Option<u64> {
    if in_window_at(origin, destination, 0) {
        return Some(0);
    }
    (1..=MAX_PREDICTION_TICKS).find(|&t| {
        in_window_at(origin, destination, t) && !in_window_at(origin, destination, t - 1)
    })
}

/// The start ticks of the next `count` low-fuel windows for the pair
/// (entry edges: in window at the reported tick, out of window the
/// tick before). Fewer than `count` when the horizon runs out.
#[must_use]
pub fn upcoming_windows(origin: &Orbit, destination: &Orbit, count: usize) -> Vec<u64> {
    let mut starts = Vec::new();
    if count == 0 {
        return starts;
    }
    if in_window_at(origin, destination, 0) {
        starts.push(0);
    }
    let mut t = 1;
    while starts.len() < count && t <= MAX_PREDICTION_TICKS {
        if in_window_at(origin, destination, t) && !in_window_at(origin, destination, t - 1) {
            starts.push(t);
        }
        t += 1;
    }
    starts
}

/// Tracking component stamped on a fleet for the length of its
/// transfer: the priced plan it launched under. Removed on arrival by
/// [`transfer_completion_system`]; the underlying movement is the
/// spec-099 `InTransit` leg running alongside it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferOrbit {
    /// Body the fleet departed.
    pub origin: Entity,
    /// Body the fleet is bound for.
    pub destination: Entity,
    /// Fuel paid at launch, from the colony pool.
    pub fuel_cost: u32,
    /// Whether the launch was priced in window.
    pub in_window: bool,
    /// Planned leg length in ticks.
    pub duration_ticks: u64,
}

impl TransferOrbit {
    /// Plan a transfer between two bodies from their current orbits.
    #[must_use]
    pub fn plan(origin: Entity, destination: Entity, a: &Orbit, b: &Orbit) -> Self {
        let cost = LaunchCostCalculator::calculate_from_orbits(a, b);
        Self {
            origin,
            destination,
            fuel_cost: cost.fuel(),
            in_window: cost.is_in_window(),
            duration_ticks: TRANSFER_DURATION_TICKS,
        }
    }
}

/// Spec REFACTOR name for the transfer tracking component. This is an
/// alias, not a second system: the transfer rides the existing
/// `InTransit` leg rather than simulating two-burn phasing.
pub type HohmannTransfer = TransferOrbit;

/// Order a fleet to launch for another body. The launch is priced by
/// [`LaunchCostCalculator`] and refused when the colony fuel pool
/// cannot pay (see [`launch_command_system`]).
#[derive(Event, Debug, Clone, Copy)]
pub struct LaunchCommand {
    /// The fleet to launch (must be `InOrbit` somewhere).
    pub fleet: Entity,
    /// The body to transfer to.
    pub destination: Entity,
}

/// A launch went ahead: fuel was paid and the fleet is `InTransit`.
#[derive(Event, Debug, Clone, Copy)]
pub struct LaunchedEvent {
    /// The fleet that launched.
    pub fleet: Entity,
    /// Body it departed.
    pub origin: Entity,
    /// Body it is bound for.
    pub destination: Entity,
    /// The priced (and paid) cost.
    pub cost: LaunchCost,
}

/// A launch was refused: the fleet stays in orbit and no fuel burns.
#[derive(Event, Debug, Clone, Copy)]
pub struct LaunchRefusedEvent {
    /// The fleet that was ordered to launch.
    pub fleet: Entity,
    /// The body it was ordered to.
    pub destination: Entity,
    /// Fuel the transfer would have cost (`None` when it could not be
    /// priced — a body without an `Orbit`, or a fleet not in orbit).
    pub required: Option<u32>,
    /// Fuel in the colony pool when the refusal landed.
    pub available: f32,
}

/// Running ledger of launch activity, for the headless panel and the
/// STATS `window=` field.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct LaunchLedger {
    /// Launches that went ahead.
    pub launches: u32,
    /// Launches refused (fuel gate or unpriceable).
    pub refused: u32,
    /// Total fuel burned by launches.
    pub fuel_spent: f32,
}

/// Advances every `Orbit` by its `speed`, wrapping at a full turn —
/// the drift `layer2::system::Orbit` documents but nothing performed,
/// so windows genuinely open and close over time.
pub fn advance_orbits_system(mut orbits: Query<&mut Orbit>) {
    for mut orbit in &mut orbits {
        orbit.angle = (orbit.angle + orbit.speed).rem_euclid(TAU);
    }
}

/// Processes [`LaunchCommand`]s: prices the transfer from the fleet's
/// current body to the destination, refuses when the colony fuel pool
/// (`ColonyResources.fuel`) cannot pay, otherwise deducts the fuel and
/// sends the fleet down the spec-099 seam (`InOrbit` -> `InTransit`
/// plus a [`TransferOrbit`] plan).
#[allow(clippy::too_many_arguments)]
pub fn launch_command_system(
    mut commands: Commands,
    mut requests: EventReader<LaunchCommand>,
    fleets: Query<&InOrbit, With<Fleet>>,
    orbits: Query<&Orbit>,
    mut resources: ResMut<ColonyResources>,
    mut ledger: ResMut<LaunchLedger>,
    mut launched: EventWriter<LaunchedEvent>,
    mut refused: EventWriter<LaunchRefusedEvent>,
) {
    for request in requests.read() {
        let available = resources.fuel;
        let priced = fleets.get(request.fleet).ok().and_then(|in_orbit| {
            LaunchCostCalculator::calculate(in_orbit.parent, request.destination, &orbits)
                .map(|cost| (in_orbit.parent, cost))
        });
        let Some((origin, cost)) = priced else {
            ledger.refused += 1;
            refused.send(LaunchRefusedEvent {
                fleet: request.fleet,
                destination: request.destination,
                required: None,
                available,
            });
            continue;
        };
        if available < cost.fuel() as f32 {
            ledger.refused += 1;
            refused.send(LaunchRefusedEvent {
                fleet: request.fleet,
                destination: request.destination,
                required: Some(cost.fuel()),
                available,
            });
            continue;
        }
        resources.fuel -= cost.fuel() as f32;
        ledger.launches += 1;
        ledger.fuel_spent += cost.fuel() as f32;
        let origin_orbit = orbits.get(origin).ok();
        let dest_orbit = orbits.get(request.destination).ok();
        let plan = match (origin_orbit, dest_orbit) {
            (Some(a), Some(b)) => TransferOrbit::plan(origin, request.destination, a, b),
            _ => TransferOrbit {
                origin,
                destination: request.destination,
                fuel_cost: cost.fuel(),
                in_window: cost.is_in_window(),
                duration_ticks: TRANSFER_DURATION_TICKS,
            },
        };
        commands
            .entity(request.fleet)
            .remove::<InOrbit>()
            .insert(InTransit {
                origin,
                destination: request.destination,
                progress: 0.0,
                duration: TRANSFER_DURATION_TICKS as f32,
            })
            .insert(plan);
        launched.send(LaunchedEvent {
            fleet: request.fleet,
            origin,
            destination: request.destination,
            cost,
        });
    }
}

/// Clears the [`TransferOrbit`] plan once its fleet has arrived (the
/// spec-099 movement system swaps `InTransit` back to `InOrbit`).
/// Runs after `fleet_movement_system` in the simulation schedule.
#[allow(clippy::type_complexity)]
pub fn transfer_completion_system(
    mut commands: Commands,
    arrived: Query<(Entity, &InOrbit), (With<TransferOrbit>, Without<InTransit>)>,
) {
    for (entity, _) in &arrived {
        commands.entity(entity).remove::<TransferOrbit>();
    }
}

/// Every orbiting body in the world, sorted by entity id so panels
/// and labels are deterministic: `(entity, name, orbit)`. The name is
/// the body's `OrbitalBody` name when it has one, else `body <id>`.
#[must_use]
pub fn orbiting_bodies(world: &mut World) -> Vec<(Entity, String, Orbit)> {
    let mut bodies: Vec<(Entity, String, Orbit)> = Vec::new();
    let mut query = world.query::<(Entity, &Orbit, Option<&crate::layer2::system::OrbitalBody>)>();
    for (entity, orbit, body) in query.iter(world) {
        let name = body.map_or_else(|| format!("body {}", entity.index()), |b| b.name.clone());
        bodies.push((entity, name, orbit.clone()));
    }
    bodies.sort_by_key(|(entity, _, _)| entity.index());
    bodies
}

/// One-line STATS label for the `window=` field: the launch ledger
/// once anything has launched or been refused (`2L,1R,1100f`), else
/// the current price of the first body pair (`in:100` / `out:1000`),
/// else `off` when the world has fewer than two orbiting bodies.
#[must_use]
pub fn launch_stats_label(world: &mut World) -> String {
    if let Some(ledger) = world.get_resource::<LaunchLedger>() {
        if ledger.launches > 0 || ledger.refused > 0 {
            return format!(
                "{}L,{}R,{:.0}f",
                ledger.launches, ledger.refused, ledger.fuel_spent
            );
        }
    }
    let bodies = orbiting_bodies(world);
    if bodies.len() < 2 {
        return "off".to_string();
    }
    match LaunchCostCalculator::calculate_from_orbits(&bodies[0].2, &bodies[1].2) {
        LaunchCost::InWindow(cost) => format!("in:{cost}"),
        LaunchCost::OutOfWindow(cost) => format!("out:{cost}"),
    }
}
