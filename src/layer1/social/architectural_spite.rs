//! Spec 363: Architectural Spite.
//!
//! Pops express social grievances through petty construction: a pop with
//! a deeply negative [`Relationships`] affinity toward a nearby rival
//! raises a **spite wall** — a junk-built [`BuildingType::Wall`] segment
//! on a tile adjacent to the rival's home (or, for the homeless, adjacent
//! to where the rival stands) — and may jam a shared amenity against that
//! one rival.
//!
//! ## Adaptations from the mock-style spec (established §8 pattern)
//!
//! * **No new `ActionType` (spec §7).** The real utility AI menu is a
//!   closed, index-counted enum (`ActionType::COUNT` / `as_index`), and
//!   the spec's own predecessor — Spec 110's
//!   `architecture::spontaneous_architecture::check_spontaneous_build_system`
//!   — is *not* utility-AI driven at all: it is a standalone exclusive
//!   system that scans housing each pass. Spite follows its actual
//!   predecessor: [`check_spite_build_system`] is a standalone exclusive
//!   system, and the spec's scoring intent lives in the pure, unit-tested
//!   [`evaluate_build_spite`] function (deeply negative affinity + rival
//!   nearby scores high; anything else scores zero).
//! * **Obstacle via the real building pipeline (spec §5).** A spite wall
//!   is a genuine `Building { building_type: BuildingType::Wall }` entity
//!   registered in both `BuildingMap` and `OccupiedTiles`. `Wall` is an
//!   obstacle type, so `pathfinding::is_walkable` refuses it and A*
//!   routes around it with no pathfinding changes at all.
//! * **No colony resources (spec §7).** The wall is spawned directly
//!   (junk and personal spite), never through `try_place_building`, so no
//!   stockpile, material, or tech gate is consulted or charged.
//! * **Never seal a crucial path (spec §7).** Before a wall is
//!   finalized, [`valid_path_after`] tentatively occupies the candidate
//!   tile and proves with the real A* that (a) the rival's home still
//!   has a reachable entrance tile from the builder, and (b) when a
//!   Command Center exists, it too still has a reachable entrance tile.
//!   A candidate that would seal either is refused.
//! * **Builder stored on the component (spec §5).** [`SpiteWall`]
//!   carries `builder` and `target`; [`demolish_spite_wall`] reads the
//!   builder back off the component and applies the "My Work Destroyed"
//!   mood modifier (-0.30, 2000 ticks — the heritage-sold scale from the
//!   Artifact Market) to that pop, whoever swung the hammer.
//! * **Amenity sabotage stress (spec RED 4).** [`SpiteSabotage`] sits on
//!   the amenity building, naming one target pop. The game's needs have
//!   no separate stress field — leisure *is* the stress-adjacent need
//!   (its own docs: "< 0.2: Bored/Stressed") — so a blocked attempt
//!   costs the target 0.10 leisure plus a minor "Spite-Blocked Amenity"
//!   mood modifier (-0.05, 300 ticks). Everyone else uses the amenity
//!   normally.

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::building::{Building, BuildingMap, BuildingType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::health::Dead;
use crate::layer1::housing::Housing;
use crate::layer1::map::GridPosition;
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::pathfinding::find_path;
use crate::layer1::social::Relationships;
use crate::layer1::terrain::{TerrainGrid, TerrainType};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Affinity at or below which a pop counts as a spite-worthy rival.
/// The Relationships scale calls <-20 Rival and <-80 Nemesis; spite
/// construction is reserved for the deep end of that range.
pub const RIVAL_AFFINITY_THRESHOLD: f32 = -50.0;
/// Chebyshev radius (tiles) within which a rival counts as "nearby".
pub const SPITE_NEARBY_RADIUS: i32 = 12;
/// Passes a builder must wait between spite constructions.
pub const SPITE_COOLDOWN_PASSES: u64 = 200;
/// Maximum standing spite walls per builder.
pub const MAX_WALLS_PER_BUILDER: usize = 3;
/// Per-pass build probability once the score gate passes (the
/// spontaneous-architecture system's 0.01 precedent, doubled: spite is
/// motivated).
pub const SPITE_BUILD_CHANCE: f64 = 0.02;
/// Per-pass amenity-sabotage probability for a builder who already has
/// a standing wall against the rival.
pub const SPITE_SABOTAGE_CHANCE: f64 = 0.01;
/// Mood value of the "My Work Destroyed" modifier when a spite wall
/// built by the pop is torn down.
pub const DEMOLISH_MOOD_VALUE: f32 = -0.30;
/// Duration (ticks) of the "My Work Destroyed" modifier.
pub const DEMOLISH_MOOD_TICKS: u32 = 2000;
/// Leisure (stress-proxy) cost of being spite-blocked from an amenity.
pub const AMENITY_STRESS_LEISURE: f32 = 0.10;
/// Mood value of the minor "Spite-Blocked Amenity" modifier.
pub const AMENITY_MOOD_VALUE: f32 = -0.05;
/// Duration (ticks) of the "Spite-Blocked Amenity" modifier.
pub const AMENITY_MOOD_TICKS: u32 = 300;
/// Lifetime (ticks) of one amenity sabotage before it quietly lapses.
pub const SABOTAGE_DURATION: u32 = 1000;

// ---------------------------------------------------------------------------
// Components, resource, events of state
// ---------------------------------------------------------------------------

/// A junk wall raised by one pop to inconvenience one rival (Spec 363).
///
/// Spawned alongside `Building { building_type: BuildingType::Wall }`
/// and a [`GridPosition`], and registered in `BuildingMap` /
/// `OccupiedTiles`, so the existing pathfinding treats it as an
/// ordinary obstacle.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiteWall {
    /// The pop who built the wall. Read back on demolition so the
    /// "My Work Destroyed" mood lands on the right pop (spec §5).
    pub builder: Entity,
    /// The rival the wall was built against.
    pub target: Entity,
}

