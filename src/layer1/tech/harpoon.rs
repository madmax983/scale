//! Spec 324: Kinetic Harpoons.
//!
//! Layer-1 harpoon tether launchers snag passing comets and small asteroids
//! drifting through the system, then winch them down for immediate, massive
//! resource extraction. A clean catch delivers a fortune in metal, ore, and
//! stone; an overweight body snaps the winch and comes down as a catastrophe.
//!
//! ## Mechanics
//! * [`HarpoonLauncher`] buildings (BuildingType::HarpoonLauncher, Astronomy
//!   tech gate) fire a tether at an orbiting [`WandererBody`] (comet or
//!   asteroid) via [`HarpoonLaunchEvent`].
//! * The tether becomes a [`WinchSystem`] that reels the body in over
//!   ~40 ticks of powered operation ([`WINCH_PROGRESS_PER_TICK`] per tick).
//! * Reeling needs grid power: the launcher's
//!   [`PowerConsumer`][crate::layer1::energy::PowerConsumer] must be active
//!   (managed by the live power grid system — brownouts and blackouts stall
//!   the winch). No power, no progress.
//! * Arrival: a small [`ExplosionEvent`][crate::layer1::environment::volatile::ExplosionEvent]
//!   (the real volatile/explosion damage pipeline shared with spec 184
//!   Orbital Debris) gives localized impact damage, the tile becomes
//!   [`TerrainType::Crater`][crate::layer1::nature::terrain::TerrainType],
//!   and the yield is credited to
//!   [`ColonyResources`][crate::layer1::economy::resources::ColonyResources]
//!   (metal / ore / stone split).
//! * Overweight body (mass > winch strength): the winch snaps — a big
//!   [`ExplosionEvent`], zero yield, [`OrbitalDebris`][crate::layer2::debris::OrbitalDebris]
//!   fed past the Kessler line, Major chronicle, launcher left snapped.
//! * Passing wanderers spawn on their own: [`WandererDirector`] drops a new
//!   orbiting comet/asteroid every few thousand ticks when the sky is empty.
//!
//! ## Spec adaptations
//! * Spec 149 (Cometary Injection) is unbuilt — there are no Layer-2 comet
//!   types in the codebase — so `WandererBody`/`BodyStatus` live here on
//!   Layer 1 instead of reusing nonexistent Layer-2 types. No parallel
//!   invention: when 149 lands, these bodies are the snag targets.
//! * The sketch's `ImpactEvent` maps onto the real [`ExplosionEvent`]
//!   pipeline (volatile.rs, shared with spec 184 Orbital Debris) plus a
//!   resource-credit/crater listener; the sketch's `damage_radius` and
//!   `yield_amount` semantics are asserted through those real types in
//!   the tests.
//! * §7's Layer-2 overlay targeting UI is intentionally headless/console
//!   only this pass (same call as spec 277); noted as a follow-on.
//! * All names are original and generic (see the IP-guard test).

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::energy::PowerConsumer;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer2::debris::OrbitalDebris;
use crate::shared::log::MessageLog;

/// Default winch strength of a fresh harpoon launcher (mass units).
pub const WINCH_DEFAULT_STRENGTH: u32 = 3000;
/// Winch progress per powered tick (0.0 = orbit, 100.0 = ground).
pub const WINCH_PROGRESS_PER_TICK: f32 = 2.5;
/// Power draw of an active winch, in grid units per tick.
pub const WINCH_POWER_DRAW: f32 = 30.0;
/// Damage of a successful catch's localized impact.
pub const CATCH_DAMAGE: f32 = 15.0;
/// Blast radius of a successful catch's localized impact.
pub const CATCH_RADIUS: u32 = 1;
/// Damage of a winch-snap catastrophic impact.
pub const SNAP_DAMAGE: f32 = 60.0;
/// Blast radius of a winch-snap catastrophic impact.
pub const SNAP_RADIUS: u32 = 5;
/// Orbital-debris feed on a catastrophic impact (past the 0.1 Kessler line).
pub const SNAP_DEBRIS_FEED: f32 = 0.15;
/// Min ticks between passing-wanderer spawns.
pub const WANDERER_SPAWN_TICKS_MIN: u32 = 1500;
/// Max ticks between passing-wanderer spawns.
pub const WANDERER_SPAWN_TICKS_MAX: u32 = 3000;
/// Share of the yield credited as metal.
pub const YIELD_METAL_SHARE: f32 = 0.6;
/// Share of the yield credited as ore.
pub const YIELD_ORE_SHARE: f32 = 0.25;
/// Share of the yield credited as stone.
pub const YIELD_STONE_SHARE: f32 = 0.15;

