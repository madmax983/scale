use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use ratatui::style::Color;

use crate::layer1::actions::{AssignedTo, AssignmentType};
use crate::layer1::admin::{AdminProvider, Office};
use crate::layer1::execution::components::{AtTarget, MovementTarget};
use crate::layer1::farm::Farm;
use crate::layer1::funeral::{handle_bury_corpse, Corpse, Grave};
use crate::layer1::housing::Housing;
use crate::layer1::items::{Clothing, ClothingType, Equipment, Item, Tool, ToolType, UnequipEvent};
use crate::layer1::map::GridPosition;
use crate::layer1::memory::Memories;
use crate::layer1::pop::Job;
use crate::layer1::resources::ColonyResources;
use crate::layer1::social::{handle_socialize, Tavern};
use crate::layer1::social_stratification::Prestige;
use crate::layer1::utility_types::{ActionType, PopAction};
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;

#[derive(SystemParam)]
pub struct ArrivalContext<'w, 's> {
    bio_suits: Query<'w, 's, &'static crate::layer1::economy::bio_loom::BioSuit>,
    items: Query<'w, 's, &'static crate::layer1::items::Item>,
    farms: Query<'w, 's, &'static mut Farm>,
    housing_q: Query<'w, 's, &'static mut Housing>,
    taverns: Query<'w, 's, &'static mut Tavern>,
    offices: Query<'w, 's, &'static mut Office>,
    corpses: Query<'w, 's, &'static Corpse>,
    graves: Query<'w, 's, (Entity, &'static GridPosition, &'static mut Grave)>,
    memories: Query<'w, 's, &'static mut Memories>,
    resources: ResMut<'w, ColonyResources>,
    log: Option<ResMut<'w, MessageLog>>,
    graffiti_map: Option<ResMut<'w, crate::layer1::graffiti::GraffitiMap>>,
    unequip_events: EventWriter<'w, UnequipEvent>,
    failed_events: EventWriter<'w, crate::layer1::economy::bio_loom::UnequipFailedEvent>,
    time: Res<'w, SimulationTime>,
}

