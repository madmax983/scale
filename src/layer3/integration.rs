//! Layer 3 Integration
//!
//! Exposes the plugin and registration schedules that wire the Layer 3 subsystems
//! (Market, Council, Diplomacy, Fleets) into the global Bevy App schedule.

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer3::planet::black_market_terraforming::RogueTerraformEvent;
#[allow(unused_imports)]
use bevy_ecs::prelude::*;

/// Bridges `TradeRouteSeveredEvent` (Hyperlane Collapse) to `AddChronicleEvent` (Chronicle).
pub fn hyperlane_collapse_chronicle_bridge(
    mut sever_events: EventReader<crate::layer3::map::TradeRouteSeveredEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in sever_events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "A major hyperlane has collapsed due to stellar drift. Trade routes are severed, reshaping the galactic geography."
                .to_string(),
        });
    }
}

pub fn dead_internet_chronicle_bridge(
    mut events: EventReader<crate::layer3::diplomacy::dead_internet::DiplomaticInteraction>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle.send(AddChronicleEvent {
            importance: EventImportance::Standard,
            text: "A hauntingly familiar trade protocol echoes from the void... an automated script from a dead civilization.".to_string(),
        });
    }
}

/// Bridges `RogueTerraformEvent` (Black Market Terraforming) to `AddChronicleEvent` (Chronicle).
pub fn black_market_terraforming_bridge(
    mut events: EventReader<RogueTerraformEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "Unseasonal terraforming in Sector {} caused extreme local weather disruptions",
                event.target_sector
            ),
        });
    }
}

use crate::layer3::diplomacy::succession::{SuccessionCrisisEvent, SuccessionEvent};

/// Bridges `SuccessionEvent` to `AddChronicleEvent`
pub fn dynastic_succession_chronicle_bridge(
    mut succession_events: EventReader<SuccessionEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in succession_events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "Succession in {}: {} has died. {} takes the throne.",
                event.faction_name, event.old_leader_name, event.new_leader_name
            ),
        });
    }
}

/// Bridges `SuccessionCrisisEvent` to `AddChronicleEvent`
pub fn dynastic_crisis_chronicle_bridge(
    mut crisis_events: EventReader<SuccessionCrisisEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in crisis_events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Legendary,
            text: format!(
                "Succession crisis in {}! {} has died without an heir. The realm bleeds.",
                event.faction_name, event.old_leader_name
            ),
        });
    }
}

use crate::layer3::diplomacy::god_mind::GodMindWarDeclared;

/// Bridges `GodMindWarDeclared` to `AddChronicleEvent`: the Eternal Ruler's
/// corrupted-memory war becomes history.
pub fn god_mind_war_chronicle_bridge(
    mut war_events: EventReader<GodMindWarDeclared>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in war_events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "The Eternal Ruler of {} declares a corrupted-memory war on {} — an enemy that no longer exists.",
                event.faction_name, event.target_name
            ),
        });
    }
}

use crate::layer2::navigation::stellar_weather::FleetDamagedEvent;
use crate::layer3::stellar_cartography::JumpRisk;

/// Bridges `JumpRisk` (Stellar Cartography) to `FleetDamagedEvent` and `AddChronicleEvent` (Chronicle).
pub fn jump_risk_bridge_system(
    mut commands: Commands,
    query: Query<Entity, With<JumpRisk>>,
    mut damage_events: EventWriter<FleetDamagedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for entity in query.iter() {
        // Remove the risk component so we don't repeatedly damage
        commands.entity(entity).remove::<JumpRisk>();

        // Apply a flat minor damage for jumping blind
        damage_events.send(FleetDamagedEvent {
            fleet: entity,
            amount: 20.0,
        });

        // Add to chronicle
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "A fleet suffered hull damage after jumping blind into an uncharted system."
                .to_string(),
        });
    }
}

use crate::layer3::diplomacy::red_tape_defense::BureaucraticHold;

/// Bridges `BureaucraticHold` to `AddChronicleEvent` (Chronicle).
pub fn red_tape_chronicle_bridge(
    query: Query<Entity, Added<BureaucraticHold>>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in query.iter() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "Hostile fleet stalled by bureaucratic red tape.".to_string(),
        });
    }
}

