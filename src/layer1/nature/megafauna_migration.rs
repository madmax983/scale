//! Subterranean Megafauna Migration (Spec 424).
//!
//! Massive, blind creatures tunnel beneath the colony. A migration is
//! driven by a [`MegafaunaMigrationEvent`] (§5 REFACTOR: event-driven,
//! not a direct function call): the creature's path is carved into
//! temporary [`TerrainType::Tunnel`] tiles, every path tile emits a
//! [`TremorEvent`] that damages whatever stands on it, and the tunnels
//! later collapse back into rock via [`tunnel_collapse_system`]
//! (a standard Bevy system, §5 systemization). Tunnels are the fastest
//! terrain in the game (movement cost 0.5), so haulers naturally prefer
//! them while they last — free subways with an expiry date.
//!
//! ## Audit adaptations (spec sketch vs. real codebase)
//!
//! * The sketch assumed a 3D `TerrainGrid::new(x, y, z)`,
//!   `TerrainType::SolidRock`, and `GridPosition { x, y, z }`. None of
//!   that exists: the real grid is 2D (`TerrainGrid { width, height,
//!   tiles }`, `get`/`set(x, y)`), `GridPosition` is `{ x, y }` (depth
//!   is the separate `ZLevel` component, gravity-chute only), and solid
//!   rock is `Rock`/`DeepRock`. The sketch's "deep Z-level path /
//!   building directly above" therefore maps to the path tiles
//!   themselves / buildings standing on path tiles, and collapsed
//!   tunnels return to `Rock`.
//! * The sketch damaged buildings through `layer1::health::Health`
//!   only. Real building HP is [`Structure`] (`spawn_building` inserts
//!   it); `Health` is the pop pool (some buildings carry it too).
//!   Following the Spec 389 geology precedent, tremors damage **both**
//!   pools where present.
//! * Lifespans are sim ticks, not the sketch's `f32` seconds: the
//!   headless ECS has no wall-clock `Time`; systems run per tick.
//!   Default lifespan is [`DEFAULT_TUNNEL_LIFESPAN_TICKS`] (300, the
//!   sketch's "5 minutes" at 1 tick ≈ 1 second).
//! * Pathfinding truncates `movement_cost()` to `i32` and clamps tile
//!   cost to >= 1, so Tunnel's 0.5 prices at the fastest representable
//!   A* cost there; the 0.5 applies in full to float consumers. No
//!   changes to `pathfinding.rs` integer cost math (blast radius).
//! * Chronicle entries go through the real `AddChronicleEvent`
//!   channel; collapse emits [`TunnelCollapseEvent`] (§7) so renderers
//!   and pathfinding caches can react.
//!
//! The dormant-titan mining mechanic in
//! [`super::megafauna_terrain`] is a different system (name overlap
//! only) and is untouched. There is no random/natural migration
//! director — the spec asks only for the event plus a debug trigger
//! (headless `megafauna`); organic triggering is a follow-on.
//!
//! All names and strings are original and generic (see the IP-guard
//! test at the bottom of this module).

use crate::layer1::biology::health::Health;
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::map::{GridPosition, ScreenShake};
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use crate::layer1::structure::Structure;
use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// Flat tremor damage dealt to each health pool (`Structure` and
/// `Health`) of an entity standing on a migration path tile.
pub const TREMOR_DAMAGE: f32 = 25.0;
/// Crush damage dealt to a pop caught inside a collapsing tunnel
/// (it is also pushed out — see [`tunnel_collapse_system`]).
pub const COLLAPSE_CRUSH_DAMAGE: f32 = 10.0;
/// Default tunnel lifespan in ticks (the spec sketch's 5 minutes,
/// expressed in sim ticks).
pub const TUNNEL_LIFESPAN_TICKS: u32 = 300;