/// Kind of orbital wanderer a harpoon can snag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WandererKind {
    /// An icy comet: light, rich in volatiles — the classic prize.
    #[default]
    Comet,
    /// A rocky small asteroid: heavier, denser yield.
    Asteroid,
}

impl WandererKind {
    /// Human-readable label for chronicles and headless output.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Comet => "comet",
            Self::Asteroid => "asteroid",
        }
    }
}

/// Snag status of an orbital wanderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyStatus {
    /// Drifting through the system, available as a target.
    #[default]
    Orbiting,
    /// Tethered to a launcher and being reeled down.
    Snagged,
}

/// A passing comet or small asteroid in a snaggable orbit.
#[derive(Component, Debug, Clone)]
pub struct WandererBody {
    /// What kind of wanderer this is.
    pub kind: WandererKind,
    /// Mass in arbitrary units. Above the winch strength, the winch snaps.
    pub mass: u32,
    /// Resource units delivered on a successful catch.
    pub resource_yield: u32,
    /// Orbiting or tethered.
    pub status: BodyStatus,
}

/// A harpoon tether launcher. Lives on a `BuildingType::HarpoonLauncher`
/// building entity (configured in architecture/building.rs).
#[derive(Component, Debug, Clone)]
pub struct HarpoonLauncher {
    /// True when the launcher can fire.
    pub ready: bool,
    /// Max body mass this winch can reel down without snapping.
    pub winch_strength: u32,
}

impl HarpoonLauncher {
    /// A fresh, ready launcher at default strength.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ready: true,
            winch_strength: WINCH_DEFAULT_STRENGTH,
        }
    }
}

impl Default for HarpoonLauncher {
    fn default() -> Self {
        Self::new()
    }
}

/// The tether winch reeling a snagged body down to the surface.
#[derive(Component, Debug, Clone)]
pub struct WinchSystem {
    /// The body being reeled in (None when the winch is spent).
    pub target: Option<Entity>,
    /// Reel progress: 0.0 (orbit) to 100.0 (ground).
    pub progress: f32,
}

/// Debug/console event: fire a launcher's tether at an orbiting body.
#[derive(Event, Debug, Clone)]
pub struct HarpoonLaunchEvent {
    /// Launcher entity (must be ready, with a [`GridPosition`]).
    pub launcher: Entity,
    /// Orbiting [`WandererBody`] entity.
    pub target: Entity,
}

/// A snagged body has reached the surface (or the winch snapped).
#[derive(Event, Debug, Clone)]
pub struct HarpoonImpactEvent {
    /// Where the body comes down (the launcher's tile).
    pub center: GridPosition,
    /// Resource units to credit on a clean catch (0 on a snap).
    pub yield_amount: u32,
    /// True when the winch snapped — catastrophic impact, zero yield.
    pub catastrophic: bool,
}

/// Spawn director: keeps passing wanderers in the sky on a slow timer.
#[derive(Resource, Debug, Clone)]
pub struct WandererDirector {
    /// Ticks until the next spawn attempt.
    pub ticks_until_next: u32,
}

