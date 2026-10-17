use crate::layer1::access_control::{AccessControl, AccessMode};
use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::admin::{AdminProvider, Office};
use crate::layer1::building::{Building, BuildingType, OccupiedTiles};
use crate::layer1::defense::Gate;
use crate::layer1::erosion::{ErosionGrid, MOVEMENT_EROSION_AMOUNT};
use crate::layer1::execution::components::{AtTarget, DetourPath, MovementTarget};
use crate::layer1::farm::Farm;
use crate::layer1::housing::Housing;
use crate::layer1::map::GridPosition;
use crate::layer1::particles::Particle;
use crate::layer1::physics::hit_stop::HitStop;
use crate::layer1::pop::{Role, Speed};
use crate::layer1::social::Tavern;
use crate::layer1::tech::hypno_learning::MentalFog;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use crate::layer1::traits::{get_trait_move_speed_modifier, Traits};
use crate::layer1::utility_types::{ActionType, StartPlan};
use bevy_ecs::prelude::*;
use ratatui::style::Color;
use std::collections::VecDeque;

/// Removes pops from farms/housing when they switch to a different action.
///
/// This system runs before `process_start_plan_system` to ensure pops are
/// properly removed from their previous assignment before moving to a new one.
pub fn cleanup_previous_assignment_system(
    pops_query: Query<(Entity, &AssignedTo), With<StartPlan>>,
    mut farms: Query<&mut Farm>,
    mut housing: Query<&mut Housing>,
    mut taverns: Query<&mut Tavern>,
    mut offices: Query<&mut Office>,
    mut commands: Commands,
) {
    for (pop_entity, assigned) in &pops_query {
        let assigned_entity = assigned.entity;
        match assigned.assignment_type {
            AssignmentType::FarmWorker => {
                if let Ok(mut farm) = farms.get_mut(assigned_entity) {
                    farm.workers.retain(|&w| w != pop_entity);
                }
            }
            AssignmentType::HousingResident => {
                if let Ok(mut h) = housing.get_mut(assigned_entity) {
                    h.residents.retain(|&r| r != pop_entity);
                }
            }
            AssignmentType::TavernVisitor => {
                if let Ok(mut tavern) = taverns.get_mut(assigned_entity) {
                    tavern.visitors.retain(|&v| v != pop_entity);
                }
            }
            AssignmentType::Administrator => {
                // Remove AdminProvider
                commands.entity(pop_entity).remove::<AdminProvider>();
                if let Ok(mut office) = offices.get_mut(assigned_entity) {
                    office.workers.retain(|&w| w != pop_entity);
                }
            }
            AssignmentType::LibraryWorker
            | AssignmentType::Patient
            | AssignmentType::Funeral
            | AssignmentType::ObservatoryWorker
            | AssignmentType::Surgery
            | AssignmentType::Sheriff
            | AssignmentType::DeepMining
            | AssignmentType::RationingBureaucrat
            | AssignmentType::Doctor => {}
        }

        commands.entity(pop_entity).remove::<AssignedTo>();
    }
}

/// Consumes `StartPlan` markers and creates `MovementTarget` components.
///
/// This system bridges the utility AI's decision (`StartPlan`) with the
/// movement system by creating `MovementTarget` for each pop.
pub fn process_start_plan_system(
    plans: Query<(Entity, &StartPlan)>,
    positions: Query<&GridPosition>,
    mut commands: Commands,
) {
    for (pop_entity, start_plan) in &plans {
        let action = start_plan.action;
        let target = start_plan.target;

        // Remove the StartPlan marker and any existing movement components
        commands
            .entity(pop_entity)
            .remove::<StartPlan>()
            .remove::<MovementTarget>()
            .remove::<AtTarget>()
            .remove::<DetourPath>();

        // If there's no target, skip (e.g., Idle action)
        let Some(target_entity) = target else {
            continue;
        };

        // Get the target's position (also validates entity existence)
        let Ok(&target_position) = positions.get(target_entity) else {
            continue;
        };

        // Insert new MovementTarget
        commands.entity(pop_entity).insert(MovementTarget {
            target_entity,
            target_position,
            for_action: action,
        });
    }
}

use bevy_ecs::system::SystemParam;

