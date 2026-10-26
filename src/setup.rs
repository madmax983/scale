//! Shared world setup used by all entry points (native, headless, WASM).

use bevy_ecs::{prelude::*, system::RunSystemOnce};
use rand::{RngCore, SeedableRng};

// Fix: Unconditional import of AddChronicleEvent because init_resource usage is unconditional below
use crate::layer1::chronicle::AddChronicleEvent;

// use crate::layer1::chronicle::AddChronicleEvent; // Removed conditional import
use crate::layer1::heirloom::RetrogradeEngineeringEvent;
use crate::layer1::pop::{PopBorn, PopDied};
use crate::layer1::social::AffinityChange;
use crate::layer1::{
    generate_terrain, initial_chronicle_event, initial_naming_system, spawn_ancient_structures,
    spawn_initial_anomalies, spawn_initial_pops, AmbientLight, AtmosphereGrid, BuildMode,
    BuildingTracker, CameraCurrent, CameraTarget, Chronicle, ColonyPolicies, ColonyResources,
    DesignationMode, GlobalHitStop, LightMap, NamedLocations, NotificationQueue, OccupiedTiles,
    PopBundle, ScreenShake, SeasonState, TechState, TerrainType, UtilityConfig, Viewport,
};
use crate::shared::colony::ColonyName;
use crate::shared::keyboard::Input;
use crate::shared::log::MessageLog;
use crate::shared::narrative::NarrativeGenerator;
use crate::shared::state::GameState;
use crate::shared::time::{SimulationTime, WallTime};
use crate::ui::input::InputContextStack;
use crate::ui::map::RenderCache;
use crate::ui::selection::Selection;
use crate::ui::state::UiState;
use crate::ui::world_history::generate_world_history;

/// Ensures the Bevy task pools are initialized (required for `par_iter_mut`).
///
/// Safe to call multiple times — uses `get_or_init` internally.
pub fn init_task_pools() {
    bevy_tasks::ComputeTaskPool::get_or_init(bevy_tasks::TaskPool::default);
}

/// Configuration for world setup.
pub use crate::shared::scenario::*;

/// Create and initialize a new game world with all resources.
#[must_use]
pub fn setup_world() -> World {
    let mut world = World::new();
    world.init_resource::<crate::layer1::logistics::mycelial::MycelialNetwork>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::fleet::FleetCommand>>();
    world.init_resource::<crate::layer1::social::politics::ElectionCycle>();
    world.init_resource::<crate::layer1::social::politics::ActiveMandate>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::OpenRiftEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::cassandra_syndrome::DoomsdayWarningEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::cassandra_syndrome::DisasterOccurredEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::PayTemporalDebtEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::temporal_smuggling::ParadoxEvent>>();
    world.init_resource::<bevy_ecs::prelude::Events<crate::layer1::logistics::mycelial::ContaminationEvent>>();
    world.init_resource::<bevy::prelude::Events<
        crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent,
    >>();
    world.init_resource::<Events<crate::layer1::economy::black_market::SmugglerArrivalEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::ecology::HarvestEvent>>();
    world.init_resource::<Events<crate::layer1::economy::black_market::ShutdownDropNodeEvent>>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::DerelictArrivalEvent>>();
    world
        .init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::InterceptDropEvent>>(
        );
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::physics::magnetic_reversal::PoleFlipEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::trade::phantom_trade_routes::FixPhantomRouteEvent>>();
    world.init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::AuditRiskEvent>>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::SwarmArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::SwarmHostileEvent>>();
    world.init_resource::<Events<crate::layer2::events_new::orphaned_swarm::SwarmHostileEvent>>();
    // The sub-light arrival chronicle bridge system reads this event every
    // tick; without registration the first tick panics on the missing
    // Events<SubLightArrivalEvent> resource.
    world.init_resource::<Events<crate::layer2::sub_light_arrival::SubLightArrivalEvent>>();
    world.insert_resource(crate::layer3::economy::market_shock::MarketShockMarket {
        luxury_price: 10.0,
    });
    world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
        is_sovereign: false,
        overlord_id: Some(1),
    });
    world
        .insert_resource(crate::layer3::diplomacy::system_sovereignty::FactionRelations::default());
    setup_world_with_config(SetupConfig::default())
}

