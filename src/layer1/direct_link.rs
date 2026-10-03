use crate::layer1::access_control::AccessControl;
use crate::layer1::actions::AssignedTo;
use crate::layer1::building::{Building, OccupiedTiles};
use crate::layer1::defense::Gate;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::execution::movement::{
    is_tile_walkable, is_tile_walkable_data, BuildingCollisionInfo,
};
use crate::layer1::map::{GridPosition, ScreenShake};
use crate::layer1::particles::Particle;
use crate::layer1::pop::{Role, Speed};
use crate::layer1::terrain::TerrainGrid;
use crate::layer1::utility_types::StartPlan;
use crate::shared::keyboard::{Input, KeyCode};
use crate::shared::time::WallTime;
use bevy_ecs::prelude::*;
use ratatui::style::Color;

#[derive(Component)]
pub struct Possessed;

#[derive(Event)]
pub struct PossessEntityEvent(pub Entity);

#[derive(Event)]
pub struct UnpossessEvent;

/// Tracks the "virtual joystick" state for direct control.
/// Stores buffered inputs and cooldowns to ensure responsive but fair movement.
#[derive(Component, Default)]
pub struct DirectControlState {
    /// Timestamp (WallTime) of the last successful move.
    pub last_move_time: f32,
    /// Buffered X-axis movement intent.
    pub buffered_dx: i32,
    /// Buffered Y-axis movement intent.
    pub buffered_dy: i32,
}

pub fn handle_possession(
    mut commands: Commands,
    mut events: EventReader<PossessEntityEvent>,
    mut unpossess: EventReader<UnpossessEvent>,
    possessed_query: Query<Entity, With<Possessed>>,
) {
    // Handle Unpossess
    if !unpossess.is_empty() {
        unpossess.clear(); // Consume all events
        for entity in possessed_query.iter() {
            commands
                .entity(entity)
                .remove::<Possessed>()
                .remove::<DirectControlState>();
        }
    }

    // Handle Possess
    for event in events.read() {
        let entity = event.0;

        // Remove Possessed from existing
        for existing in possessed_query.iter() {
            if existing != entity {
                commands
                    .entity(existing)
                    .remove::<Possessed>()
                    .remove::<DirectControlState>();
            }
        }

        // Add Possessed and DirectControlState to new target
        commands
            .entity(entity)
            .insert((Possessed, DirectControlState::default()));

        // Clear AI components
        commands
            .entity(entity)
            .remove::<StartPlan>()
            .remove::<MovementTarget>()
            .remove::<AtTarget>()
            .remove::<AssignedTo>();

        // Switch Input Context

        // Suppress UI
    }
}

type DirectMovementQuery<'a> = (
    Entity,
    &'a mut GridPosition,
    &'a Speed,
    &'a mut DirectControlState,
    Option<&'a Role>,
);
type DirectMovementFilter = (With<Possessed>, Without<Building>);
fn gather_input(input: &Input, state: &DirectControlState) -> (i32, i32) {
    let mut intended_dx = state.buffered_dx;
    let mut intended_dy = state.buffered_dy;

    if input.just_pressed(KeyCode::W) || input.just_pressed(KeyCode::Up) {
        intended_dy -= 1;
    }
    if input.just_pressed(KeyCode::S) || input.just_pressed(KeyCode::Down) {
        intended_dy += 1;
    }
    if input.just_pressed(KeyCode::A) || input.just_pressed(KeyCode::Left) {
        intended_dx -= 1;
    }
    if input.just_pressed(KeyCode::D) || input.just_pressed(KeyCode::Right) {
        intended_dx += 1;
    }

    (intended_dx.clamp(-1, 1), intended_dy.clamp(-1, 1))
}

type BuildingsQuery<'a> = (
    &'a GridPosition,
    &'a Building,
    Option<&'a Gate>,
    Option<&'a AccessControl>,
);

