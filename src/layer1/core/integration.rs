//! Integration systems that bridge multiple domains in Layer 1.

use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::cybernetics::MissingLimb;
use crate::layer1::edicts::{ColonyPolicies, Policy};
use crate::layer1::environment::hazards::AmputationEvent;
use crate::layer1::factions::Factions;
use crate::layer1::fire::Fire;
use crate::layer1::geodetic::GolemFormedEvent;
use crate::layer1::health::Health;
use crate::layer1::inspector::{Inspector, Reported};
use crate::layer1::medical::PatientTreated;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::nature::temperature::ThermalDamageEvent;
use crate::layer1::needs::Needs;
use crate::layer1::notifications::NotificationQueue;
use crate::layer1::petrification::PopPetrifiedEvent;
use crate::layer1::pop::{Pop, PopBorn, PopDied, PopName};
use crate::layer1::resources::ColonyResources;
use crate::layer1::rumor::{Knowledge, Rumor, RumorTopic};
use crate::layer1::social::placebo::{ActivePlacebo, PlaceboProtocol};
use crate::layer1::utility_types::{ActionType, PopAction};
use crate::layer1::vermin::VerminState;
use crate::shared::colony::ColonyName;
use crate::shared::log::MessageLog;
use crate::shared::narrative::{NarrativeContext, NarrativeGenerator};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::prelude::*;
use ratatui::style::Color;

use std::collections::HashSet;

/// Bridges `KineticStrikeEvent` to `AddChronicleEvent` (Chronicle).
pub fn kinetic_strike_chronicle_bridge(
    mut strike_events: EventReader<crate::layer1::geology::subsurface::KineticStrikeEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in strike_events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "Kinetic Strike! A heavy orbital payload struck ({}, {}).",
                event.target_x, event.target_y
            ),
            importance: EventImportance::Major,
        });
    }
}

/// INT-1335: Bridges `ThermalDamageEvent` (The Deep Chill) to `AddChronicleEvent`
pub fn thermal_damage_chronicle_bridge(
    mut events: EventReader<ThermalDamageEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    time: Res<SimulationTime>,
    mut last_recorded_tick: Local<u64>,
) {
    let mut any_damage = false;
    if events.read().next().is_some() {
        any_damage = true;
    }

    if any_damage && time.tick > *last_recorded_tick + TICKS_PER_YEAR {
        *last_recorded_tick = time.tick;
        chronicle.send(AddChronicleEvent {
            text: "The Deep Chill took its toll. Pops are suffering from hypothermia.".to_string(),
            importance: EventImportance::Standard,
        });
    }
}

pub fn bureaucratic_strike_chronicle_bridge(
    red_tape: Option<Res<crate::layer1::social::bureaucratic_strike::RedTapeEvent>>,
    mut was_active: Local<bool>,
    mut chronicle: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    if let Some(red_tape) = red_tape {
        if red_tape.active && !*was_active {
            chronicle.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: "A massive bureaucratic strike has crippled administration. Expect severe red tape!".to_string(),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
            *was_active = true;
        } else if !red_tape.active && *was_active {
            chronicle.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: "The bureaucratic strike has ended. Paperwork is flowing again.".to_string(),
                importance: crate::layer1::core::chronicle::EventImportance::Standard,
            });
            *was_active = false;
        }
    }
}

/// INT-1023: Bridges `Added<TemporalFugue>` to `AddChronicleEvent`
pub fn temporal_fugue_chronicle_bridge(
    query: Query<(), Added<crate::layer1::mind::temporal_fugue::TemporalFugue>>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in query.iter() {
        chronicle_events.send(AddChronicleEvent {
            text: "A highly skilled worker has entered a Temporal Fugue trance. They are working at impossible speeds, entirely oblivious to their own mortal needs.".to_string(),
            importance: EventImportance::Minor,
        });
    }
}

/// Marker component to avoid duplicate chronicle entries for blueprint corruption.
#[derive(Component)]
pub struct ChronicleCorruptedLogged;

/// INT-493: Bridges `Changed<Blueprint>` to `AddChronicleEvent` when corrupted
#[derive(bevy_ecs::query::QueryData)]
pub struct LivingArchiveQuery {
    entity: Entity,
    blueprint: &'static crate::layer1::tech::living_archive::Blueprint,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct LivingArchiveFilter {
    _changed: Changed<crate::layer1::tech::living_archive::Blueprint>,
    _without: Without<ChronicleCorruptedLogged>,
}

pub fn living_archive_chronicle_bridge(
    mut commands: Commands,
    query: Query<LivingArchiveQuery, LivingArchiveFilter>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for item in query.iter() {
        if item.blueprint.is_corrupted {
            let entity = item.entity;
            commands.entity(entity).insert(ChronicleCorruptedLogged);
            chronicle_events.send(AddChronicleEvent {
                text: "A living Flesh-Server has suffered acute stress, permanently corrupting the stored technological blueprints and introducing horrifying secondary effects.".to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// INT-1024: Bridges `Added<ArtWork>` to `AddChronicleEvent` when quality is Masterpiece
pub fn artists_muse_chronicle_bridge(
    query: Query<
        &crate::layer1::artists_muse::ArtWork,
        Added<crate::layer1::artists_muse::ArtWork>,
    >,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for artwork in query.iter() {
        if artwork.quality == crate::layer1::crafting::Quality::Masterpiece {
            chronicle_events.send(AddChronicleEvent {
                text: format!("A tortured artist has created a masterpiece born from suffering. The new {} is both disturbing and brilliant.", artwork.item_type),
                importance: EventImportance::Major,
            });
        }
    }
}

/// INT-680: Bridges MemoryBlackoutEvent to AddChronicleEvent (Chronicle).
pub fn memory_blackout_chronicle_bridge(
    mut events: EventReader<crate::layer1::psychology::memory_blackout::MemoryBlackoutEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "A Memory Blackout occurred. All colony memories and relationships formed between cycle {} and {} were erased.",
                event.start_time, event.end_time
            ),
        });
    }
}

pub fn crop_mutation_mycelial_bridge(
    mut events: EventReader<crate::layer1::biology::genetics::crop_modification::CropMutationEvent>,
    mut contamination: EventWriter<crate::layer1::logistics::mycelial::ContaminationEvent>,
    crops: Query<
        &crate::layer1::core::map::GridPosition,
        With<crate::layer1::biology::genetics::crop_modification::Crop>,
    >,
) {
    for event in events.read() {
        if let crate::layer1::biology::genetics::crop_modification::MutationType::ToxicSpores =
            event.mutation_type
        {
            if let Ok(pos) = crops.get(event.crop_entity) {
                contamination
                    .send(crate::layer1::logistics::mycelial::ContaminationEvent { source: *pos });
            }
        }
    }
}

pub fn mycelial_chronicle_bridge(
    mut events: EventReader<crate::layer1::logistics::mycelial::ContaminationEvent>,
    mut chronicle: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The subterranean mycelial network has been contaminated! The pathogen is spreading rapidly.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// Bridges `TemporalChamber` to `ColonyResources` (Fuel) and `AddChronicleEvent` for shockwave.
pub fn temporal_chamber_power_bridge_system(
    mut resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
    mut chambers: Query<&mut crate::layer1::temporal_chamber::TemporalChamber>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for mut chamber in chambers.iter_mut() {
        if chamber.active {
            if resources.fuel >= chamber.energy_cost {
                resources.fuel -= chamber.energy_cost;
            } else {
                chamber.active = false;
                chronicle_events.send(AddChronicleEvent {
                    text: "Temporal shockwave released due to power failure in echo chamber!"
                        .to_string(),
                    importance: EventImportance::Major,
                });
            }
        }
    }
}

/// Bridges `PopPetrifiedEvent` (Petrification Sickness) to `AddChronicleEvent` (Chronicle).
pub fn petrification_chronicle_bridge(
    mut events: EventReader<PopPetrifiedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "A colonist has fully petrified. {} now stands as a morbid monument to our greed.",
                ev.pop_name
            ),
            importance: EventImportance::Major,
        });
    }
}

pub fn access_denied_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer1::administration::edicts::AccessDeniedEvent,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
            text: format!("Access Denied: {}", event.reason),
        });
    }
}