/// An amenity (building) jammed against one specific pop (Spec 363).
///
/// Attached to the amenity's building entity. Only `target` is denied;
/// everyone else uses the amenity normally.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiteSabotage {
    /// The pop who sabotaged the amenity.
    pub saboteur: Entity,
    /// The rival who is denied access.
    pub target: Entity,
    /// Ticks until the sabotage lapses.
    pub remaining: u32,
}

/// Ledger and pacing state for architectural spite (Spec 363).
#[derive(Resource, Default, Debug)]
pub struct SpiteState {
    /// Total spite walls ever built.
    pub walls_built: u32,
    /// Total amenity sabotages ever committed.
    pub sabotages_done: u32,
    /// Total spite walls torn down.
    pub demolitions: u32,
    /// Passes run by [`check_spite_build_system`] (the cooldown clock).
    pub passes: u64,
    /// Last pass on which each builder raised a wall.
    pub last_build_pass: HashMap<Entity, u64>,
}

// ---------------------------------------------------------------------------
// Scoring (spec §7: the BuildSpite action's score, as a pure function)
// ---------------------------------------------------------------------------

/// Scores the urge to build spite against a rival.
///
/// Zero unless the affinity is deeply negative ([`RIVAL_AFFINITY_THRESHOLD`])
/// and the rival is nearby ([`SPITE_NEARBY_RADIUS`]); then rises with
/// hatred and falls with distance, in `0.0..=1.0`.
#[must_use]
pub fn evaluate_build_spite(affinity: f32, chebyshev_distance: i32) -> f32 {
    if affinity > RIVAL_AFFINITY_THRESHOLD || chebyshev_distance > SPITE_NEARBY_RADIUS {
        return 0.0;
    }
    let hatred = ((-affinity + RIVAL_AFFINITY_THRESHOLD) / 50.0).clamp(0.0, 1.0);
    #[allow(clippy::cast_precision_loss)]
    let proximity = 1.0 - (chebyshev_distance as f32 / (SPITE_NEARBY_RADIUS as f32 * 2.0));
    (0.4 + 0.6 * hatred) * proximity.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Placement helpers
// ---------------------------------------------------------------------------

fn chebyshev(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// The tile a rival calls home: their housing building's tile, or —
/// for a homeless rival — the tile they are standing on.
fn rival_anchor(world: &mut World, target: Entity) -> Option<(i32, i32)> {
    {
        let mut query = world.query::<(&GridPosition, &Housing)>();
        for (pos, housing) in query.iter(world) {
            if housing.residents.contains(&target) {
                return Some((pos.x, pos.y));
            }
        }
    }
    world
        .get::<GridPosition>(target)
        .map(|pos| (pos.x, pos.y))
}

/// True when `(x, y)` could host a spite wall right now: walkable
/// terrain, unoccupied, and no building registered there.
fn tile_free(world: &World, x: i32, y: i32) -> bool {
    let Ok(ux) = usize::try_from(x) else { return false };
    let Ok(uy) = usize::try_from(y) else { return false };
    let Some(terrain) = world.get_resource::<TerrainGrid>() else {
        return false;
    };
    if !terrain.get(ux, uy).is_some_and(TerrainType::is_walkable) {
        return false;
    }
    if let Some(occupied) = world.get_resource::<OccupiedTiles>() {
        if occupied.0.contains(&(x, y)) {
            return false;
        }
    }
    if let Some(map) = world.get_resource::<BuildingMap>() {
        if map.0.contains_key(&(x, y)) {
            return false;
        }
    }
    true
}

/// True when at least one entrance tile (cardinal neighbour) of
/// `anchor` is reachable from `from` under the current occupancy.
fn entrance_reachable(world: &World, from: (i32, i32), anchor: (i32, i32)) -> bool {
    for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
        let entrance = (anchor.0 + dx, anchor.1 + dy);
        if entrance == from {
            return true;
        }
        let Ok(ux) = usize::try_from(entrance.0) else { continue };
        let Ok(uy) = usize::try_from(entrance.1) else { continue };
        let Some(terrain) = world.get_resource::<TerrainGrid>() else {
            return false;
        };
        if !terrain.get(ux, uy).is_some_and(TerrainType::is_walkable) {
            continue;
        }
        if let Some(occupied) = world.get_resource::<OccupiedTiles>() {
            if occupied.0.contains(&entrance) {
                continue;
            }
        }
        if find_path(world, from, entrance).is_some() {
            return true;
        }
    }
    false
}

/// The spec §7 valid-path check, as a comparative invariant: the
/// wall may not *remove* the last reachable entrance to the rival's
/// home, nor to the Command Center (the colony's crucial path) when
/// one exists. An entrance counts only if it was reachable *before*
/// the wall — in a built-up habitat every tile beside the Command
/// Center is already occupied by other buildings, so an absolute
/// "CC must be reachable" test would vacuously refuse every wall in
/// a real colony; what must never happen is *this wall* sealing it.
#[must_use]
pub fn valid_path_after(
    world: &mut World,
    candidate: (i32, i32),
    builder_pos: (i32, i32),
    anchor: (i32, i32),
) -> bool {
    // The Command Center is the colony's crucial path (spec §7).
    let command_center = {
        let mut found = None;
        let mut query = world.query::<(&GridPosition, &Building)>();
        for (pos, building) in query.iter(world) {
            if building.building_type == BuildingType::CommandCenter {
                found = Some((pos.x, pos.y));
                break;
            }
        }
        found
    };

    let anchor_reachable_before = entrance_reachable(world, builder_pos, anchor);
    let cc_reachable_before = command_center.is_some_and(|cc| {
        cc != anchor && entrance_reachable(world, builder_pos, cc)
    });

    let inserted = if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
        occupied.0.insert(candidate)
    } else {
        false
    };

    let mut ok = !anchor_reachable_before || entrance_reachable(world, builder_pos, anchor);
    if ok && cc_reachable_before {
        if let Some(cc) = command_center {
            ok = entrance_reachable(world, builder_pos, cc);
        }
    }

    if inserted {
        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&candidate);
        }
    }
    ok
}

