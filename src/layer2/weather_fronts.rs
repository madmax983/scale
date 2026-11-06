//! Spec 395: Planetary Weather Fronts — Layer 2 fronts drive Layer 1 weather.
//!
//! Adaptations from the spec sketch (§8 grounding, following the
//! 360-394 pattern):
//! * The spec's `layer2::planet::{Planet, ColonyLocation}` and
//!   `layer1::weather::LocalWeather` do not exist. The real L1 weather
//!   state is [`WeatherState`](crate::layer1::weather::WeatherState)
//!   (resource, spec 079 lane in `layer1::nature::weather`), driven
//!   seasonally by `update_weather_system`; this module bridges into
//!   THAT resource — no parallel weather state, no second enum.
//! * The spec's `WeatherType::DustStorm` does not exist in the real
//!   [`WeatherType`](crate::layer1::weather::WeatherType) enum. Dust
//!   storms map to `WeatherType::Storm` (the existing heavy-weather
//!   variant, speed modifier 0.5) — see [`parse_front_weather`].
//! * The spec's `ColonyLocation(Vec2)` resource does not exist:
//!   `layer2::generation::ColonyLocation` is a marker component on the
//!   colony *planet* entity (orbital, no surface coordinates). The
//!   colony is a single L1 site, so fronts live in a colony-centred
//!   surface plane: [`ColonyFrontAnchor`] is that site's position in
//!   front-plane coordinates (default origin).
//! * The spec sketch moves fronts by `Transform` + `Time`; real L2
//!   weather ([`crate::layer2::weather::Storm`]) carries its own
//!   `position: Vec2` and moves per simulation tick
//!   (`position += velocity`). Fronts follow the L2 pattern.
//! * §5 REFACTOR — priority/merge: when several fronts cover the
//!   colony, the winner is chosen by [`weather_front_priority`]
//!   (severity first), then by coverage (closest to the front's core
//!   wins). Documented, deterministic, unit-tested.
//! * §5 REFACTOR — transition: a front only drives L1 weather once
//!   its coverage of the colony reaches [`FRONT_EDGE_COVERAGE`]; the
//!   outer band is a leading edge (recorded in the bridge as
//!   `approaching`) and seasonal weather keeps control there, so the
//!   change is a front arriving, not a snap at the outer radius.
//! * Restoration: on entry the bridge saves the seasonal weather it
//!   displaced; when the front leaves (or is despawned) it restores
//!   exactly that weather instead of forcing Clear in the live sim.
//!   In a bare world the saved default is Clear, which is the spec
//!   RED test's "returns to Clear" case.
//! * ZERO balance retunes: no existing weather constant is touched.
//!   New constants chosen here: [`FRONT_EDGE_COVERAGE`] and
//!   [`FRONT_WEATHER_DURATION`] (documented at their definitions).

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::weather::{WeatherState, WeatherType};
use bevy::math::Vec2;
use bevy_ecs::prelude::*;

/// Coverage (0.0 at the front's rim, 1.0 at its centre) at which a
/// front takes over the colony's weather. Below this the colony is
/// only in the front's leading edge. New constant for spec 395.
pub const FRONT_EDGE_COVERAGE: f32 = 0.25;

/// While a front drives the weather, `duration_remaining` is kept at
/// least this long so the seasonal `update_weather_system` does not
/// re-roll mid-passage and fight the bridge. New constant, spec 395.
pub const FRONT_WEATHER_DURATION: u32 = 50;

/// A moving Layer 2 weather front (spec 395). Mirrors the real L2
/// [`crate::layer2::weather::Storm`] shape: own position + velocity,
/// moved per tick.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct WeatherFront {
    /// The real L1 weather this front imposes when overhead.
    pub weather_type: WeatherType,
    /// Front-plane position (colony-centred surface coordinates).
    pub position: Vec2,
    /// Per-tick velocity in the front plane.
    pub velocity: Vec2,
    /// Radius of the front's footprint.
    pub radius: f32,
}

/// The colony site's position in the front plane. The spec sketch's
/// `ColonyLocation(Vec2)` adapted: the real `ColonyLocation` is an
/// orbital marker with no surface coordinates, and the L1 colony is
/// a single site, so it anchors the plane at the origin by default.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ColonyFrontAnchor {
    /// Colony position in front-plane coordinates.
    pub position: Vec2,
}

impl Default for ColonyFrontAnchor {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
        }
    }
}

