use bevy_ecs::prelude::*;
use rand::Rng;
use ratatui::style::Color;

use crate::layer1::biology::health::Health;
use crate::layer1::economy::inventory::Inventory;
use crate::layer1::economy::items::ItemType;
use crate::layer1::environment::orbital_crossfire::{mine_scrap, ImpactSite};
use crate::layer1::map::{GridPosition, ScreenShake};
use crate::layer1::mother_lode::MotherLode;
use crate::layer1::particles::{spawn_moving_particle, spawn_particle};
use crate::layer1::purity::PurityMap;
use crate::layer1::resources::{process_logging, process_mining};
use crate::layer1::skills::{SkillType, XpGainEvent, XpSource};
use crate::shared::log::MessageLog;

use crate::layer1::execution::constants::{WORK_CRIT_CHANCE, WORK_CRIT_MULTIPLIER};

/// Handles mining work at a designation.
///
/// Reduces terrain health or mining progress, spawns resources, and removes rock/ore.
pub fn handle_mining_work(
    world: &mut World,
    entity: Entity,
    worker_entity: Entity, // Added worker entity to attribute XP
    work_amount: f32,
    pos: Option<GridPosition>,
) -> bool {
    let is_crit = rand::thread_rng().gen_bool(WORK_CRIT_CHANCE);

    let is_resonant = world
        .get::<crate::layer1::whispering_ore::ResonantTrait>(worker_entity)
        .is_some();
    let resonance_multiplier = if is_resonant { 2.0 } else { 1.0 };

    let mut effective_work = if is_crit {
        let work = work_amount * WORK_CRIT_MULTIPLIER;
        if let Some(p) = pos {
            spawn_particle(world, p, '*', Color::Yellow, 10);
            trigger_shake(world, 0.3);
        }
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add_colored("Critical Mine!", Color::Yellow);
        }
        work
    } else {
        work_amount
    };

    effective_work *= resonance_multiplier;

    // Emit XP event for mining
    if let Some(mut events) = world.get_resource_mut::<Events<XpGainEvent>>() {
        events.send(XpGainEvent {
            entity: worker_entity,
            skill: SkillType::Mining,
            amount: 5.0, // Base amount per tick of work
            source: XpSource::Action,
        });
    }

    if let Some(p) = pos {
        crate::layer1::geology::add_seismic_stress(world, p, 1.0);
    }

    let is_mother_lode = world.get::<MotherLode>(entity).is_some();
    if let Some(p) = pos {
        let is_low_purity = world
            .get_resource::<PurityMap>()
            .is_some_and(|map| map.get(p.x, p.y) < 0.5);
        if is_low_purity && !is_mother_lode {
            let mut apply_rust_lung = false;

            if let Some(inventory) = world.get::<Inventory>(worker_entity) {
                if !inventory.has_item(ItemType::Rebreather) {
                    apply_rust_lung = true;
                }
            } else {
                apply_rust_lung = true;
            }

            if apply_rust_lung {
                if let Some(mut health) = world.get_mut::<Health>(worker_entity) {
                    health.has_rust_lung = true;
                }
            }
        }
    }

    // Item 205: a Mine designation on a standing shipbreaking hull
    // tile routes through the shipbreaking MineEvent seam (below),
    // not the Rock-only generic path.
    let hull_target = pos.and_then(|p| hull_entity_at(world, p));
    if is_mother_lode {
        process_mother_lode(world, entity, effective_work, pos);
    } else if let Some(hull) = hull_target {
        process_hull_mining(world, entity, worker_entity, hull);
    } else {
        process_normal_mining(world, entity, effective_work, pos);
    }

    if let Some(p) = pos {
        handle_mining_visuals(world, entity, p, is_crit);
    }

    true
}

/// Finds the standing hull tile entity at `pos`, if any.
fn hull_entity_at(world: &mut World, pos: GridPosition) -> Option<Entity> {
    let mut query = world
        .query_filtered::<(Entity, &GridPosition), With<crate::layer1::shipbreaking::HullTile>>();
    query
        .iter(world)
        .find(|(_, p)| **p == pos)
        .map(|(e, _)| e)
}

