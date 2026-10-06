//! Utility AI Population Helpers
//!
//! This module handles the crucial "Gather" phase of the Utility AI architecture.
//! It extracts entities from the ECS world and places them into categorized,
//! lock-free buffers within the `UtilityAIBuffer`.
//!
//! # Why pre-populate buffers?
//!
//! Bevy's ECS does not allow mutable access to the world alongside concurrent read
//! queries natively without careful system parameter design. Since thousands of Pops
//! need to evaluate thousands of potential targets simultaneously, doing real-time
//! spatial queries per Pop would be devastating to performance.
//!
//! Instead, we:
//! 1. Run one large query to find all farms, stockpiles, items, etc.
//! 2. Flatten them into `ScorableCandidate` structs in a continuous `Vec`.
//! 3. Pass these read-only vectors to the parallel `ComputeTaskPool` for evaluation.
//!
//! # Optimization Strategy
//!
//! We avoid reallocating these vectors every tick. The `UtilityAIBuffer` lives as a
//! persistent resource, and we simply `clear()` and `extend()` its internal vectors.

use crate::layer1::admin::Office;
use crate::layer1::anomalies::Anomaly;
use crate::layer1::building::{Building, BuildingType, ShiftSchedule};
use crate::layer1::clutter::ClutterGrid;
use crate::layer1::day_night::TimeOfDay;
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::direct_link::Possessed;
use crate::layer1::energy::PowerConsumer;
use crate::layer1::farm::Farm;
use crate::layer1::fauna::Fauna;
use crate::layer1::flora::Flora;
use crate::layer1::funeral::{Corpse, Grave};
use crate::layer1::gene_bank::GeneBank;
use crate::layer1::housing::Housing;
use crate::layer1::hum::HumSource;
use crate::layer1::items::Item;
use crate::layer1::law::justice::Wanted;
use crate::layer1::law::predictive_policing::Suspect;
use crate::layer1::map::GridPosition;
use crate::layer1::medical::Hospital;
use crate::layer1::refining::get_refining_recipe;
use crate::layer1::resources::{RefiningProgress, ResourceItem};
use crate::layer1::social::empty_room::ActiveSanctuaries;
use crate::layer1::social::Tavern;
use crate::layer1::stockpile::Stockpile;
use crate::layer1::structure::{DeferMaintenance, Structure};
use crate::layer1::tech::ghost_code::DataResidue;
use crate::layer1::tech::Library;
use crate::layer1::utility_eval_types::{
    PopEvalData, PopEvaluationQuery, ScorableCandidate, UtilityAIBuffer, WorldContext,
};
use crate::layer1::utility_types::UtilityConfig;
use bevy_ecs::prelude::*;
use bevy_utils::HashSet;

// --- Helpers ---

fn is_active_shift(schedule: Option<&ShiftSchedule>, time: TimeOfDay) -> bool {
    schedule.is_none_or(|s| s.is_active(time))
}

fn is_powered(power: Option<&PowerConsumer>) -> bool {
    power.is_none_or(|p| p.active)
}

const fn is_at_capacity(current: usize, max: usize) -> bool {
    current >= max
}

fn extend_simple<T: Component>(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.extend(
        world
            .query_filtered::<(Entity, &GridPosition), With<T>>()
            .iter(world)
            .map(|(e, pos)| ScorableCandidate::new(e, *pos)),
    );
}

fn populate_simple<T: Component>(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    extend_simple::<T>(world, buffer);
}

fn populate_building_type(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    b_type: BuildingType,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Building)>()
            .iter(world)
            .filter(|(_, _, b)| b.building_type == b_type)
            .map(|(e, pos, _)| ScorableCandidate::new(e, *pos)),
    );
}

