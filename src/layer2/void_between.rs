//! Spec 342: The Void Between — interstellar travel incidents.
//!
//! Ships and fleets moving between stars (Layer 2 `InTransit`, Layer 3
//! galaxy hops) can be delayed, lost in transit, or return as ghost
//! ships carrying anomalous cargo. Spec §5 expansion incidents are
//! included: crew mutation, data wipe, and anomaly detection.
//!
//! ## Grounding / spec adaptations
//! * The spec's `Ship`/`CargoHold { amount, resource: Credits }` sketch
//!   does not exist. The real travel unit is a Layer-2
//!   [`Fleet`][crate::layer2::fleet::Fleet] with an
//!   [`InTransit`][crate::layer2::fleet::InTransit] component, and the
//!   real hold is [`FleetCargo`][crate::layer2::mining::FleetCargo]
//!   (stacks + capacity). [`CargoHold`] is kept as the spec's simple
//!   single-stack hold for lone ships; the resolver pays anomalous
//!   cargo into *both* hold types. `ResourceType` has no `Credits`
//!   variant — anomalous cargo is `Scrap` / `MemoryCore`.
//! * RNG is the real [`GlobalRng`][crate::shared::random::GlobalRng]
//!   (seeded in tests), per spec §5 — not `thread_rng`.
//! * Player notification is the real chronicle
//!   ([`AddChronicleEvent`][crate::layer1::chronicle::AddChronicleEvent]),
//!   per spec §5 — every resolved incident writes an entry.
//! * `LostInTransit` here is an [`IncidentType`]; the unrelated stub in
//!   `layer3::ghost_ships` (a `cycles_lost` component) is left alone.
//! * All names are original and generic (see the IP-guard test).

use bevy::prelude::*;
use rand::Rng;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::resources::ResourceType;
use crate::layer2::fleet::{FleetHealth, InTransit};
use crate::layer2::mining::FleetCargo;
use crate::shared::random::GlobalRng;

/// Anomalous cargo (units) a ghost ship returns with.
pub const GHOST_CARGO_AMOUNT: f32 = 100.0;
/// Small anomalous find carried back when an anomaly is detected.
pub const ANOMALY_CARGO_AMOUNT: f32 = 10.0;
/// Ticks a lost ship drifts before the drift system may return it.
pub const LOST_RETURN_MIN_TICKS: u32 = 50;
/// Per-tick return chance for a lost ship past the minimum drift.
pub const LOST_RETURN_CHANCE: f32 = 0.02;
/// Incident cooldown (ticks) after a ship suffers an incident.
pub const INCIDENT_COOLDOWN_TICKS: u32 = 100;

/// Kinds of travel incident (spec §4 + §5 expansion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentType {
    /// Ship vanishes from transit; it may later return as a ghost ship.
    LostInTransit,
    /// Ship is held up in the void; arrival is pushed back.
    Delayed(u32),
    /// A lost ship returns, crewed by no one, carrying anomalous cargo.
    GhostShipReturn,
    /// The void changes the crew (spec §5).
    CrewMutated,
    /// Nav / manifest data is wiped; cargo stacks are lost (spec §5).
    DataWiped,
    /// Sensors record something unexplained; a small find is recovered.
    AnomalyDetected,
}

impl IncidentType {
    /// Short label for logs, STATS and headless output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LostInTransit => "lost",
            Self::Delayed(_) => "delayed",
            Self::GhostShipReturn => "ghost",
            Self::CrewMutated => "mutated",
            Self::DataWiped => "wiped",
            Self::AnomalyDetected => "anomaly",
        }
    }

    /// Parse a headless/debug label.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "lost" => Some(Self::LostInTransit),
            "delayed" | "delay" => Some(Self::Delayed(50)),
            "ghost" => Some(Self::GhostShipReturn),
            "mutated" | "crew" | "crewmutated" => Some(Self::CrewMutated),
            "wiped" | "data" | "datawiped" => Some(Self::DataWiped),
            "anomaly" => Some(Self::AnomalyDetected),
            _ => None,
        }
    }
}

