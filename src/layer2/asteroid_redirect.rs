//! Spec 401: Asteroid Redirect.
//!
//! Layer 2 orbital tugs and asteroid-mounted mass drivers can alter
//! an asteroid's trajectory. A redirected rock can be parked in a
//! stable planetary orbit for long-term mining — or thrown at a
//! planet as a kinetic weapon, which destroys the rock and its
//! resource potential in a single devastating impact.
//!
//! ## Grounding audit (spec §8 pattern: adapt to the real codebase)
//!
//! * **The spec sketch's `layer2::movement::{Position, Velocity}`
//!   and `layer2::system::Planet { health }` do not exist.** The
//!   real Layer 2 movement seam is `layer2::navigation::inertial::
//!   Velocity { value: Vec3 }` (ship physics over `Transform`s — no
//!   2D system-map position at all), the real `SystemPosition`
//!   (spec 373) is an integer tile type owned by the nomad-fleet
//!   map, and the real `layer2::generation::Planet` is a healthless
//!   marker. Following the spec-400 precedent (which built its
//!   `OrbitalStation` / `StationFuelUpkeep` spec leg and bridged it
//!   onto real stations), this module builds the spec's exact type
//!   shapes here — [`Position`] / [`Velocity`] (f32, system-map
//!   units) and [`Planet`] (with the spec's `health`) — on top of
//!   the real [`SystemBody`] marker the sketch already names
//!   correctly, and bridges onto the real seams below instead of
//!   inventing parallel ones.
//! * **Stable-orbit capture (spec §5 REFACTOR) rides the real
//!   orbit seams.** A slow redirect near a planet attaches the real
//!   [`InOrbit`] fleet component (parent = the planet) *and* a real
//!   [`Orbit`] derived from the asteroid's offset, so the captured
//!   rock lives on the same orbital representation as planets,
//!   stations, and fleets — not just a changed velocity vector.
//!   Captured asteroids stop drifting in
//!   [`asteroid_movement_system`] and carry a real
//!   [`MiningTarget`], so the mining seam can work them.
//! * **Burn failure (spec §5 REFACTOR), kept light.** Each redirect
//!   rolls a deterministic splitmix64 failure roll (seed in
//!   [`AsteroidRedirectState`], chance in [`AsteroidDirector`] —
//!   the spec-400 deterministic-roll pattern, never `thread_rng`).
//!   A failed burn de-orbits the rock: its velocity is replaced by
//!   a fall toward the nearest planet at kinetic speed, and it is
//!   armed as a [`KineticWeapon`]. Tests pin the chance to 0.0 /
//!   1.0 for exact RED/GREEN.
//! * **Layer 1 impact bridge (spec §7) follows the spec-277
//!   precedent.** There is no `ImpactEvent` type in Layer 1; spec
//!   277's deorbiting megastructures bridge their surface effect
//!   through the real [`ExplosionEvent`] damage pipeline, and so
//!   does a kinetic impact here: on impact an `ExplosionEvent`
//!   fires at the planet's position mapped to a Layer 1
//!   `GridPosition`, with the weapon's damage and a damage-scaled
//!   radius. No new surface-damage type is invented.
//! * **Kinetic arming.** A redirect at or above
//!   [`KINETIC_MIN_SPEED`] arms the rock as a [`KineticWeapon`]
//!   with damage [`KINETIC_DAMAGE_PER_SPEED`] per unit of speed
//!   (speed 10 = 500 damage, the spec sketch's scale). A slow
//!   redirect disarms it. Asteroids *without* a `KineticWeapon`
//!   never damage a planet — drifting rocks are a mining
//!   opportunity, not a weapon.
//!
//! All names and strings are original and generic — "asteroid",
//! "kinetic weapon", and "mass driver" are ordinary terms. See
//! the IP-guard test.

use bevy_ecs::prelude::*;
use ratatui::style::Color;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::resources::ResourceType;
use crate::layer2::fleet::InOrbit;
use crate::layer2::lagrange::deterministic_roll;
use crate::layer2::mining::MiningTarget;
use crate::layer2::system::{Orbit, OrbitalBody, SystemBody};