/// Bridges `TetherSnapEvent` (Orbital Tether) to `AddChronicleEvent` (Chronicle).
pub fn tether_snap_chronicle_bridge(
    mut events: EventReader<crate::layer1::environment::orbital_tether::TetherSnapEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "The Sky Fell. The orbital tether was severed, its massive cable obliterating everything in its path.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Bridges `Awakened` component addition (Machine Awakening) to `AddChronicleEvent` (Chronicle).
pub fn bot_awakening_chronicle_bridge(
    query: Query<Entity, Added<crate::layer1::tech::machine_awakening::Awakened>>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in query.iter() {
        chronicle_events.send(AddChronicleEvent {
            text: "A tool has asked 'Why?'. A Machine Awakening has occurred.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Creates chronicle entries from [`PopDied`] events.
///
/// Bridges the Pop system (Death) and Chronicle system (History).
pub fn pop_death_chronicle_bridge(
    mut events: EventReader<PopDied>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    generator: Res<NarrativeGenerator>,
    colony: Res<ColonyName>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        let year = (1 + time.tick / TICKS_PER_YEAR).to_string();

        let mut ctx = NarrativeContext::new();
        ctx.insert("COLONY", &colony.name);
        ctx.insert("YEAR", &year);
        ctx.insert("NAME", &event.name);
        ctx.insert("REASON", &event.reason);

        let text = generator
            .generate("POP_DEATH", &ctx)
            .unwrap_or_else(|_| format!("{} has died. Cause: {}", event.name, event.reason));

        chronicle_events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

pub fn hologram_failure_chronicle_bridge(
    mut events: EventReader<crate::layer1::hologram::HologramFailureEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "A holographic facade failed at ({}, {}). The hard-light lie shattered.",
                event.position.x, event.position.y
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Creates rumors from significant chronicle events.
///
/// Bridges the Chronicle system (History) and Rumor system (Social).
pub fn chronicle_rumor_bridge_system(
    mut events: EventReader<AddChronicleEvent>,
    mut query: Query<(Entity, &mut Knowledge), With<Pop>>,
    time: Res<SimulationTime>,
) {
    let mut rng = rand::thread_rng();

    for event in events.read() {
        if matches!(
            event.importance,
            EventImportance::Major | EventImportance::Legendary
        ) {
            // Reservoir sampling to pick 3 random witnesses without collecting all entities into a Vec
            let mut witnesses = Vec::with_capacity(3);
            for (count, (entity, _)) in query.iter().enumerate() {
                if count < 3 {
                    witnesses.push(entity);
                } else {
                    let j = rng.gen_range(0..=count);
                    if j < 3 {
                        witnesses[j] = entity;
                    }
                }
            }

            if witnesses.is_empty() {
                continue;
            }

            // ⚡ Bolt Optimization: Only allocate the base rumor if we actually found witnesses.
            // By deferring the `.clone()` on the event text until after the early return,
            // we avoid generating the string allocations completely when no pops exist to hear it.
            let rumor = Rumor {
                topic: RumorTopic::EventNews(event.text.clone()),
                source: Entity::PLACEHOLDER, // Originated from "The World"
                timestamp: time.tick,
                strength: 1.0,
            };

            for witness in witnesses {
                if let Ok((_, mut knowledge)) = query.get_mut(witness) {
                    knowledge.add_rumor(rumor.clone());
                }
            }
        }
    }
}

/// Bridges `Needs` system to `BlackMarket` system.
///
/// Updates `ColonyStats.unmet_luxury` by counting pops with low leisure needs.
pub fn update_unmet_luxury_system(
    mut stats: ResMut<crate::layer1::black_market::ColonyStats>,
    pops: Query<&crate::layer1::needs::Needs, With<crate::layer1::pop::Pop>>,
) {
    let unmet_count = pops.iter().filter(|needs| needs.leisure < 30.0).count();
    stats.unmet_luxury = unmet_count.try_into().unwrap_or(u32::MAX);
}

/// Bridges `SacrilegeEvent` to `Unrest` and `Chronicle`.
///
/// Increases global unrest and logs a major event when a grave is built over.
pub fn sacrilege_unrest_bridge(
    mut events_in: EventReader<crate::layer1::ancestral_graves::SacrilegeEvent>,
    mut events_out: EventWriter<AddChronicleEvent>,
    mut unrest: ResMut<crate::layer1::unrest::Unrest>,
) {
    for _event in events_in.read() {
        unrest.level += 10.0;
        unrest.level = unrest.level.min(100.0);

        events_out.send(AddChronicleEvent {
            text: "A grave was desecrated. The colony is in uproar.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Bridges `GreatWorkCompletedEvent` to the `Chronicle` system.
///
/// Records the completion of a Great Work as a Legendary event.
pub fn great_work_chronicle_bridge(
    mut events_in: EventReader<crate::layer1::construction::GreatWorkCompletedEvent>,
    mut events_out: EventWriter<AddChronicleEvent>,
) {
    for event in events_in.read() {
        events_out.send(AddChronicleEvent {
            text: format!("The colony has completed a Great Work: {}.", event.name),
            importance: EventImportance::Legendary,
        });
    }
}

/// Issues `ActivePlacebo` entities for active `Policy::Placebo` edicts if they don't already exist.
/// Runs in `Observation` phase.
pub fn issue_placebo_from_edict_system(
    mut commands: Commands,
    policies: Option<Res<ColonyPolicies>>,
    existing_placebos: Query<&ActivePlacebo>,
) {
    let Some(policies) = policies else { return };

    // Find which Placebos are currently active in the ECS world
    let mut active_protocols = std::collections::HashSet::new();
    for placebo in existing_placebos.iter() {
        active_protocols.insert(placebo.protocol);
    }

    // Iterate over active policies to find placebos
    for policy in policies.active_policies.iter() {
        if let Policy::Placebo(protocol) = policy {
            // If the policy is active but the placebo entity doesn't exist, spawn it
            if !active_protocols.contains(protocol) {
                // Determine relief values based on protocol (could be configurable)
                let (duration, stress_relief) = match protocol {
                    PlaceboProtocol::FakeReinforcements => (100.0, 20.0),
                    PlaceboProtocol::VitaminX => (100.0, 15.0),
                    PlaceboProtocol::SafetyInspection => (100.0, 25.0),
                };

                commands.spawn(ActivePlacebo {
                    protocol: *protocol,
                    duration,
                    stress_relief,
                    revealed: false,
                    applied: false,
                });
            }
        }
    }
}

/// Applies social debt when a doctor treats a patient.
///
/// Bridges Medical system (Treatment) and Social system (Debt).
pub fn medical_debt_bridge_system(
    mut events: EventReader<PatientTreated>,
    mut social_debt_events: EventWriter<crate::layer1::social::FavorChange>,
    _doctors: Query<(Entity, &crate::layer1::pop::Job)>,
) {
    for event in events.read() {
        // Find doctors at this hospital
        // Razor: Doctor job type removed as dead code.
        // Logic removed until doctors are implemented properly.
        let hospital_doctors: Vec<Entity> = Vec::new();

        if let Some(&doctor) = hospital_doctors.first() {
            social_debt_events.send(crate::layer1::social::FavorChange {
                debtor: event.patient,
                creditor: doctor,
                amount: event.amount,
                reason: "Medical Treatment".to_string(),
            });
        }
    }
}

/// Bridges Atmosphere (Environment) and Pressure (Environment).
///
/// If a tile is a vacuum (low pressure), any pollution should be rapidly vented/cleared.
pub fn vacuum_clears_pollution_system(
    mut trace_gas: ResMut<crate::layer1::nature::atmospheric_empathy::TraceGasGrid>,
    mut atmosphere: ResMut<crate::layer1::atmosphere::AtmosphereGrid>,
    pressure: Res<crate::layer1::pressure::PressureGrid>,
) {
    // Parallel iteration would be better if these were huge, but simple loop is fine for MVP
    const VACUUM_THRESHOLD: f32 = 0.1;

    // If dimensions match, proceed
    if atmosphere.width != pressure.width
        || atmosphere.height != pressure.height
        || trace_gas.width != pressure.width
        || trace_gas.height != pressure.height
    {
        return;
    }

    for i in 0..atmosphere.values.len() {
        // If pressure is near vacuum, clear pollution
        if pressure.values[i] < VACUUM_THRESHOLD {
            atmosphere.values[i] = 0.0;
            trace_gas.euphoric[i] = 0.0;
            trace_gas.fear[i] = 0.0;
            trace_gas.rage[i] = 0.0;
        }
    }
}

/// Applies morale penalties based on faction satisfaction.
///
/// Bridges the Faction system (Social) and Pop Needs system (Psychology).
pub fn faction_satisfaction_morale_bridge(
    factions: Res<Factions>,
    mut query: Query<(&crate::layer1::factions::FactionMember, &mut Needs)>,
) {
    for (member, mut needs) in &mut query {
        if let Some(data) = member.faction_id.and_then(|id| factions.get(id)) {
            // If satisfaction < 0.9, apply penalty
            // Penalty scales: 0.9 -> 0.0, 0.0 -> 0.001 (approx 0.001)
            // Let's use 0.001 per tick for max dissatisfaction (0.0)
            if data.satisfaction < 0.9 {
                let penalty = (0.9 - data.satisfaction) * 0.001;
                needs.leisure = (needs.leisure - penalty).max(0.0);
            }
        }
    }
}

/// Applies `DisgustedByVermin` memory to pops if vermin severity is high.
///
/// Bridges the Vermin system (Environment) and Memory system (Psychology).
pub fn vermin_morale_system(
    vermin: Res<VerminState>,
    mut query: Query<&mut Memories, With<Pop>>,
    time: Res<SimulationTime>,
) {
    if vermin.severity < 50.0 {
        return;
    }

    // Chance to apply memory scales with severity
    // 50.0 -> 0.0
    // 100.0 -> 0.10 (10% chance per tick)
    let chance = (vermin.severity - 50.0) / 50.0 * 0.10;

    query.par_iter_mut().for_each(|mut memories| {
        let mut rng = rand::thread_rng();
        if rng.r#gen::<f32>() < chance {
            // Check if already has memory to avoid stacking
            let has_memory = memories
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::DisgustedByVermin);

            if !has_memory {
                memories.add(MemoryType::DisgustedByVermin, time.tick);
            }
        }
    });
}

/// Applies damage to pops standing on fire.
///
/// Bridges the Fire system (Environment) and Pop Health system (Simulation).
pub fn fire_damage_pops_system(
    fire_query: Query<(&GridPosition, &Fire)>,
    mut pop_query: Query<(&GridPosition, &mut Health), With<Pop>>,
    mut fire_tiles: Local<HashSet<GridPosition>>,
) {
    // 1. Identify dangerous tiles
    // ⚡ Bolt Optimization: Reuse Local HashSet to prevent per-frame allocation
    fire_tiles.clear();
    fire_tiles.extend(fire_query.iter().map(|(pos, _)| *pos));

    if fire_tiles.is_empty() {
        return;
    }

    // 2. Apply damage to pops on those tiles
    for (pos, mut health) in &mut pop_query {
        if fire_tiles.contains(pos) {
            // Apply 5.0 damage per tick (20 ticks to die)
            let damage = 5.0;
            health.take_damage(damage);
        }
    }
}

fn grant_inspector_memory(
    pop_memories: &mut Query<&mut Memories, With<Pop>>,
    memory_type: MemoryType,
    tick: u64,
) {
    pop_memories.par_iter_mut().for_each(|mut memories| {
        if !memories.items.iter().any(|m| m.memory_type == memory_type) {
            memories.add(memory_type, tick);
        }
    });
}

/// Applies consequences of an Inspector's report.
///
/// Bridges the Inspector system (Observation) and Pop/Resources system (Psychology/Economy).
#[allow(clippy::cast_precision_loss)]
pub fn inspector_outcome_bridge_system(
    inspectors: Query<&Inspector, Added<Reported>>,
    mut pop_memories: Query<&mut Memories, With<Pop>>,
    mut resources: ResMut<ColonyResources>,
    mut log: Option<ResMut<MessageLog>>,
    time: Res<SimulationTime>,
) {
    for inspector in &inspectors {
        let avg_score = if inspector.samples_taken > 0 {
            inspector.beauty_score / inspector.samples_taken as f32
        } else {
            0.0
        };

        // Determine outcome
        if avg_score > 5.0 {
            // S Grade
            // Grant Knowledge
            resources.add_knowledge(10.0);

            // Add Memory to ALL pops
            grant_inspector_memory(&mut pop_memories, MemoryType::InspectorImpressed, time.tick);

            if let Some(log) = log.as_mut() {
                log.add(
                    "Inspector Report: The colony is a shining beacon! (+10 Knowledge, Pop Morale Boost)",
                );
            }
        } else if avg_score > 2.0 {
            // A Grade
            resources.add_knowledge(5.0);

            grant_inspector_memory(&mut pop_memories, MemoryType::InspectorImpressed, time.tick);

            if let Some(log) = log.as_mut() {
                log.add("Inspector Report: An exemplary colony. (+5 Knowledge, Pop Morale Boost)");
            }
        } else if avg_score < -2.0 {
            // F Grade
            grant_inspector_memory(
                &mut pop_memories,
                MemoryType::InspectorDisappointed,
                time.tick,
            );

            if let Some(log) = log.as_mut() {
                log.add("Inspector Report: Disgraceful conditions! (Pop Morale Penalty)");
            }
        }
    }
}

/// Creates chronicle entries from [`crate::layer1::heirloom::RetrogradeEngineeringEvent`] events.
///
/// Bridges Retrograde Engineering (Heirloom) and Chronicle system (History).
pub fn retrograde_chronicle_bridge(
    mut events: EventReader<crate::layer1::heirloom::RetrogradeEngineeringEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        let text = format!(
            "Sacrificed {} for {:.0} Knowledge. The past fuels the future.",
            event.building_label, event.knowledge_gained
        );

        chronicle_events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

/// Spawns fire when a grid component is overloaded.
///
/// Bridges Energy (Overload) and Environment (Fire).
pub fn grid_overload_fire_bridge(
    mut events: EventReader<crate::layer1::energy::GridOverloadEvent>,
    mut commands: Commands,
    grid_positions: Query<&GridPosition>,
    existing_fires: Query<&GridPosition, With<crate::layer1::fire::Fire>>,
) {
    for event in events.read() {
        if let Ok(pos) = grid_positions.get(event.victim) {
            // Check if fire already exists at this position
            let already_burning = existing_fires.iter().any(|p| *p == *pos);

            if !already_burning {
                commands.spawn((crate::layer1::fire::Fire::default(), *pos));
            }
        }
    }
}

/// Handles amputation events by applying `MissingLimb` component and memory.
///
/// Bridges Hazards (Accident) and Cybernetics/Memory (Consequence).
pub fn amputation_handler_system(
    mut events: EventReader<AmputationEvent>,
    mut commands: Commands,
    mut memories_query: Query<&mut Memories>,
    mut log: Option<ResMut<MessageLog>>,
    time: Res<SimulationTime>,
    pop_query: Query<&crate::layer1::pop::Pop>,
) {
    for event in events.read() {
        let entity = event.entity;

        // Verify entity is a Pop (just in case)
        if pop_query.get(entity).is_err() {
            continue;
        }

        // 1. Add MissingLimb Component
        commands
            .entity(entity)
            .insert(MissingLimb { severity: 0.5 });

        // 2. Add Memory
        if let Ok(mut memories) = memories_query.get_mut(entity) {
            memories.add(MemoryType::LostLimb, time.tick);
        }

        // 3. Log
        if let Some(ref mut l) = log {
            l.add_colored(
                "CRITICAL: A colonist has lost a limb in a terrible accident!",
                Color::Red,
            );
        }
    }
}

/// Spawns drones at active `DroneHubs` if the population is low.
///
/// Accelerates decay of perishable items based on vermin severity.
///
/// Bridges Vermin system (Environment) and Spoilage system (Items).
pub fn vermin_item_rot_system(
    vermin: Res<VerminState>,
    mut query: Query<&mut crate::layer1::spoilage::Perishable>,
) {
    let modifier = crate::layer1::vermin::calculate_spoilage_modifier(&vermin);
    // Base decay (1.0) is handled by spoilage_system. We only add the EXTRA decay.
    if modifier <= 1.0 + f32::EPSILON {
        return;
    }

    let extra_decay = modifier - 1.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let guaranteed_decay = extra_decay.floor().max(0.0).min(u32::MAX as f32) as u32;
    let chance_decay = extra_decay.fract();

    query.par_iter_mut().for_each(|mut perishable| {
        let mut rng = rand::thread_rng();
        let mut decay = guaranteed_decay;
        if rng.r#gen::<f32>() < chance_decay {
            decay += 1;
        }
        perishable.current_ticks += decay;
    });
}

/// Notifies the player when a patient receives significant treatment.
///
/// Bridges Medical system (Event) and Notification system (UI).
pub fn medical_treatment_notification_system(
    mut events: EventReader<PatientTreated>,
    mut notifications: ResMut<NotificationQueue>,
    time: Res<SimulationTime>,
    pops: Query<&PopName>,
) {
    for event in events.read() {
        // Only notify for significant healing to reduce spam
        if event.amount >= 1.0 {
            let name = pops
                .get(event.patient)
                .map(|n| n.0.as_str())
                .unwrap_or("Colonist");
            notifications.add_success(
                format!(
                    "{} received medical treatment (+{:.1} HP).",
                    name, event.amount
                ),
                time.tick,
            );
        }
    }
}

/// Notifies the player when a pop is hospitalized.
///
/// Bridges Utility AI (Action Change) and Notification system (UI).
pub fn hospitalization_notification_system(
    query: Query<(&PopAction, &PopName), Changed<PopAction>>,
    mut notifications: ResMut<NotificationQueue>,
    time: Res<SimulationTime>,
) {
    for (action, name) in query.iter() {
        if action.current == ActionType::SeekMedicalCare {
            notifications.add_warning(format!("{} has been hospitalized!", name.0), time.tick);
        }
    }
}

/// Notifies the player when a pop dies.
///
/// Bridges Pop system (Death Event) and Notification system (UI).
pub fn pop_death_notification_system(
    mut events: EventReader<PopDied>,
    mut notifications: ResMut<NotificationQueue>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        notifications.add_error(
            format!("{} has died! Cause: {}", event.name, event.reason),
            time.tick,
        );
    }
}

/// Notifies the player when a pop is born.
///
/// Bridges Pop system (Birth Event) and Notification system (UI).
pub fn pop_born_notification_system(
    mut events: EventReader<PopBorn>,
    mut notifications: ResMut<NotificationQueue>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        notifications.add_info(
            format!("{} has been born. Source: {}", event.name, event.source),
            time.tick,
        );
    }
}

/// Transfers cargo from fleets orbiting the colony to the colony's resource stockpile.
///
/// Bridges System Mining (Layer 2) and Colony Resources (Layer 1).
pub fn fleet_unload_system(
    _commands: Commands,
    mut fleets: Query<
        (
            Entity,
            &crate::layer2::fleet::InOrbit,
            &mut crate::layer2::mining::FleetCargo,
        ),
        With<crate::layer2::fleet::Fleet>,
    >,
    colony_locations: Query<(Entity, &crate::layer2::generation::ColonyLocation)>,
    mut resources: ResMut<ColonyResources>,
    mut log: Option<ResMut<MessageLog>>,
) {
    // 1. Identify Colony Planet(s)
    let colony_entities: HashSet<Entity> = colony_locations.iter().map(|(e, _)| e).collect();

    for (_fleet_entity, orbit, mut cargo) in &mut fleets {
        // 2. Check if in orbit of colony
        if colony_entities.contains(&orbit.parent) {
            // 3. Unload Cargo
            let mut unloaded_something = false;
            // ⚡ Bolt Optimization: Pre-allocate capacity to avoid intermediate reallocations
            let mut summary = Vec::with_capacity(cargo.contents.len());

            for stack in cargo.contents.drain(..) {
                if stack.amount > 0.0 {
                    unloaded_something = true;
                    resources.add_resource(&stack.resource_type, stack.amount);
                    summary.push(format!("{:.1} {:?}", stack.amount, stack.resource_type));
                }
            }

            if unloaded_something {
                if let Some(log) = log.as_mut() {
                    log.add_colored(
                        format!("Fleet unloaded: {}", summary.join(", ")),
                        Color::Green,
                    );
                }
            }
        }
    }
}

pub fn mega_quake_chronicle_bridge(
    mut events: EventReader<crate::layer1::geology::tectonic::MegaQuakeEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "A Mega-Quake has ruptured the colony's foundations.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

use crate::layer1::unrest::{DenounceEvent, ScapegoatAction};

/// Bridges `DenounceEvent` to the `Chronicle` system.
///
/// Records the outcome of denouncing a scapegoat in the colony's history.
pub fn scapegoat_chronicle_bridge(
    mut events_in: EventReader<DenounceEvent>,
    mut events_out: EventWriter<AddChronicleEvent>,
) {
    for event in events_in.read() {
        let text = match event.action {
            ScapegoatAction::Exile => "A scapegoat was exiled to appease the mob.".to_string(),
            ScapegoatAction::PublicShame => {
                "A scapegoat was publicly shamed to reduce unrest.".to_string()
            }
            ScapegoatAction::Execute => {
                "A scapegoat was executed to quell the uprising.".to_string()
            }
        };

        events_out.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

/// INT-260: Bridges Industrial Rhythm system (Spec 260) to Pop Morale system (Spec 031).
///
/// When adjacent machines finish their cycles synchronously, they generate a `last_sync_bonus`.
/// This system queries machines with an active bonus and applies a `MoodModifier` to nearby Pops
/// to represent the satisfying "thrum of efficiency", boosting their morale.
pub fn industrial_rhythm_morale_bridge(
    machines: Query<(
        &crate::layer1::tech::rhythm::MachineRhythm,
        &crate::layer1::core::map::GridPosition,
    )>,
    mut pops: Query<
        (
            &mut crate::layer1::morale::Morale,
            &crate::layer1::core::map::GridPosition,
        ),
        With<crate::layer1::pop::Pop>,
    >,
) {
    // Collect active rhythms and their positions
    let active_rhythms: Vec<(f32, crate::layer1::core::map::GridPosition)> = machines
        .iter()
        .filter_map(|(rhythm, pos)| {
            if rhythm.last_sync_bonus > 0.0 {
                Some((rhythm.last_sync_bonus, *pos))
            } else {
                None
            }
        })
        .collect();

    if active_rhythms.is_empty() {
        return;
    }

    // Apply mood modifier to nearby pops
    for (mut morale, pop_pos) in pops.iter_mut() {
        for (bonus, machine_pos) in &active_rhythms {
            if pop_pos.distance_chebyshev(*machine_pos) <= 3 {
                // Determine a scaled bonus value for morale (max around +0.1 for 10.0 bonus)
                let morale_bonus = (*bonus * 0.01).clamp(0.01, 0.15);

                // Add the modifier
                morale.add_modifier(crate::layer1::morale::MoodModifier {
                    label: "Industrial Rhythm".to_string(),
                    value: morale_bonus,
                    duration: 100, // Lingers for 100 ticks
                });

                // Once applied for one machine in range, we can break to avoid
                // stacking multiple identical bonuses from a large cluster in a single tick.
                // Or we could let it stack. Breaking here to be safe and match a single "thrum" experience.
                break;
            }
        }
    }
}

/// Increases `Fauna` detection range based on `NocturnalFauna` aggression (INT-450).
/// Links `LightPollution` to actual `Fauna` behavior.
pub fn nocturnal_aggression_bridge_system(
    mut query: bevy_ecs::prelude::Query<(
        &crate::layer1::fauna::NocturnalFauna,
        &mut crate::layer1::fauna::Fauna,
    )>,
) {
    for (nocturnal, mut fauna) in query.iter_mut() {
        // Base detection range is typically ~8.0.
        // We calculate a bonus instead of overwriting the base range.
        // This makes sure we don't accidentally shrink alien fauna that have huge ranges.
        // However, we can't easily track the "base" range on the fly without a new component.
        // The simplest, safest fix is to add a small amount of range per tick it's aggressive,
        // or just apply a temporary bump if it's not already boosted.
        // Since `apply_light_pollution_system` increases `animal.aggression += 0.01` every tick
        // we can just increase `detection_range` slightly as aggression grows.

        // Wait, aggression grows continuously! We just need to ensure the detection range
        // is bumped up as well. Let's just bump it proportionally, but clamp it so it
        // doesn't go to infinity.

        if nocturnal.aggression > 0.0 {
            let max_bonus = 15.0; // The max extra range they can get

            // To prevent infinitely growing ranges, let's calculate the target range
            // based on a fixed base rather than the current range.
            // We assume a base of 8.0, but if the current range is already larger, we use that.
            // Wait, we can't store the original base.
            // The simplest approach is to just calculate a derived value based on aggression
            // and apply it. If we want a slow increase, we should target a static upper bound.
            // Let's define the absolute maximum detection range for *any* fauna due to pollution as 25.0

            let target_range = 8.0 + (nocturnal.aggression * 20.0).min(max_bonus);
            if fauna.detection_range < target_range {
                fauna.detection_range += 0.1; // slow increase
            }
        }
    }
}

/// Bridges `OverrideWillEvent` to the `Chronicle` system (INT-451).
///
/// Records the outcome of a spiteful will being forcibly overridden.
pub fn override_will_chronicle_bridge(
    mut events_in: bevy_ecs::prelude::EventReader<crate::layer1::spiteful_will::OverrideWillEvent>,
    mut events_out: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in events_in.read() {
        events_out.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A spiteful will was forcibly overridden, sparking outrage among the heirs."
                .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-291: Translates NeuralShock from Neural Leech hubs into catastrophic mental breakdowns.
pub fn apply_neural_shock_system(
    mut commands: Commands,
    mut query: Query<
        (Entity, &mut crate::layer1::unrest::MentalState),
        With<crate::layer1::tech::neural_leech::NeuralShock>,
    >,
) {
    for (entity, mut state) in &mut query {
        *state = crate::layer1::unrest::MentalState::Broken(
            crate::layer1::unrest::MentalBreakType::Daze,
        );
        commands
            .entity(entity)
            .remove::<crate::layer1::tech::neural_leech::NeuralShock>();
    }
}

/// INT-431-348: Bridge between Black Market Smugglers and Void-Weed Trade.
///
/// When a `Smuggler` (from `black_market`) or `ShadowTrader` (from `shadow_market`)
/// spawns, we want to broadcast a `MerchantArrivalEvent` of type `Smuggler`.
/// This lets `process_void_weed_trade_system` execute the stash exchanges.
pub fn smuggler_arrival_event_bridge(
    query_smuggler: Query<Entity, Added<crate::layer1::black_market::Smuggler>>,
    query_shadow: Query<Entity, Added<crate::layer1::shadow_market::ShadowTrader>>,
    mut event_writer: EventWriter<crate::layer1::void_weed::MerchantArrivalEvent>,
) {
    for _ in query_smuggler.iter() {
        event_writer.send(crate::layer1::void_weed::MerchantArrivalEvent {
            merchant_type: crate::layer1::void_weed::MerchantType::Smuggler,
        });
    }
    for _ in query_shadow.iter() {
        event_writer.send(crate::layer1::void_weed::MerchantArrivalEvent {
            merchant_type: crate::layer1::void_weed::MerchantType::Smuggler,
        });
    }
}

/// INT-453-2: Bridges Nanite Fabrication Breach to Grey Goo spawning
pub fn nanite_breach_goo_bridge(
    mut breach_events: EventReader<crate::layer1::nanite_fabrication::ContainmentBreachEvent>,
    mut commands: Commands,
) {
    for event in breach_events.read() {
        if let Some(mut entity_commands) = commands.get_entity(event.source_entity) {
            entity_commands.despawn();
        }
        commands.spawn((
            crate::layer1::nanite_fabrication::GreyGoo {
                replication_progress: 0.0,
            },
            event.position,
        ));
    }
}

/// INT-453-1089: Bridges Nanite Fabrication Breach to Nanite Storms
pub fn nanite_breach_storm_bridge(
    mut breach_events: EventReader<crate::layer1::nanite_fabrication::ContainmentBreachEvent>,
    mut commands: Commands,
) {
    for event in breach_events.read() {
        commands.insert_resource(crate::layer1::nanite_storms::ActiveNaniteStorm {
            storm_type: crate::layer1::nanite_storms::NaniteStormType::Grey,
            affected_area: bevy::prelude::Rect::new(
                event.position.x as f32 - 10.0,
                event.position.y as f32 - 10.0,
                event.position.x as f32 + 10.0,
                event.position.y as f32 + 10.0,
            ),
            intensity: 10.0,
        });
    }
}

/// INT-453: Bridges Nanite Fabrication (Containment Breach) to Chronicle (History).
pub fn nanite_breach_chronicle_bridge(
    mut breach_events: EventReader<crate::layer1::nanite_fabrication::ContainmentBreachEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in breach_events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "Catastrophic Containment Breach! Grey Goo unleashed at position ({}, {}).",
                event.position.x, event.position.y
            ),
            importance: EventImportance::Major,
        });
    }
}

use crate::layer1::genetics::GeneSplicingResultEvent;

/// Bridges GeneSplicingResultEvent to AddChronicleEvent (Chronicle).
pub fn gene_splicing_chronicle_bridge(
    mut events: EventReader<GeneSplicingResultEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        let text = match ev {
            GeneSplicingResultEvent::Success {
                target: _,
                mod_type: _,
            } => "The cut was successful. We have new mutants among us.".to_string(),
            GeneSplicingResultEvent::Failure {
                target: _,
                mod_type: _,
                mutation: _,
            } => "The splicing failed, resulting in a horrific twist.".to_string(),
        };

        chronicle_events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

/// INT-657: Bridge Pop deaths to EntityKilledEvents
pub fn diplomatic_reflection_kill_bridge(
    mut events_in: EventReader<crate::layer1::pop::PopDied>,
    mut events_out: EventWriter<crate::layer3::diplomacy_reflection::EntityKilledEvent>,
) {
    for _ in events_in.read() {
        events_out.send(crate::layer3::diplomacy_reflection::EntityKilledEvent {
            colony_entity: Entity::PLACEHOLDER,
        });
    }
}

// --- INT-183: Geodetic Sentience -> Chronicle ---

use crate::layer1::genetics::{CropMutationEvent, MutationType};

/// INT-735: Bridges CropMutationEvent to AddChronicleEvent (Chronicle).
pub fn crop_mutation_chronicle_bridge(
    mut events: EventReader<CropMutationEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        let text = match ev.mutation_type {
            MutationType::AggressiveGrowth => {
                "A genetically modified crop mutated, exhibiting aggressive growth.".to_string()
            }
            MutationType::ToxicSpores => {
                "A genetically modified crop mutated, releasing toxic spores.".to_string()
            }
        };

        chronicle_events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

use crate::layer1::grafting::GraftBuildingEvent;

/// Bridges `GraftBuildingEvent` to `AddChronicleEvent` (Chronicle).
pub fn grafting_chronicle_bridge(
    mut events: EventReader<GraftBuildingEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Minor,
            text: "A structure was grafted with mismatched technology, adopting Frankenstein architecture.".to_string(),
        });
    }
}

/// Bridges `GolemFormedEvent` to `AddChronicleEvent` (Chronicle).
pub fn golem_formed_chronicle_bridge_system(
    mut events: EventReader<GolemFormedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "The Stones Awake: A Golem has been formed from Living Stones.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// INT-657: Bridge Flora planted to FloraPlantedEvents
pub fn diplomatic_reflection_plant_bridge(
    query: Query<Entity, Added<crate::layer1::flora::Flora>>,
    mut events_out: EventWriter<crate::layer3::diplomacy_reflection::FloraPlantedEvent>,
) {
    for _ in query.iter() {
        events_out.send(crate::layer3::diplomacy_reflection::FloraPlantedEvent {
            colony_entity: Entity::PLACEHOLDER,
        });
    }
}

/// INT-570: Bridges the gap between Bio-Acoustic Miasma's paranoia and the general stress system.
/// Adds paranoia levels directly to accumulated stress, pushing Pops closer to a mental breakdown.
pub fn paranoia_stress_bridge_system(
    mut query: Query<(
        &mut crate::layer1::stress::StressTracker,
        &mut crate::layer1::environment::bio_acoustic_miasma::ParanoiaTracker,
    )>,
) {
    for (mut stress, mut paranoia) in &mut query {
        if paranoia.level > 0 {
            stress.accumulated_stress += paranoia.level as f32;
            paranoia.level = 0;
        }
    }
}

use crate::layer1::temporal_ghost_towns::TemporalStutterEvent;

/// Bridges `TemporalStutterEvent` to `AddChronicleEvent` (Chronicle).
pub fn temporal_stutter_chronicle_bridge(
    mut events: EventReader<TemporalStutterEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Minor,
            text: "A map tile stuttered in time, causing buildings to revert temporarily."
                .to_string(),
        });
    }
}

/// Bridges `BuildingConsumedEvent` (Parasitic Architecture) to `AddChronicleEvent` (Chronicle).
pub fn parasitic_architecture_chronicle_bridge(
    mut events: EventReader<crate::layer1::parasitic_architecture::BuildingConsumedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "A building was completely consumed by a parasitic megastructure.".to_string(),
            importance: EventImportance::Minor,
        });
    }
}

// --- INT-573: The Silent Generation ---

/// Bridges `PopDied` events into the `TraumaTracker` system.
pub fn trauma_death_bridge_system(
    mut events: EventReader<PopDied>,
    mut trauma: ResMut<crate::layer1::stress::TraumaTracker>,
) {
    for _ in events.read() {
        trauma.recent_deaths = trauma.recent_deaths.saturating_add(1);
    }
}

/// Bridges `ColonyResources` into the `TraumaTracker` system by detecting famine.
pub fn famine_tracking_system(
    resources: Res<ColonyResources>,
    mut trauma: ResMut<crate::layer1::stress::TraumaTracker>,
) {
    if resources.food <= 0.0 {
        trauma.famine_ticks = trauma.famine_ticks.saturating_add(1);
    }
}

/// Decays trauma values over time.
pub fn trauma_decay_system(
    time: Res<SimulationTime>,
    mut trauma: ResMut<crate::layer1::stress::TraumaTracker>,
) {
    if time.tick.is_multiple_of(10) {
        trauma.recent_deaths = trauma.recent_deaths.saturating_sub(1);
        trauma.famine_ticks = trauma.famine_ticks.saturating_sub(10);
    }
}

pub fn phantom_shift_chronicle_bridge(
    mut events: EventReader<crate::layer1::unseen_bureaucracy::PhantomShiftEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Minor,
            text: "We noticed missing resources. The desperate toil in the dark to fix our neglected infrastructure.".to_string(),
        });
    }
}

