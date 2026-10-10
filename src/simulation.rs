//! Shared simulation tick logic used by all entry points.
//!
//! Uses a Bevy `Schedule` to run all simulation systems. This enables:
//! - Typed system params (Bevy injects `Query`, `Res`, `ResMut` automatically)
//! - Automatic parallel execution of non-conflicting systems (with `multi_threaded`)
//! - `par_iter_mut` for intra-system parallelism on queries

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoSystemConfigs, Schedule, ScheduleLabel};

#[cfg(not(target_arch = "wasm32"))]
use crate::gpu::evaluate::gpu_evaluate_actions;
// On wasm32 the GPU backend is unavailable; alias the CPU fallback so the
// schedule below compiles unchanged.
#[cfg(target_arch = "wasm32")]
use crate::layer1::mind::utility_ai::evaluate_actions_system as gpu_evaluate_actions;
use crate::layer1::building::{update_building_map_system, BuildingMap};
use crate::layer1::systems::{register_layer1_systems, update_event_buffer, Layer1SystemSet};
use crate::layer1::update_action_timer_system;
use crate::layer2::events::{DetectionEvent, LaunchEvent, ShipDestroyedEvent};
use crate::layer3::silence::{
    check_hostile_spawn_system, update_detection_risk_system, DetectionRisk, HostileSpawnEvent,
};
use crate::shared::time::SimulationTime;

/// Schedule label for the main simulation tick.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimulationSchedule;

/// Build the simulation schedule with all systems and ordering constraints.
///
/// Systems are organized into ordered groups matching the original sequential execution:
///
/// ```text
/// 1. AI Decision:     gpu_evaluate_actions → update_action_timer
/// 2. Execution:       cleanup_previous → process_start_plan → movement → arrival → work/haul
/// 3. Economy:         update_resource_caps, advance_season, produce_food, process_refining,
///                     process_research, restore_rest, restore_leisure (can run in parallel)
/// 4. Consumption:     consume_food → decay_needs → kill_starving → clean_dead_*
/// 5. Observation:     track_plan_outcomes, biography, dreams, milestones (can run in parallel)
/// 6. Tick increment:  (handled outside schedule)
/// ```
#[must_use]
pub fn build_simulation_schedule() -> Schedule {
    let mut schedule = Schedule::new(SimulationSchedule);
    schedule
        .add_systems((
            crate::layer1::social::faction_diet::assign_faction_diets,
            crate::layer1::social::faction_diet::process_food_consumption,
        ))
        .add_systems(crate::layer2::planetary_scarring::process_planetary_scars_system);
    register_simulation_core_systems(&mut schedule);
    register_simulation_extended_systems(&mut schedule);
    schedule.add_systems(
        (
            crate::layer2::pulsar::pulsar_rotation_system,
            crate::layer2::pulsar::pulsar_radiation_damage_system,
        )
            .chain(),
    );
    schedule.add_systems(crate::layer2::void_leviathan::update_leviathan_eclipse_system);
    schedule.add_systems(crate::layer2::propaganda_engine::update_propaganda_system);
    schedule.add_systems((
        crate::layer1::social::bureau_of_regrets::check_penitent_faction_formation_system,
        crate::layer1::core::integration::starvation_atrocity_bridge,
    ));

    schedule.add_systems((
        crate::layer1::social::blacksite::process_blacksite_payout_system,
        crate::layer1::social::blacksite::prisoner_radicalization_system,
        crate::layer1::social::blacksite::prison_break_system,
        crate::layer1::biology::chromotaxis::chromotaxis_attraction_system,
        crate::layer1::biology::chromotaxis::chromotaxis_aggro_system,
    ));
    schedule.add_systems((
        crate::layer3::guilt::process_guilt_generation_system,
        crate::layer3::guilt::apply_guilt_unrest_system,
    ));
    schedule.add_systems((
        crate::layer1::biology::symbiotic_insurgency::transmit_mind_spore_infection_system,
        crate::layer1::biology::symbiotic_insurgency::process_mind_spore_infection_system,
        crate::layer1::biology::symbiotic_insurgency::fight_off_spore_infection_system,
        crate::layer1::biology::symbiotic_insurgency::release_dead_spore_hosts_system,
        crate::layer1::biology::symbiotic_insurgency::trigger_symbiont_sabotage_system,
        crate::layer1::core::integration::reseal_sabotaged_airlocks_system,
    ));
    schedule.add_systems((
        crate::layer1::architecture::hostage_protocol::hostage_protocol_suppression_system,
        crate::layer1::architecture::hostage_protocol::hostage_protocol_malfunction_system,
        crate::layer1::architecture::hostage_protocol::defuse_countdown_system,
        crate::layer1::architecture::hostage_protocol::hostage_protocol_detonation_system,
    ));
    schedule.add_systems((
        crate::layer1::administration::bureaucratic_black_hole::check_bureaucratic_density_system,
        crate::layer1::administration::bureaucratic_black_hole::bureaucratic_resource_loss_system,
        crate::layer1::administration::bureaucratic_black_hole::bureaucratic_pop_reassignment_system,
    ));
    schedule.add_systems((
        crate::layer1::nature::subterranean_smog::process_subterranean_smog_system,
        crate::layer1::nature::subterranean_smog::apply_smog_penalties_system,
        crate::layer1::tech::teleporter::psychosis::handle_teleport_system,
        crate::layer1::tech::teleporter::psychosis::process_psychosis_system,
        crate::layer1::tech::teleporter::psychosis::hunger_decay_system,
    ));
    schedule.add_systems((
        crate::layer1::biology::symbiotic_gear::symbiotic_hunger_modifier_system,
        crate::layer1::biology::symbiotic_gear::starving_symbiote_damage_system,
    ));
    // Spec 1370 Civilizational Compost: corpse decay -> soil enrichment.
    schedule.add_systems(
        (
            crate::layer1::agriculture::compost::tick_corpse_decay_system,
            crate::layer1::agriculture::compost::compost_decomposition_system,
            crate::layer1::agriculture::compost::apply_haunted_mood_system,
            crate::layer1::agriculture::compost::salvage_scrap_heap_system,
        )
            .chain(),
    );
    // Spec 1371 Panic Spirals: fear contagion — terrifying events panic pops,
    // panic spreads on contact, panicked pops flee and drop what they carry.
    schedule.add_systems(
        (
            crate::layer1::psychology::panic_spirals::trigger_panic_system,
            crate::layer1::psychology::panic_spirals::spread_panic_system,
            crate::layer1::psychology::panic_spirals::panic_timer_system,
            crate::layer1::psychology::panic_spirals::panic_flee_system,
        )
            .chain(),
    );
    // Spec 1373 Recalled Product: recall notices, 5% use-failure, stock returns.
    schedule.add_systems(
        (
            crate::layer1::economy::recall::process_recall_issuance_system,
            crate::layer1::economy::recall::process_item_usage_system,
            crate::layer1::economy::recall::process_item_return_system,
            crate::layer1::economy::recall::auto_recall_system,
        )
            .chain(),
    );
    // Spec 1372 Gravity-Fed Logistics: downhill chute flow is free,
    // level/uphill flow requires an active PowerConsumer (the pump).
    schedule.add_systems(
        crate::layer1::logistics::gravity::process_gravity_logistics_system,
    );
    // Spec 1375 Ship of Fools: pleasure-cruiser arrivals spawn zero-skill
    // Entitled passengers; the entitled grumble when food isn't abundant.
    schedule.add_systems((
        crate::layer1::social::ship_of_fools::process_ship_of_fools_arrival_system,
        crate::layer1::social::ship_of_fools::entitled_grumbling_system,
    ));
    // Spec 271 Subcontractor Factions: lease zones to megacorps (brutal law
    // override, rent in chunks, corporate rigs, security sweeps).
    schedule.add_systems(
        (
            crate::layer1::social::factions::subcontractor_factions::handle_leased_zones_system,
            crate::layer1::social::factions::subcontractor_factions::rent_collection_system,
            crate::layer1::social::factions::subcontractor_factions::megacorp_security_sweep_system,
            crate::layer1::social::factions::subcontractor_factions::auto_sweep_system,
        )
            .chain(),
    );
    // Spec 273 The Feral Outpost: frontier pops accumulate fringe exposure
    // far from the Command Center, cluster into named wild outposts, refuse
    // relocation outside their claimed territory, and go feral over time.
    schedule.add_systems(
        (
            crate::layer1::social::feral_outpost::update_cultural_drift_system,
            crate::layer1::social::feral_outpost::form_feral_outposts_system,
            crate::layer1::social::feral_outpost::develop_feral_traits_system,
        )
            .chain(),
    );
    // Spec 290 The Phantom Shift: Fringe pops work a secret night shift
    // repairing damaged structures off the books, growing a shadow economy
    // when the colony backlog is critical.
    schedule.add_systems(
        (
            crate::layer1::phantom_shift::update_inefficiency_tracker_system,
            crate::layer1::phantom_shift::phantom_work_execution_system,
        )
            .chain(),
    );
    // Spec 1208 The Cadet Branch: Homeworld courier arrivals spawn Noble
    // Scion pops (zero skills, Noble trait, monthly family allowance).
    schedule.add_systems(
        crate::layer1::social::cadet::process_scion_arrival_system,
    );
    // Spec 1376 Artifact Market: aging items are promoted to historical
    // artifacts, museums grant morale auras per housed artifact, and sales
    // pay credits at a colony-wide morale cost.
    schedule.add_systems(
        (
            crate::layer1::economy::artifact_market::item_aging_system,
            crate::layer1::economy::artifact_market::museum_aura_system,
            crate::layer1::economy::artifact_market::process_artifact_sale_system,
            crate::layer1::economy::artifact_market::process_museum_designation_system,
        )
            .chain(),
    );
    // Spec 270 The Organ Market: Biomass Extractor harvests organs from
    // corpses; organs sell for credits or cure critical injuries; harvests
    // spread colony-wide horror through the morale system.
    schedule.add_systems(
        (
            crate::layer1::economy::organ_market::process_corpse_harvest_system,
            crate::layer1::economy::organ_market::process_living_harvest_system,
            crate::layer1::economy::organ_market::apply_harvest_horror_system,
            crate::layer1::economy::organ_market::process_organ_sale_system,
            crate::layer1::economy::organ_market::process_transplant_system,
            crate::layer1::economy::organ_market::organ_spoilage_system,
        )
            .chain(),
    );

    schedule
}

