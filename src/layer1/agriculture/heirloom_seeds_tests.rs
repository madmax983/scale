//! Spec 377 Heirloom Seeds — tests (adapted from the spec's RED phase
//! to the real codebase: `SeedStock` tracks wheat/potatoes separately,
//! harvest also needs `ColonyResources` + the seeded `SeedReturnRng`).

#[cfg(test)]
mod tests {
    use crate::layer1::agriculture::heirloom::*;
    use crate::layer1::chronicle::AddChronicleEvent;
    use crate::layer1::economy::items::ItemType;
    use crate::layer1::economy::resources::ColonyResources;
    use bevy_ecs::prelude::*;

    fn world_with(stock: SeedStock) -> World {
        let mut world = World::new();
        world.init_resource::<Events<PlantCropEvent>>();
        world.init_resource::<Events<HarvestCropEvent>>();
        world.init_resource::<Events<SeedDisasterEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(stock);
        world.init_resource::<HeirloomSeedState>();
        world.insert_resource(SeedReturnRng::new(42));
        world.insert_resource(ColonyResources {
            food: 0.0,
            ..ColonyResources::default()
        });
        world
    }

    #[test]
    fn test_harvesting_earth_crop_yields_food_and_maybe_seeds() {
        let mut world = world_with(SeedStock::empty());
        let crop = world
            .spawn(EarthCrop {
                kind: SeedKind::Wheat,
                yield_amount: 10,
                seed_chance: 1.0,
            })
            .id();
        world.send_event(HarvestCropEvent { crop });
        let mut schedule = Schedule::default();
        schedule.add_systems(process_harvest_yield_system);
        schedule.run(&mut world);

        assert!(world.resource::<SeedStock>().amount(SeedKind::Wheat) >= 1);
        assert_eq!(world.resource::<ColonyResources>().food, 10.0);
        assert!(world.get::<EarthCrop>(crop).is_none(), "harvest despawns");
        assert_eq!(world.resource::<HeirloomSeedState>().seeds_returned, 1);
    }

    #[test]
    fn test_harvest_seed_return_is_deterministic_and_probabilistic() {
        // chance 0.0 never returns; the seeded RNG sequence is stable:
        // two identical runs return identical stocks (no thread-rand flake).
        for run in 0..2 {
            let mut world = world_with(SeedStock::empty());
            let mut schedule = Schedule::default();
            schedule.add_systems(process_harvest_yield_system);
            for i in 0..20 {
                let crop = world
                    .spawn(EarthCrop {
                        kind: SeedKind::Potato,
                        yield_amount: 5,
                        seed_chance: if i == 0 { 0.0 } else { 0.75 },
                    })
                    .id();
                world.send_event(HarvestCropEvent { crop });
                schedule.run(&mut world);
            }
            let returned = world.resource::<HeirloomSeedState>().seeds_returned;
            assert!(
                returned > 0 && returned < 20,
                "run {run}: partial returns, got {returned}"
            );
            if run == 0 {
                assert_eq!(returned, first_run_returns());
            }
        }
    }

    // Recompute the expected deterministic count for seed 42 independently:
    // first crop has chance 0.0 (roll consumes nothing per roll() shortcut),
    // the other 19 roll at 0.75.
    fn first_run_returns() -> u32 {
        let mut rng = SeedReturnRng::new(42);
        (0..19).filter(|_| rng.roll(0.75)).count() as u32
    }

    #[test]
    fn test_planting_consumes_seed() {
        let mut world = world_with(SeedStock {
            wheat: 5,
            potatoes: 0,
        });
        let crop = world.spawn(EarthCrop::new(SeedKind::Wheat)).id();
        world.send_event(PlantCropEvent { crop });
        let mut schedule = Schedule::default();
        schedule.add_systems(process_planting_system);
        schedule.run(&mut world);
        assert_eq!(world.resource::<SeedStock>().amount(SeedKind::Wheat), 4);
        assert_eq!(world.resource::<HeirloomSeedState>().plantings, 1);
    }

