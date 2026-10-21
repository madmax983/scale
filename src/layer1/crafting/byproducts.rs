//! Crafting byproducts and waste management.
//!
//! When items are crafted, they often produce secondary materials (byproducts)
//! like scrap or waste. This module defines how these byproducts are handled
//! and stored in an `Inventory`. If an inventory reaches capacity, crafting is blocked.
//!
//! ## Important Note on Capacity
//! `Inventory` capacity applies to the **sum of all items**. If an inventory is full
//! of waste, new primary outputs cannot be created until the waste is dumped.

use crate::layer1::architecture::building::BuildingType;
use crate::layer1::beauty::BeautyGrid;
use crate::layer1::economy::resources::{ColonyResources, ResourceItem, ResourceType};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;

/// Capacity of the byproduct bin attached to a live refining building
/// (Spec 354, Item 108 live wiring). A bin full of waste chokes the
/// building: refining cannot complete until the bin is dumped.
pub const BYPRODUCT_BIN_CAPACITY: u32 = 20;

/// A component representing a container for resources.
///
/// Inventories have a strict `capacity`. Production systems must check
/// [`Inventory::has_capacity_for`] before adding new items or byproducts.
///
/// ## Examples
/// ```
/// use scale::layer1::crafting::byproducts::Inventory;
/// use scale::layer1::economy::resources::ResourceType;
///
/// let mut inv = Inventory { items: vec![], capacity: 10 };
/// inv.add(ResourceType::Metal, 5);
/// assert_eq!(inv.get_amount(&ResourceType::Metal), 5);
/// assert!(inv.has_capacity_for(5));
/// assert!(!inv.has_capacity_for(6));
/// ```
#[derive(Component, Default, Clone, Debug)]
pub struct Inventory {
    /// The stored items and their quantities.
    pub items: Vec<(ResourceType, u32)>,
    /// The maximum total amount of all items combined that this inventory can hold.
    pub capacity: u32,
}

impl Inventory {
    /// Retrieves the current stored amount of a specific `ResourceType`.
    pub fn get_amount(&self, res: &ResourceType) -> u32 {
        self.items
            .iter()
            .find(|(r, _)| r == res)
            .map(|(_, amt)| *amt)
            .unwrap_or(0)
    }

    /// Adds a quantity of a `ResourceType` to the inventory.
    ///
    /// ## Panics
    /// This function does *not* panic if capacity is exceeded. It is the caller's
    /// responsibility to verify capacity using `has_capacity_for` prior to adding.
    pub fn add(&mut self, res: ResourceType, amount: u32) {
        if let Some(existing) = self.items.iter_mut().find(|(r, _)| r == &res) {
            existing.1 += amount;
        } else {
            self.items.push((res, amount));
        }
    }

    /// Checks if the inventory can accommodate the specified `amount` of new items.
    ///
    /// Evaluates the sum of all currently stored items against the `capacity`.
    pub fn has_capacity_for(&self, amount: u32) -> bool {
        let current: u32 = self.items.iter().map(|(_, amt)| amt).sum();
        current + amount <= self.capacity
    }

    /// Total amount of all items currently stored.
    pub fn total_amount(&self) -> u32 {
        self.items.iter().map(|(_, amt)| amt).sum()
    }

    /// Removes up to `amount` of a resource, returning how much was removed.
    pub fn remove(&mut self, res: &ResourceType, amount: u32) -> u32 {
        let mut removed = 0;
        if let Some(existing) = self.items.iter_mut().find(|(r, _)| r == res) {
            removed = amount.min(existing.1);
            existing.1 -= removed;
        }
        self.items.retain(|(_, amt)| *amt > 0);
        removed
    }
}

/// The crafting recipe a live building of this type works, mirroring the
/// real refining table in `crate::layer1::economy::refining` (Item 108).
///
/// Returns `Some` only for building types whose refining recipe generates
/// waste (non-zero waste chance there): every completion of such a
/// building produces its declared byproduct. Types whose primary output
/// has no `ResourceType` representation (Weaver cloth, Tailor clothing)
/// list only the byproduct — the primary still lands in `ColonyResources`
/// through the refining system itself.
pub fn recipe_for_building_type(building_type: BuildingType) -> Option<Recipe> {
    let (outputs, work_required): (Vec<(ResourceType, u32)>, f32) = match building_type {
        BuildingType::LumberMill => (vec![(ResourceType::Planks, 1)], 10.0),
        BuildingType::StoneMason => (vec![(ResourceType::Blocks, 1)], 10.0),
        BuildingType::Smelter => (vec![(ResourceType::Metal, 1)], 10.0),
        BuildingType::Smithy => (vec![(ResourceType::Tools, 1)], 10.0),
        BuildingType::Weaver | BuildingType::Tailor => (vec![], 10.0),
        BuildingType::Refinery => (vec![(ResourceType::Fuel, 1)], 20.0),
        // Item 116: the two refiners Item 108 left alone. Office's
        // primary (a building permit) IS a ResourceType and its
        // refining seam is live once placement grants RefiningProgress.
        // Recycler's bin is driven by its live recycling seam (its
        // refining recipe is dead — see the Item 116 report); its
        // primary mirrors the refining table's nutrient paste output.
        BuildingType::Office => (vec![(ResourceType::BuildingPermit, 1)], 10.0),
        BuildingType::Recycler => (vec![(ResourceType::NutrientPaste, 5)], 10.0),
        _ => return None,
    };
    Some(Recipe {
        inputs: vec![],
        outputs,
        byproducts: vec![(ResourceType::Waste, 1)],
        work_required,
    })
}