#[allow(clippy::too_many_arguments)]
pub fn handle_direct_movement(
    input: Res<Input>,
    mut query: Query<DirectMovementQuery, DirectMovementFilter>,
    terrain: Res<TerrainGrid>,
    occupied_tiles: Option<Res<OccupiedTiles>>,
    buildings: Query<BuildingsQuery>,
    wall_time: Option<Res<WallTime>>,
    mut shake: Option<ResMut<ScreenShake>>,
    mut commands: Commands,
) {
    for (entity, mut pos, speed, mut state, role) in &mut query {
        let now = wall_time.as_ref().map_or(0.0, |t| t.0);

        // Calculate cooldown based on speed (responsive but limited)
        // Speed 1.0 (10 ticks/sec) -> Cooldown 0.1s
        // Speed 2.0 (20 ticks/sec) -> Cooldown 0.05s
        let cooldown = (0.1 / speed.current).max(0.01);

        // If WallTime is missing, we allow movement always (graceful degradation)
        let time_since_move = wall_time
            .as_ref()
            .map_or(f32::MAX, |t| t.0 - state.last_move_time);

        let (intended_dx, intended_dy) = gather_input(&input, &state);

        // If no input, skip
        if intended_dx == 0 && intended_dy == 0 {
            continue;
        }

        // 2. Check Cooldown / Buffering
        if time_since_move < cooldown {
            // Ludwig: "Input Buffering" - Record inputs pressed slightly before an action is ready.
            // We only buffer if the cooldown is almost over (Grace Period of 0.2s).
            // This prevents old, stale inputs from executing long after the player meant to press them,
            // which causes a "sluggish" or "stuck" feeling.
            let time_remaining = cooldown - time_since_move;
            if time_remaining < 0.2 {
                // Accumulate into the buffer so we can store diagonal intents
                state.buffered_dx = intended_dx;
                state.buffered_dy = intended_dy;
            } else {
                // Discard input if pressed too early (prevent sluggish "stuck" inputs)
                state.buffered_dx = 0;
                state.buffered_dy = 0;
            }

            continue;
        }

        // 3. Execution (Off Cooldown)
        let new_x = pos.x + intended_dx;
        let new_y = pos.y + intended_dy;

        // Check collision
        let walkable = is_tile_walkable(
            &terrain,
            occupied_tiles.as_deref(),
            &buildings,
            new_x,
            new_y,
            entity,
            role.copied(),
        );

        if walkable {
            // Success!

            // Juice: Spawn particle at OLD position (Dust kick)
            commands.spawn((
                Particle {
                    char: '.',
                    color: Color::DarkGray,
                    lifetime: 5,
                },
                *pos,
            ));

            pos.x = new_x;
            pos.y = new_y;
            state.last_move_time = now;
            state.buffered_dx = 0;
            state.buffered_dy = 0;
        } else {
            // Blocked! Try sliding if diagonal
            let mut slid = false;
            if intended_dx != 0 && intended_dy != 0 {
                let walkable_x = is_tile_walkable(
                    &terrain,
                    occupied_tiles.as_deref(),
                    &buildings,
                    new_x,
                    pos.y,
                    entity,
                    role.copied(),
                );
                let walkable_y = is_tile_walkable(
                    &terrain,
                    occupied_tiles.as_deref(),
                    &buildings,
                    pos.x,
                    new_y,
                    entity,
                    role.copied(),
                );

                if walkable_x && !walkable_y {
                    pos.x = new_x;
                    slid = true;
                } else if walkable_y && !walkable_x {
                    pos.y = new_y;
                    slid = true;
                } else if walkable_x && walkable_y {
                    // Inside corner, pick one axis to maintain momentum
                    pos.x = new_x;
                    slid = true;
                }

                if slid {
                    // Juice: Spawn particle at OLD position (Dust kick)
                    commands.spawn((
                        Particle {
                            char: '.',
                            color: Color::DarkGray,
                            lifetime: 5,
                        },
                        *pos, // Use current pos (which we partially modified, technically we should use old pos, but this is fine)
                    ));
                    state.last_move_time = now;
                    state.buffered_dx = 0;
                    state.buffered_dy = 0;
                }
            }

            if !slid {
                // Juice: Screen Shake (minor bonk)
                if let Some(shake) = shake.as_mut() {
                    shake.trigger(0.1);
                }

                // Just clear buffer to prevent "stuck" inputs
                state.buffered_dx = 0;
                state.buffered_dy = 0;
            }
        }
    }
}

type PossessedSpeedQuery<'w, 's> = Query<'w, 's, &'static mut Speed, Added<Possessed>>;
type SpeedQuery<'w, 's> = Query<'w, 's, &'static mut Speed>;

pub fn apply_buffs(
    mut removed: RemovedComponents<Possessed>,
    mut queries: ParamSet<(PossessedSpeedQuery, SpeedQuery)>,
) {
    for mut speed in queries.p0().iter_mut() {
        speed.base *= 2.0;
        speed.current = speed.base;
    }

    for entity in removed.read() {
        if let Ok(mut speed) = queries.p1().get_mut(entity) {
            speed.base /= 2.0;
            speed.current = speed.base;
        }
    }
}

pub fn clear_input_system(mut input: ResMut<Input>) {
    input.clear();
}