/// Populates the `UtilityAIBuffer` with all candidate entities from the world.
///
/// This function calls various internal helpers to query different entity types
/// (buildings, items, designations) and fill the corresponding vectors in the buffer.
///
/// # Arguments
/// * `world` - Mutable reference to the ECS World.
/// * `buffer` - The buffer to populate.
/// * `context` - World context (time, resources, etc.).
pub fn populate_ai_buffer(world: &mut World, buffer: &mut UtilityAIBuffer, context: &WorldContext) {
    populate_buffer_buildings(world, buffer, context);
    populate_buffer_designations(world, buffer);
    populate_unpollinated_crops(world, &mut buffer.unpollinated_crops);
    populate_buffer_items_and_misc(world, buffer);
    populate_walls(world, &mut buffer.walls);
    populate_enemies(world, &mut buffer.enemies);
    populate_all_structures(world, &mut buffer.all_structures);
    populate_cleaning_targets(world, buffer);
    populate_sanctuaries(world, &mut buffer.sanctuaries);
    populate_mobs(world, &mut buffer.mobs);
}

fn populate_mobs(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &crate::layer1::social::Mob)>()
            .iter(world)
            .map(|(e, mob)| {
                ScorableCandidate::new(
                    e,
                    GridPosition {
                        x: mob.location.0,
                        y: mob.location.1,
                    },
                )
            }),
    );
}

fn populate_sanctuaries(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    let Some(manager) = world.get_resource::<ActiveSanctuaries>() else {
        return;
    };
    for sanctuary in &manager.sanctuaries {
        if sanctuary.is_valid && !sanctuary.tiles.is_empty() {
            // Pick the first tile as the target position
            let pos = sanctuary.tiles[0];
            buffer.push(ScorableCandidate {
                entity: Entity::PLACEHOLDER, // Doesn't need a specific entity
                pos,
                score_bonus: sanctuary.effectiveness,
                capacity: 1,
                usage: 0,
                item_type: None,
                resource_type: None,
                is_advanced_tech: false,
            });
        }
    }
}

fn populate_buffer_buildings(
    world: &mut World,
    buffer: &mut UtilityAIBuffer,
    context: &WorldContext,
) {
    populate_farms(world, &mut buffer.farms, context.cycle);
    populate_housing(world, &mut buffer.housing);
    populate_taverns(world, &mut buffer.taverns);
    populate_libraries(world, &mut buffer.libraries, context.cycle);
    populate_refining(world, &mut buffer.refining, context);
    populate_hospitals(world, &mut buffer.hospitals);
    populate_offices(world, &mut buffer.offices, context.cycle);
    populate_showers(world, &mut buffer.showers);
    populate_gene_banks(world, &mut buffer.gene_banks, context.cycle);
}

fn populate_gene_banks(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    cycle: &crate::layer1::day_night::DayNightCycle,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(
                Entity,
                &GridPosition,
                &GeneBank,
                Option<&ShiftSchedule>,
                Option<&PowerConsumer>,
            )>()
            .iter(world)
            .filter(|(_, _, _, schedule, power)| {
                is_active_shift(*schedule, cycle.time_of_day) && is_powered(*power)
            })
            .map(|(entity, pos, _, _, _)| ScorableCandidate::new(entity, *pos)),
    );
}

fn populate_farms(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    cycle: &crate::layer1::day_night::DayNightCycle,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(
                Entity,
                &GridPosition,
                &Farm,
                Option<&ShiftSchedule>,
                Option<&PowerConsumer>,
            )>()
            .iter(world)
            .filter(|(_, _, farm, schedule, power)| {
                is_active_shift(*schedule, cycle.time_of_day)
                    && is_powered(*power)
                    && !is_at_capacity(farm.workers.len(), farm.capacity)
            })
            .map(|(entity, pos, farm, _, _)| {
                ScorableCandidate::with_capacity(entity, *pos, farm.capacity, farm.workers.len())
            }),
    );
}

fn populate_housing(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Housing)>()
            .iter(world)
            .filter(|(_, _, housing)| !is_at_capacity(housing.residents.len(), housing.capacity))
            .map(|(entity, pos, housing)| {
                ScorableCandidate::with_capacity(
                    entity,
                    *pos,
                    housing.capacity,
                    housing.residents.len(),
                )
            }),
    );
}

