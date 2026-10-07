//! # Civilizational Compost (Spec 1370)
//!
//! Dead pops decompose into nutrient-rich soil: unburied corpses enrich the
//! fertility of their tile and haunt it. Farming corpse-fed soil yields more
//! crops but inflicts "Haunted" mood on the farmers. Destroyed buildings leave
//! salvageable scrap heaps. Burial in a Grave prevents decomposition but
//! preserves morale — respect the dead vs. use the dead.

use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::layer1::GridPosition;

/// Ticks for an unburied corpse to fully decompose.
pub const CORPSE_DECAY_TICKS: f32 = 100.0;
/// Fertility bonus added per decomposed corpse.
pub const COMPOST_FERTILITY_BONUS: f32 = 0.5;
/// Maximum enrichment bonus stackable on one tile.
pub const MAX_ENRICHMENT_BONUS: f32 = 1.0;
/// Ticks between haunted-mood applications (once per shift, not per tick).
pub const HAUNTED_MOOD_PERIOD_TICKS: u64 = 100;
/// Mood penalty for working haunted soil.
pub const HAUNTED_MOOD_PENALTY: f32 = -0.15;

/// Tracks compost-enriched and haunted soil tiles.
#[derive(Resource, Default, Debug)]
pub struct CompostSoil {
    /// Bonus fertility per tile from decomposition (added to grid value).
    pub enrichment: HashMap<(i32, i32), f32>,
    /// Tiles haunted by corpse decomposition.
    pub haunted: HashSet<(i32, i32)>,
}

impl CompostSoil {
    /// Bonus fertility at a tile (0.0 if never composted).
    pub fn enrichment_at(&self, x: i32, y: i32) -> f32 {
        self.enrichment.get(&(x, y)).copied().unwrap_or(0.0)
    }

    /// Whether a tile is haunted by decomposition.
    pub fn is_haunted(&self, x: i32, y: i32) -> bool {
        self.haunted.contains(&(x, y))
    }

    /// Record a decomposition at a tile: enrich and haunt it.
    pub fn enrich(&mut self, x: i32, y: i32, amount: f32) {
        let entry = self.enrichment.entry((x, y)).or_insert(0.0);
        *entry = (*entry + amount).min(MAX_ENRICHMENT_BONUS);
        self.haunted.insert((x, y));
    }
}

/// Scrap heap left by a destroyed building, salvageable for materials.
#[derive(Component, Debug, Clone)]
pub struct ScrapHeap {
    /// Salvageable metal remaining.
    pub metal: f32,
    /// Salvageable wood remaining.
    pub wood: f32,
}

/// Advances decay on unburied corpses.
///
/// Corpses buried via [`crate::layer1::funeral::bury_corpse`] are despawned
/// and never reach full decay — that is the intended "respect the dead"
/// counterplay to composting.
pub fn tick_corpse_decay_system(mut corpses: Query<&mut crate::layer1::funeral::Corpse>) {
    for mut corpse in corpses.iter_mut() {
        corpse.decay = (corpse.decay + 1.0 / CORPSE_DECAY_TICKS).min(1.0);
    }
}

/// Decomposes fully-decayed corpses into enriched, haunted soil.
///
/// Runs on unburied corpses only (burial despawns the corpse first).
/// Each decomposition enriches the tile and marks it haunted.
pub fn compost_decomposition_system(
    mut commands: Commands,
    corpses: Query<
        (Entity, &crate::layer1::funeral::Corpse, &GridPosition),
    >,
    mut soil: ResMut<CompostSoil>,
) {
    for (entity, corpse, pos) in corpses.iter() {
        if corpse.decay >= 1.0 {
            soil.enrich(pos.x, pos.y, COMPOST_FERTILITY_BONUS);
            commands.entity(entity).despawn();
        }
    }
}

/// Applies the Haunted mood to farm workers on haunted soil.
///
/// Periodic (once per shift), not per tick, per the spec's guidance —
/// a slow dread, not an instant breakdown.
pub fn apply_haunted_mood_system(
    soil: Res<CompostSoil>,
    mut pops: Query<
        (&mut crate::layer1::morale::Morale, &crate::layer1::pop::Job),
        With<crate::layer1::pop::Pop>,
    >,
    workplaces: Query<&GridPosition>,
    time: Res<crate::shared::time::SimulationTime>,
) {
    if !time.tick.is_multiple_of(HAUNTED_MOOD_PERIOD_TICKS) {
        return;
    }
    for (mut morale, job) in pops.iter_mut() {
        if job.job_type != crate::layer1::actions::AssignmentType::FarmWorker {
            continue;
        }
        let Ok(pos) = workplaces.get(job.workplace) else {
            continue;
        };
        if soil.is_haunted(pos.x, pos.y)
            && !morale.modifiers.iter().any(|m| m.label == "Haunted Soil")
        {
            morale.modifiers.push(crate::layer1::morale::MoodModifier {
                label: "Haunted Soil".to_string(),
                value: HAUNTED_MOOD_PENALTY,
                duration: HAUNTED_MOOD_PERIOD_TICKS as u32,
            });
        }
    }
}