/// Spawns the spite wall entity at `(x, y)` and registers it in the
/// building/occupancy maps so pathfinding treats it as an obstacle.
/// Costs no colony resources (junk construction, spec §7).
pub fn place_spite_wall_at(
    world: &mut World,
    builder: Entity,
    target: Entity,
    x: i32,
    y: i32,
) -> Entity {
    let wall = world
        .spawn((
            Building {
                building_type: BuildingType::Wall,
            },
            GridPosition { x, y },
            SpiteWall { builder, target },
        ))
        .id();
    if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
        occupied.0.insert((x, y));
    }
    if let Some(mut map) = world.get_resource_mut::<BuildingMap>() {
        map.0.insert((x, y), wall);
    }
    wall
}

fn send_chronicle(world: &mut World, text: String) {
    if let Some(mut events) = world.get_resource_mut::<Events<AddChronicleEvent>>() {
        events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Minor,
        });
    }
}

// ---------------------------------------------------------------------------
// Core actions
// ---------------------------------------------------------------------------

/// Attempts to build a spite wall for `builder` against `target`.
///
/// Requires a deeply negative affinity (read from the builder's real
/// [`Relationships`]). Candidates are the tiles adjacent to the
/// rival's home/standing tile, nearest-to-builder first; the first
/// free candidate that passes the valid-path check is walled.
/// Returns the wall entity, or `None` when there is no grudge, no
/// free tile, or every candidate would seal a crucial path.
pub fn try_build_spite_wall(
    world: &mut World,
    builder: Entity,
    target: Entity,
) -> Option<Entity> {
    let affinity = world
        .get::<Relationships>(builder)?
        .get_affinity(target);
    if affinity > RIVAL_AFFINITY_THRESHOLD {
        return None;
    }
    if world.get::<Dead>(builder).is_some() || world.get::<Dead>(target).is_some() {
        return None;
    }
    let builder_pos = world.get::<GridPosition>(builder)?;
    let builder_pos = (builder_pos.x, builder_pos.y);
    let anchor = rival_anchor(world, target)?;

    // Adjacent tiles: cardinals (entrance/window sides) first, then
    // diagonals, nearest to the builder first within each ring.
    let mut candidates: Vec<(i32, i32)> = Vec::new();
    for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
        candidates.push((anchor.0 + dx, anchor.1 + dy));
    }
    let mut diagonals: Vec<(i32, i32)> = [(1, 1), (1, -1), (-1, 1), (-1, -1)]
        .iter()
        .map(|(dx, dy)| (anchor.0 + dx, anchor.1 + dy))
        .collect();
    diagonals.sort_by_key(|pos| chebyshev(*pos, builder_pos));
    candidates.sort_by_key(|pos| chebyshev(*pos, builder_pos));
    // Cardinals were sorted too; keep the combined order stable by
    // re-sorting only within each group (done above per group).
    candidates.extend(diagonals);

    for (x, y) in candidates {
        if (x, y) == builder_pos || !tile_free(world, x, y) {
            continue;
        }
        if !valid_path_after(world, (x, y), builder_pos, anchor) {
            continue;
        }
        let wall = place_spite_wall_at(world, builder, target, x, y);
        world.init_resource::<SpiteState>();
        {
            let pass = world.resource::<SpiteState>().passes;
            let mut state = world.resource_mut::<SpiteState>();
            state.walls_built += 1;
            state.last_build_pass.insert(builder, pass);
        }
        send_chronicle(
            world,
            "A spite wall goes up by a rival's door — petty, pointed, and \
             load-bearing only for a grudge."
                .to_string(),
        );
        return Some(wall);
    }
    None
}