/// Create and initialize a new game world with custom configuration.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn setup_world_with_config(#[allow(unused_variables)] config: SetupConfig) -> World {
    init_task_pools();
    let mut world = World::new();
    // Spec 1382: save-system events + autosave state in every world.
    crate::persistence::init_save_system(&mut world);
    world.init_resource::<crate::layer1::nature::long_night::LongNightEvent>();
    world.init_resource::<crate::layer1::fungal_network::SporeNetwork>();
    world.init_resource::<Events<crate::layer1::economy::information_black_market::EarlyWarningEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::long_night::StartLongNightEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::tech::legacy_code::ReformatCommand>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::ecology::HarvestEvent>>();
    world.init_resource::<crate::layer1::culture::celestial_cemeteries::OrbitalCemetery>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::ecology::HarvestEvent>>();
    world
        .init_resource::<Events<crate::layer1::culture::celestial_cemeteries::ClearCemeteryEvent>>(
        );
    world.init_resource::<Events<crate::layer1::architecture::smart_matter::RaidEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::heroic_acts::TriggerHeroicAct>>();
    world.init_resource::<Events<crate::layer1::social::hoarder::ConfiscateHoardEvent>>();
    world.init_resource::<Events<crate::layer1::culture::memorial_revolt::PetDeathEvent>>();
    world.init_resource::<Events<crate::layer1::social::pets::PetDeathEvent>>();
    world.init_resource::<bevy_ecs::event::Events<
        crate::layer1::administration::sentient_bureaucracy::TaskAdministrativelyOptimizedEvent,
    >>();
    world.init_resource::<Events<crate::layer1::gravity_funerals::SunBurialRequestEvent>>();
    world.init_resource::<bevy_ecs::prelude::Events<crate::layer2::celestial_library::LibraryDonationEvent>>();
    world.init_resource::<crate::layer1::logistics::mycelial::MycelialNetwork>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::fleet::FleetCommand>>();
    world.init_resource::<crate::layer1::social::politics::ElectionCycle>();
    world.init_resource::<crate::layer1::social::politics::ActiveMandate>();
    world.init_resource::<bevy_ecs::prelude::Events<crate::layer1::logistics::mycelial::ContaminationEvent>>();
    world.init_resource::<bevy::prelude::Events<
        crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent,
    >>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::DerelictArrivalEvent>>();
    world
        .init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::InterceptDropEvent>>(
        );
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::physics::magnetic_reversal::PoleFlipEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::trade::phantom_trade_routes::FixPhantomRouteEvent>>();
    world.init_resource::<Events<crate::layer2::trade::phantom_limb_logistics::AuditRiskEvent>>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::SwarmArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::orphaned_swarm::SwarmHostileEvent>>();
    world.init_resource::<Events<crate::layer2::events_new::orphaned_swarm::SwarmHostileEvent>>();
    // The sub-light arrival chronicle bridge system reads this event every
    // tick; without registration the first tick panics on the missing
    // Events<SubLightArrivalEvent> resource.
    world.init_resource::<Events<crate::layer2::sub_light_arrival::SubLightArrivalEvent>>();
    world.insert_resource(crate::layer3::economy::market_shock::MarketShockMarket {
        luxury_price: 10.0,
    });
    world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
        is_sovereign: false,
        overlord_id: Some(1),
    });
    world
        .insert_resource(crate::layer3::diplomacy::system_sovereignty::FactionRelations::default());
    world.insert_resource(crate::layer1::economy::existential_audit::PrecursorAI {
        next_audit_tick: 1000,
    });
    world
        .init_resource::<bevy_ecs::prelude::Events<crate::layer1::mycelial::MycelialTripwireEvent>>(
        );
    world.init_resource::<bevy_ecs::prelude::Events<
        crate::layer3::diplomacy::dead_internet::GenerateDiplomaticInteractionEvent,
    >>();
    world.init_resource::<bevy_ecs::prelude::Events<
        crate::layer3::diplomacy::dead_internet::DiplomaticInteraction,
    >>();
    let scenario = start_scenario_definition(config.scenario);
    world.insert_resource(ActiveStartScenario {
        id: scenario.id,
        name: scenario.name,
        difficulty: scenario.difficulty,
    });
    // Spec 1388 The Director: temperament resolved once from the era.
    let director_config =
        crate::layer1::director::DirectorConfig::temperament_for(scenario.id);
    world.insert_resource(director_config);
    world.insert_resource(crate::layer1::director::DirectorState::new(
        director_config,
    ));
    world.insert_resource(GameState::default());
    world.insert_resource(crate::layer1::living_score::ColonyRenown { score: 50.0 });
    world.init_resource::<Events<crate::layer1::administration::fractal_bureaucracy::LogicCascadeEvent>>();
    world.init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>();
    world.init_resource::<crate::layer1::stress::TraumaTracker>();
    // Spec 290 The Phantom Shift: config, inefficiency tracker, shadow economy.
    world.init_resource::<crate::layer1::phantom_shift::PhantomShiftConfig>();
    world.init_resource::<crate::layer1::phantom_shift::ColonyInefficiencyTracker>();
    world.init_resource::<crate::layer1::phantom_shift::ShadowEconomy>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer1::predecessors::WorldTriggerEvent>>();

    world.init_resource::<bevy_ecs::event::Events<crate::layer3::market::ephemeral_market::MarketSpawnEvent>>();

    // Megastructure Scaffolding
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::megastructure::MegastructureProgressEvent>>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer2::megastructure::SolarAnomalyEvent>>(
        );
    world.init_resource::<crate::layer3::diplomacy::proxy_wars::ThreatMap>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer3::diplomacy::wormhole_dumping::DumpWasteEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer3::market::ephemeral_market::MarketTradeEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer3::market::ephemeral_market::MarketTradeFailedEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::secret_societies::SocietyAction>>();

    world.init_resource::<bevy_ecs::event::Events<crate::layer2::skyhooks::LaunchIntent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::solar_flare_lottery::SolarFlareEvent>>();
    world.init_resource::<crate::layer2::cartographers_curse::MapTelemetry>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::cartographers_curse::SellTelemetryEvent>>();
    world.insert_resource(MenuState::default());
    world.init_resource::<crate::layer1::unseen_bureaucracy::ShadowEconomy>();
    #[cfg(feature = "nova")]
    world.init_resource::<crate::experimental::the_haunted_cartographer::HauntedGrid>();

    let mut terrain = generate_terrain(80, 50);

    let mut extensions =
        crate::layer1::systems::map_generation::tether_stump::VerticalExtension::new();
    let center_x = terrain.width / 2;
    let center_y = terrain.height / 2;
    for x in center_x..center_x + 2 {
        for y in center_y..center_y + 2 {
            terrain.set(x, y, TerrainType::IndestructibleStump);
            extensions.mark_stump(x, y);
        }
    }
    world.insert_resource(extensions);

    let mut roof =
        crate::layer1::structural_integrity::RoofGrid::new(terrain.width, terrain.height);
    for y in 0..terrain.height {
        for x in 0..terrain.width {
            if terrain.get(x, y) == Some(crate::layer1::TerrainType::Rock) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                roof.set(x as i32, y as i32, true);
            }
        }
    }
    let fertility = crate::layer1::fertility::FertilityGrid::from_terrain(&terrain);

    // Track artifacts to spawn entities for them
    let mut artifact_positions = Vec::new();
    for y in 0..terrain.height {
        for x in 0..terrain.width {
            if terrain.get(x, y) == Some(crate::layer1::nature::terrain::TerrainType::Artifact) {
                artifact_positions.push((x, y));
            }
        }
    }

    world.insert_resource(terrain);
    world.insert_resource(fertility);
    world.insert_resource(roof);

    for (x, y) in artifact_positions {
        world.spawn((
            crate::layer1::nature::terrain::TerrainType::Artifact,
            crate::layer1::map::GridPosition {
                x: x as i32,
                y: y as i32,
            },
            crate::layer1::artifacts::ArtifactAura {
                radius: 5.0,
                effect: crate::layer1::artifacts::AuraEffect::Insight, // Default to Insight for map gen
            },
            crate::layer1::artifacts::Artifact,
        ));
    }

    world.insert_resource(Viewport::default());
    world.insert_resource(CameraTarget::default());
    world.insert_resource(CameraCurrent::default());
    world.insert_resource(ScreenShake::default());
    world.insert_resource(GlobalHitStop::default());
    world.insert_resource(SimulationTime::default());
    world.insert_resource(WallTime::default());
    world.insert_resource(BuildMode::default());
    world.insert_resource(DesignationMode::default());
    world.insert_resource(OccupiedTiles::default());
    world.insert_resource(crate::layer1::crowding::CrowdingGrid::new(80, 50));
    world.insert_resource(ColonyResources::default());
    // Item 77 Phase 2: atmosphere telegraphing snapshot + shelter order.
    world.insert_resource(crate::layer1::pressure::ColonyAtmosphere::default());
    // Item 104: dwell-filtered LifeSupport warning the status bar reads.
    world.insert_resource(crate::layer1::pressure::LifeSupportWarningDisplay::default());
    // Spec 1387 The Grind: dissatisfaction epoch median, chronicle state,
    // and the dwell-filtered simmering display the status bar reads.
    world.init_resource::<crate::layer1::social::dissatisfaction::ColonyWalletMedian>();
    world.init_resource::<crate::layer1::social::dissatisfaction::GrindChronicleState>();
    world.insert_resource(crate::layer1::social::dissatisfaction::SimmeringDisplay::default());
    // Spec 359 Shift Work: night-worker census, present from setup so
    // headless commands can read it before any tick runs.
    world.insert_resource(crate::layer1::social::shift_work::NightShiftStats::default());
    // Spec 1384: colony suit status snapshot + scenario issue floor.
    world.insert_resource(crate::layer1::eva_suit::ColonySuitStatus::default());
    world.insert_resource(crate::layer1::eva_suit::ScenarioSuitFloor::default());
    world.insert_resource(crate::layer1::pressure::ShelterOrder::default());
    world.insert_resource(crate::shared::random::GlobalRng(rand::rngs::StdRng::from_entropy()));
    world.insert_resource(crate::layer1::social::golden_age::ColonySafety {
        days_without_incident: 0,
    });
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::golden_age::AlertEvent>>();
    world.insert_resource(crate::layer1::purity::PurityMap::default());
    world.insert_resource(ColonyPolicies::default());
    world.insert_resource(crate::layer1::social::sartorial_rebellion::DressCodePolicy::default());
    world.insert_resource(crate::layer1::social::sartorial_rebellion::FactionSignifiers::default());
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::weather::StormImpactEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::environment::geothermal::GeothermalPulseEvent>>();
    world.insert_resource(crate::layer1::environment::geothermal::GeothermalPulseState::default());
    world.init_resource::<crate::layer1::civic_ideology::ActiveIdeology>();
    world.insert_resource(crate::layer1::social_mimicry::Trend::default());
    world.insert_resource(crate::layer1::factions::Factions::default());
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::protest_crowds::FormMobEvent>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::protest_crowds::DisperseMobEvent>>();
    world.insert_resource(InputContextStack::default());
    world.insert_resource(MessageLog::default());
    world.insert_resource(Chronicle::default());
    world.init_resource::<crate::layer1::economy::debt_of_the_dead::SocializedDebt>();
    world.init_resource::<crate::layer1::economy::ColonyPrices>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::pop_memories::FamineEvent>>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer1::law::justice::CrimeCommittedEvent>>(
        );
    world.insert_resource(crate::ui::tech::TechUiState::default());
    world.insert_resource(crate::ui::shell::ShellConfig::default());
    world.insert_resource(UiState::default());
    world.init_resource::<Input>();
    world.insert_resource(NotificationQueue::default());
    world.insert_resource(BuildingTracker::default());
    world.insert_resource(crate::layer1::vermin::VerminState::default());
    world.insert_resource(Selection::default());
    world.insert_resource(RenderCache::default());
    world.insert_resource(UtilityConfig::default());
    world.insert_resource(crate::layer1::sleepwalking::SleepwalkingConfig::default());
    world.insert_resource(SeasonState::default());
    world.insert_resource(NamedLocations::default());
    world.insert_resource(TechState::default());
    world.insert_resource(crate::layer1::trade::MerchantState::default());
    world.insert_resource(crate::layer1::trade::TradeMarket::default());
    world.insert_resource(crate::layer1::economy::curio_trade::CurioTradeLedger::default());
    world.init_resource::<Events<crate::layer1::economy::curio_trade::TradeCompletedEvent>>();
    world.init_resource::<Events<crate::layer1::economy::curio_trade::CulturalLossEvent>>();
    // Spec 343 The Rumor Web: adjacency-spread ledger.
    world.init_resource::<crate::layer1::social::rumor::RumorWebLedger>();
    // Spec 327 The Stowaway (Item 87): hidden-pop agenda tuning must exist
    // from setup, not only from init_simulation_resources on the first
    // tick — headless dispatches commands (stowaway spawn, muster) before
    // any tick runs, and those helpers read this config unconditionally.
    world.init_resource::<crate::layer1::social::ship_stowaway::StowawayConfig>();
    world.init_resource::<crate::layer1::social::emotional_contagion::ContagionLedger>();
    // Spec 341 Pop Memory: witness events, episode state, colony identity.
    world.init_resource::<Events<crate::layer1::memory::WitnessEvent>>();
    world.init_resource::<crate::layer1::memory::PopMemoryState>();
    world.init_resource::<crate::layer1::memory::ColonyIdentity>();
    world.insert_resource(crate::layer1::beauty::BeautyGrid::new(80, 50));
    world.insert_resource(crate::layer1::erosion::ErosionGrid::new(80, 50));
    world.insert_resource(crate::layer1::water::WaterGrid::new(80, 50));
    world.insert_resource(crate::layer1::zone::ZoneGrid::new(80, 50));
    world.insert_resource(crate::layer1::acoustic::NoiseMap::new(80, 50));
    world.init_resource::<crate::layer1::olfactory::ScentMap>();
    world.insert_resource(crate::layer1::hum::HumMap::new(80, 50));
    world.insert_resource(crate::layer1::psychology::void_stare::VoidGrid::new(80, 50));
    world.insert_resource(crate::layer1::clutter::ClutterGrid::new(80, 50));
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::social::pop_relationships::ShiftEndEvent>>();
    world.insert_resource(crate::layer1::social::empty_room::ActiveSanctuaries::default());
    world.insert_resource(AtmosphereGrid::new(80, 50));
    world.insert_resource(crate::layer1::nature::atmospheric_empathy::TraceGasGrid::new(80, 50));
    world.insert_resource(crate::layer1::wind::WindGrid::new(80, 50));
    world.insert_resource(crate::layer1::wind::GlobalWind::default());
    world.insert_resource(crate::layer1::atmosphere::BaseGlobalWind::default());
    // Spec 1374: atmospheric resonance — canyon map precomputed from terrain,
    // howling threshold + live howling state.
    world.insert_resource(crate::layer1::physics::resonance::HowlingThreshold::default());
    world.init_resource::<crate::layer1::physics::resonance::HowlingState>();
    {
        let terrain = world.resource::<crate::layer1::TerrainGrid>();
        let canyon_map =
            crate::layer1::physics::resonance::CanyonMap::from_terrain(terrain);
        world.insert_resource(canyon_map);
    }
    world.insert_resource(crate::layer1::atmosphere::AtmosphericTide::default());
    world.insert_resource(crate::layer1::atmosphere::DiffusionConfig::default());
    world.init_resource::<crate::layer1::atmosphere::CorrosiveAtmosphere>();
    world.insert_resource(crate::layer1::pressure::PressureGrid::new(80, 50));
    world.insert_resource(crate::layer1::desire_dust::DesireDust::default());
    world.insert_resource(crate::layer1::temperature::TemperatureGrid::new(
        80, 50, 15.0,
    ));
    world.insert_resource(crate::layer1::radioactive::RadiationGrid::new(80, 50));
    world.insert_resource(LightMap::new(80, 50));
    world.insert_resource(crate::layer1::environment::light_pollution::SkyGlow::default());
    world.insert_resource(AmbientLight::default());
    world.insert_resource(crate::layer1::day_night::DayNightCycle::default());
    world.insert_resource(crate::layer1::PlanetaryTraits::default());
    world.insert_resource(crate::layer2::syzygy::PlanetaryGravity::default());
    world.insert_resource(crate::layer1::weather::WeatherState::default());
    world.insert_resource(crate::layer1::environment::terraforming::PlanetaryAtmosphere::default());
    world.insert_resource(crate::layer1::solar::SolarCycleState::default());
    world.insert_resource(crate::layer1::medical::MedicalPolicy::default());
    world.insert_resource(crate::layer1::prototyping::BuildingMastery::default());
    world.insert_resource(crate::layer1::graffiti::GraffitiMap::default());
    world.insert_resource(crate::layer1::geology::SeismicGrid::new(80, 50));
    world.insert_resource(crate::layer1::environment::seismic::VibrationGrid::new(
        80, 50,
    ));
    world.insert_resource(crate::layer1::ecology::EcologyConfig::default());
    world.insert_resource(crate::layer1::social::society::SecretSocieties::default());
    world.insert_resource(crate::layer1::unrest::Unrest::default());
    // Spec 272 The Martyrdom Effect: fervor state, enemy-strike ledger, casus belli.
    world.insert_resource(crate::layer1::social::martyrdom::Martyrdom::default());
    world.insert_resource(crate::layer1::tech::posthumous_work_shift::PosthumousStats::default());
    world.insert_resource(crate::layer1::social::martyrdom::LastEnemyStrike::default());
    world.insert_resource(crate::layer1::social::martyrdom::IdeologicalCasusBelli::default());
    world.insert_resource(crate::layer1::tech_envy::TechEnvyConfig::default());
    world.insert_resource(crate::layer1::building::BuildingMap::default());
    world.insert_resource(crate::layer1::law::predictive_policing::PredictionConfig {
        threshold: 0.8,
        enabled: true,
    });
    #[cfg(feature = "nova")]
    world.init_resource::<crate::layer1::oral_tradition::OralTradition>();
    #[cfg(feature = "nova")]
    world.insert_resource(crate::layer1::loci::LociMap::new(80, 50));
    world.init_resource::<crate::layer1::social::old_guard::Demographics>();
    world.init_resource::<crate::layer1::economy::beacon::ColonyBeacon>();
    world.init_resource::<crate::layer1::economy::smugglers_cove::ColonyAuthority>();
    world.insert_resource(crate::shared::view_mode::ViewMode::default());
    world.insert_resource(crate::layer2::culture::cultural_drift::HomeworldLocation {
        position: bevy::prelude::Vec2::ZERO,
    });
    world.insert_resource(crate::layer2::system::SystemMap);
    world.insert_resource(crate::layer2::visibility::SystemVisibility::default());

    // Initialize System Generation (Layer 2)
    let mut rng = rand::thread_rng();
    let seed = crate::shared::random::WorldSeed(rng.next_u64());
    world.insert_resource(seed);

    // Run generation system once
    let mut system_schedule = Schedule::default();
    system_schedule.add_systems(crate::layer2::generation::generate_system);
    system_schedule.run(&mut world);

    // Spec 345: generation just chose this world's quirks — derive the
    // centralized GlobalModifiers from them and chronicle the survey
    // report, so the player learns what kind of world they landed on.
    {
        let traits = world.resource::<crate::layer1::PlanetaryTraits>().0.clone();
        world.insert_resource(crate::layer1::quirks::GlobalModifiers::from_traits(&traits));
        if let Some(text) = crate::layer1::quirks::quirks_chronicle_text(&traits) {
            world.resource_mut::<Chronicle>().add_event(
                0,
                text,
                crate::layer1::chronicle::EventImportance::Major,
            );
        }
    }

    initialize_visitor_source(&mut world);
    world.insert_resource(crate::layer1::inspector::InspectorSource {
        next_visit_tick: 5000,
    });

    world.init_resource::<Events<crate::layer1::psionics::FireEvent>>();
    world.init_resource::<Events<crate::layer1::psionics::WorkFailedEvent>>();
    world.init_resource::<Events<crate::layer1::shipbreaking_symbiotic::SymbioticSalvageEvent>>();
    world.init_resource::<Events<AddChronicleEvent>>();
    world.init_resource::<Events<crate::layer2::orphan_fleet::HackOrphanFleetEvent>>();
    world.init_resource::<Events<crate::layer2::orphan_fleet::OrphanFleetDefectionEvent>>();
    world.init_resource::<Events<crate::layer2::primitives::PrimitiveRetaliationEvent>>();
    world.init_resource::<Events<crate::layer1::radio_nostalgia::BroadcastReceivedEvent>>();
    world.init_resource::<Events<crate::layer1::economy::TradeImportEvent>>();
    world.init_resource::<Events<AffinityChange>>();
    // world.init_resource::<Events<crate::layer1::DeathEvent>>();
    world.init_resource::<Events<PopDied>>();
    world.init_resource::<Events<crate::layer1::culture::ancestral_graves::SacrilegeEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
    world.init_resource::<Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::open_source_science::PublishDiscoveryEvent>>();
    world.init_resource::<Events<crate::layer2::asteroid_hermits::DiscoveryEvent>>();
    world.init_resource::<Events<crate::layer2::station::ForgeCrushEvent>>();
    world.init_resource::<Events<crate::layer2::orbit::asteroid_claims::AttackColonyEvent>>();
    world.init_resource::<Events<crate::layer1::systems::dead_hand::DoomsdayTriggeredEvent>>();
    world.init_resource::<Events<PopBorn>>();
    world.init_resource::<Events<crate::layer1::core::events::BuildingCompletedEvent>>();
    world.init_resource::<Events<crate::layer1::parasitic_architecture::BuildingConsumedEvent>>();
    world.init_resource::<Events<crate::layer1::core::events::BuildingRemovedEvent>>();
    world.init_resource::<Events<crate::layer1::architecture::BiomimeticShiftEvent>>();
    world.init_resource::<Events<crate::layer1::structural_integrity::StructureCollapsed>>();
    world.init_resource::<Events<crate::layer1::structural_integrity::CaveInEvent>>();
    world.insert_resource(crate::layer1::structural_integrity::CaveInLedger::default());
    world.init_resource::<Events<RetrogradeEngineeringEvent>>();
    world.init_resource::<Events<crate::layer1::energy::GridOverloadEvent>>();
    world.init_resource::<crate::layer1::energy::grid_strain::GridStrainTracker>();
    world.init_resource::<crate::layer1::core::integration::ThermalChronicleTracker>();
    world.init_resource::<crate::layer1::energy::cable_heat::CableHeatTracker>();
    world.init_resource::<Events<crate::layer1::psychology::memory_forgery::TruthOutbreakEvent>>();
    world.init_resource::<Events<crate::layer1::geology::GeologicalEvent>>();
    world.init_resource::<Events<crate::layer1::geology::fracking::FrackEvent>>();
    world.init_resource::<Events<crate::layer1::geology::subsurface::KineticStrikeEvent>>();
    world.init_resource::<Events<crate::layer1::environment::ephemeral_moons::MoonCapturedEvent>>();
    world.init_resource::<Events<crate::layer1::environment::ephemeral_moons::MoonEjectedEvent>>();
    world.init_resource::<Events<crate::layer1::nature::megafauna_terrain::AwakenTitanEvent>>();
    world.init_resource::<Events<crate::layer1::environment::impact::ImpactWarningEvent>>();
    world.init_resource::<Events<crate::layer1::environment::impact::ImpactStrikeEvent>>();
    world.init_resource::<Events<crate::layer1::geography::HistoricalEvent>>();
    world.init_resource::<Events<crate::layer1::social::society::InvestigationEvent>>();
    world.init_resource::<Events<crate::layer1::social::society::SuppressSocietyEvent>>();
    world.init_resource::<Events<crate::layer1::geology::tectonic::MegaQuakeEvent>>();
    world.init_resource::<Events<crate::layer1::geology::tectonic::ReliefQuakeEvent>>();
    world.init_resource::<Events<crate::layer1::shields::DamageEvent>>();
    world.init_resource::<Events<crate::layer1::ancestral_graves::SacrilegeEvent>>();
    world.init_resource::<Events<crate::layer1::resources::MiningEvent>>();
    world.init_resource::<Events<crate::layer1::shipbreaking::SpawnCrashedShipEvent>>();
    world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::LeaseZoneEvent>>();
    world.init_resource::<Events<crate::layer1::social::factions::subcontractor_factions::MegacorpSecuritySweepEvent>>();
    world.init_resource::<Events<crate::layer1::shipbreaking::MineEvent>>();
    world.init_resource::<crate::layer1::geology::tectonic::TectonicStress>();
    world.init_resource::<crate::layer1::tech::infinite_archive::Archive>();
    world.init_resource::<crate::layer2::trade::blockade::ColonyDebt>();
    world.init_resource::<crate::layer2::thermal::ThermalSignature>();
    world.init_resource::<Events<crate::layer1::medical::PatientTreated>>();
    world.init_resource::<Events<crate::layer1::haunted_assembly_lines::PopDiedInAccidentEvent>>();
    world.init_resource::<Events<crate::layer1::genetics::GeneSplicingEvent>>();
    world.init_resource::<Events<crate::layer1::environment::hazards::AmputationEvent>>();
    world.init_resource::<Events<crate::layer1::social::FavorChange>>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::mobile_architecture::TransformCommand>>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer1::mobile_architecture::MoveCommand>>(
        );
    world.init_resource::<Events<crate::layer1::social::debt::LifeSavedEvent>>();
    world.init_resource::<Events<crate::layer1::social::debt::CallInFavorEvent>>();
    world.init_resource::<Events<crate::layer1::eureka::EurekaEvent>>();
    world.init_resource::<Events<crate::layer1::items::UnequipEvent>>();
    world.init_resource::<Events<crate::layer1::economy::bio_loom::UnequipFailedEvent>>();
    world.init_resource::<Events<crate::layer1::construction::GreatWorkCompletedEvent>>();
    world.init_resource::<Events<crate::layer1::construction::ContributeToGreatWorkEvent>>();
    world.init_resource::<Events<crate::layer2::trade::blockade::TradeShipArrivalEvent>>();
    world
        .init_resource::<Events<crate::layer2::orbit::kessler_gambit::TriggerKesslerGambitEvent>>();
    world.init_resource::<crate::layer1::mind::sleep_debt::SleepDebtConfig>();
    world.init_resource::<Events<crate::layer1::mind::sleep_debt::RepoManArrivalEvent>>();
    world.init_resource::<Events<crate::layer2::events::LaunchEvent>>();
    world.init_resource::<Events<crate::layer2::stolen_fleet::WarDeclarationEvent>>();
    world.init_resource::<Events<crate::layer2::events::ShipDestroyedEvent>>();
    world.init_resource::<Events<crate::layer2::dead_protocols::ViolationEvent>>();
    world.init_resource::<Events<crate::layer3::quarantine::QuarantineBounceEvent>>();
    world.init_resource::<Events<crate::layer2::orbital_mirrors::MirrorFocusEvent>>();

    world.init_resource::<Events<crate::layer3::map::HyperlaneCollapseEvent>>();
    world.init_resource::<Events<crate::layer3::map::TradeRouteSeveredEvent>>();
    // Spec 352: stellar-drift formation events + topology ledger.
    world.init_resource::<Events<crate::layer3::map::HyperlaneFormedEvent>>();
    world.init_resource::<crate::layer3::map::GalaxyTopology>();
    world.init_resource::<Events<crate::layer2::moon_hermits::PopDesertedEvent>>();
    world.init_resource::<Events<crate::layer1::social::gossip_economy::GossipEvent>>();
    world.init_resource::<Events<crate::layer1::social::gossip_economy::BrokerPurchaseEvent>>();
    world.init_resource::<crate::layer1::social::gossip_economy::IntelTokens>();
    world.init_resource::<Events<crate::layer1::direct_link::PossessEntityEvent>>();
    world.init_resource::<Events<crate::layer1::direct_link::UnpossessEvent>>();
    world.init_resource::<Events<crate::layer1::environment::volatile::ExplosionEvent>>();
    world.init_resource::<Events<crate::layer1::unrest::DenounceEvent>>();
    world.init_resource::<Events<crate::layer1::skills::XpGainEvent>>();
    world.init_resource::<Events<crate::layer1::hologram::HologramFailureEvent>>();
    world.init_resource::<Events<crate::layer1::temporal_ghost_towns::TemporalStutterEvent>>();
    world.init_resource::<Events<crate::layer2::events::DetectionEvent>>();
    world.init_resource::<Events<crate::layer1::logistics::orbital_drop::OrbitalDropEvent>>();
    world.init_resource::<Events<crate::layer1::economy::remittances::MigrantArrivalEvent>>();
    world.init_resource::<crate::layer1::economy::remittances::RemittanceTracker>();
    world.init_resource::<Events<crate::layer1::drone::DroneDisconnectedEvent>>();
    world.init_resource::<Events<crate::layer1::overview_effect::ObserveEvent>>();
    world.init_resource::<Events<crate::layer1::genetics::crop_modification::CropMutationEvent>>();
    world.init_resource::<Events<crate::layer1::tech::neural_leech::NeuralHubDeathEvent>>();
    world.init_resource::<Events<crate::layer1::economy::inflation::MarketCrashEvent>>();
    world.init_resource::<Events<crate::layer1::economy::inflation::BarterRequest>>();
    world.insert_resource(crate::layer1::economy::inflation::MarketState {
        credit_value_multiplier: 1.0,
    });

    world.init_resource::<crate::layer1::festivals::FestivalState>();
    world.init_resource::<crate::layer1::shadow_market::ShadowMarketCooldown>();
    world.insert_resource(crate::layer1::taboo::TabooState::default());
    world.insert_resource(crate::layer1::social::slippery_slope::Desensitization::default());

    // Initialize GPU compute context (non-fatal if no GPU available)
    // Skip on WASM since pollster::block_on doesn't work in browser context
    // Skip if headless mode is requested
    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    if !config.headless {
        match pollster::block_on(crate::gpu::context::GpuContext::new()) {
            Ok(ctx) => {
                world.insert_resource(ctx);
            }
            Err(e) => {
                eprintln!("GPU init failed ({e}), falling back to CPU evaluate");
            }
        }
    }
    world.init_resource::<Events<crate::layer1::nature::megafauna_terrain::AwakenTitanEvent>>();

    let generator = NarrativeGenerator::from_embedded();
    let colony_name = generator.generate_star_name();
    world.insert_resource(generator);
    #[cfg(feature = "nova")]
    world.init_resource::<crate::layer1::constellations::Sky>();
    #[cfg(feature = "nova")]
    world.init_resource::<crate::layer1::void_signals::SignalNetwork>();
    #[cfg(feature = "nova")]
    world.init_resource::<crate::experimental::genetic_memory::ColonyGeneticMemory>();

    world.init_resource::<crate::layer1::black_market::ColonyStats>();
    world.init_resource::<crate::layer1::economy::black_market::BlackMarketState>();
    world.init_resource::<crate::layer1::administration::SentientBureaucracyState>();
    world.init_resource::<crate::layer1::void_weed::TradeNetwork>();
    world.init_resource::<crate::layer1::void_weed::SmugglingHeat>();

    world.init_resource::<Events<crate::layer1::void_weed::MerchantArrivalEvent>>();
    world.init_resource::<Events<crate::layer1::void_weed::PirateRaidEvent>>();

    world.init_resource::<crate::layer1::unrest::Unrest>();

    world.init_resource::<Events<crate::layer3::events::debt_prison::BailoutOfferEvent>>();
    world.init_resource::<Events<crate::layer1::social::slippery_slope::AtrocityEvent>>();
    world.init_resource::<Events<crate::layer1::social::slippery_slope::MoraleBuffEvent>>();
    world.init_resource::<Events<crate::layer1::social::slippery_slope::StressPenaltyEvent>>();
    world.init_resource::<Events<crate::layer3::events::debt_prison::AcceptBailoutEvent>>();

    world.init_resource::<Events<crate::layer1::memory_core::ImplantMemoryCoreEvent>>();
    world.init_resource::<Events<crate::layer1::memory_core::HarvestMemoryCoreEvent>>();
    world.init_resource::<Events<crate::layer1::tech::machine_awakening::BotGlitchEvent>>();
    world.init_resource::<Events<crate::layer1::psychology::teleport_psychosis::TeleportEvent>>();
    world.init_resource::<Events<crate::layer1::environment::orbital_tether::TetherSnapEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::RepossessionInvasionEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::WarningDiplomaticMessageEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftOrderEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftComplianceEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::DraftRefusalEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::endless_draft::VeteranReturnEvent>>();
    world.init_resource::<Events<crate::layer2::station::DecommissionDebtTrapEvent>>();
    world.init_resource::<crate::layer3::resources::EmpireCredits>();
    world
        .init_resource::<Events<crate::layer1::social::ghost_shift_strike::GhostShiftStartedEvent>>(
        );
    world.init_resource::<Events<crate::layer1::unseen_bureaucracy::PhantomShiftEvent>>();
    world.init_resource::<Events<crate::layer1::digital_immortality::MindUploadEvent>>();
    world.init_resource::<Events<crate::layer1::digital_immortality::GhostHackEvent>>();
    world.init_resource::<Events<crate::layer2::orbital_necropolis::EntityDestroyedEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::retro_contracts::AcceptRetroContractEvent>>();
    world.init_resource::<Events<crate::layer3::diplomacy::retro_contracts::RetroContractFailedEvent>>();
    world.init_resource::<Events<crate::layer1::heroic_acts::TriggerHeroicAct>>();
    world.init_resource::<Events<crate::layer1::the_lottery::LotteryExecutedEvent>>();
    world
        .init_resource::<Events<crate::layer2::communications::signal_latency::ExecuteOrderEvent>>(
        );
    world.init_resource::<bevy::prelude::Events<crate::layer1::energy::gravity_siphon::OrbitalDecayEvent>>();
    // Adventurer mode: possession events drive both the sim (`handle_possession`)
    // and the UI state (`handle_possession_ui_state`); previously only the
    // direct_link unit tests initialized these, so the UI could never react.
    world.init_resource::<Events<crate::layer1::direct_link::PossessEntityEvent>>();
    world.init_resource::<Events<crate::layer1::direct_link::UnpossessEvent>>();

    world.init_resource::<bevy::prelude::Time>();
    world.init_resource::<crate::layer1::tech::machine_awakening::GlobalSentience>();
    world.init_resource::<crate::layer1::culture::gastronomers::EmpireAdvancement>();
    world.init_resource::<bevy_ecs::event::Events<crate::layer1::culture::gastronomers::CulinarySingularityEvent>>();
    world
        .init_resource::<bevy_ecs::event::Events<crate::layer1::petrification::PopPetrifiedEvent>>(
        );
    world.init_resource::<bevy_ecs::event::Events<crate::layer2::bombardment::BombardmentEvent>>();
    #[cfg(feature = "nova")]
    world.init_resource::<crate::layer1::machine_consciousness::ConsciousnessConfig>();
    world.init_resource::<crate::layer1::mind::fugue::FugueEventTracker>();

    world.insert_resource(ColonyName { name: colony_name });
    generate_world_history(&mut world);

    let starter_colony = spawn_starter_colony(&mut world);

    if let Some(layout) = starter_colony.as_ref() {
        let pop_positions = start_scenario_pop_positions(layout, scenario.id);
        spawn_initial_pops_at_positions(&mut world, &pop_positions);
    } else {
        // No valid 5x5 site for the starter habitat (pathological map). The
        // raw scatter spawns pops into hard vacuum, where
        // pressure_damage_system wipes the colony within seconds, so seed a
        // breathable patch + LifeSupport unit around every pop.
        spawn_initial_pops(&mut world);
        seed_emergency_oxygen_for_pops(&mut world);
    }
    apply_start_scenario_state(&mut world, scenario.id, starter_colony.as_ref());
    spawn_initial_anomalies(&mut world, 5);

    // Spec 1118: Spawn lost tech caches near the tether stump after terrain is placed
    let (stump_x, stump_y) =
        crate::layer1::systems::map_generation::tether_stump::find_stump_center(
            world.resource::<crate::layer1::nature::terrain::TerrainGrid>(),
        )
        .unwrap_or((50, 50));
    world.spawn((
        crate::layer1::systems::map_generation::tether_stump::LostTech,
        bevy::prelude::Transform::from_xyz(stump_x as f32, stump_y as f32, 50.0),
    ));

    spawn_ancient_structures(
        &mut world,
        starter_colony.as_ref().map(|layout| layout.center),
        STARTER_HAZARD_BUFFER_RADIUS,
    );
    initial_naming_system(&mut world);
    // Spec 352: seed the drifting galaxy (systems + hyperlanes) so the
    // registered drift systems act on a live map.
    crate::layer3::map::spawn_initial_galaxy(&mut world);
    initial_chronicle_event(&mut world);
    // Field Guide onboarding: seed the new keeper's pointers alongside the
    // founding entry (once per new colony; saves restore them verbatim).
    crate::ui::help::seed_field_guide_entries(&mut world);
    add_start_scenario_intro_event(&mut world, scenario.id);
    world.insert_resource(AppliedStartScenario { id: scenario.id });

    let initial_pop_count = world
        .query::<&crate::layer1::pop::Pop>()
        .iter(&world)
        .count();
    world.insert_resource(crate::layer1::pop::PopulationCount {
        total: initial_pop_count,
    });
    world
}