#[derive(Event, Debug, Clone)]
pub struct PirateAmnestyEvent {
    pub fleet: Entity,
}

use crate::layer1::deep_crust_resonance::ExcavationEvent;

/// INT-1132: Bridges Resonant Ore Excavation to AddChronicleEvent
pub fn deep_crust_resonance_chronicle_bridge(
    mut events: EventReader<ExcavationEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if event.discovery_type == "ResonantOre" {
            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: "Deep Crust Resonance uncovered! The miners speak of an ancient hum that invades their minds.".to_string(),
            });
        }
    }
}

/// INT-947: Bridges Aesthetic Orbital Blockade (Policy::Aesthetic) to AddChronicleEvent (Chronicle).
pub fn aesthetic_edict_chronicle_bridge(
    policies: bevy_ecs::prelude::Res<crate::layer1::administration::edicts::ColonyPolicies>,
    mut last_status: bevy_ecs::prelude::Local<bool>,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    let current_status =
        policies.is_active(crate::layer1::administration::edicts::Policy::Aesthetic);
    if current_status && !*last_status {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "The orbital elites have passed an Aesthetic Edict, halting our most productive factories to clear their view.".to_string(),
        });
    } else if !current_status && *last_status {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Standard,
            text: "The Aesthetic Edict has been lifted. The factories roar back to life, belching smoke into the sky once more.".to_string(),
        });
    }
    *last_status = current_status;
}