fn populate_taverns(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Tavern)>()
            .iter(world)
            .filter(|(_, _, tavern)| !is_at_capacity(tavern.visitors.len(), tavern.capacity))
            .map(|(entity, pos, tavern)| {
                ScorableCandidate::with_capacity(
                    entity,
                    *pos,
                    tavern.capacity,
                    tavern.visitors.len(),
                )
            }),
    );
}

fn populate_libraries(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    cycle: &crate::layer1::day_night::DayNightCycle,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Library, Option<&ShiftSchedule>)>()
            .iter(world)
            .filter(|(_, _, _, schedule)| is_active_shift(*schedule, cycle.time_of_day))
            .map(|(entity, pos, _, _)| ScorableCandidate::with_capacity(entity, *pos, 5, 0)),
    );
}

fn populate_refining(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    context: &WorldContext,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(
                Entity,
                &GridPosition,
                &Building,
                &RefiningProgress,
                Option<&ShiftSchedule>,
                Option<&PowerConsumer>,
            )>()
            .iter(world)
            .filter(|(_, _, building, _, schedule, power)| {
                is_active_shift(*schedule, context.cycle.time_of_day)
                    && is_powered(*power)
                    && get_refining_recipe(building.building_type, context.resources).0
            })
            .map(|(entity, pos, _, progress, _, _)| {
                let mut candidate = ScorableCandidate::new(entity, *pos);
                candidate.score_bonus = if progress.current > 0.0 { 0.1 } else { 0.0 };
                candidate
            }),
    );
}

fn populate_hospitals(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Hospital, Option<&PowerConsumer>)>()
            .iter(world)
            .filter(|(_, _, _, power)| is_powered(*power))
            .map(|(entity, pos, _, _)| ScorableCandidate::with_capacity(entity, *pos, 10, 0)),
    );
}

fn populate_offices(
    world: &mut World,
    buffer: &mut Vec<ScorableCandidate>,
    cycle: &crate::layer1::day_night::DayNightCycle,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Office, Option<&ShiftSchedule>)>()
            .iter(world)
            .filter(|(_, _, office, schedule)| {
                is_active_shift(*schedule, cycle.time_of_day)
                    && !is_at_capacity(office.workers.len(), office.capacity)
            })
            .map(|(entity, pos, office, _)| {
                ScorableCandidate::with_capacity(
                    entity,
                    *pos,
                    office.capacity,
                    office.workers.len(),
                )
            }),
    );
}

fn populate_showers(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_building_type(world, buffer, BuildingType::Shower);
}

fn populate_buffer_designations(world: &mut World, buffer: &mut UtilityAIBuffer) {
    // Designations (Work, Repair, Tame)
    buffer.work_designations.clear();
    buffer.repair_designations.clear();
    buffer.tame_designations.clear();
    let mut des_query = world.query::<(Entity, &GridPosition, &Designation)>();
    for (entity, pos, des) in des_query.iter(world) {
        let candidate = ScorableCandidate::new(entity, *pos);
        match des.designation_type {
            DesignationType::Repair => buffer.repair_designations.push(candidate),
            DesignationType::Tame => buffer.tame_designations.push(candidate),
            _ => buffer.work_designations.push(candidate),
        }
    }
}

fn populate_buffer_items_and_misc(world: &mut World, buffer: &mut UtilityAIBuffer) {
    populate_stockpiles(world, &mut buffer.stockpiles);

    // Create lookup set for stockpiles (Optimization: O(1) lookup instead of O(N))
    buffer.stockpile_positions.clear();
    buffer
        .stockpile_positions
        .extend(buffer.stockpiles.iter().map(|s| s.pos));

    populate_items(world, &mut buffer.items);
    populate_generic_items(
        world,
        &buffer.stockpile_positions,
        &mut buffer.item_entities,
    );
    populate_anomalies(world, &mut buffer.anomalies);
    populate_corpses(world, &mut buffer.corpses);
    populate_graves(world, &mut buffer.graves);
    populate_repair_structures(world, &mut buffer.repair_structures);
    populate_wanted_criminals(world, &mut buffer.wanted_criminals);
    populate_suspects(world, &mut buffer.suspects);
    populate_hum_sources(world, &mut buffer.hum_sources);
    populate_residues(world, &mut buffer.residues);
}