fn initialize_visitor_source(world: &mut World) {
    let terrain = world.resource::<crate::layer1::TerrainGrid>();
    let mut spawn_points = Vec::new();
    // Top and Bottom edges
    for x in 0..terrain.width {
        if terrain.get(x, 0).is_some_and(TerrainType::is_walkable) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            spawn_points.push(crate::layer1::GridPosition { x: x as i32, y: 0 });
        }
        if terrain
            .get(x, terrain.height - 1)
            .is_some_and(TerrainType::is_walkable)
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            spawn_points.push(crate::layer1::GridPosition {
                x: x as i32,
                y: (terrain.height - 1) as i32,
            });
        }
    }
    // Left and Right edges (excluding corners already added)
    for y in 1..(terrain.height - 1) {
        if terrain.get(0, y).is_some_and(TerrainType::is_walkable) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            spawn_points.push(crate::layer1::GridPosition { x: 0, y: y as i32 });
        }
        if terrain
            .get(terrain.width - 1, y)
            .is_some_and(TerrainType::is_walkable)
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            spawn_points.push(crate::layer1::GridPosition {
                x: (terrain.width - 1) as i32,
                y: y as i32,
            });
        }
    }

    world.insert_resource(crate::layer1::VisitorSource {
        spawn_points,
        next_spawn_tick: 500,
    });
}