/// Total byproduct units a recipe generates per completion.
pub fn byproduct_total(recipe: &Recipe) -> u32 {
    recipe.byproducts.iter().map(|(_, amt)| amt).sum()
}

/// True when a building's bin cannot fit its recipe's byproducts, so a
/// refining completion must be blocked (Spec 354: machines halt when
/// their local inventory lacks space for the byproducts).
pub fn bin_chokes(building: &CraftingBuilding, inventory: &Inventory) -> bool {
    match &building.current_recipe {
        Some(recipe) => {
            byproduct_total(recipe) > 0 && !inventory.has_capacity_for(byproduct_total(recipe))
        }
        None => false,
    }
}

/// Records a completed recipe's byproducts into a building's bin.
/// Returns false (adding nothing) when the bin lacks capacity.
pub fn capture_byproducts(building: &CraftingBuilding, inventory: &mut Inventory) -> bool {
    let Some(recipe) = &building.current_recipe else {
        return false;
    };
    if !inventory.has_capacity_for(byproduct_total(recipe)) {
        return false;
    }
    for (res, amt) in &recipe.byproducts {
        inventory.add(*res, *amt);
    }
    true
}

/// Live census of industrial waste across every real storage seam
/// (Spec 354 observability, Item 108).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct WasteSummary {
    /// Waste stored in the colony ledger (`ColonyResources::waste`).
    pub stored: f32,
    /// Ledger waste capacity (Landfill stockpiles raise it).
    pub stored_cap: f32,
    /// Waste items lying on the ground (dumped or spilled, unhauled).
    pub ground: f32,
    /// Number of ground Waste item stacks.
    pub ground_stacks: usize,
    /// Waste sitting in buildings' byproduct bins.
    pub binned: u32,
    /// Buildings carrying a byproduct bin.
    pub bins: usize,
    /// Bins too full to accept another completion's byproducts.
    pub choked_bins: usize,
}

/// Computes the [`WasteSummary`] from the live world.
pub fn waste_summary(world: &mut World) -> WasteSummary {
    let (stored, stored_cap) = world
        .get_resource::<ColonyResources>()
        .map_or((0.0, 0.0), |r| (r.waste, r.max_waste));
    let mut ground = 0.0f32;
    let mut ground_stacks = 0usize;
    for item in world.query::<&ResourceItem>().iter(world) {
        if item.resource_type == ResourceType::Waste {
            ground += item.amount;
            ground_stacks += 1;
        }
    }
    let mut binned = 0u32;
    let mut bins = 0usize;
    let mut choked_bins = 0usize;
    for (inv, crafting) in world
        .query::<(&Inventory, Option<&CraftingBuilding>)>()
        .iter(world)
    {
        bins += 1;
        binned += inv.get_amount(&ResourceType::Waste);
        if let Some(crafting) = crafting {
            if bin_chokes(crafting, inv) {
                choked_bins += 1;
            }
        }
    }
    WasteSummary {
        stored,
        stored_cap,
        ground,
        ground_stacks,
        binned,
        bins,
        choked_bins,
    }
}

/// Total waste burden across ledger, ground, and bins (STATS `waste=`).
pub fn waste_total(world: &mut World) -> f32 {
    let s = waste_summary(world);
    s.stored + s.ground + s.binned as f32
}

/// One-line public summary of the waste state (headless `waste` panel).
/// Original, generic wording only — covered by an IP-guard test.
pub fn format_waste_summary(summary: &WasteSummary) -> String {
    format!(
        "Stored waste: {:.1} / {:.1} (build a Landfill to raise the cap)\n\
         Waste on the ground: {:.1} in {} stack(s) — it drags down the beauty of its tile until hauled\n\
         Waste in byproduct bins: {} across {} building(s), {} choked\n\
         A choked building cannot finish refining until its bin is dumped (dump_waste <x> <y>).",
        summary.stored, summary.stored_cap, summary.ground, summary.ground_stacks,
        summary.binned, summary.bins, summary.choked_bins
    )
}

#[derive(Clone, Debug)]
pub struct Recipe {
    pub inputs: Vec<(ResourceType, u32)>,
    pub outputs: Vec<(ResourceType, u32)>,
    pub byproducts: Vec<(ResourceType, u32)>,
    pub work_required: f32,
}