/// Event: a ship suffers (or resolves) a travel incident.
#[derive(Event, Debug, Clone, Copy)]
pub struct ShipIncidentEvent {
    /// The travelling ship/fleet entity.
    pub ship_entity: Entity,
    /// What happened in the void.
    pub incident_type: IncidentType,
}

/// Per-ship travel tracking (spec §4, adapted to real transit).
#[derive(Component, Debug, Clone, Copy)]
pub struct ShipTravelTracker {
    /// Nominal distance left on the current leg.
    pub distance_remaining: f32,
    /// Per-roll incident probability (route danger, spec §7).
    pub incident_risk_factor: f32,
    /// Whether the ship is currently between stars.
    pub in_transit: bool,
    /// Ticks spent on the current leg.
    pub ticks_in_transit: u32,
    /// Delay ticks still to serve before arrival can proceed.
    pub delay_remaining: u32,
    /// Incidents suffered over the ship's lifetime.
    pub incidents_suffered: u32,
    /// Ticks until the next incident roll is allowed.
    pub cooldown_ticks: u32,
}

impl Default for ShipTravelTracker {
    fn default() -> Self {
        Self {
            distance_remaining: 100.0,
            incident_risk_factor: 0.05,
            in_transit: false,
            ticks_in_transit: 0,
            delay_remaining: 0,
            incidents_suffered: 0,
            cooldown_ticks: 0,
        }
    }
}

impl ShipTravelTracker {
    /// Tracker for a ship starting a leg of the given duration (ticks).
    /// Longer legs through the void are riskier (spec §7).
    #[must_use]
    pub fn for_leg(duration_ticks: f32) -> Self {
        Self {
            distance_remaining: duration_ticks.max(1.0),
            incident_risk_factor: risk_for_duration(duration_ticks),
            in_transit: true,
            ..Self::default()
        }
    }
}

/// Route danger from leg duration: a short hop is safe, a long
/// crossing accumulates risk. Clamped to `[0.0, 1.0]`.
#[must_use]
pub fn risk_for_duration(duration_ticks: f32) -> f32 {
    (0.01 + duration_ticks / 10_000.0).clamp(0.0, 0.5)
}

/// The spec's simple single-stack hold, for lone ships without a
/// full [`FleetCargo`]. The resolver pays anomalous cargo here too.
#[derive(Component, Debug, Clone, Copy)]
pub struct CargoHold {
    /// Resource carried.
    pub resource: ResourceType,
    /// Amount carried.
    pub amount: f32,
}

impl Default for CargoHold {
    fn default() -> Self {
        Self {
            resource: ResourceType::Scrap,
            amount: 0.0,
        }
    }
}

/// Marker/state: the ship is lost in the void.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct LostInVoid {
    /// Ticks spent lost.
    pub ticks_lost: u32,
}

/// Marker: the ship returned as a ghost ship.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct GhostShipMark;
/// Marker: the crew was changed by the void.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct CrewMutatedMark;
/// Marker: nav/manifest data was wiped.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct DataWipedMark;
/// Marker: the ship detected an anomaly.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct AnomalyDetectedMark;

/// Colony-wide ledger of void incidents (STATS / headless evidence).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct VoidLedger {
    /// All resolved incidents.
    pub total: u32,
    /// Ships lost in transit.
    pub lost: u32,
    /// Delays imposed.
    pub delayed: u32,
    /// Ghost-ship returns.
    pub ghost_returns: u32,
    /// Crew mutations.
    pub crew_mutated: u32,
    /// Data wipes.
    pub data_wiped: u32,
    /// Anomalies detected.
    pub anomalies: u32,
}

impl VoidLedger {
    fn record(&mut self, kind: IncidentType) {
        self.total += 1;
        match kind {
            IncidentType::LostInTransit => self.lost += 1,
            IncidentType::Delayed(_) => self.delayed += 1,
            IncidentType::GhostShipReturn => self.ghost_returns += 1,
            IncidentType::CrewMutated => self.crew_mutated += 1,
            IncidentType::DataWiped => self.data_wiped += 1,
            IncidentType::AnomalyDetected => self.anomalies += 1,
        }
    }
}