const STARTER_WALL_OFFSETS: &[(i32, i32)] = &[
    (-2, -2),
    (-1, -2),
    (0, -2),
    (1, -2),
    (2, -2),
    (-2, -1),
    (2, -1),
    (-2, 0),
    (2, 0),
    (-2, 1),
    (2, 1),
    (-2, 2),
    (-1, 2),
    (1, 2),
    (2, 2),
];

const STARTER_AIRLOCK_OFFSET: (i32, i32) = (0, 2);
const STARTER_LANDER_OFFSET: (i32, i32) = (1, 0);
const STARTER_LIFE_SUPPORT_OFFSET: (i32, i32) = (-1, 0);
/// Item 77 Phase 2 (forensics bug #1): the starter LifeSupport shipped
/// electrically isolated — the power grid only connects tiles carrying
/// power components, and no power tile sat between the Lander (1, 0) and
/// the LifeSupport (-1, 0), so the generator could never be powered and
/// colonies coasted on the one-time seeded air charge. A PowerPole on
/// the center tile bridges Lander → pole → LifeSupport (both are
/// cardinal-adjacent to the center), so pressure generation is live
/// from tick 0. The pole is a walkable small structure, and the center
/// tile carries no other building — except in Layer 2 Ready, whose
/// CommandCenter sits there and bridges the grid itself (see
/// `apply_layer2_ready_start`).
const STARTER_POWER_POLE_OFFSET: (i32, i32) = (0, 0);
const STARTER_FARM_OFFSET: (i32, i32) = (1, 1);
const STARTER_FARM2_OFFSET: (i32, i32) = (-1, -1);
const STARTER_STOCKPILE_OFFSET: (i32, i32) = (-1, 1);
const STARTER_POP_OFFSETS: &[(i32, i32)] = &[(0, 0), (-1, -1), (0, -1), (1, -1), (0, 1)];
const STARTER_HAZARD_BUFFER_RADIUS: i32 = 14;

#[derive(Resource, Clone)]
struct StarterColonyLayout {
    center: crate::layer1::GridPosition,
    interior_tiles: Vec<crate::layer1::GridPosition>,
    life_support_pos: crate::layer1::GridPosition,
    pop_positions: Vec<crate::layer1::GridPosition>,
}

fn start_scenario_pop_positions(
    layout: &StarterColonyLayout,
    scenario_id: StartScenarioId,
) -> Vec<crate::layer1::GridPosition> {
    match scenario_id {
        StartScenarioId::GroundSurvival => layout.pop_positions.iter().take(4).copied().collect(),
        StartScenarioId::Layer2Ready => {
            let command_center_pos = layout.center;
            let rerouted_pop_pos = crate::layer1::GridPosition {
                x: layout.center.x + 1,
                y: layout.center.y + 1,
            };
            layout
                .pop_positions
                .iter()
                .map(|pos| {
                    if *pos == command_center_pos {
                        rerouted_pop_pos
                    } else {
                        *pos
                    }
                })
                .collect()
        }
        _ => layout.pop_positions.clone(),
    }
}

fn spawn_initial_pops_at_positions(world: &mut World, positions: &[crate::layer1::GridPosition]) {
    let mut rng = rand::thread_rng();
    for pos in positions {
        world.spawn(PopBundle::random(pos.x, pos.y, &mut rng));
    }
}

/// Emergency oxygen for the no-habitat fallback path.
///
/// Gives every pop a breathable 5x5 pressure patch (seeded at 1.0) and drops a
/// LifeSupport unit on an adjacent free tile so the generator maintains it
/// against diffusion into the surrounding vacuum. Without this, scattered pops
/// start in vacuum and `pressure_damage_system` kills the colony within
/// seconds — the mass-extinction-on-tick-5 bootstrap bug.
fn seed_emergency_oxygen_for_pops(world: &mut World) {
    let positions: Vec<crate::layer1::GridPosition> = world
        .query_filtered::<&crate::layer1::GridPosition, With<crate::layer1::pop::Pop>>()
        .iter(world)
        .copied()
        .collect();

    for p in positions {
        if let Some(mut pressure) = world.get_resource_mut::<crate::layer1::pressure::PressureGrid>()
        {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    pressure.set(p.x + dx, p.y + dy, 1.0);
                }
            }
        }

        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (0, 0)] {
            let (cx, cy) = (p.x + dx, p.y + dy);
            let free = (|| {
                let ux = usize::try_from(cx).ok()?;
                let uy = usize::try_from(cy).ok()?;
                let walkable = world
                    .resource::<crate::layer1::TerrainGrid>()
                    .get(ux, uy)?
                    .is_walkable();
                let occupied = world.resource::<OccupiedTiles>().0.contains(&(cx, cy));
                Some(walkable && !occupied)
            })()
            .unwrap_or(false);
            if free {
                crate::layer1::building::spawn_building(
                    world,
                    cx,
                    cy,
                    crate::layer1::building::BuildingType::LifeSupport,
                    crate::layer1::building::MaterialType::default(),
                );
                world.resource_mut::<OccupiedTiles>().0.insert((cx, cy));
                break;
            }
        }
    }
}

fn apply_start_scenario_state(
    world: &mut World,
    scenario_id: StartScenarioId,
    layout: Option<&StarterColonyLayout>,
) {
    match scenario_id {
        StartScenarioId::GroundSurvival => apply_ground_survival_start(world, layout),
        StartScenarioId::SocialDrama => apply_social_drama_start(world),
        StartScenarioId::Layer2Ready => apply_layer2_ready_start(world, layout),
        // Spec 1384 §4.8: Classic — 3 pool suits, Base issue floor.
        StartScenarioId::Classic => {
            world.resource_mut::<ColonyResources>().eva_suits = 3.0;
        }
    }
}

fn apply_ground_survival_start(world: &mut World, layout: Option<&StarterColonyLayout>) {
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.food = 4.0;
        resources.wood = 8.0;
        resources.stone = 2.0;
        resources.tools = 1.0;
        // Spec 1384 §4.8: Ground Survival — 2 pool suits, Base floor.
        resources.eva_suits = 2.0;
    }

    let mut pop_entities: Vec<_> = world
        .query_filtered::<(Entity, &crate::layer1::map::GridPosition), With<crate::layer1::pop::Pop>>()
        .iter(world)
        .map(|(entity, pos)| (entity, pos.x, pos.y))
        .collect();
    pop_entities.sort_by_key(|(_, x, y)| (*y, *x));
    for (entity, _, _) in pop_entities.into_iter().skip(4) {
        world.despawn(entity);
    }

    let Some(layout) = layout else { return };

    if let Some(mut pressure) = world.get_resource_mut::<crate::layer1::pressure::PressureGrid>() {
        for tile in &layout.interior_tiles {
            pressure.set(tile.x, tile.y, 0.45);
        }
        // NOTE: the life-support tile is one of the interior tiles above; it
        // must stay >= 0.45 (the pressure_damage_system suffocation threshold
        // is 0.2). A previous revision seeded it at 0.15, which suffocated any
        // pop idling on the generator.
    }

    let mut life_support_query = world.query::<(
        &crate::layer1::building::Building,
        &crate::layer1::map::GridPosition,
        &mut crate::layer1::structure::Structure,
    )>();
    for (building, pos, mut structure) in life_support_query.iter_mut(world) {
        if building.building_type == crate::layer1::building::BuildingType::LifeSupport
            && *pos == layout.life_support_pos
        {
            structure.current_hp = (structure.max_hp * 0.35).max(1.0);
        }
    }
}

fn apply_social_drama_start(world: &mut World) {
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.food = resources.food.max(12.0);
        resources.tools = resources.tools.max(3.0);
        resources.wood = resources.wood.max(12.0);
        // Spec 1384 §4.8: Social Drama — 3 pool suits, Base floor.
        resources.eva_suits = 3.0;
    }

    let immigrant_arrival_tick = crate::layer1::social::old_guard::FOUNDER_CUTOFF_YEAR
        * crate::layer1::balance::TICKS_PER_YEAR
        + 1;

    let mut pop_entities: Vec<_> = world
        .query_filtered::<(Entity, &crate::layer1::map::GridPosition), With<crate::layer1::pop::Pop>>()
        .iter(world)
        .map(|(entity, pos)| (entity, pos.x, pos.y))
        .collect();
    pop_entities.sort_by_key(|(entity, x, y)| (*y, *x, entity.index()));

    for (index, (entity, _, _)) in pop_entities.into_iter().enumerate() {
        let arrival_tick = if index == 0 {
            0
        } else {
            immigrant_arrival_tick
        };
        let ethic = if index == 0 {
            crate::layer1::social::indoctrination::Ethic::StateLoyalist
        } else {
            crate::layer1::social::indoctrination::Ethic::FreeThinker
        };

        let mut entity_mut = world.entity_mut(entity);
        entity_mut.insert(crate::layer1::social::old_guard::Arrival { tick: arrival_tick });
        entity_mut.insert(crate::layer1::social::indoctrination::PopEthics {
            ethic,
            stubbornness: 0.9,
        });
        entity_mut.remove::<crate::layer1::social::old_guard::Generation>();
        entity_mut.remove::<crate::layer1::social::old_guard::FounderBuff>();
        entity_mut.remove::<crate::layer1::social::old_guard::MoodModifiers>();
    }

    world.insert_resource(crate::layer1::social::old_guard::Demographics::default());
    let _ = world.run_system_once(crate::layer1::social::old_guard::apply_founder_benefits_system);
    let _ =
        world.run_system_once(crate::layer1::social::old_guard::check_generational_friction_system);
    let _ = world.run_system_once(crate::layer1::chronicle::chronicle_event_handler_system);
}