impl Default for WandererDirector {
    fn default() -> Self {
        Self {
            ticks_until_next: WANDERER_SPAWN_TICKS_MIN,
        }
    }
}

/// System: process [`HarpoonLaunchEvent`]s — snag the target body, arm the winch.
pub fn harpoon_launch_system(world: &mut World) {
    let events: Vec<HarpoonLaunchEvent> = {
        let mut ev = world.resource_mut::<Events<HarpoonLaunchEvent>>();
        ev.drain().collect()
    };
    for event in events {
        // Validate: launcher must be ready and sited.
        let (ready, pos) = match (
            world.get::<HarpoonLauncher>(event.launcher),
            world.get::<GridPosition>(event.launcher),
        ) {
            (Some(l), Some(p)) => (l.ready, *p),
            _ => continue,
        };
        if !ready {
            continue;
        }
        // Validate: target must be an orbiting wanderer body.
        let (kind, mass) = match world.get::<WandererBody>(event.target) {
            Some(b) if b.status == BodyStatus::Orbiting => (b.kind, b.mass),
            _ => continue,
        };
        // Snag it: body tethered, launcher spent, winch armed at zero progress.
        if let Some(mut body) = world.get_mut::<WandererBody>(event.target) {
            body.status = BodyStatus::Snagged;
        }
        if let Some(mut launcher) = world.get_mut::<HarpoonLauncher>(event.launcher) {
            launcher.ready = false;
        }
        world.entity_mut(event.launcher).insert(WinchSystem {
            target: Some(event.target),
            progress: 0.0,
        });
        let label = kind.label();
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(format!(
                "HARPOON: tether away from ({}, {}) — snagged the {label} ({mass} mass). Reel-in underway.",
                pos.x, pos.y
            ));
        }
        world
            .resource_mut::<Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: format!(
                    "Harpoon away! The engineers' tether snags the passing {label} ({mass} mass) \
                     and the winch begins its long, groaning reel-in from ({}, {}).",
                    pos.x, pos.y
                ),
                importance: EventImportance::Major,
            });
    }
}

/// System: tick winches — reel snagged bodies down, or snap on overweight.
pub fn harpoon_winch_system(world: &mut World) {
    // Collect launcher state up front so the world is free for mutation.
    let launchers: Vec<(Entity, GridPosition, u32, bool)> = {
        let mut query = world.query::<(
            Entity,
            &GridPosition,
            &HarpoonLauncher,
            Option<&PowerConsumer>,
        )>();
        query
            .iter(world)
            .map(|(e, p, l, c)| (e, *p, l.winch_strength, c.is_none_or(|c| c.active)))
            .collect()
    };
    for (entity, pos, strength, powered) in launchers {
        let target = match world.get::<WinchSystem>(entity).and_then(|w| w.target) {
            Some(t) => t,
            None => continue,
        };
        let Some(body) = world.get::<WandererBody>(target) else {
            // Body vanished out from under the winch — clear the tether.
            if let Some(mut winch) = world.get_mut::<WinchSystem>(entity) {
                winch.target = None;
            }
            continue;
        };
        let (kind, mass, resource_yield) = (body.kind, body.mass, body.resource_yield);
        if mass > strength {
            // Catastrophic failure: the winch snaps, the body falls uncontrolled.
            world
                .resource_mut::<Events<HarpoonImpactEvent>>()
                .send(HarpoonImpactEvent {
                    center: pos,
                    yield_amount: 0,
                    catastrophic: true,
                });
            world.entity_mut(entity).remove::<WinchSystem>();
            world.despawn(target);
            // The snapped launcher stays broken (not ready) until rebuilt/re-armed.
            if let Some(mut launcher) = world.get_mut::<HarpoonLauncher>(entity) {
                launcher.ready = false;
            }
            let label = kind.label();
            if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
                log.add(format!(
                    "WINCH SNAP at ({}, {})! The {label} ({mass} mass) was too heavy — it is coming down uncontrolled!",
                    pos.x, pos.y
                ));
            }
            world
                .resource_mut::<Events<AddChronicleEvent>>()
                .send(AddChronicleEvent {
                    text: format!(
                        "WINCH SNAP! The {label} ({mass} mass) tore the tether free — \
                         it falls screaming toward ({}, {}). Take cover.",
                        pos.x, pos.y
                    ),
                    importance: EventImportance::Major,
                });
        } else if !powered {
            // Brownout / blackout: the winch stalls, it does not fail.
            continue;
        } else {
            // Powered reel-in.
            let progress = world
                .get::<WinchSystem>(entity)
                .map_or(0.0, |w| w.progress + WINCH_PROGRESS_PER_TICK);
            if progress >= 100.0 {
                world
                    .resource_mut::<Events<HarpoonImpactEvent>>()
                    .send(HarpoonImpactEvent {
                        center: pos,
                        yield_amount: resource_yield,
                        catastrophic: false,
                    });
                world.entity_mut(entity).remove::<WinchSystem>();
                world.despawn(target);
                if let Some(mut launcher) = world.get_mut::<HarpoonLauncher>(entity) {
                    launcher.ready = true;
                }
                if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
                    log.add(format!(
                        "HARPOON: the {label} touches down at ({}, {}) — {resource_yield} units of raw material secured.",
                        pos.x, pos.y,
                        label = kind.label()
                    ));
                }
            } else if let Some(mut winch) = world.get_mut::<WinchSystem>(entity) {
                winch.progress = progress;
            }
        }
    }
}