/// Jams `amenity` against `target` on behalf of `saboteur` (Spec 363).
/// Returns false when the amenity is missing or already sabotaged.
pub fn sabotage_amenity(
    world: &mut World,
    saboteur: Entity,
    target: Entity,
    amenity: Entity,
) -> bool {
    if world.get::<Building>(amenity).is_none()
        || world.get::<SpiteSabotage>(amenity).is_some()
    {
        return false;
    }
    world.entity_mut(amenity).insert(SpiteSabotage {
        saboteur,
        target,
        remaining: SABOTAGE_DURATION,
    });
    world.init_resource::<SpiteState>();
    world.resource_mut::<SpiteState>().sabotages_done += 1;
    true
}

/// Attempts to use an amenity. A pop targeted by a [`SpiteSabotage`]
/// on that amenity is denied and takes the minor stress penalty
/// (leisure cost + mood modifier); everyone else succeeds.
pub fn try_use_amenity(world: &mut World, pop: Entity, amenity: Entity) -> bool {
    let blocked = world
        .get::<SpiteSabotage>(amenity)
        .is_some_and(|sabotage| sabotage.target == pop);
    if !blocked {
        return true;
    }
    if let Some(mut needs) = world.get_mut::<Needs>(pop) {
        needs.leisure = (needs.leisure - AMENITY_STRESS_LEISURE).max(0.0);
    }
    if let Some(mut morale) = world.get_mut::<Morale>(pop) {
        morale.add_modifier(MoodModifier {
            label: "Spite-Blocked Amenity".to_string(),
            value: AMENITY_MOOD_VALUE,
            duration: AMENITY_MOOD_TICKS,
        });
    }
    false
}

