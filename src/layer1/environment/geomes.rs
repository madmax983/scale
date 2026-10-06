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
pub fn diffuse_geome_hazards_system(
    mut commands: Commands,
    grid: Res<TerrainGrid>,
    hazard_query: Query<&GridPosition, With<GeomeHazard>>,
) {
    // Collect existing hazard positions to avoid spawning multiple hazards on the same tile
    let mut existing_hazards = HashSet::new();
    for pos in hazard_query.iter() {
        existing_hazards.insert(*pos);
    }

    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let pos = GridPosition { x, y };
            if let Some(terrain) = grid.get(x as usize, y as usize) {
                // If this is a SporeBloom tile, it should spawn a hazard
                if terrain == TerrainType::SporeBloom && !existing_hazards.contains(&pos) {
                    commands.spawn((
                        GeomeHazard {
                            damage_per_tick: 5.0,
                        },
                        pos,
                    ));
                    existing_hazards.insert(pos);
                }

                // If this is a MagmaRock tile, it should spawn a hazard
                if terrain == TerrainType::MagmaRock && !existing_hazards.contains(&pos) {
                    commands.spawn((
                        GeomeHazard {
                            damage_per_tick: 10.0,
                        },
                        pos,
                    ));
                    existing_hazards.insert(pos);
                }
            }
        }
    }
}

/// Applies damage to entities with Health that are standing on GeomeHazards.
#[allow(clippy::type_complexity)]
pub fn environmental_damage_system(
    mut health_query: Query<
        (&GridPosition, &mut crate::layer1::health::Health),
        (
            Without<crate::layer1::hive_mind_integration::IntegratedCollective>,
            // The phased Chronostalker walks between moments: nothing
            // in the stream can touch it.
            Without<crate::layer1::culture::chronostalker::Phased>,
        ),
    >,
    hazard_query: Query<(&GridPosition, &GeomeHazard)>,
) {
    let mut hazard_map = HashMap::new();
    for (pos, hazard) in hazard_query.iter() {
        *hazard_map.entry(*pos).or_insert(0.0) += hazard.damage_per_tick;
    }

    for (health_pos, mut health) in health_query.iter_mut() {
        if let Some(damage) = hazard_map.get(health_pos) {
            health.take_damage(*damage);
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
}
