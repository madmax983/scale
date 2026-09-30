use super::Layer1SystemSet;
use crate::layer1::psychology::artifact_diet::consume_artifacts_during_famine_system;
use crate::layer1::social::biometric_drift::*;
use crate::layer1::social::old_guard::mood_lifecycle_system;
use crate::layer1::*;
use bevy_ecs::prelude::*;

#[allow(clippy::too_many_lines)]
pub fn register(schedule: &mut Schedule) {
    schedule.add_systems(
        (
            update_biometric_drift_system.after(decay_needs_system),
            check_door_access_system.after(update_biometric_drift_system),
            recalibrate_biometrics_system.after(update_biometric_drift_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::biology::xenoflora_addiction::xenoflora_consumption_system,
            crate::layer1::biology::xenoflora_addiction::addiction_withdrawal_system
                .after(crate::layer1::biology::xenoflora_addiction::xenoflora_consumption_system),
            crate::layer1::biology::xenoflora_addiction::withdrawal_causes_violence_system
                .after(crate::layer1::biology::xenoflora_addiction::addiction_withdrawal_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::social::xenoflora_pet::apply_pet_mood_boost,
            crate::layer1::social::xenoflora_pet::pet_viral_spread_system,
            crate::layer1::social::xeno_pet::apply_xeno_pet_morale,
            crate::layer1::social::xeno_pet::xeno_pet_reproduction_system,
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            consume_food_system
                .after(produce_food_system)
                .after(update_resource_caps_system),
            consume_void_ale_system.after(consume_food_system),
            crate::layer1::drone::drone_battery_system.after(consume_food_system),
            clothing_wear_system.after(consume_food_system),
            vermin_growth_system.after(consume_food_system),
            vermin_effect_system.after(vermin_growth_system),
            vermin_morale_system.after(vermin_growth_system),
            crate::layer1::integration::vermin_item_rot_system.after(vermin_growth_system),
            spoilage_system,
            crate::layer1::security::drift_accumulation_system
                .after(consume_food_system)
                .after(vermin_growth_system),
            crate::layer1::visitor::visitor_lifecycle_system.after(consume_food_system),
            theft_system
                .after(consume_food_system)
                .before(decay_needs_system),
            (
                decay_needs_system.after(consume_food_system),
                crate::layer1::architecture::edible::consume_building_system
                    .after(consume_food_system),
            ),
            crate::layer1::hygiene::filth_accumulation_system.after(decay_needs_system),
            crate::layer1::hygiene::hygiene_decay_system
                .after(crate::layer1::hygiene::filth_accumulation_system),
            crate::layer1::chemical::addiction_system.after(decay_needs_system),
            apply_palette_fatigue_system.after(consume_food_system),
            apply_cabin_fever_morale_system
                .after(decay_needs_system)
                .before(mood_lifecycle_system),
            faction_satisfaction_morale_bridge
                .after(apply_cabin_fever_morale_system)
                .before(mood_lifecycle_system),
            apply_taboo_stress_system
                .after(decay_needs_system)
                .before(mood_lifecycle_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::somnambulism::trigger_somnambulism_system.after(decay_needs_system),
            crate::layer1::somnambulism::process_somnambulist_work_system
                .after(crate::layer1::somnambulism::trigger_somnambulism_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::logistics::biomass_network::process_biomass_network_hunger,
            crate::layer1::logistics::biomass_network::digest_transit_contents
                .after(crate::layer1::logistics::biomass_network::process_biomass_network_hunger),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::addiction::init_addiction_system.after(decay_needs_system),
            crate::layer1::addiction::update_addiction_system
                .after(crate::layer1::addiction::init_addiction_system),
            crate::layer1::addiction::check_self_surgery_system
                .after(crate::layer1::addiction::update_addiction_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            mood_lifecycle_system.after(decay_needs_system),
            trend_setting_system.after(consume_food_system),
            trend_spread_system.after(trend_setting_system),
            trend_satisfaction_system.after(trend_spread_system),
            clear_just_consumed_system.after(trend_satisfaction_system),
            crate::layer1::gastronomy::handle_work_speed_buff_decay.after(decay_needs_system),
            crate::layer1::gastronomy::handle_hallucination_decay.after(decay_needs_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::law::justice::update_inmates_system.after(decay_needs_system),
            crate::layer1::day_night::circadian_rhythm_system.after(consume_food_system),
            aging_system.after(consume_food_system),
            natural_death_system.after(aging_system),
            memory_decay_system.after(decay_needs_system),
            notification_expiration_system.after(decay_needs_system),
            apply_deferred
                .after(clothing_wear_system)
                .before(crate::layer1::temperature::thermal_damage_system),
            crate::layer1::temperature::thermal_damage_system
                .after(decay_needs_system)
                .after(apply_deferred),
            crate::layer1::radioactive::sickness_damage_system.after(decay_needs_system),
            pressure_damage_system.after(decay_needs_system),
            consume_artifacts_during_famine_system.after(decay_needs_system),
            crate::layer1::needs::starvation_damage_system.after(decay_needs_system),
            // REMOVED (revival fix): kill_starving_pops_system instantly despawned
            // pops at hunger <= 0.0, which wiped the starting colony on tick 2.
            // Starvation is now handled gracefully by starvation_damage_system
            // (0.2 dmg/tick, ~500 ticks to die), giving pops a chance to eat.
            crate::layer1::biology::rust_lung::rust_lung_degradation_system
                .after(decay_needs_system),
            crate::layer1::atmosphere::apply_smog_damage_system.after(decay_needs_system),
            crate::layer1::nature::atmospheric_empathy::emit_trace_gases.after(decay_needs_system),
            crate::layer1::nature::atmospheric_empathy::apply_atmospheric_empathy
                .after(crate::layer1::nature::atmospheric_empathy::emit_trace_gases),
            crate::layer1::nature::atmospheric_empathy::decay_trace_gases_system
                .after(crate::layer1::nature::atmospheric_empathy::apply_atmospheric_empathy),
            crate::layer1::tech::legacy_code::accumulate_bloat_system.after(decay_needs_system),
            crate::layer1::health::check_health_status_system
                .after(crate::layer1::needs::starvation_damage_system)
                .after(crate::layer1::atmosphere::apply_smog_damage_system)
                .after(crate::layer1::temperature::thermal_damage_system)
                .after(crate::layer1::radioactive::sickness_damage_system)
                .after(pressure_damage_system)
                .after(natural_death_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );

    schedule.add_systems(
        (
            crate::layer1::entities::pop::handle_pop_death_system
                .after(crate::layer1::health::check_health_status_system),
            crate::layer1::fauna::handle_fauna_death_system
                .after(crate::layer1::health::check_health_status_system),
            crate::layer1::entities::pop::handle_witness_death_system
                .after(crate::layer1::entities::pop::handle_pop_death_system),
            mascot_death_grief_system.after(crate::layer1::health::check_health_status_system),
            crate::layer1::ecology::handle_keystone_death
                .after(crate::layer1::health::check_health_status_system)
                .before(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::kinetic_storage::handle_battery_destruction_system
                .after(crate::layer1::health::check_health_status_system)
                .before(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::core::integration::kinetic_battery_chronicle_bridge
                .after(crate::layer1::kinetic_storage::handle_battery_destruction_system)
                .before(crate::layer1::health::despawn_dead_entities_system),
            #[cfg(feature = "nova")]
            crate::layer1::loci::record_death_loci_system
                .after(crate::layer1::entities::pop::handle_pop_death_system)
                .before(crate::layer1::health::despawn_dead_entities_system),
            crate::layer1::health::despawn_dead_entities_system
                .after(crate::layer1::entities::pop::handle_pop_death_system)
                .after(crate::layer1::fauna::handle_fauna_death_system)
                .after(mascot_death_grief_system)
                .after(crate::layer1::ecology::handle_keystone_death),
            clean_dead_residents_system.after(crate::layer1::health::despawn_dead_entities_system),
            clean_dead_workers_system.after(crate::layer1::health::despawn_dead_entities_system),
        )
            .in_set(Layer1SystemSet::Consumption),
    );
    schedule.add_systems(
        (
            crate::layer1::bureaucracy_of_sleep::assign_sleep_permits_system,
            crate::layer1::bureaucracy_of_sleep::process_sleep_deprivation_system,
        )
            .in_set(Layer1SystemSet::Consumption),
    );
}

/// System for consuming Void-Ale to boost morale and leisure.
pub fn consume_void_ale_system(
    mut query: Query<(
        &mut crate::layer1::needs::Needs,
        &mut crate::layer1::economy::inventory::Inventory,
    )>,
) {
    for (mut needs, mut inventory) in query.iter_mut() {
        if needs.leisure < 0.5 || needs.morale() < 0.5 {
            let ale_index = inventory
                .items
                .iter()
                .position(|item| item.item_type == crate::layer1::items::ItemType::VoidAle);

            if let Some(index) = ale_index {
                // Consume the ale
                inventory.items.swap_remove(index);

                // Huge boost to leisure and rest
                needs.leisure = (needs.leisure + 0.6).min(1.0);
                needs.rest = (needs.rest + 0.4).min(1.0);
            }
        }
    }
}