#[derive(Component, Clone)]
pub struct CraftingBuilding {
    pub current_recipe: Option<Recipe>,
    pub work_progress: f32,
}

#[derive(Component)]
pub struct DumpWasteAction {
    pub x: i32,
    pub y: i32,
    pub amount: u32,
}

pub fn complete_crafting_system(mut query: Query<(&mut CraftingBuilding, &mut Inventory)>) {
    for (mut building, mut inventory) in query.iter_mut() {
        if let Some(recipe) = &building.current_recipe {
            if building.work_progress >= recipe.work_required {
                let required_space = recipe.outputs.iter().map(|(_, amt)| amt).sum::<u32>()
                    + recipe.byproducts.iter().map(|(_, amt)| amt).sum::<u32>();

                if inventory.has_capacity_for(required_space) {
                    for (res, amt) in &recipe.outputs {
                        inventory.add(*res, *amt);
                    }
                    for (res, amt) in &recipe.byproducts {
                        inventory.add(*res, *amt);
                    }
                    building.work_progress = 0.0;
                }
            }
        }
    }
}

pub fn waste_dumping_system(
    mut commands: Commands,
    mut beauty_grid: ResMut<BeautyGrid>,
    actions: Query<(Entity, &DumpWasteAction)>,
    mut bins: Query<(&GridPosition, &mut Inventory)>,
) {
    for (entity, action) in actions.iter() {
        if let (Ok(ux), Ok(uy)) = (usize::try_from(action.x), usize::try_from(action.y)) {
            let current_beauty = beauty_grid.get(ux, uy);
            #[allow(clippy::cast_precision_loss)]
            beauty_grid.set(ux, uy, current_beauty - (action.amount as f32));
        }
        // Item 108 live wiring: dumping at a tile with a byproduct bin
        // drains that bin. The drained waste becomes a real ground
        // `ResourceItem` — haulable into the colony ledger and read by
        // the live beauty/pollution systems like any other Waste item.
        let mut drained = 0u32;
        for (pos, mut inv) in bins.iter_mut() {
            if pos.x == action.x && pos.y == action.y {
                drained = inv.remove(&ResourceType::Waste, action.amount);
                break;
            }
        }
        if drained > 0 {
            commands.spawn((
                ResourceItem {
                    resource_type: ResourceType::Waste,
                    #[allow(clippy::cast_precision_loss)]
                    amount: drained as f32,
                },
                GridPosition {
                    x: action.x,
                    y: action.y,
                },
            ));
        }
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::beauty::BeautyGrid;
    use crate::layer1::economy::resources::ResourceType;

    #[test]
    fn test_crafting_generates_primary_and_byproduct() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(complete_crafting_system);

        let building = world
            .spawn((
                CraftingBuilding {
                    current_recipe: Some(Recipe {
                        inputs: vec![],
                        outputs: vec![(ResourceType::Metal, 1)],
                        byproducts: vec![(ResourceType::Waste, 1)],
                        work_required: 10.0,
                    }),
                    work_progress: 10.0, // Finished
                },
                Inventory {
                    capacity: 100,
                    items: vec![],
                },
            ))
            .id();

        schedule.run(&mut world);

        let inv = world.get::<Inventory>(building).unwrap();
        assert_eq!(inv.get_amount(&ResourceType::Metal), 1);
        assert_eq!(inv.get_amount(&ResourceType::Waste), 1);
    }

    #[test]
    fn test_dumping_waste_reduces_beauty() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(waste_dumping_system);

        let mut beauty_grid = BeautyGrid::new(10, 10);
        beauty_grid.set(5, 5, 10.0); // Initial beauty
        world.insert_resource(beauty_grid);

        // Action to dump waste at (5, 5)
        world.spawn(DumpWasteAction {
            x: 5,
            y: 5,
            amount: 5,
        });

        schedule.run(&mut world);

        let grid = world.resource::<BeautyGrid>();
        assert!(grid.get(5, 5) < 10.0); // Beauty is reduced by the dumped waste
    }

    #[test]
    fn test_stockpile_capacity_blocks_production_if_full_of_waste() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(complete_crafting_system);

        let mut inv = Inventory {
            capacity: 5,
            items: vec![],
        }; // Max capacity of 5 total items
        inv.add(ResourceType::Waste, 5); // Full of waste

        let building = world
            .spawn((
                CraftingBuilding {
                    current_recipe: Some(Recipe {
                        inputs: vec![],
                        outputs: vec![(ResourceType::Metal, 1)],
                        byproducts: vec![(ResourceType::Waste, 1)],
                        work_required: 10.0,
                    }),
                    work_progress: 10.0, // Wants to finish
                },
                inv,
            ))
            .id();

        schedule.run(&mut world);

        // Production should NOT complete because inventory is full
        let building_state = world.get::<CraftingBuilding>(building).unwrap();
        assert_eq!(building_state.work_progress, 10.0); // Still stuck at 10.0

        let final_inv = world.get::<Inventory>(building).unwrap();
        assert_eq!(final_inv.get_amount(&ResourceType::Metal), 0); // Did not output
    }
}