/// Collision radius (spec §4): a kinetic asteroid closer than this
/// to a planet's center impacts it.
pub const COLLISION_RADIUS: f32 = 1.0;
/// A redirect ending within this distance of a planet may capture.
pub const CAPTURE_RADIUS: f32 = 5.0;
/// Fastest redirect (system-map units per tick) that can capture
/// into a stable orbit; faster rocks fly past or burn up as weapons.
pub const CAPTURE_MAX_SPEED: f32 = 1.0;
/// Slowest redirect that arms the rock as a kinetic weapon.
pub const KINETIC_MIN_SPEED: f32 = 5.0;
/// Kinetic damage per unit of redirect speed.
pub const KINETIC_DAMAGE_PER_SPEED: f32 = 50.0;
/// Default per-redirect burn-failure chance (spec §5).
pub const DEFAULT_BURN_FAILURE_CHANCE: f32 = 0.05;
/// Default game seed for the deterministic failure rolls.
pub const DEFAULT_ASTEROID_SEED: u64 = 0x0401_5EED_A57E_0001;
/// Mineable stock a freshly spawned asteroid carries.
pub const ASTEROID_RESOURCE_AMOUNT: f32 = 100.0;

/// Position on the Layer 2 system map (spec type — see the module
/// grounding audit). Distinct from the nomad map's integer
/// `SystemPosition` and from ship `Transform`s.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    /// Map column (system-map units).
    pub x: f32,
    /// Map row (system-map units).
    pub y: f32,
}