/// Item 205: designated hull work, routed through the existing
/// shipbreaking seam — no parallel damage/yield system.
///
/// One work tick from a qualified worker sends one
/// [`crate::layer1::shipbreaking::MineEvent`]; `mine_system` applies
/// the tier-3 gate and the fixed 10 dmg/hit, and
/// `hull_destroyed_system` drops the Metal/AncientTech yields and
/// frees the tile's `OccupiedTiles` entry when the hull breaks. An
/// unqualified worker's events are sent but deal no damage (the
/// gate lives in `mine_system`), so the designation stays open
/// while the worker accrues Mining XP toward qualifying.
///
/// Completion: when this hit will break the hull (its health is at
/// or below one hit of damage), the designation is consumed here;
/// the hull itself despawns through `hull_destroyed_system` on the
/// next schedule pass, exactly as with the headless `shipbreak`
/// command.
fn process_hull_mining(
    world: &mut World,
    designation_entity: Entity,
    worker_entity: Entity,
    hull_entity: Entity,
) {
    use crate::layer1::shipbreaking::{MineEvent, HULL_MINE_DAMAGE};
    let tier = crate::layer1::shipbreaking::effective_tool_tier_world(world, worker_entity);
    let required = world
        .get::<crate::layer1::shipbreaking::Mineable>(hull_entity)
        .map_or(crate::layer1::shipbreaking::HULL_REQUIRED_TOOL_TIER, |m| {
            m.required_tool_tier
        });
    if let Some(mut events) = world.get_resource_mut::<Events<MineEvent>>() {
        events.send(MineEvent { miner: worker_entity, target: hull_entity });
    }
    if tier >= required {
        let health = world.get::<Health>(hull_entity).map_or(f32::MAX, |h| h.current);
        if health <= HULL_MINE_DAMAGE && world.get_entity(designation_entity).is_ok() {
            world.despawn(designation_entity);
        }
    }
}

fn process_mother_lode(
    world: &mut World,
    entity: Entity,
    effective_work: f32,
    pos: Option<GridPosition>,
) {
    if let Some(mut lode) = world.get_mut::<MotherLode>(entity) {
        lode.increment_hazard();
    }

    if world
        .get::<crate::layer1::resources::MiningProgress>(entity)
        .is_none()
    {
        world
            .entity_mut(entity)
            .insert(crate::layer1::resources::MiningProgress {
                current: 0.0,
                max: 20.0,
            });
    }

    let completed = if let Some(mut progress) =
        world.get_mut::<crate::layer1::resources::MiningProgress>(entity)
    {
        progress.current += effective_work;
        if progress.current >= progress.max {
            progress.current = 0.0;
            true
        } else {
            false
        }
    } else {
        false
    };

    if completed {
        if let Some(lode) = world.get::<MotherLode>(entity) {
            let res_type = lode.resource_type;
            if let Some(p) = pos {
                world.spawn((
                    crate::layer1::resources::ResourceItem {
                        resource_type: res_type,
                        amount: 1.0,
                    },
                    p,
                ));
                if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
                    log.add(format!("Mined {:?} from Mother Lode", res_type));
                }
            }
        }
    }
}

fn process_normal_mining(
    world: &mut World,
    entity: Entity,
    effective_work: f32,
    pos: Option<GridPosition>,
) {
    let is_scrap = if let Some(p) = pos {
        let mut found = false;
        let mut query = world.query::<(&GridPosition, &ImpactSite)>();
        for (gp, _) in query.iter(world) {
            if *gp == p {
                found = true;
                break;
            }
        }
        found
    } else {
        false
    };

    if is_scrap {
        mine_scrap(world, entity, effective_work);
    } else {
        process_mining(world, entity, effective_work);
    }
}