/// System: resolve [`HarpoonImpactEvent`]s — explosion pipeline, resources, crater.
pub fn handle_harpoon_impact_system(world: &mut World) {
    let events: Vec<HarpoonImpactEvent> = {
        let mut ev = world.resource_mut::<Events<HarpoonImpactEvent>>();
        ev.drain().collect()
    };
    for event in events {
        // Localized damage through the real explosion pipeline (shared with
        // spec 184 Orbital Debris): structures, pops, everything in radius.
        let (damage, radius) = if event.catastrophic {
            (SNAP_DAMAGE, SNAP_RADIUS)
        } else {
            (CATCH_DAMAGE, CATCH_RADIUS)
        };
        world
            .resource_mut::<Events<ExplosionEvent>>()
            .send(ExplosionEvent {
                center: event.center,
                damage,
                radius,
            });
        // The impact gouges a crater in the terrain.
        if let Some(mut grid) = world.get_resource_mut::<TerrainGrid>() {
            grid.set(
                event.center.x as usize,
                event.center.y as usize,
                TerrainType::Crater,
            );
        }
        if event.catastrophic {
            // Catastrophe: zero yield, debris fed past the Kessler line.
            if let Some(mut debris) = world.get_resource_mut::<OrbitalDebris>() {
                debris.0 += SNAP_DEBRIS_FEED;
            }
            world
                .resource_mut::<Events<AddChronicleEvent>>()
                .send(AddChronicleEvent {
                    text: format!(
                        "CATASTROPHE at ({}, {})! The snapped winch's prize struck the colony \
                         like a hammer — the sky is full of fresh debris.",
                        event.center.x, event.center.y
                    ),
                    importance: EventImportance::Major,
                });
        } else if event.yield_amount > 0 {
            // Clean catch: the yield lands in the colony's stores.
            let yield_f = event.yield_amount as f32;
            {
                let mut res = world.resource_mut::<ColonyResources>();
                res.metal += yield_f * YIELD_METAL_SHARE;
                res.ore += yield_f * YIELD_ORE_SHARE;
                res.stone += yield_f * YIELD_STONE_SHARE;
            }
            world
                .resource_mut::<Events<AddChronicleEvent>>()
                .send(AddChronicleEvent {
                    text: format!(
                        "THE SKY YIELDS ITS BOUNTY: the harpooned prize breaks apart over \
                         ({}, {}), raining {} units of metal, ore, and stone into the \
                         colony's stores.",
                        event.center.x,
                        event.center.y,
                        event.yield_amount
                    ),
                    importance: EventImportance::Major,
                });
        }
    }
}