use crate::layer1::law::penal::OrganHarvestedEvent;
use crate::layer3::diplomacy_reflection::{Civilization, DiplomaticTraits, TraitChangedEvent};

use crate::layer2::trade::routes::{Colony, TradeRouteExecutedEvent};
use crate::layer3::linguistic_drift::LinguisticNetwork;

/// Bridges `TradeRouteExecutedEvent` and `LinguisticNetwork` to apply a Translation Tax
pub fn language_drift_trade_bridge(
    mut events: EventReader<TradeRouteExecutedEvent>,
    mut colonies: Query<&mut Colony>,
    network: Res<LinguisticNetwork>,
) {
    for event in events.read() {
        let drift = network.get_drift(event.source, event.destination);

        // Applying Translation Tax: High drift imposes a tax on trade efficiency.
        // We'll calculate tax as 0.1% per point of drift, capped at 90%
        let tax_rate = (drift / 1000.0).clamp(0.0, 0.9);
        let tax_amount = (event.amount as f32 * tax_rate) as u32;

        if tax_amount > 0 {
            if let Ok(mut dest_colony) = colonies.get_mut(event.destination) {
                dest_colony.remove_resource(&event.item_type, tax_amount);
            }
        }
    }
}

/// INT-1292: Galactic Games -> Chronicle
pub fn galactic_games_chronicle_bridge(
    games_event: Option<Res<crate::layer3::diplomacy::galactic_games::GalacticGamesEvent>>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
    mut last_winner: Local<Option<crate::layer1::social::factions::FactionId>>,
) {
    if let Some(games_event) = games_event {
        if games_event.is_changed() {
            if let Some(winner) = games_event.winner {
                if *last_winner != Some(winner) {
                    *last_winner = Some(winner);
                    chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                        importance: crate::layer1::core::chronicle::EventImportance::Legendary,
                        text: format!(
                            "The Galactic Games have concluded! Faction {:?} emerged victorious, claiming glory and influence.",
                            winner
                        ),
                    });
                }
            } else if games_event.active && last_winner.is_some() {
                // Games started again
                *last_winner = None;
                chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                    importance: crate::layer1::core::chronicle::EventImportance::Major,
                    text: "The Galactic Games have begun! Champions from across the galaxy gather to compete.".to_string(),
                });
            }
        }
    }
}

/// Bridges `DraftOrderEvent` (The Endless Draft) to `DraftComplianceEvent`, `DraftRefusalEvent`, and `AddChronicleEvent` (Chronicle).
pub fn endless_draft_bridge_system(
    mut commands: Commands,
    mut order_events: EventReader<crate::layer3::diplomacy::endless_draft::DraftOrderEvent>,
    mut compliance_events: EventWriter<
        crate::layer3::diplomacy::endless_draft::DraftComplianceEvent,
    >,
    mut refusal_events: EventWriter<crate::layer3::diplomacy::endless_draft::DraftRefusalEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    pop_query: Query<
        (Entity, Option<&crate::layer1::skills::Skills>),
        With<crate::layer1::pop::Pop>,
    >,
) {
    let mut drafted_pops = bevy::utils::HashSet::new();

    for event in order_events.read() {
        let mut eligible_pops = Vec::new();

        for (entity, skills_opt) in pop_query.iter() {
            if drafted_pops.contains(&entity) {
                continue;
            }

            let physical_stat = if let Some(skills) = skills_opt {
                let mining_xp = skills
                    .xp
                    .get(&crate::layer1::skills::SkillType::Mining)
                    .copied()
                    .unwrap_or(0.0);
                let forestry_xp = skills
                    .xp
                    .get(&crate::layer1::skills::SkillType::Forestry)
                    .copied()
                    .unwrap_or(0.0);
                mining_xp + forestry_xp // Approximation of physical stats
            } else {
                0.0
            };

            if physical_stat >= event.min_physical_stat {
                eligible_pops.push(entity);
            }
        }

        if eligible_pops.len() >= event.required_pops {
            // ⚡ Bolt Optimization: Remove intermediate `.collect::<Vec<_>>()` by truncating `eligible_pops` directly
            eligible_pops.truncate(event.required_pops);
            for &pop in &eligible_pops {
                drafted_pops.insert(pop);
                commands.entity(pop).despawn();
            }

            compliance_events.send(
                crate::layer3::diplomacy::endless_draft::DraftComplianceEvent {
                    sponsor: event.sponsor,
                    pops_provided: eligible_pops,
                },
            );

            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: format!("The colony has complied with the Draft Order. {} pops were conscripted and taken away.", event.required_pops),
            });
        } else {
            refusal_events.send(crate::layer3::diplomacy::endless_draft::DraftRefusalEvent {
                sponsor: event.sponsor,
            });

            chronicle_events.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: format!("The colony failed to meet the Draft Order quota of {} pops. We brace for the consequences.", event.required_pops),
            });
        }
    }
}