#[derive(SystemParam)]
pub struct MovementContext<'w, 's> {
    erosion: ResMut<'w, ErosionGrid>,
    terrain: Res<'w, TerrainGrid>,
    occupied_tiles: Option<Res<'w, OccupiedTiles>>,
    wind_grid: Option<Res<'w, crate::layer1::wind::WindGrid>>,
    tide: Option<Res<'w, crate::layer1::atmosphere::AtmosphericTide>>,
    clutter_grid: Option<Res<'w, crate::layer1::clutter::ClutterGrid>>,
    season: Option<Res<'w, crate::layer1::seasons::SeasonState>>,
    quirks: Option<Res<'w, crate::layer1::quirks::PlanetaryTraits>>,
    buildings: Query<
        'w,
        's,
        (
            &'static GridPosition,
            &'static Building,
            Option<&'static Gate>,
            Option<&'static AccessControl>,
        ),
    >,
}

/// Moves pops 1 tile per tick toward their target (Manhattan-style).
///
/// When a pop arrives at its target position (or adjacent for work), this system
/// marks it with `AtTarget`.
#[allow(clippy::type_complexity)]
pub fn movement_system(
    mut pops: Query<
        (
            Entity,
            &mut GridPosition,
            &MovementTarget,
            Option<&mut Speed>,
            Option<&Traits>,
            Option<&HitStop>,
            Option<&Role>,
            Option<&MentalFog>,
            Option<&crate::layer1::biology::health::Health>,
            Option<&mut DetourPath>,
        ),
        (Without<AtTarget>, Without<Building>),
    >,
    mut ctx: MovementContext,
    mut commands: Commands,
) {
    for (
        pop_entity,
        mut current_pos,
        mt,
        mut speed_opt,
        traits,
        hit_stop,
        role,
        fog,
        health,
        detour,
    ) in &mut pops
    {
        process_single_movement(
            pop_entity,
            &mut current_pos,
            mt,
            &mut speed_opt,
            MovementComponents {
                traits,
                hit_stop,
                role,
                fog,
                health,
                detour,
            },
            &mut ctx,
            &mut commands,
        );
    }
}

struct MovementComponents<'a> {
    traits: Option<&'a Traits>,
    hit_stop: Option<&'a HitStop>,
    role: Option<&'a Role>,
    fog: Option<&'a MentalFog>,
    health: Option<&'a crate::layer1::biology::health::Health>,
    detour: Option<Mut<'a, DetourPath>>,
}