fn apply_layer2_ready_start(world: &mut World, layout: Option<&StarterColonyLayout>) {
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.food = resources.food.max(14.0);
        resources.wood = resources.wood.max(20.0);
        resources.stone = resources.stone.max(20.0);
        // Item 77 Phase 2 (Layer 2 Ready maintenance lane): 25 base metal
        // plus 50 earmarked for one spare LifeSupport (its build cost) —
        // the relief lane keeps a replacement unit affordable instead of
        // letting a sabotage kill become a death sentence.
        resources.metal = resources.metal.max(75.0);
        resources.tools = resources.tools.max(4.0);
        resources.knowledge = resources.knowledge.max(15.0);
        // Spec 1384 §4.8: Layer 2 Ready — 6 pool suits and a Mk II
        // issue floor (the preparation lane is already closed-cycle).
        resources.eva_suits = resources.eva_suits.max(6.0);
    }
    world.insert_resource(crate::layer1::eva_suit::ScenarioSuitFloor(
        crate::layer1::eva_suit::SuitClass::Mk2,
    ));

    let Some(layout) = layout else { return };

    let command_center_pos = layout.center;
    let rerouted_pop_pos = crate::layer1::GridPosition {
        x: layout.center.x + 1,
        y: layout.center.y + 1,
    };

    for mut pos in world
        .query_filtered::<&mut crate::layer1::map::GridPosition, With<crate::layer1::pop::Pop>>()
        .iter_mut(world)
    {
        if *pos == command_center_pos {
            *pos = rerouted_pop_pos;
        }
    }

    let has_command_center = world
        .query::<(
            &crate::layer1::building::Building,
            &crate::layer1::map::GridPosition,
        )>()
        .iter(world)
        .any(|(building, pos)| {
            building.building_type == crate::layer1::building::BuildingType::CommandCenter
                && *pos == command_center_pos
        });

    if !has_command_center {
        crate::layer1::building::spawn_building(
            world,
            command_center_pos.x,
            command_center_pos.y,
            crate::layer1::building::BuildingType::CommandCenter,
            crate::layer1::building::MaterialType::Metal,
        );
        world
            .resource_mut::<OccupiedTiles>()
            .0
            .insert((command_center_pos.x, command_center_pos.y));
    }

    // Item 77 Phase 2 (Layer 2 Ready maintenance lane ONLY — Classic,
    // Ground Survival, and Social Drama numbers are untouched): atmosphere
    // redundancy and a Generator on the free (0, -1) interior tile,
    // cardinal-adjacent to the CommandCenter's power consumer on the
    // center tile, so it joins the Lander's grid: if either LifeSupport
    // is sabotaged to scrap, the other keeps the hull pressurized, and
    // the Generator is a second power source behind the boosted Lander.
    // This is maintenance relief that does not decay — the lane Mark
    // designated "much easier to maintain".
    //
    // Item 110 Gap 3 (fire geometry): the second LifeSupport used to sit
    // on the (0, 1) interior tile — inside the same 3x3 starter heart as
    // the primary LifeSupport, both Farms, the Stockpile, the Generator
    // and the DroneHub. One ignition in the heart was adjacent to
    // everything critical, and the l2r2k_11 cascade took both
    // "redundant" units ~11 ticks apart (balance forensics 2026-10-20).
    // Redundancy that shares a fire neighborhood is not redundancy: the
    // second unit now lives in a sealed annex off the hull at distance
    // 4 in the first workable direction (east, then west, then north;
    // see the direction search below) — Chebyshev >= 3 from the primary
    // unit and every other heart building (5 for an east annex) —
    // reached by a power/pressure tunnel: the starter wall at distance
    // 2 becomes a metal PowerPole, a second metal PowerPole at
    // distance 3 continues the cardinal power chain
    // (Lander -> tunnel poles -> annex LifeSupport, all one grid), and
    // metal walls at distance 4 (flanks) and 5 (cap) seal the annex cell.
    // PowerPoles have no flow transmissivity, so hull pressure diffuses
    // through the tunnel and the annex unit still pressurizes the hull
    // if the primary dies (and vice versa). Metal, not the wood default:
    // a flammable tunnel would be a fire bridge into the annex. If the
    // annex tiles are somehow unplaceable, fall back to the legacy
    // (0, 1) interior tile so the lane never loses its second unit.
    let generator_pos = crate::layer1::GridPosition {
        x: layout.center.x,
        y: layout.center.y - 1,
    };
    let building_at = |world: &mut World,
                       pos: crate::layer1::GridPosition,
                       kind: crate::layer1::building::BuildingType|
     -> bool {
        world
            .query::<(
                &crate::layer1::building::Building,
                &crate::layer1::map::GridPosition,
            )>()
            .iter(world)
            .any(|(building, p)| building.building_type == kind && *p == pos)
    };
    // Annex direction search: the starter-site guarantee covers only
    // the 5x5 shell, so any single annex direction can be blocked by
    // worldgen (rock, water, a ruin on an annex tile). Try east, west,
    // then north; the first viable annex wins. (South is the airlock
    // side — its ring tile is the Airlock, not a wall, so no tunnel can
    // start there.) Every direction keeps the annex unit >= 3 tiles
    // (Chebyshev) from the primary LifeSupport and every other heart
    // building.
    //
    // A Rock tile on the annex footprint counts as viable: it is
    // clearable site prep (the landing clearing already guarantees the
    // 5x5 shell, and rock is mineable terrain). Measured on worldgen:
    // single Rock outcrops otherwise veto whole directions — in one
    // 30-world sample 2 worlds fell back with every direction blocked
    // by exactly one Rock tile. Water stays a hard blocker: site prep
    // does not drain lakes. Viable Rock tiles are cleared to Grass
    // when the annex is built (below).
    let annex_tile_viable = |world: &World, pos: crate::layer1::GridPosition| -> bool {
        if crate::layer1::building::can_place_building(world, pos.x, pos.y) {
            return true;
        }
        let (Ok(ux), Ok(uy)) = (usize::try_from(pos.x), usize::try_from(pos.y)) else {
            return false;
        };
        world
            .get_resource::<crate::layer1::TerrainGrid>()
            .and_then(|terrain| terrain.get(ux, uy))
            .is_some_and(|tile| tile == crate::layer1::terrain::TerrainType::Rock)
    };
    let legacy_pos = crate::layer1::GridPosition {
        x: layout.center.x,
        y: layout.center.y + 1,
    };
    let mut annex_plan: Option<(
        crate::layer1::GridPosition,
        crate::layer1::GridPosition,
        crate::layer1::GridPosition,
        [crate::layer1::GridPosition; 3],
    )> = None;
    for (ux, uy) in [(1, 0), (-1, 0), (0, -1)] {
        let (px, py) = (-uy, ux);
        let at = |k: i32, p: i32| crate::layer1::GridPosition {
            x: layout.center.x + ux * k + px * p,
            y: layout.center.y + uy * k + py * p,
        };
        let tunnel_wall_pos = at(2, 0);
        let tunnel_far_pos = at(3, 0);
        let annex_pos = at(4, 0);
        let annex_wall_positions = [at(4, -1), at(4, 1), at(5, 0)];
        let ok = building_at(
            world,
            tunnel_wall_pos,
            crate::layer1::building::BuildingType::Wall,
        ) && annex_tile_viable(world, tunnel_far_pos)
            && annex_tile_viable(world, annex_pos)
            && annex_wall_positions
                .iter()
                .all(|pos| annex_tile_viable(world, *pos));
        if ok {
            annex_plan = Some((
                tunnel_wall_pos,
                tunnel_far_pos,
                annex_pos,
                annex_wall_positions,
            ));
            break;
        }
    }
    let annex_ok = annex_plan.is_some();
    let (tunnel_wall_pos, tunnel_far_pos, annex_pos, annex_wall_positions) =
        annex_plan.unwrap_or((legacy_pos, legacy_pos, legacy_pos, [legacy_pos; 3]));
    let redundant_life_support_pos = if annex_ok { annex_pos } else { legacy_pos };
    if !building_at(
        world,
        redundant_life_support_pos,
        crate::layer1::building::BuildingType::LifeSupport,
    ) {
        if annex_ok {
            // Site prep: clear Rock outcrops on the annex footprint to
            // Grass (the viability rule above counts them as clearable).
            for clear_pos in [tunnel_far_pos, annex_pos]
                .into_iter()
                .chain(annex_wall_positions)
            {
                let (Ok(ux), Ok(uy)) = (
                    usize::try_from(clear_pos.x),
                    usize::try_from(clear_pos.y),
                ) else {
                    continue;
                };
                if let Some(mut terrain) =
                    world.get_resource_mut::<crate::layer1::TerrainGrid>()
                {
                    if terrain.get(ux, uy)
                        == Some(crate::layer1::terrain::TerrainType::Rock)
                    {
                        terrain.set(ux, uy, crate::layer1::terrain::TerrainType::Grass);
                    }
                }
            }
            // Open the tunnel: the tunnel-mouth starter wall becomes
            // the first PowerPole of the annex power/pressure chain.
            let wall_entity = world
                .query::<(
                    Entity,
                    &crate::layer1::building::Building,
                    &crate::layer1::map::GridPosition,
                )>()
                .iter(world)
                .find(|(_, building, pos)| {
                    building.building_type == crate::layer1::building::BuildingType::Wall
                        && **pos == tunnel_wall_pos
                })
                .map(|(entity, _, _)| entity);
            if let Some(entity) = wall_entity {
                world.despawn(entity);
                world
                    .resource_mut::<OccupiedTiles>()
                    .0
                    .remove(&(tunnel_wall_pos.x, tunnel_wall_pos.y));
            }
            for pole_pos in [tunnel_wall_pos, tunnel_far_pos] {
                crate::layer1::building::spawn_building(
                    world,
                    pole_pos.x,
                    pole_pos.y,
                    crate::layer1::building::BuildingType::PowerPole,
                    crate::layer1::building::MaterialType::Metal,
                );
                world
                    .resource_mut::<OccupiedTiles>()
                    .0
                    .insert((pole_pos.x, pole_pos.y));
            }
            for wall_pos in annex_wall_positions {
                crate::layer1::building::spawn_building(
                    world,
                    wall_pos.x,
                    wall_pos.y,
                    crate::layer1::building::BuildingType::Wall,
                    crate::layer1::building::MaterialType::Metal,
                );
                world
                    .resource_mut::<OccupiedTiles>()
                    .0
                    .insert((wall_pos.x, wall_pos.y));
            }
        }
        crate::layer1::building::spawn_building(
            world,
            redundant_life_support_pos.x,
            redundant_life_support_pos.y,
            crate::layer1::building::BuildingType::LifeSupport,
            crate::layer1::building::MaterialType::default(),
        );
        world
            .resource_mut::<OccupiedTiles>()
            .0
            .insert((redundant_life_support_pos.x, redundant_life_support_pos.y));
    }
    if !building_at(
        world,
        generator_pos,
        crate::layer1::building::BuildingType::Generator,
    ) {
        crate::layer1::building::spawn_building(
            world,
            generator_pos.x,
            generator_pos.y,
            crate::layer1::building::BuildingType::Generator,
            crate::layer1::building::MaterialType::default(),
        );
        world
            .resource_mut::<OccupiedTiles>()
            .0
            .insert((generator_pos.x, generator_pos.y));
    }

    // Spec 1385 (Layer 2 Ready maintenance lane ONLY): a DroneHub and
    // its starting fleet. The hub takes the starter layout's one free
    // interior tile, (+1, -1) — every other interior tile already
    // carries a starter or L2R grant building. It is cardinal-adjacent
    // to the Lander (+1, 0), a power source, and to the Generator
    // (0, -1), so it joins the grid through the same center cross as
    // the LifeSupport. The fleet is 3 drones: exactly one hub's full
    // complement under the spawner's 3-per-hub cap, so the grant fills
    // the fleet at tick 0 and the spawner only ever replaces losses.
    // Drones are the lane's second vacuum counterplay (machines don't
    // breathe): they repair damaged exterior buildings — LifeSupport
    // above all — without spending pop HP in vacuum.
    let drone_hub_pos = crate::layer1::GridPosition {
        x: layout.center.x + 1,
        y: layout.center.y - 1,
    };
    let drone_hub_entity = if building_at(
        world,
        drone_hub_pos,
        crate::layer1::building::BuildingType::DroneHub,
    ) {
        world
            .query::<(
                Entity,
                &crate::layer1::building::Building,
                &crate::layer1::map::GridPosition,
            )>()
            .iter(world)
            .find(|(_, building, pos)| {
                building.building_type == crate::layer1::building::BuildingType::DroneHub
                    && **pos == drone_hub_pos
            })
            .map(|(entity, _, _)| entity)
    } else {
        let entity = crate::layer1::building::spawn_building(
            world,
            drone_hub_pos.x,
            drone_hub_pos.y,
            crate::layer1::building::BuildingType::DroneHub,
            crate::layer1::building::MaterialType::default(),
        );
        world
            .resource_mut::<OccupiedTiles>()
            .0
            .insert((drone_hub_pos.x, drone_hub_pos.y));
        Some(entity)
    };
    if let Some(hub_entity) = drone_hub_entity {
        // Spawn the fleet with the drone_spawner_bridge_system bundle,
        // except GridConnection starts connected: the hub is powered
        // below in this same function, and a fleet that reads
        // disconnected at tick 0 would sit out its first day.
        for _ in 0..3 {
            world.spawn((
                crate::layer1::drone::Drone {
                    state: crate::layer1::drone::DroneState::Idle,
                },
                crate::layer1::drone::ConnectedTo(hub_entity),
                crate::layer1::drone::GridConnection {
                    is_connected: true,
                    time_disconnected: 0,
                },
                drone_hub_pos,
                crate::layer1::utility_ai::PopAction::default(),
                crate::layer1::drone::DroneBattery {
                    current: 100.0,
                    max: 100.0,
                },
                crate::layer1::pop::Speed {
                    base: 1.0,
                    current: 1.0,
                    accumulator: 0.0,
                },
                crate::layer1::utility_ai::UtilityWeights::default(),
            ));
        }
    }

    let lander_pos = crate::layer1::GridPosition {
        x: layout.center.x + STARTER_LANDER_OFFSET.0,
        y: layout.center.y + STARTER_LANDER_OFFSET.1,
    };
    for (building, pos, mut source) in world
        .query::<(
            &crate::layer1::building::Building,
            &crate::layer1::map::GridPosition,
            &mut crate::layer1::energy::PowerSource,
        )>()
        .iter_mut(world)
    {
        if building.building_type == crate::layer1::building::BuildingType::Lander
            && *pos == lander_pos
        {
            source.output = 100.0;
            source.active = true;
        }
    }

    crate::layer1::energy::power_grid_system(world);
    for (building, pos, mut consumer) in world
        .query::<(
            &crate::layer1::building::Building,
            &crate::layer1::map::GridPosition,
            &mut crate::layer1::energy::PowerConsumer,
        )>()
        .iter_mut(world)
    {
        if building.building_type == crate::layer1::building::BuildingType::CommandCenter
            && *pos == command_center_pos
        {
            consumer.active = true;
        }
        // Spec 1385: the DroneHub gets the same belt-and-braces as the
        // CommandCenter — the fleet's charge point is live at tick 0.
        if building.building_type == crate::layer1::building::BuildingType::DroneHub
            && *pos == drone_hub_pos
        {
            consumer.active = true;
        }
    }
    let _ = world.run_system_once(crate::layer2::visibility::update_visibility_system);
}

fn add_start_scenario_intro_event(world: &mut World, scenario_id: StartScenarioId) {
    match scenario_id {
        StartScenarioId::GroundSurvival => {
            let founder_count = world
                .query::<&crate::layer1::pop::Pop>()
                .iter(world)
                .count();
            world.resource_mut::<Chronicle>().add_event(
                0,
                format!(
                    "Ground Survival: a hard landing left {founder_count} battered founders with thin stores and a failing habitat core."
                ),
                crate::layer1::chronicle::EventImportance::Major,
            );
        }
        StartScenarioId::SocialDrama => {
            world.resource_mut::<Chronicle>().add_event(
                0,
                "Social Drama: the habitat is stocked, but one founder now sleeps among four latecomers and the whole camp feels like a powder keg.".to_string(),
                crate::layer1::chronicle::EventImportance::Major,
            );
        }
        StartScenarioId::Layer2Ready => {
            world.resource_mut::<Chronicle>().add_event(
                0,
                "Layer 2 Ready: an orbital charter and a live command deck put the colony's eyes on the system before the dust has even settled.".to_string(),
                crate::layer1::chronicle::EventImportance::Major,
            );
            // Layer 2 legibility pass: the flavor line above promises a
            // prepared colony; this second entry reads the actual
            // manifest into the record — the drone crew, the suit pool,
            // the redundant Life Support — because nothing else in the
            // game names the kit a Layer 2 Ready keeper starts with.
            world.resource_mut::<Chronicle>().add_event(
                0,
                "The Layer 2 manifest, read into the record: three drones wake on their hub — an autonomous crew that takes the vacuum-side repairs first; keep the hub powered — with six Mk II suits in the pool and a second Life Support already breathing in reserve.".to_string(),
                crate::layer1::chronicle::EventImportance::Major,
            );
        }
        StartScenarioId::Classic => {}
    }
}

/// Applies the currently selected startup scenario to an already-created world.
pub fn apply_selected_start_scenario(world: &mut World) {
    let selected = world.resource::<ActiveStartScenario>().id;
    if world
        .get_resource::<AppliedStartScenario>()
        .is_some_and(|applied| applied.id == selected)
    {
        return;
    }

    let layout = world.get_resource::<StarterColonyLayout>().cloned();
    apply_start_scenario_state(world, selected, layout.as_ref());
    add_start_scenario_intro_event(world, selected);
    world.insert_resource(AppliedStartScenario { id: selected });
}