/// Bridges `OrganHarvestedEvent` to `DiplomaticTraits` for Layer 3 Diplomacy
pub fn organ_trade_diplomacy_bridge(
    mut harvest_events: EventReader<OrganHarvestedEvent>,
    mut civ_query: Query<(Entity, &mut DiplomaticTraits), With<Civilization>>,
    mut trait_events: EventWriter<TraitChangedEvent>,
) {
    if harvest_events.read().next().is_some() {
        for (entity, mut traits) in civ_query.iter_mut() {
            if !traits.is_barbarian {
                traits.is_barbarian = true;
                trait_events.send(TraitChangedEvent { civ_entity: entity });
            }
        }
    }
}

use crate::layer3::intellectual_property_wars::{CassusBelli, CassusBelliReason};

/// Bridges IP Piracy (CassusBelli) to DiplomaticTraits (is_barbarian).
///
/// When a civilization illegally uses a patented tech, a CassusBelli is generated.
/// This system catches the new CassusBelli and updates the pirating civilization's
/// traits to mark them as a barbarian (violator of galactic law), which then
/// triggers diplomatic sanctions from pacifist neighbors.
pub fn ip_piracy_diplomacy_bridge(
    cb_query: Query<&CassusBelli, Added<CassusBelli>>,
    mut civ_query: Query<(Entity, &mut DiplomaticTraits), With<Civilization>>,
    mut trait_events: EventWriter<TraitChangedEvent>,
) {
    for cb in cb_query.iter() {
        if cb.reason == CassusBelliReason {
            // Target is the one pirating the tech
            if let Ok((entity, mut traits)) = civ_query.get_mut(cb.target) {
                if !traits.is_barbarian {
                    traits.is_barbarian = true;
                    trait_events.send(TraitChangedEvent { civ_entity: entity });
                }
            }
        }
    }
}

use crate::layer1::resources::ColonyResources;
use crate::layer1::social::unrest::Unrest;
use crate::layer3::bureaucracy_of_truth::ColonyState;

/// Bridges Layer 1 `ColonyResources` and `Unrest` to Layer 3 `ColonyState`.
pub fn bureaucracy_of_truth_integration_system(
    resources: Option<Res<ColonyResources>>,
    unrest: Option<Res<Unrest>>,
    mut colonies: Query<&mut ColonyState>,
) {
    if let (Some(res), Some(unr)) = (resources, unrest) {
        // Find the main colony entity (assuming there's one for now)
        for mut state in colonies.iter_mut() {
            state.food_reserves = res.food as u32;
            state.unrest = unr.level * 100.0; // Assuming unrest level is 0.0-1.0
        }
    }
}

/// INT-585: Bridges AnomalyDiscoveredEvent (Cartographic Delusion) to AddChronicleEvent (Chronicle).
pub fn anomaly_discovered_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<crate::layer3::map::AnomalyDiscoveredEvent>,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Minor,
            text: format!("Anomaly Discovered in Sector {}", event.sector.0),
        });
    }
}

/// INT-681: Bridges FeralDeliveryTriggerEvent to AddChronicleEvent
pub fn feral_logistics_chronicle_bridge(
    mut events: EventReader<crate::layer2::trade::feral_logistics::FeralDeliveryTriggerEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "The Feral Network has dropped mysterious cargo.".to_string(),
        });
    }
}