/// Run one simulation tick: all game systems via schedule, then increment tick counter.
#[allow(clippy::too_many_lines)]
fn init_simulation_resources(world: &mut World) {
    world.init_resource::<crate::layer1::agriculture::compost::CompostSoil>();
    // Spec 1373 Recalled Product: manufacturer recalls, failure chance, returns.
    world.init_resource::<crate::layer1::economy::recall::RecallManager>();
    world.init_resource::<crate::layer1::economy::recall::RecallConfig>();
    // Spec 1375 Ship of Fools: its arrival-event resource was never initialized;
    // without this the first tick panics on a fresh world (pre-existing bug).
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::ship_of_fools::ShipOfFoolsArrivalEvent>>();
    // Spec 271 Subcontractor Factions: lease + sweep events.
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent>>();
    // Spec 1208 The Cadet Branch: scion arrival events.
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::cadet::ScionArrivalEvent>>();
    // Spec 1376 Artifact Market: market config, provenance registry, events.
    world.init_resource::<crate::layer1::economy::artifact_market::ArtifactMarketConfig>();
    world.init_resource::<crate::layer1::economy::artifact_market::ArtifactRegistry>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::artifact_market::SellArtifactEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::artifact_market::DesignateMuseumEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::recall::IssueRecallEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::recall::UseItemEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::recall::ReturnRecalledItemEvent>>();
    // Spec 270 The Organ Market: config + harvest/sale/transplant events.
    world.init_resource::<crate::layer1::economy::organ_market::OrganMarketConfig>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::organ_market::OrganHarvestEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::organ_market::HarvestLivingEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::organ_market::SellOrgansEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::organ_market::TransplantOrganEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::OpenRiftEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::PayTemporalDebtEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::ParadoxEvent>>();
    world.init_resource::<bevy_ecs::event::Events<
        crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent,
    >>();
    world.init_resource::<bevy_ecs::event::Events<
        crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
    >>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::shadow_ecosystems::ShortCircuitEvent>>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer2::orbit::tether::AsteroidCrashEvent>>(
        );
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::spatial_compression::PocketCollapseEvent>>();
    // Fallen Sovereign (adventurer origin)
    world.init_resource::<crate::layer1::culture::sovereign::SovereignState>();
    world.init_resource::<crate::layer1::culture::sovereign::FarmProductionLedger>();
    world.init_resource::<crate::layer1::culture::sovereign::LaborFervor>();
    world.init_resource::<crate::layer1::culture::sovereign::SporeSuppression>();
    // The Corsair (adventurer origin)
    world.init_resource::<crate::layer1::culture::corsair::CorsairState>();
    // Spec 272 The Martyrdom Effect: fervor state, enemy-strike ledger, casus belli.
    world.init_resource::<crate::layer1::social::martyrdom::Martyrdom>();
    world.init_resource::<crate::layer1::social::martyrdom::LastEnemyStrike>();
    world.init_resource::<crate::layer1::social::martyrdom::IdeologicalCasusBelli>();
    // Spec 275 Architectural Sabotage: vetting policy, director, failure event.
    world.init_resource::<crate::layer1::architecture::construction_sabotage::VettingPolicy>();
    world.init_resource::<crate::layer1::architecture::construction_sabotage::SabotageDirector>();
    world.init_resource::<Events<crate::layer1::architecture::construction_sabotage::BuildingFailureEvent>>();
    // Spec 277 Orbital Megastructure Deorbiting: event, countdown state, director.
    world.init_resource::<Events<crate::layer1::disasters::deorbit::DeorbitEvent>>();
    world.init_resource::<crate::layer1::disasters::deorbit::DeorbitState>();
    world.init_resource::<crate::layer1::disasters::deorbit::DeorbitDirector>();
    // The Planetary Governor (adventurer origin)
    world.init_resource::<crate::layer1::culture::governor::GovernorState>();
    world.init_resource::<crate::layer1::culture::governor::Treasury>();
    world.init_resource::<crate::layer1::culture::governor::QuotaOrder>();
    world.init_resource::<crate::layer1::culture::governor::RationOrder>();
    world.init_resource::<crate::layer1::culture::governor::WorksProgram>();
    world.init_resource::<crate::layer2::propaganda_engine::PropagandaEngine>();
    world.init_resource::<crate::layer2::propaganda_engine::DiplomaticWeight>();
    world.init_resource::<crate::layer2::propaganda_engine::InspectorEvent>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::core::chronicle::AddChronicleEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::architecture::potemkin::PotemkinDestroyedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::accidental_terraforming::BiomeShiftEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::biomass_dividend::RecycleEvent>>();
    world.init_resource::<bevy_ecs::event::Events<
        crate::layer1::administration::sentient_bureaucracy::TaskAdministrativelyOptimizedEvent,
    >>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::temperature::ThermalDamageEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::cross_layer::generation_ship_mutiny::MutinyEvent>>();

    world.init_resource::<bevy_ecs::event::Events<crate::cross_layer::interplanetary_pollination::SporeReleaseEvent>>();

    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::blacksite::PrisonBreakEvent>>();
    world.init_resource::<Events<crate::layer1::cassandra_syndrome::DoomsdayWarningEvent>>();
    world.init_resource::<Events<crate::layer1::cassandra_syndrome::DisasterOccurredEvent>>();
    world.init_resource::<Events<crate::layer3::market::phantom_tax::HackSlushFundEvent>>();
    world.init_resource::<crate::layer3::market::phantom_tax::SlushFund>();

    world.init_resource::<bevy_ecs::event::Events<crate::layer1::physics::gravity_plating::PowerGridEvent>>();
    world.init_resource::<crate::layer1::physics::gravity_plating::GlobalGravity>();
    world.init_resource::<crate::layer1::social::bureau_of_regrets::AtrocityScore>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer1::disasters::mega_event::MegaEvent>>(
        );
    world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
    world.init_resource::<crate::layer2::void_leviathan::VoidLeviathan>();
    world.init_resource::<crate::layer2::void_leviathan::LeviathanEclipse>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::scrap_code_prophets::CultFormationEvent>>();

    #[cfg(feature = "nova")]
    world.init_resource::<crate::experimental::ghost_grid::GhostGrid>();
    world.init_resource::<crate::layer1::environment::atmosphere::GlobalAtmosphere>();
    world.init_resource::<Events<crate::layer1::economy::information_black_market::EarlyWarningEvent>>();
    world.init_resource::<Events<crate::layer1::economy::black_market::SmugglerArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::economy::black_market::ShutdownDropNodeEvent>>();
    world.init_resource::<Events<crate::layer1::architecture::chrono_vault::SealVaultEvent>>();
    world.init_resource::<Events<crate::layer2::auction::VaultOpenedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::orbit::asteroid_claims::AttackColonyEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
    world.init_resource::<Events<crate::layer1::AcceptSponsorshipEvent>>();
    world.init_resource::<Events<crate::layer1::crafting::CraftEvent>>();
    world.init_resource::<Events<crate::layer1::biology::symbiotic_insurgency::SabotageEvent>>();
    world.insert_resource(crate::layer1::biology::symbiotic_insurgency::SymbiontFaction {
        members: 0,
        // INT-555: the trigger system escalates to critical (airlock) sabotage
        // at this size; the unit tests assume 40. A default of 0 made EVERY
        // sabotage event take the critical path.
        critical_mass: 40,
    });
    world.init_resource::<crate::layer1::biology::symbiotic_insurgency::InfectionConfig>();
    world
        .init_resource::<Events<crate::layer3::events::generational_debt::RepoFleetArrivalEvent>>();
    world.init_resource::<Events<crate::layer3::events::generational_debt::AttackRepoFleetEvent>>();
    world
        .init_resource::<Events<crate::layer1::psychology::memory_blackout::MemoryBlackoutEvent>>();
    world.init_resource::<Events<crate::layer1::RepairBuildingEvent>>();
    world.init_resource::<Events<crate::layer2::trade::routes::TradeRouteExecutedEvent>>();
    world
        .init_resource::<Events<crate::layer2::trade::feral_logistics::FeralDeliveryTriggerEvent>>(
        );
    world.init_resource::<Events<crate::layer2::ship::logistics::StrandedEvent>>();

    world.init_resource::<Events<crate::layer1::social::hedonic_treadmill::ConsumeItemEvent>>();
    world.init_resource::<Events<crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::DeclarationOfIndependenceEvent>>();
    world
        .init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::WarDeclarationEvent>>(
        );
    world
        .init_resource::<Events<crate::layer1::agriculture::zero_g_flora::DepressurizationEvent>>();
    world.init_resource::<Events<crate::layer1::anomalies::echo::SpawnEchoSourceEvent>>();
    world.init_resource::<Events<crate::layer1::anomalies::void_sirens::SirenSignalEvent>>();
    world.init_resource::<Events<crate::layer1::agriculture::pollination::GrowthCycleEvent>>();
    world.init_resource::<Events<crate::layer1::memetics::memetic_hazards::ConversationEvent>>();

    world.init_resource::<Events<crate::layer1::economy::black_market::SmugglerArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::economy::black_market::ShutdownDropNodeEvent>>();
    world.init_resource::<Events<crate::layer1::economy::apex_diet::ConsumeFoodEvent>>();
    world.init_resource::<crate::layer1::economy::apex_diet::ApexMeatStores>();
    world.init_resource::<Events<crate::layer1::culture::nostalgia::RumorSpreadEvent>>();
    world.init_resource::<crate::layer1::biology::cybernetic_ascendancy::ColonyAverageUtility>();
    // Initialize schedule on first call (stored in World's Schedules resource)
    if !world.contains_resource::<Schedules>() {
        world.insert_resource(Schedules::default());
    }

    world.init_resource::<crate::layer1::social::old_guard::Demographics>();
    world.init_resource::<crate::layer1::skills::generational_atrophy::AutomationLevel>();

    world.init_resource::<crate::layer1::diplomacy::factions::rivals::TerritoryGrid>();

    world.init_resource::<BuildingMap>();

    world.init_resource::<crate::layer1::stress::TraumaTracker>();
    // Spec 276 The Flesh Famine.
    world.init_resource::<crate::layer1::social::flesh_famine::FleshFamineState>();
    world.init_resource::<Events<crate::layer1::social::flesh_famine::PlantBlight>>();
    world.init_resource::<crate::layer1::economy::deep_sleep_syndicates::ProductionModifier>();
    world
        .init_resource::<Events<crate::layer1::economy::deep_sleep_syndicates::ThawSyndicateEvent>>(
        );
    world.init_resource::<Events<crate::layer2::skyhooks::LaunchIntent>>();
    world.init_resource::<Events<crate::layer1::tech::rogue_automation_cults::MachineCultFormedEvent>>();
    world
        .init_resource::<Events<crate::layer3::diplomacy::fading_homeworld::CoreWorldDemandEvent>>(
        );
    world
        .init_resource::<Events<crate::layer3::diplomacy::fading_homeworld::PlayerDemandResponse>>(
        );
    world.init_resource::<Events<crate::layer1::heirloom_tool::EquipHeirloomEvent>>();
    world.init_resource::<Events<crate::layer1::infrastructure::ancient::ConduitSurgeEvent>>();
    world.init_resource::<Events<crate::layer1::ransom_broker::RansomDemandEvent>>();
    world.init_resource::<Events<crate::layer1::ransom_broker::PayRansomEvent>>();
    world.init_resource::<Events<crate::layer1::ransom_broker::RefuseRansomEvent>>();
    world.init_resource::<Events<crate::layer1::ransom_broker::PopRansomedEvent>>();
    world.init_resource::<Events<crate::layer1::ransom_broker::PopLostToPiratesEvent>>();

    world.init_resource::<crate::layer1::tech_envy::TechEnvyConfig>();

    world.init_resource::<crate::layer1::shadow_market::ShadowMarketCooldown>();

    world.init_resource::<crate::layer3::bureaucracy_of_vanity::ActiveDemands>();
    world.init_resource::<crate::layer1::administration::invasive_bureaucracy::EmpireStability>();
    if !world.contains_resource::<crate::layer3::bureaucracy_of_vanity::ImperialStanding>() {
        world.insert_resource(crate::layer3::bureaucracy_of_vanity::ImperialStanding { value: 50 });
    }
    if !world.contains_resource::<crate::layer3::bureaucracy_of_vanity::GlobalEfficiency>() {
        world
            .insert_resource(crate::layer3::bureaucracy_of_vanity::GlobalEfficiency { value: 1.0 });
    }

    // Initialize Layer 2 Events
    world.init_resource::<Events<crate::layer1::geography::HistoricalEvent>>();
    world
        .init_resource::<Events<crate::layer1::social::ghost_shift_strike::GhostShiftStartedEvent>>(
        );
    world.init_resource::<Events<crate::layer3::fleets::ColonyFoundedEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftOrderEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftComplianceEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftRefusalEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::VeteranReturnEvent>>();
    world.init_resource::<crate::layer1::diplomacy::factions::rivals::TerritoryGrid>();
    world.init_resource::<Events<crate::layer3::diplomacy::succession::SuccessionEvent>>();
    world
        .init_resource::<Events<crate::layer3::diplomacy::xenolinguistics::MessageResponseEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::succession::SuccessionCrisisEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::god_mind::GodMindEdict>>();
    world.init_resource::<Events<crate::layer3::diplomacy::god_mind::GodMindSchism>>();
    world.init_resource::<Events<crate::layer3::diplomacy::god_mind::GodMindWarDeclared>>();
    world.init_resource::<Events<crate::layer3::diplomacy::diplomatic_fashion::DiplomaticMeetingEvent>>();
    world.init_resource::<crate::layer1::mind::fugue::FugueEventTracker>();
    world.init_resource::<Events<crate::layer1::pop_memories::FamineEvent>>();
    world.init_resource::<Events<crate::layer1::diplomacy::wards::WarDeclaredEvent>>();
    world.init_resource::<Events<crate::layer1::execution::mining::harmonic_mining::TriggerSonicDrillEvent>>();
    if !world.contains_resource::<crate::layer1::diplomacy::wards::DiplomaticStanding>() {
        world.insert_resource(crate::layer1::diplomacy::wards::DiplomaticStanding {
            faction_relations: std::collections::HashMap::new(),
        });
    }

    if !world.contains_resource::<crate::layer1::living_score::ColonyRenown>() {
        world.insert_resource(crate::layer1::living_score::ColonyRenown { score: 50.0 });
    }

    world.init_resource::<Events<LaunchEvent>>();
    world.init_resource::<Events<ShipDestroyedEvent>>();
    world.init_resource::<Events<DetectionEvent>>();
    world.init_resource::<Events<HostileSpawnEvent>>();
    world.init_resource::<DetectionRisk>();
    world.init_resource::<Events<crate::layer1::unrest::DenounceEvent>>();
    world.init_resource::<Events<crate::layer1::law::embassy::ArrestEvent>>();
    world.init_resource::<Events<crate::layer1::law::embassy::DiplomaticIncidentEvent>>();
    world.init_resource::<Events<crate::layer1::environment::volatile::ExplosionEvent>>();

    if !world
        .contains_resource::<Events<crate::layer1::administration::edicts::TogglePolicyEvent>>()
    {
        world.init_resource::<Events<crate::layer1::administration::edicts::TogglePolicyEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::AccessDeniedEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::HackCentralHubEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::RevokePolicyEvent>>();
    }
    world.init_resource::<Events<crate::layer1::geology::tectonic::MegaQuakeEvent>>();
    world.init_resource::<Events<crate::layer1::geology::tectonic::ReliefQuakeEvent>>();
    world.init_resource::<Events<crate::layer1::whispering_ore::MinedOreEvent>>();
    world.init_resource::<Events<crate::layer1::whispering_ore::MineSealedEvent>>();
    world.init_resource::<Events<crate::layer1::social::grievances::PostGrievanceEvent>>();
    world.init_resource::<Events<crate::layer1::deep_crust_resonance::ExcavationEvent>>();
    world.init_resource::<Events<crate::layer1::logistics::mycelial::ContaminationEvent>>();
    world.init_resource::<Events<crate::layer1::resources::MiningEvent>>();
    world.init_resource::<Events<crate::layer1::spiteful_will::InheritanceEvent>>();
    if !world
        .contains_resource::<Events<crate::layer1::nature::biosphere_empathy::FloraDamagedEvent>>()
    {
        world
            .init_resource::<Events<crate::layer1::nature::biosphere_empathy::FloraDamagedEvent>>();
        world.init_resource::<Events<crate::layer1::blackout_bazaars::PlayerTradeEvent>>();
        world.init_resource::<Events<crate::layer1::nature::long_night::StartLongNightEvent>>();
    }
    world.init_resource::<crate::layer1::nature::biosphere_empathy::GlobalFloraHealth>();
    world.init_resource::<Events<crate::layer1::nature::long_night::StartLongNightEvent>>();
    world.init_resource::<crate::layer1::nature::long_night::LongNightEvent>();
    world.init_resource::<Events<crate::layer1::spiteful_will::OverrideWillEvent>>();
    world.init_resource::<crate::layer1::geology::tectonic::TectonicStress>();

    world.init_resource::<crate::layer1::unrest::Unrest>();
    world.init_resource::<crate::layer1::atmosphere::CorrosiveAtmosphere>();

    // Initialize Thermal Bloom Resource
    world.init_resource::<crate::layer2::thermal::ThermalSignature>();

    // Initialize Detection Risk
    world.init_resource::<DetectionRisk>();

    world.init_resource::<crate::layer2::phantom::EmpireAutomationState>();
    world.init_resource::<Events<crate::layer2::trade::blockade::TradeShipArrivalEvent>>();
    world.init_resource::<Events<crate::layer2::trade::routes::SentientTollDemandEvent>>();
    world.init_resource::<Events<crate::layer2::trade::routes::TradeRouteExecutedEvent>>();
    world.init_resource::<Events<crate::layer2::auction::BlindAuctionTriggeredEvent>>();
    world.init_resource::<Events<crate::layer2::auction::PlaceBidEvent>>();
    world.init_resource::<Events<crate::layer2::auction::VaultOpenedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::orbit::asteroid_claims::AttackColonyEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
    world.init_resource::<Events<crate::layer2::auction::TemporalAnomalyEvent>>();
    if !world.contains_resource::<Events<crate::layer2::trade::escape_velocity::LaunchShipEvent>>()
    {
        world.init_resource::<Events<crate::layer2::trade::escape_velocity::LaunchShipEvent>>();
    }
    world.init_resource::<Events<crate::layer3::events::debt_prison::BailoutOfferEvent>>();
    if !world.contains_resource::<Events<crate::layer3::events::debt_prison::AcceptBailoutEvent>>()
    {
        world.init_resource::<Events<crate::layer3::events::debt_prison::AcceptBailoutEvent>>();
    }
    if !world.contains_resource::<Events<crate::layer3::events::refugee_waves::RefugeeWaveEvent>>()
    {
        world.init_resource::<Events<crate::layer3::events::refugee_waves::RefugeeWaveEvent>>();
    }
    world.init_resource::<crate::layer3::diplomacy::galactic_games::FactionInfluences>();
    world.init_resource::<crate::layer3::diplomacy::galactic_games::GalacticGamesEvent>();
    world.init_resource::<crate::layer2::trade::blockade::ColonyDebt>();

    world.init_resource::<Events<crate::layer2::phantom::SpawnGhostFleetEvent>>();
    world.init_resource::<Events<crate::layer2::silent_mutiny::SensorGlitchEvent>>();

    if !world
        .contains_resource::<Events<crate::layer1::nanite_fabrication::ContainmentBreachEvent>>()
    {
        world.init_resource::<Events<crate::layer1::nanite_fabrication::ContainmentBreachEvent>>();
    }

    world.init_resource::<Events<crate::layer1::social::sub_lithic::SabotageEvent>>();
    world.init_resource::<Events<crate::layer1::cryo_prison::SabotageEvent>>();

    if !world
        .contains_resource::<Events<crate::layer2::trade::penal_contracts::PrisonerDiedEvent>>()
    {
        world.init_resource::<Events<crate::layer1::genetics::GeneSplicingEvent>>();
        world.init_resource::<Events<crate::layer1::genetics::GeneSplicingResultEvent>>();
        world.init_resource::<Events<crate::layer2::trade::penal_contracts::PrisonerDiedEvent>>();
    }

    world.init_resource::<Events<crate::layer2::governance::RebellionEvent>>();

    world.init_resource::<Events<crate::layer1::environment::disasters::DisasterEvent>>();

    world.init_resource::<Events<crate::layer2::tourism::disaster_tourism::GriefTouristArrivalEvent>>();
    world.init_resource::<Events<crate::layer2::station::ShipConstructionCompletedEvent>>();

    if !world.contains_resource::<Events<crate::layer1::agony_extract::HarvestAgonyExtractEvent>>()
    {
        world.init_resource::<Events<crate::layer1::agony_extract::HarvestAgonyExtractEvent>>();
        world.init_resource::<crate::layer1::agony_extract::AgonyExtractConfig>();
    }

    world.init_resource::<Events<crate::layer2::moon_hermits::PopDesertedEvent>>();
    world.init_resource::<Events<crate::layer2::ftl::wakes::FtlJumpEvent>>();
    world.init_resource::<Events<crate::layer2::ftl::wakes::SubspaceWakeEvent>>();
    world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
    world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();

    world.init_resource::<Events<crate::layer1::environment::disasters::DisasterEvent>>();

    world.init_resource::<Events<crate::layer2::trade::biomass_tariff::TradeDeal>>();
    world
        .init_resource::<Events<crate::layer2::trade::feral_logistics::FeralDeliveryTriggerEvent>>(
        );
    world.init_resource::<crate::layer2::trade::feral_logistics::FeralLogisticsNetwork>();
    world.init_resource::<Events<crate::layer1::geodetic::GolemFormedEvent>>();

    world.init_resource::<crate::layer3::market::GalacticMarket>();

    world
        .init_resource::<Events<crate::layer2::events_new::reverse_quarantine::RefugeeFleetEvent>>(
        );
    world.init_resource::<Events<crate::layer1::grafting::GraftBuildingEvent>>();
    world.init_resource::<Events<crate::layer2::refugees::fleet_arrival::RefugeeArrivalEvent>>();

    world.init_resource::<Events<crate::layer3::fleets::ColonyFoundedEvent>>();
    world.init_resource::<crate::layer1::diplomacy::factions::rivals::TerritoryGrid>();
    world.init_resource::<Events<crate::layer3::planet::black_market_terraforming::RogueTerraformEvent>>();
    world.init_resource::<Events<crate::layer3::market::quantum_famine::MarketPanicEvent>>();
    world.init_resource::<Events<crate::layer3::market::quantum_famine::ExportDumpEvent>>();
    world.init_resource::<Events<crate::layer2::exploration::void_whispers::FleetReturnedEvent>>();
    world.init_resource::<Events<crate::layer1::core::integration::PirateAmnestyEvent>>();
    world.init_resource::<Events<crate::layer1::economy::resources::ResourceMinedEvent>>();
    world.init_resource::<crate::layer3::pirates::PirateThreatLevel>();
    world.init_resource::<crate::layer3::pirates::ResourceCurseSettings>();
    world.init_resource::<Events<crate::layer1::economy::resources::ResourceMinedEvent>>();
    world.init_resource::<crate::layer3::pirates::PirateThreatLevel>();
    world.init_resource::<crate::layer3::pirates::ResourceCurseSettings>();
    if !world
        .contains_resource::<Events<crate::layer2::navigation::stellar_weather::FleetDamagedEvent>>(
        )
    {
        world
            .init_resource::<Events<crate::layer2::navigation::stellar_weather::FleetDamagedEvent>>(
            );
    }

    world.init_resource::<Events<crate::layer1::environment::ignition::SparkEvent>>();
    world.init_resource::<Events<crate::layer1::environment::ignition::ExplosionEvent>>();

    world.init_resource::<Events<crate::layer1::environment::events::DebrisFallEvent>>();
    world.init_resource::<Events<crate::layer1::social::scrap_code_prophets::CultFormationEvent>>();

    world.init_resource::<crate::layer3::intellectual_property_wars::PatentRegistry>();

    world.init_resource::<Events<crate::layer3::intellectual_property_wars::TechDiscoveredEvent>>();

    world
        .init_resource::<Events<crate::layer3::intellectual_property_wars::EspionageSuccessEvent>>(
        );
    world.init_resource::<Events<crate::layer1::law::penal::OrganHarvestedEvent>>();
    world.init_resource::<crate::layer1::law::penal::ColonyInventory>();
    world.init_resource::<crate::layer3::council::GalacticCouncil>();
    world.init_resource::<crate::layer2::syzygy::SyzygyCycle>();
    world.init_resource::<crate::layer2::syzygy::PlanetaryGravity>();
    world.init_resource::<crate::layer2::syzygy::TidalForce>();

    // Digital Detritus
    world.init_resource::<crate::layer3::digital_detritus::DataMiningQueue>();
    world.init_resource::<crate::layer3::digital_detritus::DiscoveredTechs>();
    world.init_resource::<Events<crate::layer3::digital_detritus::VirusEvent>>();
    world.init_resource::<Events<crate::layer1::architecture::living_architecture::PopConsumedEvent>>();
    world.init_resource::<Events<crate::layer1::architecture::embezzlement::EmbezzlementEvent>>();
    world.init_resource::<crate::layer3::digital_detritus::JunkDataFilter>();

    world.init_resource::<crate::layer1::mind::sleep_debt::SleepDebtConfig>();
    world.init_resource::<Events<crate::layer1::mind::sleep_debt::RepoManArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::logistics::mass_driver::LaunchEvent>>();
    world.init_resource::<Events<crate::layer1::logistics::mass_driver::BombardmentEvent>>();
    world.init_resource::<Events<crate::layer2::bombardment::BombardmentEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::flesh_tax::FleshTaxPaymentEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::flesh_tax::FleshTaxFailedEvent>>();

    world.init_resource::<crate::layer3::council::GalacticCouncil>();

    world.init_resource::<crate::layer2::syzygy::SyzygyCycle>();
    world.init_resource::<crate::layer2::syzygy::PlanetaryGravity>();
    world.init_resource::<crate::layer2::syzygy::TidalForce>();
    // Initialize Infinite Archive Resource (Spec 248)
    world.init_resource::<crate::layer1::tech::infinite_archive::Archive>();

    world.init_resource::<Events<HostileSpawnEvent>>();

    world.init_resource::<crate::layer3::map::MapData>();
    world.init_resource::<Events<crate::layer3::map::FleetArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::diplomacy::TributeDemandEvent>>();
    world.init_resource::<Events<crate::layer3::galaxy::FleetTravelEvent>>();
    world.init_resource::<Events<crate::layer3::map::AnomalyDiscoveredEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy_reflection::EntityKilledEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy_reflection::FloraPlantedEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy_reflection::TraitChangedEvent>>();
    world.init_resource::<Events<crate::layer2::cascade::LogisticsStrainedEvent>>();
    world.init_resource::<crate::layer3::physics::relativity::SimulationTime>();
    world.init_resource::<Events<crate::layer1::void_sirens::SirenSignalEvent>>();
    world.init_resource::<crate::layer1::void_sirens::SirenConfig>();
    world.init_resource::<Events<crate::layer2::cascade::DefenseWeakenedEvent>>();
    // Add our schedule if not yet added
    {
        let schedules = world.resource::<Schedules>();
        if schedules.get(SimulationSchedule).is_none() {
            world.init_resource::<Events<crate::layer3::ghost_ships::EvaluateTransitEvent>>();
            world
                .init_resource::<Events<crate::layer3::ghost_ships::EvaluateLostShipReturnEvent>>();
            world.init_resource::<Events<crate::layer1::unseen_bureaucracy::PhantomShiftEvent>>();
            world.init_resource::<Events<crate::layer3::diplomacy::cultural_ransom::RaidEvent>>();
            world.init_resource::<Events<crate::layer2::planetary_spin_up::PlanetaryTorqueEvent>>();
            world.insert_resource(
                crate::layer1::environment::terminator_habitats::TerminatorLine {
                    x_coordinate: 50.0,
                },
            );
            world.insert_resource(
                crate::layer1::environment::terminator_habitats::LibrationCycle {
                    current_tick: 0.0,
                    amplitude: 5.0,
                    speed: 0.1,
                },
            );

            world.init_resource::<Events<crate::layer3::diplomacy::cultural_ransom::DiplomaticNegotiationEvent>>();
            world.init_resource::<crate::layer3::linguistic_drift::LinguisticNetwork>();
            world.init_resource::<Events<crate::layer3::linguistic_drift::CulturalSyncEvent>>();
            world.init_resource::<Events<crate::layer3::linguistic_drift::TradeEvent>>();
            world.init_resource::<Events<crate::layer1::logistics::beanstalk::BeanstalkEvent>>();
            world.init_resource::<Events<crate::layer3::treaty_cruisers::InspectionEvent>>();
            world.init_resource::<crate::layer3::treaty_cruisers::ActiveTreaties>();

            let mut schedule = build_simulation_schedule();
            schedule.add_systems(
                (
                    crate::layer1::environment::atmosphere::produce_smog_system,
                    crate::layer1::environment::atmosphere::absorb_smog_system,
                    crate::layer1::environment::atmosphere::apply_smog_effects_system,
                )
                    .chain(),
            );
            schedule.add_systems((
                crate::layer1::archaeological_contagion::archaeological_infection_system,
                crate::layer1::core::integration::archaeological_contagion_chronicle_bridge,
                crate::layer1::core::integration::orphaned_swarm_chronicle_bridge,
                crate::layer1::core::integration::cultural_vandalism_chronicle_bridge,
                crate::layer1::core::integration::invasive_xeno_aesthetics_chronicle_bridge,
                crate::layer1::archaeological_contagion::ancient_routine_observation_system,
                crate::layer1::archaeological_contagion::evaluate_ancient_routine,
            ));
            schedule.add_systems((
                (
                    crate::layer2::planetary_spin_up::apply_planetary_torque_system,
                    crate::layer2::planetary_spin_up::calculate_effective_gravity_system,
                    crate::layer2::planetary_spin_up::trigger_coriolis_weather_system,
                )
                    .chain(),
                crate::layer2::signature::consume_spoofing_energy,
                crate::layer2::signature::apply_signature_spoofing
                    .after(crate::layer2::signature::consume_spoofing_energy),
                crate::layer1::anomalies::void_sirens::apply_siren_obsession,
                crate::layer1::anomalies::void_sirens::handle_obsessed_jobs,
                crate::layer1::logistics::beanstalk::beanstalk_morale_system,
                crate::layer1::logistics::beanstalk::beanstalk_collapse_system,
            ));
            world.add_schedule(schedule);
        }
    }
}