/// INT-890: Bridges FamineEvent to AddChronicleEvent (Chronicle).
pub fn famine_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<crate::layer1::pop_memories::FamineEvent>,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "A devastating famine swept through the colony, searing memories of starvation into the survivors.".to_string(),
        });
    }
}

/// INT-961: Bridges Silent Flora discovery to AddChronicleEvent (Chronicle).
pub fn silent_flora_chronicle_bridge(
    query: bevy_ecs::prelude::Query<
        &crate::layer1::flora::Flora,
        bevy_ecs::prelude::Added<crate::layer1::flora::Flora>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for flora in query.iter() {
        if flora.flora_type == crate::layer1::flora::FloraType::SilentFlora {
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                importance: crate::layer1::core::chronicle::EventImportance::Major,
                text: "We discovered a strange new flora. It grows rapidly, but an eerie silence surrounds it.".to_string(),
            });
            break;
        }
    }
}

// Pheromone Gardening integration (984)
#[derive(bevy_ecs::query::QueryData)]
pub struct FloraScentQuery {
    entity: bevy_ecs::prelude::Entity,
    flora: &'static crate::layer1::flora::PheromoneFlora,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct FloraScentFilter {
    _or: bevy_ecs::prelude::Or<(
        bevy_ecs::prelude::Added<crate::layer1::flora::PheromoneFlora>,
        bevy_ecs::prelude::Changed<crate::layer1::flora::PheromoneFlora>,
    )>,
}

pub fn flora_scent_bridge_system(
    mut commands: bevy_ecs::prelude::Commands,
    query: bevy_ecs::prelude::Query<FloraScentQuery, FloraScentFilter>,
) {
    for item in query.iter() {
        let entity = item.entity;
        let flora = item.flora;
        match flora.emission_type {
            crate::layer1::flora::PheromoneEmission::Calming => {
                commands
                    .entity(entity)
                    .insert(crate::layer1::olfactory::ScentEmitter {
                        is_pleasant: true,
                        strength: flora.strength,
                    });
            }
            crate::layer1::flora::PheromoneEmission::Danger => {
                commands
                    .entity(entity)
                    .insert(crate::layer1::olfactory::ScentEmitter {
                        is_pleasant: false,
                        strength: flora.strength,
                    });
            }
            crate::layer1::flora::PheromoneEmission::Normal => {
                commands
                    .entity(entity)
                    .remove::<crate::layer1::olfactory::ScentEmitter>();
            }
        }
    }
}

// Predatory Weather Integration (969)
pub fn predatory_weather_emission_bridge_system(
    mut commands: bevy_ecs::prelude::Commands,
    power_sources: bevy_ecs::prelude::Query<&crate::layer1::energy::PowerSource>,
    heat_sources: bevy_ecs::prelude::Query<&crate::layer1::nature::temperature::HeatSource>,
    mut targets: bevy_ecs::prelude::Query<&mut crate::layer2::weather::AggroTarget>,
) {
    let mut total_energy = 0.0;
    for power in power_sources.iter() {
        if power.active {
            total_energy += power.output;
        }
    }

    let mut total_heat = 0.0;
    for heat in heat_sources.iter() {
        total_heat += heat.output;
    }

    if let Some(mut target) = targets.iter_mut().next() {
        target.energy_emission = total_energy;
        target.heat_signature = total_heat;
    } else {
        commands.spawn(crate::layer2::weather::AggroTarget {
            position: bevy::math::Vec2::new(0.0, 0.0),
            energy_emission: total_energy,
            heat_signature: total_heat,
        });
    }
}

pub fn predatory_weather_impact_bridge_system(
    mut events: bevy_ecs::prelude::EventReader<crate::layer2::weather::StormImpactEvent>,
    mut structures: bevy_ecs::prelude::Query<
        &mut crate::layer1::architecture::structure::Structure,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        for mut structure in structures.iter_mut() {
            structure.current_hp -= event.damage;
        }

        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: format!("A massive planetary storm impacted the colony, dealing {} damage to our infrastructure.", event.damage),
        });
    }
}

/// INT-805: Bridges MindUploadEvent to AddChronicleEvent (Chronicle).
pub fn digital_immortality_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::digital_immortality::MindUploadEvent>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
    query: bevy_ecs::system::Query<&bevy::prelude::Name>,
) {
    for event in events.read() {
        if let Ok(name) = query.get(event.target_pop) {
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!(
                    "{} has achieved digital immortality, leaving behind their mortal shell to become a Ghost in the Mainframe.",
                    name.as_str()
                ),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
        }
    }
}

/// INT-900: Bridges Latent Psionics `FireEvent` to spawn a `Fire` component in the world.
pub fn psionic_fire_bridge_system(
    mut events: EventReader<crate::layer1::psychology::psionics::FireEvent>,
    mut commands: Commands,
) {
    for event in events.read() {
        commands.spawn((crate::layer1::nature::fire::Fire::default(), event.position));
    }
}

/// INT-762: Bridges MigrantArrivalEvent to Pop spawning
pub fn beacon_migrant_arrival_bridge(
    mut commands: Commands,
    mut events: EventReader<crate::layer1::economy::remittances::MigrantArrivalEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    sim_time: Option<Res<SimulationTime>>,
    pop_positions: Query<
        &crate::layer1::core::map::GridPosition,
        With<crate::layer1::pop::Pop>,
    >,
) {
    use rand::seq::SliceRandom;
    use rand::Rng;

    let tick = sim_time.map_or(0, |t| t.tick);
    // Migrants arrive at the colony proper, not at the map origin: pick a
    // random living pop's tile so they land inside the pressurized habitat
    // instead of (0, 0), which may be vacuum.
    let colony_tiles: Vec<crate::layer1::core::map::GridPosition> =
        pop_positions.iter().copied().collect();

    for event in events.read() {
        let mut rng = rand::thread_rng();

        for _ in 0..event.count {
            let is_criminal = rng.gen::<f32>() < event.criminal_chance;
            let is_low_skill = rng.gen::<f32>() < event.low_skill_chance;

            let mut traits = crate::layer1::psychology::traits::Traits::default();
            if is_criminal {
                traits.add(crate::layer1::psychology::traits::Trait::Greedy);
            }
            if is_low_skill {
                traits.add(crate::layer1::psychology::traits::Trait::Lazy);
            }

            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(crate::layer1::core::map::GridPosition { x: 0, y: 0 });

            // WINTER-ECONOMY PASS (2026-10-03): migrants used to spawn as bare
            // Pop+Traits+Needs ghosts — no Health, no PopName, no Wallet — so
            // they were invisible to STATS, immune to damage and infection,
            // and ate from the stockpile for free. Spawn a full PopBundle like
            // starter pops instead, keeping the beacon-selected traits and a
            // real arrival tick.
            let mut bundle =
                crate::layer1::PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            bundle.traits = traits;
            bundle.arrival = crate::layer1::social::old_guard::Arrival { tick };
            commands.spawn(bundle);
        }

        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: format!("{} migrants have arrived in the colony.", event.count),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-762: Bridges TradeShipArrivalEvent to Merchant spawning
pub fn beacon_trade_ship_bridge(
    mut events: EventReader<crate::layer2::trade::blockade::TradeShipArrivalEvent>,
    mut merchant_state: ResMut<crate::layer1::trade::MerchantState>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    time: Res<crate::shared::time::SimulationTime>,
) {
    for event in events.read() {
        if merchant_state.active_merchant.is_none() {
            merchant_state.active_merchant = Some(crate::layer1::trade::Merchant {
                name: format!("{} Ship", event.faction),
                arrival_tick: time.tick,
                departure_tick: time.tick + 500,
                deals: vec![], // For integration purposes, this just forces the state change
            });
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!("A trade ship from {} has arrived.", event.faction),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
        }
    }
}

/// INT-762: Bridges PirateRaidEvent to Resource loss and Morale penalty
pub fn beacon_pirate_raid_bridge(
    mut events: EventReader<crate::layer1::void_weed::PirateRaidEvent>,
    mut resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
    mut pops: Query<&mut crate::layer1::morale::Morale, With<crate::layer1::pop::Pop>>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        // Pirates steal resources
        // WINTER-ECONOMY PASS (2026-10-03): the flat 50-food theft erased a
        // small colony's entire stockpile in one raid (starter food is 10),
        // which is what starved colonies into winter. Steal a quarter of the
        // stockpile capped at 10: it stings without deleting the buffer.
        let stolen_food = (resources.food * 0.25).min(10.0);
        resources.food = (resources.food - stolen_food).max(0.0);
        resources.metal = (resources.metal - 20.0).max(0.0);

        // Morale drops
        for mut morale in pops.iter_mut() {
            morale.add_modifier(crate::layer1::morale::MoodModifier {
                label: "Pirate Raid".to_string(),
                value: -0.2,
                duration: 500,
            });
        }

        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "Pirates have raided the colony!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-XXX: Bridges the gap between Sonic Turrets and the acoustic system.
/// Adds a `NoiseSource` to active `SonicTurret` entities that don't already have one,
/// and updates or removes it based on the turret's state.
pub fn sonic_turret_noise_bridge_system(
    mut commands: bevy_ecs::system::Commands,
    turrets: bevy_ecs::system::Query<(
        bevy_ecs::entity::Entity,
        &crate::layer1::sonic_suppression::SonicTurret,
        Option<&crate::layer1::physics::acoustic::NoiseSource>,
    )>,
) {
    for (entity, turret, noise_source_opt) in turrets.iter() {
        if turret.active {
            if noise_source_opt.is_none() {
                commands
                    .entity(entity)
                    .insert(crate::layer1::physics::acoustic::NoiseSource {
                        radius: turret.range,
                        intensity: 1.0,
                    });
            } else {
                // we could also update the radius if we want to be safe
            }
        } else {
            if noise_source_opt.is_some() {
                commands
                    .entity(entity)
                    .remove::<crate::layer1::physics::acoustic::NoiseSource>();
            }
        }
    }
}

use crate::layer1::factions::{FactionId, FactionState};
use crate::layer1::social::protest_crowds::{DisperseMobEvent, FormMobEvent, Mob};
use bevy::utils::HashMap;

pub fn faction_strike_mob_bridge_system(
    factions: Res<Factions>,
    mut form_events: EventWriter<FormMobEvent>,
    mut disperse_events: EventWriter<DisperseMobEvent>,
    mut prev_states: Local<HashMap<FactionId, FactionState>>,
    mob_query: Query<(Entity, &Mob)>,
) {
    for (faction_id, data) in &factions.map {
        let current_state = data.state;
        let prev_state = prev_states
            .get(faction_id)
            .copied()
            .unwrap_or(FactionState::Loyal);

        if current_state == FactionState::Striking && prev_state != FactionState::Striking {
            // Faction just went on strike, form a mob!
            // We use (0, 0) as a fallback location for now
            form_events.send(FormMobEvent {
                location: (0, 0),
                faction: *faction_id,
            });
        } else if current_state != FactionState::Striking && prev_state == FactionState::Striking {
            // Faction is no longer striking, disperse their mob
            for (mob_entity, mob) in &mob_query {
                if mob.faction == *faction_id {
                    disperse_events.send(DisperseMobEvent { mob: mob_entity });
                }
            }
        }

        prev_states.insert(*faction_id, current_state);
    }
}

/// Bridges `DayNightCycle` to `ShiftEndEvent` for Pop Relationships.
pub fn trigger_shift_end_system(
    cycle: Res<crate::layer1::day_night::DayNightCycle>,
    mut events: EventWriter<crate::layer1::social::pop_relationships::ShiftEndEvent>,
    mut last_time_of_day: Local<Option<crate::layer1::day_night::TimeOfDay>>,
) {
    if let Some(last) = *last_time_of_day {
        if last == crate::layer1::day_night::TimeOfDay::Day
            && cycle.time_of_day == crate::layer1::day_night::TimeOfDay::Dusk
        {
            events.send(crate::layer1::social::pop_relationships::ShiftEndEvent);
        }
    }
    *last_time_of_day = Some(cycle.time_of_day);
}

/// INT-1235: Bridges CulinarySingularityEvent to AddChronicleEvent (Chronicle).
pub fn gastronomer_chronicle_bridge(
    mut events: EventReader<crate::layer1::culture::gastronomers::CulinarySingularityEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Gastronomers have achieved the Culinary Singularity!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// Bridges TheVisitor spawning to AddChronicleEvent (Chronicle).
pub fn visitor_chronicle_bridge(
    query: bevy_ecs::prelude::Query<
        bevy_ecs::prelude::Entity,
        bevy_ecs::prelude::Added<crate::layer1::entities::the_visitor::TheVisitor>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Visitor has arrived.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-643: Bridges PopConsumedEvent (Living Architecture) to AddChronicleEvent (Chronicle).
pub fn living_architecture_chronicle_bridge(
    mut consumed_events: EventReader<
        crate::layer1::architecture::living_architecture::PopConsumedEvent,
    >,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _event in consumed_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A starving living building has consumed a colonist!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-661: Bridges Predictive Policing -> Secret Societies
/// When an active PredictiveModel is present, Pops in a SecretSociety are flagged as Suspects.
pub fn society_suspicion_bridge_system(
    mut commands: bevy_ecs::system::Commands,
    members: bevy_ecs::system::Query<
        (
            bevy_ecs::entity::Entity,
            &crate::layer1::social::secret_societies::SecretSocietyMember,
        ),
        bevy_ecs::query::Without<crate::layer1::law::predictive_policing::Suspect>,
    >,
    societies: bevy_ecs::system::Query<&crate::layer1::social::secret_societies::SecretSociety>,
    config: bevy_ecs::system::Res<crate::layer1::law::predictive_policing::PredictionConfig>,
    models: bevy_ecs::system::Query<
        Option<&crate::layer1::energy::PowerConsumer>,
        bevy_ecs::query::With<crate::layer1::law::predictive_policing::PredictiveModel>,
    >,
) {
    if !config.enabled {
        return;
    }

    let has_active_model = models.iter().any(|pc| pc.is_none_or(|p| p.active));
    if !has_active_model {
        return;
    }

    for (entity, member) in members.iter() {
        if let Ok(society) = societies.get(member.society_id) {
            if society.is_hidden {
                commands
                    .entity(entity)
                    .insert(crate::layer1::law::predictive_policing::Suspect {
                        probability: 0.85,
                        predicted_crime: "Secret Society Conspiracy".to_string(),
                    });
            }
        }
    }
}

/// INT-661: Bridges Secret Societies -> Justice/Chronicle
/// When a SecretSocietyMember is arrested (gets Inmate component), the society is uncovered and disbanded.
pub fn secret_society_discovery_bridge_system(
    mut commands: bevy_ecs::system::Commands,
    arrested_members: bevy_ecs::system::Query<
        &crate::layer1::social::secret_societies::SecretSocietyMember,
        bevy_ecs::query::Added<crate::layer1::law::justice::Inmate>,
    >,
    mut societies: bevy_ecs::system::Query<
        &mut crate::layer1::social::secret_societies::SecretSociety,
    >,
    all_members: bevy_ecs::system::Query<(
        bevy_ecs::entity::Entity,
        &crate::layer1::social::secret_societies::SecretSocietyMember,
    )>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for member in arrested_members.iter() {
        if let Ok(mut society) = societies.get_mut(member.society_id) {
            if society.is_hidden {
                society.is_hidden = false;

                chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                    text: "A secret society was uncovered during a preemptive arrest and has been disbanded.".to_string(),
                    importance: crate::layer1::core::chronicle::EventImportance::Major,
                });

                // Disband the society
                commands.entity(member.society_id).despawn();

                // Remove membership from all members
                for (ent, m) in all_members.iter() {
                    if m.society_id == member.society_id {
                        commands.entity(ent).remove::<crate::layer1::social::secret_societies::SecretSocietyMember>();
                    }
                }
            }
        }
    }
}

pub fn sub_lithic_sabotage_bridge(_commands: bevy_ecs::system::Commands) {}

/// Forces the `SeasonState` to `Season::Spring` if a `PredecessorWeatherArray` is active.
pub fn predecessor_weather_array_bridge_system(
    array_query: Query<&crate::layer1::predecessors::PredecessorWeatherArray>,
    mut season_state: ResMut<crate::layer1::nature::seasons::SeasonState>,
) {
    if !array_query.is_empty() {
        season_state.current_season = crate::layer1::nature::seasons::Season::Spring;
    }
}

/// INT-1067: Bridges SirenSignalEvent to AddChronicleEvent (Chronicle).
pub fn siren_signal_chronicle_bridge(
    mut events: EventReader<crate::layer1::void_sirens::SirenSignalEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "A mesmerizing deep-space signal is detected, driving our brightest minds into an obsession.".to_string(),
        });
    }
}

/// INT-635: Bridges MachineCultFormedEvent to AddChronicleEvent (Chronicle).
pub fn rogue_cult_chronicle_bridge(
    mut cult_events: bevy_ecs::event::EventReader<
        crate::layer1::tech::rogue_automation_cults::MachineCultFormedEvent,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in cult_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The hauling bots formed a Machine Cult around the failing relay.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-687: Bridges GhostShiftStartedEvent to AddChronicleEvent (Chronicle).
pub fn ghost_shift_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer1::social::ghost_shift_strike::GhostShiftStartedEvent,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "A subtle Ghost-Shift strike has been detected!".to_string(),
        });
    }
}