/// INT-1060: Bridges StrandedEvent to AddChronicleEvent
pub fn stranded_fleet_chronicle_bridge(
    mut events: EventReader<crate::layer2::ship::logistics::StrandedEvent>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for _ in events.read() {
        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: "A fleet is stranded in deep space. Distress beacons activated.".to_string(),
        });
    }
}

use crate::layer2::piracy::PirateRepublic;

/// INT-1061: Bridges PirateRepublic creation to Layer 3 Diplomacy components.
pub fn pirate_republic_diplomacy_bridge(
    mut commands: Commands,
    query: Query<Entity, Added<PirateRepublic>>,
) {
    for entity in query.iter() {
        commands.entity(entity).insert((
            crate::layer3::diplomacy_reflection::Civilization {
                id: format!("PirateRepublic_{:?}", entity),
            },
            crate::layer3::diplomacy_reflection::DiplomaticTraits {
                is_barbarian: true,
                is_warlike: true,
                ..Default::default()
            },
            crate::layer3::diplomacy_reflection::DiplomaticRelations { relations: vec![] },
        ));
    }
}

/// INT-775: Bridges `ExportDumpEvent` (Quantum Famine) to `ColonyResources` and `AddChronicleEvent`.
pub fn quantum_famine_export_dump_bridge(
    mut dump_events: EventReader<crate::layer3::market::quantum_famine::ExportDumpEvent>,
    mut resources: ResMut<ColonyResources>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    for event in dump_events.read() {
        // Add credits from speculative markup
        resources.add_credits(event.credits_earned);

        chronicle_events.send(AddChronicleEvent {
            importance: EventImportance::Major,
            text: format!(
                "Market panic caused speculative fleets to buy out local {:?} stockpiles, bringing in {} credits but causing immediate shortages.",
                event.commodity, event.credits_earned
            ),
        });
    }
}

use crate::layer1::entities::pop::Pop;
use crate::layer3::bureaucracy::AutomatedReporting;

/// Marker component indicating a ghost town has been discovered.
#[derive(Component)]
pub struct GhostTownDiscovered;

/// INT-1101: Bridges `AutomatedReporting` with 0 population to `AddChronicleEvent`.
/// Discovers if a ghost town is hoarding resources while reporting a false population.
pub fn discover_ghost_town_system(
    mut commands: Commands,
    colonies: Query<(Entity, &AutomatedReporting), Without<GhostTownDiscovered>>,
    pops: Query<(), With<Pop>>,
    resources: Res<ColonyResources>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
) {
    let pop_count = pops.iter().count();

    // If the entire colony is dead, but they have resources and are still reporting
    if pop_count == 0 && resources.food > 0.0 {
        for (entity, reporting) in colonies.iter() {
            if reporting.is_active {
                // Mark as discovered to avoid spamming the chronicle
                commands.entity(entity).insert(GhostTownDiscovered);

                chronicle_events.send(AddChronicleEvent {
                    importance: EventImportance::Legendary,
                    text: format!(
                        "A Bureaucratic Ghost Town was discovered! Despite the population being wiped out, automated systems continued hoarding {} food, blinding the Empire.",
                        resources.food
                    ),
                });
            }
        }
    }
}

pub fn retro_contract_accepted_bridge_system(
    mut accepted_events: EventReader<
        crate::layer3::diplomacy::retro_contracts::AcceptRetroContractEvent,
    >,
    mut chronicle_events: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    for event in accepted_events.read() {
        chronicle_events.send(crate::layer1::chronicle::AddChronicleEvent {
            importance: crate::layer1::chronicle::EventImportance::Major,
            text: format!(
                "Accepted a retro-causality contract for {} credits.",
                event.credit_advance
            ),
        });
    }
}

pub fn retro_contract_failed_bridge_system(
    mut failed_events: EventReader<
        crate::layer3::diplomacy::retro_contracts::RetroContractFailedEvent,
    >,
    mut chronicle_events: EventWriter<crate::layer1::chronicle::AddChronicleEvent>,
) {
    for event in failed_events.read() {
        chronicle_events.send(crate::layer1::chronicle::AddChronicleEvent {
            importance: crate::layer1::chronicle::EventImportance::Major,
            text: format!(
                "Failed to fulfill a retro-causality contract, penalized {} credits.",
                event.penalty
            ),
        });
    }
}