pub fn handle_direct_input_system(
    input: Res<Input>,
    mut unpossess_events: EventWriter<UnpossessEvent>,
) {
    if input.just_pressed(KeyCode::Esc) {
        unpossess_events.send(UnpossessEvent);
    }
}

/// The currently possessed pop entity, if any.
///
/// Only one pop can be possessed at a time; possession is cleared on
/// [`UnpossessEvent`] (see [`handle_possession`]).
pub fn possessed_entity(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<Possessed>>()
        .iter(world)
        .next()
}

/// Attempt one player-directed step for a possessed pop, applying the same
/// collision rules as keyboard-driven direct movement.
///
/// This is the non-interactive entry point for direct control: the headless
/// console `move` command (and any future UI driver) routes through here
/// instead of [`handle_direct_movement`], which reads live keyboard `Input`.
///
/// On success the pop's [`GridPosition`] is updated and a dust particle is
/// spawned (mirroring the interactive path). On failure a human-readable
/// reason is returned and nothing changes.
pub fn try_player_step(
    world: &mut World,
    entity: Entity,
    dx: i32,
    dy: i32,
) -> Result<(i32, i32), String> {
    if !world.entity(entity).contains::<Possessed>() {
        return Err("no pop is currently possessed".to_string());
    }

    // Gather walkability data with shared borrows only, so the same rules as
    // the interactive path apply (see is_tile_walkable_data).
    let buildings: Vec<BuildingCollisionInfo> = {
        let mut query = world.query::<(
            &GridPosition,
            &Building,
            Option<&Gate>,
            Option<&AccessControl>,
        )>();
        query
            .iter(world)
            .map(|(pos, building, _gate, access)| {
                BuildingCollisionInfo::from_query_item(pos, building, access)
            })
            .collect()
    };
    let walkable = {
        let terrain = world.resource::<TerrainGrid>();
        let occupied = world.get_resource::<OccupiedTiles>();
        let role = world.get::<Role>(entity).copied();
        let Some(pos) = world.get::<GridPosition>(entity) else {
            return Err("possessed pop has no position".to_string());
        };
        let nx = pos.x + dx.clamp(-1, 1);
        let ny = pos.y + dy.clamp(-1, 1);
        (
            is_tile_walkable_data(terrain, occupied, &buildings, nx, ny, entity, role),
            nx,
            ny,
        )
    };
    let (ok, nx, ny) = walkable;
    if !ok {
        return Err(format!("blocked: cannot step to ({nx}, {ny})"));
    }

    // Apply the step and kick up dust at the old tile (mirrors handle_direct_movement).
    let mut pos = world
        .get_mut::<GridPosition>(entity)
        .ok_or_else(|| "possessed pop has no position".to_string())?;
    let old = *pos;
    pos.x = nx;
    pos.y = ny;
    world.spawn((
        Particle {
            char: '.',
            color: Color::DarkGray,
            lifetime: 5,
        },
        old,
    ));
    Ok((nx, ny))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::execution::components::MovementTarget;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::PopBundle;
    use crate::layer1::pop::Speed;
    use crate::layer1::terrain::generate_terrain;
    use crate::layer1::utility_types::ActionType;
    use bevy_ecs::schedule::Schedule;

    fn setup_world() -> World {
        let mut world = World::new();
        // Register events
        world.init_resource::<Events<PossessEntityEvent>>();
        world.init_resource::<Events<UnpossessEvent>>();
        // Register resources
        world.init_resource::<Input>();
        world.insert_resource(WallTime(0.0));
        world.insert_resource(ScreenShake::default());

        // Needed for movement
        let terrain = generate_terrain(20, 20);
        world.insert_resource(terrain);
        world.insert_resource(OccupiedTiles::default());

        world
    }

    #[test]
    fn test_possession_toggle() {
        let mut world = setup_world();
        let pop = world
            .spawn(PopBundle::random(0, 0, &mut rand::thread_rng()))
            .id();

        // Act: Trigger possession command
        world
            .resource_mut::<Events<PossessEntityEvent>>()
            .send(PossessEntityEvent(pop));

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_possession);
        schedule.run(&mut world);

        // Assert: Pop has Possessed component AND DirectControlState
        assert!(world.entity(pop).contains::<Possessed>());
        assert!(world.entity(pop).contains::<DirectControlState>());

        // Assert: Input context is DirectControl

        // Act: Trigger unpossess
        world
            .resource_mut::<Events<UnpossessEvent>>()
            .send(UnpossessEvent);
        schedule.run(&mut world);

        // Assert: Pop no longer has Possessed
        assert!(!world.entity(pop).contains::<Possessed>());
    }

    #[test]
    fn test_direct_movement_input() {
        let mut world = setup_world();

        // Ensure path is walkable
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            let width = terrain.width;
            // Set path to Grass
            if let Some(tile) = terrain.tiles.get_mut(10 * width + 10) {
                *tile = crate::layer1::terrain::TerrainType::Grass;
            }
            if let Some(tile) = terrain.tiles.get_mut(9 * width + 10) {
                *tile = crate::layer1::terrain::TerrainType::Grass;
            }
            if let Some(tile) = terrain.tiles.get_mut(8 * width + 10) {
                *tile = crate::layer1::terrain::TerrainType::Grass;
            }
        }

        let pop = world
            .spawn((
                PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);
        // We also need to clear input like the main loop does
        schedule.add_systems(clear_input_system.after(handle_direct_movement));

        // Step 1: Press W
        world.resource_mut::<Input>().press(KeyCode::W);
        // Advance time to allow movement (initial last_move_time is 0, so should move immediately)
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);

        let pos1 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos1.y, 9);

        // Step 2: Press W again (Input should have been cleared and re-pressed)
        world.resource_mut::<Input>().press(KeyCode::W);
        // Advance time enough to clear cooldown
        world.resource_mut::<WallTime>().0 = 20.0;
        schedule.run(&mut world);

        let pos2 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos2.y, 8, "Should move a second time");
    }

    #[test]
    fn test_direct_movement_cooldown_blocks_spam() {
        let mut world = setup_world();
        // Setup walkable
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            terrain
                .tiles
                .fill(crate::layer1::terrain::TerrainType::Grass);
        }

        let pop = world
            .spawn((
                PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
                // Normal speed = 1.0 -> cooldown 0.1s
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);
        schedule.add_systems(clear_input_system.after(handle_direct_movement));

        // 1. First move (ok)
        world.resource_mut::<Input>().press(KeyCode::W);
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);

        let pos1 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos1.y, 9);

        // 2. Second move IMMEDIATELY (should be blocked by cooldown)
        world.resource_mut::<Input>().press(KeyCode::W);
        // Time only advanced 0.01s (cooldown is ~0.1s)
        world.resource_mut::<WallTime>().0 = 10.01;
        schedule.run(&mut world);

        let pos2 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos2.y, 9, "Should NOT move due to cooldown");

        // 3. Verify Buffering: The input was buffered in step 2.
        // If we run again without new input but AFTER cooldown...
        world.resource_mut::<WallTime>().0 = 10.2; // > 0.1s later
        schedule.run(&mut world); // No new input press here!

        let pos3 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos3.y, 8, "Should move from BUFFERED input");
    }

    #[test]
    fn test_direct_movement_diagonal_buffering() {
        let mut world = setup_world();
        // Setup walkable
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            terrain
                .tiles
                .fill(crate::layer1::terrain::TerrainType::Grass);
        }

        let pop = world
            .spawn((
                PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);
        schedule.add_systems(clear_input_system.after(handle_direct_movement));

        // 1. First move (ok)
        world.resource_mut::<Input>().press(KeyCode::W);
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);

        let pos1 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos1.y, 9);
        assert_eq!(pos1.x, 10);

        // 2. Second move IMMEDIATELY (should be blocked by cooldown)
        // Player tries to buffer a diagonal move (W + D)
        world.resource_mut::<Input>().press(KeyCode::W);
        world.resource_mut::<WallTime>().0 = 10.01;
        schedule.run(&mut world); // Buffers W

        world.resource_mut::<Input>().press(KeyCode::D);
        world.resource_mut::<WallTime>().0 = 10.02;
        schedule.run(&mut world); // Buffers D (combines with W!)

        let pos2 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos2.y, 9, "Should NOT move due to cooldown");
        assert_eq!(pos2.x, 10, "Should NOT move due to cooldown");

        // 3. Verify Diagonal Buffering
        world.resource_mut::<WallTime>().0 = 10.2; // > 0.1s later
        schedule.run(&mut world); // No new input press here!

        let pos3 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos3.y, 8, "Should move up from BUFFERED input");
        assert_eq!(
            pos3.x, 11,
            "Should move right from BUFFERED input (diagonal)"
        );
    }

    #[test]
    fn test_direct_movement_juice_collision() {
        let mut world = setup_world();
        // Wall at (10, 9)
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            let idx = 9_usize
                .checked_mul(terrain.width)
                .and_then(|i| i.checked_add(10))
                .unwrap_or(usize::MAX);
            if idx < terrain.tiles.len() {
                terrain.tiles[idx] = crate::layer1::terrain::TerrainType::Rock;
            }
        }

        let pop = world
            .spawn((
                PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);

        world.resource_mut::<Input>().press(KeyCode::W);
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);

        let shake = world.resource::<ScreenShake>();
        assert!(
            shake.intensity > 0.0,
            "Screen shake should trigger on collision"
        );

        let pos = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos.y, 10, "Should not move into rock");
    }

    #[test]
    fn test_direct_movement_juice_particles() {
        let mut world = setup_world();
        // All grass
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            terrain
                .tiles
                .fill(crate::layer1::terrain::TerrainType::Grass);
        }

        let _pop = world
            .spawn((
                PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);

        world.resource_mut::<Input>().press(KeyCode::W);
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);

        // Check for particle at OLD position (10, 10)
        let particle_count = world.query::<&Particle>().iter(&world).count();
        assert!(particle_count > 0, "Should spawn particle on move");
    }

    #[test]
    fn test_ai_components_cleared_on_possession() {
        let mut world = setup_world();
        let pop = world
            .spawn((
                PopBundle::random(0, 0, &mut rand::thread_rng()),
                MovementTarget {
                    target_entity: Entity::from_raw(999),
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Idle,
                },
            ))
            .id();

        // Possess
        world
            .resource_mut::<Events<PossessEntityEvent>>()
            .send(PossessEntityEvent(pop));

        let mut schedule = Schedule::default();
        schedule.add_systems(handle_possession);
        schedule.run(&mut world);

        // Assert Possessed
        assert!(world.entity(pop).contains::<Possessed>());
        // Assert MovementTarget removed
        assert!(!world.entity(pop).contains::<MovementTarget>());
    }

    #[test]
    fn test_possession_buffs() {
        let mut world = setup_world();
        let pop = world
            .spawn(PopBundle::random(0, 0, &mut rand::thread_rng()))
            .id();
        let initial_speed = world.entity(pop).get::<Speed>().unwrap().base;

        // Possess
        world
            .resource_mut::<Events<PossessEntityEvent>>()
            .send(PossessEntityEvent(pop));

        let mut schedule = Schedule::default();
        schedule.add_systems((handle_possession, apply_buffs.after(handle_possession)));
        schedule.run(&mut world);

        let new_speed = world.entity(pop).get::<Speed>().unwrap().base;
        assert!(new_speed > initial_speed, "Speed should increase");

        // Unpossess
        world
            .resource_mut::<Events<UnpossessEvent>>()
            .send(UnpossessEvent);
        schedule.run(&mut world);

        let restored_speed = world.entity(pop).get::<Speed>().unwrap().base;
        assert_eq!(restored_speed, initial_speed, "Speed should be restored");
    }
    #[test]
    fn test_direct_movement_wall_sliding() {
        let mut world = setup_world();
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            terrain
                .tiles
                .fill(crate::layer1::terrain::TerrainType::Grass);
            let width = terrain.width;
            terrain.tiles[11 * width + 11] = crate::layer1::terrain::TerrainType::Rock;
            terrain.tiles[10 * width + 11] = crate::layer1::terrain::TerrainType::Rock;
        }
        let pop = world
            .spawn((
                crate::layer1::pop::PopBundle::random(10, 10, &mut rand::thread_rng()),
                Possessed,
                DirectControlState::default(),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_direct_movement);
        world.resource_mut::<Input>().press(KeyCode::S);
        world.resource_mut::<Input>().press(KeyCode::D);
        world.resource_mut::<WallTime>().0 = 10.0;
        schedule.run(&mut world);
        let pos = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos.x, 10);
        assert_eq!(pos.y, 11, "Should slide along Y axis when X is blocked");
        world.entity_mut(pop).insert(GridPosition { x: 10, y: 10 });
        world
            .entity_mut(pop)
            .get_mut::<DirectControlState>()
            .unwrap()
            .last_move_time = 0.0;
        if let Some(mut terrain) = world.get_resource_mut::<TerrainGrid>() {
            let width = terrain.width;
            terrain.tiles[11 * width + 10] = crate::layer1::terrain::TerrainType::Rock;
            terrain.tiles[10 * width + 11] = crate::layer1::terrain::TerrainType::Grass;
        }
        world.resource_mut::<Input>().clear();
        world.resource_mut::<Input>().press(KeyCode::S);
        world.resource_mut::<Input>().press(KeyCode::D);
        world.resource_mut::<WallTime>().0 = 20.0;
        schedule.run(&mut world);
        let pos2 = world.entity(pop).get::<GridPosition>().unwrap();
        assert_eq!(pos2.x, 11, "Should slide along X axis when Y is blocked");
        assert_eq!(pos2.y, 10);
    }
}