fn init_simulation_resources_more(world: &mut World) {
    if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>() {
        world.init_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>();
    }
    if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>() {
        world.init_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>();
            }
    if !world
        .contains_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>()
    {
        world.init_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>();
    }
    if !world.contains_resource::<crate::layer1::nature::atmosphere::SmogGrid>() {
        world.init_resource::<crate::layer1::nature::atmosphere::SmogGrid>();
    }
}

pub fn run_simulation_tick(world: &mut World) {
    init_simulation_resources(world);
    init_simulation_resources_more(world);
    world.run_schedule(SimulationSchedule);
    world.resource_mut::<SimulationTime>().tick += 1;
    world
        .resource_mut::<crate::layer3::physics::relativity::SimulationTime>()
        .tick += 1;
}

fn register_simulation_core_systems(schedule: &mut Schedule) {
    schedule.add_systems((
        crate::layer3::diplomacy::open_source_science::process_publication_system,
        crate::layer3::diplomacy::open_source_science::process_enemy_exploits_system,
    ));

    // --- Register Core Layer 1 Systems ---

    schedule.add_systems((
        crate::cross_layer::interplanetary_pollination::spore_escape_system,
        crate::cross_layer::interplanetary_pollination::spore_infection_system,
        crate::layer2::integration::interplanetary_pollination_chronicle_bridge,
        crate::layer3::tourism_integration::weaponized_tourism_chronicle_bridge,
    ));

    register_layer1_systems(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::engine_cultist_rituals::register(schedule);

    // Register orphaned swarm systems standalone here since we don't use App
    schedule.add_systems((
        crate::layer1::orphaned_swarm::check_for_derelict_arrival,
        crate::layer1::orphaned_swarm::apply_swarm_efficiency_boost,
        crate::layer1::orphaned_swarm::increase_swarm_corruption,
        crate::layer1::orphaned_swarm::trigger_swarm_hostility,
        crate::layer1::orphaned_swarm::apply_swarm_damage,
    ));

    // Black Market Terraforming

    schedule.add_systems(
        (
            crate::layer1::deep_crust_resonance::resonant_ore_exposure_system,
            crate::layer1::deep_crust_resonance::resonance_social_spread_system,
            crate::layer1::core::integration::deep_crust_resonance_chronicle_bridge,
            crate::layer1::core::integration::truth_outbreak_chronicle_bridge,
            crate::layer1::core::integration::thermal_damage_chronicle_bridge,
            crate::layer1::core::integration::siren_signal_chronicle_bridge,
        )
            .in_set(Layer1SystemSet::Economy),
    );

    schedule.add_systems(
        (
            crate::layer1::law::embassy::evaluate_diplomatic_crime_system,
            crate::layer1::psychology::memory_blackout::process_memory_blackout,
            crate::layer1::core::integration::memory_blackout_chronicle_bridge,
            crate::layer1::law::embassy::process_diplomatic_arrest_system,
            crate::layer1::diplomacy::factions::rivals::rival_colony_expansion_system,
            crate::layer1::diplomacy::factions::rivals::rival_resource_drain_system,
        )
            .chain()
            .in_set(Layer1SystemSet::Economy),
    );

    schedule.add_systems(crate::layer1::physics::harpoon::process_harpoon_impact_system);

    schedule.add_systems((
        crate::layer2::memorial_fleet::generate_tragedy_scrap_system,
        crate::layer2::memorial_fleet::apply_memorial_aura_system,
        crate::layer2::memorial_fleet::handle_memorial_ship_destruction_system,
        crate::layer2::memorial_fleet::decay_shattered_legacy_system,
    ));

    schedule.add_systems((crate::layer2::weather::weather_movement_system,));
    schedule.add_systems((
        crate::layer1::nature::long_night::start_long_night,
        crate::layer1::nature::long_night::process_long_night_effects
            .after(crate::layer1::nature::solar::update_solar_output_system),
        crate::layer2::solar_sail_migration::check_sail_fleet_proximity,
        crate::layer2::solar_sail_migration::apply_solar_sail_effects
            .after(crate::layer1::nature::solar::update_solar_output_system),
    ));
    schedule.add_systems((
        crate::layer1::economy::apply_cultural_contraband_system,
        crate::layer3::planet::black_market_terraforming::trigger_rogue_terraforming,
        crate::layer3::planet::black_market_terraforming::apply_rogue_terraforming_events,
        crate::layer3::integration::black_market_terraforming_bridge,
        crate::layer2::station::process_megastructure_upkeep,
        crate::layer2::station::decommission_megastructure_system,
        crate::layer2::station::log_generous_gift_system,
    ));

    // Whispering Ore
    schedule.add_systems((
        crate::layer1::psionics::latent_awakening_system,
        crate::layer1::psionics::pyrokinesis_power_activation_system,
        crate::layer1::whispering_ore::process_whispering_ore_system,
        crate::layer1::whispering_ore::handle_mine_sealing_system,
        crate::layer2::trade::phantom_limb_logistics::phantom_limb_logistics_system,
        crate::layer2::trade::phantom_limb_logistics::intercept_phantom_drop_system,
        crate::layer2::integration::phantom_limb_chronicle_bridge,
        crate::layer2::integration::phantom_trade_routes_chronicle_bridge,
    ));
}

#[allow(clippy::too_many_lines)]
fn register_simulation_extended_systems(schedule: &mut Schedule) {
    schedule.add_systems(crate::layer2::integration::sub_light_arrival_chronicle_bridge_system);
    #[cfg(feature = "nova")]
    schedule.add_systems(
        crate::experimental::photosynthetic_nourishment::photosynthetic_nourishment_system,
    );
    #[cfg(feature = "nova")]
    schedule.add_systems(crate::experimental::landfill_rummaging::landfill_rummaging_system);
    schedule.add_systems((
        crate::layer1::blackout_bazaars::spawn_blackout_bazaars_system,
        crate::layer1::blackout_bazaars::despawn_blackout_bazaars_system,
        crate::layer1::blackout_bazaars::bazaar_trading_system,
    ));
    schedule.add_systems((
        crate::layer1::psychology::doomsday::apply_doomsday_panic_effects,
        crate::layer1::psychology::doomsday::resolve_doomsday_event,
        crate::layer1::psychology::doomsday::cleanup_nihilism_debuffs,
    ));

    schedule.add_systems((
        crate::layer1::economy::bio_loom::apply_bio_suit_armor,
        crate::layer1::economy::bio_loom::process_bio_suit_parasitism,
    ));
    schedule.add_systems(
        (
            crate::experimental::the_weight_of_silence::track_colony_isolation_system,
            crate::experimental::the_weight_of_silence::process_isolation_needs_system,
            crate::experimental::the_weight_of_silence::spawn_silence_cult_system,
            crate::layer3::integration::reset_isolation_on_trade_system,
            crate::layer3::integration::silence_cult_chronicle_bridge,
            crate::layer2::integration::attack_colony_chronicle_bridge,
        )
            .after(Layer1SystemSet::Economy),
    );

    schedule.add_systems((
        crate::layer3::economy::biological_stock_market::biological_stock_market_bridge,
        crate::layer3::integration::pirate_republic_diplomacy_bridge,
        crate::layer1::process_sponsorship_acceptance,
        crate::layer2::leviathans::process_void_leviathan_hunger,
        crate::layer1::enforce_sponsorship_requirements,
        crate::layer1::handle_repair_requests,
        crate::layer3::diplomacy::system_sovereignty::process_sovereignty_declaration,
        crate::layer1::agriculture::zero_g_flora::handle_depressurization,
        crate::layer1::nature::solar_flare_lottery::solar_flare_emp_system,
        crate::layer1::nature::solar_flare_lottery::solar_flare_radiation_system,
        crate::layer3::diplomacy::dead_internet::automated_diplomat_system,
        crate::layer3::integration::dead_internet_chronicle_bridge,
        crate::layer1::nature::solar_flare_lottery::spawn_flare_isotopes_system,
        crate::layer1::nature::solar_flare_lottery::decay_flare_isotopes_system,
        crate::layer1::core::integration::solar_flare_chronicle_bridge,
        crate::layer1::core::integration::bio_rhythmic_commute_eclipse_bridge,
        crate::layer1::core::integration::bio_rhythmic_commute_flare_bridge,
    ));

    schedule.add_systems((
        crate::layer2::events_new::system_quarantine::apply_quarantine_effects,
        crate::layer2::events_new::system_quarantine::handle_quarantine_decay,
    ));
    // --- Spec 622 ---
    schedule.add_systems((
        crate::layer1::biology::cybernetic_ascendancy::cybernetic_integration_system,
        crate::layer1::biology::cybernetic_ascendancy::update_colony_average_utility_system,
        crate::layer1::biology::cybernetic_ascendancy::cybernetic_mind_merge_system.after(
            crate::layer1::biology::cybernetic_ascendancy::update_colony_average_utility_system,
        ),
    ));
    // --- AI Decision Chain (GPU compute) ---
    schedule.add_systems((
        update_building_map_system,
        gpu_evaluate_actions.after(update_building_map_system),
        crate::layer1::visitor::visitor_behavior_system,
        crate::layer1::drone::evaluate_drone_actions_system.after(update_building_map_system),
        crate::layer1::social::secret_societies::secret_society_formation_system,
        crate::layer1::social::secret_societies::society_action_system,
        crate::layer1::architecture::chrono_vault::handle_seal_vault,
        crate::layer1::architecture::chrono_vault::process_vault_timers,
        crate::layer1::architecture::potemkin::calculate_intimidation_system,
        crate::layer1::architecture::potemkin::destroy_potemkin_building_system,
        crate::layer1::core::integration::accidental_terraforming_chronicle_bridge,
        crate::layer1::core::integration::biomass_dividend_chronicle_bridge,
        crate::layer1::core::integration::potemkin_chronicle_bridge,
        crate::layer1::memetics::memory_smugglers::process_memory_smuggling_system,
        crate::layer1::memetics::memory_smugglers::process_job_execution_system,
        update_action_timer_system
            .after(gpu_evaluate_actions)
            .before(Layer1SystemSet::Execution),
    ));

    // --- Layer 3 Integration ---
    schedule.add_systems((
        (
            crate::layer2::cartographers_curse::process_telemetry_sale,
            crate::layer2::integration::cartographers_curse_chronicle_bridge,
            crate::layer2::bombardment::execute_bombardment_system,
            crate::layer2::integration::orbital_bombardment_chronicle_bridge,
            crate::layer3::diplomacy::flesh_tax::process_flesh_tax_payment,
            crate::layer3::diplomacy::flesh_tax::process_flesh_tax_failure,
            crate::layer2::integration::orbital_mirror_chronicle_bridge,
            crate::layer2::integration::signal_latency_fleet_bridge,
        )
            .chain(),
        crate::layer2::cartographers_curse::apply_drop_pod_accuracy,
        update_detection_risk_system.after(Layer1SystemSet::Economy),
        check_hostile_spawn_system.after(update_detection_risk_system),
        crate::layer3::integration::the_silence_chronicle_bridge.after(check_hostile_spawn_system),
        crate::layer3::market::ephemeral_market::spawn_ephemeral_market_system,
        crate::layer3::market::ephemeral_market::process_market_despawn_system,
        crate::layer3::market::ephemeral_market::fulfill_market_trade_system,
        crate::layer3::council::enforce_resolutions_system,
        crate::layer3::ghost_ships::evaluate_transit_system,
        crate::layer3::ghost_ships::evaluate_lost_ship_return_system,
        crate::layer3::treaty_cruisers::compliance_check_system,
        crate::layer3::intellectual_property_wars::register_patents_system,
        crate::layer3::intellectual_property_wars::process_licensing_fees_system,
        crate::layer3::intellectual_property_wars::detect_ip_piracy_system,
        crate::layer3::integration::ip_piracy_diplomacy_bridge,
        crate::layer3::intellectual_property_wars::process_espionage_system,
        crate::layer3::market::phantom_tax::accumulate_phantom_tax_system,
        crate::layer3::market::phantom_tax::execute_hack_system,
    ));

    // --- Layer 2 Integration ---
    schedule.add_systems((
        crate::layer2::integration::inauguration_system,
        crate::layer2::integration::asteroid_crash_chronicle_bridge,
    ));
    schedule.add_systems((
        crate::layer2::orbital_necropolis::apply_necropolis_bonus,
        crate::layer2::orbital_necropolis::handle_necropolis_destruction,
    ));
    schedule.add_systems((
        // Cleanup events
        update_event_buffer::<crate::layer1::blackout_bazaars::PlayerTradeEvent>,
        update_event_buffer::<crate::layer1::administration::edicts::TogglePolicyEvent>,
        update_event_buffer::<crate::layer1::administration::edicts::AccessDeniedEvent>,
        update_event_buffer::<crate::layer1::administration::edicts::HackCentralHubEvent>,
        update_event_buffer::<crate::layer1::administration::edicts::RevokePolicyEvent>,
        update_event_buffer::<crate::layer1::nature::solar_flare_lottery::SolarFlareEvent>,
        update_event_buffer::<crate::layer1::nature::long_night::StartLongNightEvent>,
    ));
    schedule.add_systems((
        // Cleanup Layer 2 events
        update_event_buffer::<LaunchEvent>,
        update_event_buffer::<ShipDestroyedEvent>,
        update_event_buffer::<crate::layer2::orbital_necropolis::EntityDestroyedEvent>,
        update_event_buffer::<crate::layer2::dead_protocols::ViolationEvent>,
    ));
    schedule.add_systems((update_event_buffer::<DetectionEvent>,));
    schedule.add_systems((
        crate::layer1::tech::legacy_code::apply_latency_system,
        crate::layer1::tech::legacy_code::process_reformat_system,
    ));
    schedule.add_systems((
        crate::layer2::integration::predecessor_orbital_shield_bridge_system,
        crate::layer3::quarantine::enforce_quarantine_system,
        crate::layer2::fleet::fleet_order_system
            .after(crate::layer2::integration::predecessor_orbital_shield_bridge_system)
            .after(crate::layer3::quarantine::enforce_quarantine_system),
        crate::layer2::station::build_station_system,
        crate::layer2::station::process_drydock_construction_system,
        crate::layer2::dead_protocols::protocol_violation_system,
        crate::layer2::integration::dead_protocol_chronicle_bridge,
        crate::layer2::communications::signal_decay::calculate_signal_decay_system,
        crate::layer2::communications::radio_broadcasts::process_comms_broadcasts,
        crate::layer2::integration::radio_broadcasts_threat_bridge_system
            .after(crate::layer2::communications::radio_broadcasts::process_comms_broadcasts),
        crate::layer2::integration::signal_decay_chronicle_bridge
            .after(crate::layer2::communications::signal_decay::calculate_signal_decay_system),
        crate::layer2::integration::orbital_drydock_fleet_bridge_system
            .after(crate::layer2::station::process_drydock_construction_system),
        crate::layer2::station::zero_g_fermentation_system
            .after(crate::layer2::fleet::fleet_order_system)
            .after(crate::layer2::fleet::fleet_order_system),
        crate::layer2::derelict_stations::claim_station_system
            .after(crate::layer2::fleet::fleet_order_system),
        crate::layer2::gravitational_doldrums::doldrums_effects_system,
        crate::layer2::fleet::fleet_movement_system
            .after(crate::layer2::fleet::fleet_order_system)
            .after(crate::layer2::gravitational_doldrums::doldrums_effects_system),
        crate::layer2::phantom_signal::process_phantom_signal_evasion_system,
        crate::layer2::phantom_signal::apply_sensor_probes_system,
        crate::layer2::mutiny::decay_fleet_morale,
    ));
    schedule.add_systems((
        crate::layer2::phantom_signal::process_phantom_signal_evasion_system,
        crate::layer2::phantom_signal::apply_sensor_probes_system,
        crate::layer2::mutiny::evaluate_fleet_mutiny,
        crate::layer2::integration::primitive_retaliation_chronicle_bridge,
        crate::layer2::integration::assign_sensors_to_player_fleets_system
            .after(crate::layer2::fleet::fleet_movement_system),
        crate::layer2::sensor_ambiguity::resolve_sensors_system
            .after(crate::layer2::integration::assign_sensors_to_player_fleets_system),
        crate::layer2::integration::ensure_player_fleets_identified_system
            .after(crate::layer2::sensor_ambiguity::resolve_sensors_system),
        crate::layer1::integration::fleet_unload_system
            .after(crate::layer2::fleet::fleet_movement_system),
        crate::layer2::fleet::ensure_fleet_health_system,
        crate::layer2::blind_jump::blind_jump_system
            .after(crate::layer2::combat::fleet_combat_system),
        crate::layer2::combat::fleet_combat_system
            .after(crate::layer2::fleet::fleet_movement_system),
        crate::layer2::barnacles::barnacle_accumulation_system,
        // Debris Systems
        crate::layer2::orbit::kessler_gambit::trigger_kessler_gambit_system,
        crate::layer2::debris::debris_accumulation_system
            .after(crate::layer2::combat::fleet_combat_system),
        crate::layer2::debris::debris_attrition_system
            .after(crate::layer2::debris::debris_accumulation_system),
        crate::layer2::debris::debris_decay_system,
        // Propaganda Constellation Systems (spec 1380)
        crate::layer2::propaganda::update_constellation,
        crate::layer2::propaganda::apply_constellation_morale
            .after(crate::layer2::propaganda::update_constellation)
            .before(crate::layer1::social::morale::update_morale_cache_system),
        crate::layer2::propaganda::constellation_hack_system
            .after(crate::layer2::propaganda::update_constellation),
    ));
    // Origin spawn director (adventurer origins).
    // Registered in their own block: the tuple above is at the
    // IntoSystemConfigs arity limit.
    //
    // Replaces the eight unconditional `spawn_*_once` entries
    // (Mark, 2026-10-06): the director finalizes the run's 3-origin
    // roster once (chosen origin at tick 0, two more trickling in), then
    // fires due spawns. It carries the explicit before-edges the lawbound
    // and chronostalker spawns used to hold: their sabotage/crisis
    // bridges read origin state resources that only exist after a
    // spawn/tick inserts them, and unordered systems panicked on the
    // missing resource when the build order reshuffled.
    schedule.add_systems((
        crate::layer1::culture::origins::origin_spawn_director
            .before(crate::layer1::culture::lawbound::sabotage_lawbound_bridge)
            .before(
                crate::layer1::culture::chronostalker::crisis_chronostalker_bridge,
            ),
        crate::layer1::culture::sovereign::sovereign_tick,
        crate::layer1::culture::sovereign::sabotage_melancholy_bridge.after(
            crate::layer1::biology::symbiotic_insurgency::trigger_symbiont_sabotage_system,
        ),
        crate::layer1::culture::sovereign::apply_sovereign_morale
            .before(crate::layer1::social::morale::update_morale_cache_system),
    ));
    // The Corsair systems (adventurer origin: the raider skiff).
    // The tick holds the skiff's pressure envelope, so it must run before
    // the pressure damage system suffocates the tethered crew.
    // (Origin spawning is handled by the origin spawn director above.)
    schedule.add_systems((
        crate::layer1::culture::corsair::corsair_tick.before(
            crate::layer1::physics::pressure::pressure_damage_system,
        ),
    ));
    // The Planetary Governor systems (adventurer origin: the appointment seal).
    // Own block: the tuple above sits near the IntoSystemConfigs arity limit.
    // (Origin spawning is handled by the origin spawn director above.)
    schedule.add_systems((
        crate::layer1::culture::governor::governor_tick,
        crate::layer1::culture::governor::apply_governor_morale
            .before(crate::layer1::social::morale::update_morale_cache_system),
    ));
    // The Salvager systems (adventurer origin: the derelict hulk).
    // The tick holds the hulk's pressure envelope (sealed cabin + patched
    // breaches) and drops open breaches to vacuum, so it must run before
    // the pressure damage system.
    // (Origin spawning is handled by the origin spawn director above.)
    schedule.add_systems((
        crate::layer1::culture::salvager::salvager_tick.before(
            crate::layer1::physics::pressure::pressure_damage_system,
        ),
    ));
    // The Improbable Pilot systems (adventurer origin: the Longshot Drive).
    // The tick holds a pressure envelope around the shuttle, so it must
    // run before the pressure damage system.
    // (Origin spawning is handled by the origin spawn director above.)
    schedule.add_systems((
        crate::layer1::culture::improbable::pilot_tick.before(
            crate::layer1::physics::pressure::pressure_damage_system,
        ),
    ));
    // The Lawbound systems (adventurer origin: the Three Statutes).
    // The sabotage bridge reads SabotageEvent, so it runs after the
    // symbiont sabotage trigger (same pattern as the sovereign bridge).
    // The spawn-before-bridge edge now lives on the origin spawn
    // director (see the sovereign block above).
    schedule.add_systems((
        crate::layer1::culture::lawbound::lawbound_tick,
        crate::layer1::culture::lawbound::sabotage_lawbound_bridge.after(
            crate::layer1::biology::symbiotic_insurgency::trigger_symbiont_sabotage_system,
        ),
    ));
    // The Chronostalker systems (adventurer origin: the Moment-Wound).
    // The crisis bridge reads SabotageEvent, PirateRaidEvent, and
    // DepressurizationEvent, so it runs after the symbiont sabotage
    // trigger (same pattern as the lawbound bridge). Same explicit
    // spawn-before-bridge edge as above, now carried by the origin
    // spawn director.
    schedule.add_systems((
        crate::layer1::culture::chronostalker::chronostalker_tick,
        crate::layer1::culture::chronostalker::crisis_chronostalker_bridge.after(
            crate::layer1::biology::symbiotic_insurgency::trigger_symbiont_sabotage_system,
        ),
    ));
    // The Bloom-Touched systems (adventurer origin: the bloom scar).
    // The tick holds the scar's expansion + entry-mutation passes; it has
    // no ordering constraints beyond the other origin ticks.
    // (Origin spawning is handled by the origin spawn director above.)
    schedule.add_systems((crate::layer1::culture::bloomtouched::bloomtouched_tick,));
    schedule.add_systems((
        crate::layer2::orbit::debris_cult::evaluate_debris_cult_formation_system,
        crate::layer2::orbit::debris_cult::apply_debris_cult_morale_system
            .after(crate::layer2::debris::debris_attrition_system),
        crate::layer2::orbit::secession::evaluate_orbital_secession_system,
        crate::layer2::integration::orbital_secession_chronicle_bridge,
    ));

    schedule.add_systems((
        // Thermal Bloom Systems
        crate::layer2::thermal::update_thermal_bloom_system.after(Layer1SystemSet::Economy),
        crate::layer2::thermal::detection_risk_system
            .after(crate::layer2::thermal::update_thermal_bloom_system),
        crate::layer2::orbital_mirrors::orbital_mirror_focus_system,
        crate::layer2::integration::thermal_detection_handler_system
            .after(crate::layer2::thermal::detection_risk_system),
        crate::layer2::culture::founder_effect::found_colony_system,
        crate::layer2::culture::founder_effect::update_colony_culture_system,
    ));

    schedule.add_systems((
        crate::layer2::culture::cultural_drift::calculate_cultural_drift_system,
        crate::layer2::culture::cultural_drift::handle_independence_system,
        crate::layer2::integration::cultural_drift_independence_bridge
            .after(crate::layer2::culture::cultural_drift::handle_independence_system),
        crate::layer2::integration::founder_effect_bridge_system,
        crate::layer2::integration::escape_velocity_traits_bridge_system
            .before(crate::layer2::trade::escape_velocity::process_launch_system),
        crate::layer2::integration::celestial_cemeteries_trade_bridge_system
            .before(crate::layer2::trade::escape_velocity::process_launch_system),
        crate::layer2::integration::blind_auction_chronicle_bridge_system,
        crate::layer2::integration::refugee_arrival_bridge_system,
        crate::layer1::culture::celestial_cemeteries::process_corpses_system,
        crate::layer1::culture::celestial_cemeteries::calculate_launch_risk_system,
        crate::layer1::environment::thermal_camouflage::update_thermal_signatures,
        crate::layer1::environment::thermal_camouflage::predator_detection_system
            .after(crate::layer1::environment::thermal_camouflage::update_thermal_signatures),
        crate::layer1::culture::celestial_cemeteries::clear_cemetery_system,
        crate::layer1::culture::memorial_revolt::handle_pet_death_system,
        crate::layer1::culture::memorial_revolt::process_memorial_demand_system,
    ));

    schedule.add_systems((
        crate::layer2::mycelial_network::evaluate_ecological_damage_system,
        crate::layer2::integration::mycelial_network_immune_response_chronicle_bridge,
        crate::layer2::integration::observe_forge_crush_event,
        crate::layer2::integration::celestial_library_chronicle_bridge,
        crate::layer2::cascade::evaluate_system_logistics,
        crate::layer2::cascade::update_sector_defenses
            .after(crate::layer2::cascade::evaluate_system_logistics),
        crate::layer2::cascade::calculate_invasion_threat
            .after(crate::layer2::cascade::update_sector_defenses),
        crate::layer2::integration::logistics_strained_chronicle_bridge
            .after(crate::layer2::cascade::evaluate_system_logistics),
        crate::layer2::integration::defense_weakened_chronicle_bridge
            .after(crate::layer2::cascade::update_sector_defenses),
    ));

    schedule.add_systems((
        crate::layer2::syzygy::update_syzygy_cycle_system,
        crate::layer2::integration::gravity_debt_to_planetary_gravity_system
            .before(crate::layer2::syzygy::apply_syzygy_effects_system),
        crate::layer2::syzygy::apply_syzygy_effects_system
            .after(crate::layer2::syzygy::update_syzygy_cycle_system),
        crate::layer2::integration::astrological_beliefs_bridge_system
            .after(crate::layer2::syzygy::update_syzygy_cycle_system),
        crate::layer2::trade::escape_velocity::process_launch_system
            .after(crate::layer2::syzygy::apply_syzygy_effects_system),
        crate::layer2::skyhooks::process_skyhook_launch
            .after(crate::layer2::syzygy::apply_syzygy_effects_system),
        crate::layer2::visibility::update_visibility_system.after(Layer1SystemSet::Economy),
        crate::layer2::visibility::enforce_view_mode_system
            .after(crate::layer2::visibility::update_visibility_system),
    ));

    schedule.add_systems((
        crate::layer2::orphan_fleet::hack_orphan_fleet_system,
        crate::layer2::orphan_fleet::orphan_fleet_defection_check_system,
        crate::layer2::orphan_fleet::process_orphan_defection_system
            .after(crate::layer2::orphan_fleet::orphan_fleet_defection_check_system),
    ));

    schedule.add_systems((
        crate::layer2::silent_mutiny::check_silent_mutiny_system,
        crate::layer2::silent_mutiny::process_mutiny_effects_system
            .after(crate::layer2::silent_mutiny::check_silent_mutiny_system),
        crate::layer2::integration::sensor_glitch_chronicle_bridge_system
            .after(crate::layer2::silent_mutiny::process_mutiny_effects_system),
        crate::layer2::integration::pre_trade_route_sync_system
            .before(crate::layer2::trade::routes::execute_trade_routes_system),
        crate::layer2::trade::routes::execute_trade_routes_system,
        crate::layer3::integration::language_drift_trade_bridge
            .after(crate::layer2::trade::routes::execute_trade_routes_system),
        crate::layer2::trade::routes::increase_route_complexity_system,
        crate::layer2::trade::routes::check_sentient_route_system,
        crate::layer2::integration::sentient_route_chronicle_bridge
            .after(crate::layer2::trade::routes::check_sentient_route_system),
        crate::layer2::integration::ideological_contraband_route_bridge
            .after(crate::layer2::trade::routes::execute_trade_routes_system),
        crate::layer2::integration::post_trade_route_sync_system
            .after(crate::layer2::trade::routes::execute_trade_routes_system),
        crate::layer3::integration::trade_route_market_bridge_system
            .after(crate::layer2::trade::routes::execute_trade_routes_system),
    ));
    schedule.add_systems((
        crate::layer2::trade::biomass_tariff::process_biomass_tariff_system
            .after(crate::layer2::integration::post_trade_route_sync_system),
        crate::layer2::trade::feral_logistics::process_feral_logistics,
        crate::layer2::trade::penal_contracts::process_penal_contracts_system,
        crate::layer2::trade::penal_contracts::check_prisoner_status_system,
        crate::layer2::integration::penal_funds_to_resources_system,
        crate::layer2::integration::prisoner_death_chronicle_bridge_system,
        crate::layer2::trade::blockade::debt_blockade_system,
        crate::layer2::trade::blockade::blockade_interception_system
            .after(crate::layer2::trade::blockade::debt_blockade_system),
        crate::layer3::events::debt_prison::check_bailout_condition_system
            .after(crate::layer2::trade::blockade::blockade_interception_system),
        crate::layer3::events::debt_prison::process_bailout_acceptance_system
            .after(crate::layer3::events::debt_prison::check_bailout_condition_system),
        crate::layer3::market::update_market_prices_system,
        crate::layer3::diplomacy::diplomatic_negotiation_system,
        crate::layer3::diplomacy_reflection::aggregate_colony_stats,
        crate::layer3::diplomacy_reflection::update_diplomatic_traits
            .after(crate::layer3::diplomacy_reflection::aggregate_colony_stats),
        crate::layer3::diplomacy_reflection::apply_diplomatic_reactions
            .after(crate::layer3::diplomacy_reflection::update_diplomatic_traits),
    ));

    schedule.add_systems(
        (
            crate::layer3::diplomacy::succession::process_succession_system,
            crate::layer3::diplomacy::xenolinguistics::process_alien_responses_system,
            crate::layer3::diplomacy::cultural_ransom::process_artifact_raid_system,
            crate::layer3::diplomacy::cultural_ransom::apply_hostage_penalties_system,
            crate::layer3::diplomacy::cultural_ransom::handle_ransom_negotiation_system,
            crate::layer3::diplomacy::red_tape_defense::process_bureaucracy_delays,
            crate::layer3::integration::red_tape_chronicle_bridge,
            crate::layer3::integration::sovereign_armada_chronicle_bridge,
            crate::layer3::integration::dynastic_succession_chronicle_bridge,
            crate::layer3::integration::dynastic_crisis_chronicle_bridge,
            crate::layer3::diplomacy::fading_homeworld::update_core_world_decay_system,
            crate::layer3::diplomacy::fading_homeworld::generate_core_world_demand_system,
            crate::layer3::diplomacy::fading_homeworld::handle_core_world_demands_system,
            crate::layer3::diplomacy::endless_draft::draft_order_generation_system,
            crate::layer3::diplomacy::endless_draft::process_draft_compliance_system,
            crate::layer3::diplomacy::endless_draft::process_draft_refusal_system,
            crate::layer3::diplomacy::endless_draft::spawn_veteran_system,
            crate::layer3::integration::endless_draft_bridge_system,
        )
            .chain(),
    );

    // The Degraded God-Mind (spec 1381): bit-rot, monument upkeep, schism
    // crises, and the corrupted-memory war chronicle bridge.
    schedule.add_systems((
        crate::layer3::diplomacy::god_mind::god_mind_rot_system,
        crate::layer3::diplomacy::god_mind::god_mind_monument_upkeep_system,
        crate::layer3::diplomacy::god_mind::god_mind_schism_crisis_system,
        crate::layer3::integration::god_mind_war_chronicle_bridge,
    ));
    // Exclusive system: rolls edicts from &mut World each tick.
    schedule.add_systems(crate::layer3::diplomacy::god_mind::god_mind_edict_tick_system);

    schedule.add_systems((
        crate::layer3::map::stellar_drift_system,
        crate::layer3::map::hyperlane_maintenance_system
            .after(crate::layer3::map::stellar_drift_system),
        crate::layer3::map::hyperlane_formation_system
            .after(crate::layer3::map::hyperlane_maintenance_system),
        crate::layer3::map::trigger_hyperlane_collapse_system,
        crate::layer3::map::process_hyperlane_collapse_system
            .after(crate::layer3::map::trigger_hyperlane_collapse_system)
            .after(crate::layer3::map::hyperlane_maintenance_system),
        crate::layer3::map::recalculate_trade_routes_system
            .after(crate::layer3::map::process_hyperlane_collapse_system),
        crate::layer3::integration::hyperlane_collapse_chronicle_bridge,
        crate::layer3::integration::anomaly_discovered_chronicle_bridge
            .after(crate::layer3::map::process_hyperlane_collapse_system),
        crate::layer3::integration::diplomatic_fashion_chronicle_bridge,
        crate::layer3::integration::feral_logistics_chronicle_bridge,
        crate::layer3::integration::stranded_fleet_chronicle_bridge,
        crate::layer3::diplomacy::wormhole_dumping::process_dump_waste,
        crate::layer3::diplomacy::retro_contracts::handle_retro_contract_acceptance,
        crate::layer3::diplomacy::retro_contracts::evaluate_retro_contracts,
        crate::layer3::integration::retro_contract_accepted_bridge_system,
        crate::layer3::integration::retro_contract_failed_bridge_system,
    ));

    schedule.add_systems((crate::layer3::events::refugee_waves::process_refugee_decision,));
    schedule.add_systems((
        crate::layer3::economy::market_shock::monitor_luxury_production_system,
        crate::layer3::economy::market_shock::trigger_ally_civil_war_system,
    ));
    schedule.add_systems((
        crate::layer3::diplomacy::galactic_games::resolve_galactic_games_system,
        crate::layer3::integration::galactic_games_chronicle_bridge
            .after(crate::layer3::diplomacy::galactic_games::resolve_galactic_games_system),
    ));

    schedule.add_systems((
        crate::layer2::phantom::check_scrapcode_threshold_system
            .after(crate::layer1::scrapcode::scrapcode_decay_system),
        crate::layer2::phantom::spawn_ghost_fleet_system
            .after(crate::layer2::phantom::check_scrapcode_threshold_system),
    ));

    schedule.add_systems((
        crate::layer2::governance::apply_governor_effects_system,
        crate::layer2::governance::update_governor_ambition_system
            .after(crate::layer2::governance::apply_governor_effects_system),
        crate::layer2::governance::check_governor_rebellion_system
            .after(crate::layer2::governance::update_governor_ambition_system),
        crate::layer2::integration::rebellion_chronicle_bridge_system
            .after(crate::layer2::governance::check_governor_rebellion_system),
        crate::layer2::tourism::process_disaster_tourism_system.after(Layer1SystemSet::Execution),
        crate::layer2::stolen_fleet::process_defection_events,
        crate::layer2::stolen_fleet::process_fleet_upkeep,
        crate::layer2::integration::stolen_fleet_chronicle_bridge_system
            .after(crate::layer2::stolen_fleet::process_defection_events),
        crate::layer2::rogue_planets::rogue_planet_drift_system,
        crate::layer2::integration::process_grief_tourist_arrival_system
            .after(crate::layer2::tourism::process_disaster_tourism_system),
        crate::layer2::navigation::stellar_weather::apply_stellar_weather_effects,
        crate::layer2::integration::stellar_weather_damage_bridge_system
            .after(crate::layer2::navigation::stellar_weather::apply_stellar_weather_effects),
        crate::layer2::events_new::reverse_quarantine::process_refugee_decisions_system,
        crate::layer2::integration::reverse_quarantine_chronicle_bridge
            .after(crate::layer2::events_new::reverse_quarantine::process_refugee_decisions_system),
        crate::layer2::moon_hermits::process_hermit_desertions,
        crate::layer2::integration::moon_hermits_chronicle_bridge_system
            .after(crate::layer2::moon_hermits::process_hermit_desertions),
        crate::layer2::moon_hermits::hermit_theft_system,
    ));

    schedule.add_systems((
        crate::layer2::asteroid_hermits::evaluate_hermit_exodus_system,
        crate::layer2::asteroid_hermits::process_hermit_discoveries_system,
        crate::layer2::empathic_plague::process_empathic_resonance
            .after(Layer1SystemSet::Observation),
        crate::layer3::pirates::evaluate_pirate_amnesty_system,
        crate::layer3::pirates::process_hyper_resources,
        crate::layer3::pirates::threat_cooldown_system,
    ));

    schedule.add_systems((
        crate::layer2::integration::asteroid_hermit_exodus_chronicle_bridge
            .after(crate::layer2::asteroid_hermits::evaluate_hermit_exodus_system),
        crate::layer2::integration::asteroid_hermit_discovery_chronicle_bridge
            .after(crate::layer2::asteroid_hermits::process_hermit_discoveries_system),
        crate::layer3::pirates::resource_curse_raid_bridge,
        crate::layer1::social::pirates::process_pirate_amnesty_system
            .after(crate::layer3::pirates::evaluate_pirate_amnesty_system),
        crate::layer1::social::pirates::pirate_crime_system,
    ));

    #[cfg(feature = "nova")]
    crate::experimental::tavern_brawls::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::grave_robbery::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::tectonic_prophets::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::feral_foraging::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::gloom_sickness::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::echo_chamber::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::the_haunted_cartographer::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::the_final_will::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::digital_seance::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::whispering_well::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::echoing_footsteps::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::panic_buying::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::sleepwalking_hazards::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::solar_flare_sickness::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::magnetic_lightning::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::supercharged_anomalies::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::cassandra_warning::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::weather_madness::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::engine_cultist_rituals::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::acoustic_hallucinations::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::hoarder_sleepwalking::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::storm_thieves::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::manic_cleaning::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::fungal_death::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::homesick_craving::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::privatized_healthcare::register(schedule);

    schedule.add_systems((
        crate::layer3::map::map_data_rot_system,
        crate::layer3::map::scout_ship_scan_system,
        crate::layer3::map::fleet_arrival_anomaly_system,
        crate::layer3::sovereign_armada::armada_arrival_system,
        crate::layer3::physics::relativity::process_time_dilation_system,
        crate::layer3::physics::relativity::update_fleet_local_time_system
            .after(crate::layer3::physics::relativity::process_time_dilation_system),
        crate::layer2::exploration::void_whispers::accumulate_void_whispers_in_deep_space,
        crate::layer2::exploration::void_whispers::spread_whispers_to_colony,
        crate::layer2::integration::void_whispers_chronicle_bridge,
        crate::layer2::integration::orbital_junkyard_bridge_system,
    ));

    schedule.add_systems((crate::layer3::integration::jump_risk_bridge_system
        .after(crate::layer3::stellar_cartography::handle_jump_risk_system),));

    schedule.add_systems((
        crate::layer2::ftl::wakes::generate_subspace_wake_system,
        crate::layer1::hazards::wakes::resolve_subspace_wakes_system,
        crate::layer1::core::integration::subspace_stowaways_chronicle_bridge,
    ));

    schedule.add_systems((
        crate::layer3::fleets::simulate_transit_drift_system,
        crate::layer3::fleets::apply_drift_on_foundation_system,
        crate::cross_layer::generation_ship_mutiny::generation_ship_radicalization_system,
        crate::cross_layer::generation_ship_mutiny::evaluate_mutiny_system,
        // 1064 Digital Detritus
        crate::layer3::digital_detritus::process_data_mining_system,
        crate::layer3::digital_detritus::record_virus_event_chronicle_system,
    ));

    schedule.add_systems((
        crate::layer1::culture::gastronomers::spawn_gastronomer_faction_system,
        crate::layer1::culture::gastronomers::apply_culinary_singularity_buff_system,
        crate::layer1::core::integration::gastronomer_chronicle_bridge,
        crate::layer1::core::integration::visitor_chronicle_bridge,
        crate::layer1::economy::apex_diet::apex_meat_harvest_bridge_system,
        crate::layer1::economy::apex_diet::apex_meat_distribution_system
            .before(crate::layer1::agriculture::farm::consume_food_system),
    ));

    schedule.add_systems((
        crate::layer3::bureaucracy_of_vanity::vanity_building_listener_system,
        crate::layer3::bureaucracy_of_vanity::vanity_sabotage_system,
        (
            crate::layer3::integration::bureaucracy_of_truth_integration_system,
            crate::layer3::bureaucracy_of_truth::generate_colony_reports_system,
        )
            .chain(),
        (
            crate::layer3::bureaucracy::colony_reporting_system,
            crate::layer3::bureaucracy::empire_resource_distribution_system,
            crate::layer3::integration::cargo_cult_chronicle_bridge,
            crate::layer3::integration::discover_ghost_town_system,
        )
            .chain(),
    ));
    #[cfg(feature = "nova")]
    crate::experimental::subconscious_computing::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::adblock_siphoning::register(schedule);
    #[cfg(feature = "nova")]
    crate::experimental::rainy_day_reading::register(schedule);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::setup_world;
    use crate::shared::state::GameState;
    use bevy::prelude::Time;

    #[test]
    fn test_run_simulation_tick_increments() {
        let mut world = setup_world();

        world.init_resource::<bevy::prelude::Events<
            crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent,
        >>();
        world.init_resource::<bevy::prelude::Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer2::dead_protocols::ViolationEvent>>();
        world
            .init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>(
            );
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>>();

        world.init_resource::<bevy::prelude::Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::infrastructure::ancient::ConduitSurgeEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::justice::CrimeCommittedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::social::grievances::PostGrievanceEvent>>();
        world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
        world.init_resource::<Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer1::agriculture::zero_g_flora::DepressurizationEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::DeclarationOfIndependenceEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::WarDeclarationEvent>>();
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(
            crate::layer3::diplomacy::system_sovereignty::FactionRelations::default(),
        );
        world.init_resource::<bevy::prelude::Events<crate::layer1::pop_memories::FamineEvent>>();
        world.init_resource::<crate::layer3::digital_detritus::DataMiningQueue>();
        world.init_resource::<crate::layer3::digital_detritus::DiscoveredTechs>();
        world.init_resource::<Events<crate::layer3::digital_detritus::VirusEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::living_architecture::PopConsumedEvent>>();
        world
            .init_resource::<Events<crate::layer1::architecture::embezzlement::EmbezzlementEvent>>(
            );
        world.init_resource::<crate::layer3::digital_detritus::JunkDataFilter>();
        world.init_resource::<crate::layer1::culture::gastronomers::EmpireAdvancement>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::culture::gastronomers::CulinarySingularityEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::petrification::PopPetrifiedEvent>>();
        world.init_resource::<Events<crate::layer1::predecessors::WorldTriggerEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::chrono_vault::SealVaultEvent>>();
        world.init_resource::<Events<crate::layer2::auction::VaultOpenedEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer2::orbit::asteroid_claims::AttackColonyEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>(
            );
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>(
            );
        world.init_resource::<Events<crate::layer1::AcceptSponsorshipEvent>>();
        world.init_resource::<Events<crate::layer1::crafting::CraftEvent>>();
        world
            .init_resource::<Events<crate::layer1::biology::symbiotic_insurgency::SabotageEvent>>();
        world.insert_resource(crate::layer1::biology::symbiotic_insurgency::SymbiontFaction {
        members: 0,
        // INT-555: the trigger system escalates to critical (airlock) sabotage
        // at this size; the unit tests assume 40. A default of 0 made EVERY
        // sabotage event take the critical path.
        critical_mass: 40,
    });
        world.init_resource::<crate::layer1::biology::symbiotic_insurgency::InfectionConfig>();
        world.init_resource::<Events<crate::layer1::psychology::memory_blackout::MemoryBlackoutEvent>>();
        world.init_resource::<Events<crate::layer1::RepairBuildingEvent>>();
        world.init_resource::<Events<crate::layer1::social::hedonic_treadmill::ConsumeItemEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent>>();
        *world.resource_mut::<GameState>() = GameState::Running;

        let tick_before = world.resource::<SimulationTime>().tick;
        run_simulation_tick(&mut world);
        let tick_after = world.resource::<SimulationTime>().tick;

        assert_eq!(tick_after, tick_before + 1);
    }

    #[test]
    fn test_run_multiple_ticks() {
        let mut world = setup_world();

        world.init_resource::<bevy::prelude::Events<
            crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent,
        >>();
        world.init_resource::<bevy::prelude::Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world
            .init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>(
            );
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>>();

        world.init_resource::<Events<crate::layer2::celestial_library::LibraryDonationEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::justice::CrimeCommittedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::social::grievances::PostGrievanceEvent>>();
        world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
        world.init_resource::<Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer1::agriculture::zero_g_flora::DepressurizationEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::DeclarationOfIndependenceEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::WarDeclarationEvent>>();
        world.init_resource::<Events<crate::layer2::celestial_library::LibraryDonationEvent>>();
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(
            crate::layer3::diplomacy::system_sovereignty::FactionRelations::default(),
        );
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::pop_memories::FamineEvent>>();
        world.init_resource::<crate::layer3::digital_detritus::DataMiningQueue>();
        world.init_resource::<crate::layer3::digital_detritus::DiscoveredTechs>();
        world.init_resource::<Events<crate::layer3::digital_detritus::VirusEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::living_architecture::PopConsumedEvent>>();
        world
            .init_resource::<Events<crate::layer1::architecture::embezzlement::EmbezzlementEvent>>(
            );
        world.init_resource::<crate::layer3::digital_detritus::JunkDataFilter>();
        world.init_resource::<Events<crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent>>();
        *world.resource_mut::<GameState>() = GameState::Running;

        for _ in 0..10 {
            run_simulation_tick(&mut world);
        }

        assert_eq!(world.resource::<SimulationTime>().tick, 10);
    }

    #[test]
    fn test_schedule_builds_without_panic() {
        let _schedule = build_simulation_schedule();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn test_schedule_runs_on_fresh_world() {
        let mut world = setup_world();
        init_simulation_resources(&mut world);
        *world.resource_mut::<GameState>() = GameState::Running;
        world
            .init_resource::<crate::layer1::administration::invasive_bureaucracy::EmpireStability>(
            );
        #[cfg(feature = "nova")]
        world.init_resource::<crate::experimental::ghost_grid::GhostGrid>();

        // Initialize Detection Risk for test
        world.init_resource::<Events<crate::layer1::architecture::chrono_vault::SealVaultEvent>>();
        world.init_resource::<Events<crate::layer2::auction::VaultOpenedEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer2::orbit::asteroid_claims::AttackColonyEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>(
            );
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>(
            );
        world.init_resource::<Events<crate::layer1::AcceptSponsorshipEvent>>();
        world.init_resource::<Events<crate::layer1::crafting::CraftEvent>>();
        world
            .init_resource::<Events<crate::layer1::biology::symbiotic_insurgency::SabotageEvent>>();
        world.insert_resource(crate::layer1::biology::symbiotic_insurgency::SymbiontFaction {
        members: 0,
        // INT-555: the trigger system escalates to critical (airlock) sabotage
        // at this size; the unit tests assume 40. A default of 0 made EVERY
        // sabotage event take the critical path.
        critical_mass: 40,
    });
        world.init_resource::<crate::layer1::biology::symbiotic_insurgency::InfectionConfig>();
        world.init_resource::<Events<crate::layer1::psychology::memory_blackout::MemoryBlackoutEvent>>();
        world.init_resource::<Events<crate::layer1::RepairBuildingEvent>>();
        world.init_resource::<Events<crate::layer1::social::hedonic_treadmill::ConsumeItemEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::sunk_cost_monument::CancelConstructionEvent>>();
        world.init_resource::<Events<crate::layer1::anomalies::echo::SpawnEchoSourceEvent>>();
        world.init_resource::<Events<crate::layer1::anomalies::void_sirens::SirenSignalEvent>>();
        world.init_resource::<Events<crate::layer1::agriculture::pollination::GrowthCycleEvent>>();
        world
            .init_resource::<Events<crate::layer1::memetics::memetic_hazards::ConversationEvent>>();

        world.init_resource::<Events<crate::layer1::economy::black_market::SmugglerArrivalEvent>>();
        world
            .init_resource::<Events<crate::layer1::economy::black_market::ShutdownDropNodeEvent>>();
        world.init_resource::<Events<crate::layer1::economy::apex_diet::ConsumeFoodEvent>>();
        world.init_resource::<crate::layer1::economy::apex_diet::ApexMeatStores>();
        world
            .init_resource::<crate::layer1::biology::cybernetic_ascendancy::ColonyAverageUtility>();
        world.init_resource::<crate::layer3::silence::DetectionRisk>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        world
            .init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>(
            );
        world.init_resource::<bevy::prelude::Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<Events<crate::layer2::moon_hermits::PopDesertedEvent>>();
        world.init_resource::<Events<crate::layer2::ftl::wakes::FtlJumpEvent>>();
        world.init_resource::<Events<crate::layer2::ftl::wakes::SubspaceWakeEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::social::grievances::PostGrievanceEvent>>();
        world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
        world.init_resource::<Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer1::agriculture::zero_g_flora::DepressurizationEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::DeclarationOfIndependenceEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::WarDeclarationEvent>>();
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(
            crate::layer3::diplomacy::system_sovereignty::FactionRelations::default(),
        );
        world.init_resource::<Events<crate::layer1::law::embassy::ArrestEvent>>();
        world.init_resource::<Events<crate::layer1::law::embassy::DiplomaticIncidentEvent>>();
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>>();

        world.init_resource::<Events<crate::layer1::ransom_broker::RansomDemandEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::fading_homeworld::CoreWorldDemandEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::fading_homeworld::PlayerDemandResponse>>();
        world.init_resource::<Events<crate::layer1::ransom_broker::PayRansomEvent>>();
        world.init_resource::<Events<crate::layer1::ransom_broker::RefuseRansomEvent>>();
        world.init_resource::<Events<crate::layer1::ransom_broker::PopRansomedEvent>>();
        world.init_resource::<Events<crate::layer1::ransom_broker::PopLostToPiratesEvent>>();
        world.init_resource::<Events<crate::layer3::silence::HostileSpawnEvent>>();
        world.init_resource::<Events<crate::layer1::grafting::GraftBuildingEvent>>();
        world
            .init_resource::<Events<crate::layer2::refugees::fleet_arrival::RefugeeArrivalEvent>>();

        world.init_resource::<Events<crate::layer1::administration::edicts::TogglePolicyEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::AccessDeniedEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::HackCentralHubEvent>>();
        world.init_resource::<Events<crate::layer1::administration::edicts::RevokePolicyEvent>>();
        world.init_resource::<Events<crate::layer3::fleets::ColonyFoundedEvent>>();
        world.init_resource::<crate::layer1::diplomacy::factions::rivals::TerritoryGrid>();
        world.init_resource::<Events<crate::layer1::temporal_ghost_towns::TemporalStutterEvent>>();
        world.init_resource::<Events<crate::layer3::planet::black_market_terraforming::RogueTerraformEvent>>();
        world.init_resource::<Events<crate::layer3::market::quantum_famine::MarketPanicEvent>>();
        world.init_resource::<Events<crate::layer3::market::quantum_famine::ExportDumpEvent>>();
        if !world
            .contains_resource::<Events<crate::layer3::diplomacy_reflection::EntityKilledEvent>>()
        {
            world.init_resource::<Events<crate::layer3::diplomacy_reflection::EntityKilledEvent>>();
        }
        if !world
            .contains_resource::<Events<crate::layer3::diplomacy_reflection::FloraPlantedEvent>>()
        {
            world.init_resource::<Events<crate::layer3::diplomacy_reflection::FloraPlantedEvent>>();
        }
        if !world
            .contains_resource::<Events<crate::layer3::diplomacy_reflection::TraitChangedEvent>>()
        {
            world.init_resource::<Events<crate::layer3::diplomacy_reflection::TraitChangedEvent>>();
        }
        world.init_resource::<crate::layer2::cartographers_curse::MapTelemetry>();
        world.init_resource::<Events<crate::layer2::cartographers_curse::SellTelemetryEvent>>();

        world.init_resource::<Events<crate::layer1::spiteful_will::InheritanceEvent>>();
        world.init_resource::<Events<crate::layer1::spiteful_will::OverrideWillEvent>>();
        world
            .init_resource::<Events<crate::layer1::nature::biosphere_empathy::FloraDamagedEvent>>();
        world.init_resource::<Events<crate::layer1::nature::long_night::StartLongNightEvent>>();
        world.init_resource::<crate::layer2::phantom::EmpireAutomationState>();
        world.init_resource::<Events<crate::layer2::trade::blockade::TradeShipArrivalEvent>>();
        world.init_resource::<Events<crate::layer2::trade::routes::SentientTollDemandEvent>>();
        world.init_resource::<Events<crate::layer2::trade::routes::TradeRouteExecutedEvent>>();
        world.init_resource::<Events<crate::layer2::trade::escape_velocity::LaunchShipEvent>>();
        world.init_resource::<crate::layer2::trade::blockade::ColonyDebt>();

        world.init_resource::<Events<crate::layer3::events::debt_prison::BailoutOfferEvent>>();
        world.init_resource::<Events<crate::layer3::events::debt_prison::AcceptBailoutEvent>>();
        world.init_resource::<Events<crate::layer3::events::refugee_waves::RefugeeWaveEvent>>();
        world.init_resource::<crate::layer1::economy::smugglers_cove::ColonyAuthority>();

        world.init_resource::<Events<crate::layer1::diplomacy::wards::WarDeclaredEvent>>();
        if !world.contains_resource::<crate::layer1::diplomacy::wards::DiplomaticStanding>() {
            world.insert_resource(crate::layer1::diplomacy::wards::DiplomaticStanding {
                faction_relations: std::collections::HashMap::new(),
            });
        }

        world.init_resource::<Events<crate::layer2::phantom::SpawnGhostFleetEvent>>();

        world.init_resource::<Events<crate::layer2::silent_mutiny::SensorGlitchEvent>>();
        world.init_resource::<Events<crate::layer1::nanite_fabrication::ContainmentBreachEvent>>();
        world.init_resource::<Events<crate::layer1::social::sub_lithic::SabotageEvent>>();
        world.init_resource::<Events<crate::layer1::cryo_prison::SabotageEvent>>();
        world.init_resource::<Events<crate::layer1::genetics::GeneSplicingEvent>>();
        world.init_resource::<Events<crate::layer1::genetics::GeneSplicingResultEvent>>();
        world.init_resource::<Events<crate::layer2::trade::penal_contracts::PrisonerDiedEvent>>();
        world.init_resource::<Events<crate::layer2::governance::RebellionEvent>>();
        world.init_resource::<Events<crate::layer1::environment::disasters::DisasterEvent>>();
        world.init_resource::<Events<crate::layer2::tourism::disaster_tourism::GriefTouristArrivalEvent>>();
        world.init_resource::<Events<crate::layer1::culture::celestial_cemeteries::ClearCemeteryEvent>>();
        world.init_resource::<Events<crate::layer1::culture::memorial_revolt::PetDeathEvent>>();
        world.init_resource::<Events<crate::layer1::social::pets::PetDeathEvent>>();
        world.init_resource::<Events<crate::layer1::gravity_funerals::SunBurialRequestEvent>>();
        world.init_resource::<Events<crate::layer2::celestial_library::LibraryDonationEvent>>();
        world.init_resource::<Events<crate::layer2::station::ShipConstructionCompletedEvent>>();
        world.init_resource::<Events<crate::layer1::agony_extract::HarvestAgonyExtractEvent>>();
        world.init_resource::<crate::layer1::agony_extract::AgonyExtractConfig>();
        world.init_resource::<Events<crate::layer2::trade::biomass_tariff::TradeDeal>>();

        world.init_resource::<Events<crate::layer2::trade::routes::TradeRouteExecutedEvent>>();
        world.init_resource::<Events<crate::layer2::trade::feral_logistics::FeralDeliveryTriggerEvent>>();
        world.init_resource::<Events<crate::layer2::ship::logistics::StrandedEvent>>();

        world.init_resource::<crate::layer2::trade::feral_logistics::FeralLogisticsNetwork>();
        world.init_resource::<Events<crate::layer1::geodetic::GolemFormedEvent>>();
        world.init_resource::<crate::layer3::council::GalacticCouncil>();
        world.init_resource::<crate::layer3::market::GalacticMarket>();
        world.init_resource::<crate::layer1::stress::TraumaTracker>();
        // Spec 276 The Flesh Famine.
        world.init_resource::<crate::layer1::social::flesh_famine::FleshFamineState>();
        world.init_resource::<Events<crate::layer1::social::flesh_famine::PlantBlight>>();
        world.init_resource::<crate::layer1::economy::deep_sleep_syndicates::ProductionModifier>();
        world.init_resource::<Events<crate::layer1::economy::deep_sleep_syndicates::ThawSyndicateEvent>>();
        world.init_resource::<Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<Events<crate::layer1::tech::rogue_automation_cults::MachineCultFormedEvent>>();
        world.init_resource::<Events<crate::layer1::heirloom_tool::EquipHeirloomEvent>>();
        world.init_resource::<Events<crate::layer2::events_new::reverse_quarantine::RefugeeFleetEvent>>();

        world.init_resource::<Events<crate::layer1::predecessors::WorldTriggerEvent>>();
        world.init_resource::<Events<crate::layer1::grafting::GraftBuildingEvent>>();
        world
            .init_resource::<Events<crate::layer2::refugees::fleet_arrival::RefugeeArrivalEvent>>();

        world.init_resource::<Events<crate::layer3::fleets::ColonyFoundedEvent>>();
        world.init_resource::<crate::layer1::diplomacy::factions::rivals::TerritoryGrid>();
        world.init_resource::<Events<crate::layer3::planet::black_market_terraforming::RogueTerraformEvent>>();
        world.init_resource::<Events<crate::layer3::market::quantum_famine::MarketPanicEvent>>();
        world.init_resource::<Events<crate::layer3::market::quantum_famine::ExportDumpEvent>>();
        world
            .init_resource::<Events<crate::layer2::exploration::void_whispers::FleetReturnedEvent>>(
            );
        world.init_resource::<Events<crate::layer1::core::integration::PirateAmnestyEvent>>();
        world.init_resource::<Events<crate::layer1::economy::resources::ResourceMinedEvent>>();
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>(
            );
        world
            .init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>(
            );
        world.init_resource::<crate::layer3::pirates::PirateThreatLevel>();
        world.init_resource::<crate::layer3::pirates::ResourceCurseSettings>();
        world.init_resource::<Events<crate::layer1::law::justice::CrimeCommittedEvent>>();
        world.init_resource::<Events<crate::layer1::pop_memories::FamineEvent>>();
        world
            .init_resource::<Events<crate::layer2::navigation::stellar_weather::FleetDamagedEvent>>(
            );
        world.init_resource::<Events<crate::layer1::environment::ignition::SparkEvent>>();
        world.init_resource::<Events<crate::layer1::environment::ignition::ExplosionEvent>>();
        world.init_resource::<Events<crate::layer1::environment::events::DebrisFallEvent>>();
        world.init_resource::<Events<crate::layer1::shields::DamageEvent>>();

        world.init_resource::<crate::layer3::intellectual_property_wars::PatentRegistry>();

        world.init_resource::<Events<crate::layer3::intellectual_property_wars::TechDiscoveredEvent>>();

        world.init_resource::<Events<crate::layer3::intellectual_property_wars::EspionageSuccessEvent>>();
        world.init_resource::<Events<crate::layer1::law::penal::OrganHarvestedEvent>>();
        world.init_resource::<crate::layer1::law::penal::ColonyInventory>();
        world.init_resource::<Events<crate::layer1::whispering_ore::MinedOreEvent>>();
        world.init_resource::<Events<crate::layer1::whispering_ore::MineSealedEvent>>();
        world.init_resource::<Events<crate::layer1::deep_crust_resonance::ExcavationEvent>>();
        world.init_resource::<Events<crate::layer1::logistics::mycelial::ContaminationEvent>>();
        world.init_resource::<Events<crate::layer1::mycelial::MycelialTripwireEvent>>();
        world.init_resource::<crate::layer3::council::GalacticCouncil>();
        world.init_resource::<crate::layer2::syzygy::SyzygyCycle>();
        world.init_resource::<crate::layer2::syzygy::PlanetaryGravity>();
        world.init_resource::<crate::layer2::syzygy::TidalForce>();

        world.init_resource::<crate::layer3::map::MapData>();
        world.init_resource::<Events<crate::layer3::map::FleetArrivalEvent>>();
        world.init_resource::<Events<crate::layer3::map::AnomalyDiscoveredEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy_reflection::EntityKilledEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy_reflection::FloraPlantedEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy_reflection::TraitChangedEvent>>();

        world.init_resource::<crate::layer1::mind::sleep_debt::SleepDebtConfig>();
        world.init_resource::<Events<crate::layer1::mind::sleep_debt::RepoManArrivalEvent>>();
        world.init_resource::<Events<crate::layer1::logistics::mass_driver::LaunchEvent>>();
        world.init_resource::<Events<crate::layer1::logistics::mass_driver::BombardmentEvent>>();
        world.init_resource::<Events<crate::layer2::bombardment::BombardmentEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::flesh_tax::FleshTaxPaymentEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::flesh_tax::FleshTaxFailedEvent>>();
        world.init_resource::<Events<crate::layer2::bombardment::BombardmentEvent>>();

        world.init_resource::<Events<crate::layer2::cascade::LogisticsStrainedEvent>>();
        world.init_resource::<Events<crate::layer2::cascade::DefenseWeakenedEvent>>();

        world.init_resource::<crate::layer3::physics::relativity::SimulationTime>();
        world.init_resource::<Events<crate::layer1::void_sirens::SirenSignalEvent>>();
        world.init_resource::<crate::layer1::void_sirens::SirenConfig>();

        world.init_resource::<Events<crate::layer2::moon_hermits::PopDesertedEvent>>();
        world.init_resource::<Events<crate::layer2::ftl::wakes::FtlJumpEvent>>();
        world.init_resource::<Events<crate::layer2::ftl::wakes::SubspaceWakeEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::social::grievances::PostGrievanceEvent>>();
        world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
        world.init_resource::<Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer1::agriculture::zero_g_flora::DepressurizationEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::DeclarationOfIndependenceEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::system_sovereignty::WarDeclarationEvent>>();
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(
            crate::layer3::diplomacy::system_sovereignty::FactionRelations::default(),
        );

        world.init_resource::<Time>();
        world.init_resource::<Events<crate::layer2::primitives::InvasionEvent>>();
        world.init_resource::<Events<crate::layer2::orphan_fleet::HackOrphanFleetEvent>>();
        world.init_resource::<Events<crate::layer2::orphan_fleet::OrphanFleetDefectionEvent>>();

        world.init_resource::<Events<crate::layer1::social::ghost_shift_strike::GhostShiftStartedEvent>>();
        world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
        world.init_resource::<Events<
            crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent,
        >>();
        world.init_resource::<Events<crate::layer3::diplomacy::succession::SuccessionEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::xenolinguistics::MessageResponseEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::diplomatic_fashion::DiplomaticMeetingEvent>>();
        world.init_resource::<Events<crate::layer1::pop_memories::FamineEvent>>();
        world
            .init_resource::<Events<crate::layer3::diplomacy::succession::SuccessionCrisisEvent>>();

        world.init_resource::<Events<crate::layer3::ghost_ships::EvaluateTransitEvent>>();
        world.init_resource::<Events<crate::layer3::ghost_ships::EvaluateLostShipReturnEvent>>();
        world.init_resource::<Events<crate::layer1::unseen_bureaucracy::PhantomShiftEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::cultural_ransom::RaidEvent>>();
        world.init_resource::<Events<crate::layer2::planetary_spin_up::PlanetaryTorqueEvent>>();
        world.insert_resource(
            crate::layer1::environment::terminator_habitats::TerminatorLine { x_coordinate: 50.0 },
        );
        world.insert_resource(
            crate::layer1::environment::terminator_habitats::LibrationCycle {
                current_tick: 0.0,
                amplitude: 5.0,
                speed: 0.1,
            },
        );

        world.init_resource::<Events<crate::layer3::diplomacy::cultural_ransom::DiplomaticNegotiationEvent>>();
        world.init_resource::<crate::layer3::linguistic_drift::LinguisticNetwork>();
        world.init_resource::<Events<crate::layer3::linguistic_drift::CulturalSyncEvent>>();
        world.init_resource::<Events<crate::layer3::linguistic_drift::TradeEvent>>();
        world.init_resource::<Events<crate::layer1::logistics::beanstalk::BeanstalkEvent>>();
        world.init_resource::<Events<crate::layer3::treaty_cruisers::InspectionEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::retro_contracts::AcceptRetroContractEvent>>();
        world.init_resource::<Events<crate::layer3::diplomacy::retro_contracts::RetroContractFailedEvent>>();
        world.init_resource::<crate::layer3::treaty_cruisers::ActiveTreaties>();

        world.init_resource::<crate::layer3::digital_detritus::DataMiningQueue>();
        world.init_resource::<crate::layer3::digital_detritus::DiscoveredTechs>();
        world.init_resource::<Events<crate::layer3::digital_detritus::VirusEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::living_architecture::PopConsumedEvent>>();
        world
            .init_resource::<Events<crate::layer1::architecture::embezzlement::EmbezzlementEvent>>(
            );
        world.init_resource::<crate::layer3::digital_detritus::JunkDataFilter>();
        world.init_resource::<crate::layer1::culture::gastronomers::EmpireAdvancement>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::culture::gastronomers::CulinarySingularityEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::petrification::PopPetrifiedEvent>>();
        world.init_resource::<Events<crate::layer1::predecessors::WorldTriggerEvent>>();
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>>();

        world.init_resource::<Events<crate::layer2::communications::signal_latency::ExecuteOrderEvent>>();
        world.init_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>();
        world.init_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::physics::magnetic_reversal::PoleFlipEvent>>();
        world.init_resource::<bevy_ecs::event::Events<
            crate::layer2::trade::phantom_trade_routes::FixPhantomRouteEvent,
        >>();
        world.init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::InterceptDropEvent>>();
        world
            .init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::AuditRiskEvent>>(
            );
        world.init_resource::<crate::layer1::nature::atmosphere::SmogGrid>();
        world.init_resource::<Events<crate::layer1::heroic_acts::TriggerHeroicAct>>();
        world.init_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>();
        world.init_resource::<crate::layer1::nature::long_night::LongNightEvent>();
        world.init_resource::<crate::layer1::fungal_network::SporeNetwork>();
        world.init_resource::<crate::layer2::solar_sail_migration::SolarMigrationState>();

        let schedule = build_simulation_schedule();
        world.add_schedule(schedule);
        world.run_schedule(SimulationSchedule);

        // Should not panic — all systems run correctly on a fresh world
    }
}