#[cfg(test)]
mod live_wiring_tests {
    //! Spec 354 live-wiring tests (Item 108). Unlike the mock-style tests
    //! above, these exercise the REAL production path: buildings placed
    //! through `try_place_building` (the same entry point the UI/headless
    //! build commands use), the real `process_refining_system`, the real
    //! live schedule via `run_simulation_tick`, and the real dumping
    //! system. Every one of these would have caught the dead wiring where
    //! no live building ever carried a `CraftingBuilding`.

    use super::*;
    use crate::layer1::building::{try_place_building, BuildingMap, BuildingType, OccupiedTiles};
    use crate::layer1::refining::process_refining_system;
    use crate::layer1::resources::{ColonyResources, RefiningProgress, ResourceItem};
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::pop::Pop;
    use crate::layer1::utility_ai::{ActionType, PopAction};
    use crate::layer1::GridPosition;

    fn placement_world() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 100,
            height: 100,
            tiles: vec![TerrainType::Grass; 100 * 100],
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());
        world.init_resource::<Events<crate::layer1::ancestral_graves::SacrilegeEvent>>();
        world.init_resource::<Events<crate::layer1::core::events::BuildingRemovedEvent>>();
        world.init_resource::<Events<crate::layer1::core::events::BuildingCompletedEvent>>();
        world.insert_resource(ColonyResources {
            wood: 100_000.0,
            stone: 100_000.0,
            ore: 100_000.0,
            metal: 100_000.0,
            max_wood: 100_000.0,
            max_planks: 100_000.0,
            ..Default::default()
        });
        world
    }

    fn place_mill(world: &mut World, x: i32, y: i32) -> Entity {
        assert!(
            try_place_building(world, x, y, BuildingType::LumberMill),
            "mill placement at ({x},{y}) should succeed"
        );
        let mut q = world.query::<(Entity, &GridPosition)>();
        q.iter(world)
            .find(|(_, p)| p.x == x && p.y == y)
            .map(|(e, _)| e)
            .expect("placed mill entity")
    }

    fn spawn_refine_worker(world: &mut World, x: i32, y: i32) {
        world.spawn((
            Pop,
            GridPosition { x, y },
            PopAction {
                current: ActionType::Refine,
                ..Default::default()
            },
        ));
    }

    fn set_progress(world: &mut World, mill: Entity, current: f32) {
        let mut prog = world.get_mut::<RefiningProgress>(mill).unwrap();
        prog.current = current;
        prog.max = 10.0;
    }

    #[test]
    fn live_placed_refiner_gets_byproduct_bin() {
        let mut world = placement_world();
        let mill = place_mill(&mut world, 5, 5);
        let crafting = world.get::<CraftingBuilding>(mill).unwrap_or_else(|| {
            panic!("a live-placed LumberMill must carry a CraftingBuilding (Spec 354)")
        });
        let recipe = crafting
            .current_recipe
            .as_ref()
            .expect("refiner recipe must be set");
        assert!(
            recipe
                .byproducts
                .iter()
                .any(|(r, n)| *r == ResourceType::Waste && *n >= 1),
            "refiner recipe must declare a Waste byproduct, got {:?}",
            recipe.byproducts
        );
        let inv = world
            .get::<Inventory>(mill)
            .expect("a live-placed LumberMill must carry a byproduct Inventory bin");
        assert!(inv.capacity > 0, "byproduct bin must have capacity");
    }

    #[test]
    fn live_full_sim_world_placed_refiner_gets_bin() {
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.wood = 100_000.0;
            res.stone = 100_000.0;
            res.max_wood = 100_000.0;
            res.max_planks = 100_000.0;
        }
        let mut placed = None;
        'outer: for x in 4..60 {
            for y in 4..60 {
                if try_place_building(&mut world, x, y, BuildingType::LumberMill) {
                    placed = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (mx, my) = placed.expect("a mill should be placeable somewhere in a live world");
        crate::simulation::run_simulation_tick(&mut world);
        let mut q = world.query::<(Entity, &GridPosition, &CraftingBuilding)>();
        assert!(
            q.iter(&world).any(|(_, p, _)| p.x == mx && p.y == my),
            "mill placed in a full live world must carry CraftingBuilding after a tick"
        );
    }

    #[test]
    fn live_refining_completion_fills_bin_not_ground() {
        let mut world = placement_world();
        let mut mills = Vec::new();
        for i in 0..60 {
            let (x, y) = (5 + (i % 10) * 3, 5 + (i / 10) * 3);
            if try_place_building(&mut world, x, y, BuildingType::LumberMill) {
                let mut q = world.query::<(Entity, &GridPosition)>();
                if let Some((e, _)) = q.iter(&world).find(|(_, p)| p.x == x && p.y == y) {
                    mills.push(e);
                    set_progress(&mut world, e, 9.9);
                    spawn_refine_worker(&mut world, x, y);
                }
            }
        }
        assert!(mills.len() >= 40, "need many mills, placed {}", mills.len());
        process_refining_system(&mut world);
        let res = world.resource::<ColonyResources>();
        assert!(res.planks >= 40.0, "mills should complete, planks={}", res.planks);
        let mut binned = 0u32;
        for e in &mills {
            if let Some(inv) = world.get::<Inventory>(*e) {
                binned += inv.get_amount(&ResourceType::Waste);
            }
        }
        assert!(
            binned >= 1,
            "refining completions must generate Waste byproduct into live bins"
        );
        let ground_waste = world
            .query::<&ResourceItem>()
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::Waste)
            .count();
        assert_eq!(
            ground_waste, 0,
            "binned refiners capture their byproduct; ground spawn is the no-bin fallback"
        );
    }

    #[test]
    fn live_full_bin_blocks_refining_completion() {
        let mut world = placement_world();
        let mill = place_mill(&mut world, 5, 5);
        // Fill the bin to capacity with waste.
        {
            let mut inv = world.get_mut::<Inventory>(mill).unwrap();
            let cap = inv.capacity;
            inv.add(ResourceType::Waste, cap);
        }
        set_progress(&mut world, mill, 9.9);
        spawn_refine_worker(&mut world, 5, 5);
        let wood_before = world.resource::<ColonyResources>().wood;
        process_refining_system(&mut world);
        let res = world.resource::<ColonyResources>();
        assert_eq!(
            res.wood, wood_before,
            "a waste-choked mill must not consume inputs"
        );
        assert_eq!(res.planks, 0.0, "a waste-choked mill must not produce");
        let prog = world.get::<RefiningProgress>(mill).unwrap();
        assert!(
            prog.current >= prog.max,
            "choked mill stays stuck at full progress, got {}",
            prog.current
        );
    }

    #[test]
    fn live_dump_waste_drains_bin_to_ground_and_beauty() {
        let mut world = World::new();
        let mut beauty = BeautyGrid::new(20, 20);
        beauty.set(5, 5, 10.0);
        world.insert_resource(beauty);
        let mill = world
            .spawn((
                CraftingBuilding {
                    current_recipe: Some(Recipe {
                        inputs: vec![],
                        outputs: vec![(ResourceType::Planks, 1)],
                        byproducts: vec![(ResourceType::Waste, 1)],
                        work_required: 10.0,
                    }),
                    work_progress: 0.0,
                },
                Inventory {
                    capacity: 20,
                    items: vec![(ResourceType::Waste, 5)],
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();
        world.spawn(DumpWasteAction {
            x: 5,
            y: 5,
            amount: 5,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(waste_dumping_system);
        schedule.run(&mut world);
        world.clear_trackers();
        let inv = world.get::<Inventory>(mill).unwrap();
        assert_eq!(
            inv.get_amount(&ResourceType::Waste),
            0,
            "dumping must drain the building's byproduct bin"
        );
        let ground: f32 = world
            .query::<(&ResourceItem, &GridPosition)>()
            .iter(&world)
            .filter(|(i, p)| i.resource_type == ResourceType::Waste && p.x == 5 && p.y == 5)
            .map(|(i, _)| i.amount)
            .sum();
        assert_eq!(
            ground, 5.0,
            "dumped waste must become a real ground Waste item (haulable, beauty-emitting)"
        );
        assert!(
            world.resource::<BeautyGrid>().get(5, 5) < 10.0,
            "dumping must degrade the real BeautyGrid tile"
        );
    }
}

#[cfg(test)]
mod observability_tests {
    use super::*;

    #[test]
    fn waste_summary_counts_all_live_seams() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            waste: 3.0,
            max_waste: 100.0,
            ..Default::default()
        });
        world.spawn((
            ResourceItem {
                resource_type: ResourceType::Waste,
                amount: 2.0,
            },
            GridPosition { x: 1, y: 1 },
        ));
        world.spawn((
            CraftingBuilding {
                current_recipe: recipe_for_building_type(BuildingType::LumberMill),
                work_progress: 0.0,
            },
            Inventory {
                capacity: BYPRODUCT_BIN_CAPACITY,
                items: vec![(ResourceType::Waste, BYPRODUCT_BIN_CAPACITY)],
            },
            GridPosition { x: 2, y: 2 },
        ));
        let summary = waste_summary(&mut world);
        assert_eq!(summary.stored, 3.0);
        assert_eq!(summary.stored_cap, 100.0);
        assert_eq!(summary.ground, 2.0);
        assert_eq!(summary.ground_stacks, 1);
        assert_eq!(summary.binned, BYPRODUCT_BIN_CAPACITY);
        assert_eq!(summary.bins, 1);
        assert_eq!(summary.choked_bins, 1, "a full bin must read as choked");
        assert_eq!(waste_total(&mut world), 3.0 + 2.0 + 20.0);
    }

    #[test]
    fn ip_guard_no_banned_terms_in_public_strings() {
        let summary = WasteSummary {
            stored: 1.0,
            stored_cap: 100.0,
            ground: 2.0,
            ground_stacks: 1,
            binned: 3,
            bins: 2,
            choked_bins: 1,
        };
        let public = format_waste_summary(&summary);
        // Distinctive borrowed-IP terms from the creative palette must
        // never leak into public strings; mechanics stay generic.
        let banned = [
            "Shimmer", "Area X", "Southern Reach", "Hyperion", "Shrike",
            "Foundation", "psychohistory", "ansible", "Ender", "Buggers",
            "Hitchhiker", "Babel fish", "Slag-heaps of Mordor",
        ];
        for term in banned {
            assert!(
                !public.contains(term),
                "banned term '{term}' in public string: {public}"
            );
        }
    }
}

#[cfg(test)]
mod live_wiring_116_tests {
    //! Item 116 live-wiring tests: Recycler + Office byproduct bins.
    //! Recycler's live seam is `recycle_processing_system` (its refining
    //! recipe is dead: no RefiningProgress at placement, bio-waste/paste
    //! caps stuck at 0, ledger ignores both fields). Office's refining
    //! seam is real but unwired (no RefiningProgress at placement).

    use super::*;
    use crate::layer1::building::{try_place_building, Building, BuildingMap, BuildingType, OccupiedTiles};
    use crate::layer1::pop::Pop;
    use crate::layer1::refining::process_refining_system;
    use crate::layer1::resources::{ColonyResources, RefiningProgress, ResourceItem};
    use crate::layer1::terrain::{TerrainGrid, TerrainType};
    use crate::layer1::utility_ai::{ActionType, PopAction};
    use crate::layer1::GridPosition;

    fn placement_world_116() -> World {
        let mut world = World::new();
        world.insert_resource(TerrainGrid {
            width: 100,
            height: 100,
            tiles: vec![TerrainType::Grass; 100 * 100],
        });
        world.insert_resource(OccupiedTiles::default());
        world.insert_resource(BuildingMap::default());
        world.init_resource::<Events<crate::layer1::ancestral_graves::SacrilegeEvent>>();
        world.init_resource::<Events<crate::layer1::core::events::BuildingRemovedEvent>>();
        world.init_resource::<Events<crate::layer1::core::events::BuildingCompletedEvent>>();
        // TechState::default() has total_capacity 0, and unlock() runs
        // update_corruption — a tech unlocked into zero capacity is
        // corrupted instantly. Grant capacity first (same pattern as
        // the tech module's own unlock test).
        let mut tech = crate::layer1::tech::TechState {
            total_capacity: 100.0,
            ..Default::default()
        };
        tech.unlock(crate::layer1::tech::Tech::Medical);
        world.insert_resource(tech);
        world.insert_resource(ColonyResources {
            wood: 100_000.0,
            stone: 100_000.0,
            ore: 100_000.0,
            metal: 100_000.0,
            max_wood: 100_000.0,
            max_planks: 100_000.0,
            ..Default::default()
        });
        world
    }

    fn place(world: &mut World, x: i32, y: i32, bt: BuildingType) -> Entity {
        assert!(
            try_place_building(world, x, y, bt),
            "placement of {bt:?} at ({x},{y}) should succeed"
        );
        let mut q = world.query::<(Entity, &GridPosition, &Building)>();
        q.iter(world)
            .find(|(_, p, b)| p.x == x && p.y == y && b.building_type == bt)
            .map(|(e, _, _)| e)
            .expect("placed building entity")
    }

    fn spawn_refine_worker(world: &mut World, x: i32, y: i32) {
        world.spawn((
            Pop,
            GridPosition { x, y },
            PopAction {
                current: ActionType::Refine,
                ..Default::default()
            },
        ));
    }

    #[test]
    fn live_placed_recycler_gets_byproduct_bin() {
        let mut world = placement_world_116();
        let rec = place(&mut world, 5, 5, BuildingType::Recycler);
        let crafting = world.get::<CraftingBuilding>(rec).unwrap_or_else(|| {
            panic!("a live-placed Recycler must carry a CraftingBuilding (Item 116)")
        });
        let recipe = crafting
            .current_recipe
            .as_ref()
            .expect("recycler recipe must be set");
        assert!(
            recipe
                .byproducts
                .iter()
                .any(|(r, n)| *r == ResourceType::Waste && *n >= 1),
            "recycler recipe must declare a Waste byproduct, got {:?}",
            recipe.byproducts
        );
        let bin = world
            .get::<Inventory>(rec)
            .expect("a live-placed Recycler must carry a byproduct Inventory bin");
        assert_eq!(bin.capacity, BYPRODUCT_BIN_CAPACITY);
        // The recycler's physical-item inventory (its live input seam)
        // must still be present alongside the byproduct bin.
        assert!(
            world
                .get::<crate::layer1::inventory::Inventory>(rec)
                .is_some(),
            "placed Recycler must keep its physical Inventory for hauler delivery"
        );
        assert!(world.get::<crate::layer1::recycling::Recycler>(rec).is_some());
    }

    #[test]
    fn live_placed_office_gets_bin_and_refining_progress() {
        let mut world = placement_world_116();
        let office = place(&mut world, 5, 5, BuildingType::Office);
        assert!(
            world.get::<Inventory>(office).is_some(),
            "a live-placed Office must carry a byproduct Inventory bin (Item 116)"
        );
        assert!(
            world.get::<CraftingBuilding>(office).is_some(),
            "a live-placed Office must carry a CraftingBuilding (Item 116)"
        );
        assert!(
            world.get::<RefiningProgress>(office).is_some(),
            "a live-placed Office must carry RefiningProgress so its real refining seam can run (Item 116)"
        );
        // Admin function must survive the wiring.
        assert!(world.get::<crate::layer1::admin::Office>(office).is_some());
        assert!(
            world
                .get::<crate::layer1::admin::AdminProvider>(office)
                .is_some()
        );
    }

    #[test]
    fn live_office_refining_completion_fills_bin_not_ground() {
        let mut world = placement_world_116();
        let mut offices = Vec::new();
        for i in 0..80 {
            let (x, y) = (5 + (i % 9) * 3, 5 + (i / 9) * 3);
            if try_place_building(&mut world, x, y, BuildingType::Office) {
                let found = {
                    let mut q = world.query::<(Entity, &GridPosition, &Building)>();
                    q.iter(&world)
                        .find(|(_, p, b)| {
                            p.x == x && p.y == y && b.building_type == BuildingType::Office
                        })
                        .map(|(e, _, _)| e)
                };
                if let Some(e) = found {
                    offices.push(e);
                    {
                        let mut prog = world.get_mut::<RefiningProgress>(e).unwrap();
                        prog.current = 9.9;
                        prog.max = 10.0;
                    }
                    spawn_refine_worker(&mut world, x, y);
                }
            }
        }
        assert!(offices.len() >= 60, "need many offices, placed {}", offices.len());
        process_refining_system(&mut world);
        let permits = world
            .query::<&ResourceItem>()
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::BuildingPermit)
            .count();
        assert!(permits >= 40, "offices should complete and spawn permits, got {permits}");
        let mut binned = 0u32;
        for e in &offices {
            if let Some(inv) = world.get::<Inventory>(*e) {
                binned += inv.get_amount(&ResourceType::Waste);
            }
        }
        assert!(
            binned >= 1,
            "office refining completions must generate Waste byproduct into live bins (0.1 chance x many offices)"
        );
        let ground_waste = world
            .query::<&ResourceItem>()
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::Waste)
            .count();
        assert_eq!(
            ground_waste, 0,
            "binned offices capture their byproduct; ground spawn is the no-bin fallback"
        );
    }

    #[test]
    fn live_full_bin_blocks_office_refining_completion() {
        let mut world = placement_world_116();
        let office = place(&mut world, 5, 5, BuildingType::Office);
        {
            let mut inv = world.get_mut::<Inventory>(office).unwrap();
            let cap = inv.capacity;
            inv.add(ResourceType::Waste, cap);
        }
        {
            let mut prog = world.get_mut::<RefiningProgress>(office).unwrap();
            prog.current = 9.9;
            prog.max = 10.0;
        }
        spawn_refine_worker(&mut world, 5, 5);
        let wood_before = world.resource::<ColonyResources>().wood;
        process_refining_system(&mut world);
        let res = world.resource::<ColonyResources>();
        assert_eq!(res.wood, wood_before, "a waste-choked office must not consume inputs");
        let permits = world
            .query::<&ResourceItem>()
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::BuildingPermit)
            .count();
        assert_eq!(permits, 0, "a waste-choked office must not produce permits");
        let prog = world.get::<RefiningProgress>(office).unwrap();
        assert!(
            prog.current >= prog.max,
            "choked office stays stuck at full progress, got {}",
            prog.current
        );
    }

    #[test]
    fn live_placed_recycler_processing_fills_bin() {
        let mut world = placement_world_116();
        let rec = place(&mut world, 5, 5, BuildingType::Recycler);
        {
            // Raise the bin capacity for this test only: at 0.5/item the
            // stock cap-20 bin legitimately chokes mid-batch (that
            // behavior has its own test below); here we isolate capture.
            let mut bin = world.get_mut::<Inventory>(rec).unwrap();
            bin.capacity = 10_000;
        }
        {
            let mut inv = world
                .get_mut::<crate::layer1::inventory::Inventory>(rec)
                .unwrap();
            inv.capacity = 100;
            inv.items = (0..40)
                .map(|_| crate::layer1::inventory::InventoryItem {
                    item_type: crate::layer1::items::ItemType::Waste,
                    entity: None,
                })
                .collect();
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(crate::layer1::recycling::recycle_processing_system);
        schedule.run(&mut world);
        let res = world.resource::<ColonyResources>();
        assert!(
            (res.rations - 20.0).abs() < f32::EPSILON,
            "40 waste items should recycle into 20 rations, got {}",
            res.rations
        );
        let bin = world.get::<Inventory>(rec).expect("recycler bin");
        assert!(
            bin.get_amount(&ResourceType::Waste) >= 1,
            "recycling completions must capture Waste byproduct into the bin (0.5 chance x 40 items)"
        );
        let ground_waste = world
            .query::<&ResourceItem>()
            .iter(&world)
            .filter(|i| i.resource_type == ResourceType::Waste)
            .count();
        assert_eq!(ground_waste, 0, "binned recycler captures its byproduct");
    }

    #[test]
    fn live_placed_recycler_full_bin_blocks_processing() {
        let mut world = placement_world_116();
        let rec = place(&mut world, 5, 5, BuildingType::Recycler);
        {
            let mut bin = world.get_mut::<Inventory>(rec).unwrap();
            let cap = bin.capacity;
            bin.add(ResourceType::Waste, cap);
        }
        {
            let mut inv = world
                .get_mut::<crate::layer1::inventory::Inventory>(rec)
                .unwrap();
            inv.items = (0..2)
                .map(|_| crate::layer1::inventory::InventoryItem {
                    item_type: crate::layer1::items::ItemType::Waste,
                    entity: None,
                })
                .collect();
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(crate::layer1::recycling::recycle_processing_system);
        schedule.run(&mut world);
        let res = world.resource::<ColonyResources>();
        assert_eq!(
            res.rations, 0.0,
            "a waste-choked recycler must not process items until its bin is dumped"
        );
        let inv = world
            .get::<crate::layer1::inventory::Inventory>(rec)
            .unwrap();
        assert_eq!(inv.items.len(), 2, "choked recycler leaves its input items unprocessed");
    }

    #[test]
    fn live_recycler_dump_drains_bin_to_ground() {
        let mut world = placement_world_116();
        let mut beauty = BeautyGrid::new(20, 20);
        beauty.set(5, 5, 10.0);
        world.insert_resource(beauty);
        let rec = place(&mut world, 5, 5, BuildingType::Recycler);
        {
            let mut bin = world.get_mut::<Inventory>(rec).unwrap();
            bin.add(ResourceType::Waste, 5);
        }
        world.spawn(DumpWasteAction { x: 5, y: 5, amount: 5 });
        let mut schedule = Schedule::default();
        schedule.add_systems(waste_dumping_system);
        schedule.run(&mut world);
        let bin = world.get::<Inventory>(rec).unwrap();
        assert_eq!(bin.get_amount(&ResourceType::Waste), 0, "dumping must drain the recycler bin");
        let ground: f32 = world
            .query::<(&ResourceItem, &GridPosition)>()
            .iter(&world)
            .filter(|(i, p)| i.resource_type == ResourceType::Waste && p.x == 5 && p.y == 5)
            .map(|(i, _)| i.amount)
            .sum();
        assert_eq!(ground, 5.0, "dumped recycler waste must become a real ground Waste item");
    }

    #[test]
    fn live_full_sim_world_placed_recycler_and_office_get_bins() {
        let mut world = crate::setup::setup_world_with_config(crate::setup::SetupConfig::default());
        {
            let mut res = world.resource_mut::<ColonyResources>();
            res.wood = 100_000.0;
            res.stone = 100_000.0;
            res.metal = 100_000.0;
            res.max_wood = 100_000.0;
        }
        if let Some(mut tech) = world.get_resource_mut::<crate::layer1::tech::TechState>() {
            tech.total_capacity = tech.total_capacity.max(100.0);
            tech.unlock(crate::layer1::tech::Tech::Medical);
        }
        let mut placed_office = None;
        let mut placed_recycler = None;
        'outer: for x in 4..60 {
            for y in 4..60 {
                if placed_office.is_none()
                    && try_place_building(&mut world, x, y, BuildingType::Office)
                {
                    placed_office = Some((x, y));
                }
                if placed_recycler.is_none()
                    && try_place_building(&mut world, x, y, BuildingType::Recycler)
                {
                    placed_recycler = Some((x, y));
                }
                if placed_office.is_some() && placed_recycler.is_some() {
                    break 'outer;
                }
            }
        }
        let (ox, oy) = placed_office.expect("an office should be placeable in a live world");
        let (rx, ry) = placed_recycler.expect("a recycler should be placeable in a live world");
        crate::simulation::run_simulation_tick(&mut world);
        let mut q = world.query::<(&GridPosition, &Inventory)>();
        let positions: Vec<(i32, i32)> = q.iter(&world).map(|(p, _)| (p.x, p.y)).collect();
        assert!(
            positions.contains(&(ox, oy)),
            "office placed in a full live world must carry a byproduct bin after a tick"
        );
        assert!(
            positions.contains(&(rx, ry)),
            "recycler placed in a full live world must carry a byproduct bin after a tick"
        );
    }
}