fn handle_mining_visuals(world: &mut World, entity: Entity, pos: GridPosition, is_crit: bool) {
    let mut rng = rand::thread_rng();
    if world.get_entity(entity).is_err() {
        // Finished: Big shake + Debris
        trigger_shake(world, 0.5);
        spawn_particle(world, pos, '*', Color::White, 10);

        for _ in 0..5 {
            let dx = rng.gen_range(-1.0..1.0);
            let dy = rng.gen_range(-1.0..1.0);
            spawn_moving_particle(world, pos, '.', Color::DarkGray, 15, dx, dy);
        }
    } else if !is_crit {
        // Working: occasional dust puff only — no per-tick shake. Screen shake is
        // event-driven (finish/crit/combat/explosions); triggering it every tick
        // while work is in progress saturates the shake resource at its cap and
        // turns the game into a permanent earthquake simulator.
        if rng.gen_bool(0.3) {
            spawn_particle(world, pos, '.', Color::DarkGray, 3);
        }

        if rng.gen_bool(0.3) {
            let dx = rng.gen_range(-0.5..0.5);
            let dy = rng.gen_range(-0.5..0.5);
            spawn_moving_particle(world, pos, '.', Color::Gray, 10, dx, dy);
        }
    }
}

/// Handles wood chopping work at a designation.
///
/// Reduces tree HP, spawns wood, and removes the tree entity.
pub fn handle_chopping_work(
    world: &mut World,
    entity: Entity,
    worker_entity: Entity, // Added worker entity
    work_amount: f32,
    pos: Option<GridPosition>,
) -> bool {
    let mut rng = rand::thread_rng();
    let is_crit = rng.gen_bool(WORK_CRIT_CHANCE);

    let mut effective_work = work_amount;
    if is_crit {
        effective_work *= WORK_CRIT_MULTIPLIER;
        if let Some(p) = pos {
            spawn_particle(world, p, '^', Color::LightGreen, 10);
            trigger_shake(world, 0.3);
        }
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add_colored("Critical Chop!", Color::LightGreen);
        }
    }

    emit_forestry_xp(world, worker_entity);
    process_logging(world, entity, effective_work);

    if let Some(p) = pos {
        handle_chopping_visuals(world, entity, p, is_crit);
    }
    true
}

fn emit_forestry_xp(world: &mut World, worker_entity: Entity) {
    if let Some(mut events) = world.get_resource_mut::<Events<XpGainEvent>>() {
        events.send(XpGainEvent {
            entity: worker_entity,
            skill: SkillType::Forestry,
            amount: 5.0,
            source: XpSource::Action,
        });
    }
}

fn handle_chopping_visuals(world: &mut World, entity: Entity, pos: GridPosition, is_crit: bool) {
    let mut rng = rand::thread_rng();
    if world.get_entity(entity).is_err() {
        // Finished
        trigger_shake(world, 0.3);
        spawn_particle(world, pos, '^', Color::Green, 10);

        // Ludwig: Wood chips flying
        for _ in 0..4 {
            let dx = rng.gen_range(-0.8..0.8);
            let dy = rng.gen_range(-0.8..0.8);
            spawn_moving_particle(world, pos, '\'', Color::Rgb(139, 69, 19), 15, dx, dy);
        }
    } else if !is_crit {
        // Working: occasional wood chip only — no per-tick shake (see above).
        if rng.gen_bool(0.3) {
            spawn_particle(world, pos, '\'', Color::Rgb(139, 69, 19), 3);
        }

        // Ludwig: Occasional flying chip
        if rng.gen_bool(0.3) {
            let dx = rng.gen_range(-0.5..0.5);
            let dy = rng.gen_range(-0.5..0.5);
            spawn_moving_particle(world, pos, '\'', Color::Rgb(160, 82, 45), 10, dx, dy);
        }
    }
}