/// Bridge state between Layer 2 fronts and the real Layer 1
/// [`WeatherState`]: what the bridge last applied, the seasonal
/// weather it displaced (restored on exit), and live coverage for
/// the headless panel / STATS.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct WeatherFrontBridge {
    /// Weather the bridge currently has applied, if any.
    pub applied: Option<WeatherType>,
    /// Seasonal weather saved on entry, restored on exit.
    pub saved_weather: Option<WeatherType>,
    /// `duration_remaining` saved alongside `saved_weather`.
    pub saved_duration: u32,
    /// Coverage of the winning front right now (0.0-1.0).
    pub coverage: f32,
    /// Fronts currently covering the colony at any coverage.
    pub overhead: u32,
    /// Total fronts in the world.
    pub total_fronts: u32,
    /// Fronts that have ever taken over the colony weather.
    pub passages: u32,
}

/// Severity priority for the §5 merge rule: higher wins when two
/// fronts overlap the colony. Ordered by how hard each real weather
/// type already hits the colony (its speed modifier is the existing
/// severity signal); Clear never wins.
#[must_use]
pub const fn weather_front_priority(weather: WeatherType) -> u8 {
    match weather {
        WeatherType::Storm => 100,
        WeatherType::MagneticStorm => 95,
        WeatherType::MutagenicRain => 90,
        WeatherType::SporeStorm => 85,
        WeatherType::Snow => 70,
        WeatherType::Heatwave => 60,
        WeatherType::Rain => 50,
        WeatherType::Fog => 40,
        WeatherType::BlissStorm => 30,
        WeatherType::ThermalInversion => 20,
        WeatherType::Clear => 0,
    }
}

/// Parse a headless front kind (case-insensitive, aliases ok). The
/// spec's `duststorm` maps to the real `WeatherType::Storm` — there
/// is no DustStorm variant in the real enum.
#[must_use]
pub fn parse_front_weather(text: &str) -> Option<WeatherType> {
    match text.to_ascii_lowercase().as_str() {
        "storm" | "duststorm" | "dust" | "dust-storm" => Some(WeatherType::Storm),
        "heatwave" | "heat" | "drought" => Some(WeatherType::Heatwave),
        "rain" => Some(WeatherType::Rain),
        "snow" | "blizzard" => Some(WeatherType::Snow),
        "fog" => Some(WeatherType::Fog),
        "magnetic" | "magneticstorm" => Some(WeatherType::MagneticStorm),
        "clear" => Some(WeatherType::Clear),
        _ => None,
    }
}

/// Coverage of a point by a front: 1.0 at the centre, 0.0 at or
/// beyond the rim.
#[must_use]
pub fn front_coverage(front_pos: Vec2, radius: f32, point: Vec2) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let dist = front_pos.distance(point);
    if dist >= radius {
        0.0
    } else {
        1.0 - dist / radius
    }
}

/// Moves every front by its per-tick velocity (the real L2 Storm
/// pattern — no `Time`, the simulation is tick-based).
pub fn update_weather_fronts_l2(mut fronts: Query<&mut WeatherFront>) {
    for mut front in &mut fronts {
        let velocity = front.velocity;
        front.position += velocity;
    }
}