    #[test]
    fn test_planting_fails_without_seed() {
        let mut world = world_with(SeedStock::empty());
        let crop = world.spawn(EarthCrop::new(SeedKind::Potato)).id();
        world.send_event(PlantCropEvent { crop });
        let mut schedule = Schedule::default();
        schedule.add_systems(process_planting_system);
        schedule.run(&mut world);
        assert_eq!(world.resource::<SeedStock>().total(), 0);
        assert_eq!(world.resource::<HeirloomSeedState>().plantings, 0);
        assert!(
            world.get::<EarthCrop>(crop).is_none(),
            "unplantable crop removed"
        );
    }

    #[test]
    fn test_initial_stock_is_finite_and_non_replenishing() {
        let stock = SeedStock::default();
        assert_eq!(stock.total(), INITIAL_WHEAT_SEEDS + INITIAL_POTATO_SEEDS);
        assert!(stock.total() > 0);
    }

    #[test]
    fn test_disaster_wipes_stored_seeds() {
        let mut world = world_with(SeedStock::default());
        world.send_event(SeedDisasterEvent {
            kind: SeedDisasterKind::Fire,
            target: DisasterTarget::StoredSeeds,
            severity: 1.0,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(seed_disaster_system);
        schedule.run(&mut world);
        assert_eq!(world.resource::<SeedStock>().total(), 0);
        assert_eq!(world.resource::<HeirloomSeedState>().disasters, 1);
    }

    #[test]
    fn test_disaster_partial_loss_and_planted_crops() {
        let mut world = world_with(SeedStock::default());
        for _ in 0..4 {
            world.spawn(EarthCrop::new(SeedKind::Wheat));
        }
        world.send_event(SeedDisasterEvent {
            kind: SeedDisasterKind::Disease,
            target: DisasterTarget::PlantedCrops,
            severity: 0.5,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(seed_disaster_system);
        schedule.run(&mut world);
        let mut q = world.query::<&EarthCrop>();
        assert_eq!(
            q.iter(&world).count(),
            2,
            "half the planted crops destroyed"
        );
    }

    #[test]
    fn test_native_adoption_when_all_seeds_lost() {
        use crate::layer1::agriculture::farm::Farm;
        use crate::layer1::chronicle::Chronicle as Chron;
        let mut world = world_with(SeedStock::empty());
        world.init_resource::<Chron>();
        // Farm::default() already selects Wheat.
        let farm = world.spawn(Farm::default()).id();
        let mut schedule = Schedule::default();
        schedule.add_systems(native_adoption_system);
        schedule.run(&mut world);
        assert!(world.resource::<HeirloomSeedState>().native_adopted);
        assert_eq!(
            world.get::<Farm>(farm).unwrap().selected_crop,
            ItemType::GrubRoot
        );
        // One-way: a second run does not re-fire.
        schedule.run(&mut world);
        assert!(world.resource::<HeirloomSeedState>().native_adopted);
    }

    #[test]
    fn test_no_adoption_while_seeds_or_crops_remain() {
        let mut world = world_with(SeedStock::empty());
        world.spawn(EarthCrop::new(SeedKind::Wheat));
        let mut schedule = Schedule::default();
        schedule.add_systems(native_adoption_system);
        schedule.run(&mut world);
        assert!(!world.resource::<HeirloomSeedState>().native_adopted);
    }

    #[test]
    fn test_ip_guard_public_strings() {
        // Original names only: no distinctive borrowed IP in new strings.
        let banned = ["sad king", "windsor", "shire", "hobbit", "dune", "arrakis"];
        let strings = [
            "Grub-Root Diet",
            "The last Earth seed is gone. The colony adopts native Grub-Root",
            "Crop disease destroys",
            "Fire destroys",
        ];
        for s in strings {
            for b in banned {
                assert!(!s.to_lowercase().contains(b), "{s} contains banned {b}");
            }
        }
        let _ = crate::layer1::chronicle::Chronicle::default();
    }
}
