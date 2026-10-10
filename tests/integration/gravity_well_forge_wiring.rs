#[cfg(test)]
mod tests {
    use scale::layer1::economy::resources::ColonyResources;
    use scale::layer2::station::{DeepForge, MaintenanceLevel};
    use scale::setup::setup_world;
    use scale::shared::random::GlobalRng;
    use scale::simulation::run_simulation_tick;

    #[test]
    fn test_setup_world_provides_global_rng() {
        let world = setup_world();
        assert!(
            world.get_resource::<GlobalRng>().is_some(),
            "setup_world must insert GlobalRng: process_deep_forges needs it"
        );
    }

    #[test]
    fn test_scheduled_forge_produces_hyper_alloys_in_live_tick() {
        let mut world = setup_world();
        world.spawn((
            DeepForge {
                production_rate: 10.0,
                base_crush_chance: 0.0, // Guaranteed no crush: deterministic
                is_active: true,
            },
            MaintenanceLevel { current: 100.0 }, // Perfect maintenance
        ));

        run_simulation_tick(&mut world);

        let alloys = world.resource::<ColonyResources>().hyper_alloys;
        assert!(
            alloys >= 10.0,
            "A well-maintained active Deep Forge must produce hyper-alloys in a live tick, got {alloys}"
        );
    }
}