/// Handles arrival at targets: assigns pops to farms/housing.
#[allow(clippy::type_complexity)]
pub fn arrival_handler_system(
    mut arrivals: Query<
        (
            Entity,
            &GridPosition,
            &MovementTarget,
            Option<&mut Equipment>,
            Option<&mut crate::layer1::chemical::ChemicalState>,
            Option<&mut crate::layer1::health::Health>,
            Option<&mut crate::layer1::stress::StressTracker>,
            Option<&crate::layer1::social::grievances::Ostracized>,
        ),
        With<AtTarget>,
    >,
    mut ctx: ArrivalContext,
    mut commands: Commands,
) {
    for (
        pop_entity,
        pop_pos,
        mt,
        mut equipment_opt,
        mut chem_opt,
        mut health_opt,
        mut stress_opt,
        ostracized_opt,
    ) in &mut arrivals
    {
        let should_remove = process_arrival(
            mt.for_action,
            pop_entity,
            mt.target_entity,
            *pop_pos,
            mt.target_position,
            &mut equipment_opt,
            &mut chem_opt,
            &mut health_opt,
            &mut stress_opt,
            &mut commands,
            &mut ctx,
            ostracized_opt.is_some(),
        );

        if should_remove {
            // One-shot errands complete on arrival: release the pop back to the
            // idle pool so the utility AI picks a fresh task next tick.
            // Otherwise the pop keeps the errand's snapshot utility forever (it
            // never decays), and ongoing work like farming can never outbid it
            // — the colony starves next to a working farm. Ongoing actions
            // (Farm, Rest, Work, ...) are intentionally left alone.
            if matches!(
                mt.for_action,
                ActionType::SatisfyHunger | ActionType::FetchTool | ActionType::FetchClothing
            ) {
                commands.entity(pop_entity).insert(PopAction {
                    current: ActionType::Idle,
                    current_utility: 0.0,
                    ticks_committed: 0,
                });
            }
            remove_movement_components(&mut commands, pop_entity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_arrival(
    action: ActionType,
    pop_entity: Entity,
    target_entity: Entity,
    pop_pos: GridPosition,
    target_pos: GridPosition,
    equipment_opt: &mut Option<Mut<Equipment>>,
    chemical_state_opt: &mut Option<Mut<crate::layer1::chemical::ChemicalState>>,
    health_opt: &mut Option<Mut<crate::layer1::health::Health>>,
    stress_opt: &mut Option<Mut<crate::layer1::stress::StressTracker>>,
    commands: &mut Commands,
    ctx: &mut ArrivalContext,
    is_ostracized: bool,
) -> bool {
    match action {
        ActionType::ConsumeChemical => {
            handle_consume_chemical_arrival(
                target_entity,
                pop_entity,
                &ctx.items,
                chemical_state_opt,
                health_opt,
                stress_opt,
                commands,
                &ctx.time,
            );
            true
        }
        ActionType::MemeticObsession => {
            // Keep the pop occupied at their current location doing nothing productive
            false // Return false so they don't pop off the queue next tick
        }
        ActionType::ScrawlMemeticSigil => {
            handle_scrawl_memetic_sigil_arrival(
                target_pos,
                ctx.graffiti_map.as_deref_mut(),
                ctx.log.as_deref_mut(),
            );
            true
        }
        ActionType::Binge => {
            handle_binge_arrival(&mut ctx.resources, ctx.log.as_deref_mut());
            true
        }
        ActionType::FetchTool => {
            handle_fetch_tool(commands, &mut ctx.resources, pop_entity, equipment_opt);
            true
        }
        ActionType::FetchClothing => {
            handle_fetch_clothing(
                commands,
                &mut ctx.resources,
                pop_entity,
                equipment_opt,
                &mut ctx.unequip_events,
                &ctx.bio_suits,
                &mut ctx.failed_events,
            );
            true
        }
        ActionType::SatisfyHunger => {
            // SatisfyHunger intentionally assigns no job and no worker slot;
            // the `consume_food_system` globally checks for hungry pops.
            true
        }
        ActionType::SatisfyRest => {
            handle_rest_arrival(pop_entity, target_entity, &mut ctx.housing_q, commands);
            true
        }
        ActionType::Socialize | ActionType::Gossip => {
            handle_socialize(
                commands,
                &mut ctx.taverns,
                target_entity,
                pop_entity,
                is_ostracized,
            );
            true
        }
        ActionType::SeekMedicalCare => {
            handle_seek_medical_care_arrival(commands, pop_entity, target_entity)
        }
        ActionType::Research => handle_research_arrival(commands, pop_entity, target_entity),
        ActionType::Farm => {
            handle_farm_arrival(commands, &mut ctx.farms, pop_entity, target_entity)
        }
        ActionType::Admin => {
            handle_admin_arrival(commands, &mut ctx.offices, pop_entity, target_entity)
        }
        ActionType::BuryCorpse => {
            handle_bury_corpse(
                commands,
                &ctx.corpses,
                &mut ctx.graves,
                &mut ctx.memories,
                &ctx.time,
                target_entity,
                pop_entity,
                pop_pos,
            );
            true
        }
        ActionType::Work | ActionType::Repair | ActionType::Haul | ActionType::Tame => {
            // Work/Repair/Haul/Tame is handled by their respective systems
            // Just keep the AtTarget marker for that system
            false
        }
        _ => true,
    }
}

fn assign_pop(
    commands: &mut Commands,
    pop_entity: Entity,
    target_entity: Entity,
    assignment_type: AssignmentType,
) -> bool {
    let mut entity_cmds = commands.entity(pop_entity);
    entity_cmds.insert(AssignedTo {
        entity: target_entity,
        assignment_type,
    });

    // If this assignment counts as a Job (persistent employment), update the Job component.
    match assignment_type {
        AssignmentType::FarmWorker
        | AssignmentType::LibraryWorker
        | AssignmentType::ObservatoryWorker => {
            entity_cmds.insert((
                Job {
                    workplace: target_entity,
                    job_type: assignment_type,
                },
                Prestige::from_job(assignment_type),
            ));
        }
        AssignmentType::Administrator => {
            entity_cmds.insert((
                Job {
                    workplace: target_entity,
                    job_type: assignment_type,
                },
                Prestige::from_job(assignment_type),
                AdminProvider { amount: 5.0 },
            ));
        }
        AssignmentType::HousingResident
        | AssignmentType::TavernVisitor
        | AssignmentType::Patient
        | AssignmentType::Funeral
        | AssignmentType::Surgery
        | AssignmentType::Sheriff => {
            // These are not jobs, so we don't update Job component.
            // The pop keeps their previous job (if any).
        }
        AssignmentType::DeepMining | AssignmentType::RationingBureaucrat => {
            entity_cmds.insert((
                Job {
                    workplace: target_entity,
                    job_type: assignment_type,
                },
                Prestige::from_job(assignment_type),
            ));
        }
    }

    true
}

fn remove_movement_components(commands: &mut Commands, pop_entity: Entity) {
    commands
        .entity(pop_entity)
        .remove::<MovementTarget>()
        .remove::<AtTarget>();
}

fn handle_binge_arrival(resources: &mut ColonyResources, log: Option<&mut MessageLog>) {
    let amount_needed = 5.0;
    if resources.food >= amount_needed {
        resources.food -= amount_needed;
    } else {
        let taken = resources.food;
        resources.food = 0.0;
        let remaining = amount_needed - taken;
        if remaining > 0.0 {
            // Subtract remaining from rations
            resources.rations = (resources.rations - remaining).max(0.0);
        }
    }

    if let Some(log) = log {
        log.add("Pop is binge eating!");
    }
}

// --- Consolidated Handler Functions ---

pub fn handle_fetch_tool(
    commands: &mut Commands,
    resources: &mut ColonyResources,
    pop_entity: Entity,
    equipment_opt: &mut Option<Mut<Equipment>>,
) {
    if resources.tools >= 1.0 {
        resources.tools -= 1.0;
        let tool_history = crate::layer1::heirloom::ToolHistory::default();
        let tool_entity = commands
            .spawn((
                Item::default(),
                Tool {
                    tool_type: ToolType::Pickaxe, // Generic for now
                    durability: 100.0,
                    max_durability: 100.0,
                },
                tool_history,
            ))
            .id();

        if let Some(eq) = equipment_opt {
            eq.tool = Some(tool_entity);
        } else {
            commands.entity(pop_entity).insert(Equipment {
                tool: Some(tool_entity),
                ..Default::default()
            });
        }
    }
}

pub fn handle_fetch_clothing(
    commands: &mut Commands,
    resources: &mut ColonyResources,
    pop_entity: Entity,
    equipment_opt: &mut Option<Mut<Equipment>>,
    unequip_events: &mut EventWriter<UnequipEvent>,
    bio_suits: &Query<&crate::layer1::economy::bio_loom::BioSuit>,
    failed_events: &mut EventWriter<crate::layer1::economy::bio_loom::UnequipFailedEvent>,
) {
    if resources.clothing >= 1.0 {
        let mut is_upgrade = false;
        if let Some(eq) = equipment_opt {
            if eq.body.is_some() {
                if let Some(old_entity) = eq.body {
                    if let Ok(suit) = bio_suits.get(old_entity) {
                        if suit.attachment_level > 75.0 {
                            failed_events.send(
                                crate::layer1::economy::bio_loom::UnequipFailedEvent {
                                    entity: pop_entity,
                                    reason:
                                        "The Bio-Suit has fused with the host's nervous system."
                                            .to_string(),
                                },
                            );
                            return; // Block unequip
                        }
                    }
                }
                is_upgrade = true;
                if let Some(old_entity) = eq.body {
                    unequip_events.send(UnequipEvent {
                        actor: pop_entity,
                        item: old_entity,
                        slot: "body".to_string(),
                    });
                    commands.entity(old_entity).despawn();
                }
            }
        }

        resources.clothing -= 1.0;

        let (clothing_type, insulation) = if is_upgrade {
            (ClothingType::Parka, 2.0)
        } else {
            (ClothingType::Tunic, 1.0)
        };

        let clothing_entity = commands
            .spawn((
                Item::default(),
                Clothing {
                    clothing_type,
                    insulation,
                    durability: 100.0,
                    max_durability: 100.0,
                },
            ))
            .id();

        if let Some(eq) = equipment_opt {
            eq.body = Some(clothing_entity);
        } else {
            commands.entity(pop_entity).insert(Equipment {
                body: Some(clothing_entity),
                ..Default::default()
            });
        }
    }
}

fn handle_rest_arrival(
    pop_entity: Entity,
    target_entity: Entity,
    housing: &mut Query<&mut Housing>,
    commands: &mut Commands,
) {
    if let Ok(mut house) = housing.get_mut(target_entity) {
        if house.residents.len() < house.capacity {
            house.residents.push(pop_entity);
            commands.entity(pop_entity).insert(AssignedTo {
                entity: target_entity,
                assignment_type: AssignmentType::HousingResident,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_consume_chemical_arrival(
    target_entity: Entity,
    pop_entity: Entity,
    items: &Query<&crate::layer1::items::Item>,
    chemical_state_opt: &mut Option<Mut<crate::layer1::chemical::ChemicalState>>,
    health_opt: &mut Option<Mut<crate::layer1::health::Health>>,
    stress_opt: &mut Option<Mut<crate::layer1::stress::StressTracker>>,
    commands: &mut Commands,
    time: &Res<SimulationTime>,
) {
    if let Ok(item) = items.get(target_entity) {
        let chem_type = match item.item_type {
            crate::layer1::items::ItemType::Stim => {
                Some(crate::layer1::chemical::ChemicalType::Stim)
            }
            crate::layer1::items::ItemType::Sedative => {
                Some(crate::layer1::chemical::ChemicalType::Sedative)
            }
            _ => None,
        };

        if let Some(ct) = chem_type {
            let tick = time.tick;
            if let Some(state) = chemical_state_opt {
                crate::layer1::chemical::consume_chemical_logic(
                    state,
                    ct,
                    tick,
                    health_opt.as_deref_mut(),
                    stress_opt.as_deref_mut(),
                );
            } else {
                let mut state = crate::layer1::chemical::ChemicalState::default();
                crate::layer1::chemical::consume_chemical_logic(
                    &mut state,
                    ct,
                    tick,
                    health_opt.as_deref_mut(),
                    stress_opt.as_deref_mut(),
                );
                commands.entity(pop_entity).insert(state);
            }
            commands.entity(target_entity).despawn();
        }
    }
}

fn handle_scrawl_memetic_sigil_arrival(
    target_pos: GridPosition,
    graffiti_map: Option<&mut crate::layer1::graffiti::GraffitiMap>,
    log: Option<&mut MessageLog>,
) {
    if let Some(map) = graffiti_map {
        use crate::layer1::graffiti::{Graffiti, GraffitiType};
        map.markings.insert(
            (target_pos.x, target_pos.y),
            Graffiti {
                graffiti_type: GraffitiType::MemeticSigil,
                decay: 500.0,
                modifier: -0.2, // Strong debuff
            },
        );
        if let Some(l) = log {
            l.add_colored("A Memetic Sigil has been scrawled on a wall!", Color::Red);
        }
    }
}

fn handle_farm_arrival(
    commands: &mut Commands,
    farms: &mut Query<&mut Farm>,
    pop_entity: Entity,
    target_entity: Entity,
) -> bool {
    #[allow(clippy::collapsible_if)]
    if let Ok(mut farm) = farms.get_mut(target_entity) {
        if farm.workers.len() < farm.capacity {
            farm.workers.push(pop_entity);
            assign_pop(
                commands,
                pop_entity,
                target_entity,
                AssignmentType::FarmWorker,
            );
        }
    }
    true
}

fn handle_admin_arrival(
    commands: &mut Commands,
    offices: &mut Query<&mut Office>,
    pop_entity: Entity,
    target_entity: Entity,
) -> bool {
    if let Ok(mut office) = offices.get_mut(target_entity) {
        if !office.workers.contains(&pop_entity) {
            office.workers.push(pop_entity);
        }
    }
    assign_pop(
        commands,
        pop_entity,
        target_entity,
        AssignmentType::Administrator,
    )
}

fn handle_seek_medical_care_arrival(
    commands: &mut Commands,
    pop_entity: Entity,
    target_entity: Entity,
) -> bool {
    assign_pop(commands, pop_entity, target_entity, AssignmentType::Patient)
}

fn handle_research_arrival(
    commands: &mut Commands,
    pop_entity: Entity,
    target_entity: Entity,
) -> bool {
    assign_pop(
        commands,
        pop_entity,
        target_entity,
        AssignmentType::LibraryWorker,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::admin::Office;
    use crate::layer1::chemical::{ChemicalState, ChemicalType};
    use crate::layer1::economy::items::{Item, ItemType};
    use crate::layer1::graffiti::{GraffitiMap, GraffitiType};
    use crate::layer1::housing::Housing;
    use crate::layer1::map::GridPosition;
    use crate::shared::log::MessageLog;
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_handle_admin_arrival() {
        let mut world = World::new();
        let target = world
            .spawn(Office {
                workers: vec![],
                capacity: 2,
            })
            .id();
        let pop = world.spawn(()).id();

        let _ = world.run_system_once(
            move |mut commands: Commands, mut offices: Query<&mut Office>| {
                handle_admin_arrival(&mut commands, &mut offices, pop, target);
            },
        );

        let office = world.get::<Office>(target).unwrap();
        assert_eq!(office.workers.len(), 1);
        assert_eq!(office.workers[0], pop);
    }

    #[test]
    fn test_handle_scrawl_memetic_sigil_arrival() {
        let pos = GridPosition { x: 5, y: 5 };
        let mut map = GraffitiMap::default();
        let mut log = MessageLog::default();

        handle_scrawl_memetic_sigil_arrival(pos, Some(&mut map), Some(&mut log));

        assert!(map.markings.contains_key(&(5, 5)));
        assert_eq!(
            map.markings.get(&(5, 5)).unwrap().graffiti_type,
            GraffitiType::MemeticSigil
        );
        assert!(!log.messages.is_empty());
    }

    #[test]
    fn test_handle_consume_chemical_arrival() {
        let mut world = World::new();
        let time = SimulationTime {
            tick: 100,
            ..SimulationTime::default()
        };
        world.insert_resource(time);
        let target = world
            .spawn(Item {
                item_type: ItemType::Stim,
            })
            .id();
        let pop = world.spawn(()).id();

        let _ = world.run_system_once(
            move |mut commands: Commands, items: Query<&Item>, time: Res<SimulationTime>| {
                let mut chem_opt = None;
                let mut health_opt = None;
                let mut stress_opt = None;
                handle_consume_chemical_arrival(
                    target,
                    pop,
                    &items,
                    &mut chem_opt,
                    &mut health_opt,
                    &mut stress_opt,
                    &mut commands,
                    &time,
                );
            },
        );

        assert!(world.get_entity(target).is_err());
        let chemstate = world.get::<ChemicalState>(pop).unwrap();
        assert!(chemstate
            .active_effects
            .iter()
            .any(|e| e.chemical == ChemicalType::Stim));
    }

    #[test]
    fn test_handle_rest_arrival() {
        let mut world = World::new();
        let target = world
            .spawn(Housing {
                capacity: 2,
                residents: vec![],
            })
            .id();
        let pop = world.spawn(()).id();

        let _ = world.run_system_once(
            move |mut commands: Commands, mut housing: Query<&mut Housing>| {
                handle_rest_arrival(pop, target, &mut housing, &mut commands);
            },
        );

        let housing = world.get::<Housing>(target).unwrap();
        assert_eq!(housing.residents.len(), 1);
        assert_eq!(housing.residents[0], pop);
    }

    #[test]
    fn test_handle_binge_arrival() {
        let mut log = MessageLog::default();
        let mut resources = crate::layer1::resources::ColonyResources {
            food: 10.0,
            rations: 0.0,
            ..Default::default()
        };
        handle_binge_arrival(&mut resources, Some(&mut log));
        assert_eq!(resources.food, 5.0);
        assert!(!log.messages.is_empty());
        assert!(log.messages[0].text.contains("binge eating"));

        let mut resources2 = crate::layer1::resources::ColonyResources {
            food: 2.0,
            rations: 5.0,
            ..Default::default()
        };
        handle_binge_arrival(&mut resources2, None);
        assert_eq!(resources2.food, 0.0);
        assert_eq!(resources2.rations, 2.0);
    }

    #[test]
    fn test_handle_fetch_tool() {
        let mut world = World::new();
        let pop = world.spawn(()).id();
        let mut resources = crate::layer1::resources::ColonyResources {
            tools: 2.0,
            ..Default::default()
        };

        let _ = world.run_system_once(move |mut commands: Commands| {
            let mut equip_opt = None;
            handle_fetch_tool(&mut commands, &mut resources, pop, &mut equip_opt);
        });

        let equip = world.get::<crate::layer1::Equipment>(pop).unwrap();
        assert!(equip.tool.is_some());
    }
}