impl Position {
    /// Euclidean distance to another position.
    #[must_use]
    pub fn distance_to(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Velocity on the Layer 2 system map (spec type, exact sketch
/// shape). Units are system-map units per tick.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity {
    /// Velocity along the map column axis.
    pub x: f32,
    /// Velocity along the map row axis.
    pub y: f32,
}

impl Velocity {
    /// Speed (vector magnitude).
    #[must_use]
    pub fn speed(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

/// A planet that can be struck (spec type, exact sketch shape —
/// see the grounding audit: the real `generation::Planet` is a
/// healthless marker, so the health leg is built here and carried
/// by real `SystemBody` planet entities).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Planet {
    /// Structural health of the planet's surface/colony.
    pub health: f32,
}

/// The rock has been weaponized (spec type, exact sketch shape):
/// on collision it deals `damage` to the planet and is destroyed.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct KineticWeapon {
    /// Damage dealt to `Planet` health on impact.
    pub damage: f32,
}

/// Spec event (exact sketch shape): redirect one asteroid to a
/// new velocity. Processed by [`process_asteroid_redirect_system`].
#[derive(Event, Debug, Clone, Copy)]
pub struct RedirectAsteroidEvent {
    /// The asteroid (`SystemBody` + [`Velocity`]) to redirect.
    pub target_asteroid: Entity,
    /// The velocity the tugs / mass drivers impart.
    pub new_velocity: Velocity,
}

/// Ledger + deterministic seed for the redirect systems.
#[derive(Resource, Debug, Clone)]
pub struct AsteroidRedirectState {
    /// Game seed for the deterministic burn-failure rolls.
    pub seed: u64,
    /// Redirects successfully applied so far.
    pub redirects: u64,
    /// Burn failures suffered so far.
    pub burn_failures: u64,
    /// Asteroids captured into stable orbits so far.
    pub captures: u64,
    /// Kinetic impacts so far.
    pub impacts: u64,
}

impl Default for AsteroidRedirectState {
    fn default() -> Self {
        Self {
            seed: DEFAULT_ASTEROID_SEED,
            redirects: 0,
            burn_failures: 0,
            captures: 0,
            impacts: 0,
        }
    }
}

/// Tuning knobs for the redirect pipeline. Tests pin these.
#[derive(Resource, Debug, Clone, Copy)]
pub struct AsteroidDirector {
    /// Chance that one redirect burn fails and de-orbits the rock.
    pub burn_failure_chance: f32,
}

impl Default for AsteroidDirector {
    fn default() -> Self {
        Self {
            burn_failure_chance: DEFAULT_BURN_FAILURE_CHANCE,
        }
    }
}

/// Planet candidates for capture / failure targeting: the module
/// [`Planet`] leg by [`Position`], plus real orbiting bodies by
/// their polar [`Orbit`] position.
fn planet_candidates(
    planets: &Query<(Entity, &Position), With<Planet>>,
    orbit_bodies: &Query<(Entity, &Orbit)>,
) -> Vec<(Entity, Position)> {
    let mut out: Vec<(Entity, Position)> =
        planets.iter().map(|(entity, pos)| (entity, *pos)).collect();
    for (entity, orbit) in orbit_bodies.iter() {
        if out.iter().any(|(e, _)| *e == entity) {
            continue;
        }
        out.push((
            entity,
            Position {
                x: orbit.radius * orbit.angle.cos(),
                y: orbit.radius * orbit.angle.sin(),
            },
        ));
    }
    out
}

fn nearest_planet(
    candidates: &[(Entity, Position)],
    from: Position,
) -> Option<(Entity, Position, f32)> {
    let mut best: Option<(Entity, Position, f32)> = None;
    for (entity, pos) in candidates {
        let dist = from.distance_to(*pos);
        if best.is_none_or(|(_, _, d)| dist < d) {
            best = Some((*entity, *pos, dist));
        }
    }
    best
}

/// Spec system (exact sketch name/signature shape, extended with
/// the §5 refactor legs): apply [`RedirectAsteroidEvent`]s.
///
/// Per event, in order:
/// 1. **Burn-failure roll** (deterministic): on failure the rock
///    de-orbits — velocity toward the nearest planet at kinetic
///    speed, armed as a weapon, chronicle entry.
/// 2. **Stable-orbit capture**: a slow redirect ending within
///    [`CAPTURE_RADIUS`] of a planet attaches the real [`InOrbit`]
///    + [`Orbit`] pair and disarms the rock.
/// 3. **Kinetic arming**: speed at/above [`KINETIC_MIN_SPEED`]
///    arms (or re-arms) the rock; a fast redirect also breaks any
///    existing captured orbit. A slow uncaptured redirect disarms.
#[allow(clippy::too_many_arguments)]
pub fn process_asteroid_redirect_system(
    mut events: EventReader<RedirectAsteroidEvent>,
    mut commands: Commands,
    mut asteroids: Query<(&mut Velocity, &Position), With<SystemBody>>,
    planets: Query<(Entity, &Position), With<Planet>>,
    orbit_bodies: Query<(Entity, &Orbit)>,
    director: Option<Res<AsteroidDirector>>,
    state: Option<ResMut<AsteroidRedirectState>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let failure_chance = director.map_or(DEFAULT_BURN_FAILURE_CHANCE, |d| d.burn_failure_chance);
    let seed = state.as_ref().map_or(DEFAULT_ASTEROID_SEED, |s| s.seed);
    let candidates = planet_candidates(&planets, &orbit_bodies);
    // Counters are accumulated locally and folded into the state
    // resource at the end (the reader loop borrows nothing else).
    let mut applied = 0u64;
    let mut failures = 0u64;
    let mut captures = 0u64;
    for event in events.read() {
        let Ok((mut velocity, position)) = asteroids.get_mut(event.target_asteroid) else {
            continue;
        };
        let position = *position;
        let ordinal = state.as_ref().map_or(applied, |s| s.redirects + applied);
        applied += 1;
        let roll = deterministic_roll(seed, ordinal, event.target_asteroid.to_bits());
        if roll < failure_chance {
            if let Some((_, planet_pos, _)) = nearest_planet(&candidates, position) {
                // De-orbit: fall toward the planet at kinetic speed.
                let dx = planet_pos.x - position.x;
                let dy = planet_pos.y - position.y;
                let len = (dx * dx + dy * dy).sqrt();
                let speed = event.new_velocity.speed().max(KINETIC_MIN_SPEED);
                if len > f32::EPSILON {
                    velocity.x = dx / len * speed;
                    velocity.y = dy / len * speed;
                }
                failures += 1;
                commands
                    .entity(event.target_asteroid)
                    .remove::<InOrbit>()
                    .insert(KineticWeapon {
                        damage: speed * KINETIC_DAMAGE_PER_SPEED,
                    });
                chronicle.send(AddChronicleEvent {
                    text: "A redirect burn has gone wrong — an engine failure mid-burn has sent the asteroid falling toward the planet instead of along its plotted course.".to_string(),
                    importance: EventImportance::Major,
                });
                continue;
            }
        }
        velocity.x = event.new_velocity.x;
        velocity.y = event.new_velocity.y;
        let speed = event.new_velocity.speed();
        let capture = if speed <= CAPTURE_MAX_SPEED {
            nearest_planet(&candidates, position).filter(|(_, _, dist)| *dist <= CAPTURE_RADIUS)
        } else {
            None
        };
        if let Some((parent, _, dist)) = capture {
            let parent_pos = candidates
                .iter()
                .find(|(e, _)| *e == parent)
                .map(|(_, p)| *p)
                .unwrap_or_default();
            commands
                .entity(event.target_asteroid)
                .insert(InOrbit { parent })
                .insert(Orbit {
                    parent,
                    radius: dist.max(0.5),
                    speed: 0.01,
                    angle: (position.y - parent_pos.y).atan2(position.x - parent_pos.x),
                })
                .remove::<KineticWeapon>();
            captures += 1;
            chronicle.send(AddChronicleEvent {
                text: "A redirected asteroid has settled into a stable orbit — the tugs have nudged the rock into a parking orbit where mining crews can work it safely for years.".to_string(),
                importance: EventImportance::Standard,
            });
        } else if speed >= KINETIC_MIN_SPEED {
            commands
                .entity(event.target_asteroid)
                .remove::<InOrbit>()
                .insert(KineticWeapon {
                    damage: speed * KINETIC_DAMAGE_PER_SPEED,
                });
        } else {
            commands
                .entity(event.target_asteroid)
                .remove::<KineticWeapon>();
        }
    }
    if let Some(mut state) = state {
        state.redirects += applied;
        state.burn_failures += failures;
        state.captures += captures;
    }
}

/// Drift: uncaptured asteroids advance by their velocity each
/// tick. Captured asteroids ([`InOrbit`]) hold their orbit — the
/// [`Orbit`] component owns their motion from here.
#[allow(clippy::type_complexity)]
pub fn asteroid_movement_system(
    mut asteroids: Query<(&mut Position, &Velocity), (With<SystemBody>, Without<InOrbit>)>,
) {
    for (mut position, velocity) in asteroids.iter_mut() {
        position.x += velocity.x;
        position.y += velocity.y;
    }
}

/// Spec system (exact sketch name): kinetic impacts. An asteroid
/// carrying [`KineticWeapon`] within [`COLLISION_RADIUS`] of a
/// [`Planet`] deals its damage to the planet's health, fires the
/// Layer 1 [`ExplosionEvent`] bridge (spec §7, spec-277
/// precedent), writes a Major chronicle entry, and is despawned.
/// Unarmed asteroids never impact.
pub fn check_asteroid_impacts_system(
    mut commands: Commands,
    asteroid_query: Query<(Entity, &KineticWeapon, &Position)>,
    mut planet_query: Query<(&mut Planet, &Position)>,
    state: Option<ResMut<AsteroidRedirectState>>,
    mut explosions: EventWriter<ExplosionEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut impacts = 0u64;
    for (a_ent, weapon, a_pos) in asteroid_query.iter() {
        for (mut planet, p_pos) in planet_query.iter_mut() {
            if a_pos.distance_to(*p_pos) < COLLISION_RADIUS {
                planet.health = (planet.health - weapon.damage).max(0.0);
                impacts += 1;
                explosions.send(ExplosionEvent {
                    center: GridPosition {
                        x: (p_pos.x.round() as i32).max(0),
                        y: (p_pos.y.round() as i32).max(0),
                    },
                    damage: weapon.damage,
                    radius: ((weapon.damage / 200.0).ceil() as u32).clamp(1, 8),
                });
                chronicle.send(AddChronicleEvent {
                    text: "A redirected asteroid has struck the planet as a kinetic weapon — the impact has devastated the surface below, and the rock and all it held are gone.".to_string(),
                    importance: EventImportance::Major,
                });
                commands.entity(a_ent).despawn();
                break;
            }
        }
    }
    if let Some(mut state) = state {
        state.impacts += impacts;
    }
}

// ---------------------------------------------------------------------------
// World helpers (headless / census)
// ---------------------------------------------------------------------------

/// Census of the redirect layer for STATS / panels.
#[derive(Debug, Clone, Copy, Default)]
pub struct AsteroidCensus {
    /// Asteroids on the system map (redirect-leg bodies).
    pub asteroids: usize,
    /// Asteroids currently armed as kinetic weapons.
    pub kinetic: usize,
    /// Asteroids captured into stable orbits.
    pub captured: usize,
    /// Redirects applied (ledger).
    pub redirects: u64,
    /// Kinetic impacts (ledger).
    pub impacts: u64,
}

/// Census of asteroids, weapons, captures, and the ledger.
#[must_use]
pub fn asteroid_census(world: &mut World) -> AsteroidCensus {
    let asteroids = {
        let mut q = world.query_filtered::<(), (With<SystemBody>, With<Velocity>)>();
        q.iter(world).count()
    };
    let kinetic = {
        let mut q = world.query::<&KineticWeapon>();
        q.iter(world).count()
    };
    let captured = {
        let mut q = world.query_filtered::<&InOrbit, With<Velocity>>();
        q.iter(world).count()
    };
    let (redirects, impacts) = world
        .get_resource::<AsteroidRedirectState>()
        .map_or((0, 0), |s| (s.redirects, s.impacts));
    AsteroidCensus {
        asteroids,
        kinetic,
        captured,
        redirects,
        impacts,
    }
}

/// Compact STATS label, e.g. `3ast/1kinetic/1captured/0impacts`.
/// No spaces.
#[must_use]
pub fn asteroid_stats_label(world: &mut World) -> String {
    let census = asteroid_census(world);
    format!(
        "{}ast/{}kinetic/{}captured/{}impacts",
        census.asteroids, census.kinetic, census.captured, census.impacts
    )
}

/// Spawn a redirectable asteroid: a real [`SystemBody`] carrying
/// the spec leg ([`Position`] / [`Velocity`]) plus a real
/// [`MiningTarget`], so a captured rock can actually be mined.
pub fn spawn_asteroid(world: &mut World, pos: Position, vel: Velocity) -> Entity {
    world
        .spawn((
            SystemBody,
            pos,
            vel,
            MiningTarget {
                resource_type: ResourceType::Metal,
                amount: ASTEROID_RESOURCE_AMOUNT,
                mining_difficulty: 1.0,
            },
            OrbitalBody {
                name: "Asteroid".to_string(),
                radius: 0.4,
                color: Color::Gray,
                char: '*',
            },
        ))
        .id()
}

/// Spawn the demo target planet for headless play: a real
/// [`SystemBody`] carrying the [`Planet`] health leg.
pub fn spawn_redirect_planet(world: &mut World, pos: Position, health: f32) -> Entity {
    world
        .spawn((
            SystemBody,
            Planet { health },
            pos,
            OrbitalBody {
                name: "Target World".to_string(),
                radius: 1.0,
                color: Color::Blue,
                char: 'O',
            },
        ))
        .id()
}

/// The first redirectable asteroid on the map, if any (headless
/// `redirect` targets it).
#[must_use]
pub fn first_asteroid(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<Entity, (With<SystemBody>, With<Velocity>)>();
    q.iter(world).next()
}

/// The first [`Planet`]-leg planet on the map, if any.
#[must_use]
pub fn first_planet(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<Entity, With<Planet>>();
    q.iter(world).next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{App, Update};

    fn base_app() -> App {
        let mut app = App::new();
        app.add_event::<RedirectAsteroidEvent>();
        // Adaptation: the spec sketch's bare app omits the two
        // bridge buses the real systems write to (chronicle +
        // Layer 1 explosions); the real simulation registers both.
        app.add_event::<AddChronicleEvent>();
        app.add_event::<ExplosionEvent>();
        app
    }

    // Spec §3 RED test 1, adapted only in type paths (the sketch's
    // `crate::layer2::movement::{Position, Velocity}` do not exist;
    // the types live here — see the grounding audit).
    #[test]
    fn test_asteroid_redirect_changes_velocity() {
        let mut app = base_app();
        app.add_systems(Update, process_asteroid_redirect_system);

        let asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 0.0, y: 0.0 },
                Velocity { x: 1.0, y: 0.0 },
            ))
            .id();