fn spawn_starter_colony(world: &mut World) -> Option<StarterColonyLayout> {
    let center = find_starter_colony_site(world)?;
    let interior_tiles: Vec<crate::layer1::GridPosition> = (-1..=1)
        .flat_map(|dy| {
            (-1..=1).map(move |dx| crate::layer1::GridPosition {
                x: center.x + dx,
                y: center.y + dy,
            })
        })
        .collect();

    for &(dx, dy) in STARTER_WALL_OFFSETS {
        let x = center.x + dx;
        let y = center.y + dy;
        crate::layer1::building::spawn_building(
            world,
            x,
            y,
            crate::layer1::building::BuildingType::Wall,
            crate::layer1::building::MaterialType::Metal,
        );
        world.resource_mut::<OccupiedTiles>().0.insert((x, y));
    }

    let airlock_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_AIRLOCK_OFFSET.0,
        y: center.y + STARTER_AIRLOCK_OFFSET.1,
    };
    crate::layer1::building::spawn_building(
        world,
        airlock_pos.x,
        airlock_pos.y,
        crate::layer1::building::BuildingType::Airlock,
        crate::layer1::building::MaterialType::Metal,
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((airlock_pos.x, airlock_pos.y));

    let lander_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_LANDER_OFFSET.0,
        y: center.y + STARTER_LANDER_OFFSET.1,
    };
    crate::layer1::building::spawn_building(
        world,
        lander_pos.x,
        lander_pos.y,
        crate::layer1::building::BuildingType::Lander,
        crate::layer1::building::MaterialType::default(),
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((lander_pos.x, lander_pos.y));

    let life_support_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_LIFE_SUPPORT_OFFSET.0,
        y: center.y + STARTER_LIFE_SUPPORT_OFFSET.1,
    };
    crate::layer1::building::spawn_building(
        world,
        life_support_pos.x,
        life_support_pos.y,
        crate::layer1::building::BuildingType::LifeSupport,
        crate::layer1::building::MaterialType::default(),
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((life_support_pos.x, life_support_pos.y));

    // Starter power bridge (Item 77 Phase 2, forensics bug #1): a
    // PowerPole on the center tile connects the Lander's PowerSource to
    // the LifeSupport's PowerConsumer through the power grid's tile
    // adjacency, so `power_grid_system` activates the LifeSupport and
    // `update_pressure_system` actually generates pressure from tick 0.
    // Layer 2 Ready is the one exception: its CommandCenter occupies the
    // center tile and is itself a PowerConsumer bridging the same two
    // neighbors, so a pole there would collide in the grid map.
    let scenario_id = world
        .get_resource::<ActiveStartScenario>()
        .map_or(StartScenarioId::Classic, |active| active.id);
    if scenario_id != StartScenarioId::Layer2Ready {
        let pole_pos = crate::layer1::GridPosition {
            x: center.x + STARTER_POWER_POLE_OFFSET.0,
            y: center.y + STARTER_POWER_POLE_OFFSET.1,
        };
        crate::layer1::building::spawn_building(
            world,
            pole_pos.x,
            pole_pos.y,
            crate::layer1::building::BuildingType::PowerPole,
            crate::layer1::building::MaterialType::default(),
        );
        world
            .resource_mut::<OccupiedTiles>()
            .0
            .insert((pole_pos.x, pole_pos.y));
    }

    // Starter farm: the colony's food supply. Placed on a free interior
    // tile (1, 1) inside the pressurized hull — never on a wall tile.
    // Fertility is set explicitly so production never depends on the
    // underlying terrain, and the crop is Potato for its strong winter
    // modifier (0.8); Wheat would starve the colony in winter (0.2).
    let farm_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_FARM_OFFSET.0,
        y: center.y + STARTER_FARM_OFFSET.1,
    };
    let farm_entity = crate::layer1::building::spawn_building(
        world,
        farm_pos.x,
        farm_pos.y,
        crate::layer1::building::BuildingType::Farm,
        crate::layer1::building::MaterialType::default(),
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((farm_pos.x, farm_pos.y));
    if let Some(mut farm) = world.get_mut::<crate::layer1::agriculture::Farm>(farm_entity) {
        farm.selected_crop = crate::layer1::items::ItemType::Potato;
    }
    if let Some(mut fertility) =
        world.get_resource_mut::<crate::layer1::fertility::FertilityGrid>()
    {
        if let (Ok(ux), Ok(uy)) = (
            usize::try_from(farm_pos.x),
            usize::try_from(farm_pos.y),
        ) {
            fertility.set(ux, uy, 1.0);
        }
    }

    // Second starter farm: one farm's worth of workers (~2) only break even
    // against the colony's burn; two farms give the production headroom to
    // build a buffer against raids, winter, and sabotage-repair duty cycles.
    // Same treatment: Potato for winter resilience, explicit fertility.
    let farm2_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_FARM2_OFFSET.0,
        y: center.y + STARTER_FARM2_OFFSET.1,
    };
    let farm2_entity = crate::layer1::building::spawn_building(
        world,
        farm2_pos.x,
        farm2_pos.y,
        crate::layer1::building::BuildingType::Farm,
        crate::layer1::building::MaterialType::default(),
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((farm2_pos.x, farm2_pos.y));
    if let Some(mut farm) = world.get_mut::<crate::layer1::agriculture::Farm>(farm2_entity) {
        farm.selected_crop = crate::layer1::items::ItemType::Potato;
    }
    if let Some(mut fertility) =
        world.get_resource_mut::<crate::layer1::fertility::FertilityGrid>()
    {
        if let (Ok(ux), Ok(uy)) = (
            usize::try_from(farm2_pos.x),
            usize::try_from(farm2_pos.y),
        ) {
            fertility.set(ux, uy, 1.0);
        }
    }

    // Starter stockpile: a walkable Stockpile building so pops can actually
    // complete FetchTool/FetchClothing/Haul. The Lander also carries a
    // Stockpile component, but it is an obstacle tile — arrival requires
    // standing exactly on the target tile, so the Lander's stockpile is
    // unreachable and used to wedge every pop on FetchTool forever.
    let stockpile_pos = crate::layer1::GridPosition {
        x: center.x + STARTER_STOCKPILE_OFFSET.0,
        y: center.y + STARTER_STOCKPILE_OFFSET.1,
    };
    crate::layer1::building::spawn_building(
        world,
        stockpile_pos.x,
        stockpile_pos.y,
        crate::layer1::building::BuildingType::Stockpile,
        crate::layer1::building::MaterialType::default(),
    );
    world
        .resource_mut::<OccupiedTiles>()
        .0
        .insert((stockpile_pos.x, stockpile_pos.y));

    if let Some(mut pressure) = world.get_resource_mut::<crate::layer1::pressure::PressureGrid>() {
        for tile in &interior_tiles {
            pressure.set(tile.x, tile.y, 1.0);
        }
    }

    let layout = StarterColonyLayout {
        center,
        interior_tiles,
        life_support_pos,
        pop_positions: STARTER_POP_OFFSETS
            .iter()
            .map(|(dx, dy)| crate::layer1::GridPosition {
                x: center.x + dx,
                y: center.y + dy,
            })
            .collect(),
    };
    world.insert_resource(layout.clone());
    Some(layout)
}

