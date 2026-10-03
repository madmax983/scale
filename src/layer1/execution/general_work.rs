use bevy_ecs::prelude::*;
use rand::Rng;

use crate::layer1::admin::AdminStats;
use crate::layer1::architecture::edible::execute_consume;
use crate::layer1::cybernetics::get_efficiency_bonus;
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::designation::{Designation, DesignationType};
use crate::layer1::economy::{get_wage_for_job, pay_wage};
use crate::layer1::edicts::{get_work_speed_modifier, ColonyPolicies};
use crate::layer1::environment::hazards::handle_workplace_hazards;
use crate::layer1::eureka::check_for_eureka_world;
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::execution::demolish::{
    execute_cannibalize, execute_demolish, execute_destroy, execute_jury_rig,
};
use crate::layer1::execution::mining::{handle_chopping_work, handle_mining_work};
use crate::layer1::flora::process_flora_clearing;
use crate::layer1::gastronomy::WorkSpeedBuff;
use crate::layer1::heirloom::{Heirloom, ToolHistory};
use crate::layer1::items::{Equipment, Tool, UnequipEvent};
use crate::layer1::language::{calculate_coordination_penalty, Dialect, Linguistics};
use crate::layer1::map::GridPosition;
use crate::layer1::memory::{calculate_effective_morale, Memories};
use crate::layer1::morale::Morale;
use crate::layer1::mother_lode::MotherLode;
use crate::layer1::needs::{get_morale_efficiency, Needs};
use crate::layer1::pop::Job;
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::layer1::skills::{get_skill_efficiency, SkillType, Skills};
use crate::layer1::social::SocialBuff;
use crate::layer1::tech::hypno_learning::MentalFog;
use crate::layer1::tech::Tech;
use crate::layer1::traits::{get_job_efficiency_modifier, get_trait_work_speed_modifier, Traits};
use crate::layer1::utility_types::{ActionType, PopAction};
use crate::shared::log::MessageLog;

/// Work amount applied per tick when a pop is working.
const WORK_PER_TICK: f32 = 10.0;

/// Durability loss per tick when working.
const TOOL_DURABILITY_LOSS: f32 = 0.1;

struct WorkerData {
    entity: Entity,
    morale: f32,
    action: ActionType,
    equipment: Option<Equipment>,
    speed_modifier: f32,
    job: Option<Job>,
    dialect: Dialect,
    linguistics: Linguistics,
    is_ostracized: bool,
}

/// Executes work at designations when pop is at target with Work action.
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
pub fn work_execution_system(world: &mut World) {
    let policies = world.get_resource::<ColonyPolicies>().cloned();
    let global_work_speed_mod = policies.as_ref().map_or(1.0, get_work_speed_modifier);

    // Fetch DayNightCycle
    let cycle = world.get_resource::<DayNightCycle>().map(|c| c.time_of_day);

    // Fetch Factions for strike check
    // We collect striking factions into a set to avoid borrowing conflicts with world
    let mut striking_factions =
        bevy_utils::HashSet::<crate::layer1::factions::FactionId>::default();
    if let Some(factions) = world.get_resource::<crate::layer1::factions::Factions>() {
        for (id, d) in &factions.map {
            if d.state == crate::layer1::factions::FactionState::Striking {
                striking_factions.insert(*id);
            }
        }
    }

    // Spec 218: Calculate Improvised Efficiency (Global Fallback)
    let (improvised_efficiency, consumed_resource_type) = world
        .get_resource::<crate::layer1::resources::ColonyResources>()
        .map_or((0.5, None), |res| {
            crate::layer1::execution::efficiency::calculate_work_efficiency(res)
        });

    let workers_by_target =
        collect_workers_by_target(world, policies.as_ref(), &striking_factions, cycle);

    for (target_entity, group) in workers_by_target {
        for worker in &group {
            let coordination_mod = calculate_group_coordination(worker, &group);

            process_single_worker(
                world,
                worker.entity,
                target_entity,
                worker.morale,
                worker.action,
                worker.equipment,
                global_work_speed_mod
                    * worker.speed_modifier
                    * coordination_mod
                    * if worker.is_ostracized { 0.2 } else { 1.0 },
                worker.job,
                improvised_efficiency,
                consumed_resource_type,
            );
        }
    }
}