/// Pick an incident kind from a `[0,6)` roll (weighted toward delay).
#[must_use]
pub fn incident_for_roll(roll: u32) -> IncidentType {
    match roll % 6 {
        0 => IncidentType::LostInTransit,
        1 | 2 => IncidentType::Delayed(50),
        3 => IncidentType::CrewMutated,
        4 => IncidentType::DataWiped,
        _ => IncidentType::AnomalyDetected,
    }
}

/// Roll for incidents on every active leg (spec §3/§5: real
/// `GlobalRng`, so seeded tests are deterministic). A risk factor of
/// 1.0 always incidents on the first eligible tick; otherwise the
/// per-tick chance is `risk * 0.01` (route danger accumulates over a
/// leg instead of detonating every tick). Delayed ships serve their
/// delay instead of rolling, and an incident imposes a cooldown.
pub fn process_travel_incidents(
    mut query: Query<(Entity, &mut ShipTravelTracker, Option<&InTransit>)>,
    mut events: EventWriter<ShipIncidentEvent>,
    mut rng: ResMut<GlobalRng>,
) {
    for (entity, mut tracker, transit) in &mut query {
        if !tracker.in_transit && transit.is_none() {
            continue;
        }
        tracker.ticks_in_transit += 1;
        if tracker.delay_remaining > 0 {
            tracker.delay_remaining -= 1;
            continue;
        }
        if tracker.cooldown_ticks > 0 {
            tracker.cooldown_ticks -= 1;
            continue;
        }
        let risk = tracker.incident_risk_factor;
        if risk <= 0.0 {
            continue;
        }
        let chance = if risk >= 1.0 { 1.0 } else { risk * 0.01 };
        if rng.0.gen::<f32>() < chance {
            let kind = incident_for_roll(rng.0.gen_range(0..6));
            tracker.cooldown_ticks = INCIDENT_COOLDOWN_TICKS;
            events.send(ShipIncidentEvent {
                ship_entity: entity,
                incident_type: kind,
            });
        }
    }
}