/// System: spawn passing comets/asteroids on a slow timer when the sky is empty.
pub fn spawn_passing_wanderer_system(world: &mut World) {
    {
        let mut director = match world.get_resource_mut::<WandererDirector>() {
            Some(d) => d,
            None => return,
        };
        if director.ticks_until_next > 0 {
            director.ticks_until_next -= 1;
            return;
        }
        director.ticks_until_next =
            rand::thread_rng().gen_range(WANDERER_SPAWN_TICKS_MIN..=WANDERER_SPAWN_TICKS_MAX);
    }
    // One wanderer at a time — the sky is not a shooting gallery.
    let occupied = world
        .query::<&WandererBody>()
        .iter(world)
        .any(|b| b.status == BodyStatus::Orbiting);
    if occupied {
        return;
    }
    let mut rng = rand::thread_rng();
    let kind = if rng.gen_bool(0.6) {
        WandererKind::Comet
    } else {
        WandererKind::Asteroid
    };
    let (mass, resource_yield) = match kind {
        WandererKind::Comet => (rng.gen_range(400..=1200), rng.gen_range(800..=1500)),
        // Asteroids can run heavy — an overweight snag snaps the winch.
        WandererKind::Asteroid => (rng.gen_range(1500..=6000), rng.gen_range(1500..=3000)),
    };
    world.spawn(WandererBody {
        kind,
        mass,
        resource_yield,
        status: BodyStatus::Orbiting,
    });
    if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
        log.add(format!(
            "Orbital spotters report a passing {} ({} mass) — a harpoon launcher could snag it.",
            kind.label(),
            mass
        ));
    }
    world
        .resource_mut::<Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "The spotters' bells ring: a {} drifts through the system's edge ({} mass). \
                 The harpoon crews are already calculating trajectories.",
                kind.label(),
                mass
            ),
            importance: EventImportance::Minor,
        });
}

/// One-line status for the headless STATS line.
#[must_use]
pub fn harpoon_status_label(world: &mut World) -> String {
    let ready = {
        let mut query = world.query::<&HarpoonLauncher>();
        query.iter(world).filter(|l| l.ready).count()
    };
    format!(
        "launchers={} ready={} winching={} orbiting={}",
        launcher_count(world),
        ready,
        active_winch_count(world),
        orbiting_wanderer_count(world)
    )
}

/// Count of harpoon launcher buildings.
#[must_use]
pub fn launcher_count(world: &mut World) -> usize {
    let mut query = world.query::<&HarpoonLauncher>();
    query.iter(world).count()
}

/// Count of bodies currently being winched down.
#[must_use]
pub fn active_winch_count(world: &mut World) -> usize {
    let mut query = world.query::<&WinchSystem>();
    query.iter(world).filter(|w| w.target.is_some()).count()
}

