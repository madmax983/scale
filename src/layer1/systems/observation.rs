use super::Layer1SystemSet;
use crate::layer1::direct_link::clear_input_system;
use crate::layer1::inspector::{inspector_report_system, observe_inspector_system};
use crate::layer1::social::old_guard::check_generational_friction_system;
use crate::layer1::*;
use bevy_ecs::prelude::*;

#[allow(clippy::too_many_lines)]
pub fn register(schedule: &mut Schedule) {
    schedule.add_systems(
        (
            crate::layer1::core::integration::bureaucratic_strike_chronicle_bridge,
            crate::layer1::core::integration::sentient_bureaucracy_chronicle_bridge,
            crate::layer1::architecture::gravity_engineering::gravity_engineering_chronicle_bridge,
            crate::layer1::core::integration::kinetic_strike_chronicle_bridge,
            crate::layer1::actions::escape::escape_pods_chronicle_bridge,
            crate::layer1::core::integration::subconscious_grid_lockdown_chronicle_bridge,
            crate::layer1::core::integration::ego_stat_chronicle_bridge,
            crate::layer1::core::integration::consultant_chronicle_bridge,
            crate::layer1::mycelial::detect_tripwire_step,
            crate::layer1::mycelial::aggro_network_entities,
            crate::layer1::social::subversion::run_subversion_spread_system,
            crate::layer1::social::subversion::run_surveillance_morale_system,
        )
            .in_set(super::Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            crate::layer1::memorial_economy::process_pop_deaths_for_relics,
            crate::layer1::memorial_economy::apply_memorial_morale_boost,
            crate::layer1::social::pop_relationships::update_workplace_relationships_system,
            crate::layer1::social::emotional_contagion::contagion_system,
            crate::layer1::psychology::generational_amnesia::apply_amnesia_system,
            crate::layer1::psychology::generational_amnesia::apply_amnesia_to_memories_system,
            crate::layer1::social::emotional_contagion::trigger_emotional_contagion_system,
            crate::layer1::social::pop_relationships::calculate_relationship_mood_buff_system,
            crate::layer1::culture::cultural_artifacts::cultural_aura_system,
            crate::layer1::stress::assign_generational_traits_system,
            crate::layer1::stress::silent_needs_suppression_system,
            crate::layer1::psychology::psychic_stains::process_violent_deaths_system,
            crate::layer1::psychology::psychic_stains::apply_stain_stress_system,
            crate::layer1::core::integration::psionic_fire_bridge_system,
            crate::layer1::core::integration::track_negative_events_bridge_system,
            crate::layer1::architecture_superstition::evaluate_architectural_superstition,
            crate::layer1::architecture_sentience::architectural_union_trigger_system,
            crate::layer1::architecture_sentience::sentient_architecture_strike_system,
            crate::layer1::architecture_sentience::architectural_sentience_chronicle_bridge,
            crate::layer1::architecture_superstition::apply_cursed_penalties,
        )
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            crate::layer1::core::integration::cryo_prison_sabotage_bridge_system,
            crate::layer1::core::integration::xenoflora_pet_death_bridge_system,
            crate::layer1::cryo_prison::thaw_cryo_pod_system,
            crate::layer1::cryo_prison::criminal_sabotage_system,
            crate::layer1::psychology::psychic_stains::decay_stains_system,
            crate::layer1::law::aesthetic_edict::evaluate_aesthetic_edict_system,
            crate::layer1::integration::diplomatic_reflection_kill_bridge,
            crate::layer1::integration::diplomatic_reflection_plant_bridge,
            crate::layer1::resources::waste_scent_bridge,
            crate::layer1::core::integration::flora_scent_bridge_system,
            crate::layer1::core::integration::digital_immortality_chronicle_bridge,
            crate::layer1::core::integration::visitor_chronicle_bridge,
            crate::layer1::psychology::simulacrum::simulacrum_broadcast_system,
            crate::layer1::core::integration::simulacrum_chronicle_bridge,
            crate::layer1::olfactory::scent_diffusion_system
                .after(crate::layer1::resources::waste_scent_bridge)
                .after(crate::layer1::core::integration::flora_scent_bridge_system),
            crate::layer1::olfactory::scent_mood_system
                .after(crate::layer1::olfactory::scent_diffusion_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    #[cfg(feature = "nova")]
    schedule.add_systems(
        (
            crate::experimental::chrono_stutter::spawn_chrono_anomaly_system
                .after(decay_needs_system),
            crate::experimental::chrono_stutter::apply_chrono_stutter_system
                .after(crate::experimental::chrono_stutter::spawn_chrono_anomaly_system),
            crate::experimental::cartography_export::map_export_system,
            crate::experimental::paranoia_network::paranoia_network_system,
            crate::experimental::epigenetic_stress::epigenetic_mutation_system,
            crate::experimental::cursed_artifacts::spawn_cursed_anomalies_system,
            crate::experimental::cursed_artifacts::apply_curse_system,
            crate::experimental::empathy_cascade::init_empathic_health_tracker_system,
            crate::experimental::empathy_cascade::empathy_cascade_system
                .after(crate::experimental::empathy_cascade::init_empathic_health_tracker_system),
            crate::experimental::symbiotic_parasite::symbiotic_parasite_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            biography_monitor_system.after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::social::cadet::death_consequence_system,
            crate::layer1::social::golden_age::complacency_accumulation_system,
            crate::layer1::social::golden_age::apply_complacency_debuffs_system,
            crate::layer1::social::golden_age::pop_alert_response_system,
            crate::layer1::graffiti::graffiti_placement_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            dream_system.after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::psychology::cryo_dreams::cryo_dream_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            cleanup_dream_marker_system.after(dream_system),
            #[cfg(feature = "nova")]
            crate::experimental::dream_economy::harvest_dreams_system
                .after(crate::layer1::dreams::dream_system),
            #[cfg(feature = "nova")]
            crate::experimental::dream_economy::nightmare_paranoia_system,
            #[cfg(feature = "nova")]
            crate::ui::selection::observer_awareness_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            #[cfg(feature = "nova")]
            crate::layer1::machine_consciousness::machine_personality_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::psychology::void_stare::update_void_exposure_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::psychology::void_stare::void_manifestation_system
                .after(crate::layer1::psychology::void_stare::update_void_exposure_system),
            crate::layer1::psychology::void_stare::apply_void_stare_stress_system
                .after(crate::layer1::psychology::void_stare::update_void_exposure_system),
            crate::layer1::core::integration::void_stare_chronicle_bridge
                .after(crate::layer1::psychology::void_stare::void_manifestation_system),
            crate::layer1::integration::scapegoat_chronicle_bridge,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            check_milestones_system.after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::culture::nostalgia::nostalgia_trigger_system,
            crate::layer1::core::integration::nostalgia_rumor_generation_bridge
                .after(crate::layer1::culture::nostalgia::nostalgia_trigger_system),
            crate::layer1::rumor::generate_rumor_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::rumor::exchange_rumors_system,
            crate::layer1::culture::nostalgia::nostalgia_spread_system
                .after(crate::layer1::rumor::exchange_rumors_system)
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::social::society::form_societies_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::social::society::society_meeting_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::social::society::investigation_handler_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::social::society::suppression_handler_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::funeral::grief_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::gravity_funerals::process_high_status_deaths_system,
            crate::layer1::gravity_funerals::evaluate_sun_burial_requests_system,
            crate::layer1::ancestral_graves::grave_visit_system
                .after(crate::layer1::funeral::grief_system),
            crate::experimental::toxic_personalities::toxic_personalities_system
                .after(crate::layer1::needs::decay_needs_system),
            crate::layer1::unrest::calculate_unrest_system.after(decay_needs_system),
            crate::layer1::environment::bio_acoustic::bio_acoustic_chorus_system
                .after(crate::layer1::unrest::calculate_unrest_system),
            crate::layer1::environment::bio_acoustic_miasma::record_miasma_secret
                .after(crate::layer1::unrest::calculate_unrest_system),
            crate::layer1::environment::bio_acoustic_miasma::broadcast_miasma_secrets
                .after(crate::layer1::environment::bio_acoustic_miasma::record_miasma_secret),
            crate::layer1::integration::paranoia_stress_bridge_system
                .after(crate::layer1::environment::bio_acoustic_miasma::broadcast_miasma_secrets)
                .before(crate::layer1::stress::check_stress_breakdown_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    // Spec 362 The Nostalgia Plague: separate add_systems — the tuple
    // above is at Bevy's IntoSystemConfigs arity limit (flesh-famine
    // lesson). Exposure accrues from relics/ruins/artifacts and carrier
    // pops; the reality anchor cures.
    schedule.add_systems(
        (
            crate::layer1::social::nostalgia_plague::nostalgia_exposure_system,
            crate::layer1::social::nostalgia_plague::reality_anchor_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    // Spec 343: The Rumor Web — TTL decay, adjacency spread, panic reaction.
    // Separate block: the rumor block above is at the tuple-arity limit.
    schedule.add_systems(
        (
            crate::layer1::rumor::decay_rumors_system,
            crate::layer1::rumor::spread_rumors_system
                .after(crate::layer1::rumor::generate_rumor_system)
                .after(crate::layer1::rumor::exchange_rumors_system),
            crate::layer1::rumor::react_to_rumors_system
                .after(crate::layer1::rumor::spread_rumors_system)
                .before(crate::layer1::stress::check_stress_breakdown_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::unrest::identify_scapegoat_system
                .after(crate::layer1::unrest::calculate_unrest_system),
            crate::layer1::unrest::handle_denounce_event_system
                .after(crate::layer1::unrest::calculate_unrest_system),
            crate::layer1::social::ghost_shift_strike::evaluate_ghost_shifts
                .after(crate::layer1::unrest::calculate_unrest_system),
            crate::layer1::psychology::memory_forgery::mnestic_archiver_system
                .after(decay_needs_system),
            crate::layer1::psychology::memory_forgery::truth_outbreak_system
                .after(decay_needs_system),
            crate::layer1::unrest::check_mental_break_system.after(decay_needs_system),
            check_stress_breakdown_system.after(decay_needs_system),
            crate::layer1::totems::check_spontaneous_totem_creation
                .after(check_stress_breakdown_system),
            crate::layer1::totems::unequip_totem_system.after(decay_needs_system),
            crate::layer1::administration::edicts::update_policy_tradition_system,
            crate::layer1::administration::edicts::handle_revoke_policy_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::psychology::stress::apply_breakdown_effects_system
                .after(check_stress_breakdown_system),
            update_breakdown_system.after(check_stress_breakdown_system),
            crate::layer1::mind::process_fugue_onset.after(check_stress_breakdown_system),
            crate::layer1::mind::enforce_fugue_job.after(crate::layer1::mind::process_fugue_onset),
            crate::layer1::mind::process_fugue_spread.after(crate::layer1::mind::enforce_fugue_job),
            update_catharsis_duration_system.after(decay_needs_system),
            #[cfg(feature = "nova")]
            crate::layer1::sleep_deprived_savant::fever_dream_system
                .after(crate::layer1::needs::decay_needs_system),
            check_sleepwalking_start_system.after(decay_needs_system),
            crate::layer1::cryo_shock::decay_cryo_shock_system.after(decay_needs_system),
            sleepwalk_end_system.after(decay_needs_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        crate::layer1::law::sanctuary::wanted_pop_migration_scoring_system
            .after(crate::layer1::law::justice::check_crime_system)
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::layer1::law::sanctuary::filter_police_targets_system
            .after(crate::layer1::mind::evaluate_actions_system)
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::law::justice::check_crime_system
                .after(crate::layer1::unrest::check_mental_break_system),
            crate::layer1::law::contraband::detect_contraband_system
                .after(crate::layer1::law::justice::check_crime_system),
            crate::layer1::unrest::recover_mental_break_system.after(decay_needs_system),
            crate::layer1::civic_ideology::apply_ideological_modifiers_system
                .after(crate::layer1::unrest::recover_mental_break_system),
            crate::layer1::civic_ideology::decay_recent_action_system
                .after(crate::layer1::civic_ideology::apply_ideological_modifiers_system),
            check_generational_friction_system.after(decay_needs_system),
            crate::layer1::social::seniority::seniority_demographic_friction_system
                .after(decay_needs_system),
            crate::layer1::social::seniority::seniority_friction_event_system
                .after(crate::layer1::social::seniority::seniority_demographic_friction_system),
            crate::layer1::social::process_generational_dissonance_system.after(decay_needs_system),
            crate::layer1::social::evaluate_safety_edicts_system.after(decay_needs_system),
            crate::layer1::social::architecture_of_paranoia::run_subversion_spread_system,
            crate::layer1::social::architecture_of_paranoia::run_surveillance_morale_system,
            crate::layer1::hobby::assign_hobby_system.after(decay_needs_system),
            crate::layer1::law::predictive_policing::check_prediction_system
                .after(decay_needs_system),
            crate::layer1::social::grievances::post_grievance_system.after(decay_needs_system),
            crate::layer1::social::grievances::read_board_system.after(decay_needs_system),
            crate::layer1::social::cultural_vandalism::vandalism_system.after(decay_needs_system),
            crate::layer1::social::cultural_vandalism::update_structure_buffs,
            crate::layer1::social::cultural_vandalism::evaluate_vandalism_targets
                .after(decay_needs_system)
                .after(crate::layer1::social::cultural_vandalism::vandalism_system),
            crate::layer1::social::indoctrination::process_indoctrination_system
                .after(decay_needs_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    #[cfg(feature = "nova")]
    schedule.add_systems(
        (
            crate::experimental::dreams_of_genesis::dreams_of_genesis_system
                .after(decay_needs_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );
    #[cfg(feature = "nova")]
    schedule.add_systems(
        (
            crate::experimental::the_feral_choir::detect_feral_choir_system
                .after(decay_needs_system),
            crate::experimental::the_feral_choir::dissolve_feral_choir_system
                .after(crate::experimental::the_feral_choir::detect_feral_choir_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            #[cfg(feature = "nova")]
            crate::experimental::the_humming_monolith::detect_and_spawn_monolith_system
                .after(decay_needs_system),
            #[cfg(feature = "nova")]
            crate::experimental::the_humming_monolith::monolith_influence_system
                .after(crate::experimental::the_humming_monolith::detect_and_spawn_monolith_system),
            crate::layer1::integration::update_unmet_luxury_system.after(decay_needs_system),
            crate::layer1::integration::sacrilege_unrest_bridge.after(decay_needs_system),
            crate::layer1::culture::cult_of_first_ship::first_ship_morale_system
                .after(decay_needs_system),
            crate::layer1::culture::cult_of_first_ship::first_ship_destruction_system
                .after(decay_needs_system),
            crate::layer1::core::integration::first_ship_destruction_chronicle_bridge
                .after(crate::layer1::culture::cult_of_first_ship::first_ship_destruction_system),
            crate::layer1::integration::issue_placebo_from_edict_system.after(decay_needs_system),
            crate::layer1::social::placebo::placebo_tick_system.after(decay_needs_system),
            crate::layer1::social::placebo::reveal_betrayal_system.after(decay_needs_system),
            crate::layer1::social::exile::process_banishments.after(decay_needs_system),
            crate::layer1::social::exile::evaluate_exile_returns.after(decay_needs_system),
            crate::layer1::contagion::emotional_contagion_system.after(decay_needs_system),
            crate::layer1::social::debt::accrue_debt_system,
            crate::layer1::social::debt::debt_decay_system,
            crate::layer1::social::debt::debt_impact_system,
            crate::layer1::social::debt::process_life_saved_system,
            crate::layer1::social::debt::process_favors_system,
            crate::layer1::social::debt::evaluate_faction_support_system,
            crate::layer1::social::sentient_standard::apply_sentient_standard_stress_system
                .after(decay_needs_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    #[cfg(feature = "nova")]
    schedule.add_systems(
        (
            crate::experimental::necro_industry::necro_industry_system
                .after(crate::layer1::needs::decay_needs_system),
            crate::experimental::emotional_weather::emotional_weather_system
                .after(crate::layer1::needs::decay_needs_system),
            crate::experimental::synesthesia::synesthesia_system
                .after(crate::layer1::needs::decay_needs_system),
            crate::experimental::fungal_death::fungal_death_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            crate::experimental::bioluminescent_trails::spawn_bioluminescent_trails_system
                .after(crate::layer1::needs::decay_needs_system),
            crate::experimental::cargo_cult_fleet::spawn_cargo_cult_fleet_system,
            crate::experimental::cargo_cult_fleet::feed_cargo_cult_tether_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    #[cfg(feature = "nova")]
    schedule.add_systems(
        (
            crate::experimental::phantom_workforce::spawn_phantom_workers_system,
            crate::experimental::phantom_workforce::phantom_production_system
                .after(crate::experimental::phantom_workforce::spawn_phantom_workers_system),
            crate::experimental::phantom_workforce::phantom_terror_system
                .after(crate::experimental::phantom_workforce::spawn_phantom_workers_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            #[cfg(feature = "nova")]
            crate::experimental::bioluminescent_trails::fade_bioluminescent_trails_system.after(
                crate::experimental::bioluminescent_trails::spawn_bioluminescent_trails_system,
            ),
            #[cfg(feature = "nova")]
            crate::experimental::sympathetic_architecture::sympathetic_architecture_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            #[cfg(feature = "nova")]
            crate::experimental::psychic_resonance::psychic_resonance_system
                .after(crate::layer1::needs::decay_needs_system),
            #[cfg(feature = "nova")]
            crate::experimental::sleep_deprived_savant::sleep_deprived_savant_system,
            #[cfg(feature = "nova")]
            crate::experimental::fever_pitch::fever_pitch_system
                .after(crate::layer1::needs::decay_needs_system),
            #[cfg(feature = "nova")]
            crate::experimental::genetic_memory::absorb_genetic_memory_system
                .after(crate::layer1::health::despawn_dead_entities_system),
            #[cfg(feature = "nova")]
            crate::experimental::genetic_memory::inherit_genetic_memory_system
                .after(crate::experimental::genetic_memory::absorb_genetic_memory_system),
            crate::layer1::psychic::apply_psychic_radiation_system
                .after(crate::layer1::needs::decay_needs_system),
            #[cfg(feature = "nova")]
            crate::experimental::meme_plague::process_meme_contagion
                .after(crate::layer1::needs::decay_needs_system),
            #[cfg(feature = "nova")]
            crate::experimental::meme_plague::apply_meme_effects
                .after(crate::experimental::meme_plague::process_meme_contagion),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            // Mentorship
            crate::layer1::mentorship::check_mentorship_system,
            crate::layer1::mentorship::apply_mentorship_xp_system,
            crate::layer1::mentorship::mentorship_mood_system
                .after(crate::layer1::mentorship::check_mentorship_system),
            // Process new rumors and affinity changes
            modify_affinity_system.after(crate::layer1::rumor::exchange_rumors_system),
            crate::layer1::social::gossip_economy::process_gossip
                .after(crate::layer1::rumor::exchange_rumors_system),
            crate::layer1::social::gossip_economy::spend_intel
                .after(crate::layer1::social::gossip_economy::process_gossip),
            crate::layer1::social::gossip_economy::intel_decay_system
                .after(crate::layer1::social::gossip_economy::spend_intel),
            crate::layer1::social::gossip_economy::spawn_broker_system
                .after(crate::layer1::social::gossip_economy::intel_decay_system),
            crate::layer1::social::proximity_social_system.after(modify_affinity_system),
            crate::layer1::biology::genetics::splicers_dilemma::evaluate_social_friction_system
                .after(crate::layer1::social::proximity_social_system),
            crate::layer1::social::pen_pals::update_pen_pals_system.after(modify_affinity_system),
            crate::layer1::social::grievances::apply_grievance_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::social::grievances::ostracization_system,
            // Process chronicle events
            crate::layer1::geography::process_historical_events,
            crate::layer1::flora::detect_hazards_system,
            crate::layer1::flora::flora_growth_system,
            crate::layer1::flora::ecological_succession_system,
            chronicle_event_handler_system.after(check_milestones_system),
            crate::layer1::festivals::check_for_festivals_system
                .after(chronicle_event_handler_system),
            crate::layer1::festivals::festival_lifecycle_system
                .after(chronicle_event_handler_system),
            chronicle_rumor_bridge_system.after(check_milestones_system),
            #[cfg(feature = "nova")]
            crate::layer1::oral_tradition::collect_chronicles_system
                .after(chronicle_event_handler_system),
            #[cfg(feature = "nova")]
            crate::layer1::oral_tradition::storytelling_system
                .after(crate::layer1::rumor::exchange_rumors_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (crate::layer1::integration::trauma_death_bridge_system,)
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (crate::layer1::anomalies::cryptid::cryptid_chronicle_bridge_system,)
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            crate::layer1::integration::famine_tracking_system,
            crate::layer1::integration::trauma_decay_system,
            pop_death_chronicle_bridge.after(crate::layer1::health::despawn_dead_entities_system),
            mega_quake_chronicle_bridge.after(crate::layer1::geology::tectonic::check_quake_system),
            crate::layer1::core::integration::reformat_chronicle_bridge,
            crate::layer1::core::integration::tectonic_fracking_chronicle_bridge
                .after(crate::layer1::geology::fracking::tectonic_fracking_system),
            crate::layer1::logistics::orbital_drop::orbital_drop_chronicle_bridge
                .after(crate::layer1::logistics::orbital_drop::process_orbital_drops),
            crate::layer1::social::cargo_cult::apply_cargo_cult_belief_system
                .after(crate::layer1::logistics::orbital_drop::process_orbital_drops),
            crate::layer1::social::cargo_cult::process_ritual_actions_system
                .after(crate::layer1::social::cargo_cult::apply_cargo_cult_belief_system),
            crate::layer1::logistics::mass_driver::mass_driver_chronicle_bridge
                .after(crate::layer1::logistics::mass_driver::package_arrival_system),
            crate::layer1::integration::predatory_weather_emission_bridge_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    // Spec 341: Pop Memory — witness recording, famine/founder producers,
    // and the colony-identity census.
    schedule.add_systems(
        (
            crate::layer1::memory::famine_witness_system
                .after(crate::layer1::integration::famine_tracking_system),
            crate::layer1::memory::stamp_founder_memories_system,
            crate::layer1::memory::record_pop_memory_system
                .after(crate::layer1::memory::famine_witness_system)
                .after(crate::layer1::memory::stamp_founder_memories_system),
            crate::layer1::memory::colony_identity_census_system
                .after(crate::layer1::memory::record_pop_memory_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::integration::predatory_weather_impact_bridge_system,
            crate::layer1::integration::hologram_failure_chronicle_bridge
                .after(crate::layer1::hologram::update_holograms_system),
            retrograde_chronicle_bridge.after(work_execution_system), // work_execution_system is in Execution set
            amputation_handler_system.after(work_execution_system),
            art_observation_system.after(crate::layer1::health::despawn_dead_entities_system),
            observe_inspector_system.after(art_observation_system),
            crate::layer1::integration::medical_treatment_notification_system.after(healing_system),
            crate::layer1::integration::hospitalization_notification_system
                .after(work_execution_system),
            (
                crate::layer1::core::integration::pop_died_count_system
                    .after(crate::layer1::entities::pop::handle_pop_death_system),
                crate::layer1::core::integration::pop_born_count_system,
                crate::layer1::integration::pop_death_notification_system
                    .after(natural_death_system),
            ),
        )
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            crate::layer1::core::integration::artists_muse_chronicle_bridge,
            crate::layer1::core::integration::hypno_learning_chronicle_bridge,
            crate::layer1::core::integration::living_archive_chronicle_bridge,
            crate::layer1::core::integration::temporal_fugue_chronicle_bridge,
            crate::layer1::core::integration::institutional_memory_chronicle_bridge,
            crate::layer1::foundation_soil::apply_foundation_soil_system
                .after(crate::layer1::entities::pop::handle_pop_death_system),
            crate::layer1::integration::pop_born_notification_system
                .after(Layer1SystemSet::Economy),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            inspector_report_system.after(chronicle_event_handler_system),
            inspector_outcome_bridge_system.after(inspector_report_system),
            taboo_event_system.after(crate::layer1::health::despawn_dead_entities_system),
            (
                crate::layer1::social::slippery_slope::process_atrocities,
                crate::layer1::social::slippery_slope::apply_morale_buffs,
                crate::layer1::social::slippery_slope::apply_stress_penalties,
            )
                .chain(),
            crate::layer1::radio_nostalgia::handle_broadcasts_system,
            crate::layer1::quantum_twins::update_twin_sync_system.after(Layer1SystemSet::Economy),
            crate::layer1::quantum_twins::update_twin_mood_system
                .after(Layer1SystemSet::Consumption),
            // Fix: handle_pop_death_system is re-exported in layer1/mod.rs or located in layer1/pop.rs
            // The previous error was referencing crate::layer1::health::handle_pop_death_system
            crate::layer1::quantum_twins::handle_severance_system
                .after(crate::layer1::entities::pop::handle_pop_death_system),
            crate::layer1::ad_screen::update_ad_screens_system,
            crate::layer1::integration::industrial_rhythm_morale_bridge,
            crate::layer1::integration::great_work_chronicle_bridge,
            crate::layer1::integration::gene_splicing_chronicle_bridge,
            crate::layer1::integration::crop_mutation_chronicle_bridge,
            crate::layer1::integration::grafting_chronicle_bridge,
            crate::layer1::integration::temporal_stutter_chronicle_bridge,
            crate::layer1::anomalies::temporal_echoes::temporal_echo_maintenance_bridge_system,
            crate::layer1::integration::parasitic_architecture_chronicle_bridge,
            crate::layer1::core::integration::living_architecture_chronicle_bridge,
            crate::layer1::core::integration::secret_society_discovery_bridge_system,
            crate::layer1::core::integration::society_suspicion_bridge_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::social::echoes::echo_spawn_system,
            crate::layer1::core::integration::bureaucratic_black_hole_chronicle_bridge,
            crate::layer1::social::echoes::echo_aura_system
                .after(crate::layer1::social::echoes::echo_spawn_system),
            crate::layer1::social::echoes::echo_despawn_system
                .after(crate::layer1::social::echoes::echo_aura_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::core::integration::crustal_tide_chronicle_bridge,
            crate::layer1::core::integration::propaganda_graffiti_chronicle_bridge,
            crate::layer1::core::integration::petrification_chronicle_bridge,
            crate::layer1::core::integration::phantom_shift_chronicle_bridge,
            crate::layer1::integration::tether_snap_chronicle_bridge,
            crate::layer1::social::generational_knowledge::mark_newborns_system,
            crate::layer1::social::generational_knowledge::apply_generational_knowledge_system
                .after(crate::layer1::social::generational_knowledge::mark_newborns_system),
            crate::layer1::social::inherited_grudges::inherit_grudges_on_birth_system,
            crate::layer1::social::inherited_grudges::transfer_grudges_on_death_system,
            crate::layer1::social::inherited_grudges::prevent_grudge_work_system,
            crate::layer1::core::integration::public_grievance_grudge_bridge,
        )
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        (
            crate::layer1::integration::famine_chronicle_bridge,
            crate::layer1::integration::silent_flora_chronicle_bridge,
            crate::layer1::integration::aesthetic_edict_chronicle_bridge,
            crate::layer1::core::integration::generational_amnesia_chronicle_bridge,
            crate::layer1::integration::access_denied_chronicle_bridge,
            crate::layer1::core::integration::ghost_shift_chronicle_bridge,
            crate::layer1::core::integration::impact_warning_chronicle_bridge,
            crate::layer1::core::integration::debt_of_the_dead_chronicle_bridge,
            crate::layer1::core::integration::diplomatic_incident_chronicle_bridge,
            crate::layer1::administration::edicts::hack_hub_chronicle_bridge,
            crate::layer1::integration::smuggler_arrival_event_bridge
                .before(crate::layer1::void_weed::process_void_weed_trade_system),
            crate::layer1::ransom_broker::ransom_broker_chronicle_bridge,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::void_weed::process_void_weed_trade_system,
            crate::layer1::void_weed::evaluate_smuggling_heat_system,
            crate::layer1::overview_effect::overview_effect_system,
            crate::layer1::spiteful_will::process_spiteful_will_system,
            crate::layer1::spiteful_will::process_override_will_system,
            crate::layer1::integration::override_will_chronicle_bridge
                .after(crate::layer1::spiteful_will::process_override_will_system),
            crate::layer1::tech::neural_leech::apply_neural_link_buffs_system,
            crate::layer1::tech::neural_leech::process_neural_hub_decay_system,
            crate::layer1::tech::neural_leech::handle_hub_death_system,
            crate::layer1::integration::apply_neural_shock_system
                .after(crate::layer1::tech::neural_leech::handle_hub_death_system),
            crate::layer1::tech::neural_leech::neural_hub_death_bridge_system,
            crate::layer1::memory_core::implant_memory_core_system,
            crate::layer1::memory_core::harvest_memory_core_system,
            crate::layer1::integration::bot_awakening_chronicle_bridge
                .after(crate::layer1::tech::machine_awakening::process_bot_sentience),
            crate::layer1::fauna::shadow::spawn_shadow_fauna_system,
            crate::layer1::fauna::shadow::shadow_feed_system,
            crate::layer1::fauna::shadow::shadow_visibility_system,
            crate::layer1::spore_diplomat::spore_stat_boost_system,
            crate::layer1::spore_diplomat::spore_stat_remove_system,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::diplomacy::wards::process_diplomatic_wards_system,
            crate::layer1::diplomacy::wards::process_ward_deaths_system,
            crate::layer1::unseen_bureaucracy::phantom_shift_system,
            crate::layer1::psychology::void_sickness::process_void_exposure_system,
            crate::layer1::psychology::void_sickness::apply_void_touched_trait_system,
            crate::layer1::psychology::void_sickness::apply_trait_stat_modifiers_system,
            crate::layer1::core::integration::crop_mutation_mycelial_bridge,
            crate::layer1::core::integration::mycelial_chronicle_bridge,
            crate::layer1::core::integration::scrap_code_cult_formation_chronicle_bridge,
            crate::layer1::fungal_network::fungal_network_chronicle_bridge,
            crate::layer1::core::integration::dead_hand_chronicle_bridge,
            crate::layer1::core::integration::lottery_chronicle_bridge,
            crate::layer1::core::integration::early_warning_chronicle_bridge,
            crate::layer1::core::integration::planetary_scarring_chronicle_bridge,
            crate::layer1::cassandra_syndrome::cassandra_syndrome_chronicle_bridge,
            crate::layer1::cassandra_syndrome::cassandra_cult_chronicle_bridge,
            crate::layer1::core::integration::blackout_bazaar_chronicle_bridge,
            crate::layer1::core::integration::symbiont_sabotage_bridge_system,
            crate::layer1::core::integration::pet_death_bridge_system,
            crate::layer1::core::integration::ghost_code_chronicle_bridge,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::core::integration::feral_administration_chronicle_bridge,
            crate::layer1::core::integration::fractal_bureaucracy_chronicle_bridge,
            crate::layer1::anomalies::echoes_lost_fleet::lure_militaristic_pops_system,
            crate::layer1::core::integration::lured_pops_escape_bridge_system,
            crate::layer1::shadow_ecosystems::shadow_ecosystems_short_circuit_bridge,
            crate::layer1::cassandra_syndrome::cassandra_syndrome_disaster_bridge,
            crate::layer1::core::integration::dreaming_sickness_chronicle_bridge,
        )
            .in_set(Layer1SystemSet::Observation),
    );

    schedule.add_systems(
        (
            crate::layer1::social::hoarder::check_for_hoarder_trait_system,
            crate::layer1::social::hoarder::hoarder_collection_system,
            crate::layer1::social::hoarder::apply_hoard_morale_buff_system,
            crate::layer1::social::hoarder::process_confiscation_system,
            crate::layer1::social::sartorial_rebellion::enforce_dress_code_system,
            crate::layer1::social::sartorial_rebellion::adopt_visual_signifier_system
                .after(crate::layer1::social::sartorial_rebellion::enforce_dress_code_system),
        )
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::layer1::memetics::memory_smugglers::memory_smugglers_chronicle_bridge
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::cross_layer::generation_ship_mutiny::generation_ship_mutiny_chronicle_bridge
            .in_set(Layer1SystemSet::Observation),
    );

    #[cfg(feature = "nova")]
    schedule.add_systems(
        crate::experimental::the_final_will::the_final_will_system
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::layer1::core::integration::subterranean_biosphere_chronicle_bridge
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::layer1::core::integration::architecture_of_paranoia_chronicle_bridge
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(
        crate::layer1::core::integration::spatial_compression_chronicle_bridge
            .in_set(Layer1SystemSet::Observation),
    );
    // Spec 353 §5: keeper–animal bonding mood.
    schedule.add_systems(
        crate::layer1::fauna::domestication::bonding_mood_system
            .in_set(Layer1SystemSet::Observation),
    );
    schedule.add_systems(clear_input_system.after(Layer1SystemSet::Observation));
}
#[cfg(feature = "nova")]
pub use crate::experimental::toxic_personalities::toxic_personalities_system;
