//! Spec 425: Stellar Wind Surfing.
//!
//! Layer 2 fleets equipped with solar sails ([`SolarSails`], the existing
//! component in `navigation::stellar_weather`) can catch a solar flare
//! while [`InTransit`] and ride it to their destination instantly — the
//! ride completes the transit in the same tick, with no fuel cost (the
//! real `fleet_movement_system` never burns fuel; a normal transit of
//! the same route takes `InTransit::duration` ticks). The price is
//! radiation: exposure scales with flare intensity minus the fleet's
//! shielding, scaled by sail efficiency.
//!
//! # Spec adaptations (spec §2/§3 types are mocks)
//!
//! - Spec `Fleet { current_node, destination_node, fuel }` does not
//!   exist. The real fleet is the marker [`crate::layer2::fleet::Fleet`]
//!   plus [`InTransit`] (`origin`/`destination: Entity`, `progress`,
//!   `duration`). Instant travel is therefore expressed as completing
//!   the transit: this system sets `progress = 1.0` and the real
//!   `fleet_movement_system` (ordered after this system) performs the
//!   arrival swap to `InOrbit { parent: destination }` in the same tick.
//! - Spec `SolarSail { efficiency }` does not exist; the real type is
//!   [`SolarSails`] `{ deployed, efficiency }` in `stellar_weather`.
//!   This module extends that seam — no parallel sail type is created.
//!   Only fleets with `deployed == true` and `efficiency > 0.0` ride.
//! - Spec `layer2::events::SolarFlareEvent { origin, target, intensity }`
//!   does not exist. The real flare is the unit
//!   [`SolarFlareEvent`] in `layer1::nature::solar_flare_lottery` — it
//!   carries no direction and no intensity. Consequences:
//!   * Directional matching (origin == current, target == destination)
//!     is impossible against the real event, so any flare can be ridden
//!     by any in-transit sail fleet (flares are system-wide). The spec's
//!     "sailing against a flare" edge case (§7) is therefore moot.
//!   * A bare `SolarFlareEvent` rides at [`DEFAULT_FLARE_INTENSITY`].
//!   * [`StellarWindFlare`] is the one new event this module adds: a
//!     flare with a measured intensity, used by the headless `flare`
//!     debug command and by tests that need exact radiation math.
//! - Spec `ShipConfig { shielding }` does not exist; the closest real
//!   type, [`crate::layer2::shielding::OrbitalShield`], is a station
//!   shield pool (`capacity`), not a per-fleet rating. The missing leg
//!   is built as [`FleetShielding`] `{ shielding }`.
//! - Spec `RadiationExposure { level }` (in a `layer1::health` module
//!   that does not exist) is built as [`FleetRadiationExposure`]. The
//!   gain is also mirrored into the real
//!   [`RadiationSickness`] component (`severity`, clamped to 100) on the
//!   fleet entity, so the exposure flows through the real radiation
//!   type rather than a parallel one.
//!
//! # Radiation formula (spec §3/§4, kept over the §5 percentage idea)
//!
//! `gain = max(0, intensity - shielding) * sail.efficiency`, accumulated
//! into `FleetRadiationExposure::level`. New constants introduced by
//! this spec (no unrelated balance retunes): [`DEFAULT_FLARE_INTENSITY`].

use crate::layer1::nature::radioactive::RadiationSickness;
use crate::layer1::nature::solar_flare_lottery::SolarFlareEvent;
use crate::layer2::fleet::{Fleet, InTransit};
use crate::layer2::navigation::stellar_weather::SolarSails;
use bevy_ecs::prelude::*;

/// Intensity assumed for a bare [`SolarFlareEvent`], which carries no
/// intensity field. Matches the intensity used by the spec's RED tests.
pub const DEFAULT_FLARE_INTENSITY: f32 = 100.0;

/// A solar flare with a measured intensity (Spec 425).
///
/// The real [`SolarFlareEvent`] is a unit struct; this event is the
/// intensity-carrying companion used when the flare strength matters
/// (radiation math, headless debug trigger). Fleets ride on either
/// event; if both fire in one tick the stronger intensity wins.
#[derive(Event, Debug, Clone, Copy)]
pub struct StellarWindFlare {
    /// Flare intensity (same scale as shielding: 0..=~100+).
    pub intensity: f32,
}