/// Count of orbiting (snaggable) wanderer bodies.
#[must_use]
pub fn orbiting_wanderer_count(world: &mut World) -> usize {
    let mut query = world.query::<&WandererBody>();
    query
        .iter(world)
        .filter(|b| b.status == BodyStatus::Orbiting)
        .count()
}
/// Static public strings (chronicle fragments, labels) for the IP-guard test.
#[must_use]
pub fn harpoon_public_strings() -> Vec<&'static str> {
    vec![
        "comet",
        "asteroid",
        "harpoon",
        "winch",
        "tether",
        "The sky yields its bounty.",
        "WINCH SNAP",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<HarpoonLaunchEvent>>();
        world.init_resource::<Events<HarpoonImpactEvent>>();
        world.init_resource::<Events<ExplosionEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<ColonyResources>();
        world
    }

    fn spawn_body(world: &mut World, mass: u32, resource_yield: u32, status: BodyStatus) -> Entity {
        world
            .spawn(WandererBody {
                kind: WandererKind::Comet,
                mass,
                resource_yield,
                status,
            })
            .id()
    }

    fn spawn_launcher(world: &mut World, powered: bool) -> Entity {
        world
            .spawn((
                GridPosition { x: 10, y: 10 },
                HarpoonLauncher::new(),
                PowerConsumer {
                    demand: WINCH_POWER_DRAW,
                    active: powered,
                },
            ))
            .id()
    }

    #[test]
    fn test_harpoon_launch_snags_target() {
        // Arrange
        let mut world = test_world();
        let comet = spawn_body(&mut world, 500, 1000, BodyStatus::Orbiting);
        let launcher = spawn_launcher(&mut world, true);
        world
            .resource_mut::<Events<HarpoonLaunchEvent>>()
            .send(HarpoonLaunchEvent {
                launcher,
                target: comet,
            });

        // Act
        world.run_system_once(harpoon_launch_system).unwrap();

        // Assert
        let body = world.get::<WandererBody>(comet).unwrap();
        assert_eq!(body.status, BodyStatus::Snagged, "Comet should be snagged");

        let launcher_state = world.get::<HarpoonLauncher>(launcher).unwrap();
        assert!(!launcher_state.ready, "Launcher should not be ready after firing");

        let winch = world.get::<WinchSystem>(launcher).unwrap();
        assert_eq!(winch.target, Some(comet), "Winch should track the body");
        assert_eq!(winch.progress, 0.0, "Winch starts at zero progress");
    }

    #[test]
    fn test_snagged_body_impacts_surface() {
        // Arrange
        let mut world = test_world();
        let comet = spawn_body(&mut world, 500, 1000, BodyStatus::Snagged);
        let launcher = world
            .spawn((
                GridPosition { x: 10, y: 10 },
                HarpoonLauncher {
                    ready: false,
                    winch_strength: 3000,
                },
                PowerConsumer {
                    demand: WINCH_POWER_DRAW,
                    active: true,
                },
                WinchSystem {
                    target: Some(comet),
                    progress: 99.0, // Almost landed
                },
            ))
            .id();

        // Act
        world.run_system_once(harpoon_winch_system).unwrap();

        // Assert
        let events = world.resource::<Events<HarpoonImpactEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_cursor();
        let mut impact_occurred = false;
        for ev in reader.read(events) {
            if ev.center.x == 10 && ev.center.y == 10 {
                impact_occurred = true;
                assert_eq!(ev.yield_amount, 1000, "Impact should deliver the resource yield");
                assert!(!ev.catastrophic, "A safe arrival is not catastrophic");
            }
        }
        assert!(impact_occurred, "Body reaching 100 progress should impact");
        assert!(
            world.get::<WandererBody>(comet).is_none(),
            "Body is consumed on landing"
        );
        // Winch is spent and the launcher re-arms for the next shot.
        assert!(world.get::<WinchSystem>(launcher).is_none());
        assert!(world.get::<HarpoonLauncher>(launcher).unwrap().ready);
    }

    #[test]
    fn test_winch_failure_causes_catastrophe() {
        // Arrange
        let mut world = test_world();
        let massive_comet = spawn_body(&mut world, 5000, 1000, BodyStatus::Snagged); // Too heavy
        let launcher = world
            .spawn((
                GridPosition { x: 10, y: 10 },
                HarpoonLauncher {
                    ready: false,
                    winch_strength: 1000,
                },
                PowerConsumer {
                    demand: WINCH_POWER_DRAW,
                    active: true,
                },
                WinchSystem {
                    target: Some(massive_comet),
                    progress: 50.0,
                },
            ))
            .id();

        // Act
        world.run_system_once(harpoon_winch_system).unwrap();

        // Assert
        let events = world.resource::<Events<HarpoonImpactEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_cursor();
        let mut catastrophic_impact = false;
        for ev in reader.read(events) {
            if ev.center.x == 10 && ev.center.y == 10 {
                catastrophic_impact = true;
                assert!(ev.catastrophic, "Overweight body snaps the winch");
                assert_eq!(ev.yield_amount, 0, "Winch failure destroys the resource yield");
            }
        }
        assert!(catastrophic_impact, "Overweight body must cause a catastrophic impact");
        // Winch target is cleared on failure; the snapped launcher stays broken.
        assert!(
            world.get::<WinchSystem>(launcher).is_none()
                || world.get::<WinchSystem>(launcher).unwrap().target.is_none()
        );
        assert!(!world.get::<HarpoonLauncher>(launcher).unwrap().ready);
    }

    #[test]
    fn test_winch_stalls_without_power() {
        // Arrange
        let mut world = test_world();
        let comet = spawn_body(&mut world, 500, 1000, BodyStatus::Snagged);
        let launcher = world
            .spawn((
                GridPosition { x: 10, y: 10 },
                HarpoonLauncher::new(),
                PowerConsumer {
                    demand: WINCH_POWER_DRAW,
                    active: false, // brownout / blackout
                },
                WinchSystem {
                    target: Some(comet),
                    progress: 20.0,
                },
            ))
            .id();

        // Act
        world.run_system_once(harpoon_winch_system).unwrap();

        // Assert
        let winch = world.get::<WinchSystem>(launcher).unwrap();
        assert_eq!(winch.progress, 20.0, "Unpowered winch stalls, it does not fail");
        assert!(world.resource::<Events<HarpoonImpactEvent>>().is_empty());
    }

    #[test]
    fn test_winch_reels_progress_when_powered() {
        // Arrange
        let mut world = test_world();
        let comet = spawn_body(&mut world, 500, 1000, BodyStatus::Snagged);
        let launcher = spawn_launcher(&mut world, true);
        world.entity_mut(launcher).insert(WinchSystem {
            target: Some(comet),
            progress: 10.0,
        });

        // Act
        world.run_system_once(harpoon_winch_system).unwrap();

        // Assert
        let winch = world.get::<WinchSystem>(launcher).unwrap();
        assert!(
            (winch.progress - (10.0 + WINCH_PROGRESS_PER_TICK)).abs() < f32::EPSILON,
            "Powered winch reels in at the spec rate"
        );
    }

    #[test]
    fn test_impact_handler_credits_resources_and_craters() {
        // Arrange
        let mut world = test_world();
        world.insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });
        world
            .resource_mut::<Events<HarpoonImpactEvent>>()
            .send(HarpoonImpactEvent {
                center: GridPosition { x: 10, y: 10 },
                yield_amount: 1000,
                catastrophic: false,
            });

        // Arrange: baseline the default (non-zero) resource pool.
        let baseline = {
            let res = world.resource::<ColonyResources>();
            res.metal + res.ore + res.stone
        };

        // Act
        world.run_system_once(handle_harpoon_impact_system).unwrap();

        // Assert
        let res = world.resource::<ColonyResources>();
        let credited = res.metal + res.ore + res.stone - baseline;
        assert!(
            (credited - 1000.0).abs() < 0.01,
            "Full yield credited across metal/ore/stone, got {credited}"
        );
        let explosions = world.resource::<Events<ExplosionEvent>>();
        #[allow(deprecated)]
        let mut reader = explosions.get_cursor();
        let mut saw = false;
        for ev in reader.read(explosions) {
            if ev.center.x == 10 && ev.center.y == 10 {
                saw = true;
                assert_eq!(ev.radius, CATCH_RADIUS, "Catch is a small localized impact");
                assert_eq!(ev.damage, CATCH_DAMAGE);
            }
        }
        assert!(saw, "Catch fires through the real explosion pipeline");
        let terrain = world.resource::<TerrainGrid>();
        assert_eq!(terrain.get(10, 10).unwrap(), TerrainType::Crater);
    }

    #[test]
    fn test_snap_impact_feeds_debris_and_yields_nothing() {
        // Arrange
        let mut world = test_world();
        world.insert_resource(OrbitalDebris(0.0));
        world
            .resource_mut::<Events<HarpoonImpactEvent>>()
            .send(HarpoonImpactEvent {
                center: GridPosition { x: 5, y: 5 },
                yield_amount: 0,
                catastrophic: true,
            });

        // Act
        world.run_system_once(handle_harpoon_impact_system).unwrap();

        // Assert
        let res = world.resource::<ColonyResources>();
        let baseline = ColonyResources::default();
        let delta = (res.metal - baseline.metal)
            + (res.ore - baseline.ore)
            + (res.stone - baseline.stone);
        assert_eq!(delta, 0.0, "Snapped catch yields nothing");
        assert_eq!(
            world.resource::<OrbitalDebris>().0,
            SNAP_DEBRIS_FEED,
            "Catastrophe feeds orbital debris past the Kessler line"
        );
        let explosions = world.resource::<Events<ExplosionEvent>>();
        #[allow(deprecated)]
        let mut reader = explosions.get_cursor();
        let mut saw = false;
        for ev in reader.read(explosions) {
            if ev.center.x == 5 && ev.center.y == 5 {
                saw = true;
                assert_eq!(ev.radius, SNAP_RADIUS, "Snap is a damaging impact radius");
            }
        }
        assert!(saw, "Snap fires through the real explosion pipeline");
    }

    #[test]
    fn test_launch_requires_ready_launcher_and_orbiting_body() {
        // Arrange
        let mut world = test_world();
        let comet = spawn_body(&mut world, 500, 1000, BodyStatus::Snagged); // already snagged
        let launcher = spawn_launcher(&mut world, true);
        world.entity_mut(launcher).get_mut::<HarpoonLauncher>().unwrap().ready = false;
        world
            .resource_mut::<Events<HarpoonLaunchEvent>>()
            .send(HarpoonLaunchEvent {
                launcher,
                target: comet,
            });

        // Act
        world.run_system_once(harpoon_launch_system).unwrap();

        // Assert
        assert!(
            world.get::<WinchSystem>(launcher).is_none(),
            "Not-ready launcher and non-orbiting body must not fire"
        );
        assert_eq!(
            world.get::<WandererBody>(comet).unwrap().status,
            BodyStatus::Snagged,
            "Body status untouched"
        );
    }

    #[test]
    fn test_spawn_director_spawns_when_sky_empty() {
        // Arrange
        let mut world = test_world();
        world.insert_resource(WandererDirector { ticks_until_next: 0 });

        // Act
        world.run_system_once(spawn_passing_wanderer_system).unwrap();

        // Assert
        let mut bodies = world.query::<&WandererBody>();
        assert_eq!(bodies.iter(&world).count(), 1, "Director spawns a wanderer");
        let director = world.resource::<WandererDirector>();
        assert!(
            director.ticks_until_next >= WANDERER_SPAWN_TICKS_MIN
                && director.ticks_until_next <= WANDERER_SPAWN_TICKS_MAX,
            "Director resets its countdown"
        );
    }

    #[test]
    fn ip_guard_no_banned_terms() {
        // Concept-only rule: no named characters, places, or distinctive IP.
        // Harpoons are generic tech; the banned list is the shared standing one.
        let banned = [
            "shrike", "hyperion", "ouster",
            "hitchhiker", "vogon", "zaphod", "ford prefect", "marvin",
            "dont panic", "don't panic", "babel fish", "infinite improbability",
            "asimov", "psychohistory", "hari seldon", "foundation", "spacer",
            "annihilation", "area x", "southern reach", "vandermeer",
            "sad king billy", "windsor", "windsor-in-exile",
        ];
        for s in harpoon_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
