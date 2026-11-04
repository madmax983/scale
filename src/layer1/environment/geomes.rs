use crate::layer1::map::GridPosition;
use crate::layer1::nature::terrain::{TerrainGrid, TerrainType};
use bevy::utils::{HashMap, HashSet};
use bevy_ecs::prelude::*;

/// A hazard present on a specific tile (e.g., Toxic Spores, Extreme Heat).
#[derive(Component, Debug, Clone, Copy)]
pub struct GeomeHazard {
    pub damage_per_tick: f32,
}

/// Diffuses hazards from breached geome tiles into adjacent empty/walkable tiles.
///
/// ⚡ Bolt Optimization:
/// - Replaced `std::collections::{HashMap, HashSet}` with `bevy::utils::{HashMap, HashSet}` (AHash).
/// - Using AHash eliminates SipHash overhead on `GridPosition` keys, reducing lookup/insertion times.
/// The hazard a terrain type sustains, if any: SporeBloom seeps spore
/// damage, MagmaRock radiates heat damage. Any other terrain sustains no
/// `GeomeHazard` — a hazard entity left on such a tile is stale.
#[must_use]
pub fn hazard_damage_for_terrain(terrain: TerrainType) -> Option<f32> {
    match terrain {
        TerrainType::SporeBloom => Some(5.0),
        TerrainType::MagmaRock => Some(10.0),
        _ => None,
    }
}

pub fn diffuse_geome_hazards_system(
    mut commands: Commands,
    grid: Res<TerrainGrid>,
    hazard_query: Query<(Entity, &GridPosition, &GeomeHazard)>,
) {
    // Collect existing hazard positions to avoid spawning multiple hazards
    // on the same tile. Hazards whose tile no longer sustains them (the
    // terrain changed off SporeBloom/MagmaRock — e.g. a quarantine burn
    // scorched the tile to Ash, item 165) are despawned here: before this
    // cleanup they were immortal, so burned tiles kept dealing damage.
    let mut existing_hazards = HashSet::new();
    for (entity, pos, hazard) in hazard_query.iter() {
        let sustained = usize::try_from(pos.x)
            .ok()
            .zip(usize::try_from(pos.y).ok())
            .and_then(|(x, y)| grid.get(x, y))
            .and_then(hazard_damage_for_terrain)
            .is_some_and(|damage| (damage - hazard.damage_per_tick).abs() < f32::EPSILON);
        if sustained {
            existing_hazards.insert(*pos);
        } else {
            commands.entity(entity).despawn();
        }
    }

    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let pos = GridPosition { x, y };
            if let Some(terrain) = grid.get(x as usize, y as usize) {
                // Only terrain that sustains a hazard spawns one — a tile
                // scorched off SporeBloom (or any other terrain change)
                // must never (re)gain a hazard here.
                if let Some(damage_per_tick) = hazard_damage_for_terrain(terrain) {
                    if !existing_hazards.contains(&pos) {
                        commands.spawn((GeomeHazard { damage_per_tick }, pos));
                        existing_hazards.insert(pos);
                    }
                }
            }
        }
    }
}