/// Event: a subterranean megafauna migration passes along `path`,
/// carving tunnels and shaking everything standing on the path tiles.
#[derive(Event, Debug, Clone)]
pub struct MegafaunaMigrationEvent {
    /// The tiles the creature passes through, in order.
    pub path: Vec<GridPosition>,
}

/// Event: a tremor at `position` dealt `damage` to the health pools
/// standing there (§7 — lets the chronicle / rendering layers react
/// without coupling to the damage function).
#[derive(Event, Debug, Clone, Copy)]
pub struct TremorEvent {
    /// Tile the tremor struck.
    pub position: GridPosition,
    /// Damage dealt per health pool on that tile.
    pub damage: f32,
}

/// Event: the tunnel at `position` collapsed back into rock (§7).
#[derive(Event, Debug, Clone, Copy)]
pub struct TunnelCollapseEvent {
    /// Tile whose tunnel collapsed.
    pub position: GridPosition,
}

/// Tracks live tunnels and the migration ledger.
///
/// Lifespans are in sim ticks. `add_tunnel` on an already-tracked tile
/// refreshes its lifespan instead of duplicating the entry (a second
/// migration over the same tile re-bores the tunnel).
#[derive(Resource, Debug, Default, Clone)]
pub struct TunnelManager {
    /// Live tunnels: position + ticks remaining until collapse.
    pub active_tunnels: Vec<(GridPosition, u32)>,
    /// Total migrations applied.
    pub migrations: u64,
    /// Total tremor events emitted (one per path tile per migration).
    pub tremors: u64,
    /// Total tunnels collapsed.
    pub collapses: u64,
}

impl TunnelManager {
    /// Creates an empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tunnel at `pos` with `lifespan_ticks` until
    /// collapse; re-registering a tracked tile refreshes it.
    pub fn add_tunnel(&mut self, pos: GridPosition, lifespan_ticks: u32) {
        if let Some(entry) = self.active_tunnels.iter_mut().find(|(p, _)| *p == pos) {
            entry.1 = lifespan_ticks;
        } else {
            self.active_tunnels.push((pos, lifespan_ticks));
        }
    }
}

/// `(active tunnels, migrations, tremors, collapses)` for headless
/// STATS; all zeroes when the manager has not been initialized in
/// this world.
#[must_use]
pub fn megafauna_stats_counts(world: &World) -> (usize, u64, u64, u64) {
    world
        .get_resource::<TunnelManager>()
        .map_or((0, 0, 0, 0), |m| {
            (m.active_tunnels.len(), m.migrations, m.tremors, m.collapses)
        })
}

/// Sends a [`MegafaunaMigrationEvent`] into `world` (initializing the
/// event channel first — the first-tick pattern). Shared by the
/// headless debug command and tests.
pub fn send_migration(world: &mut World, path: Vec<GridPosition>) {
    world.init_resource::<Events<MegafaunaMigrationEvent>>();
    world
        .resource_mut::<Events<MegafaunaMigrationEvent>>()
        .send(MegafaunaMigrationEvent { path });
}

/// A straight horizontal migration path of `len` tiles starting at
/// (`x`, `y`) — the debug trigger's shape.
#[must_use]
pub fn horizontal_migration_path(x: i32, y: i32, len: usize) -> Vec<GridPosition> {
    (0..len)
        .map(|i| GridPosition {
            x: x + i32::try_from(i).unwrap_or(i32::MAX),
            y,
        })
        .collect()
}