/// Tears down a spite wall: frees its tile, despawns it, and applies
/// the "My Work Destroyed" mood modifier to its *builder* (read off
/// the component, spec §5), whoever issued the demolition.
/// Returns false when `wall` is not a standing spite wall.
pub fn demolish_spite_wall(world: &mut World, wall: Entity) -> bool {
    let Some(spite) = world.get::<SpiteWall>(wall).copied() else {
        return false;
    };
    let pos = world.get::<GridPosition>(wall).map(|p| (p.x, p.y));
    if let Some((x, y)) = pos {
        if let Some(mut occupied) = world.get_resource_mut::<OccupiedTiles>() {
            occupied.0.remove(&(x, y));
        }
        if let Some(mut map) = world.get_resource_mut::<BuildingMap>() {
            map.0.remove(&(x, y));
        }
    }
    world.despawn(wall);

    if world.get::<Dead>(spite.builder).is_none() {
        if let Some(mut morale) = world.get_mut::<Morale>(spite.builder) {
            morale.add_modifier(MoodModifier {
                label: "My Work Destroyed".to_string(),
                value: DEMOLISH_MOOD_VALUE,
                duration: DEMOLISH_MOOD_TICKS,
            });
        }
    }
    world.init_resource::<SpiteState>();
    world.resource_mut::<SpiteState>().demolitions += 1;
    send_chronicle(
        world,
        "A spite wall is torn down. Somewhere, its builder seethes: \
         that was *their* work."
            .to_string(),
    );
    true
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Standalone spite driver, in the spontaneous-architecture pattern:
/// each pass, living pops with a deeply negative affinity toward a
/// nearby living rival may raise a spite wall (score-gated, chance-
/// rolled, per-builder cooldown and cap), and a builder with a
/// standing wall may escalate to amenity sabotage against the rival.
#[allow(clippy::type_complexity)]
pub fn check_spite_build_system(world: &mut World) {
    world.init_resource::<SpiteState>();
    world.resource_mut::<SpiteState>().passes += 1;
    let pass = world.resource::<SpiteState>().passes;

    // Snapshot candidate builders and their worst nearby rival.
    let mut plans: Vec<(Entity, Entity, f32)> = Vec::new();
    {
        let mut query =
            world.query::<(Entity, &GridPosition, &Relationships, Option<&Dead>)>();
        let pops: Vec<(Entity, (i32, i32), Vec<(Entity, f32)>)> = query
            .iter(world)
            .filter(|(_, _, _, dead)| dead.is_none())
            .map(|(entity, pos, rel, _)| {
                (
                    entity,
                    (pos.x, pos.y),
                    rel.affinities
                        .iter()
                        .map(|(target, affinity)| (*target, *affinity))
                        .collect(),
                )
            })
            .collect();
        let positions: HashMap<Entity, (i32, i32)> =
            pops.iter().map(|(e, p, _)| (*e, *p)).collect();
        for (builder, builder_pos, affinities) in &pops {
            let mut worst: Option<(Entity, f32, f32)> = None;
            for (target, affinity) in affinities {
                if *affinity > RIVAL_AFFINITY_THRESHOLD {
                    continue;
                }
                let Some(target_pos) = positions.get(target) else {
                    continue;
                };
                let distance = chebyshev(*builder_pos, *target_pos);
                let score = evaluate_build_spite(*affinity, distance);
                if score > 0.0 && worst.is_none_or(|(_, _, s)| score > s) {
                    worst = Some((*target, *affinity, score));
                }
            }
            if let Some((target, _, score)) = worst {
                plans.push((*builder, target, score));
            }
        }
    }

    let mut rng = rand::thread_rng();
    for (builder, target, score) in plans {
        if score < 0.5 {
            continue;
        }
        // Standing-wall census for this builder.
        let standing = {
            let mut query = world.query::<&SpiteWall>();
            query
                .iter(world)
                .filter(|wall| wall.builder == builder)
                .count()
        };
        let on_cooldown = world
            .resource::<SpiteState>()
            .last_build_pass
            .get(&builder)
            .is_some_and(|last| pass.saturating_sub(*last) < SPITE_COOLDOWN_PASSES);
        if standing < MAX_WALLS_PER_BUILDER
            && !on_cooldown
            && rng.gen_bool(SPITE_BUILD_CHANCE)
        {
            try_build_spite_wall(world, builder, target);
            continue;
        }
        // Escalation: a standing wall against this rival, but the
        // grudge is not done — jam the colony Tavern against them.
        let has_wall_vs_rival = {
            let mut query = world.query::<&SpiteWall>();
            query
                .iter(world)
                .any(|wall| wall.builder == builder && wall.target == target)
        };
        if has_wall_vs_rival && rng.gen_bool(SPITE_SABOTAGE_CHANCE) {
            let tavern = {
                let mut found = None;
                let mut query = world.query::<(Entity, &Building)>();
                for (entity, building) in query.iter(world) {
                    if building.building_type == BuildingType::Tavern
                        && world.get::<SpiteSabotage>(entity).is_none()
                    {
                        found = Some(entity);
                        break;
                    }
                }
                found
            };
            if let Some(tavern) = tavern {
                sabotage_amenity(world, builder, target, tavern);
            }
        }
    }
}

/// Ages amenity sabotages; a lapsed sabotage quietly frees the amenity.
pub fn spite_sabotage_decay_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut SpiteSabotage)>,
) {
    for (entity, mut sabotage) in &mut query {
        sabotage.remaining = sabotage.remaining.saturating_sub(1);
        if sabotage.remaining == 0 {
            commands.entity(entity).remove::<SpiteSabotage>();
        }
    }
}