#[derive(bevy_ecs::query::QueryData)]
struct WorkerQuery {
    entity: Entity,
    mt: &'static MovementTarget,
    needs: Option<&'static Needs>,
    memories: Option<&'static Memories>,
    social_buff: Option<&'static SocialBuff>,
    equipment: Option<&'static Equipment>,
    traits: Option<&'static Traits>,
    morale_comp: Option<&'static Morale>,
    faction_member: Option<&'static crate::layer1::factions::FactionMember>,
    buff: Option<&'static WorkSpeedBuff>,
    job: Option<&'static Job>,
    dialect: Option<&'static Dialect>,
    linguistics: Option<&'static Linguistics>,
    fog: Option<&'static MentalFog>,
    ostracized: Option<&'static crate::layer1::social::grievances::Ostracized>,
    on_strike: Has<crate::layer1::culture::memorial_revolt::OnStrike>,
}

/// ⚡ Bolt Optimization: Uses `bevy::utils::HashMap` (AHash) instead of `std::collections::HashMap`
/// to eliminate SipHash overhead during high-frequency worker grouping operations.
fn collect_workers_by_target(
    world: &mut World,
    policies: Option<&ColonyPolicies>,
    striking_factions: &bevy_utils::HashSet<crate::layer1::factions::FactionId>,
    cycle: Option<crate::layer1::day_night::TimeOfDay>,
) -> bevy::utils::HashMap<Entity, Vec<WorkerData>> {
    let mut workers_by_target: bevy::utils::HashMap<Entity, Vec<WorkerData>> =
        bevy::utils::HashMap::default();

    let mut query = world.query_filtered::<WorkerQuery, With<AtTarget>>();

    // ⚡ Bolt Optimization:
    // We iterate over `query.iter(world)` and stream directly into the `HashMap`.
    // This removes an intermediate `.collect::<Vec<_>>()` allocation that was previously used,
    // significantly reducing heap allocations per frame when evaluating large worker populations.
    for (target, worker) in query
        .iter(world)
        .filter(|item| {
            let is_work =
                item.mt.for_action == ActionType::Work || item.mt.for_action == ActionType::Repair;
            if !is_work {
                return false;
            }

            let is_striking = item.on_strike
                || item
                    .faction_member
                    .and_then(|m| m.faction_id)
                    .is_some_and(|fid| striking_factions.contains(&fid));

            !is_striking
        })
        .map(|item| {
            let morale = item.needs.map_or(0.5, |n| {
                calculate_effective_morale(
                    n,
                    item.memories,
                    item.social_buff,
                    policies,
                    item.traits,
                    cycle,
                    item.morale_comp,
                )
            });
            let trait_work_mod = item.traits.map_or(1.0, get_trait_work_speed_modifier);
            let job_eff_mod = if let (Some(t), Some(j)) = (item.traits, item.job) {
                get_job_efficiency_modifier(t, j.job_type)
            } else {
                1.0
            };
            let buff_mod = item.buff.map_or(1.0, |b| b.multiplier);
            let fog_mod = item.fog.map_or(1.0, |f| f.work_speed_penalty);

            (
                item.mt.target_entity,
                WorkerData {
                    entity: item.entity,
                    morale,
                    action: item.mt.for_action,
                    equipment: item.equipment.copied(),
                    speed_modifier: trait_work_mod * job_eff_mod * buff_mod * fog_mod,
                    job: item.job.copied(),
                    dialect: item.dialect.copied().unwrap_or_default(),
                    linguistics: item.linguistics.cloned().unwrap_or_default(),
                    is_ostracized: item.ostracized.is_some(),
                },
            )
        })
    {
        workers_by_target.entry(target).or_default().push(worker);
    }

    workers_by_target
}