/// INT-668: Bridges ImpactWarningEvent to AddChronicleEvent (Chronicle).
pub fn impact_warning_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer1::environment::impact::ImpactWarningEvent,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: format!(
                "Impact Warning: Projectile inbound at {}, {}, ETA: {} ticks.",
                event.target_pos.x, event.target_pos.y, event.ticks_remaining
            ),
        });
    }
}

/// INT-877: Bridges the debt of the dead system to AddChronicleEvent (Chronicle).
pub fn debt_of_the_dead_chronicle_bridge(
    mut inherited_events: bevy_ecs::prelude::EventReader<
        crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent,
    >,
    mut socialized_events: bevy_ecs::prelude::EventReader<
        crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in inherited_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
            text: format!(
                "{} died. {} credits of debt inherited by next of kin.",
                event.pop_name, event.amount
            ),
        });
    }

    for event in socialized_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Standard,
            text: format!(
                "{} died, leaving {} credits of debt to be socialized by the colony.",
                event.pop_name, event.amount
            ),
        });
    }
}

/// INT-642: Bridges the construction of a Simulacrum to AddChronicleEvent (Chronicle).
pub fn simulacrum_chronicle_bridge(
    query: bevy_ecs::prelude::Query<
        bevy_ecs::prelude::Entity,
        bevy_ecs::prelude::Added<crate::layer1::psychology::simulacrum::Simulacrum>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "A Propaganda Simulacrum was constructed, replacing harsh reality with a golden narrative.".to_string(),
        });
    }
}

/// INT-764: Bridges DiplomaticIncidentEvent to AddChronicleEvent (Chronicle).
pub fn diplomatic_incident_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer1::law::embassy::DiplomaticIncidentEvent,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: format!("Diplomatic Incident: {}", event.reason),
        });
    }
}

pub fn nostalgia_rumor_generation_bridge(
    mut query: Query<
        (Entity, &mut crate::layer1::rumor::Knowledge),
        With<crate::layer1::culture::nostalgia::Nostalgia>,
    >,
    time: Res<crate::shared::time::SimulationTime>,
) {
    for (entity, mut knowledge) in &mut query {
        let topic = crate::layer1::rumor::RumorTopic::EventNews("Past Glory".to_string());
        if !knowledge.knows(&topic) {
            knowledge.add_rumor(crate::layer1::rumor::Rumor {
                topic,
                source: entity,
                timestamp: time.tick,
                strength: 1.0,
            });
        }
    }
}

/// Bridges `SabotageEvent` (Cryo Prison) to `Structure` damage and `Chronicle` tracking.
pub fn cryo_prison_sabotage_bridge_system(
    mut events: EventReader<crate::layer1::cryo_prison::SabotageEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    mut structures: Query<&mut crate::layer1::architecture::Structure>,
) {
    for _event in events.read() {
        if let Some(mut structure) = structures.iter_mut().next() {
            structure.current_hp -= 50.0;
            if structure.current_hp < 0.0 {
                structure.current_hp = 0.0;
            }
        }

        chronicle_events.send(AddChronicleEvent {
            text: "A thawed criminal sabotaged colony infrastructure!".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// INT-887: Bridges `SunkCostUpkeep` (Sunk-Cost Monument) to `ColonyResources` (Economy).
pub fn sunk_cost_resource_drain_system(
    query: Query<(
        Entity,
        &crate::layer1::architecture::sunk_cost_monument::SunkCostUpkeep,
    )>,
    mut resources: ResMut<crate::layer1::economy::resources::ColonyResources>,
    mut cancel_events: EventWriter<
        crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent,
    >,
) {
    for (entity, upkeep) in query.iter() {
        let cost = upkeep.base_cost * upkeep.multiplier.powf(upkeep.ticks_building as f32);

        if resources.get_amount(crate::layer1::economy::resources::ResourceType::Metal) >= cost {
            resources.consume(crate::layer1::economy::resources::ResourceType::Metal, cost);
        } else {
            cancel_events.send(
                crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent(entity),
            );
        }
    }
}

/// INT-1258: Bridges Cassandra Protocol to Chronicle.
pub fn cassandra_protocol_chronicle_bridge(
    protocol_events: Query<
        Entity,
        Added<crate::layer1::cassandra_protocol::CassandraProtocolActive>,
    >,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in protocol_events.iter() {
        chronicle_events.send(AddChronicleEvent {
            text: "Cassandra Protocol activated. The colony is hoarding resources in preparation for a disaster.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// INT-1068: Bridges Crustal Tides (TidalForce) to AddChronicleEvent (Chronicle).
pub fn crustal_tide_chronicle_bridge(
    tidal_force: Res<crate::layer2::syzygy::TidalForce>,
    mut last_tide: Local<f32>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    let high_tide = tidal_force.current > 0.7;
    let low_tide = tidal_force.current < 0.3;
    let prev_high = *last_tide > 0.7;
    let prev_low = *last_tide < 0.3;

    if high_tide && !prev_high {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "Extreme tidal forces cause the planet's crust to groan and fracture."
                .to_string(),
        });
    } else if low_tide && !prev_low {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "The low tide allows the planet's crust to snap shut, crushing anything in the fissures.".to_string(),
        });
    }

    *last_tide = tidal_force.current;
}

/// INT-549: Bridges RepoManArrivalEvent to spawning a RepoMan and AddChronicleEvent
pub fn repo_man_arrival_bridge(
    mut commands: bevy_ecs::system::Commands,
    mut events: bevy_ecs::event::EventReader<crate::layer1::mind::sleep_debt::RepoManArrivalEvent>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        // Spawn a RepoMan targeting the pop
        commands.spawn(crate::layer1::mind::sleep_debt::RepoMan {
            target: event.target_pop,
        });

        // Log it to the Chronicle
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "Corporate Repo Men have arrived to collect unpaid sleep debt!".to_string(),
        });
    }
}

/// Bridges `EmbezzlementEvent` to `AddChronicleEvent`.
pub fn embezzlement_chronicle_bridge(
    mut events: EventReader<crate::layer1::architecture::embezzlement::EmbezzlementEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!("A corrupt governor secretly embezzled {} materials to build a hidden parasitic structure in the colony.", event.embezzled_amount),
            importance: EventImportance::Minor,
        });
    }
}

/// INT-1114: Bridges GenerationalAmnesia to AddChronicleEvent
pub fn generational_amnesia_chronicle_bridge(
    query: Query<&crate::layer1::psychology::generational_amnesia::GenerationalAmnesia>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    mut last_logged: Local<bool>,
) {
    let mut high_amnesia = false;
    for amnesia in query.iter() {
        if amnesia.current_amnesia >= 90.0 {
            high_amnesia = true;
            break;
        }
    }

    if high_amnesia && !*last_logged {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "Generational amnesia has reached a critical level. Our past is forgotten."
                .to_string(),
        });
        *last_logged = true;
    } else if !high_amnesia {
        *last_logged = false;
    }
}

/// INT-1057: Bridges `Morale` to `ColonyRenown` (The Living Score).
///
/// Averages the morale across all pops and updates the global `ColonyRenown`
/// score (scaling from 0.0 - 1.0 to 0.0 - 100.0).
#[allow(clippy::cast_precision_loss)]
pub fn update_renown_from_morale_system(
    mut renown: ResMut<crate::layer1::living_score::ColonyRenown>,
    query: Query<&crate::layer1::morale::Morale, With<crate::layer1::pop::Pop>>,
) {
    let mut total_morale = 0.0;
    let mut count = 0;

    for morale in query.iter() {
        total_morale += morale.value;
        count += 1;
    }

    if count > 0 {
        let avg_morale = total_morale / count as f32;
        renown.score = avg_morale * 100.0;
    }
}

/// INT-1102: Bridges FirstShip destruction to AddChronicleEvent (Chronicle).
pub fn first_ship_destruction_chronicle_bridge(
    mut removed: RemovedComponents<crate::layer1::culture::cult_of_first_ship::FirstShip>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    if !removed.is_empty() {
        for _ in removed.read() {}
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "The First Ship, our original colony vessel and sacred monument, has been destroyed! The people are devastated.".to_string(),
        });
    }
}

/// INT-480: Bridges TruthOutbreakEvent to AddChronicleEvent (Chronicle).
pub fn truth_outbreak_chronicle_bridge(
    mut events: EventReader<crate::layer1::psychology::memory_forgery::TruthOutbreakEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "The truth has been revealed. A fabricated reality collapses as pops discover the memory forgery.".to_string(),
        });
    }
}

/// INT-1069: Bridges ExistentialAuditCompletedEvent to AddChronicleEvent (Chronicle).
pub fn existential_audit_chronicle_bridge(
    mut events: EventReader<
        crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent,
    >,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if event.passed {
            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: "The precursor AI audit concluded. The colony's cultural output justifies its industrial footprint.".to_string(),
            });
        } else {
            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: "The precursor AI audit failed! Our industrial expansion lacks meaningful cultural weight. An existential crisis sweeps the colony.".to_string(),
            });
        }
    }
}

/// INT-1207: Bridges KineticBattery destruction to AddChronicleEvent (Chronicle).
pub fn kinetic_battery_chronicle_bridge(
    query: Query<
        (
            &crate::layer1::kinetic_storage::KineticBattery,
            &crate::layer1::map::GridPosition,
        ),
        Added<crate::layer1::health::Dead>,
    >,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for (battery, pos) in query.iter() {
        if battery.charge > 10.0 {
            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: format!(
                    "A kinetic storage battery suffered a catastrophic structural failure, releasing {} damage at ({}, {}).",
                    battery.charge, pos.x, pos.y
                ),
            });
        }
    }
}