fn populate_hum_sources(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &HumSource)>()
            .iter(world)
            .map(|(entity, pos, source)| {
                let mut c = ScorableCandidate::new(entity, *pos);
                c.score_bonus = source.intensity;
                c
            }),
    );
}

fn populate_residues(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<DataResidue>(world, buffer);
}

fn populate_stockpiles(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    // Skip stockpiles sitting on obstacle-building tiles. Fetch/haul arrival
    // requires the pop to stand exactly on the target tile, and obstacle
    // tiles are unwalkable — offering them traps pops in an unreachable loop
    // (e.g. the starter Lander carries a Stockpile component but is an
    // obstacle, which used to wedge every pop on FetchTool forever).
    let blocked: bevy_utils::HashSet<GridPosition> = world
        .query::<(&GridPosition, &Building)>()
        .iter(world)
        .filter(|(_, b)| b.building_type.is_obstacle())
        .map(|(pos, _)| *pos)
        .collect();
    buffer.clear();
    buffer.extend(
        world
            .query_filtered::<(Entity, &GridPosition), With<Stockpile>>()
            .iter(world)
            .filter(|(_, pos)| !blocked.contains(*pos))
            .map(|(e, pos)| ScorableCandidate::new(e, *pos)),
    );
}

fn populate_items(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &ResourceItem)>()
            .iter(world)
            .map(|(entity, pos, item)| {
                let mut c = ScorableCandidate::new(entity, *pos);
                c.resource_type = Some(item.resource_type);
                c
            }),
    );
}

/// Populates loose items (Tools, Clothing, etc.), filtering out those already in stockpiles.
///
/// # Optimization
/// Uses a `HashSet` for stockpile lookup to avoid O(N*M) complexity where N=Items and M=Stockpiles.
/// This reduces the check to O(1) per item.
fn populate_generic_items(
    world: &mut World,
    stockpiles: &HashSet<GridPosition>,
    buffer: &mut Vec<ScorableCandidate>,
) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Item)>()
            .iter(world)
            .filter(|(_, pos, _)| !stockpiles.contains(*pos))
            .map(|(entity, pos, item)| {
                let mut c = ScorableCandidate::new(entity, *pos);
                c.item_type = Some(item.item_type);
                c
            }),
    );
}

fn populate_anomalies(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<Anomaly>(world, buffer);
}

fn populate_corpses(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<Corpse>(world, buffer);
}

fn populate_graves(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(Entity, &GridPosition, &Grave)>()
            .iter(world)
            .filter(|(_, _, grave)| !grave.occupied)
            .map(|(entity, pos, _)| ScorableCandidate::new(entity, *pos)),
    );
}

/// Repair urgency bonus for LifeSupport units, scaled by damage fraction.
///
/// LifeSupport is existential infrastructure: a sabotage-battered unit must
/// outbid routine chores (which sit at ~0.6 utility) or the colony loses its
/// air before anyone bothers to fix it. At full damage this raises repair
/// utility from 0.6 to 1.8 (before distance falloff).
pub const LIFESUPPORT_REPAIR_URGENCY: f32 = 1.2;

fn populate_repair_structures(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(
                Entity,
                &GridPosition,
                &Structure,
                Option<&Building>,
                Option<&DeferMaintenance>,
            )>()
            .iter(world)
            .filter(|(_, _, structure, _, defer)| {
                defer.is_none() && (structure.current_hp - structure.max_hp).abs() >= f32::EPSILON
            })
            .map(|(entity, pos, structure, building, _)| {
                let mut candidate = ScorableCandidate::new(entity, *pos);
                // LifeSupport repair urgency scales with damage: the closer to
                // failure, the harder it outbids competing work.
                if building.is_some_and(|b| b.building_type == BuildingType::LifeSupport)
                    && structure.max_hp > 0.0
                {
                    let damage_fraction =
                        1.0 - (structure.current_hp / structure.max_hp).clamp(0.0, 1.0);
                    candidate.score_bonus = damage_fraction * LIFESUPPORT_REPAIR_URGENCY;
                }
                candidate
            }),
    );
}