/// Bridges the execution of trade routes to resetting the isolation level of the colonies involved.
pub fn reset_isolation_on_trade_system(
    mut trade_events: EventReader<crate::layer2::trade::routes::TradeRouteExecutedEvent>,
    mut colonies: Query<(
        Entity,
        &mut crate::experimental::the_weight_of_silence::ColonyNode,
    )>,
    tick: Option<Res<crate::shared::time::SimulationTime>>,
) {
    if let Some(tick) = tick {
        for event in trade_events.read() {
            if let Ok((_, mut colony)) = colonies.get_mut(event.destination) {
                colony.last_communication_tick = tick.tick;
                colony.isolation_level = 0.0;
            }
            if let Ok((_, mut colony)) = colonies.get_mut(event.source) {
                colony.last_communication_tick = tick.tick;
                colony.isolation_level = 0.0;
            }
        }
    }
}

pub fn silence_cult_chronicle_bridge(
    query: Query<Entity, Added<crate::experimental::the_weight_of_silence::SilenceCult>>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _ in query.iter() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Major,
            text: "A Silence Cult has emerged, demanding an end to external communication!"
                .to_string(),
        });
    }
}

/// INT-1287: Cargo Cult Diplomat -> Chronicle
/// Emits a Chronicle event when a CasusBelli is declared by a cargo cult.
pub fn cargo_cult_chronicle_bridge(
    new_wars: Query<
        &crate::layer3::diplomacy::cargo_cult_diplomat::CasusBelli,
        Added<crate::layer3::diplomacy::cargo_cult_diplomat::CasusBelli>,
    >,
    cults: Query<&crate::layer3::diplomacy::cargo_cult_diplomat::DivineAmbassador>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for casus_belli in new_wars.iter() {
        if cults.get(casus_belli.source).is_ok() {
            chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                importance: crate::layer1::core::chronicle::EventImportance::Major,
                text: "A primitive colony declared holy war over a desecrated divine ambassador!"
                    .to_string(),
            });
        }
    }
}

/// INT-1303: Bridges TradeRouteExecutedEvent to GalacticMarket, adding to the market supply.
pub fn trade_route_market_bridge_system(
    mut trade_events: bevy_ecs::event::EventReader<
        crate::layer2::trade::routes::TradeRouteExecutedEvent,
    >,
    mut market: bevy_ecs::system::ResMut<crate::layer3::market::GalacticMarket>,
) {
    for event in trade_events.read() {
        if let Some(res_type) = crate::layer2::trade::routes::parse_resource(&event.item_type) {
            let pool = market.supply_pool.entry(res_type).or_insert(0.0);
            *pool += event.amount as f32;
        }
    }
}

/// INT-1124: Bridges DiplomaticMeetingEvent to AddChronicleEvent based on Fashion matching
pub fn diplomatic_fashion_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<
        crate::layer3::diplomacy::diplomatic_fashion::DiplomaticMeetingEvent,
    >,
    query_ambassador: bevy_ecs::prelude::Query<
        &crate::layer3::diplomacy::diplomatic_fashion::PreferredAttire,
    >,
    query_envoy: bevy_ecs::prelude::Query<
        &crate::layer3::diplomacy::diplomatic_fashion::Apparel,
        bevy_ecs::prelude::With<crate::layer1::entities::pop::Pop>,
    >,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        if let Ok(preferred) = query_ambassador.get(event.ambassador) {
            if let Ok(apparel) = query_envoy.get(event.envoy) {
                let mut matched = false;
                for tag in &preferred.tags {
                    if apparel.tags.contains(tag) {
                        matched = true;
                        break;
                    }
                }

                if matched {
                    chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                        importance: crate::layer1::core::chronicle::EventImportance::Standard,
                        text: "A diplomatic meeting with a foreign ambassador went well thanks to our envoy's impeccable fashion sense.".to_string(),
                    });
                } else {
                    chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
                        importance: crate::layer1::core::chronicle::EventImportance::Major,
                        text: "A diplomatic incident! Our envoy's attire deeply offended the foreign ambassador.".to_string(),
                    });
                }
            }
        }
    }
}