/// Resolve incident events (spec §4 + §5): apply the mechanical
/// effect, mark the ship, update the [`VoidLedger`], and notify the
/// player through the real chronicle.
#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
pub fn resolve_ship_incident(
    mut events: EventReader<ShipIncidentEvent>,
    mut commands: Commands,
    mut trackers: Query<&mut ShipTravelTracker>,
    mut holds: Query<&mut CargoHold>,
    mut fleet_cargo: Query<&mut FleetCargo>,
    mut health: Query<&mut FleetHealth>,
    mut transit: Query<&mut InTransit>,
    mut ledger: ResMut<VoidLedger>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let entity = event.ship_entity;
        let kind = event.incident_type;
        ledger.record(kind);
        if let Ok(mut tracker) = trackers.get_mut(entity) {
            tracker.incidents_suffered += 1;
            tracker.cooldown_ticks = INCIDENT_COOLDOWN_TICKS;
        }
        match kind {
            IncidentType::LostInTransit => {
                if let Ok(mut tracker) = trackers.get_mut(entity) {
                    tracker.in_transit = false;
                }
                commands
                    .entity(entity)
                    .insert(LostInVoid::default())
                    .remove::<InTransit>();
                chronicle.send(AddChronicleEvent {
                    text: "A ship has been lost in the void. Its beacon fades; the crossing keeps its secrets.".to_string(),
                    importance: EventImportance::Major,
                });
            }
            IncidentType::Delayed(extra) => {
                if let Ok(mut tracker) = trackers.get_mut(entity) {
                    tracker.delay_remaining += extra;
                }
                if let Ok(mut leg) = transit.get_mut(entity) {
                    leg.duration += extra as f32;
                }
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "A ship is delayed in the void — arrival pushed back by {extra} ticks."
                    ),
                    importance: EventImportance::Standard,
                });
            }
            IncidentType::GhostShipReturn => {
                if let Ok(mut tracker) = trackers.get_mut(entity) {
                    tracker.in_transit = false;
                }
                commands
                    .entity(entity)
                    .insert(GhostShipMark)
                    .remove::<LostInVoid>();
                if let Ok(mut hold) = holds.get_mut(entity) {
                    hold.amount += GHOST_CARGO_AMOUNT;
                }
                if let Ok(mut cargo) = fleet_cargo.get_mut(entity) {
                    cargo.add(ResourceType::Scrap, GHOST_CARGO_AMOUNT);
                }
                chronicle.send(AddChronicleEvent {
                    text: "A ghost ship has returned from the void, crewed by no one, its hold full of anomalous cargo.".to_string(),
                    importance: EventImportance::Major,
                });
            }
            IncidentType::CrewMutated => {
                commands.entity(entity).insert(CrewMutatedMark);
                if let Ok(mut hp) = health.get_mut(entity) {
                    hp.current = (hp.current - 10.0).max(1.0);
                }
                chronicle.send(AddChronicleEvent {
                    text: "A returning crew has been changed by the void. They work, they smile, they do not blink.".to_string(),
                    importance: EventImportance::Major,
                });
            }
            IncidentType::DataWiped => {
                commands.entity(entity).insert(DataWipedMark);
                if let Ok(mut cargo) = fleet_cargo.get_mut(entity) {
                    cargo.contents.clear();
                }
                if let Ok(mut hold) = holds.get_mut(entity) {
                    hold.amount = 0.0;
                }
                chronicle.send(AddChronicleEvent {
                    text: "A ship limps home with its nav core and cargo manifest wiped clean by the void.".to_string(),
                    importance: EventImportance::Standard,
                });
            }
            IncidentType::AnomalyDetected => {
                commands.entity(entity).insert(AnomalyDetectedMark);
                if let Ok(mut hold) = holds.get_mut(entity) {
                    hold.amount += ANOMALY_CARGO_AMOUNT;
                }
                if let Ok(mut cargo) = fleet_cargo.get_mut(entity) {
                    cargo.add(ResourceType::MemoryCore, ANOMALY_CARGO_AMOUNT);
                }
                chronicle.send(AddChronicleEvent {
                    text: "A ship's sensors record an anomaly in the void; a small, impossible find is brought home.".to_string(),
                    importance: EventImportance::Standard,
                });
            }
        }
    }
}

/// Lost ships drift; past a minimum drift they may return as ghost
/// ships (spec §4/§7: the robust lost-ship state is `LostInVoid` —
/// out of transit and out of the arrival UI until it returns).
pub fn lost_ship_drift_system(
    mut query: Query<(Entity, &mut LostInVoid)>,
    mut events: EventWriter<ShipIncidentEvent>,
    mut rng: ResMut<GlobalRng>,
) {
    for (entity, mut lost) in &mut query {
        lost.ticks_lost += 1;
        if lost.ticks_lost >= LOST_RETURN_MIN_TICKS && rng.0.gen::<f32>() < LOST_RETURN_CHANCE {
            events.send(ShipIncidentEvent {
                ship_entity: entity,
                incident_type: IncidentType::GhostShipReturn,
            });
        }
    }
}

/// Attach a tracker to fleets that enter transit without one.
pub fn ensure_travel_tracker_system(
    mut commands: Commands,
    query: Query<(Entity, &InTransit), Without<ShipTravelTracker>>,
) {
    for (entity, transit) in &query {
        commands
            .entity(entity)
            .insert(ShipTravelTracker::for_leg(transit.duration));
    }
}

/// STATS field: `total/lost/ghost` from the ledger, or `0/0/0`.
#[must_use]
pub fn void_stats_label(world: &World) -> String {
    world.get_resource::<VoidLedger>().map_or_else(
        || "0/0/0".to_string(),
        |l| format!("{}/{}/{}", l.total, l.lost, l.ghost_returns),
    )
}