fn populate_wanted_criminals(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<Wanted>(world, buffer);
}

fn populate_suspects(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<Suspect>(world, buffer);
}

fn populate_walls(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_building_type(world, buffer, BuildingType::Wall);
}

fn populate_enemies(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    extend_simple::<Fauna>(world, buffer);
    extend_simple::<Flora>(world, buffer);
}

fn populate_all_structures(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    populate_simple::<Structure>(world, buffer);
}

fn populate_cleaning_targets(world: &mut World, buffer: &mut UtilityAIBuffer) {
    buffer.cleaning_targets.clear();
    let Some(grid) = world.get_resource::<ClutterGrid>() else {
        return;
    };

    for y in 0..grid.height {
        for x in 0..grid.width {
            let clutter = grid.get(x, y);
            if clutter <= 50.0 {
                continue;
            }

            // Only add cleaning targets if a building is present, to ensure valid entity targeting.
            // TODO: Support cleaning empty tiles if architecture allows Position targets or ephemeral entities.
            // This is a known limitation: clutter in empty hallways/roads is currently ignored.
            let Some(building_entity) = world
                .resource::<crate::layer1::building::BuildingMap>()
                .0
                .get(&(x as i32, y as i32))
            else {
                continue;
            };

            let mut c = ScorableCandidate::new(
                *building_entity,
                GridPosition {
                    x: x as i32,
                    y: y as i32,
                },
            );
            c.score_bonus = clutter / 100.0; // Higher clutter = higher score bonus
            buffer.cleaning_targets.push(c);
        }
    }
}

/// Collects data for all Pops that need to be evaluated this tick.
///
/// Filters pops based on `ticks_committed` and `UtilityConfig::evaluation_interval`.
/// Also populates additional data like insulation from equipped clothing.
///
/// # Arguments
/// * `world` - Mutable reference to the ECS World.
/// * `buffer` - The buffer to store `PopEvalData`.
/// * `config` - Utility AI configuration.
pub fn collect_pop_data(world: &mut World, buffer: &mut UtilityAIBuffer, config: &UtilityConfig) {
    buffer.pop_data.clear();
    buffer.pop_data.extend(
        world
            .query_filtered::<PopEvaluationQuery, (
                Without<crate::layer1::cryo::CryoStasis>,
                Without<Possessed>,
                Without<crate::layer1::mind::sleep_debt::ForcedComa>,
                Without<crate::layer1::artifacts::vr_pod::InVrPod>,
                // The Salvager waits aboard the derelict — never a colony worker.
                Without<crate::layer1::culture::salvager::Salvager>,
                // The Improbable Pilot waits aboard the shuttle — never a colony worker.
                Without<crate::layer1::culture::improbable::ImprobablePilot>,
                // The Lawbound serves the Statutes, not the chore wheel — never a colony worker.
                Without<crate::layer1::culture::lawbound::LawboundAutomaton>,
                // The dormant automaton waits by its cradle — never a colony worker.
                Without<crate::layer1::culture::lawbound::DormantAutomaton>,
                // The Chronostalker walks between moments — never a colony worker.
                Without<crate::layer1::culture::chronostalker::Chronostalker>,
                // The dormant stranger waits in the moment-wound — never a colony worker.
                Without<crate::layer1::culture::chronostalker::DormantStalker>,
                // The Bloom-Touched walks the bloom's path — never a colony worker.
                Without<crate::layer1::culture::bloomtouched::BloomTouched>,
                // The dormant scout waits by the scar — never a colony worker.
                Without<crate::layer1::culture::bloomtouched::DormantTouched>,
                // Bloomkin are no longer quite colonists — never colony workers.
                Without<crate::layer1::culture::bloomtouched::BloomKin>,
            )>()
            .iter(world)
            .filter(|item| {
                item.action.ticks_committed >= config.evaluation_interval
                    && (item.inmate.is_none() || item.penal_labor.is_some())
            })
            .map(PopEvalData::from_query_item),
    );

    // Populate Insulation from Clothing entities
    let mut clothing_query = world.query::<&crate::layer1::items::Clothing>();
    // Populate Carrying Item Type
    let mut item_query = world.query::<&crate::layer1::items::Item>();

    for data in &mut buffer.pop_data {
        if let Some(body_entity) = data.equipment.and_then(|eq| eq.body) {
            if let Ok(clothing) = clothing_query.get(world, body_entity) {
                data.insulation = clothing.insulation;
            }
        }

        if let Some(item_entity) = data.carrying_item {
            if let Ok(item) = item_query.get(world, item_entity) {
                data.carrying_item_type = Some(item.item_type);
            }
        }
    }
}