/// INT-287: The Silence -> Chronicle
/// Bridges HostileSpawnEvent to AddChronicleEvent
pub fn the_silence_chronicle_bridge(
    mut events: bevy_ecs::prelude::EventReader<crate::layer3::silence::HostileSpawnEvent>,
    mut chronicle_events: bevy_ecs::prelude::EventWriter<
        crate::layer1::core::chronicle::AddChronicleEvent,
    >,
) {
    for event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            importance: crate::layer1::core::chronicle::EventImportance::Legendary,
            text: format!("The silence of the void is broken. Unknown hostile entities detected at the sector edge (Severity: {}).", event.severity),
        });
    }
}

/// INT-1011: Bridges `TributeDemandEvent` (Sovereign Armada) to `AddChronicleEvent`
pub fn sovereign_armada_chronicle_bridge(
    mut events: EventReader<crate::layer1::diplomacy::TributeDemandEvent>,
    mut chronicle_events: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    for _event in events.read() {
        chronicle_events.send(crate::layer1::core::chronicle::AddChronicleEvent {
            text: "The Sovereign Armada has arrived and demands tribute.".to_string(),
            importance: crate::layer1::core::chronicle::EventImportance::Major,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use bevy_ecs::prelude::*;

    #[test]
    fn test_diplomatic_fashion_chronicle_bridge_match() {
        use crate::layer1::entities::pop::Pop;
        use crate::layer3::diplomacy::diplomatic_fashion::{
            Apparel, AttireTag, DiplomaticMeetingEvent, PreferredAttire,
        };

        let mut app = bevy_app::App::new();
        app.add_event::<DiplomaticMeetingEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, diplomatic_fashion_chronicle_bridge);

        let ambassador = app
            .world_mut()
            .spawn(PreferredAttire {
                tags: vec![AttireTag::Ceremonial],
            })
            .id();

        let envoy = app
            .world_mut()
            .spawn((
                Pop,
                Apparel {
                    tags: vec![AttireTag::Ceremonial, AttireTag::Organic],
                },
            ))
            .id();

        app.world_mut()
            .resource_mut::<Events<DiplomaticMeetingEvent>>()
            .send(DiplomaticMeetingEvent {
                ambassador,
                envoy,
                player_civ_id: "Player".to_string(),
            });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Standard);
        assert!(events[0]
            .text
            .contains("went well thanks to our envoy's impeccable fashion sense"));
    }

    #[test]
    fn test_diplomatic_fashion_chronicle_bridge_mismatch() {
        use crate::layer1::entities::pop::Pop;
        use crate::layer3::diplomacy::diplomatic_fashion::{
            Apparel, AttireTag, DiplomaticMeetingEvent, PreferredAttire,
        };

        let mut app = bevy_app::App::new();
        app.add_event::<DiplomaticMeetingEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, diplomatic_fashion_chronicle_bridge);

        let ambassador = app
            .world_mut()
            .spawn(PreferredAttire {
                tags: vec![AttireTag::Ceremonial],
            })
            .id();

        let envoy = app
            .world_mut()
            .spawn((
                Pop,
                Apparel {
                    tags: vec![AttireTag::Organic],
                },
            ))
            .id();

        app.world_mut()
            .resource_mut::<Events<DiplomaticMeetingEvent>>()
            .send(DiplomaticMeetingEvent {
                ambassador,
                envoy,
                player_civ_id: "Player".to_string(),
            });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Major);
        assert!(events[0]
            .text
            .contains("deeply offended the foreign ambassador"));
    }

    #[test]
    fn test_the_silence_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<crate::layer3::silence::HostileSpawnEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, the_silence_chronicle_bridge);

        app.world_mut()
            .resource_mut::<Events<crate::layer3::silence::HostileSpawnEvent>>()
            .send(crate::layer3::silence::HostileSpawnEvent { severity: 10 });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Legendary);
        assert!(events[0].text.contains("The silence of the void is broken"));
    }

    #[test]
    fn test_hyperlane_collapse_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<crate::layer3::map::TradeRouteSeveredEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, hyperlane_collapse_chronicle_bridge);

        app.world_mut()
            .resource_mut::<Events<crate::layer3::map::TradeRouteSeveredEvent>>()
            .send(crate::layer3::map::TradeRouteSeveredEvent {
                system_a: Entity::PLACEHOLDER,
                system_b: Entity::PLACEHOLDER,
            });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Major);
        assert!(events[0].text.contains("A major hyperlane has collapsed"));
    }

    #[test]
    fn test_dead_internet_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<crate::layer3::diplomacy::dead_internet::DiplomaticInteraction>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, dead_internet_chronicle_bridge);

        app.world_mut()
            .resource_mut::<Events<crate::layer3::diplomacy::dead_internet::DiplomaticInteraction>>(
            )
            .send(crate::layer3::diplomacy::dead_internet::DiplomaticInteraction);

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Standard);
        assert!(events[0]
            .text
            .contains("A hauntingly familiar trade protocol echoes"));
    }

    #[test]
    fn test_black_market_terraforming_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<RogueTerraformEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, black_market_terraforming_bridge);

        app.world_mut()
            .resource_mut::<Events<RogueTerraformEvent>>()
            .send(RogueTerraformEvent { target_sector: 42 });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Major);
        assert!(events[0]
            .text
            .contains("Unseasonal terraforming in Sector 42"));
    }

    #[test]
    fn test_dynastic_succession_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<SuccessionEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, dynastic_succession_chronicle_bridge);

        app.world_mut()
            .resource_mut::<Events<SuccessionEvent>>()
            .send(SuccessionEvent {
                faction_name: "Empire".to_string(),
                old_leader_name: "Emperor".to_string(),
                new_leader_name: "Prince".to_string(),
            });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Major);
        assert!(events[0]
            .text
            .contains("Succession in Empire: Emperor has died. Prince takes the throne."));
    }

    #[test]
    fn test_dynastic_crisis_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<SuccessionCrisisEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, dynastic_crisis_chronicle_bridge);

        app.world_mut()
            .resource_mut::<Events<SuccessionCrisisEvent>>()
            .send(SuccessionCrisisEvent {
                faction_name: "Kingdom".to_string(),
                old_leader_name: "King".to_string(),
            });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, EventImportance::Legendary);
        assert!(events[0]
            .text
            .contains("Succession crisis in Kingdom! King has died without an heir."));
    }

    #[test]
    fn test_jump_risk_bridge_system() {
        let mut app = bevy_app::App::new();
        app.add_event::<FleetDamagedEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, jump_risk_bridge_system);

        let entity = app.world_mut().spawn(JumpRisk).id();

        app.update();

        assert!(
            app.world().get::<JumpRisk>(entity).is_none(),
            "JumpRisk should be removed"
        );

        let damage_events = app.world().resource::<Events<FleetDamagedEvent>>();
        let mut reader1 = damage_events.get_cursor();
        let d_events: Vec<&FleetDamagedEvent> = reader1.read(damage_events).collect();
        assert_eq!(d_events.len(), 1);
        assert_eq!(d_events[0].fleet, entity);
        assert_eq!(d_events[0].amount, 20.0);

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader2 = chronicle_events.get_cursor();
        let c_events: Vec<&AddChronicleEvent> = reader2.read(chronicle_events).collect();
        assert_eq!(c_events.len(), 1);
        assert_eq!(c_events[0].importance, EventImportance::Major);
        assert!(c_events[0]
            .text
            .contains("A fleet suffered hull damage after jumping blind"));
    }

    #[test]
    fn test_red_tape_chronicle_bridge() {
        let mut app = bevy_app::App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(bevy_app::Update, red_tape_chronicle_bridge);

        app.world_mut().spawn(BureaucraticHold {
            timer: 1.0,
            cost_multiplier: 1,
        });

        app.update();

        let chronicle_events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = chronicle_events.get_cursor();
        let c_events: Vec<&AddChronicleEvent> = reader.read(chronicle_events).collect();
        assert_eq!(c_events.len(), 1);
        assert_eq!(c_events[0].importance, EventImportance::Major);
        assert!(c_events[0]
            .text
            .contains("Hostile fleet stalled by bureaucratic red tape."));
    }
}