        app.world_mut().send_event(RedirectAsteroidEvent {
            target_asteroid: asteroid,
            new_velocity: Velocity { x: 0.0, y: 1.0 },
        });

        app.update();

        let velocity = app.world().get::<Velocity>(asteroid).unwrap();
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 1.0);
    }

    // Spec §3 RED test 2, adapted only in type paths.
    #[test]
    fn test_asteroid_kinetic_impact() {
        let mut app = base_app();
        app.add_systems(Update, check_asteroid_impacts_system);

        let planet = app
            .world_mut()
            .spawn((
                SystemBody,
                Planet { health: 1000.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();

        let asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                KineticWeapon { damage: 500.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();

        app.update();

        let p_data = app.world().get::<Planet>(planet).unwrap();
        assert_eq!(p_data.health, 500.0);
        assert!(
            app.world().get_entity(asteroid).is_err()
                || app.world().get::<SystemBody>(asteroid).is_none()
        );
    }

    #[test]
    fn test_redirect_arms_kinetic_weapon_at_high_speed() {
        let mut app = base_app();
        app.insert_resource(AsteroidDirector {
            burn_failure_chance: 0.0,
        });
        app.add_systems(Update, process_asteroid_redirect_system);
        let asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 0.0, y: 0.0 },
                Velocity { x: 0.0, y: 0.0 },
            ))
            .id();
        app.world_mut().send_event(RedirectAsteroidEvent {
            target_asteroid: asteroid,
            new_velocity: Velocity { x: 10.0, y: 0.0 },
        });
        app.update();
        let weapon = app.world().get::<KineticWeapon>(asteroid).unwrap();
        assert_eq!(weapon.damage, 500.0);
    }

    #[test]
    fn test_redirect_captures_stable_orbit_near_planet() {
        let mut app = base_app();
        app.insert_resource(AsteroidDirector {
            burn_failure_chance: 0.0,
        });
        app.add_systems(Update, process_asteroid_redirect_system);
        let planet = app
            .world_mut()
            .spawn((
                SystemBody,
                Planet { health: 1000.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();
        let asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 12.0, y: 10.0 },
                Velocity { x: 1.0, y: 0.0 },
            ))
            .id();
        app.world_mut().send_event(RedirectAsteroidEvent {
            target_asteroid: asteroid,
            new_velocity: Velocity { x: 0.2, y: 0.0 },
        });
        app.update();
        let orbit = app.world().get::<InOrbit>(asteroid).unwrap();
        assert_eq!(orbit.parent, planet);
        assert!(app.world().get::<Orbit>(asteroid).is_some());
        assert!(app.world().get::<KineticWeapon>(asteroid).is_none());
    }

    #[test]
    fn test_burn_failure_deorbits_toward_planet() {
        let mut app = base_app();
        app.insert_resource(AsteroidDirector {
            burn_failure_chance: 1.0,
        });
        app.add_systems(Update, process_asteroid_redirect_system);
        let _planet = app
            .world_mut()
            .spawn((
                SystemBody,
                Planet { health: 1000.0 },
                Position { x: 10.0, y: 0.0 },
            ))
            .id();
        let asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 0.0, y: 0.0 },
                Velocity { x: 0.0, y: 0.0 },
            ))
            .id();
        // The plotted course points AWAY from the planet.
        app.world_mut().send_event(RedirectAsteroidEvent {
            target_asteroid: asteroid,
            new_velocity: Velocity { x: -1.0, y: 0.0 },
        });
        app.update();
        let velocity = app.world().get::<Velocity>(asteroid).unwrap();
        assert!(
            velocity.x > 0.0,
            "failed burn should fall toward the planet, got {velocity:?}"
        );
        assert!(app.world().get::<KineticWeapon>(asteroid).is_some());
    }

    #[test]
    fn test_movement_drifts_free_asteroid_but_not_captured() {
        let mut app = base_app();
        app.add_systems(Update, asteroid_movement_system);
        let free = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 0.0, y: 0.0 },
                Velocity { x: 1.0, y: 2.0 },
            ))
            .id();
        let anchor = app.world_mut().spawn_empty().id();
        let captured = app
            .world_mut()
            .spawn((
                SystemBody,
                Position { x: 5.0, y: 5.0 },
                Velocity { x: 1.0, y: 0.0 },
                InOrbit { parent: anchor },
            ))
            .id();
        app.update();
        assert_eq!(
            *app.world().get::<Position>(free).unwrap(),
            Position { x: 1.0, y: 2.0 }
        );
        assert_eq!(
            *app.world().get::<Position>(captured).unwrap(),
            Position { x: 5.0, y: 5.0 }
        );
    }

    #[test]
    fn test_unarmed_asteroid_does_not_damage_planet() {
        let mut app = base_app();
        app.add_systems(Update, check_asteroid_impacts_system);
        let planet = app
            .world_mut()
            .spawn((
                SystemBody,
                Planet { health: 1000.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();
        let asteroid = app
            .world_mut()
            .spawn((SystemBody, Position { x: 10.0, y: 10.0 }))
            .id();
        app.update();
        assert_eq!(app.world().get::<Planet>(planet).unwrap().health, 1000.0);
        assert!(app.world().get_entity(asteroid).is_ok());
    }

    #[test]
    fn test_impact_fires_layer1_explosion_bridge() {
        let mut app = base_app();
        app.add_systems(Update, check_asteroid_impacts_system);
        let _planet = app
            .world_mut()
            .spawn((
                SystemBody,
                Planet { health: 1000.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();
        let _asteroid = app
            .world_mut()
            .spawn((
                SystemBody,
                KineticWeapon { damage: 500.0 },
                Position { x: 10.0, y: 10.0 },
            ))
            .id();
        app.update();
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut cursor = events.get_cursor();
        let fired: Vec<&ExplosionEvent> = cursor.read(events).collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].center, GridPosition { x: 10, y: 10 });
        assert_eq!(fired[0].damage, 500.0);
    }

    #[test]
    fn test_deterministic_failure_roll_is_stable() {
        let a = deterministic_roll(DEFAULT_ASTEROID_SEED, 7, 42);
        let b = deterministic_roll(DEFAULT_ASTEROID_SEED, 7, 42);
        assert_eq!(a, b);
        assert!((0.0..1.0).contains(&a));
    }

    #[test]
    fn test_ip_guard_no_borrowed_names() {
        const BANNED: [&str; 12] = [
            "star wars",
            "star trek",
            "deep space nine",
            "babylon 5",
            "mass effect",
            "citadel station",
            "the expanse",
            "battlestar",
            "firefly",
            "serenity",
            "elite dangerous",
            "eve online",
        ];
        let mut haystack = String::new();
        haystack.push_str("Asteroid Target World Kinetic Weapon Mass Driver Tug ");
        haystack.push_str("A redirect burn has gone wrong — an engine failure mid-burn has sent the asteroid falling toward the planet instead of along its plotted course. ");
        haystack.push_str("A redirected asteroid has settled into a stable orbit — the tugs have nudged the rock into a parking orbit where mining crews can work it safely for years. ");
        haystack.push_str("A redirected asteroid has struck the planet as a kinetic weapon — the impact has devastated the surface below, and the rock and all it held are gone.");
        let haystack = haystack.to_lowercase();
        for banned in BANNED {
            assert!(!haystack.contains(banned), "banned term present: {banned}");
        }
    }
}