/// The bridge: writes the winning overhead front into the real L1
/// [`WeatherState`], and restores the saved seasonal weather when
/// no front qualifies any more. No fronts in the world = no-op, so
/// the seasonal system owns the weather in ordinary runs.
#[allow(clippy::too_many_arguments)]
pub fn sync_weather_l2_to_l1(
    anchor: Res<ColonyFrontAnchor>,
    fronts: Query<&WeatherFront>,
    mut weather: ResMut<WeatherState>,
    mut bridge: ResMut<WeatherFrontBridge>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    bridge.total_fronts = fronts.iter().count() as u32;

    // Pick the winner: coverage > 0 counts as overhead; a front must
    // reach the transition coverage to drive the weather. Priority
    // (severity) first, then coverage.
    let mut overhead = 0u32;
    let mut winner: Option<(WeatherType, f32)> = None;
    for front in fronts.iter() {
        let coverage = front_coverage(front.position, front.radius, anchor.position);
        if coverage <= 0.0 {
            continue;
        }
        overhead += 1;
        if coverage < FRONT_EDGE_COVERAGE {
            continue;
        }
        let replace = match winner {
            None => true,
            Some((kind, cov)) => {
                let p = weather_front_priority(front.weather_type);
                let wp = weather_front_priority(kind);
                p > wp || (p == wp && coverage > cov)
            }
        };
        if replace {
            winner = Some((front.weather_type, coverage));
        }
    }
    bridge.overhead = overhead;

    match winner {
        Some((kind, coverage)) => {
            bridge.coverage = coverage;
            if bridge.applied != Some(kind) {
                if bridge.applied.is_none() {
                    // Entry: save the seasonal weather we displace.
                    bridge.saved_weather = Some(weather.current_weather);
                    bridge.saved_duration = weather.duration_remaining;
                    bridge.passages += 1;
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "A {} weather front has moved over the colony.",
                            kind.name()
                        ),
                        importance: EventImportance::Minor,
                    });
                } else {
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "A {} weather front has taken over from the passing front.",
                            kind.name()
                        ),
                        importance: EventImportance::Minor,
                    });
                }
                bridge.applied = Some(kind);
            }
            weather.current_weather = kind;
            weather.duration_remaining =
                weather.duration_remaining.max(FRONT_WEATHER_DURATION);
        }
        None => {
            bridge.coverage = 0.0;
            if let Some(applied) = bridge.applied.take() {
                let restored = bridge.saved_weather.take().unwrap_or(WeatherType::Clear);
                weather.current_weather = restored;
                weather.duration_remaining = bridge.saved_duration.max(1);
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "The {} weather front has passed; skies return to {}.",
                        applied.name(),
                        restored.name()
                    ),
                    importance: EventImportance::Minor,
                });
            }
        }
    }
}

/// Census for STATS / the headless panel:
/// `fronts={total}/{overhead}/{applied label}`.
#[must_use]
pub fn fronts_stats_label(world: &mut World) -> String {
    let total = world.query::<&WeatherFront>().iter(world).count();
    world.get_resource::<WeatherFrontBridge>().map_or_else(
        || format!("{total}/0/none"),
        |bridge| {
            let applied = bridge
                .applied
                .map_or("none", |kind| kind.name());
            format!("{total}/{}/{applied}", bridge.overhead)
        },
    )
}