/// Per-fleet shielding rating (Spec 425; adapts the spec's mock
/// `ShipConfig { shielding }`). Higher shielding subtracts directly
/// from flare intensity before radiation is applied.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FleetShielding {
    /// Flat shielding rating subtracted from flare intensity.
    pub shielding: f32,
}

/// Accumulated radiation exposure of a fleet's crew (Spec 425; adapts
/// the spec's mock `RadiationExposure`). Monotonic ledger: rides add,
/// nothing in this module removes exposure.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FleetRadiationExposure {
    /// Total accumulated exposure.
    pub level: f32,
}

/// Marker inserted on a fleet that has ridden at least one flare
/// (Spec 425 §7's suggested `SolarSailingStatus`, as a unit marker).
#[derive(Component, Debug, Clone, Copy)]
pub struct WindSurfer;

/// Ledger of stellar-wind surfing activity, for STATS/headless.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct WindSurfLedger {
    /// Total completed flare rides.
    pub rides: u32,
    /// Total radiation exposure dealt by rides, summed over fleets.
    pub total_exposure: f32,
    /// Intensity of the most recent flare that produced a ride.
    pub last_intensity: f32,
}

/// Pure radiation math (spec §4): exposure gained from one ride.
///
/// `max(0, intensity - shielding) * efficiency` — full shielding means
/// a free, harmless ride; undeployed/inefficient sails are handled by
/// the caller (no ride at all), not here.
#[must_use]
pub fn flare_exposure(intensity: f32, shielding: f32, efficiency: f32) -> f32 {
    (intensity - shielding).max(0.0) * efficiency
}

/// Stellar Wind Surfing system (Spec 425).
///
/// Reads flare events with `EventReader` (spec §5: never drains, so
/// the EMP / radiation / isotope systems still see the same flare),
/// then completes the transit of every sail-equipped fleet in transit
/// and applies radiation. Order this `.before(fleet_movement_system)`
/// so the real movement system performs the arrival in the same tick.
#[allow(clippy::type_complexity)]
pub fn stellar_wind_surfing_system(
    mut commands: Commands,
    mut flares: EventReader<SolarFlareEvent>,
    mut wind_flares: EventReader<StellarWindFlare>,
    ledger: Option<ResMut<WindSurfLedger>>,
    mut fleets: Query<
        (
            Entity,
            &mut InTransit,
            &SolarSails,
            Option<&FleetShielding>,
            Option<&mut FleetRadiationExposure>,
            Option<&mut RadiationSickness>,
        ),
        With<Fleet>,
    >,
) {
    let mut intensity: Option<f32> = None;
    for _ in flares.read() {
        intensity = Some(intensity.map_or(DEFAULT_FLARE_INTENSITY, |i: f32| {
            i.max(DEFAULT_FLARE_INTENSITY)
        }));
    }
    for flare in wind_flares.read() {
        intensity = Some(intensity.map_or(flare.intensity, |i: f32| i.max(flare.intensity)));
    }
    let Some(intensity) = intensity else {
        return;
    };

    let mut rides: u32 = 0;
    let mut dealt: f32 = 0.0;
    for (entity, mut transit, sails, shielding, exposure, sickness) in &mut fleets {
        if !sails.deployed || sails.efficiency <= 0.0 {
            continue;
        }
        let gain = flare_exposure(
            intensity,
            shielding.map_or(0.0, |s| s.shielding),
            sails.efficiency,
        );
        // Instant, fuel-free travel: completing progress lets the real
        // fleet_movement_system arrive the fleet this same tick.
        transit.progress = 1.0;
        rides += 1;
        dealt += gain;
        match exposure {
            Some(mut exposure) => exposure.level += gain,
            None => {
                commands
                    .entity(entity)
                    .insert(FleetRadiationExposure { level: gain });
            }
        }
        match sickness {
            Some(mut sickness) => {
                sickness.severity = (sickness.severity + gain).min(100.0);
            }
            None if gain > 0.0 => {
                commands.entity(entity).insert(RadiationSickness {
                    severity: gain.min(100.0),
                });
            }
            None => {}
        }
        commands.entity(entity).insert(WindSurfer);
    }

    if rides > 0 {
        if let Some(mut ledger) = ledger {
            ledger.rides += rides;
            ledger.total_exposure += dealt;
            ledger.last_intensity = intensity;
        }
    }
}