/// Applies damage to entities with Health that are standing on GeomeHazards.
#[allow(clippy::type_complexity)]
pub fn environmental_damage_system(
    mut commands: Commands,
    mut health_query: Query<
        (Entity, &GridPosition, &mut crate::layer1::health::Health),
        (
            Without<crate::layer1::hive_mind_integration::IntegratedCollective>,
            // The phased Chronostalker walks between moments: nothing
            // in the stream can touch it.
            Without<crate::layer1::culture::chronostalker::Phased>,
            // Item 167: dormant origin pops are in stasis at their
            // sites (see pressure_damage_system / thermal_damage_system,
            // items 100/101) — they are excluded from utility AI and
            // tethered in place, so they can never step off a hazard
            // tile; environmental damage cannot touch them until they
            // wake and the dormant marker is removed.
            Without<crate::layer1::culture::lawbound::DormantAutomaton>,
            Without<crate::layer1::culture::bloomtouched::DormantTouched>,
            Without<crate::layer1::culture::chronostalker::DormantStalker>,
        ),
    >,
    hazard_query: Query<(&GridPosition, &GeomeHazard)>,
    terrain: Option<Res<TerrainGrid>>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    let mut hazard_map = HashMap::new();
    for (pos, hazard) in hazard_query.iter() {
        *hazard_map.entry(*pos).or_insert(0.0) += hazard.damage_per_tick;
    }

    for (entity, health_pos, mut health) in health_query.iter_mut() {
        if let Some(damage) = hazard_map.get(health_pos) {
            health.take_damage(*damage);
            // Item 84: name the terrain hazard that dealt the damage.
            let cause = match terrain.as_deref().and_then(|grid| {
                grid.get(
                    usize::try_from(health_pos.x).ok()?,
                    usize::try_from(health_pos.y).ok()?,
                )
            }) {
                Some(TerrainType::MagmaRock) => {
                    crate::layer1::biology::death_attribution::CAUSE_MAGMA
                }
                Some(TerrainType::SporeBloom) => {
                    crate::layer1::biology::death_attribution::CAUSE_SPORE_BLOOM
                }
                _ => crate::layer1::biology::death_attribution::CAUSE_HAZARDOUS_TERRAIN,
            };
            crate::layer1::biology::death_attribution::record_damage(
                &mut commands,
                entity,
                cause,
                tick,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::health::Health;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_breaching_geome_triggers_hazard() {
        let mut world = World::new();

        let mut grid = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Rock; 100],
        };
        // Setup a SporeBloom tile
        grid.set(6, 5, TerrainType::SporeBloom);
        world.insert_resource(grid);

        let miner = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 6, y: 5 }, // Standing on the SporeBloom tile
            ))
            .id();

        // Run hazard diffusion system to spawn the hazard entity
        let mut hazard_schedule = Schedule::default();
        hazard_schedule.add_systems(diffuse_geome_hazards_system);
        hazard_schedule.run(&mut world);

        // Run environmental damage system to apply damage
        let mut damage_schedule = Schedule::default();
        damage_schedule.add_systems(environmental_damage_system);
        damage_schedule.run(&mut world);

        // The miner should have taken damage from the Spore hazard
        let health = world.get::<Health>(miner).unwrap();
        assert!(
            health.current < 100.0,
            "Miner should take damage from breached geome hazard"
        );
    }

    fn hazard_count(world: &mut World) -> usize {
        world.query::<&GeomeHazard>().iter(world).count()
    }

    #[test]
    fn test_hazard_despawns_when_terrain_changes_off_spore_bloom() {
        // Item 165: a burned (or otherwise changed) tile must stop being
        // hazardous — GeomeHazard entities used to be immortal.
        let mut world = World::new();
        let mut grid = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Rock; 100],
        };
        grid.set(6, 5, TerrainType::SporeBloom);
        grid.set(2, 2, TerrainType::MagmaRock);
        world.insert_resource(grid);

        let mut schedule = Schedule::default();
        schedule.add_systems(diffuse_geome_hazards_system);
        schedule.run(&mut world);
        assert_eq!(hazard_count(&mut world), 2, "both hazard tiles spawn");

        // Terrain changes off SporeBloom (quarantine burn -> Ash).
        world
            .resource_mut::<TerrainGrid>()
            .set(6, 5, TerrainType::Ash);
        schedule.run(&mut world);
        assert_eq!(
            hazard_count(&mut world),
            1,
            "the Ash tile's hazard must despawn; the MagmaRock hazard stays"
        );

        // And it must not respawn on later ticks.
        schedule.run(&mut world);
        assert_eq!(hazard_count(&mut world), 1, "no respawn on Ash");
    }

    #[test]
    fn test_no_damage_on_scorched_tile() {
        let mut world = World::new();
        let mut grid = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Rock; 100],
        };
        grid.set(6, 5, TerrainType::SporeBloom);
        world.insert_resource(grid);
        let miner = world
            .spawn((
                Pop,
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                GridPosition { x: 6, y: 5 },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((
            diffuse_geome_hazards_system,
            environmental_damage_system.after(diffuse_geome_hazards_system),
        ));
        schedule.run(&mut world);
        let hurt = world.get::<Health>(miner).unwrap().current;
        assert!(hurt < 100.0, "standing on bloom hurts before the burn");

        // Scorch the tile (what quarantine_burn_system does) and re-run.
        world
            .resource_mut::<TerrainGrid>()
            .set(6, 5, TerrainType::Ash);
        schedule.run(&mut world);
        assert_eq!(
            world.get::<Health>(miner).unwrap().current,
            hurt,
            "a scorched tile must deal no further environmental damage"
        );
    }

    fn dormant_world_with_hazard() -> World {
        let mut world = World::new();
        world.spawn((
            GridPosition { x: 5, y: 5 },
            GeomeHazard {
                damage_per_tick: 5.0,
            },
        ));
        world
    }

    fn run_environmental_damage(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(environmental_damage_system);
        schedule.run(world);
    }

    fn mk_health() -> Health {
        Health {
            current: 100.0,
            max: 100.0,
            has_rust_lung: false,
        }
    }

    #[test]
    fn test_dormant_origin_pops_take_no_environmental_damage() {
        // Item 167 (item 160 sub-item 4): dormant origin pops are in
        // stasis at their sites — the same exemption items 100/101 added
        // to pressure and thermal damage must cover environmental
        // (geome hazard) damage. Proof case: Quill, the dormant
        // Bloom-Touched scout, died standing on a hazard tile.
        // Uses the diffuse-from-terrain path: SporeBloom tiles seep a
        // GeomeHazard, then environmental_damage_system applies it.
        let mut world = World::new();
        let mut grid = TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Rock; 100],
        };
        for x in 1..=4 {
            grid.set(x, 5, TerrainType::SporeBloom);
        }
        world.insert_resource(grid);

        let vigil = world
            .spawn((
                mk_health(),
                GridPosition { x: 1, y: 5 },
                crate::layer1::culture::lawbound::DormantAutomaton,
            ))
            .id();
        let quill = world
            .spawn((
                mk_health(),
                GridPosition { x: 2, y: 5 },
                crate::layer1::culture::bloomtouched::DormantTouched,
            ))
            .id();
        let tock = world
            .spawn((
                mk_health(),
                GridPosition { x: 3, y: 5 },
                crate::layer1::culture::chronostalker::DormantStalker,
            ))
            .id();
        let control = world
            .spawn((mk_health(), GridPosition { x: 4, y: 5 }))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems((
            diffuse_geome_hazards_system,
            environmental_damage_system.after(diffuse_geome_hazards_system),
        ));
        schedule.run(&mut world);

        for dormant in [vigil, quill, tock] {
            assert_eq!(
                world.get::<Health>(dormant).unwrap().current,
                100.0,
                "dormant origin pops are in stasis — hazard damage cannot touch them"
            );
        }
        assert!(
            world.get::<Health>(control).unwrap().current < 100.0,
            "a non-dormant pop on the same hazard terrain still takes damage"
        );
    }

    #[test]
    fn test_dormant_environmental_damage_resumes_after_waking() {
        // Item 167: the exemption is tied to the dormant marker — once
        // the marker is removed (the pop wakes / takes its path),
        // hazard damage applies like anyone else. Uses directly
        // spawned GeomeHazards, following the chronostalker
        // red_phased_cannot_be_hurt test pattern.
        // DormantAutomaton
        {
            let mut world = dormant_world_with_hazard();
            let e = world
                .spawn((
                    mk_health(),
                    GridPosition { x: 5, y: 5 },
                    crate::layer1::culture::lawbound::DormantAutomaton,
                ))
                .id();
            run_environmental_damage(&mut world);
            assert_eq!(world.get::<Health>(e).unwrap().current, 100.0);
            world
                .entity_mut(e)
                .remove::<crate::layer1::culture::lawbound::DormantAutomaton>();
            run_environmental_damage(&mut world);
            assert_eq!(
                world.get::<Health>(e).unwrap().current,
                95.0,
                "awakened automaton takes hazard damage again"
            );
        }
        // DormantTouched
        {
            let mut world = dormant_world_with_hazard();
            let e = world
                .spawn((
                    mk_health(),
                    GridPosition { x: 5, y: 5 },
                    crate::layer1::culture::bloomtouched::DormantTouched,
                ))
                .id();
            run_environmental_damage(&mut world);
            assert_eq!(world.get::<Health>(e).unwrap().current, 100.0);
            world
                .entity_mut(e)
                .remove::<crate::layer1::culture::bloomtouched::DormantTouched>();
            run_environmental_damage(&mut world);
            assert_eq!(
                world.get::<Health>(e).unwrap().current,
                95.0,
                "awakened touched takes hazard damage again"
            );
        }
        // DormantStalker
        {
            let mut world = dormant_world_with_hazard();
            let e = world
                .spawn((
                    mk_health(),
                    GridPosition { x: 5, y: 5 },
                    crate::layer1::culture::chronostalker::DormantStalker,
                ))
                .id();
            run_environmental_damage(&mut world);
            assert_eq!(world.get::<Health>(e).unwrap().current, 100.0);
            world
                .entity_mut(e)
                .remove::<crate::layer1::culture::chronostalker::DormantStalker>();
            run_environmental_damage(&mut world);
            assert_eq!(
                world.get::<Health>(e).unwrap().current,
                95.0,
                "awakened stalker takes hazard damage again"
            );
        }
    }
}