// ---------------------------------------------------------------------------
// Read helpers (headless / STATS)
// ---------------------------------------------------------------------------

/// Number of standing spite walls in the world.
#[must_use]
pub fn spite_wall_count(world: &mut World) -> usize {
    let mut query = world.query::<&SpiteWall>();
    query.iter(world).count()
}

/// Number of active amenity sabotages in the world.
#[must_use]
pub fn spite_sabotage_count(world: &mut World) -> usize {
    let mut query = world.query::<&SpiteSabotage>();
    query.iter(world).count()
}

/// One-line status for the headless `spite` panel.
#[must_use]
pub fn spite_status_label(world: &mut World) -> String {
    let walls = spite_wall_count(world);
    let sabotages = spite_sabotage_count(world);
    world.init_resource::<SpiteState>();
    let state = world.resource::<SpiteState>();
    format!(
        "walls standing={walls} sabotages active={sabotages} \
         (built={}, sabotaged={}, demolished={})",
        state.walls_built, state.sabotages_done, state.demolitions
    )
}

// ---------------------------------------------------------------------------
// Tests — the spec's RED tests, adapted to real types
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;
    use bevy_ecs::system::RunSystemOnce;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![TerrainType::Grass; 400],
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());
        world.init_resource::<SpiteState>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    /// Builder at (5,10) who hates the rival at (11,10); the rival's
    /// home is the Housing building at (10,10).
    fn rival_pair(world: &mut World) -> (Entity, Entity) {
        let rival = world
            .spawn((Pop, GridPosition { x: 11, y: 10 }, Morale::default(), Needs::default()))
            .id();
        let builder = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 10 },
                Relationships::with_affinity(rival, -90.0),
                Morale::default(),
                Needs::default(),
            ))
            .id();
        let home = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 10, y: 10 },
                Housing {
                    capacity: 2,
                    residents: vec![rival],
                },
            ))
            .id();
        world.resource_mut::<BuildingMap>().0.insert((10, 10), home);
        world.resource_mut::<OccupiedTiles>().0.insert((10, 10));
        (builder, rival)
    }

    // Spec RED 1: a rivalrous pop builds a spite wall by the rival's home.
    #[test]
    fn test_pop_builds_spite_wall_to_block_rival() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);

        let wall = try_build_spite_wall(&mut world, builder, rival)
            .expect("deeply negative affinity should produce a spite wall");

        let spite = world.get::<SpiteWall>(wall).unwrap();
        assert_eq!(spite.builder, builder);
        assert_eq!(spite.target, rival);
        let pos = world.get::<GridPosition>(wall).unwrap();
        assert_eq!(
            chebyshev((pos.x, pos.y), (10, 10)),
            1,
            "wall must be adjacent to the rival's house, got ({}, {})",
            pos.x,
            pos.y
        );
        assert!(world
            .resource::<OccupiedTiles>()
            .0
            .contains(&(pos.x, pos.y)));
        assert_eq!(
            world.resource::<BuildingMap>().0.get(&(pos.x, pos.y)),
            Some(&wall)
        );
        assert_eq!(world.resource::<SpiteState>().walls_built, 1);
    }

    #[test]
    fn test_no_spite_without_a_grudge() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);
        world
            .get_mut::<Relationships>(builder)
            .unwrap()
            .set_affinity(rival, 40.0);
        assert!(try_build_spite_wall(&mut world, builder, rival).is_none());
        assert_eq!(spite_wall_count(&mut world), 0);
        // Scoring gate: friendship and distance both zero the urge.
        assert_eq!(evaluate_build_spite(40.0, 2), 0.0);
        assert_eq!(evaluate_build_spite(-90.0, 99), 0.0);
        assert!(evaluate_build_spite(-90.0, 2) > 0.5);
    }

    // Spec RED 2: pathfinding treats the wall as an obstacle and detours.
    #[test]
    fn test_spite_structures_reduce_efficiency_and_cause_pathfinding_detours() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);
        // Wall squarely on the straight line (0,0) -> (2,0).
        place_spite_wall_at(&mut world, builder, rival, 1, 0);

        let path = find_path(&world, (0, 0), (2, 0)).expect("a detour must exist");
        assert!(
            !path.contains(&(1, 0)),
            "path must route around the spite wall: {path:?}"
        );
        assert!(
            path.len() > 2,
            "detour must be longer than the direct 2-step path: {path:?}"
        );
    }

    // Spec §7: a spite wall may never seal off the rival's home.
    #[test]
    fn test_spite_wall_never_seals_off_rival_home() {
        let mut world = setup_world();
        // Rival home in the corner at (1,1); builder far at (10,10).
        let rival = world
            .spawn((Pop, GridPosition { x: 1, y: 2 }, Morale::default()))
            .id();
        let builder = world
            .spawn((
                Pop,
                GridPosition { x: 10, y: 10 },
                Relationships::with_affinity(rival, -95.0),
                Morale::default(),
            ))
            .id();
        let home = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 1, y: 1 },
                Housing {
                    capacity: 2,
                    residents: vec![rival],
                },
            ))
            .id();
        world.resource_mut::<BuildingMap>().0.insert((1, 1), home);
        world.resource_mut::<OccupiedTiles>().0.insert((1, 1));
        // Pre-wall three of the home's four entrances; the fourth is
        // the only candidate left and walling it would seal the home.
        for (x, y) in [(0, 1), (2, 1), (1, 0)] {
            place_spite_wall_at(&mut world, builder, rival, x, y);
        }
        // The last entrance itself must never be walled. A diagonal
        // wall is legitimate spite (it seals nothing), so a build may
        // still happen — but never on (1,2), and the entrance must
        // remain reachable afterwards.
        let built = try_build_spite_wall(&mut world, builder, rival);
        if let Some(wall) = built {
            let pos = world.get::<GridPosition>(wall).unwrap();
            assert_ne!(
                (pos.x, pos.y),
                (1, 2),
                "the last open entrance must never be walled"
            );
        }
        assert!(!world.resource::<OccupiedTiles>().0.contains(&(1, 2)));
        assert!(
            entrance_reachable(&world, (10, 10), (1, 1)),
            "the rival's home entrance must remain reachable"
        );
    }

    // Spec RED 3: tearing a wall down angers its builder.
    #[test]
    fn test_tearing_down_spite_wall_angers_builder() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);
        let wall = try_build_spite_wall(&mut world, builder, rival).unwrap();
        let pos = *world.get::<GridPosition>(wall).unwrap();

        assert!(demolish_spite_wall(&mut world, wall));
        assert!(world.get::<SpiteWall>(wall).is_none());
        assert!(!world.resource::<OccupiedTiles>().0.contains(&(pos.x, pos.y)));
        let morale = world.get::<Morale>(builder).unwrap();
        let modifier = morale
            .modifiers
            .iter()
            .find(|m| m.label == "My Work Destroyed")
            .expect("builder must receive the My Work Destroyed modifier");
        assert!((modifier.value - DEMOLISH_MOOD_VALUE).abs() < f32::EPSILON);
        assert_eq!(modifier.duration, DEMOLISH_MOOD_TICKS);
        assert_eq!(world.resource::<SpiteState>().demolitions, 1);
        // Demolishing a non-wall is a no-op.
        assert!(!demolish_spite_wall(&mut world, builder));
    }

    // Spec RED 4: amenity sabotage denies the target, with stress.
    #[test]
    fn test_spite_amenity_sabotage_restricts_access() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);
        let tavern = world
            .spawn((
                Building {
                    building_type: BuildingType::Tavern,
                },
                GridPosition { x: 3, y: 3 },
            ))
            .id();

        assert!(sabotage_amenity(&mut world, builder, rival, tavern));
        // Already sabotaged: a second jam is refused.
        assert!(!sabotage_amenity(&mut world, builder, rival, tavern));

        let leisure_before = world.get::<Needs>(rival).unwrap().leisure;
        assert!(
            !try_use_amenity(&mut world, rival, tavern),
            "the targeted rival must be denied"
        );
        let needs = world.get::<Needs>(rival).unwrap();
        assert!(
            (needs.leisure - (leisure_before - AMENITY_STRESS_LEISURE)).abs() < 0.001,
            "denial must cost the minor stress penalty"
        );
        assert!(world
            .get::<Morale>(rival)
            .unwrap()
            .modifiers
            .iter()
            .any(|m| m.label == "Spite-Blocked Amenity"));

        // Everyone else uses the amenity normally.
        assert!(try_use_amenity(&mut world, builder, tavern));
    }

    #[test]
    fn test_sabotage_lapses_after_duration() {
        let mut world = setup_world();
        let (builder, rival) = rival_pair(&mut world);
        let tavern = world
            .spawn((
                Building {
                    building_type: BuildingType::Tavern,
                },
                GridPosition { x: 3, y: 3 },
            ))
            .id();
        assert!(sabotage_amenity(&mut world, builder, rival, tavern));
        world.get_mut::<SpiteSabotage>(tavern).unwrap().remaining = 1;
        world.run_system_once(spite_sabotage_decay_system).unwrap();
        assert!(world.get::<SpiteSabotage>(tavern).is_none());
        assert!(try_use_amenity(&mut world, rival, tavern));
    }

    #[test]
    fn test_ip_guard_public_strings_are_original() {
        // IP rule: concept-only, original names. None of the public
        // strings this spec introduces may carry distinctive IP terms.
        let public_strings = [
            "My Work Destroyed",
            "Spite-Blocked Amenity",
            "A spite wall goes up by a rival's door — petty, pointed, and \
             load-bearing only for a grudge.",
        ];
        let banned = [
            "dwarf fortress",
            "stellaris",
            "rimworld",
            "windsor",
            "sad king billy",
        ];
        for text in public_strings {
            let lower = text.to_lowercase();
            for term in banned {
                assert!(!lower.contains(term), "public string leaks IP term {term}");
            }
        }
    }
}