/// Salvages scrap heaps into colony resources via nearby pops.
///
/// Any pop within 2 tiles hauls a chunk per tick; the heap despawns empty.
pub fn salvage_scrap_heap_system(
    mut commands: Commands,
    mut heaps: Query<(Entity, &mut ScrapHeap, &GridPosition)>,
    pops: Query<&GridPosition, With<crate::layer1::pop::Pop>>,
    mut resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
) {
    const SALVAGE_PER_TICK: f32 = 1.0;
    const SALVAGE_RADIUS: i32 = 2;
    for (entity, mut heap, heap_pos) in heaps.iter_mut() {
        let near_pop = pops.iter().any(|pop_pos| {
            (pop_pos.x - heap_pos.x).abs() <= SALVAGE_RADIUS
                && (pop_pos.y - heap_pos.y).abs() <= SALVAGE_RADIUS
        });
        if !near_pop {
            continue;
        }
        let metal_take = heap.metal.min(SALVAGE_PER_TICK);
        let wood_take = heap.wood.min(SALVAGE_PER_TICK);
        heap.metal -= metal_take;
        heap.wood -= wood_take;
        resources.metal += metal_take;
        resources.wood += wood_take;
        if heap.metal <= 0.0 && heap.wood <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::funeral::Corpse;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::{Job, Pop};

    fn corpse_at(x: i32, y: i32, decay: f32) -> (Corpse, GridPosition) {
        (
            Corpse {
                name: "Test Pop".to_string(),
                decay,
            },
            GridPosition { x, y },
        )
    }

    #[test]
    fn test_corpse_decomposes_into_enriched_haunted_soil() {
        let mut world = World::new();
        world.insert_resource(CompostSoil::default());

        // Fully-decayed corpse on a tile
        let corpse = world.spawn(corpse_at(5, 5, 1.0)).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(compost_decomposition_system);
        schedule.run(&mut world);

        // Corpse should be despawned
        assert!(
            world.get_entity(corpse).is_err(),
            "fully-decayed corpse should despawn"
        );

        // Soil should be enriched and haunted
        let soil = world.resource::<CompostSoil>();
        assert!(
            soil.enrichment_at(5, 5) > 0.0,
            "tile should gain fertility from compost"
        );
        assert!(soil.is_haunted(5, 5), "tile should be marked haunted");
    }

    #[test]
    fn test_partial_decay_corpse_does_not_compost() {
        let mut world = World::new();
        world.insert_resource(CompostSoil::default());

        let corpse = world.spawn(corpse_at(5, 5, 0.5)).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(compost_decomposition_system);
        schedule.run(&mut world);

        assert!(
            world.get_entity(corpse).is_ok(),
            "partially-decayed corpse should remain"
        );
        let soil = world.resource::<CompostSoil>();
        assert_eq!(soil.enrichment_at(5, 5), 0.0);
        assert!(!soil.is_haunted(5, 5));
    }

    #[test]
    fn test_corpse_decay_ticks_up() {
        let mut world = World::new();
        let corpse = world.spawn(corpse_at(3, 3, 0.0)).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(tick_corpse_decay_system);
        schedule.run(&mut world);

        let corpse_comp = world.get::<Corpse>(corpse).unwrap();
        assert!(
            corpse_comp.decay > 0.0,
            "corpse decay should advance each tick"
        );
    }

    #[test]
    fn test_farming_haunted_soil_applies_haunted_mood() {
        let mut world = World::new();
        let mut soil = CompostSoil::default();
        soil.enrich(5, 5, COMPOST_FERTILITY_BONUS);
        world.insert_resource(soil);

        // Farm building on the haunted tile
        let farm_entity = world
            .spawn((
                crate::layer1::building::Building {
                    building_type: crate::layer1::building::BuildingType::Farm,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Farmer assigned to that farm
        let farmer = world
            .spawn((
                Pop,
                Morale::default(),
                Job {
                    workplace: farm_entity,
                    job_type: crate::layer1::actions::AssignmentType::FarmWorker,
                },
            ))
            .id();

        // Advance past the mood period
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: HAUNTED_MOOD_PERIOD_TICKS,
            speed: crate::shared::time::SimSpeed::Normal,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(apply_haunted_mood_system);
        schedule.run(&mut world);

        let morale = world.get::<Morale>(farmer).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == "Haunted Soil"),
            "farmer on haunted soil should gain Haunted Soil mood modifier"
        );
    }

    #[test]
    fn test_scrap_heap_salvage_yields_resources() {
        let mut world = World::new();
        world.insert_resource(crate::layer1::economy::resources::ColonyResources::default());

        let heap = world
            .spawn((
                ScrapHeap {
                    metal: 10.0,
                    wood: 10.0,
                },
                GridPosition { x: 2, y: 2 },
            ))
            .id();
        // A pop standing on the heap
        world.spawn((Pop, GridPosition { x: 2, y: 2 }));

        let mut schedule = Schedule::default();
        schedule.add_systems(salvage_scrap_heap_system);
        schedule.run(&mut world);

        let resources = world.resource::<crate::layer1::economy::resources::ColonyResources>();
        assert!(
            resources.metal > 0.0 || resources.wood > 0.0,
            "salvage should move materials into colony resources"
        );
        // Heap should be depleted or reduced
        let heap_still_there = world.get::<ScrapHeap>(heap);
        assert!(
            heap_still_there.is_none()
                || heap_still_there.unwrap().metal < 10.0,
            "heap should be salvaged"
        );
    }
}