fn trigger_shake(world: &mut World, intensity: f32) {
    if let Some(mut shake) = world.get_resource_mut::<ScreenShake>() {
        shake.trigger(intensity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::mother_lode::MotherLode;
    use crate::layer1::resources::ResourceType;

    #[test]
    fn test_process_mother_lode() {
        let mut world = World::new();
        let lode_entity = world
            .spawn(MotherLode {
                resource_type: ResourceType::Stone,
                current_hazard: 1.0,
                heat_output: 10.0,
            })
            .id();

        // Run process once, it should initialize progress
        process_mother_lode(&mut world, lode_entity, 10.0, None);

        let progress = world
            .get::<crate::layer1::resources::MiningProgress>(lode_entity)
            .unwrap();
        assert_eq!(progress.current, 10.0);
        let lode = world.get::<MotherLode>(lode_entity).unwrap();
        assert!(lode.current_hazard > 1.0);
        assert!(lode.heat_output > 10.0);

        // Run again to complete the progress and spawn the resource
        process_mother_lode(
            &mut world,
            lode_entity,
            15.0,
            Some(GridPosition { x: 5, y: 5 }),
        );

        let progress2 = world
            .get::<crate::layer1::resources::MiningProgress>(lode_entity)
            .unwrap();
        assert_eq!(progress2.current, 0.0);

        // Verify resource spawned
        let resources: Vec<_> = world
            .query::<&crate::layer1::resources::ResourceItem>()
            .iter(&world)
            .collect();
        assert!(!resources.is_empty());
        assert_eq!(resources[0].resource_type, ResourceType::Stone);
    }

    // Item 205 RED: a Mine designation on a hull tile must complete
    // autonomously through handle_mining_work -> shipbreaking
    // MineEvent/mine_system (tier-3 gate, 10 dmg/hit, yields,
    // occupancy freed). Pre-205 the generic executor only completed
    // on Rock terrain, so designated hull mining never finished.
    fn hull_work_world() -> World {
        let mut world = World::new();
        world.insert_resource(crate::layer1::terrain::TerrainGrid {
            width: 20,
            height: 20,
            tiles: vec![crate::layer1::terrain::TerrainType::Grass; 400],
        });
        world.insert_resource(crate::layer1::building::OccupiedTiles::default());
        world.init_resource::<Events<crate::layer1::shipbreaking::MineEvent>>();
        world
    }

    fn spawn_hull_designation(world: &mut World) -> (Entity, Entity) {
        use crate::layer1::shipbreaking::{
            HullTile, Mineable, YieldsOnMine, HULL_REQUIRED_TOOL_TIER, HULL_TILE_HEALTH,
        };
        let hull = world
            .spawn((
                HullTile,
                Mineable { required_tool_tier: HULL_REQUIRED_TOOL_TIER },
                Health { current: HULL_TILE_HEALTH, max: HULL_TILE_HEALTH, has_rust_lung: false },
                YieldsOnMine(crate::layer1::shipbreaking::default_hull_yields()),
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        world
            .resource_mut::<crate::layer1::building::OccupiedTiles>()
            .0
            .insert((5, 5));
        let designation = world
            .spawn((
                crate::layer1::designation::Designation {
                    designation_type: crate::layer1::designation::DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        (hull, designation)
    }

    /// The live game runs ONE persistent schedule, whose EventReader
    /// cursors consume each MineEvent exactly once. Tests must do
    /// the same: rebuilding the schedule per run re-reads buffered
    /// events and multiplies damage.
    fn ship_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems((
            crate::layer1::shipbreaking::mine_system,
            crate::layer1::shipbreaking::hull_destroyed_system,
        ).chain());
        schedule
    }

    fn run_shipbreaking(world: &mut World) {
        ship_schedule().run(world);
    }

    #[test]
    fn test_designated_hull_mined_to_completion_by_qualified_worker() {
        let mut world = hull_work_world();
        let (hull, designation) = spawn_hull_designation(&mut world);
        let worker = world
            .spawn(crate::layer1::shipbreaking::Miner { tool_tier: 3 })
            .id();
        let pos = Some(GridPosition { x: 5, y: 5 });
        let mut schedule = ship_schedule();
        for _ in 0..12 {
            if world.get_entity(designation).is_err() && world.get_entity(hull).is_err() {
                break;
            }
            handle_mining_work(&mut world, designation, worker, 10.0, pos);
            schedule.run(&mut world);
        }
        assert!(world.get_entity(hull).is_err(), "designated hull must be mined to destruction");
        assert!(
            world.get_entity(designation).is_err(),
            "designation must complete (despawn) when the hull breaks"
        );
        let drops: Vec<(ResourceType, f32)> = world
            .query::<&crate::layer1::resources::ResourceItem>()
            .iter(&world)
            .map(|r| (r.resource_type, r.amount))
            .collect();
        assert!(
            drops.contains(&(ResourceType::Metal, 5.0)),
            "hull yields Metal x5, got {drops:?}"
        );
        assert!(
            drops.contains(&(ResourceType::AncientTech, 1.0)),
            "hull yields AncientTech x1, got {drops:?}"
        );
        assert!(
            !world.resource::<crate::layer1::building::OccupiedTiles>().0.contains(&(5, 5)),
            "broken hull frees its OccupiedTiles entry"
        );
    }

    #[test]
    fn test_designated_hull_resists_unqualified_worker() {
        let mut world = hull_work_world();
        let (hull, designation) = spawn_hull_designation(&mut world);
        let worker = world
            .spawn(crate::layer1::shipbreaking::Miner { tool_tier: 1 })
            .id();
        let pos = Some(GridPosition { x: 5, y: 5 });
        for _ in 0..5 {
            handle_mining_work(&mut world, designation, worker, 10.0, pos);
            run_shipbreaking(&mut world);
        }
        let health = world.get::<Health>(hull).expect("hull still standing");
        assert_eq!(health.current, 100.0, "tier-1 worker must not dent hull (tier-3 gate)");
        assert!(world.get_entity(designation).is_ok(), "designation stays open while hull stands");
    }

    #[test]
    fn test_designated_hull_mined_by_real_pop_tier() {
        // No Miner seam: pickaxe + Mining level 2 (400 XP) derives tier 3.
        let mut world = hull_work_world();
        let (hull, designation) = spawn_hull_designation(&mut world);
        let pick = world
            .spawn(crate::layer1::items::Tool {
                tool_type: crate::layer1::items::ToolType::Pickaxe,
                durability: 100.0,
                max_durability: 100.0,
            })
            .id();
        let mut skills = crate::layer1::skills::Skills::default();
        skills.xp.insert(crate::layer1::skills::SkillType::Mining, 400.0);
        let worker = world
            .spawn((
                crate::layer1::items::Equipment { tool: Some(pick), ..Default::default() },
                skills,
            ))
            .id();
        handle_mining_work(&mut world, designation, worker, 10.0, Some(GridPosition { x: 5, y: 5 }));
        run_shipbreaking(&mut world);
        let health = world.get::<Health>(hull).expect("hull still standing after one hit");
        assert_eq!(health.current, 90.0, "one qualified hit deals HULL_MINE_DAMAGE (10)");
    }

    #[test]
    fn test_working_ticks_do_not_shake_screen() {
        // Regression test for the "earthquake simulator" bug: ordinary mining
        // and chopping work ticks used to call trigger_shake() every tick.
        // ScreenShake only decays by 0.9 per update, so continuous work
        // saturated it at its 5.0 cap and the viewport never stopped shaking.
        // Working ticks must not touch ScreenShake at all; shake is reserved
        // for discrete events (completion, crits, combat, explosions).
        let mut world = World::new();
        world.insert_resource(ScreenShake::default());
        let worker = world.spawn_empty().id();
        let pos = GridPosition { x: 10, y: 10 };
        for _ in 0..200 {
            handle_mining_visuals(&mut world, worker, pos, false);
            handle_chopping_visuals(&mut world, worker, pos, false);
        }
        let shake = world.resource::<ScreenShake>();
        assert_eq!(
            shake.intensity, 0.0,
            "ordinary work ticks must not trigger screen shake"
        );
    }
}