fn find_starter_colony_site(world: &World) -> Option<crate::layer1::GridPosition> {
    let (width, height) = {
        let terrain = world.resource::<crate::layer1::TerrainGrid>();
        (terrain.width, terrain.height)
    };
    let center_x = i32::try_from(width / 2).ok()?;
    let center_y = i32::try_from(height / 2).ok()?;
    let max_radius = center_x.max(center_y);

    for radius in 0..=max_radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if radius > 0 && dx.abs() != radius && dy.abs() != radius {
                    continue;
                }

                let candidate = crate::layer1::GridPosition {
                    x: center_x + dx,
                    y: center_y + dy,
                };

                if starter_colony_site_is_valid(world, candidate) {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn starter_colony_site_is_valid(world: &World, center: crate::layer1::GridPosition) -> bool {
    let terrain = world.resource::<crate::layer1::TerrainGrid>();
    let occupied = world.resource::<OccupiedTiles>();

    for dy in -2..=2 {
        for dx in -2..=2 {
            let x = center.x + dx;
            let y = center.y + dy;
            let Ok(ux) = usize::try_from(x) else {
                return false;
            };
            let Ok(uy) = usize::try_from(y) else {
                return false;
            };

            let Some(tile) = terrain.get(ux, uy) else {
                return false;
            };

            if !tile.is_walkable() {
                return false;
            }

            if occupied.0.contains(&(x, y)) {
                return false;
            }
        }
    }

    for &(dx, dy) in STARTER_WALL_OFFSETS {
        if !crate::layer1::building::can_place_building(world, center.x + dx, center.y + dy) {
            return false;
        }
    }

    for &(dx, dy) in &[
        STARTER_AIRLOCK_OFFSET,
        STARTER_LANDER_OFFSET,
        STARTER_LIFE_SUPPORT_OFFSET,
        STARTER_FARM_OFFSET,
        STARTER_FARM2_OFFSET,
        STARTER_STOCKPILE_OFFSET,
    ] {
        if !crate::layer1::building::can_place_building(world, center.x + dx, center.y + dy) {
            return false;
        }
    }

    true
}

use crate::ui::menu_state::MenuState;

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::social::old_guard::{Generation, MoodModifiers};
    use crate::layer1::structure::Structure;
    use crate::layer1::{ColonyResources, GridPosition, Pop, TerrainGrid};
    use crate::shared::colony::ColonyName;
    use crate::shared::narrative::NarrativeGenerator;

    fn starter_shell(world: &mut World) -> Vec<(BuildingType, i32, i32)> {
        let mut shell: Vec<_> = world
            .query::<(&Building, &GridPosition)>()
            .iter(world)
            .filter(|(building, _)| {
                matches!(
                    building.building_type,
                    BuildingType::Wall
                        | BuildingType::Airlock
                        | BuildingType::Lander
                        | BuildingType::LifeSupport
                        | BuildingType::PowerPole
                )
            })
            .map(|(building, pos)| (building.building_type, pos.x, pos.y))
            .collect();
        // Item 110 Gap 3: trim Layer 2 Ready's annex in ABSOLUTE space
        // before normalizing (an annex in any direction would otherwise
        // shift the min-based normalization). The colony center is the
        // Lander's tile minus STARTER_LANDER_OFFSET. The shared shell
        // lives within Chebyshev 2 of the center; the annex unit (4),
        // its walls (4-5) and the tunnel's far pole (3) sit outside and
        // drop out. The tunnel mouth (distance 2) is a shared wall tile
        // that L2R converts into a PowerPole — it still occupies the
        // shared tile, so it is mapped back to Wall below. The shared
        // center bridge pole (distance 0) was never part of the
        // compared shell and drops out.
        if let Some((_, lander_x, lander_y)) = shell
            .iter()
            .find(|(building_type, _, _)| *building_type == BuildingType::Lander)
            .copied()
        {
            let (center_x, center_y) = (
                lander_x - STARTER_LANDER_OFFSET.0,
                lander_y - STARTER_LANDER_OFFSET.1,
            );
            let chebyshev = |x: i32, y: i32| (x - center_x).abs().max((y - center_y).abs());
            shell.retain(|(_, x, y)| chebyshev(*x, *y) <= 2);
            shell.retain(|(building_type, x, y)| {
                *building_type != BuildingType::PowerPole || chebyshev(*x, *y) == 2
            });
            for entry in &mut shell {
                if entry.0 == BuildingType::PowerPole {
                    entry.0 = BuildingType::Wall;
                }
            }
        }
        let min_x = shell.iter().map(|(_, x, _)| *x).min().unwrap_or(0);
        let min_y = shell.iter().map(|(_, _, y)| *y).min().unwrap_or(0);
        for (_, x, y) in &mut shell {
            *x -= min_x;
            *y -= min_y;
        }
        shell.sort_by_key(|(_, x, y)| (*x, *y));
        // Item 77 Phase 2: Layer 2 Ready adds a redundant second
        // LifeSupport inside the shell; the shared-shell comparison is
        // about the starter layout every scenario has in common, so it
        // keeps only the first (shared) LifeSupport entry.
        let mut seen_life_support = false;
        shell.retain(|(building_type, _, _)| {
            if *building_type == BuildingType::LifeSupport {
                if seen_life_support {
                    return false;
                }
                seen_life_support = true;
            }
            true
        });
        shell
    }

    fn life_support_structure(world: &mut World) -> Structure {
        world
            .query::<(&Building, &Structure)>()
            .iter(world)
            .find(|(building, _)| building.building_type == BuildingType::LifeSupport)
            .map(|(_, structure)| *structure)
            .expect("starter colony should include life support")
    }

    fn generation_counts(world: &mut World) -> (usize, usize) {
        let mut founders = 0;
        let mut immigrants = 0;
        for generation in world.query::<&Generation>().iter(world) {
            match generation {
                Generation::Founder => founders += 1,
                Generation::Immigrant => immigrants += 1,
            }
        }
        (founders, immigrants)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ScenarioSignature {
        pop_count: usize,
        founders: usize,
        immigrants: usize,
        food_tenths: i32,
        tools_tenths: i32,
        visibility: crate::layer2::visibility::SystemVisibility,
        command_center_count: usize,
        life_support_damaged: bool,
    }

    fn scenario_signature(scenario: StartScenarioId) -> ScenarioSignature {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario,
        });

        let resources = *world.resource::<ColonyResources>();
        let visibility = *world.resource::<crate::layer2::visibility::SystemVisibility>();
        let (founders, immigrants) = generation_counts(&mut world);
        let pop_count = world.query::<&Pop>().iter(&world).count();
        let life_support = life_support_structure(&mut world);
        let command_center_count = world
            .query::<&Building>()
            .iter(&world)
            .filter(|building| building.building_type == BuildingType::CommandCenter)
            .count();

        ScenarioSignature {
            pop_count,
            founders,
            immigrants,
            food_tenths: (resources.food * 10.0).round() as i32,
            tools_tenths: (resources.tools * 10.0).round() as i32,
            visibility,
            command_center_count,
            life_support_damaged: life_support.current_hp < life_support.max_hp,
        }
    }

    fn run_start_scenario_smoke_ticks(scenario: StartScenarioId, ticks: u64) -> World {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario,
        });
        world.init_resource::<crate::layer1::culture::celestial_cemeteries::OrbitalCemetery>();
        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::nature::ecology::HarvestEvent>>(
            );
        world.init_resource::<Events<crate::layer1::culture::celestial_cemeteries::ClearCemeteryEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::smart_matter::RaidEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
        world
            .init_resource::<bevy_ecs::event::Events<crate::layer1::heroic_acts::TriggerHeroicAct>>(
            );
        world.init_resource::<Events<crate::layer1::culture::memorial_revolt::PetDeathEvent>>();
        world.init_resource::<Events<crate::layer1::social::pets::PetDeathEvent>>();
        world.init_resource::<bevy_ecs::event::Events<
            crate::layer1::administration::sentient_bureaucracy::TaskAdministrativelyOptimizedEvent,
        >>();
        world.init_resource::<Events<crate::layer1::gravity_funerals::SunBurialRequestEvent>>();
        world
            .init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>(
            );
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::justice::CrimeCommittedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::embassy::ArrestEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::embassy::DiplomaticIncidentEvent>>();
        world.insert_resource(crate::layer3::economy::market_shock::MarketShockMarket {
            luxury_price: 10.0,
        });
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(
            crate::layer3::diplomacy::system_sovereignty::FactionRelations::default(),
        );

        world.init_resource::<bevy::prelude::Events<crate::layer1::pop_memories::FamineEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        *world.resource_mut::<GameState>() = GameState::Running;

        for _ in 0..ticks {
            crate::simulation::run_simulation_tick(&mut world);
            if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>() { world.init_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>(); }
            if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>() { world.init_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>(); }
            if !world.contains_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>() { world.init_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>(); }
            if !world.contains_resource::<crate::layer1::nature::atmosphere::SmogGrid>() {
                world.init_resource::<crate::layer1::nature::atmosphere::SmogGrid>();
            }
        }

        world
    }

    #[test]
    fn test_setup_world_default_scenario_is_classic() {
        let config = SetupConfig::default();
        assert_eq!(config.scenario, StartScenarioId::Classic);
    }

    #[test]
    fn test_setup_world_records_selected_scenario() {
        let world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::GroundSurvival,
        });

        let active = world.resource::<ActiveStartScenario>();
        assert_eq!(active.id, StartScenarioId::GroundSurvival);
    }

    #[test]
    fn test_built_in_start_scenarios_have_definitions() {
        for id in StartScenarioId::all() {
            let definition = start_scenario_definition(id);
            assert_eq!(definition.id, id);
        }
    }

    // Spec 1385: Layer 2 Ready starts with a DroneHub and a fleet of 3.
    #[test]
    fn test_layer2_ready_starts_with_drone_hub_and_fleet() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let hub = world
            .query::<(Entity, &Building, &crate::layer1::map::GridPosition)>()
            .iter(&world)
            .find(|(_, building, _)| building.building_type == BuildingType::DroneHub)
            .map(|(entity, _, pos)| (entity, *pos));
        let (hub_entity, hub_pos) = hub.expect("Layer 2 Ready should grant a DroneHub");

        // The fleet count is measured through the Drone query as well,
        // so a drone missing its link/battery cannot hide.
        let drone_count = world
            .query::<&crate::layer1::drone::Drone>()
            .iter(&world)
            .count();
        assert_eq!(drone_count, 3, "Layer 2 Ready fleet should be 3 drones");
        let drones: Vec<(
            Entity,
            crate::layer1::drone::DroneBattery,
            crate::layer1::map::GridPosition,
        )> = world
            .query::<(
                &crate::layer1::drone::ConnectedTo,
                &crate::layer1::drone::DroneBattery,
                &crate::layer1::map::GridPosition,
            )>()
            .iter(&world)
            .map(|(connected, battery, pos)| (connected.0, *battery, *pos))
            .collect();
        assert_eq!(drones.len(), 3);
        for (connected_to, battery, pos) in &drones {
            assert_eq!(*connected_to, hub_entity);
            assert!((battery.current - 100.0).abs() < f32::EPSILON);
            assert_eq!(*pos, hub_pos);
        }

        // No scenario leakage: Classic starts with no hub and no drones.
        let mut classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let classic_hubs = classic
            .query::<&Building>()
            .iter(&classic)
            .filter(|building| building.building_type == BuildingType::DroneHub)
            .count();
        let classic_drones = classic
            .query::<&crate::layer1::drone::Drone>()
            .iter(&classic)
            .count();
        assert_eq!(classic_hubs, 0);
        assert_eq!(classic_drones, 0);
    }

    // Spec 1385 schedule-level: in a real Layer 2 Ready simulation, a
    // drone takes a Repair assignment and a damaged LifeSupport ends
    // with more HP than it started the run with.
    #[test]
    fn test_layer2_ready_drone_repairs_damaged_lifesupport_in_real_schedule() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });
        // Same defensive resource inits as run_start_scenario_smoke_ticks.
        world.init_resource::<crate::layer1::culture::celestial_cemeteries::OrbitalCemetery>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::nature::ecology::HarvestEvent>>();
        world.init_resource::<Events<crate::layer1::culture::celestial_cemeteries::ClearCemeteryEvent>>();
        world.init_resource::<Events<crate::layer1::architecture::smart_matter::RaidEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtInheritedEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::economy::debt_of_the_dead::DebtSocializedEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::heroic_acts::TriggerHeroicAct>>();
        world.init_resource::<Events<crate::layer1::culture::memorial_revolt::PetDeathEvent>>();
        world.init_resource::<Events<crate::layer1::social::pets::PetDeathEvent>>();
        world.init_resource::<bevy_ecs::event::Events<crate::layer1::administration::sentient_bureaucracy::TaskAdministrativelyOptimizedEvent>>();
        world.init_resource::<Events<crate::layer1::gravity_funerals::SunBurialRequestEvent>>();
        world.init_resource::<crate::layer1::environment::bio_acoustic_miasma::MiasmaRecordedSecret>();
        world.init_resource::<Events<crate::layer1::economy::existential_audit::ExistentialAuditCompletedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::skyhooks::LaunchIntent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::justice::CrimeCommittedEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::embassy::ArrestEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer1::law::embassy::DiplomaticIncidentEvent>>();
        world.insert_resource(crate::layer3::economy::market_shock::MarketShockMarket {
            luxury_price: 10.0,
        });
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::ColonyStatus {
            is_sovereign: false,
            overlord_id: Some(1),
        });
        world.insert_resource(crate::layer3::diplomacy::system_sovereignty::FactionRelations::default());
        world.init_resource::<bevy::prelude::Events<crate::layer1::pop_memories::FamineEvent>>();
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        *world.resource_mut::<GameState>() = GameState::Running;

        // Damage the northernmost LifeSupport (the L2R redundant unit)
        // to half HP.
        let target = world
            .query::<(Entity, &Building, &crate::layer1::map::GridPosition)>()
            .iter(&world)
            .filter(|(_, building, _)| building.building_type == BuildingType::LifeSupport)
            .max_by_key(|(_, _, pos)| pos.y)
            .map(|(entity, _, _)| entity)
            .expect("Layer 2 Ready should have LifeSupport units");
        let damaged_hp = {
            let mut structure = world
                .get_mut::<crate::layer1::structure::Structure>(target)
                .expect("LifeSupport should carry a Structure");
            structure.current_hp = structure.max_hp * 0.5;
            structure.current_hp
        };

        let mut repair_seen = false;
        for _ in 0..300 {
            crate::simulation::run_simulation_tick(&mut world);
            if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>() { world.init_resource::<Events<crate::layer2::communications::signal_decay::RawCommsMessageEvent>>(); }
            if !world.contains_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>() { world.init_resource::<Events<crate::layer2::communications::signal_decay::CommsMessageEvent>>(); }
            if !world.contains_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>() { world.init_resource::<Events<crate::layer1::tech::teleporter::psychosis::TeleportEvent>>(); }
            if !world.contains_resource::<crate::layer1::nature::atmosphere::SmogGrid>() {
                world.init_resource::<crate::layer1::nature::atmosphere::SmogGrid>();
            }
            let any_repairing = world
                .query_filtered::<&crate::layer1::utility_ai::PopAction, With<crate::layer1::drone::Drone>>()
                .iter(&world)
                .any(|action| action.current == crate::layer1::utility_ai::ActionType::Repair);
            repair_seen |= any_repairing;
            if repair_seen
                && world
                    .get::<crate::layer1::structure::Structure>(target)
                    .is_some_and(|s| s.current_hp >= s.max_hp)
            {
                break;
            }
        }

        assert!(
            repair_seen,
            "a Layer 2 Ready drone should hold a Repair assignment while a LifeSupport is damaged"
        );
        let final_hp = world
            .get::<crate::layer1::structure::Structure>(target)
            .expect("damaged LifeSupport should still exist")
            .current_hp;
        assert!(
            final_hp > damaged_hp,
            "LifeSupport HP should recover from {damaged_hp}, got {final_hp}"
        );
    }

    #[test]
    fn test_ground_survival_keeps_shared_starter_shell_layout() {
        let mut classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let mut ground_survival = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::GroundSurvival,
        });

        assert_eq!(
            starter_shell(&mut classic),
            starter_shell(&mut ground_survival)
        );
    }

    #[test]
    fn test_ground_survival_reduces_opening_population_and_supplies() {
        let mut classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let mut ground_survival = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::GroundSurvival,
        });

        let classic_pop_count = classic.query::<&Pop>().iter(&classic).count();
        let ground_survival_pop_count = ground_survival
            .query::<&Pop>()
            .iter(&ground_survival)
            .count();
        let classic_resources = *classic.resource::<ColonyResources>();
        let ground_survival_resources = *ground_survival.resource::<ColonyResources>();

        assert!(
            ground_survival_pop_count < classic_pop_count,
            "Ground Survival should start with fewer pops than Classic"
        );
        assert!(
            ground_survival_resources.food < classic_resources.food,
            "Ground Survival should start with less food than Classic"
        );
        assert!(
            ground_survival_resources.tools < classic_resources.tools,
            "Ground Survival should start with fewer tools than Classic"
        );
    }

    #[test]
    fn test_ground_survival_damages_starter_life_support() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::GroundSurvival,
        });

        let life_support = life_support_structure(&mut world);
        assert!(
            life_support.current_hp < life_support.max_hp,
            "Ground Survival should start with damaged life support"
        );
    }

    #[test]
    fn test_ground_survival_adds_distinct_intro_chronicle_text() {
        let world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::GroundSurvival,
        });

        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|event| event.text.contains("hard landing")),
            "Ground Survival should add a distinct startup chronicle entry"
        );
    }

    #[test]
    fn test_social_drama_rebalances_founders_and_immigrants() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::SocialDrama,
        });

        assert_eq!(generation_counts(&mut world), (1, 4));
    }

    #[test]
    fn test_social_drama_is_materially_stable() {
        let classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let social_drama = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::SocialDrama,
        });

        let classic_resources = *classic.resource::<ColonyResources>();
        let social_resources = *social_drama.resource::<ColonyResources>();

        assert!(
            social_resources.food >= classic_resources.food,
            "Social Drama should not start with less food than Classic"
        );
        assert!(
            social_resources.tools >= classic_resources.tools,
            "Social Drama should not start with fewer tools than Classic"
        );
    }

    #[test]
    fn test_social_drama_triggers_old_guard_tension_on_startup() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::SocialDrama,
        });

        let demographics = world.resource::<crate::layer1::social::old_guard::Demographics>();
        assert_eq!(demographics.founders, 1);
        assert_eq!(demographics.immigrants, 4);
        assert!(
            demographics.has_triggered_turning_point,
            "Social Drama should trigger the turning point immediately"
        );

        let founder_pressure = world
            .query::<(&Generation, &MoodModifiers)>()
            .iter(&world)
            .filter(|(generation, modifiers)| {
                **generation == Generation::Founder
                    && modifiers
                        .entries
                        .iter()
                        .any(|entry| entry.source == "Overwhelmed by Strangers")
            })
            .count();
        assert!(
            founder_pressure > 0,
            "Founders should feel the immigrant-majority pressure at startup"
        );
    }

    #[test]
    fn test_social_drama_adds_distinct_intro_chronicle_text() {
        let world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::SocialDrama,
        });

        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|event| event.text.contains("powder keg")),
            "Social Drama should add a distinct startup chronicle entry"
        );
    }

    #[test]
    fn test_layer2_ready_keeps_shared_starter_shell_layout() {
        let mut classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let mut layer2_ready = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        assert_eq!(
            starter_shell(&mut classic),
            starter_shell(&mut layer2_ready)
        );
    }

    #[test]
    fn test_layer2_ready_is_materially_stronger_and_unlocks_system_view() {
        let classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let layer2_ready = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let classic_resources = *classic.resource::<ColonyResources>();
        let layer2_resources = *layer2_ready.resource::<ColonyResources>();

        assert!(
            layer2_resources.food > classic_resources.food,
            "Layer 2 Ready should start with more food than Classic"
        );
        assert!(
            layer2_resources.tools > classic_resources.tools,
            "Layer 2 Ready should start with more tools than Classic"
        );
        assert_eq!(
            *layer2_ready.resource::<crate::layer2::visibility::SystemVisibility>(),
            crate::layer2::visibility::SystemVisibility::Full,
            "Layer 2 Ready should unlock system visibility immediately"
        );
    }

    #[test]
    fn test_layer2_ready_starts_with_powered_command_center() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let mut command_center_count = 0;
        let mut command_center_pos = None;
        let mut command_center_active = false;
        for (building, pos, power) in world
            .query::<(
                &Building,
                &GridPosition,
                &crate::layer1::energy::PowerConsumer,
            )>()
            .iter(&world)
        {
            if building.building_type == BuildingType::CommandCenter {
                command_center_count += 1;
                command_center_pos = Some(*pos);
                command_center_active = power.active;
            }
        }

        assert_eq!(command_center_count, 1);
        assert!(
            command_center_active,
            "Layer 2 Ready command center should be powered"
        );

        let command_center_pos = command_center_pos.expect("command center should exist");
        let pop_on_command_center = world
            .query::<(&Pop, &GridPosition)>()
            .iter(&world)
            .any(|(_, pos)| *pos == command_center_pos);
        assert!(
            !pop_on_command_center,
            "Layer 2 Ready should not leave a starter pop standing on the command center tile"
        );
    }

    #[test]
    fn test_starter_life_support_is_powered_from_tick_zero() {
        // Item 77 Phase 2 (forensics bug #1): the starter LifeSupport used
        // to be its own power grid (production 0, consumer permanently
        // inactive). The starter PowerPole bridges Lander -> LifeSupport,
        // so the grid containing the LifeSupport must now include the
        // Lander's production in every scenario whose center tile carries
        // the pole. (Per-tick activation is the brownout mechanic's
        // business: every building spawns as a Prototype at 0.5
        // efficiency, so the Lander's 10 output counts as 5 against the
        // LifeSupport's 10 demand.)
        for scenario in [
            StartScenarioId::Classic,
            StartScenarioId::GroundSurvival,
            StartScenarioId::SocialDrama,
        ] {
            let mut world = setup_world_with_config(SetupConfig {
                headless: true,
                scenario,
            });
            let pole_count = world
                .query::<&Building>()
                .iter(&world)
                .filter(|b| b.building_type == BuildingType::PowerPole)
                .count();
            assert_eq!(
                pole_count, 1,
                "scenario {scenario:?}: starter layout should ship one PowerPole bridge"
            );
            let life_support = world
                .query::<(Entity, &Building)>()
                .iter(&world)
                .find(|(_, b)| b.building_type == BuildingType::LifeSupport)
                .map(|(e, _)| e)
                .expect("starter colony has a LifeSupport");
            let (production, demand) =
                crate::layer1::energy::calculate_grid_stats(&mut world, life_support);
            assert_eq!(
                production, 5.0,
                "scenario {scenario:?}: Lander production should reach the LifeSupport grid through the pole"
            );
            assert_eq!(
                demand, 10.0,
                "scenario {scenario:?}: LifeSupport demand should be on the bridged grid"
            );
        }
    }

    #[test]
    fn test_layer2_ready_has_redundant_powered_atmosphere() {
        // Item 77 Phase 2, Layer 2 Ready maintenance lane: a redundant
        // second LifeSupport plus a Generator on the completed power
        // chain, and metal earmarked for a spare LifeSupport.
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let life_support_count = world
            .query::<&Building>()
            .iter(&world)
            .filter(|b| b.building_type == BuildingType::LifeSupport)
            .count();
        assert_eq!(
            life_support_count, 2,
            "Layer 2 Ready should start with a redundant second LifeSupport"
        );
        let generator_count = world
            .query::<&Building>()
            .iter(&world)
            .filter(|b| b.building_type == BuildingType::Generator)
            .count();
        assert_eq!(generator_count, 1, "Layer 2 Ready should place a Generator");
        // Both LifeSupport units and the Generator share the
        // CommandCenter grid with the Lander, so production reaching a
        // LifeSupport is at least Lander (5 after Prototype efficiency) +
        // Generator (5) = 10 — worldgen structures may add more. (The
        // CommandCenter's own 50 demand keeps
        // the grid in trunk's brownout regime; redundancy plus doubled
        // production is what the lane buys.)
        let life_support = world
            .query::<(Entity, &Building)>()
            .iter(&world)
            .find(|(_, b)| b.building_type == BuildingType::LifeSupport)
            .map(|(e, _)| e)
            .expect("Layer 2 Ready has LifeSupport units");
        let (production, _) =
            crate::layer1::energy::calculate_grid_stats(&mut world, life_support);
        assert!(
            production >= 10.0,
            "Layer 2 Ready LifeSupport grid should include Lander + Generator production, got {production}"
        );
        let metal = world.resource::<ColonyResources>().metal;
        assert!(
            metal >= 75.0,
            "Layer 2 Ready metal should include the 50-metal spare-LifeSupport earmark, got {metal}"
        );
    }

    #[test]
    fn test_layer2_ready_second_life_support_is_decluttered_from_heart() {
        // Item 110 Gap 3: the redundant LifeSupport used to sit inside
        // the 3x3 starter heart (offset (0, 1)), where one ignition was
        // adjacent to both units — the l2r2k_11 cascade killed both
        // ~11 ticks apart. The annex unit must sit >= 3 tiles (Chebyshev)
        // from the primary unit and stay on the live power grid.
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let positions: Vec<GridPosition> = world
            .query::<(&Building, &GridPosition)>()
            .iter(&world)
            .filter(|(b, _)| b.building_type == BuildingType::LifeSupport)
            .map(|(_, pos)| *pos)
            .collect();
        assert_eq!(positions.len(), 2, "Layer 2 Ready keeps two LifeSupports");
        let separation = positions[0].distance_chebyshev(positions[1]);
        assert!(
            separation >= 3,
            "the two LifeSupports must not share one fire neighborhood, separation is {separation}"
        );

        // The far (annex) unit must be on the same powered grid as the
        // heart: grid stats computed from its own entity must still see
        // the Lander + Generator production through the tunnel poles.
        let annex = world
            .query::<(Entity, &Building, &GridPosition)>()
            .iter(&world)
            .filter(|(_, b, _)| b.building_type == BuildingType::LifeSupport)
            .max_by_key(|(_, _, pos)| pos.x)
            .map(|(e, _, _)| e)
            .expect("annex LifeSupport exists");
        let (production, _) =
            crate::layer1::energy::calculate_grid_stats(&mut world, annex);
        assert!(
            production >= 10.0,
            "annex LifeSupport must share the heart grid (Lander + Generator), got {production}"
        );
    }

    #[test]
    fn test_layer2_ready_adds_distinct_intro_chronicle_text() {
        let world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let chronicle = world.resource::<Chronicle>();
        assert!(
            chronicle
                .events
                .iter()
                .any(|event| event.text.contains("orbital charter")),
            "Layer 2 Ready should add a distinct startup chronicle entry"
        );
    }

    // Layer 2 legibility pass: alongside the flavor line, tick 0 must
    // orient a Layer 2 Ready keeper to the concrete kit they were just
    // handed — the autonomous drone crew, the suit pool, the redundant
    // Life Support — because nothing else in the game names it.
    #[test]
    fn test_layer2_ready_intro_names_drone_crew_and_kit() {
        let world = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Layer2Ready,
        });

        let chronicle = world.resource::<Chronicle>();
        let orientation = chronicle
            .events
            .iter()
            .find(|event| event.text.contains("three drones"))
            .expect("Layer 2 Ready should name its drone crew in a tick-0 chronicle entry");
        assert_eq!(orientation.tick, 0, "orientation must land at tick 0");
        assert_eq!(
            orientation.importance,
            crate::layer1::chronicle::EventImportance::Major
        );
        assert!(
            orientation.text.contains("hub"),
            "orientation should point at the drone hub"
        );
        assert!(
            orientation.text.contains("suits"),
            "orientation should name the suit pool"
        );
        assert!(
            orientation.text.contains("Life Support"),
            "orientation should name the redundant Life Support"
        );

        // No scenario leakage: Classic founders get no drone orientation.
        let classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let classic_chronicle = classic.resource::<Chronicle>();
        assert!(
            !classic_chronicle
                .events
                .iter()
                .any(|event| event.text.contains("three drones")),
            "Classic must not receive the Layer 2 Ready drone orientation"
        );
    }

    #[test]
    fn test_curated_start_scenarios_share_first_pass_starter_shell() {
        let mut classic = setup_world_with_config(SetupConfig {
            headless: true,
            scenario: StartScenarioId::Classic,
        });
        let classic_shell = starter_shell(&mut classic);

        for scenario in [
            StartScenarioId::GroundSurvival,
            StartScenarioId::SocialDrama,
            StartScenarioId::Layer2Ready,
        ] {
            let mut world = setup_world_with_config(SetupConfig {
                headless: true,
                scenario,
            });
            assert_eq!(
                starter_shell(&mut world),
                classic_shell,
                "{scenario:?} should keep the shared starter shell"
            );
        }
    }

    #[test]
    fn test_curated_start_scenarios_have_distinct_mechanical_signatures() {
        let classic = scenario_signature(StartScenarioId::Classic);
        let ground_survival = scenario_signature(StartScenarioId::GroundSurvival);
        let social_drama = scenario_signature(StartScenarioId::SocialDrama);
        let layer2_ready = scenario_signature(StartScenarioId::Layer2Ready);

        assert_ne!(classic, ground_survival);
        assert_ne!(classic, social_drama);
        assert_ne!(classic, layer2_ready);
        assert_ne!(ground_survival, social_drama);
        assert_ne!(ground_survival, layer2_ready);
        assert_ne!(social_drama, layer2_ready);

        assert!(
            ground_survival.food_tenths < classic.food_tenths,
            "Ground Survival should be leaner than Classic"
        );
        assert!(
            layer2_ready.food_tenths > classic.food_tenths,
            "Layer 2 Ready should be richer than Classic"
        );
        assert!(
            ground_survival.pop_count < social_drama.pop_count,
            "Ground Survival should open with fewer pops than Social Drama"
        );
        assert!(
            social_drama.immigrants > social_drama.founders,
            "Social Drama should start immigrant-heavy"
        );
        assert!(
            ground_survival.life_support_damaged,
            "Ground Survival should carry a damaged survival core"
        );
        assert_eq!(
            layer2_ready.visibility,
            crate::layer2::visibility::SystemVisibility::Full,
            "Layer 2 Ready should expose the system immediately"
        );
        assert_eq!(
            layer2_ready.command_center_count, 1,
            "Layer 2 Ready should start with a command center"
        );
    }

    #[test]
    fn test_built_in_start_scenarios_survive_early_headless_ticks() {
        for scenario in StartScenarioId::all() {
            let world = run_start_scenario_smoke_ticks(scenario, 5);
            assert_eq!(
                world.resource::<SimulationTime>().tick,
                5,
                "{scenario:?} should survive five early ticks"
            );
        }
    }

    #[test]
    fn test_setup_world_creates_resources() {
        let world = setup_world();

        assert!(world.contains_resource::<GameState>());
        assert!(world.contains_resource::<SimulationTime>());
        assert!(world.contains_resource::<TerrainGrid>());
        assert!(world.contains_resource::<ColonyResources>());
        assert!(world.contains_resource::<Chronicle>());
        assert!(world.contains_resource::<Selection>());
        assert!(world.contains_resource::<NotificationQueue>());
        assert!(world.contains_resource::<RenderCache>());
        assert!(world.contains_resource::<UtilityConfig>());
        assert!(world.contains_resource::<SeasonState>());
        assert!(world.contains_resource::<NamedLocations>());
        assert!(world.contains_resource::<TechState>());
        assert!(world.contains_resource::<NarrativeGenerator>());
        assert!(world.contains_resource::<ColonyName>());
    }

    #[test]
    fn test_setup_world_spawns_pops() {
        let mut world = setup_world();
        world.init_resource::<bevy::prelude::Events<crate::layer2::weather::StormImpactEvent>>();
        let pop_count = world.query::<&Pop>().iter(&world).count();
        assert!(pop_count > 0, "Should have spawned initial pops");
    }

    #[test]
    fn test_setup_world_has_chronicle_event() {
        let world = setup_world();
        let chronicle = world.resource::<Chronicle>();
        assert!(
            !chronicle.events.is_empty(),
            "Should have initial chronicle event"
        );
    }

    #[test]
    fn test_setup_world_default_state_is_main_menu() {
        let world = setup_world();
        assert_eq!(*world.resource::<GameState>(), GameState::MainMenu);
    }

    #[test]
    fn test_setup_world_creates_factions() {
        let world = setup_world();
        assert!(world.contains_resource::<crate::layer1::factions::Factions>());
    }

    #[test]
    fn test_setup_world_generates_system() {
        use crate::layer2::generation::{Planet, Star};
        let mut world = setup_world();

        let stars = world.query::<&Star>().iter(&world).count();
        assert_eq!(stars, 1, "Should generate exactly one star");

        let planets = world.query::<&Planet>().iter(&world).count();
        assert!(planets >= 3, "Should generate at least 3 planets");
    }

    #[test]
    fn test_starting_pops_spawn_in_breathable_air() {
        // Regression test for the player-reported "mass extinction on tick 5":
        // starting pops used to begin with NO oxygen and suffocate within
        // seconds. Every scenario must spawn pops on tiles at or above the
        // pressure_damage_system suffocation threshold (0.2), with a
        // LifeSupport unit present to maintain the air.
        for scenario in [
            StartScenarioId::Classic,
            StartScenarioId::GroundSurvival,
            StartScenarioId::SocialDrama,
            StartScenarioId::Layer2Ready,
        ] {
            let mut world = setup_world_with_config(SetupConfig {
                headless: true,
                scenario,
            });
            // Collect positions first: the query borrows world mutably, so the
            // pressure grid borrow has to come after.
            let positions: Vec<GridPosition> = world
                .query_filtered::<&GridPosition, With<crate::layer1::pop::Pop>>()
                .iter(&world)
                .copied()
                .collect();
            assert!(
                !positions.is_empty(),
                "scenario {scenario:?} should spawn starting pops"
            );
            let grid = world.resource::<crate::layer1::pressure::PressureGrid>();
            for pos in &positions {
                let pressure = grid.get(pos.x, pos.y);
                assert!(
                    pressure >= 0.2,
                    "scenario {scenario:?}: pop at ({}, {}) starts at pressure {pressure:.2}, below the 0.2 suffocation threshold",
                    pos.x,
                    pos.y,
                );
            }
            let life_support = world
                .query::<&Building>()
                .iter(&world)
                .filter(|b| b.building_type == BuildingType::LifeSupport)
                .count();
            assert!(
                life_support >= 1,
                "scenario {scenario:?}: colony starts with no LifeSupport unit"
            );
        }
    }

    #[test]
    fn test_emergency_oxygen_fallback_seeds_pressure_and_life_support() {
        // The no-habitat fallback (spawn_initial_pops scatter) must not leave
        // pops in vacuum either.
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        // Despawn any existing pops + life support to isolate the fallback.
        let pops: Vec<bevy_ecs::prelude::Entity> =
            world.query_filtered::<bevy_ecs::prelude::Entity, With<Pop>>().iter(&world).collect();
        for entity in pops {
            world.despawn(entity);
        }
        let units: Vec<bevy_ecs::prelude::Entity> = world
            .query_filtered::<bevy_ecs::prelude::Entity, With<Building>>()
            .iter(&world)
            .collect();
        for entity in units {
            world.despawn(entity);
        }

        crate::layer1::entities::pop::spawn_initial_pops(&mut world);
        seed_emergency_oxygen_for_pops(&mut world);

        let positions: Vec<GridPosition> = world
            .query_filtered::<&GridPosition, With<crate::layer1::pop::Pop>>()
            .iter(&world)
            .copied()
            .collect();
        assert!(!positions.is_empty(), "fallback should spawn pops");
        let grid = world.resource::<crate::layer1::pressure::PressureGrid>();
        for pos in &positions {
            assert!(
                grid.get(pos.x, pos.y) >= 0.2,
                "fallback pop at ({}, {}) has no oxygen",
                pos.x,
                pos.y,
            );
        }
        let life_support = world
            .query::<&Building>()
            .iter(&world)
            .filter(|b| b.building_type == BuildingType::LifeSupport)
            .count();
        assert!(
            life_support >= 1,
            "fallback should place at least one LifeSupport unit"
        );
    }
}