/// Headless/debug seam: spawn a front in the front plane.
pub fn spawn_weather_front(
    world: &mut World,
    weather_type: WeatherType,
    position: Vec2,
    velocity: Vec2,
    radius: f32,
) -> Entity {
    world
        .spawn(WeatherFront {
            weather_type,
            position,
            velocity,
            radius,
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn front_world(weather: WeatherType) -> World {
        let mut world = World::new();
        world.insert_resource(WeatherState {
            current_weather: weather,
            duration_remaining: 100,
        });
        world.init_resource::<ColonyFrontAnchor>();
        world.init_resource::<WeatherFrontBridge>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn run_move(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(update_weather_fronts_l2);
        schedule.run(world);
    }

    fn run_sync(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(sync_weather_l2_to_l1);
        schedule.run(world);
    }

    fn run_both(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems((update_weather_fronts_l2, sync_weather_l2_to_l1).chain());
        schedule.run(world);
    }

    // Spec RED test 1, grounded: a front moves across the L2 plane.
    // (Spec sketch used Transform + Time; real L2 fronts carry their
    // own position and move per tick, like Storm.)
    #[test]
    fn test_weather_front_movement_layer2() {
        let mut world = World::new();
        let front = world
            .spawn(WeatherFront {
                weather_type: WeatherType::Storm,
                position: Vec2::new(0.0, 0.0),
                velocity: Vec2::new(1.0, 0.0),
                radius: 5.0,
            })
            .id();
        run_move(&mut world);
        let moved = world.get::<WeatherFront>(front).unwrap();
        assert!(
            moved.position.x > 0.0,
            "Weather front should move across the planet surface"
        );
    }

    // Spec RED test 2, grounded: a storm directly over the colony
    // applies its weather to the real L1 WeatherState. (The spec's
    // DustStorm is WeatherType::Storm in the real enum.)
    #[test]
    fn test_front_intersection_applies_layer1_weather() {
        let mut world = front_world(WeatherType::Clear);
        world.insert_resource(ColonyFrontAnchor {
            position: Vec2::new(10.0, 10.0),
        });
        world.spawn(WeatherFront {
            weather_type: WeatherType::Storm,
            position: Vec2::new(10.0, 10.0),
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Storm,
            "Colony should inherit the L2 weather front overhead"
        );
        assert_eq!(world.resource::<WeatherFrontBridge>().passages, 1);
    }

    // Spec RED test 3, grounded: the front leaves and the colony
    // returns to Clear — the bridge restores the seasonal weather it
    // saved on entry (Clear in this bare world).
    #[test]
    fn test_front_leaves_colony() {
        let mut world = front_world(WeatherType::Storm);
        world.insert_resource(ColonyFrontAnchor {
            position: Vec2::new(10.0, 10.0),
        });
        world.insert_resource(WeatherFrontBridge {
            applied: Some(WeatherType::Storm),
            saved_weather: Some(WeatherType::Clear),
            saved_duration: 100,
            ..Default::default()
        });
        world.spawn(WeatherFront {
            weather_type: WeatherType::Storm,
            position: Vec2::new(100.0, 100.0),
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Clear,
            "Colony should return to Clear weather when the front passes"
        );
    }

    #[test]
    fn test_far_front_leaves_seasonal_weather_alone() {
        // A distant front that never applied anything must not
        // stomp the seasonal system's weather.
        let mut world = front_world(WeatherType::Rain);
        world.spawn(WeatherFront {
            weather_type: WeatherType::Storm,
            position: Vec2::new(100.0, 100.0),
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Rain
        );
    }

    #[test]
    fn test_no_fronts_is_a_noop() {
        let mut world = front_world(WeatherType::Snow);
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Snow
        );
    }

    #[test]
    fn test_overlapping_fronts_priority_wins() {
        // §5 merge rule: Storm (priority 100) beats Heatwave (60)
        // over the colony, regardless of spawn order.
        let mut world = front_world(WeatherType::Clear);
        world.spawn(WeatherFront {
            weather_type: WeatherType::Heatwave,
            position: Vec2::ZERO,
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        world.spawn(WeatherFront {
            weather_type: WeatherType::Storm,
            position: Vec2::new(1.0, 0.0),
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Storm
        );
    }

    #[test]
    fn test_leading_edge_does_not_snap_weather() {
        // §5 transition: the colony sits in the front's outer band
        // (coverage 0.1 < FRONT_EDGE_COVERAGE) — overhead is recorded
        // but seasonal weather keeps control.
        let mut world = front_world(WeatherType::Clear);
        world.spawn(WeatherFront {
            weather_type: WeatherType::Storm,
            position: Vec2::new(4.5, 0.0),
            velocity: Vec2::ZERO,
            radius: 5.0,
        });
        run_sync(&mut world);
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Clear
        );
        assert_eq!(world.resource::<WeatherFrontBridge>().overhead, 1);
        assert_eq!(world.resource::<WeatherFrontBridge>().applied, None);
    }

    #[test]
    fn test_moving_front_applies_then_restores_seasonal() {
        // Full passage: front walks over the colony and away; the
        // Rain it displaced comes back afterwards.
        let mut world = front_world(WeatherType::Rain);
        world.spawn(WeatherFront {
            weather_type: WeatherType::Heatwave,
            position: Vec2::new(-3.0, 0.0),
            velocity: Vec2::new(1.0, 0.0),
            radius: 2.0,
        });
        for _ in 0..3 {
            run_both(&mut world);
        }
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Heatwave
        );
        for _ in 0..6 {
            run_both(&mut world);
        }
        assert_eq!(
            world.resource::<WeatherState>().current_weather,
            WeatherType::Rain,
            "seasonal weather saved on entry should be restored on exit"
        );
    }

    #[test]
    fn test_parse_and_priority_and_ip_guard() {
        assert_eq!(parse_front_weather("duststorm"), Some(WeatherType::Storm));
        assert_eq!(parse_front_weather("Drought"), Some(WeatherType::Heatwave));
        assert_eq!(parse_front_weather("bogus"), None);
        assert!(
            weather_front_priority(WeatherType::Storm)
                > weather_front_priority(WeatherType::Heatwave)
        );
        // IP guard: player-facing front strings are generic weather
        // names only — no borrowed setting terms.
        for kind in [
            WeatherType::Storm,
            WeatherType::Heatwave,
            WeatherType::Rain,
            WeatherType::Snow,
            WeatherType::Fog,
            WeatherType::MagneticStorm,
        ] {
            let name = kind.name().to_ascii_lowercase();
            for banned in ["arrakis", "dune", "sandworm", "spice"] {
                assert!(!name.contains(banned), "front name leaks IP: {name}");
            }
        }
    }
}