/// STATS census for headless: `(rides, total_exposure, surfing_fleets)`.
/// Follows the `megafauna_stats_counts` pattern — plain `World`
/// function, no App required.
#[must_use]
pub fn wind_surf_stats_counts(world: &mut World) -> (u32, f32, usize) {
    let (rides, total) = world
        .get_resource::<WindSurfLedger>()
        .map_or((0, 0.0), |l| (l.rides, l.total_exposure));
    let surfers = world.query::<&WindSurfer>().iter(world).count();
    (rides, total, surfers)
}

/// Sends a [`StellarWindFlare`] into `world` (initializing the event
/// channel first — the first-tick pattern). Shared by the headless
/// debug command and tests.
pub fn send_wind_flare(world: &mut World, intensity: f32) {
    world.init_resource::<Events<StellarWindFlare>>();
    world
        .resource_mut::<Events<StellarWindFlare>>()
        .send(StellarWindFlare { intensity });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer2::fleet::{fleet_movement_system, InOrbit};
    use bevy_ecs::system::RunSystemOnce;

    fn run_surf(world: &mut World) {
        world.run_system_once(stellar_wind_surfing_system).unwrap();
        world.flush();
    }

    fn spawn_transit_fleet(world: &mut World, dest: Entity) -> (Entity, Entity) {
        let origin = world.spawn_empty().id();
        let fleet = world
            .spawn((
                Fleet,
                InTransit {
                    origin,
                    destination: dest,
                    progress: 0.0,
                    duration: 100.0,
                },
            ))
            .id();
        (origin, fleet)
    }

    /// Spec RED test 1, adapted: a sail fleet in transit rides a flare
    /// to instant arrival; the (fuel-free) ride costs no ticks beyond
    /// the one it fires on, unlike the 100-tick normal transit.
    #[test]
    fn test_solar_sail_instant_travel() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        world.insert_resource(WindSurfLedger::default());
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: true,
                efficiency: 1.0,
            },
            FleetShielding { shielding: 50.0 },
            FleetRadiationExposure { level: 0.0 },
        ));

        send_wind_flare(&mut world, 100.0);
        run_surf(&mut world);

        // Surfing completes progress; the real movement system then
        // performs the arrival in the same tick.
        let mut schedule = Schedule::default();
        schedule.add_systems(fleet_movement_system);
        schedule.run(&mut world);

        assert!(
            world.get::<InTransit>(fleet).is_none(),
            "Fleet should have arrived instantly"
        );
        let orbit = world
            .get::<InOrbit>(fleet)
            .expect("Fleet should be in orbit at its destination");
        assert_eq!(orbit.parent, dest);
        // Shielding 50 vs intensity 100 at efficiency 1.0.
        assert_eq!(
            world.get::<FleetRadiationExposure>(fleet).unwrap().level,
            50.0
        );
        assert_eq!(world.resource::<WindSurfLedger>().rides, 1);
    }

    /// Spec RED test 2, adapted verbatim in math: radiation equals
    /// intensity minus shielding (100 - 10 = 90).
    #[test]
    fn test_solar_sail_radiation_damage() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: true,
                efficiency: 1.0,
            },
            FleetShielding { shielding: 10.0 },
            FleetRadiationExposure { level: 0.0 },
        ));

        send_wind_flare(&mut world, 100.0);
        run_surf(&mut world);

        let exposure = world.get::<FleetRadiationExposure>(fleet).unwrap();
        assert_eq!(
            exposure.level, 90.0,
            "Crew should suffer heavy radiation due to low shielding"
        );
        // The gain is mirrored into the real RadiationSickness type.
        let sickness = world
            .get::<RadiationSickness>(fleet)
            .expect("Ride should apply RadiationSickness");
        assert_eq!(sickness.severity, 90.0);
    }

    /// Spec RED test 3, adapted: a fleet WITHOUT sails is unaffected
    /// by a flare — no instant arrival, no exposure; its transit
    /// advances only by the normal movement step (the real system's
    /// analogue of "normal travel consumes fuel / is not instant":
    /// fleet_movement_system burns time, 1/duration per tick).
    #[test]
    fn test_normal_travel_unaffected() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            FleetShielding { shielding: 50.0 },
            FleetRadiationExposure { level: 0.0 },
        ));

        send_wind_flare(&mut world, 100.0);
        run_surf(&mut world);

        let transit = world
            .get::<InTransit>(fleet)
            .expect("Sail-less fleet must still be in transit");
        assert_eq!(transit.progress, 0.0, "Surfing must not move it");
        assert_eq!(
            world.get::<FleetRadiationExposure>(fleet).unwrap().level,
            0.0
        );

        // One normal movement tick advances it by exactly 1/duration.
        let mut schedule = Schedule::default();
        schedule.add_systems(fleet_movement_system);
        schedule.run(&mut world);
        let transit = world.get::<InTransit>(fleet).unwrap();
        assert!((transit.progress - 0.01).abs() < f32::EPSILON);
    }

    #[test]
    fn test_no_flare_no_ride() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: true,
                efficiency: 1.0,
            },
            FleetRadiationExposure { level: 0.0 },
        ));

        run_surf(&mut world);

        let transit = world.get::<InTransit>(fleet).unwrap();
        assert_eq!(transit.progress, 0.0);
        assert_eq!(
            world.get::<FleetRadiationExposure>(fleet).unwrap().level,
            0.0
        );
    }

    #[test]
    fn test_undeployed_sails_do_not_ride() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: false,
                efficiency: 1.0,
            },
            FleetRadiationExposure { level: 0.0 },
        ));

        send_wind_flare(&mut world, 100.0);
        run_surf(&mut world);

        assert_eq!(world.get::<InTransit>(fleet).unwrap().progress, 0.0);
    }

    #[test]
    fn test_full_shielding_rides_for_free() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: true,
                efficiency: 1.0,
            },
            FleetShielding { shielding: 150.0 },
            FleetRadiationExposure { level: 0.0 },
        ));

        send_wind_flare(&mut world, 100.0);
        run_surf(&mut world);

        assert_eq!(world.get::<InTransit>(fleet).unwrap().progress, 1.0);
        assert_eq!(
            world.get::<FleetRadiationExposure>(fleet).unwrap().level,
            0.0
        );
        assert!(world.get::<RadiationSickness>(fleet).is_none());
    }

    #[test]
    fn test_bare_solar_flare_event_rides_at_default_intensity() {
        let mut world = World::new();
        world.init_resource::<Events<SolarFlareEvent>>();
        world.init_resource::<Events<StellarWindFlare>>();
        let dest = world.spawn_empty().id();
        let (_, fleet) = spawn_transit_fleet(&mut world, dest);
        world.entity_mut(fleet).insert((
            SolarSails {
                deployed: true,
                efficiency: 1.0,
            },
            FleetShielding { shielding: 10.0 },
            FleetRadiationExposure { level: 0.0 },
        ));

        world
            .resource_mut::<Events<SolarFlareEvent>>()
            .send(SolarFlareEvent);
        run_surf(&mut world);

        assert_eq!(world.get::<InTransit>(fleet).unwrap().progress, 1.0);
        assert_eq!(
            world.get::<FleetRadiationExposure>(fleet).unwrap().level,
            DEFAULT_FLARE_INTENSITY - 10.0
        );
    }

    #[test]
    fn test_flare_exposure_math() {
        assert_eq!(flare_exposure(100.0, 10.0, 1.0), 90.0);
        assert_eq!(flare_exposure(100.0, 50.0, 1.0), 50.0);
        assert_eq!(flare_exposure(100.0, 150.0, 1.0), 0.0);
        assert_eq!(flare_exposure(100.0, 10.0, 0.5), 45.0);
        assert_eq!(flare_exposure(0.0, 0.0, 1.0), 0.0);
    }

    /// IP guard: player-facing strings introduced by this spec are
    /// generic/original — no named IP, characters, or places.
    #[test]
    fn test_ip_guard_strings_are_generic() {
        let strings = ["Stellar Wind Surfing", "A fleet rode a solar flare."];
        let banned = [
            "anacreon",
            "terminus",
            "trantor",
            "seldon",
            "arrakis",
            "enterprise",
            "millennium",
            "nostromo",
        ];
        for s in strings {
            let lower = s.to_lowercase();
            for term in banned {
                assert!(!lower.contains(term), "IP term '{term}' in '{s}'");
            }
        }
    }
}