/// INT-878: Bridges SolarFlareEvent to AddChronicleEvent (Chronicle).
pub fn solar_flare_chronicle_bridge(
    mut flare_events: EventReader<crate::layer1::nature::solar_flare_lottery::SolarFlareEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in flare_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A massive solar flare has struck the colony! Unshielded electronics are damaged and exposed pops are at risk of radiation sickness.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-550: Bridges OrbitalDecayEvent (Gravity Siphon) to AddChronicleEvent (Chronicle).
pub fn gravity_siphon_chronicle_bridge(
    mut events: EventReader<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!("The massive gravitational pull of the Micro-Singularity Generator has destabilized the planet's orbit (Anomaly Strength: {:.1}). Asteroids are drawing dangerously close.", event.anomaly_strength),
            importance: crate::layer1::core::chronicle::EventImportance::Legendary,
        });
    }
}

// --- INT-1286: Ruin Integration ---

#[derive(bevy_ecs::prelude::Component)]
pub struct AncientRuin {
    pub is_active: bool,
}

#[derive(bevy_ecs::prelude::Component)]
pub struct TemperatureRegulation {
    pub bonus: f32,
}

#[derive(bevy_ecs::prelude::Component)]
pub struct MachineryTrigger {
    pub threshold: u64,
}

/// Applies the temperature regulation bonus from an AncientRuin to all pops residing in its Housing.
pub fn apply_ruin_environmental_buffs_system(
    ruins: bevy_ecs::prelude::Query<&TemperatureRegulation, bevy_ecs::prelude::With<AncientRuin>>,
    housing_query: bevy_ecs::prelude::Query<(
        bevy_ecs::prelude::Entity,
        &crate::layer1::architecture::housing::Housing,
    )>,
    mut temps: bevy_ecs::prelude::Query<
        &mut crate::layer1::environment::terminator_habitats::EnvironmentalTemperature,
    >,
) {
    for (housing_entity, housing) in housing_query.iter() {
        if let Ok(regulator) = ruins.get(housing_entity) {
            for &resident in &housing.residents {
                if let Ok(mut temp) = temps.get_mut(resident) {
                    temp.degrees = (temp.degrees + regulator.bonus)
                        .min(regulator.bonus + 10.0)
                        .max(10.0);
                }
            }
        }
    }
}

/// Randomly activates AncientRuin machinery if the simulation tick exceeds the threshold.
pub fn trigger_ruin_machinery_system(
    time: bevy_ecs::prelude::Res<crate::shared::time::SimulationTime>,
    mut ruins: bevy_ecs::prelude::Query<(&mut AncientRuin, &MachineryTrigger)>,
) {
    for (mut ruin, trigger) in ruins.iter_mut() {
        if time.tick >= trigger.threshold && !ruin.is_active {
            // Give it a 5% chance to activate per tick once past threshold
            let roll: f32 = rand::random();
            if roll < 0.05 {
                ruin.is_active = true;
            }
        }
    }
}

/// Applies psychological stress to residents of active AncientRuins.
use crate::layer1::architecture::building::Building;
use crate::layer1::architecture::ruins::Ruin;
use crate::layer3::guilt::PsychicResonance;

/// Bridges the gap between Ruins with PsychicResonance and newly constructed Buildings.
/// When a Building is placed on a tile that has a Ruin with PsychicResonance,
/// the building absorbs the resonance.
#[derive(bevy_ecs::query::QueryData)]
pub struct BuildingRegretQuery {
    entity: Entity,
    pos: &'static GridPosition,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct BuildingRegretFilter {
    _with: With<Building>,
    _without: Without<PsychicResonance>,
}

#[derive(bevy_ecs::query::QueryData)]
pub struct RuinRegretQuery {
    entity: Entity,
    pos: &'static GridPosition,
    resonance: &'static PsychicResonance,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct RuinRegretFilter {
    _with: With<Ruin>,
}

pub fn architecture_of_regret_bridge_system(
    mut commands: Commands,
    buildings: Query<BuildingRegretQuery, BuildingRegretFilter>,
    ruins: Query<RuinRegretQuery, RuinRegretFilter>,
) {
    for b_item in buildings.iter() {
        for r_item in ruins.iter() {
            if b_item.pos == r_item.pos {
                let b_entity = b_item.entity;
                let r_entity = r_item.entity;
                let resonance = r_item.resonance;
                commands.entity(b_entity).insert(PsychicResonance {
                    intensity: resonance.intensity,
                });
                commands.entity(r_entity).remove::<PsychicResonance>();
                // Ruin may be despawned or kept, but its resonance is gone.
            }
        }
    }
}

pub fn apply_ruin_psychological_stress_system(
    ruins: bevy_ecs::prelude::Query<&AncientRuin>,
    housing_query: bevy_ecs::prelude::Query<(
        bevy_ecs::prelude::Entity,
        &crate::layer1::architecture::housing::Housing,
    )>,
    mut pops: bevy_ecs::prelude::Query<&mut crate::layer1::psychology::stress::StressTracker>,
) {
    for (housing_entity, housing) in housing_query.iter() {
        if let Ok(ruin) = ruins.get(housing_entity) {
            if ruin.is_active {
                for &resident in &housing.residents {
                    if let Ok(mut stress) = pops.get_mut(resident) {
                        stress.accumulated_stress += 5.0; // Fixed stress penalty per tick
                    }
                }
            }
        }
    }
}