/// Total resolved incidents (headless helper).
#[must_use]
pub fn void_incident_total(world: &World) -> u32 {
    world.get_resource::<VoidLedger>().map_or(0, |l| l.total)
}

/// Ships currently lost in the void (headless helper).
pub fn lost_ship_count(world: &mut World) -> usize {
    let mut q = world.query::<&LostInVoid>();
    q.iter(world).count()
}

/// Ships currently tracked in transit (headless helper).
pub fn tracked_transit_count(world: &mut World) -> usize {
    let mut q = world.query_filtered::<&ShipTravelTracker, With<InTransit>>();
    q.iter(world).filter(|t| t.in_transit).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn app_with_systems() -> App {
        let mut app = App::new();
        app.add_event::<ShipIncidentEvent>();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(VoidLedger::default());
        app.insert_resource(GlobalRng(rand::rngs::StdRng::seed_from_u64(42)));
        app.add_systems(
            Update,
            (
                process_travel_incidents,
                resolve_ship_incident,
                lost_ship_drift_system,
            )
                .chain(),
        );
        app
    }

    #[test]
    fn test_high_risk_ship_suffers_incident() {
        let mut app = app_with_systems();
        let ship = app
            .world_mut()
            .spawn(ShipTravelTracker {
                incident_risk_factor: 1.0,
                in_transit: true,
                ..ShipTravelTracker::default()
            })
            .id();
        app.update();
        let tracker = app.world().get::<ShipTravelTracker>(ship).unwrap();
        assert!(tracker.incidents_suffered >= 1, "risk 1.0 must incident");
        assert!(app.world().resource::<VoidLedger>().total >= 1);
    }

    #[test]
    fn test_zero_risk_ship_never_incidents() {
        let mut app = app_with_systems();
        let ship = app
            .world_mut()
            .spawn(ShipTravelTracker {
                incident_risk_factor: 0.0,
                in_transit: true,
                ..ShipTravelTracker::default()
            })
            .id();
        for _ in 0..10 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<ShipTravelTracker>(ship)
                .unwrap()
                .incidents_suffered,
            0
        );
        assert_eq!(app.world().resource::<VoidLedger>().total, 0);
    }

    #[test]
    fn test_ghost_ship_returns_with_cargo() {
        // Spec RED test 2, adapted: CargoHold amount is f32, resource
        // is Scrap (no Credits variant exists).
        let mut app = app_with_systems();
        let ship = app
            .world_mut()
            .spawn((
                ShipTravelTracker::default(),
                CargoHold {
                    amount: 0.0,
                    resource: ResourceType::Scrap,
                },
                LostInVoid::default(),
            ))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::GhostShipReturn,
        });
        app.update();
        let cargo = app.world().get::<CargoHold>(ship).unwrap();
        assert!(cargo.amount > 0.0, "ghost cargo must appear");
        assert!(app.world().get::<LostInVoid>(ship).is_none());
        assert!(app.world().get::<GhostShipMark>(ship).is_some());
    }

    #[test]
    fn test_ghost_ship_fills_fleet_cargo() {
        let mut app = app_with_systems();
        let ship = app
            .world_mut()
            .spawn((
                ShipTravelTracker::default(),
                FleetCargo {
                    contents: Vec::new(),
                    capacity: 1000.0,
                },
            ))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::GhostShipReturn,
        });
        app.update();
        let cargo = app.world().get::<FleetCargo>(ship).unwrap();
        assert!(cargo.current_load() > 0.0);
    }

    #[test]
    fn test_lost_in_transit_marks_ship_lost() {
        let mut app = app_with_systems();
        let dest = app.world_mut().spawn_empty().id();
        let origin = app.world_mut().spawn_empty().id();
        let ship = app
            .world_mut()
            .spawn((
                ShipTravelTracker {
                    in_transit: true,
                    ..ShipTravelTracker::default()
                },
                InTransit {
                    origin,
                    destination: dest,
                    progress: 0.5,
                    duration: 100.0,
                },
            ))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::LostInTransit,
        });
        app.update();
        assert!(app.world().get::<LostInVoid>(ship).is_some());
        assert!(
            app.world().get::<InTransit>(ship).is_none(),
            "lost ship leaves transit"
        );
        assert!(
            !app.world()
                .get::<ShipTravelTracker>(ship)
                .unwrap()
                .in_transit
        );
    }

    #[test]
    fn test_delayed_pushes_back_arrival() {
        let mut app = app_with_systems();
        let dest = app.world_mut().spawn_empty().id();
        let origin = app.world_mut().spawn_empty().id();
        let ship = app
            .world_mut()
            .spawn((
                ShipTravelTracker {
                    in_transit: true,
                    ..ShipTravelTracker::default()
                },
                InTransit {
                    origin,
                    destination: dest,
                    progress: 0.5,
                    duration: 100.0,
                },
            ))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::Delayed(40),
        });
        app.update();
        let tracker = app.world().get::<ShipTravelTracker>(ship).unwrap();
        assert_eq!(tracker.delay_remaining, 40);
        let transit = app.world().get::<InTransit>(ship).unwrap();
        assert!(transit.duration > 100.0, "delay extends the leg");
    }

    #[test]
    fn test_data_wiped_clears_fleet_cargo() {
        let mut app = app_with_systems();
        let mut hold = FleetCargo {
            contents: Vec::new(),
            capacity: 1000.0,
        };
        hold.add(ResourceType::Metal, 50.0);
        let ship = app
            .world_mut()
            .spawn((ShipTravelTracker::default(), hold))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::DataWiped,
        });
        app.update();
        assert_eq!(
            app.world().get::<FleetCargo>(ship).unwrap().current_load(),
            0.0
        );
        assert!(app.world().get::<DataWipedMark>(ship).is_some());
    }

    #[test]
    fn test_crew_mutated_and_anomaly_marks() {
        let mut app = app_with_systems();
        let ship = app
            .world_mut()
            .spawn((
                ShipTravelTracker::default(),
                CargoHold::default(),
                FleetHealth {
                    current: 100.0,
                    max: 100.0,
                },
            ))
            .id();
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::CrewMutated,
        });
        app.update();
        assert!(app.world().get::<CrewMutatedMark>(ship).is_some());
        app.world_mut().send_event(ShipIncidentEvent {
            ship_entity: ship,
            incident_type: IncidentType::AnomalyDetected,
        });
        app.update();
        assert!(app.world().get::<AnomalyDetectedMark>(ship).is_some());
        assert!(app.world().get::<CargoHold>(ship).unwrap().amount > 0.0);
        assert_eq!(app.world().resource::<VoidLedger>().total, 2);
    }

    #[test]
    fn test_risk_scales_with_duration() {
        assert!(risk_for_duration(10_000.0) > risk_for_duration(100.0));
        assert_eq!(risk_for_duration(0.0), 0.01);
        assert!(risk_for_duration(1_000_000.0) <= 1.0);
    }

    #[test]
    fn test_incident_roll_covers_all_kinds() {
        let kinds: Vec<&str> = (0..6).map(|r| incident_for_roll(r).label()).collect();
        for expected in ["lost", "delayed", "mutated", "wiped", "anomaly"] {
            assert!(kinds.contains(&expected), "missing {expected}");
        }
        // Ghost returns come from the drift system, not the roll table.
        assert!(!kinds.contains(&"ghost"));
    }

    #[test]
    fn test_ip_guard_no_named_ip() {
        let public = [
            IncidentType::LostInTransit.label(),
            IncidentType::GhostShipReturn.label(),
            "The Void Between",
            "A ship has been lost in the void.",
            "A ghost ship has returned from the void.",
        ]
        .join(" ")
        .to_ascii_lowercase();
        for banned in [
            "event horizon",
            "warhammer",
            "star trek",
            "star wars",
            "battlestar",
            "babylon 5",
            "firefly",
            "serenity",
            "nostromo",
            "enterprise",
            "millennium falcon",
        ] {
            assert!(!public.contains(banned), "banned term: {banned}");
        }
    }
}