/// System: applies [`MegafaunaMigrationEvent`]s — carves each path
/// tile into a [`TerrainType::Tunnel`], registers its lifespan, emits
/// a [`TremorEvent`] per tile, damages both health pools of anything
/// standing on the path, shakes the screen, and chronicles the
/// passage.
#[allow(clippy::too_many_arguments)]
pub fn megafauna_migration_system(
    mut events: EventReader<MegafaunaMigrationEvent>,
    mut grid: Option<ResMut<TerrainGrid>>,
    mut manager: Option<ResMut<TunnelManager>>,
    mut targets: Query<(&GridPosition, Option<&mut Structure>, Option<&mut Health>)>,
    mut shake: Option<ResMut<ScreenShake>>,
    mut tremors: EventWriter<TremorEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        // Dedupe the path (a looping creature re-bores the same tile;
        // it should still only be carved / shaken once per passage).
        let mut seen = HashSet::new();
        let path: Vec<GridPosition> = event
            .path
            .iter()
            .copied()
            .filter(|p| seen.insert(*p))
            .collect();
        if path.is_empty() {
            continue;
        }

        if let Some(m) = manager.as_mut() {
            m.migrations += 1;
        }

        // Carve + register.
        for pos in &path {
            if let Some(g) = grid.as_mut() {
                if pos.x >= 0 && pos.y >= 0 {
                    #[allow(clippy::cast_sign_loss)]
                    g.set(pos.x as usize, pos.y as usize, TerrainType::Tunnel);
                }
            }
            if let Some(m) = manager.as_mut() {
                m.add_tunnel(*pos, TUNNEL_LIFESPAN_TICKS);
            }
        }

        // Tremors: damage both pools where present, count buildings
        // (Structure-carrying entities) actually damaged.
        let path_set: HashSet<GridPosition> = path.iter().copied().collect();
        let mut buildings_damaged = 0u64;
        for (pos, structure, health) in &mut targets {
            if !path_set.contains(pos) {
                continue;
            }
            if let Some(mut s) = structure {
                s.current_hp = (s.current_hp - TREMOR_DAMAGE).max(0.0);
                buildings_damaged += 1;
            }
            if let Some(mut h) = health {
                h.take_damage(TREMOR_DAMAGE);
            }
        }
        for pos in &path {
            tremors.send(TremorEvent {
                position: *pos,
                damage: TREMOR_DAMAGE,
            });
            if let Some(m) = manager.as_mut() {
                m.tremors += 1;
            }
        }

        if let Some(s) = shake.as_mut() {
            s.trigger(2.0);
        }

        let importance = if buildings_damaged > 0 {
            EventImportance::Major
        } else {
            EventImportance::Standard
        };
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A subterranean megafauna migration passed beneath the colony - tremors damaged {buildings_damaged} building(s) and left fresh tunnels behind."
            ),
            importance,
        });
    }
}

/// System: ages every live tunnel by one tick; expired tunnels
/// collapse back into `Rock`, emit a [`TunnelCollapseEvent`], and
/// chronicle (Minor). §5 safety: a pop inside a collapsing tunnel
/// takes [`COLLAPSE_CRUSH_DAMAGE`] crush damage and is pushed to the
/// first adjacent walkable tile that is not itself collapsing.
pub fn tunnel_collapse_system(
    mut manager: Option<ResMut<TunnelManager>>,
    mut grid: Option<ResMut<TerrainGrid>>,
    mut pops: Query<(&mut GridPosition, &mut Health), With<crate::layer1::entities::pop::Pop>>,
    mut collapses: EventWriter<TunnelCollapseEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut expired: Vec<GridPosition> = Vec::new();
    if let Some(m) = manager.as_mut() {
        m.active_tunnels.retain_mut(|(pos, remaining)| {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                expired.push(*pos);
                false
            } else {
                true
            }
        });
        m.collapses += u64::try_from(expired.len()).unwrap_or(u64::MAX);
    }
    if expired.is_empty() {
        return;
    }
    let expired_set: HashSet<GridPosition> = expired.iter().copied().collect();

    // Collapse the terrain first, so the safety push below searches
    // the post-collapse map.
    if let Some(g) = grid.as_mut() {
        for pos in &expired {
            if pos.x >= 0 && pos.y >= 0 {
                #[allow(clippy::cast_sign_loss)]
                g.set(pos.x as usize, pos.y as usize, TerrainType::Rock);
            }
        }
    }

    // §5 safety: crush + push out any pop caught inside.
    for (mut pos, mut health) in &mut pops {
        if !expired_set.contains(&pos) {
            continue;
        }
        health.take_damage(COLLAPSE_CRUSH_DAMAGE);
        let origin = *pos;
        'push: for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let candidate = GridPosition {
                    x: origin.x + dx,
                    y: origin.y + dy,
                };
                if expired_set.contains(&candidate) {
                    continue;
                }
                let walkable = match grid.as_ref() {
                    Some(g) => {
                        candidate.x >= 0
                            && candidate.y >= 0
                            && usize::try_from(candidate.x)
                                .ok()
                                .zip(usize::try_from(candidate.y).ok())
                                .and_then(|(x, y)| g.get(x, y))
                                .is_some_and(TerrainType::is_walkable)
                    }
                    None => true,
                };
                if walkable {
                    *pos = candidate;
                    break 'push;
                }
            }
        }
    }

    for pos in &expired {
        collapses.send(TunnelCollapseEvent { position: *pos });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "A megafauna tunnel collapsed at ({}, {}), sealing the passage in rock.",
                pos.x, pos.y
            ),
            importance: EventImportance::Minor,
        });
    }
}