fn populate_unpollinated_crops(world: &mut World, buffer: &mut Vec<ScorableCandidate>) {
    buffer.clear();
    buffer.extend(
        world
            .query::<(
                Entity,
                &crate::layer1::map::GridPosition,
                &crate::layer1::agriculture::pollination::FarmCrop,
                &crate::layer1::agriculture::pollination::PollinationStatus,
            )>()
            .iter(world)
            .filter(|(_, _, crop, status)| {
                crop.growth_stage
                    == crate::layer1::agriculture::pollination::FarmGrowthStage::Flowering
                    && crop.requires_pollination
                    && !status.is_pollinated
            })
            .map(|(entity, pos, _, _)| ScorableCandidate::new(entity, *pos)),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::{Item, ItemType};
    use crate::layer1::map::GridPosition;

    #[test]
    fn test_populate_generic_items_filtering() {
        let mut world = World::new();

        // 1. Create Stockpile Positions Set
        let mut stockpiles = bevy_utils::HashSet::new();
        stockpiles.insert(GridPosition { x: 10, y: 0 });

        // 2. Spawn Item inside stockpile (Should be filtered)
        let _item_in = world
            .spawn((
                Item {
                    item_type: ItemType::Manual,
                },
                GridPosition { x: 10, y: 0 },
            ))
            .id();

        // 3. Spawn Item outside stockpile (Should be included)
        let item_out = world
            .spawn((
                Item {
                    item_type: ItemType::Manual,
                },
                GridPosition { x: 5, y: 0 },
            ))
            .id();

        // 4. Run population
        let mut buffer = Vec::with_capacity(1);
        populate_generic_items(&mut world, &stockpiles, &mut buffer);

        // 5. Verify
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer[0].entity, item_out);
    }

    #[test]
    fn test_populate_farms_filters_inactive_schedule() {
        use crate::layer1::building::ShiftSchedule;
        use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
        use crate::layer1::farm::Farm;

        let mut world = World::new();
        let cycle = DayNightCycle {
            time_of_day: TimeOfDay::Night, // Currently Night
            ..Default::default()
        };

        // Farm active only during Day
        world.spawn((
            Farm::default(),
            GridPosition { x: 0, y: 0 },
            ShiftSchedule {
                day_shift: true,
                night_shift: false,
            },
        ));

        let mut buffer = Vec::with_capacity(1);
        populate_farms(&mut world, &mut buffer, &cycle);

        assert!(
            buffer.is_empty(),
            "Should filter out farm inactive at night"
        );

        // Change cycle to Day
        let cycle_day = DayNightCycle {
            time_of_day: TimeOfDay::Day,
            ..Default::default()
        };
        populate_farms(&mut world, &mut buffer, &cycle_day);
        assert_eq!(buffer.len(), 1, "Should include farm active during day");
    }

    #[test]
    fn test_populate_farms_filters_unpowered() {
        use crate::layer1::day_night::DayNightCycle;
        use crate::layer1::energy::PowerConsumer;
        use crate::layer1::farm::Farm;

        let mut world = World::new();
        let cycle = DayNightCycle::default(); // Default is Day

        // Unpowered Farm
        world.spawn((
            Farm::default(),
            GridPosition { x: 0, y: 0 },
            PowerConsumer {
                active: false,
                ..Default::default()
            },
        ));

        let mut buffer = Vec::with_capacity(1);
        populate_farms(&mut world, &mut buffer, &cycle);

        assert!(buffer.is_empty(), "Should filter out unpowered farm");

        // Powered Farm
        world.spawn((
            Farm::default(),
            GridPosition { x: 1, y: 0 },
            PowerConsumer {
                active: true,
                ..Default::default()
            },
        ));
        populate_farms(&mut world, &mut buffer, &cycle);
        assert_eq!(buffer.len(), 1, "Should include powered farm");
    }

    #[test]
    fn test_populate_farms_filters_full_capacity() {
        use crate::layer1::day_night::DayNightCycle;
        use crate::layer1::farm::Farm;

        let mut world = World::new();
        let cycle = DayNightCycle::default();

        // Full Farm
        let full_farm = Farm {
            capacity: 1,
            workers: vec![Entity::from_raw(123)],
            ..Default::default()
        };

        world.spawn((full_farm, GridPosition { x: 0, y: 0 }));

        let mut buffer = Vec::with_capacity(1);
        populate_farms(&mut world, &mut buffer, &cycle);

        assert!(buffer.is_empty(), "Should filter out full farm");

        // Empty Farm
        let empty_farm = Farm {
            capacity: 1,
            ..Default::default()
        };
        world.spawn((empty_farm, GridPosition { x: 1, y: 0 }));
        populate_farms(&mut world, &mut buffer, &cycle);
        assert_eq!(buffer.len(), 1, "Should include empty farm");
    }

    #[test]
    fn test_populate_refining_filters_unaffordable() {
        use crate::layer1::building::{Building, BuildingType};
        use crate::layer1::day_night::DayNightCycle;
        use crate::layer1::resources::{ColonyResources, RefiningProgress};
        use crate::layer1::taboo::TabooState;
        use crate::layer1::utility_eval_types::WorldContext;
        use crate::layer1::zone::ZoneGrid;

        let mut world = World::new();
        // LumberMill requires 1 Wood.
        let resources = ColonyResources {
            wood: 0.0,
            ..Default::default()
        };

        let cycle = DayNightCycle::default();
        let taboo = TabooState::default();
        let zone_grid = ZoneGrid::new(10, 10);

        let context = WorldContext {
            resources: &resources,
            cycle: &cycle,
            taboo: &taboo,
            factions: None,
            zone_grid: &zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 0, y: 0 },
            RefiningProgress::default(),
        ));

        let mut buffer = Vec::with_capacity(1);
        populate_refining(&mut world, &mut buffer, &context);

        assert!(buffer.is_empty(), "Should filter out unaffordable recipe");

        // Now afford
        let resources = ColonyResources {
            wood: 1.0,
            ..Default::default()
        };
        let context_valid = WorldContext {
            resources: &resources,
            cycle: &cycle,
            taboo: &taboo,
            factions: None,
            zone_grid: &zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };
        populate_refining(&mut world, &mut buffer, &context_valid);
        assert_eq!(buffer.len(), 1, "Should include affordable recipe");
    }

    #[test]
    fn test_populate_refining_filters_inactive_schedule() {
        use crate::layer1::building::{Building, BuildingType, ShiftSchedule};
        use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
        use crate::layer1::resources::{ColonyResources, RefiningProgress};
        use crate::layer1::taboo::TabooState;
        use crate::layer1::utility_eval_types::WorldContext;
        use crate::layer1::zone::ZoneGrid;

        let mut world = World::new();
        let resources = ColonyResources {
            wood: 1.0,
            ..Default::default()
        };

        let cycle = DayNightCycle {
            time_of_day: TimeOfDay::Night,
            ..Default::default()
        };
        let taboo = TabooState::default();
        let zone_grid = ZoneGrid::new(10, 10);

        let context = WorldContext {
            resources: &resources,
            cycle: &cycle,
            taboo: &taboo,
            factions: None,
            zone_grid: &zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 0, y: 0 },
            RefiningProgress::default(),
            ShiftSchedule {
                day_shift: true,
                night_shift: false,
            },
        ));

        let mut buffer = Vec::with_capacity(1);
        populate_refining(&mut world, &mut buffer, &context);

        assert!(buffer.is_empty(), "Should filter out inactive shift");
    }

    #[test]
    fn test_populate_refining_filters_unpowered() {
        use crate::layer1::building::{Building, BuildingType};
        use crate::layer1::day_night::DayNightCycle;
        use crate::layer1::energy::PowerConsumer;
        use crate::layer1::resources::{ColonyResources, RefiningProgress};
        use crate::layer1::taboo::TabooState;
        use crate::layer1::utility_eval_types::WorldContext;
        use crate::layer1::zone::ZoneGrid;

        let mut world = World::new();
        let resources = ColonyResources {
            wood: 1.0,
            ..Default::default()
        };

        let cycle = DayNightCycle::default();
        let taboo = TabooState::default();
        let zone_grid = ZoneGrid::new(10, 10);

        let context = WorldContext {
            resources: &resources,
            cycle: &cycle,
            taboo: &taboo,
            factions: None,
            zone_grid: &zone_grid,
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        world.spawn((
            Building {
                building_type: BuildingType::LumberMill,
            },
            GridPosition { x: 0, y: 0 },
            RefiningProgress::default(),
            PowerConsumer {
                active: false,
                ..Default::default()
            },
        ));

        let mut buffer = Vec::with_capacity(1);
        populate_refining(&mut world, &mut buffer, &context);

        assert!(buffer.is_empty(), "Should filter out unpowered building");
    }

    #[test]
    fn test_populate_housing_filters_full() {
        use crate::layer1::housing::Housing;

        let mut world = World::new();

        // Full Housing
        let housing = Housing {
            capacity: 1,
            residents: vec![Entity::from_raw(123)],
        };

        world.spawn((housing, GridPosition { x: 0, y: 0 }));

        let mut buffer = Vec::with_capacity(1);
        populate_housing(&mut world, &mut buffer);

        assert!(buffer.is_empty(), "Should filter out full housing");

        // Available Housing
        let housing_empty = Housing::default();
        world.spawn((housing_empty, GridPosition { x: 1, y: 0 }));
        populate_housing(&mut world, &mut buffer);
        assert_eq!(buffer.len(), 1, "Should include available housing");
    }

    #[test]
    fn test_lifesupport_repair_urgency_bonus() {
        let mut world = World::new();

        // Damaged LifeSupport at 50% (25/50 HP)
        let ls = world
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                Structure {
                    current_hp: 25.0,
                    max_hp: 50.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        // Damaged non-critical building (Housing at 50%)
        let housing = world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                Structure {
                    current_hp: 50.0,
                    max_hp: 100.0,
                },
                GridPosition { x: 6, y: 6 },
            ))
            .id();

        let mut buffer = Vec::with_capacity(2);
        populate_repair_structures(&mut world, &mut buffer);

        assert_eq!(buffer.len(), 2, "Both damaged structures are candidates");

        let ls_cand = buffer.iter().find(|c| c.entity == ls).unwrap();
        let housing_cand = buffer.iter().find(|c| c.entity == housing).unwrap();

        // 50% damage -> bonus = 0.5 * 1.2 = 0.6
        assert!(
            (ls_cand.score_bonus - 0.6).abs() < 0.001,
            "LifeSupport should get urgency bonus, got {}",
            ls_cand.score_bonus
        );
        assert_eq!(
            housing_cand.score_bonus, 0.0,
            "Non-critical building should get no bonus"
        );
    }

    #[test]
    fn test_lifesupport_repair_urgency_scales_with_damage() {
        let mut world = World::new();

        // Critically damaged LifeSupport (10% HP remaining)
        let ls = world
            .spawn((
                Building {
                    building_type: BuildingType::LifeSupport,
                },
                Structure {
                    current_hp: 5.0,
                    max_hp: 50.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        let mut buffer = Vec::with_capacity(1);
        populate_repair_structures(&mut world, &mut buffer);

        let ls_cand = buffer.iter().find(|c| c.entity == ls).unwrap();
        // 90% damage -> bonus = 0.9 * 1.2 = 1.08
        assert!(
            (ls_cand.score_bonus - 1.08).abs() < 0.001,
            "Critical LifeSupport should get max urgency, got {}",
            ls_cand.score_bonus
        );
    }
}