fn process_single_movement(
    pop_entity: Entity,
    current_pos: &mut GridPosition,
    mt: &MovementTarget,
    speed_opt: &mut Option<Mut<Speed>>,
    components: MovementComponents<'_>,
    ctx: &mut MovementContext,
    commands: &mut Commands,
) {
    let MovementComponents {
        traits,
        hit_stop,
        role,
        fog,
        health,
        mut detour,
    } = components;
    // Ludwig: Check Hit Stop
    if let Some(hs) = hit_stop {
        if hs.ticks_remaining > 0 {
            return;
        }
    }

    let trait_mod = traits.map_or(1.0, get_trait_move_speed_modifier);
    let fog_mod = fog.map_or(1.0, |f| f.movement_penalty);
    let rust_lung_mod = health.map_or(1.0, |h| if h.has_rust_lung { 0.8 } else { 1.0 });

    // Spec 344: seasonal ground conditions (winter snow, spring mud)
    // scale how fast pops accumulate movement each tick.
    let season_mod = ctx
        .season
        .as_deref()
        .map_or(1.0, |s| s.current_season.movement_modifier());

    // Accumulate speed
    if let Some(ref mut speed) = speed_opt {
        speed.accumulator += speed.current * trait_mod * fog_mod * rust_lung_mod * season_mod;
    }

    let target_pos = mt.target_position;
    let action = mt.for_action;

    // Check pre-move adjacency for work
    if check_work_adjacency(
        *current_pos,
        target_pos,
        action,
        &ctx.terrain,
        ctx.occupied_tiles.as_deref(),
        &ctx.buildings,
        pop_entity,
        role.copied(),
    ) {
        commands.entity(pop_entity).insert(AtTarget);
        commands.entity(pop_entity).remove::<DetourPath>();
        return;
    }

    // Check if already at target (for non-work actions requiring exact position)
    if *current_pos == target_pos {
        commands.entity(pop_entity).insert(AtTarget);
        commands.entity(pop_entity).remove::<DetourPath>();
        return;
    }

    let (primary, secondary) = calculate_next_positions(*current_pos, target_pos);

    // Selection Phase, in order:
    // 1. Follow a committed DetourPath (Item 94) if one is active
    //    (peek only — the step is consumed after the move succeeds).
    // 2. The greedy primary/secondary candidates.
    // 3. If both greedy candidates fail, plan a detour; it is stored
    //    as a DetourPath only after this tick's move succeeds, so a
    //    failed speed check never loses a step.
    let mut chosen_pos = follow_detour_step(
        &mut detour,
        target_pos,
        &ctx.terrain,
        ctx.occupied_tiles.as_deref(),
        &ctx.buildings,
        pop_entity,
        role.copied(),
    );
    let from_active_detour = chosen_pos.is_some();
    if chosen_pos.is_none() {
        chosen_pos = try_get_walkable_pos(
            primary,
            &ctx.terrain,
            ctx.occupied_tiles.as_deref(),
            &ctx.buildings,
            pop_entity,
            role.copied(),
        );
    }
    if chosen_pos.is_none() {
        chosen_pos = try_get_walkable_pos(
            secondary,
            &ctx.terrain,
            ctx.occupied_tiles.as_deref(),
            &ctx.buildings,
            pop_entity,
            role.copied(),
        );
    }
    let mut planned_detour: Option<VecDeque<GridPosition>> = None;
    if chosen_pos.is_none() {
        // Item 94: both greedy candidates failed. When the pop is
        // aligned with its target on one axis, calculate_next_positions
        // only ever offered ONE candidate (the other axis is None), so
        // a single unwalkable tile wedged the pop forever while its
        // speed accumulator grew and no decider re-plan could displace
        // a high-utility action. Plan a bounded BFS detour (see
        // detour_path) and COMMIT to it instead of waiting.
        if let Some(path) = detour_path(
            *current_pos,
            target_pos,
            &ctx.terrain,
            ctx.occupied_tiles.as_deref(),
            &ctx.buildings,
            pop_entity,
            role.copied(),
        ) {
            let steps: VecDeque<GridPosition> = path.into();
            chosen_pos = steps.front().copied();
            planned_detour = Some(steps);
        }
    }

    let Some(new_pos) = chosen_pos else {
        return;
    };

    // Spec 345: planetary quirks scale the per-tile movement *cost* —
    // the other side of the accumulator from the Spec 344 season factor
    // above. High gravity makes every tile cost 1.5x the accumulated
    // speed (the sketch's PopMovementCost, on the real cost path);
    // Speed.current already carries the quirk speed factor via
    // apply_quirk_modifiers_system, so the two compose, not duplicate.
    let quirk_cost_mod = ctx.quirks.as_deref().map_or(1.0, |q| {
        crate::layer1::quirks::GlobalModifiers::from_traits(&q.0).movement_cost
    });
    let movement_cost = calculate_movement_cost(
        new_pos,
        *current_pos,
        &ctx.terrain,
        ctx.wind_grid.as_deref(),
        ctx.tide.as_deref(),
        ctx.clutter_grid.as_deref(),
    ) * quirk_cost_mod;

    if !try_apply_movement_speed(speed_opt, movement_cost, commands, *current_pos) {
        return;
    }

    current_pos.x = new_pos.x;
    current_pos.y = new_pos.y;

    // Item 94: the move succeeded — now consume/commit detour state.
    // (Doing this before the speed check would lose a step whenever
    // the accumulator was short, leaving the pop one tile behind its
    // stored path.)
    if from_active_detour {
        if let Some(d) = detour.as_mut() {
            let _ = d.steps.pop_front();
        }
    }
    if let Some(mut steps) = planned_detour {
        let _ = steps.pop_front(); // the step just taken
        commands.entity(pop_entity).insert(DetourPath {
            target: target_pos,
            steps,
        });
    }

    apply_erosion(&mut ctx.erosion, new_pos);

    if new_pos == target_pos {
        commands.entity(pop_entity).insert(AtTarget);
        commands.entity(pop_entity).remove::<DetourPath>();
    }

    // Check post-move adjacency for work
    if check_work_adjacency(
        new_pos,
        target_pos,
        action,
        &ctx.terrain,
        ctx.occupied_tiles.as_deref(),
        &ctx.buildings,
        pop_entity,
        role.copied(),
    ) {
        commands.entity(pop_entity).insert(AtTarget);
        commands.entity(pop_entity).remove::<DetourPath>();
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check_work_adjacency(
    current_pos: GridPosition,
    target_pos: GridPosition,
    action: ActionType,
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    buildings: &Query<(
        &GridPosition,
        &Building,
        Option<&Gate>,
        Option<&AccessControl>,
    )>,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> bool {
    if action != ActionType::Work
        && action != ActionType::Repair
        && action != ActionType::ScrawlMemeticSigil
        && action != ActionType::MemeticObsession
    {
        return false;
    }

    if is_tile_walkable(
        terrain,
        occupied,
        buildings,
        target_pos.x,
        target_pos.y,
        pop_entity,
        pop_role,
    ) {
        return false;
    }

    let distance = current_pos.distance_manhattan(target_pos);
    distance == 1
}

pub(crate) fn try_get_walkable_pos(
    pos: Option<GridPosition>,
    terrain: &TerrainGrid,
    occupied_tiles: Option<&OccupiedTiles>,
    buildings: &Query<(
        &GridPosition,
        &Building,
        Option<&Gate>,
        Option<&AccessControl>,
    )>,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> Option<GridPosition> {
    let p = pos?;
    if is_tile_walkable(
        terrain,
        occupied_tiles,
        buildings,
        p.x,
        p.y,
        pop_entity,
        pop_role,
    ) {
        Some(p)
    } else {
        None
    }
}

/// Collision-relevant data for one building tile, pre-extracted so callers
/// without a Bevy system `Query` (e.g. the headless console) can reuse the
/// exact same walkability rules as [`is_tile_walkable`].
#[derive(Clone)]
pub(crate) struct BuildingCollisionInfo {
    pub pos: GridPosition,
    pub building_type: BuildingType,
    pub access: Option<AccessControl>,
}

impl BuildingCollisionInfo {
    pub fn from_query_item(
        pos: &GridPosition,
        building: &Building,
        access: Option<&AccessControl>,
    ) -> Self {
        Self {
            pos: *pos,
            building_type: building.building_type,
            access: access.cloned(),
        }
    }
}

/// Data-driven core of the walkability check.
///
/// Same rules as [`is_tile_walkable`], but takes pre-extracted building
/// data instead of a system `Query`.
pub(crate) fn is_tile_walkable_data(
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    buildings: &[BuildingCollisionInfo],
    x: i32,
    y: i32,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> bool {
    // Check Terrain bounds and type
    let Ok(x_idx) = usize::try_from(x) else {
        return false;
    };
    let Ok(y_idx) = usize::try_from(y) else {
        return false;
    };

    if !terrain
        .get(x_idx, y_idx)
        .is_some_and(TerrainType::is_walkable)
    {
        return false;
    }

    // Check Buildings
    let Some(occupied_tiles) = occupied else {
        return true;
    };
    if !occupied_tiles.0.contains(&(x, y)) {
        return true;
    }

    for info in buildings {
        if info.pos.x == x && info.pos.y == y {
            // Priority: AccessControl
            if let Some(access) = &info.access {
                return match access.mode {
                    AccessMode::Public => true,
                    AccessMode::Lockdown => false,
                    AccessMode::Restricted => {
                        access.allowed_pops.contains(&pop_entity)
                            || pop_role.is_some_and(|r| access.allowed_roles.contains(&r))
                    }
                };
            }

            // Fallback: Gate (Legacy) - deprecated but kept for safety if AccessControl missing
            // If we decide to fully remove logic, we can just skip this.
            // But spec said "Replace Gate.is_locked".
            // Since we are adding AccessControl to Gates, we should rely on AccessControl.
            // If AccessControl is missing, we check building.is_obstacle().
            // Gate is an obstacle.
            if info.building_type.is_obstacle() {
                return false;
            }
            return true;
        }
    }
    true
}

pub(crate) fn is_tile_walkable(
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    buildings: &Query<(
        &GridPosition,
        &Building,
        Option<&Gate>,
        Option<&AccessControl>,
    )>,
    x: i32,
    y: i32,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> bool {
    let info: Vec<BuildingCollisionInfo> = buildings
        .iter()
        .map(|(pos, building, _gate, access_opt)| {
            BuildingCollisionInfo::from_query_item(pos, building, access_opt)
        })
        .collect();
    is_tile_walkable_data(terrain, occupied, &info, x, y, pop_entity, pop_role)
}

#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub(crate) fn calculate_next_positions(
    current: GridPosition,
    target: GridPosition,
) -> (Option<GridPosition>, Option<GridPosition>) {
    // Calculate movement direction (Manhattan)
    let (dx, dy) = current.direction_to(target);

    let move_x = if dx != 0 {
        Some(GridPosition {
            x: current.x + dx,
            y: current.y,
        })
    } else {
        None
    };

    let move_y = if dy != 0 {
        Some(GridPosition {
            x: current.x,
            y: current.y + dy,
        })
    } else {
        None
    };

    // Prefer horizontal movement
    if dx != 0 {
        (move_x, move_y)
    } else {
        (move_y, None)
    }
}

/// Item 94: peek at the next step of the pop's committed
/// [`DetourPath`], or abandon the detour if it no longer applies.
///
/// Returns the front step WITHOUT consuming it (the caller consumes
/// it only after the move actually succeeds). Returns `None` — and
/// clears the component — when the detour's target no longer matches
/// the current movement target, when its steps are exhausted, or when
/// the next stored step has become unwalkable; movement then falls
/// back to greedy candidates and, if those fail, a fresh plan.
fn follow_detour_step(
    detour: &mut Option<Mut<DetourPath>>,
    target: GridPosition,
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    buildings: &Query<(
        &GridPosition,
        &Building,
        Option<&Gate>,
        Option<&AccessControl>,
    )>,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> Option<GridPosition> {
    let (clear, next) = {
        let d = detour.as_ref()?;
        if d.target != target || d.steps.is_empty() {
            (true, None)
        } else {
            let next = d.steps[0];
            if is_tile_walkable(
                terrain, occupied, buildings, next.x, next.y, pop_entity, pop_role,
            ) {
                (false, Some(next))
            } else {
                (true, None)
            }
        }
    };
    if clear {
        *detour = None;
        return None;
    }
    next
}

/// Item 94 detour plan: a shortest walkable path from `current` to
/// `target`, found with bounded breadth-first search. The path
/// excludes `current` and ends at the goal tile.
///
/// This is the sidestep-with-lookahead fix for the greedy wedge, and
/// the lookahead is why the caller COMMITS the whole path as a
/// [`DetourPath`] instead of taking one sidestep and re-deciding
/// greedily next tick: greedy re-decision oscillates. (Measured on
/// the first version of this fix, which replanned per tick: a pop at
/// (5,5) detouring around rock at (5,6) bounced (6,5) <-> (5,5)
/// forever, because from the sidestep tile the greedy horizontal
/// step back into the blocked column is walkable and is preferred.)
/// Following stored steps cannot reverse like that.
///
/// Goal: `target` itself when it is walkable; otherwise any walkable
/// tile Manhattan-adjacent to it (the stopping condition work-style
/// actions already use for unwalkable building/item tiles).
///
/// Fails closed (`None`) when no goal is reachable within
/// `MAX_DETOUR_VISITED` visited tiles — a genuinely enclosed target
/// leaves the pop waiting exactly as before, never teleporting and
/// never entering an unwalkable tile. The search only runs when both
/// greedy candidates failed, so unobstructed movement pays nothing.
pub(crate) fn detour_path(
    current: GridPosition,
    target: GridPosition,
    terrain: &TerrainGrid,
    occupied: Option<&OccupiedTiles>,
    buildings: &Query<(
        &GridPosition,
        &Building,
        Option<&Gate>,
        Option<&AccessControl>,
    )>,
    pop_entity: Entity,
    pop_role: Option<Role>,
) -> Option<Vec<GridPosition>> {
    /// Upper bound on tiles one detour search may visit. The standard
    /// map is 80x50 = 4000 tiles, so this covers the whole map while
    /// keeping a wedged pop's detour planning bounded.
    const MAX_DETOUR_VISITED: usize = 4096;

    // Snapshot building collision data once: is_tile_walkable would
    // rebuild this Vec for every one of the (potentially thousands
    // of) BFS nodes.
    let building_info: Vec<BuildingCollisionInfo> = buildings
        .iter()
        .map(|(pos, building, _gate, access_opt)| {
            BuildingCollisionInfo::from_query_item(pos, building, access_opt)
        })
        .collect();
    let walkable = |p: GridPosition| {
        is_tile_walkable_data(
            terrain,
            occupied,
            &building_info,
            p.x,
            p.y,
            pop_entity,
            pop_role,
        )
    };

    let target_walkable = walkable(target);
    let is_goal = |p: GridPosition| {
        if target_walkable {
            p == target
        } else {
            p.distance_manhattan(target) == 1
        }
    };
    if is_goal(current) {
        return None;
    }

    use std::collections::{HashMap, VecDeque};
    let mut parent: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut queue: VecDeque<GridPosition> = VecDeque::new();
    parent.insert((current.x, current.y), (current.x, current.y));
    queue.push_back(current);

    while let Some(node) = queue.pop_front() {
        if node != current && is_goal(node) {
            // Reconstruct the full path (excluding `current`).
            let mut path = Vec::new();
            let mut cur = node;
            while cur != current {
                path.push(cur);
                let p = parent[&(cur.x, cur.y)];
                cur = GridPosition { x: p.0, y: p.1 };
            }
            path.reverse();
            return Some(path);
        }
        if parent.len() >= MAX_DETOUR_VISITED {
            return None;
        }
        // Explore neighbours closest-to-target first so equal-length
        // shortest paths prefer the natural around-the-obstacle route.
        let mut neighbours = [
            GridPosition {
                x: node.x + 1,
                y: node.y,
            },
            GridPosition {
                x: node.x - 1,
                y: node.y,
            },
            GridPosition {
                x: node.x,
                y: node.y + 1,
            },
            GridPosition {
                x: node.x,
                y: node.y - 1,
            },
        ];
        neighbours.sort_by_key(|p| p.distance_manhattan(target));
        for next in neighbours {
            if parent.contains_key(&(next.x, next.y)) || !walkable(next) {
                continue;
            }
            parent.insert((next.x, next.y), (node.x, node.y));
            queue.push_back(next);
        }
    }
    None
}

fn calculate_movement_cost(
    new_pos: GridPosition,
    current_pos: GridPosition,
    terrain: &TerrainGrid,
    wind_grid: Option<&crate::layer1::wind::WindGrid>,
    tide: Option<&crate::layer1::atmosphere::AtmosphericTide>,
    clutter_grid: Option<&crate::layer1::clutter::ClutterGrid>,
) -> f32 {
    let base_cost = if let (Ok(x), Ok(y)) = (usize::try_from(new_pos.x), usize::try_from(new_pos.y))
    {
        let cost = terrain
            .get(x, y)
            .map_or(1.0, crate::layer1::terrain::TerrainType::movement_cost);
        let clutter_mod = clutter_grid.map_or(0.0, |c| c.get(x, y) * 0.1);
        cost + clutter_mod
    } else {
        1.0
    };

    let wind_mod = if let Some(w) = wind_grid {
        let wind_vec = w.get_wind(current_pos.x, current_pos.y);
        let move_dir = crate::layer1::wind::Vec2::new(
            (new_pos.x - current_pos.x) as f32,
            (new_pos.y - current_pos.y) as f32,
        );
        crate::layer1::wind::calculate_wind_movement_penalty(wind_vec, move_dir)
    } else {
        1.0
    };

    let pressure_mod = tide.map_or(1.0, |t| {
        crate::layer1::atmosphere::calculate_atmospheric_movement_cost(t.pressure)
    });

    base_cost * wind_mod * pressure_mod
}

fn try_apply_movement_speed(
    speed_opt: &mut Option<Mut<Speed>>,
    movement_cost: f32,
    commands: &mut Commands,
    current_pos: GridPosition,
) -> bool {
    let Some(ref mut speed) = speed_opt else {
        return true;
    };

    // Ludwig: "Coyote Speed" - Allow moving if we are *almost* there.
    // This prevents the feeling of "just missing the bus" by 0.01 speed.
    // Increased to 0.35 for even better flow, making movement feel far less clunky when encumbered.
    const COYOTE_THRESHOLD: f32 = 0.35;
    if speed.accumulator >= (movement_cost - COYOTE_THRESHOLD) {
        // If we are using the threshold to squeeze by...
        if speed.accumulator < movement_cost {
            // Ludwig: "Hustle" particle to show effort
            commands.spawn((
                Particle {
                    char: '.',
                    color: Color::DarkGray,
                    lifetime: 5,
                },
                current_pos,
            ));
        }
        speed.accumulator -= movement_cost;
        true
    } else {
        false
    }
}

fn apply_erosion(erosion: &mut ErosionGrid, new_pos: GridPosition) {
    if let (Ok(x), Ok(y)) = (usize::try_from(new_pos.x), usize::try_from(new_pos.y)) {
        erosion.add_erosion(x, y, MOVEMENT_EROSION_AMOUNT);
    }
}
