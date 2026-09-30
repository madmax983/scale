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

    if is_mother_lode {
        process_mother_lode(world, entity, effective_work, pos);
    } else {
        process_normal_mining(world, entity, effective_work, pos);
    }

    if let Some(p) = pos {
        handle_mining_visuals(world, entity, p, is_crit);
    }

    true
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