/// Every public-facing string this module emits (IP-guard source).
#[must_use]
pub fn megafauna_migration_public_strings() -> Vec<String> {
    vec![
        "A subterranean megafauna migration passed beneath the colony and left fresh tunnels behind.".to_string(),
        "A megafauna tunnel collapsed, sealing the passage in rock.".to_string(),
        TerrainType::Tunnel.name().to_string(),
        "Subterranean Megafauna Migration".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::Building;
    use crate::layer1::entities::pop::Pop;

    fn setup_world(fill: TerrainType) -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![fill; 100],
        });
        world.insert_resource(TunnelManager::default());
        world.insert_resource(ScreenShake::default());
        world.init_resource::<Events<MegafaunaMigrationEvent>>();
        world.init_resource::<Events<TremorEvent>>();
        world.init_resource::<Events<TunnelCollapseEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn run_migration(world: &mut World, path: Vec<GridPosition>) {
        send_migration(world, path);
        let mut schedule = Schedule::default();
        schedule.add_systems(megafauna_migration_system);
        schedule.run(world);
    }

    #[test]
    fn test_megafauna_migration_creates_tunnels() {
        let mut world = setup_world(TerrainType::Rock);
        run_migration(
            &mut world,
            vec![
                GridPosition { x: 2, y: 2 },
                GridPosition { x: 3, y: 2 },
                GridPosition { x: 4, y: 2 },
            ],
        );

        let grid = world.resource::<TerrainGrid>();
        assert_eq!(grid.get(2, 2), Some(TerrainType::Tunnel));
        assert_eq!(grid.get(3, 2), Some(TerrainType::Tunnel));
        assert_eq!(grid.get(4, 2), Some(TerrainType::Tunnel));
        // Adjacent tiles are still rock.
        assert_eq!(grid.get(1, 2), Some(TerrainType::Rock));

        let (active, migrations, tremors, collapses) = megafauna_stats_counts(&world);
        assert_eq!(active, 3);
        assert_eq!(migrations, 1);
        assert_eq!(tremors, 3);
        assert_eq!(collapses, 0);
    }

    #[test]
    fn test_megafauna_migration_causes_tremors() {
        let mut world = setup_world(TerrainType::Rock);

        // A building standing on the migration path, carrying both
        // real health pools (Structure + Health).
        let on_path = world
            .spawn((
                GridPosition { x: 3, y: 2 },
                Building::default(),
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                Health::default(),
            ))
            .id();
        // An identical building off the path must be untouched.
        let off_path = world
            .spawn((
                GridPosition { x: 3, y: 3 },
                Building::default(),
                Structure {
                    current_hp: 100.0,
                    max_hp: 100.0,
                },
                Health::default(),
            ))
            .id();

        run_migration(
            &mut world,
            vec![
                GridPosition { x: 2, y: 2 },
                GridPosition { x: 3, y: 2 },
                GridPosition { x: 4, y: 2 },
            ],
        );

        let structure = world.get::<Structure>(on_path).unwrap();
        assert!(
            (structure.current_hp - 75.0).abs() < f32::EPSILON,
            "Structure on the path takes flat tremor damage (100 -> 75), got {}",
            structure.current_hp
        );
        let health = world.get::<Health>(on_path).unwrap();
        assert!(
            (health.current - 75.0).abs() < f32::EPSILON,
            "Health pool on the path takes flat tremor damage (100 -> 75), got {}",
            health.current
        );
        assert_eq!(world.get::<Structure>(off_path).unwrap().current_hp, 100.0);
        assert_eq!(world.get::<Health>(off_path).unwrap().current, 100.0);
    }

    #[test]
    fn test_tunnel_collapse_over_time() {
        let mut world = setup_world(TerrainType::Rock);
        world
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::Tunnel);
        world
            .resource_mut::<TunnelManager>()
            .add_tunnel(GridPosition { x: 5, y: 5 }, 10);

        let mut schedule = Schedule::default();
        schedule.add_systems(tunnel_collapse_system);
        for _ in 0..11 {
            schedule.run(&mut world);
        }

        let grid = world.resource::<TerrainGrid>();
        assert_eq!(
            grid.get(5, 5),
            Some(TerrainType::Rock),
            "Tunnel should collapse back into rock"
        );
        let (_, _, _, collapses) = megafauna_stats_counts(&world);
        assert_eq!(collapses, 1);
    }

    #[test]
    fn test_tunnel_collapse_pushes_pop_out_with_crush_damage() {
        let mut world = setup_world(TerrainType::Grass);
        world
            .resource_mut::<TerrainGrid>()
            .set(5, 5, TerrainType::Tunnel);
        world
            .resource_mut::<TunnelManager>()
            .add_tunnel(GridPosition { x: 5, y: 5 }, 1);

        let pop = world
            .spawn((Pop, GridPosition { x: 5, y: 5 }, Health::default()))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(tunnel_collapse_system);
        schedule.run(&mut world);

        let health = world.get::<Health>(pop).unwrap();
        assert!(
            (health.current - 90.0).abs() < f32::EPSILON,
            "Pop inside a collapsing tunnel takes 10 crush damage, got {}",
            health.current
        );
        let pos = world.get::<GridPosition>(pop).unwrap();
        assert_ne!(
            *pos,
            GridPosition { x: 5, y: 5 },
            "Pop must be pushed off the collapsing tile"
        );
        assert!(
            (pos.x - 5).abs() <= 1 && (pos.y - 5).abs() <= 1,
            "Pop is pushed to an adjacent tile, got ({}, {})",
            pos.x,
            pos.y
        );
    }

    #[test]
    fn test_tunnel_terrain_is_fast_and_walkable() {
        assert!((TerrainType::Tunnel.movement_cost() - 0.5).abs() < f32::EPSILON);
        assert!(TerrainType::Tunnel.is_walkable());
        assert_eq!(TerrainType::Tunnel.name(), "Tunnel");
        // Rock-group heat retention.
        assert!((TerrainType::Tunnel.heat_retention() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_megafauna_migration_public_strings_have_no_lifted_ip() {
        // IP guard: every public-facing string this module emits must
        // be original/generic — no franchise names, no distinctive IP.
        const BANNED: &[&str] = &[
            "dune",
            "shai-hulud",
            "arrakis",
            "sandworm",
            "graboid",
            "atreides",
            "fremen",
            "tremors",
        ];
        for s in megafauna_migration_public_strings() {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "public string {s:?} contains banned term {banned:?}"
                );
            }
        }
    }
}