/// INT-1010: Bridges AncientRuins excavation to AddChronicleEvent
pub fn archaeological_contagion_chronicle_bridge(
    mut events: EventReader<ExcavationEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if event.discovery_type == "AncientRuins" {
            chronicle_events.send(AddChronicleEvent {
                text: "Miners breached Ancient Ruins, exposing the colony to a memetic contagion. The Ancient Routines have begun.".to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// INT-1112: Periodically triggers `FrackEvent` on `TectonicFracker` buildings when waste is high.
pub fn trigger_tectonic_fracking_system(
    mut events: bevy_ecs::event::EventWriter<crate::layer1::geology::fracking::FrackEvent>,
    query: bevy_ecs::system::Query<
        bevy_ecs::entity::Entity,
        bevy_ecs::query::With<crate::layer1::geology::fracking::TectonicFracker>,
    >,
    resources: Option<bevy_ecs::system::Res<crate::layer1::resources::ColonyResources>>,
    mut timer: bevy_ecs::system::Local<f32>,
    time: bevy_ecs::system::Res<bevy_time::Time>,
) {
    *timer += time.delta_secs();
    if *timer >= 10.0 {
        *timer = 0.0;
        if let Some(res) = resources {
            if res.waste >= 50.0 {
                for entity in query.iter() {
                    events.send(crate::layer1::geology::fracking::FrackEvent { entity });
                }
            }
        }
    }
}

/// INT-1112: Bridges `FrackEvent` to `AddChronicleEvent` (Chronicle).
pub fn tectonic_fracking_chronicle_bridge(
    mut frack_events: bevy_ecs::event::EventReader<crate::layer1::geology::fracking::FrackEvent>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    let mut fracked = false;
    for _ in frack_events.read() {
        fracked = true;
    }
    if fracked {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "Tectonic fracking has been initiated, injecting toxic waste deep into the crust for fuel. The ground groans in protest.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1118: Bridges Lost Tech recovery to AddChronicleEvent (Chronicle) and ColonyResources (knowledge).
pub fn tether_stump_lost_tech_bridge(
    mut inventories: Query<&mut crate::layer1::economy::inventory::Inventory>,
    mut resources: ResMut<ColonyResources>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for mut inventory in inventories.iter_mut() {
        if let Some(index) = inventory
            .items
            .iter()
            .position(|item| item.item_type == crate::layer1::economy::items::ItemType::LostTech)
        {
            inventory.items.swap_remove(index);
            resources.add_knowledge(50.0);
            chronicle_events.send(AddChronicleEvent {
                text: "Lost Tech recovered from the Tether Stump yielded vast knowledge."
                    .to_string(),
                importance: EventImportance::Major,
            });
        }
    }
}

/// INT-1298: Bridges Cultural Vandalism (`Defaced` or `Vandalized`) to AddChronicleEvent (Chronicle).
pub fn cultural_vandalism_chronicle_bridge(
    new_defaced: Query<Entity, Added<crate::layer1::social::cultural_vandalism::Defaced>>,
    new_vandalized: Query<Entity, Added<crate::layer1::social::cultural_vandalism::Vandalized>>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    if !new_defaced.is_empty() || !new_vandalized.is_empty() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "A structure has been vandalized, turning a symbol of power into a focal point for rebellion!".to_string(),
        });
    }
}

fn upsert_grudge(
    grudge_list: &mut crate::layer1::social::inherited_grudges::GrudgeList,
    target: bevy_ecs::entity::Entity,
    impact: f32,
) {
    let mut found = false;
    for grudge in &mut grudge_list.0 {
        if grudge.target_entity == target {
            grudge.intensity += impact.abs();
            found = true;
            break;
        }
    }

    if !found {
        grudge_list
            .0
            .push(crate::layer1::social::inherited_grudges::Grudge {
                target_entity: target,
                intensity: impact.abs(),
                origin_reason: "Public grievance".to_string(),
            });
    }
}

/// INT-1121: Bridges `PostGrievanceEvent` to `Grudge` components to link negative social interactions
/// to the formation of long-lasting generational grudges.
pub fn public_grievance_grudge_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::social::grievances::PostGrievanceEvent>,
    mut query: bevy_ecs::system::Query<&mut crate::layer1::social::inherited_grudges::GrudgeList>,
    mut commands: bevy_ecs::system::Commands,
) {
    use bevy::utils::HashMap;
    use bevy_ecs::entity::Entity;
    // ⚡ Bolt Optimization: Uses `bevy::utils::HashMap` (AHash) instead of `std::collections::HashMap`
    // to eliminate SipHash overhead for `Entity` keys, avoiding bottlenecks during heavy grievance processing.

    // To prevent multiple inserts on the same entity overwriting each other in the same frame
    let mut pending_inserts: HashMap<Entity, crate::layer1::social::inherited_grudges::GrudgeList> =
        HashMap::new();

    for event in events.read() {
        if event.impact < 0.0 {
            // It's a grievance
            if let Ok(mut grudges) = query.get_mut(event.poster) {
                upsert_grudge(&mut grudges, event.target, event.impact);
            } else {
                let grudge_list = pending_inserts.entry(event.poster).or_insert_with(|| {
                    crate::layer1::social::inherited_grudges::GrudgeList(Vec::new())
                });

                upsert_grudge(grudge_list, event.target, event.impact);
            }
        }
    }

    for (entity, grudge_list) in pending_inserts {
        commands.entity(entity).insert(grudge_list);
    }
}

/// Updates `PopulationCount` when a `PopDied` event is received.
pub fn pop_died_count_system(
    mut events: EventReader<crate::layer1::pop::PopDied>,
    mut pop_count: ResMut<crate::layer1::pop::PopulationCount>,
) {
    for _ in events.read() {
        pop_count.total = pop_count.total.saturating_sub(1);
    }
}

/// INT-1036: Bridges `Added<Manual>` to `AddChronicleEvent`
pub fn institutional_memory_chronicle_bridge(
    query: Query<(), Added<crate::layer1::institutional_memory::Manual>>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in query.iter() {
        chronicle_events.send(AddChronicleEvent {
            text: "A high-skill colonist has authored an instructional manual, passing their knowledge down to future generations.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Updates `PopulationCount` when a `PopBorn` event is received.
pub fn pop_born_count_system(
    mut events: EventReader<crate::layer1::pop::PopBorn>,
    mut pop_count: ResMut<crate::layer1::pop::PopulationCount>,
) {
    for _ in events.read() {
        pop_count.total = pop_count.total.saturating_add(1);
    }
}

/// INT-1291: Bridges Invasive Xeno-Aesthetics (`AestheticDeprivation`) to AddChronicleEvent (Chronicle).
pub fn invasive_xeno_aesthetics_chronicle_bridge(
    new_deprivation: Query<
        Entity,
        Added<crate::layer1::culture::invasive_xeno_aesthetics::AestheticDeprivation>,
    >,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    if !new_deprivation.is_empty() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
            text: "A growing aesthetic deprivation among the populace has led to public dissatisfaction, as foreign art and style become the preferred norm.".to_string(),
        });
    }
}

/// INT-1277: Bridges GhostCode infection to AddChronicleEvent (Chronicle).
pub fn ghost_code_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::core::events::BuildingCompletedEvent>,
    query: bevy_ecs::system::Query<&crate::layer1::tech::ghost_code::GhostCode>,
    mut chronicle_events: bevy_ecs::event::EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        if query.get(event.entity).is_ok() {
            chronicle_events.send(AddChronicleEvent {
                text: "A newly constructed building has inherited phantom protocols from the site's previous structure.".to_string(),
                importance: EventImportance::Minor,
            });
        }
    }
}

/// INT-1302: Bridges `SymbioticSalvageEvent` to `AddChronicleEvent` (Chronicle).
pub fn symbiotic_salvage_chronicle_bridge(
    mut events: EventReader<crate::layer1::shipbreaking_symbiotic::SymbioticSalvageEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    generator: Res<NarrativeGenerator>,
    colony: Res<ColonyName>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        let year = (1 + time.tick / TICKS_PER_YEAR).to_string();

        let mut ctx = NarrativeContext::new();
        ctx.insert("COLONY", &colony.name);
        ctx.insert("YEAR", &year);
        ctx.insert("DERELICT_NAME", "Unknown Derelict");
        ctx.insert(
            "RESOURCE_GAINED",
            format!("{:.1} Stellar Alloy", event.resource_gained),
        );

        let text = generator
            .generate("SYMBIOTIC_SALVAGE", &ctx)
            .unwrap_or_else(|_| "A derelict was salvaged.".to_string());

        chronicle_events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

/// Bridges `ColonyResources` to `ResourceStorage` for the Scarcity Bureaucracy.
pub fn sync_scarcity_resources_bridge_system(
    resources: Query<&ColonyResources, With<crate::layer1::bureaucracy_of_scarcity::Colony>>,
    mut storage: Query<
        &mut crate::layer1::bureaucracy_of_scarcity::ResourceStorage,
        With<crate::layer1::bureaucracy_of_scarcity::Colony>,
    >,
) {
    if let (Ok(res), Ok(mut store)) = (resources.get_single(), storage.get_single_mut()) {
        store.food = res.food.max(0.0).min(u32::MAX as f32) as u32;
    }
}

/// Applies the `GlobalRationingModifier` to `Needs.hunger` decay by restoring a portion of the decay.
pub fn apply_scarcity_rationing_bridge_system(
    colonies: Query<
        &crate::layer1::bureaucracy_of_scarcity::GlobalRationingModifier,
        With<crate::layer1::bureaucracy_of_scarcity::Colony>,
    >,
    mut consumers: Query<(&mut Needs, Option<&crate::layer1::traits::Traits>), With<Pop>>,
) {
    if let Ok(modifier) = colonies.get_single() {
        if modifier.reduction_percent > 0.0 {
            for (mut needs, traits) in consumers.iter_mut() {
                let hunger_trait_mod =
                    traits.map_or(1.0, crate::layer1::traits::get_trait_hunger_decay_modifier);
                let restored_hunger = 0.001 * hunger_trait_mod * modifier.reduction_percent;
                needs.hunger = (needs.hunger + restored_hunger).min(1.0);
            }
        }
    }
}

/// Syncs real Job components to the Scarcity Bureaucracy's JobAssignment.
pub fn sync_scarcity_jobs_bridge_system(
    mut commands: Commands,
    pops: Query<(Entity, Option<&crate::layer1::pop::Job>), With<Pop>>,
) {
    for (entity, job_opt) in pops.iter() {
        let is_bureaucrat = job_opt.is_some_and(|job| {
            job.job_type == crate::layer1::utility_types::AssignmentType::RationingBureaucrat
        });
        commands
            .entity(entity)
            .insert(crate::layer1::bureaucracy_of_scarcity::JobAssignment {
                is_active_bureaucrat: is_bureaucrat,
            });
    }
}

/// INT-1041: Bridges `SwarmHostileEvent` to `AddChronicleEvent` for Orphaned Swarm.
pub fn orphaned_swarm_chronicle_bridge(
    mut events: EventReader<crate::layer2::events_new::orphaned_swarm::SwarmHostileEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Orphaned Swarm's corruption has reached critical levels. They have turned hostile and are attacking the population to 'optimize' it!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

pub fn edible_architecture_chronicle_bridge(
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    q_edible: Query<(), Added<crate::layer1::architecture::edible::Consumed>>,
) {
    for _ in q_edible.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The walls are Edible Architecture now. We consumed the mycelial scaffolding to survive.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1306: Architectural Superstition Bridge
///
/// Listens to `PopDied`, `BuildingRemovedEvent`, and `PopDiedInAccidentEvent`.
/// When these occur, it looks for nearby buildings and adds a `NegativeEvent` to their `NegativeEventHistory`.
#[derive(bevy_ecs::query::QueryData)]
#[query_data(mutable)]
pub struct BuildingNegativeHistoryQuery {
    pos: &'static crate::layer1::core::map::GridPosition,
    history: &'static mut crate::layer1::architecture_superstition::NegativeEventHistory,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct BuildingNegativeHistoryFilter {
    _with: bevy_ecs::query::With<crate::layer1::architecture::Building>,
}

pub fn track_negative_events_bridge_system(
    mut pop_died_events: bevy_ecs::event::EventReader<crate::layer1::pop::PopDied>,
    mut building_removed_events: bevy_ecs::event::EventReader<
        crate::layer1::core::events::BuildingRemovedEvent,
    >,
    mut pop_died_accident_events: bevy_ecs::event::EventReader<
        crate::layer1::haunted_assembly_lines::PopDiedInAccidentEvent,
    >,
    pops_query: bevy_ecs::system::Query<&crate::layer1::core::map::GridPosition>,
    pos_query: bevy_ecs::system::Query<&crate::layer1::core::map::GridPosition>,
    mut building_query: bevy_ecs::system::Query<
        BuildingNegativeHistoryQuery,
        BuildingNegativeHistoryFilter,
    >,
    time: bevy_ecs::system::Res<crate::shared::time::SimulationTime>,
) {
    use crate::layer1::architecture_superstition::NegativeEvent;

    // ⚡ Bolt Optimization: Pre-allocate vector capacity to avoid reallocation overhead during event processing.
    let capacity =
        pop_died_events.len() + building_removed_events.len() + pop_died_accident_events.len();
    let mut negative_locations = Vec::with_capacity(capacity);

    for event in pop_died_events.read() {
        if let Ok(pos) = pops_query.get(event.entity) {
            negative_locations.push((*pos, 5.0)); // Base severity for death
        }
    }

    for event in building_removed_events.read() {
        negative_locations.push((event.position, 3.0)); // Base severity for destruction
    }

    for event in pop_died_accident_events.read() {
        if let Ok(pos) = pos_query.get(event.location) {
            negative_locations.push((*pos, 7.0)); // Higher severity for workplace accident
        }
    }

    if negative_locations.is_empty() {
        return;
    }

    for mut item in building_query.iter_mut() {
        for (event_pos, severity) in &negative_locations {
            let dx = item.pos.x.abs_diff(event_pos.x);
            let dy = item.pos.y.abs_diff(event_pos.y);
            // If within 5 tiles
            if dx <= 5 && dy <= 5 {
                item.history.events.push(NegativeEvent {
                    severity: *severity,
                    #[allow(clippy::cast_precision_loss)]
                    time: time.tick as f32,
                });
            }
        }
    }
}

use crate::layer1::systems::dead_hand::DoomsdayTriggeredEvent;

/// INT-1026: Bridges `LotteryExecutedEvent` to `AddChronicleEvent`
pub fn lottery_chronicle_bridge(
    mut events: EventReader<crate::layer1::the_lottery::LotteryExecutedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: format!(
                "The Lottery was executed. {} Pop(s) sacrificed so that {} might live. The survivors are traumatized.",
                event.sacrifice_count, event.survivor_count
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Bridges `DoomsdayTriggeredEvent` to `AddChronicleEvent`
pub fn dead_hand_chronicle_bridge(
    mut events: EventReader<DoomsdayTriggeredEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Legendary,
            text: format!(
                "The Dead Hand was triggered by device {:?}! A doomsday scenario is unfolding.",
                event.device
            ),
        });
    }
}

/// INT-1300: Phantom Commutes Bridge
///
/// Listens for `BuildingRemovedEvent` and updates any Pop whose current `MovementTarget` matches the removed
/// building's position. It replaces normal pathing with a `HabituatedRoute` pointing to the removed building,
/// causing a "phantom commute".
pub fn phantom_commutes_bridge_system(
    mut commands: bevy_ecs::system::Commands,
    mut events: bevy_ecs::event::EventReader<crate::layer1::core::events::BuildingRemovedEvent>,
    q_pops: bevy_ecs::system::Query<
        (
            bevy_ecs::entity::Entity,
            &crate::layer1::map::GridPosition,
            &crate::layer1::execution::components::MovementTarget,
        ),
        bevy_ecs::query::With<crate::layer1::pop::Pop>,
    >,
    mut chronicle: bevy_ecs::event::EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for event in events.read() {
        for (entity, pos, target) in q_pops.iter() {
            if target.target_position == event.position {
                commands.entity(entity).insert(
                    crate::layer1::execution::components::HabituatedRoute {
                        path: vec![*pos, event.position],
                        urgency: 1.0,
                        frustration: 0,
                        last_pos: None,
                    },
                );

                chronicle.send(crate::layer1::core::chronicle::AddChronicleEvent {
                    text: format!(
                        "A Pop continues to walk towards {x},{y} in a daze, attempting a phantom commute to a destroyed building.",
                        x = event.position.x,
                        y = event.position.y
                    ),
                    importance: crate::layer1::core::chronicle::EventImportance::Minor,
                });
            }
        }
    }
}

/// Component indicating that a pop's Void Stare manifestation has been logged to the chronicle.
#[derive(Component)]
pub struct VoidStareChronicleLogged;

type UnloggedVoidStarePopQuery<'a> = (Entity, &'a PopAction);
type UnloggedVoidStarePopFilter = (With<Pop>, Without<VoidStareChronicleLogged>);

/// Bridges the Void Stare manifestation to the Chronicle.
pub fn void_stare_chronicle_bridge(
    mut commands: Commands,
    pops: Query<UnloggedVoidStarePopQuery, UnloggedVoidStarePopFilter>,
    logged_pops: Query<(Entity, &PopAction), With<VoidStareChronicleLogged>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    // Log newly staring pops
    for (entity, action) in pops.iter() {
        if action.current == ActionType::VoidStare {
            commands.entity(entity).insert(VoidStareChronicleLogged);
            chronicle.send(AddChronicleEvent {
                text: "A colonist has lost themselves staring into the Abyss.".to_string(),
                importance: EventImportance::Standard,
            });
        }
    }

    // Remove the marker if they stop staring
    for (entity, action) in logged_pops.iter() {
        if action.current != ActionType::VoidStare {
            commands.entity(entity).remove::<VoidStareChronicleLogged>();
        }
    }
}

/// INT-555: Bridges `SabotageEvent` from Symbiotic Insurgency to `Building` access control and structure damage.
pub fn symbiont_sabotage_bridge_system(
    mut commands: Commands,
    faction: Res<crate::layer1::biology::symbiotic_insurgency::SymbiontFaction>,
    mut events: EventReader<crate::layer1::biology::symbiotic_insurgency::SabotageEvent>,
    mut buildings: Query<(
        Entity,
        &crate::layer1::architecture::Building,
        Option<&mut crate::layer1::access_control::AccessControl>,
        Option<&mut crate::layer1::architecture::Structure>,
        Option<&mut crate::layer1::control::DoorControl>,
    )>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    use crate::layer1::access_control::AccessMode;
    use crate::layer1::architecture::BuildingType;
    use crate::layer1::biology::symbiotic_insurgency::{
        SabotageTarget, SABOTAGED_AIRLOCK_RESEAL_TICKS, SABOTAGE_BASE_DAMAGE,
        SABOTAGE_DAMAGE_PER_MEMBER, SABOTAGE_MAX_DAMAGE,
    };
    use crate::layer1::control::DoorState;

    for event in events.read() {
        match event.target {
            SabotageTarget::Airlocks => {
                for (entity, building, access, _, door) in buildings.iter_mut() {
                    if building.building_type == BuildingType::Airlock {
                        if let Some(mut access) = access {
                            access.mode = AccessMode::Public;
                        }
                        // Physically force the airlock open: it vents until
                        // the crew wrestles it shut (reseal system below).
                        if let Some(mut door) = door {
                            door.state = DoorState::Open;
                            commands.entity(entity).insert(
                                crate::layer1::biology::symbiotic_insurgency::SabotagedAirlock {
                                    ticks_remaining: SABOTAGED_AIRLOCK_RESEAL_TICKS,
                                },
                            );
                        }
                    }
                }
                chronicle.send(AddChronicleEvent {
                    text: "Critical sabotage! Symbiont faction forces airlocks open to let the spores in!".to_string(),
                    importance: EventImportance::Major,
                });
            }
            SabotageTarget::AirFiltration => {
                // Sabotage escalates with the faction: a lone carrier is a
                // nuisance, a entrenched faction guts life support faster
                // than a single repair crew can keep up with.
                let damage = (SABOTAGE_BASE_DAMAGE
                    + SABOTAGE_DAMAGE_PER_MEMBER * faction.members as f32)
                    .min(SABOTAGE_MAX_DAMAGE);
                for (_, building, _, structure, _) in buildings.iter_mut() {
                    if building.building_type == BuildingType::LifeSupport {
                        if let Some(mut structure) = structure {
                            structure.current_hp -= damage;
                            if structure.current_hp < 0.0 {
                                structure.current_hp = 0.0;
                            }
                        }
                    }
                }
                chronicle.send(AddChronicleEvent {
                    text: format!(
                        "Air filtration systems sabotaged by Symbiont faction ({:.0} damage to life support).",
                        damage
                    ),
                    importance: EventImportance::Minor,
                });
            }
        }
    }
}

/// Counts down sabotaged airlocks; when the timer expires the crew forces
/// the doors shut again. This is the colony's counterplay against the
/// critical airlock-venting sabotage: the leak is real, but it ends.
pub fn reseal_sabotaged_airlocks_system(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut crate::layer1::biology::symbiotic_insurgency::SabotagedAirlock,
        &mut crate::layer1::control::DoorControl,
    )>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    use crate::layer1::control::DoorState;
    for (entity, mut sabotaged, mut door) in query.iter_mut() {
        if sabotaged.ticks_remaining > 0 {
            sabotaged.ticks_remaining -= 1;
        }
        if sabotaged.ticks_remaining == 0 {
            door.state = DoorState::Auto;
            commands
                .entity(entity)
                .remove::<crate::layer1::biology::symbiotic_insurgency::SabotagedAirlock>();
            chronicle.send(AddChronicleEvent {
                text: "The crew wrestles a sabotaged airlock shut; the hissing stops."
                    .to_string(),
                importance: EventImportance::Standard,
            });
        }
    }
}

/// INT-1128: Bridges Hypno-Learning to Chronicle
pub fn hypno_learning_chronicle_bridge(
    query: Query<
        &crate::layer1::pop::PopName,
        Added<crate::layer1::tech::hypno_learning::MentalFog>,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for name in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: format!(
                "{} awoke from a Hypno-Learning session with severe mental fog.",
                name.0
            ),
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
        });
    }
}

/// INT-1307: Bridges `EarlyWarningEvent` to `AddChronicleEvent`
pub fn early_warning_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<
        crate::layer1::economy::information_black_market::EarlyWarningEvent,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: format!(
                "The Information Black Market intercepted an early warning: {} (Sector {}).",
                event.event_type, event.sector_id
            ),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1296: Bridges Subconscious Grid Lockdown to Chronicle Events
