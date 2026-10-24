use crate::layer1::resources::{
    ColonyResources, ResourceItem, ResourceType, BASE_MAX_BIOLOGICAL_WASTE, BASE_MAX_FOOD,
    BASE_MAX_NUTRIENT_PASTE, BASE_MAX_STONE, BASE_MAX_WOOD,
};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;

/// Component that increases the maximum resource capacity of the colony.
#[derive(Component)]
pub struct Stockpile {
    /// Bonus to max food capacity.
    pub food_bonus: f32,
    /// Bonus to max wood capacity.
    pub wood_bonus: f32,
    /// Bonus to max stone capacity.
    pub stone_bonus: f32,
    /// Bonus to max waste capacity.
    pub waste_bonus: f32,
}

impl Default for Stockpile {
    fn default() -> Self {
        Self {
            food_bonus: 0.0,
            wood_bonus: 100.0,
            stone_bonus: 100.0,
            waste_bonus: 0.0,
        }
    }
}

/// System that calculates and updates the colony's resource caps based on existing stockpiles.
///
/// Spec 360: when a recomputed cap shrinks below what the ledger holds
/// (a stockpile was destroyed, say), the clamped excess is NOT silently
/// discarded — it spills onto the ground as a [`ResourceItem`] at the
/// first stockpile's position, where haulers can recover it once there
/// is room again. If no stockpile has a position, the excess is lost
/// (there is nowhere it was stored).
pub fn update_resource_caps_system(
    mut commands: Commands,
    query: Query<(&Stockpile, Option<&GridPosition>)>,
    mut resources: ResMut<ColonyResources>,
) {
    let mut total_food_bonus = 0.0;
    let mut total_wood_bonus = 0.0;
    let mut total_stone_bonus = 0.0;
    let mut total_waste_bonus = 0.0;
    let mut spill_pos: Option<GridPosition> = None;

    for (stockpile, pos) in &query {
        total_food_bonus += stockpile.food_bonus;
        total_wood_bonus += stockpile.wood_bonus;
        total_stone_bonus += stockpile.stone_bonus;
        total_waste_bonus += stockpile.waste_bonus;
        if spill_pos.is_none() {
            spill_pos = pos.copied();
        }
    }

    resources.max_food = BASE_MAX_FOOD + total_food_bonus;
    resources.max_wood = BASE_MAX_WOOD + total_wood_bonus;
    resources.max_stone = BASE_MAX_STONE + total_stone_bonus;
    resources.max_waste = total_waste_bonus; // No base capacity for waste
    // Item 118: wire the Recycler ledger recipe's caps, which no system
    // ever raised from 0.0 before. Biological waste is stored with the
    // waste stream, so the waste bonus (Landfill: 100) raises its cap
    // over a base capacity; nutrient paste is a foodstuff (it counts in
    // `total_food`), so the food bonus raises its cap over a base.
    resources.max_biological_waste = BASE_MAX_BIOLOGICAL_WASTE + total_waste_bonus;
    resources.max_nutrient_paste = BASE_MAX_NUTRIENT_PASTE + total_food_bonus;

    // Clamp each held amount to its (possibly shrunk) cap, spilling the
    // excess to the ground instead of discarding it.
    let clamped: [(ResourceType, f32); 6] = [
        (ResourceType::Food, resources.food),
        (ResourceType::Wood, resources.wood),
        (ResourceType::Stone, resources.stone),
        (ResourceType::Waste, resources.waste),
        (ResourceType::BiologicalWaste, resources.biological_waste),
        (ResourceType::NutrientPaste, resources.nutrient_paste),
    ];
    resources.food = resources.food.min(resources.max_food);
    resources.wood = resources.wood.min(resources.max_wood);
    resources.stone = resources.stone.min(resources.max_stone);
    resources.waste = resources.waste.min(resources.max_waste);
    resources.biological_waste = resources.biological_waste.min(resources.max_biological_waste);
    resources.nutrient_paste = resources.nutrient_paste.min(resources.max_nutrient_paste);
    let after: [f32; 6] = [
        resources.food,
        resources.wood,
        resources.stone,
        resources.waste,
        resources.biological_waste,
        resources.nutrient_paste,
    ];
    if let Some(pos) = spill_pos {
        for (idx, (resource_type, before)) in clamped.iter().enumerate() {
            let excess = before - after[idx];
            if excess.is_finite() && excess > 0.0 {
                commands.spawn((
                    ResourceItem {
                        resource_type: *resource_type,
                        amount: excess,
                    },
                    pos,
                ));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::resources::ColonyResources;
    use crate::layer1::stockpile::{update_resource_caps_system, Stockpile};
    use bevy_ecs::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_colony_resources_caps_default() {
        let resources = ColonyResources::default();
        // Default small capacity for survival
        assert_eq!(resources.max_food, 50.0);
        assert_eq!(resources.max_wood, 50.0);
        assert_eq!(resources.max_stone, 20.0);
    }

    #[test]
    fn test_add_resource_clamped_to_max() {
        let mut resources = ColonyResources {
            wood: 40.0,
            max_wood: 50.0,
            ..Default::default()
        };

        // Add 20, should cap at 50
        resources.add_wood(20.0);
        assert_eq!(resources.wood, 50.0);
    }

    #[test]
    fn test_add_resource_no_overflow() {
        let mut resources = ColonyResources {
            stone: 20.0,
            max_stone: 20.0,
            ..Default::default()
        };

        resources.add_stone(10.0);
        assert_eq!(resources.stone, 20.0);
    }

    #[test]
    fn test_stockpile_component_defaults() {
        let stockpile = Stockpile::default();
        assert_eq!(stockpile.food_bonus, 0.0);
        assert_eq!(stockpile.wood_bonus, 100.0);
        assert_eq!(stockpile.stone_bonus, 100.0);
    }

    #[test]
    fn test_update_resource_caps_system() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        // Spawn 2 Stockpiles
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile {
                food_bonus: 0.0,
                wood_bonus: 100.0,
                stone_bonus: 50.0,
                waste_bonus: 0.0,
            },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile {
                food_bonus: 0.0,
                wood_bonus: 100.0,
                stone_bonus: 50.0,
                waste_bonus: 0.0,
            },
        ));

        // Run system
        world.run_system_once(update_resource_caps_system).unwrap();

        let resources = world.resource::<ColonyResources>();

        // Base (50) + 2 * 100 = 250
        assert_eq!(resources.max_wood, 250.0);
        // Base (20) + 2 * 50 = 120
        assert_eq!(resources.max_stone, 120.0);
    }

    #[test]
    fn test_building_type_stockpile_exists() {
        let bt = BuildingType::Stockpile;
        assert_eq!(bt.label(), "Stockpile");
    }

    // ===== Spec 360: Resource Capacities (live-wiring completion) =====
    //
    // The spec's sketch types (a per-building storage component, a
    // store-resource event, a resource-node ground type, under a
    // `layer1::resource` module) do not exist in this codebase. The real
    // architecture is the `ColonyResources` ledger (caps per
    // `ResourceType`), the `Stockpile` capacity component, the hauling
    // drop-off as the storage-insertion path, and `ResourceItem` +
    // `GridPosition` as the ground-item type. These tests are written
    // against those LIVE systems.

    #[test]
    fn spec360_haul_dropoff_caps_at_capacity() {
        use crate::layer1::execution::AtTarget;
        use crate::layer1::hauling::haul_system;
        use crate::layer1::resources::{Carrying, ResourceType};
        use crate::layer1::utility_ai::{ActionType, PopAction};
        use crate::layer1::{GridPosition, Pop};
        use crate::shared::time::SimulationTime;

        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        // Stone at 90 of a 100 cap (the spec's RED numbers, on the ledger).
        world.insert_resource(ColonyResources {
            stone: 90.0,
            max_stone: 100.0,
            ..Default::default()
        });
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile::default(),
            GridPosition { x: 5, y: 5 },
        ));
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
                Carrying {
                    resource_type: ResourceType::Stone,
                    amount: 20.0,
                },
                AtTarget,
            ))
            .id();

        haul_system(&mut world);

        let res = world.resource::<ColonyResources>();
        assert_eq!(res.stone, 100.0, "ledger must cap at max_stone");
        assert!(
            world.get::<Carrying>(pop).is_none(),
            "hauler must not keep carrying after drop-off"
        );
    }

    #[test]
    fn spec360_haul_dropoff_excess_spawns_as_ground_item() {
        use crate::layer1::execution::AtTarget;
        use crate::layer1::hauling::haul_system;
        use crate::layer1::resources::{Carrying, ResourceItem, ResourceType};
        use crate::layer1::utility_ai::{ActionType, PopAction};
        use crate::layer1::{GridPosition, Pop};
        use crate::shared::time::SimulationTime;

        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(ColonyResources {
            stone: 90.0,
            max_stone: 100.0,
            ..Default::default()
        });
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile::default(),
            GridPosition { x: 5, y: 5 },
        ));
        let pop = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                PopAction {
                    current: ActionType::Haul,
                    ..Default::default()
                },
                Carrying {
                    resource_type: ResourceType::Stone,
                    amount: 20.0,
                },
                AtTarget,
            ))
            .id();

        haul_system(&mut world);

        // The 10 stone that did not fit must be on the ground at the
        // stockpile's position as a real ResourceItem (the spec's
        // "excess spawns as ground nodes", on the real ground type).
        let mut query = world.query::<(&ResourceItem, &GridPosition)>();
        let spilled: Vec<(ResourceItem, GridPosition)> =
            query.iter(&world).map(|(i, p)| (*i, *p)).collect();
        assert_eq!(spilled.len(), 1, "exactly one spill pile expected: {spilled:?}");
        assert_eq!(spilled[0].0.resource_type, ResourceType::Stone);
        assert_eq!(spilled[0].0.amount, 10.0);
        assert_eq!(spilled[0].1, GridPosition { x: 5, y: 5 });
        assert!(world.get::<Carrying>(pop).is_none());
    }

    #[test]
    fn spec360_caps_shrink_spills_excess_as_ground_item() {
        use crate::layer1::resources::{ResourceItem, ResourceType};
        use crate::layer1::GridPosition;

        let mut world = World::new();
        // 200 wood held while the only storage grants no wood bonus:
        // the recomputed cap is BASE_MAX_WOOD (50).
        world.insert_resource(ColonyResources {
            wood: 200.0,
            ..Default::default()
        });
        world.spawn((
            Building {
                building_type: BuildingType::Stockpile,
            },
            Stockpile {
                food_bonus: 0.0,
                wood_bonus: 0.0,
                stone_bonus: 0.0,
                waste_bonus: 0.0,
            },
            GridPosition { x: 3, y: 3 },
        ));

        // Run through a Schedule so deferred spawns are applied.
        let mut schedule = Schedule::default();
        schedule.add_systems(update_resource_caps_system);
        schedule.run(&mut world);

        let res = world.resource::<ColonyResources>();
        assert_eq!(res.max_wood, 50.0);
        assert_eq!(res.wood, 50.0, "ledger must clamp to the shrunk cap");
        let mut query = world.query::<(&ResourceItem, &GridPosition)>();
        let spilled: Vec<(ResourceItem, GridPosition)> =
            query.iter(&world).map(|(i, p)| (*i, *p)).collect();
        assert_eq!(spilled.len(), 1, "shrunk excess must spill, not vanish: {spilled:?}");
        assert_eq!(spilled[0].0.resource_type, ResourceType::Wood);
        assert_eq!(spilled[0].0.amount, 150.0);
        assert_eq!(spilled[0].1, GridPosition { x: 3, y: 3 });
    }

    #[test]
    fn spec360_add_resource_hypervaluable_actually_adds() {
        use crate::layer1::resources::ResourceType;
        let mut res = ColonyResources {
            hyper_valuable: 10.0,
            max_hyper_valuable: 50.0,
            ..Default::default()
        };
        res.add_resource(&ResourceType::HyperValuable, 5.0);
        assert_eq!(
            res.hyper_valuable, 15.0,
            "add_resource(HyperValuable) must add (and cap), never subtract"
        );
        res.add_resource(&ResourceType::HyperValuable, 100.0);
        assert_eq!(res.hyper_valuable, 50.0, "and it must cap at max");
    }

    #[test]
    fn spec360_full_schedule_tick_enforces_caps_and_spills() {
        use crate::layer1::resources::{ResourceItem, ResourceType};
        // The REAL registration path: the same world the headless bin
        // builds (setup_world_with_config) + the full simulation
        // schedule, not the caps system in isolation.
        let mut world =
            crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        // A few event queues the full schedule reads are only inited on
        // the first real tick by the bins; init them up front (the
        // documented Events<T> first-tick pattern).
        world.init_resource::<Events<crate::layer3::ghost_ships::EvaluateTransitEvent>>();
        world.init_resource::<crate::layer3::treaty_cruisers::ActiveTreaties>();
        world.init_resource::<Events<crate::layer3::treaty_cruisers::InspectionEvent>>();
        world.init_resource::<crate::layer3::linguistic_drift::LinguisticNetwork>();
        world
            .init_resource::<Events<crate::layer3::linguistic_drift::CulturalSyncEvent>>();
        world.init_resource::<Events<crate::layer3::linguistic_drift::TradeEvent>>();
        let (max_wood_before, wood_before) = {
            let res = world.resource::<ColonyResources>();
            (res.max_wood, res.wood)
        };
        assert!(max_wood_before > 0.0 && wood_before <= max_wood_before);
        world.resource_mut::<ColonyResources>().wood = max_wood_before + 500.0;

        // The real tick entry point: it lazily builds the
        // SimulationSchedule and inits first-tick resources (the same
        // path the headless bin ticks through). Do NOT pre-add the
        // schedule — the lazy block inits several event queues only
        // when it adds the schedule itself.
        crate::simulation::run_simulation_tick(&mut world);

        let res = world.resource::<ColonyResources>();
        assert!(
            res.wood <= res.max_wood + 1.0,
            "after one live tick wood {} must be within cap {}",
            res.wood,
            res.max_wood
        );
        let mut query = world.query::<&ResourceItem>();
        let spilled_wood: f32 = query
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::Wood)
            .map(|i| i.amount)
            .sum();
        assert!(
            spilled_wood > 100.0,
            "the over-cap excess must spill as ground items, got {spilled_wood}"
        );
    }

    #[test]
    fn spec360_no_spec_sketch_dead_types_copied() {
        // Sanity guard: the spec 360 sketch types must NOT have been
        // blindly copied into the real modules (they would duplicate
        // the ColonyResources ledger / ResourceItem ground type).
        for src in [
            include_str!("stockpile.rs"),
            include_str!("resources.rs"),
            include_str!("hauling.rs"),
        ] {
            // Needles are concat!-built so this guard does not match
            // its own source text.
            for dead in [
                concat!("struct ", "Storage"),
                concat!("Store", "ResourceEvent"),
                concat!("Resource", "Node"),
                concat!("process_storage_", "insertion_system"),
            ] {
                assert!(
                    !src.contains(dead),
                    "spec-sketch dead type '{dead}' must not be copied into the live code"
                );
            }
        }
    }
}