fn calculate_group_coordination(worker: &WorkerData, group: &[WorkerData]) -> f32 {
    let mut coordination_mod = 1.0;
    for other in group {
        if worker.entity == other.entity {
            continue;
        }
        let p =
            calculate_coordination_penalty(&worker.dialect, &worker.linguistics, &other.dialect);
        if p < coordination_mod {
            coordination_mod = p;
        }
    }
    coordination_mod
}

fn get_designation_type(
    world: &World,
    entity: Entity,
    action: ActionType,
) -> Option<DesignationType> {
    if let Some(des) = world.get::<Designation>(entity) {
        return Some(des.designation_type);
    }
    if action == ActionType::Repair
        && world
            .get::<crate::layer1::structure::Structure>(entity)
            .is_some()
    {
        return Some(DesignationType::Repair);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn process_single_worker(
    world: &mut World,
    pop_entity: Entity,
    designation_entity: Entity,
    morale: f32,
    action_type: ActionType,
    equipment_opt: Option<Equipment>,
    work_speed_mod: f32,
    job_opt: Option<Job>,
    improvised_efficiency: f32,
    consumed_resource_type: Option<ResourceType>,
) {
    // Check if designation/target still exists (early exit)
    if world.get_entity(designation_entity).is_err() {
        cleanup_pop_work_state(world, pop_entity);
        return;
    }

    // Get designation type
    let Some(designation_type) = get_designation_type(world, designation_entity, action_type)
    else {
        cleanup_pop_work_state(world, pop_entity);
        return;
    };

    // Check per-pop tool availability
    let tool_entity_opt = equipment_opt.as_ref().and_then(|e| e.tool);

    // Update Tool History
    if let Some(tool_entity) = tool_entity_opt {
        if let Some(mut history) = world.get_mut::<ToolHistory>(tool_entity) {
            history.ticks_used += 1;
        }
    }

    // Calculate Work Amount
    let work_amount = calculate_work_amount(
        world,
        pop_entity,
        designation_type,
        tool_entity_opt,
        morale,
        work_speed_mod,
        improvised_efficiency,
    );

    // Execute Work
    let worked = execute_work_on_designation(
        world,
        pop_entity,
        designation_entity,
        designation_type,
        work_amount,
    );

    if check_work_completion(world, designation_entity, designation_type) {
        cleanup_pop_work_state(world, pop_entity);

        // Pay Wage
        let wage = job_opt.map_or(1.0, |job| get_wage_for_job(job.job_type));
        pay_wage(world, pop_entity, wage);
    }

    // Post-work effects (XP, Hazards, Durability, Improvised Tool Consumption)
    if worked {
        handle_post_work_effects(
            world,
            pop_entity,
            designation_entity,
            designation_type,
            action_type,
            tool_entity_opt,
        );

        handle_eureka_moment(world, pop_entity, designation_type, action_type);

        // Spec 218: Consume improvised materials if no tool was used
        if tool_entity_opt.is_none() {
            if let Some(res_type) = consumed_resource_type {
                // Probabilistic consumption
                let break_chance = match res_type {
                    ResourceType::Tools => 0.01,
                    _ => 0.05,
                };
                if rand::thread_rng().gen_bool(break_chance) {
                    if let Some(mut resources) = world.get_resource_mut::<ColonyResources>() {
                        resources.consume(res_type, 1.0);
                    }
                }
            }
        }
    }
}

fn check_work_completion(
    world: &World,
    target_entity: Entity,
    designation_type: DesignationType,
) -> bool {
    if world.get_entity(target_entity).is_err() {
        return true;
    }
    if designation_type == DesignationType::Repair {
        if let Some(s) = world.get::<crate::layer1::structure::Structure>(target_entity) {
            return (s.current_hp - s.max_hp).abs() < f32::EPSILON;
        }
    }
    false
}

fn handle_eureka_moment(
    world: &mut World,
    pop_entity: Entity,
    designation_type: DesignationType,
    action_type: ActionType,
) {
    let related_tech = match designation_type {
        DesignationType::Mine => Some(Tech::Masonry),
        // Add other mappings as appropriate
        _ => None,
    };

    check_for_eureka_world(world, action_type, related_tech, Some(pop_entity));
}

/// Returns the skill type associated with a designation type.
pub(crate) const fn get_skill_for_designation(
    designation_type: DesignationType,
) -> Option<SkillType> {
    match designation_type {
        DesignationType::Mine => Some(SkillType::Mining),
        DesignationType::Chop => Some(SkillType::Forestry),
        DesignationType::Repair
        | DesignationType::Demolish
        | DesignationType::JuryRig
        | DesignationType::Cannibalize
        | DesignationType::Destroy => Some(SkillType::Construction),
        DesignationType::ClearFlora | DesignationType::CollectSample => Some(SkillType::Farming),
        DesignationType::ExcavateDust => Some(SkillType::Mining),
        DesignationType::SetZone(_) | DesignationType::Tame | DesignationType::Consume => None,
    }
}

/// Calculates the amount of work a pop can perform in a tick.
///
/// # Examples
///
/// ```
/// use bevy_ecs::prelude::*;
/// use scale::layer1::execution::calculate_work_amount;
/// use scale::layer1::designation::DesignationType;
///
/// let mut world = World::new();
/// let pop = world.spawn_empty().id();
///
/// let amount = calculate_work_amount(
///     &world,
///     pop,
///     DesignationType::Mine,
///     None, // no tool
///     50.0, // morale
///     1.0,  // work speed modifier
///     0.5,  // improvised efficiency (bare hands)
/// );
/// assert!(amount > 0.0);
/// ```
pub fn calculate_work_amount(
    world: &World,
    pop_entity: Entity,
    designation_type: DesignationType,
    tool_entity: Option<Entity>,
    morale: f32,
    work_speed_mod: f32,
    improvised_efficiency: f32,
) -> f32 {
    let mut tool_efficiency = if tool_entity.is_some() {
        1.0
    } else {
        improvised_efficiency
    };

    // Apply Heirloom bonus
    if let Some(entity) = tool_entity {
        if let Some(heirloom) = world.get::<Heirloom>(entity) {
            tool_efficiency *= 1.0 + heirloom.efficiency_bonus;
        }
    }

    let skill_type = get_skill_for_designation(designation_type);

    let skill_efficiency = {
        let skills = world.get::<Skills>(pop_entity);
        skill_type.map_or(1.0, |st| get_skill_efficiency(skills, st))
    };

    let morale_efficiency = get_morale_efficiency(morale);

    // Ludwig: Add organic variation to work speed (0.9 - 1.1) so pops don't feel robotic
    let mut rng = rand::thread_rng();
    let organic_factor = rng.gen_range(0.9..1.1);

    let augmentation_bonus = get_efficiency_bonus(world, pop_entity);

    let admin_efficiency = world
        .get_resource::<AdminStats>()
        .map_or(1.0, |stats| stats.efficiency);

    let propaganda_graffiti_modifier = {
        let pop_pos = world.get::<GridPosition>(pop_entity);
        let mut modifier = 1.0;
        if let Some(pos) = pop_pos {
            if let Some(map) = world.get_resource::<crate::layer1::graffiti::GraffitiMap>() {
                let neighbors = [
                    (pos.x, pos.y), // Check same tile
                    (pos.x, pos.y - 1),
                    (pos.x + 1, pos.y),
                    (pos.x, pos.y + 1),
                    (pos.x - 1, pos.y),
                ];
                for target in neighbors {
                    if let Some(graffiti) = map.markings.get(&target) {
                        if graffiti.graffiti_type
                            == crate::layer1::graffiti::GraffitiType::Propaganda
                        {
                            modifier = 0.9;
                            break;
                        }
                    }
                }
            }
        }
        modifier
    };

    let status_modifiers = get_status_modifiers(world, pop_entity);

    let amount = WORK_PER_TICK
        * tool_efficiency
        * morale_efficiency
        * skill_efficiency
        * work_speed_mod
        * admin_efficiency
        * (1.0 + augmentation_bonus)
        * organic_factor
        * status_modifiers
        * propaganda_graffiti_modifier;

    // Cap work amount to prevent logic bugs / economy exploits
    amount.min(1000.0)
}

fn execute_work_on_designation(
    world: &mut World,
    pop_entity: Entity, // Added pop_entity for XP attribution
    designation_entity: Entity,
    designation_type: DesignationType,
    work_amount: f32,
) -> bool {
    // Ludwig: Get position for juice effects
    let pos = world.get::<GridPosition>(designation_entity).copied();

    match designation_type {
        DesignationType::Mine => {
            handle_mining_work(world, designation_entity, pop_entity, work_amount, pos)
        }
        DesignationType::Chop => {
            handle_chopping_work(world, designation_entity, pop_entity, work_amount, pos)
        }
        DesignationType::Demolish => execute_demolish(world, designation_entity),
        DesignationType::Repair => {
            crate::layer1::structure::process_repair(world, designation_entity, work_amount);
            true
        }
        DesignationType::ClearFlora => {
            process_flora_clearing(world, designation_entity, work_amount);
            true
        }
        DesignationType::JuryRig => execute_jury_rig(world, designation_entity),
        DesignationType::Cannibalize => execute_cannibalize(world, designation_entity),
        DesignationType::Consume => execute_consume(world, designation_entity),
        DesignationType::Destroy => execute_destroy(world, designation_entity),
        DesignationType::CollectSample => {
            if let Some(pos) = world.get::<GridPosition>(designation_entity).copied() {
                // Actor unused in current implementation
                let success =
                    crate::layer1::gene_bank::collect_sample_action(world, designation_entity, pos);
                if success {
                    world.despawn(designation_entity);
                }
                success
            } else {
                false
            }
        }
        DesignationType::ExcavateDust => handle_dust_excavation(world, designation_entity, pos),
        DesignationType::SetZone(_) | DesignationType::Tame => false,
    }
}

/// Excavate Desire Dust from a designation tile (Spec 1379 counterplay).
///
/// Clears the dust, shrinks or dissolves any RoadMind on the tile, and
/// despawns the completed designation.
fn handle_dust_excavation(
    world: &mut World,
    designation_entity: Entity,
    pos: Option<GridPosition>,
) -> bool {
    if let Some(p) = pos {
        crate::layer1::desire_dust::excavate_dust_tile(world, p.x, p.y);
        if let Some(mut log) = world.get_resource_mut::<crate::shared::log::MessageLog>() {
            log.add(format!("Excavated desire dust at ({}, {})", p.x, p.y));
        }
    }
    world.despawn(designation_entity);
    true
}

fn handle_post_work_effects(
    world: &mut World,
    pop_entity: Entity,
    designation_entity: Entity,
    designation_type: DesignationType,
    action_type: ActionType,
    tool_entity_opt: Option<Entity>,
) {
    let skill_type = get_skill_for_designation(designation_type);

    // Add XP
    if let Some(st) = skill_type {
        if let Some(mut skills) = world.get_mut::<Skills>(pop_entity) {
            skills.add_xp(st, 1.0);
        }
    }

    // Fetch structure if designation targets one
    let structure_opt = if world
        .get::<crate::layer1::structure::Structure>(designation_entity)
        .is_some()
    {
        world
            .get::<crate::layer1::structure::Structure>(designation_entity)
            .copied() // Copy to avoid borrow issues
    } else {
        None
    };

    // Fetch skills
    let skills = world.get::<Skills>(pop_entity).cloned().unwrap_or_default();

    // Fetch MotherLode hazard if present
    let hazard_modifier = if let Some(lode) = world.get::<MotherLode>(designation_entity) {
        f64::from(lode.current_hazard)
    } else {
        1.0
    };

    handle_workplace_hazards(
        world,
        pop_entity,
        action_type,
        structure_opt.as_ref(),
        &skills,
        hazard_modifier,
    );

    // Handle tool durability
    if let Some(tool_entity) = tool_entity_opt {
        handle_tool_durability(world, pop_entity, tool_entity);
    }
}

fn handle_tool_durability(world: &mut World, pop_entity: Entity, tool_entity: Entity) {
    let mut broke = false;
    if let Some(mut tool) = world.get_mut::<Tool>(tool_entity) {
        tool.durability -= TOOL_DURABILITY_LOSS;
        if tool.durability <= 0.0 {
            broke = true;
        }
    }

    if broke {
        // Emit UnequipEvent
        world.send_event(UnequipEvent {
            actor: pop_entity,
            item: tool_entity,
            slot: "tool".to_string(),
        });

        // Despawn tool
        world.despawn(tool_entity);

        // Clear equipment
        if let Some(mut eq) = world.get_mut::<Equipment>(pop_entity) {
            eq.tool = None;
        }

        // Log breakage
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add("CRACK! A tool has broken.");
        }
    }
}

fn cleanup_pop_work_state(world: &mut World, pop_entity: Entity) {
    world
        .entity_mut(pop_entity)
        .remove::<MovementTarget>()
        .remove::<AtTarget>();
    if let Some(mut action) = world.get_mut::<PopAction>(pop_entity) {
        action.current = ActionType::Idle;
        action.current_utility = 0.0;
        action.ticks_committed = 1;
    }
}

pub fn get_status_modifiers(world: &World, pop_entity: Entity) -> f32 {
    let mut modifier = 1.0;

    if let Some(efficiency) = world.get::<crate::layer1::economy::WorkEfficiency>(pop_entity) {
        modifier *= efficiency.multiplier;
    }

    if world
        .get::<crate::layer1::tech::neural_leech::NeuralLinked>(pop_entity)
        .is_some()
    {
        modifier *= 2.0;
    }

    if world
        .get::<crate::layer1::memetics::MemeticInfection>(pop_entity)
        .is_some()
    {
        modifier *= 0.5;
    }

    if world
        .get::<crate::layer1::social::ghost_shift_strike::GhostShiftState>(pop_entity)
        .is_some()
    {
        modifier *= 0.0;
    }

    if world
        .get::<crate::layer1::agriculture::gastronomy::Hallucinating>(pop_entity)
        .is_some()
    {
        modifier *= 0.0;
    }

    if world
        .get::<crate::layer1::somnambulism::Somnambulist>(pop_entity)
        .is_some()
    {
        modifier *= 5.0;
    }

    if world
        .get::<crate::layer1::psychology::void_sleep::GravityNightmare>(pop_entity)
        .is_some()
    {
        modifier *= 0.8; // 20% efficiency penalty due to nightmares
    }

    if world
        .get::<crate::layer1::mind::temporal_fugue::TemporalFugue>(pop_entity)
        .is_some()
    {
        modifier *= 3.0;
    }

    if let Some(sync) = world.get::<crate::layer1::flora::BioRhythmSync>(pop_entity) {
        if sync.is_synced {
            modifier *= sync.work_speed_multiplier;
        }
    }

    // Ego Machine penalty for menial jobs
    if let Some(ego) = world.get::<crate::layer1::tech::ego_machine::EgoStat>(pop_entity) {
        if ego.value > 50.0 {
            if let Some(job) = world.get::<crate::layer1::pop::Job>(pop_entity) {
                if job.job_type == crate::layer1::utility_types::AssignmentType::FarmWorker
                    || job.job_type == crate::layer1::utility_types::AssignmentType::DeepMining
                {
                    modifier *= 0.0;
                }
            }
        }
    }

    if let Some(genemod) =
        world.get::<crate::layer1::tech::black_market_genemods::UnstableGenemod>(pop_entity)
    {
        modifier *=
            crate::layer1::tech::black_market_genemods::get_genemod_efficiency_modifier(genemod);
    }

    modifier
}