pub fn subconscious_grid_lockdown_chronicle_bridge(
    mut events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    query: Query<
        &crate::layer1::infrastructure::subconscious_grid::SmartGrid,
        Changed<crate::layer1::infrastructure::subconscious_grid::SmartGrid>,
    >,
) {
    for grid in query.iter() {
        if grid.state == crate::layer1::infrastructure::subconscious_grid::GridState::Lockdown {
            events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: "The Subconscious Grid has instinctively triggered a full lockdown protocol to protect us.".to_string(),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
        }
    }
}

/// INT-1211: Heavy Industry -> Tectonic Stress bridge
/// Increases tectonic stress based on the smog output of heavy industries.
pub fn heavy_industry_tectonic_stress_bridge(
    mut stress: ResMut<crate::layer1::geology::tectonic::TectonicStress>,
    industry_query: Query<&crate::layer1::environment::atmosphere::HeavyIndustry>,
) {
    for industry in industry_query.iter() {
        stress.current += industry.smog_output * 0.05;
    }
}

/// Bridges the `Dead` component on a `ColonyPet` to a `PetDeathEvent` (Spec 1288).
pub fn pet_death_bridge_system(
    query: Query<
        (Entity, &crate::layer1::culture::memorial_revolt::ColonyPet),
        Added<crate::layer1::biology::health::Dead>,
    >,
    mut events: EventWriter<crate::layer1::culture::memorial_revolt::PetDeathEvent>,
) {
    for (entity, pet) in query.iter() {
        events.send(crate::layer1::culture::memorial_revolt::PetDeathEvent {
            pet_entity: entity,
            owner_entity: pet.owner,
        });
    }
}

/// INT-1271: The Propaganda Graffitists -> Chronicle bridge
/// Emits a chronicle event when rebellious graffiti appears.
pub fn propaganda_graffiti_chronicle_bridge(
    mut events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    query: Query<
        &crate::layer1::social::propaganda_graffitists::RebelliousGraffiti,
        Added<crate::layer1::social::propaganda_graffitists::RebelliousGraffiti>,
    >,
) {
    for _ in query.iter() {
        events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "Subversive graffiti has appeared on colony infrastructure, echoing the unrest of its creators.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Standard,
        });
    }
}

/// INT-1017: Bridges `ReformatCommand` to `AddChronicleEvent`
pub fn reformat_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::tech::legacy_code::ReformatCommand>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text:
                "The Colony's Computer Core has initiated a total reformat. Systems going offline."
                    .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

#[derive(bevy_ecs::query::QueryData)]
pub struct GravityPlatingQuery {
    entity: bevy_ecs::prelude::Entity,
    consumer: &'static crate::layer1::energy::PowerConsumer,
}

#[derive(bevy_ecs::query::QueryFilter)]
pub struct GravityPlatingFilter {
    _with: bevy_ecs::prelude::With<crate::layer1::physics::gravity_plating::GravityGenerator>,
    _changed: bevy_ecs::prelude::Changed<crate::layer1::energy::PowerConsumer>,
}

pub fn gravity_plating_power_bridge_system(
    query: bevy_ecs::prelude::Query<GravityPlatingQuery, GravityPlatingFilter>,
    mut events: bevy_ecs::prelude::EventWriter<
        crate::layer1::physics::gravity_plating::PowerGridEvent,
    >,
) {
    for item in query.iter() {
        if !item.consumer.active {
            let entity = item.entity;
            events.send(crate::layer1::physics::gravity_plating::PowerGridEvent { node: entity });
        }
    }
}

/// INT-664: Bridges CultFormationEvent to AddChronicleEvent
pub fn scrap_code_cult_formation_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<
        crate::layer1::social::scrap_code_prophets::CultFormationEvent,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "Whispers of the Scrap Code spread. The Cult of the Broken Machine claims another follower.".to_string(),
        });
    }
}

/// INT-1308: Bridges `Added<BlackoutBazaar>` to `AddChronicleEvent`
pub fn blackout_bazaar_chronicle_bridge(
    query: bevy_ecs::system::Query<
        (),
        bevy_ecs::query::Added<crate::layer1::blackout_bazaars::BlackoutBazaar>,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A Blackout Bazaar has formed in the unpowered social areas.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1018: Bridges `MegaEvent` to `AddChronicleEvent`
pub fn planetary_scarring_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::disasters::mega_event::MegaEvent>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        if event.intensity >= 50.0 {
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!(
                    "Planetary scarring detected: A massive {} struck the planet.",
                    event.event_type
                ),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
        }
    }
}

/// INT-993: Bridges `LogicCascadeEvent` to `AddChronicleEvent`
pub fn fractal_bureaucracy_chronicle_bridge(
    mut events: EventReader<crate::layer1::administration::fractal_bureaucracy::LogicCascadeEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "Logic Cascade! The fractal bureaucracy has broken down.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// INT-1310: Bridges `Added<UnprocessedForms>` to `AddChronicleEvent`
pub fn feral_administration_chronicle_bridge(
    query: bevy_ecs::system::Query<
        (),
        (
            bevy_ecs::query::Added<
                crate::layer1::administration::feral_administration::UnprocessedForms,
            >,
            bevy_ecs::query::With<crate::layer1::administration::feral_administration::FeralColony>,
        ),
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A Feral Administration has begun to produce an avalanche of paperwork."
                .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1311: Bridges `Added<DreamingSickness>` to `AddChronicleEvent`
pub fn dreaming_sickness_chronicle_bridge(
    query: bevy_ecs::system::Query<
        (),
        bevy_ecs::query::Added<crate::layer1::psychology::dreaming_sickness::DreamingSickness>,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
    mut triggered: bevy_ecs::system::Local<bool>,
) {
    if !*triggered && query.iter().next().is_some() {
        *triggered = true;
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A strange dreaming sickness has begun to spread among the colony.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}
/// INT-1267: Bridges `LuredByGhostFleet` to `AddChronicleEvent` and despawns the pop.
pub fn lured_pops_escape_bridge_system(
    mut commands: bevy_ecs::system::Commands,
    query: bevy_ecs::system::Query<
        bevy_ecs::entity::Entity,
        bevy_ecs::query::Added<crate::layer1::anomalies::echoes_lost_fleet::LuredByGhostFleet>,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A militaristic pop was lured away by the broadcasts of a Ghost Dreadnought."
                .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-299: Bridges `ParadoxEvent` to `AddChronicleEvent`
pub fn temporal_smuggling_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::tech::temporal_smuggling::ParadoxEvent>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A Temporal Paradox has occurred due to defaulted debt!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1005: Bridges `Added<HazardFlora>` to `AddChronicleEvent`
pub fn subterranean_biosphere_chronicle_bridge(
    query: bevy_ecs::prelude::Query<
        (),
        bevy_ecs::query::Added<crate::layer1::nature::biosphere_inversion::HazardFlora>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "Deep crust mining has unleashed aggressive subterranean flora into the shafts!"
                .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

pub fn architecture_of_paranoia_chronicle_bridge(
    surveillance_query: bevy_ecs::prelude::Query<
        (),
        bevy_ecs::query::Added<crate::layer1::social::subversion::Surveillance>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in surveillance_query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A new Surveillance installation has been constructed. The colony is watching its own.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1002: Bridges LeviathanEclipse to LumifloraCycle hibernation
pub fn bio_rhythmic_commute_eclipse_bridge(
    eclipse: Option<bevy_ecs::prelude::Res<crate::layer2::void_leviathan::LeviathanEclipse>>,
    mut query: bevy_ecs::prelude::Query<
        &mut crate::layer1::flora::LumifloraCycle,
        bevy_ecs::query::With<crate::layer1::flora::Flora>,
    >,
) {
    if eclipse.is_some_and(|e| e.active) {
        for mut cycle in query.iter_mut() {
            if cycle.phase != crate::layer1::flora::BloomPhase::Hibernation {
                cycle.phase = crate::layer1::flora::BloomPhase::Hibernation;
                cycle.time_in_phase = 0.0;
            }
        }
    }
}

/// INT-1002: Bridges SolarFlareEvent to LumifloraCycle hibernation
pub fn bio_rhythmic_commute_flare_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer1::nature::solar_flare_lottery::SolarFlareEvent,
    >,
    mut query: bevy_ecs::prelude::Query<
        &mut crate::layer1::flora::LumifloraCycle,
        bevy_ecs::query::With<crate::layer1::flora::Flora>,
    >,
) {
    if events.read().next().is_some() {
        for mut cycle in query.iter_mut() {
            if cycle.phase != crate::layer1::flora::BloomPhase::Hibernation {
                cycle.phase = crate::layer1::flora::BloomPhase::Hibernation;
                cycle.time_in_phase = 0.0;
            }
        }
    }
}

pub fn subspace_stowaways_chronicle_bridge(
    mut wake_events: EventReader<crate::layer2::ftl::wakes::SubspaceWakeEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for wake in wake_events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "A subspace wake disrupted the colony, tearing at reality with severity {:.2}.",
                wake.severity
            ),
        });
    }
}
pub fn xenoflora_pet_death_bridge_system(
    mut events: EventReader<crate::layer1::social::pets::PetDeathEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    query: Query<&crate::layer1::pop::PopName>,
) {
    for event in events.read() {
        if let Ok(name) = query.get(event.owner_entity) {
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!(
                    "{} lost their beloved xenological pet today. The colony mourns with them.",
                    name.0
                ),
                importance: crate::layer1::core::chronicle::EventImportance::Minor,
            });
        }
    }
}

/// INT-1136: Bridges `TaskAdministrativelyOptimizedEvent` (Sentient Bureaucracy) to `AddChronicleEvent`
pub fn sentient_bureaucracy_chronicle_bridge(
    mut events: EventReader<
        crate::layer1::administration::sentient_bureaucracy::TaskAdministrativelyOptimizedEvent,
    >,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Sentient Bureaucracy has autonomously reassigned a task, overriding player input for the sake of optimization.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
        });
    }
}
pub fn spatial_compression_chronicle_bridge(
    mut events: EventReader<crate::layer1::spatial_compression::PocketCollapseEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A Pocket Dimension collapsed!".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-496: Bridges starvation deaths to AtrocityScore
pub fn starvation_atrocity_bridge(
    mut events: bevy_ecs::event::EventReader<crate::layer1::entities::pop::PopDied>,
    mut atrocity: bevy_ecs::system::ResMut<crate::layer1::social::bureau_of_regrets::AtrocityScore>,
) {
    for event in events.read() {
        if event.reason == "Starvation" {
            atrocity.score += 10.0;
        }
    }
}
pub fn bureaucratic_black_hole_chronicle_bridge(
    query: bevy_ecs::system::Query<
        (),
        bevy_ecs::query::Added<
            crate::layer1::administration::bureaucratic_black_hole::BureaucraticBlackHole,
        >,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "A Bureaucratic Black Hole has formed! Admin density has reached critical mass, and items will now be lost in paperwork.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-1139: Bridges `PoleFlipEvent` to `AddChronicleEvent`
pub fn magnetic_reversal_chronicle_bridge(
    mut pole_flip_events: EventReader<crate::layer1::physics::magnetic_reversal::PoleFlipEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in pole_flip_events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text:
                "The planetary magnetic poles flipped, exposing the surface to intense radiation."
                    .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

/// INT-695: Bridges The Long Night to AddChronicleEvent
pub fn long_night_chronicle_bridge(
    mut events: bevy_ecs::event::EventReader<
        crate::layer1::nature::long_night::StartLongNightEvent,
    >,
    long_night: bevy_ecs::system::Res<crate::layer1::nature::long_night::LongNightEvent>,
    mut was_active: bevy_ecs::system::Local<bool>,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for _ in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Long Night has begun. The sun is blocked, and temperatures are plummeting."
                .to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }

    if !*was_active && long_night.is_active {
        *was_active = true;
    } else if *was_active && !long_night.is_active {
        *was_active = false;
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Long Night has ended. The sun has finally returned.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

use crate::layer1::accidental_terraforming::BiomeShiftEvent;
use crate::layer1::architecture::potemkin::PotemkinDestroyedEvent;
use crate::layer1::economy::biomass_dividend::RecycleEvent;

pub fn accidental_terraforming_chronicle_bridge(
    mut events: EventReader<BiomeShiftEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "Accidental terraforming caused a biome shift.".to_string(),
            importance: EventImportance::Major,
        });
    }
}

pub fn biomass_dividend_chronicle_bridge(
    mut events: EventReader<RecycleEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "A corpse was recycled for biomass rations.".to_string(),
            importance: EventImportance::Minor,
        });
    }
}

pub fn potemkin_chronicle_bridge(
    mut events: EventReader<PotemkinDestroyedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            text: "A Potemkin facade was destroyed.".to_string(),
            importance: EventImportance::Minor,
        });
    }
}

#[derive(bevy_ecs::prelude::Component)]
pub struct EgoThresholdReached;

/// INT-321: Bridges EgoStat threshold to Chronicle Events
pub fn ego_stat_chronicle_bridge(
    mut commands: bevy_ecs::prelude::Commands,
    mut events: bevy_ecs::event::EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    query: bevy_ecs::prelude::Query<
        (
            bevy_ecs::prelude::Entity,
            &crate::layer1::tech::ego_machine::EgoStat,
            Option<&crate::layer1::entities::pop::PopName>,
        ),
        bevy_ecs::prelude::Without<EgoThresholdReached>,
    >,
) {
    for (entity, ego, name_opt) in query.iter() {
        if ego.value > 50.0 {
            let name = name_opt.map_or("A Pop", |n| n.0.as_str());
            events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!("{} demands a luxury suite, refusing to haul scrap.", name),
                importance: crate::layer1::core::chronicle::EventImportance::Major,
            });
            commands.entity(entity).insert(EgoThresholdReached);
        }
    }
}
#[derive(bevy_ecs::prelude::Component)]
pub struct ConsultantChronicleEventFired;

pub fn consultant_chronicle_bridge(
    mut commands: bevy_ecs::prelude::Commands,
    query: bevy_ecs::prelude::Query<
        (
            bevy_ecs::prelude::Entity,
            &crate::layer1::consultant::ConsultantMarker,
        ),
        bevy_ecs::prelude::Without<ConsultantChronicleEventFired>,
    >,
    mut chronicle_events: bevy_ecs::event::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for (entity, _marker) in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Consultant has arrived. Efficiency is paramount.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
        commands
            .entity(entity)
            .insert(ConsultantChronicleEventFired);
    }
}
