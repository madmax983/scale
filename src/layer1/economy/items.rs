use crate::layer1::resources::ResourceType;
use bevy_ecs::prelude::*;

/// Marker component for an item entity.
#[derive(Component, Default, Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The specific type of the item.
    pub item_type: ItemType,
}

/// Type of tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolType {
    /// Used for mining.
    Pickaxe,
    /// Used for forestry.
    Axe,
    /// Used for building.
    Hammer, // For building
}

/// Component representing a physical tool with durability.
#[derive(Component, Debug, Clone)]
pub struct Tool {
    /// The type of tool.
    pub tool_type: ToolType,
    /// Current durability remaining.
    pub durability: f32,
    /// Maximum durability.
    pub max_durability: f32,
}

/// Component for equipment slots on a Pop.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Equipment {
    /// The entity ID of the equipped tool.
    pub tool: Option<Entity>,
    /// The entity ID of the equipped weapon.
    pub weapon: Option<Entity>,
    /// The entity ID of the equipped body clothing.
    pub body: Option<Entity>,
    /// The entity ID of the equipped headgear.
    pub head: Option<Entity>,
    /// The entity ID of the equipped totem.
    pub totem: Option<Entity>,
    pub mask: Option<Entity>,
    pub has_symbiote: bool,
}

/// Types of clothing items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClothingType {
    /// Basic tunic.
    Tunic,
    /// Warm parka.
    Parka,
}

/// Component representing a clothing item.
#[derive(Component, Debug, Clone)]
pub struct Clothing {
    /// The type of clothing.
    pub clothing_type: ClothingType,
    /// Insulation value (0.0 to 1.0).
    pub insulation: f32,
    /// Current durability remaining.
    pub durability: f32,
    /// Maximum durability.
    pub max_durability: f32,
}

/// Component representing a generic item being carried by a Pop.
#[derive(Component, Debug, Clone, Copy)]
pub struct CarryingItem(pub Entity);

/// Event triggered when an item is unequipped.
#[derive(Event, Debug, Clone)]
pub struct UnequipEvent {
    /// The entity performing the unequip action.
    pub actor: Entity,
    /// The item entity being unequipped.
    pub item: Entity,
    /// The slot from which the item was removed.
    pub slot: String,
}

/// Types of food items Pops can consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemType {
    /// Default generic item type (safe fallback).
    #[default]
    None,
    /// Tools (Pickaxe, etc.).
    Tool,
    /// Clothing items.
    Clothing,
    /// Cybernetic prosthetics.
    Prosthetic,
    /// Institutional memory manuals.
    Manual,
    CalciumAlloy,
    /// Default food type (e.g. from Farms).
    Potato,
    /// Grain crop.
    Wheat,
    /// Native root adopted when Earth seeds are gone (Spec 377).
    GrubRoot,
    /// Rice crop.
    Rice,
    /// Corn crop.
    Corn,
    NutrientPaste,
    /// Soy crop.
    Soy,
    /// Protein from animals.
    Meat,
    /// Protein from water.
    Fish,
    /// Gathered or grown fruit.
    Fruit,
    /// High-quality prepared food.
    LuxuryMeal,
    /// Alien meat variety A.
    AlienMeatA,
    /// Alien meat variety B.
    AlienMeatB,
    /// Glowing mushroom.
    GlowMushroom,
    /// A meal with unknown effects.
    MysteryMeal,
    /// A unique item produced by a hobby (e.g., "Wooden Duck").
    /// ⚡ Bolt Optimization: Made Copy to eliminate heap allocations
    Curio(&'static str),
    /// Xenoflora, an addictive native plant
    Xenoflora,
    /// A chemical stimulant that boosts speed but damages health.
    Stim,
    /// A chemical sedative that reduces stress but slows speed.
    Sedative,
    /// Alcohol (drink).
    Alcohol,
    /// Genetic sample from flora or fauna.
    GeneticSample,
    /// Recycled rations (Nutrient Paste).
    Rations,
    /// Industrial waste.
    Waste,
    /// A breathing apparatus.
    Rebreather,
    /// A corpse of a Pop.
    Corpse(Entity),
    /// A building permit document.
    BuildingPermit,
    /// A Memory Core extracted from a dead Pop.
    MemoryCore(Entity),
    /// A crystal that decays in light.
    ShadowCrystal,
    /// Harvested organs from dead pops.
    VitalOrgans,
    /// A living stone that moves.
    LivingStone,
    /// Scrap metal recovered from clutter.
    Scrap,
    /// Agony Extract harvested under extreme stress.
    AgonyExtract,
    /// Fermented luxury good from orbital stations.
    VoidAle,
    /// Highly valuable technology from the past.
    LostTech,
    /// Ancient archaeological artifact.
    Artifact,
}

impl ItemType {
    /// Maps the item type to a resource type, if applicable.
    ///
    /// Used for checking if an item is contraband (banned resource).
    #[must_use]
    pub const fn as_resource_type(&self) -> Option<ResourceType> {
        match self {
            Self::Alcohol => Some(ResourceType::Alcohol),
            Self::Potato
            | Self::GrubRoot
            | Self::Wheat
            | Self::Rice
            | Self::Corn
            | Self::Soy
            | Self::Meat
            | Self::Fish
            | Self::Fruit
            | Self::LuxuryMeal
            | Self::AlienMeatA
            | Self::AlienMeatB
            | Self::GlowMushroom
            | Self::MysteryMeal
            | Self::Xenoflora
            | Self::VoidAle => Some(ResourceType::Food),
            Self::Rations => Some(ResourceType::Rations),
            Self::Waste => Some(ResourceType::Waste),
            Self::BuildingPermit => Some(ResourceType::BuildingPermit),
            Self::LostTech => None,
            Self::AgonyExtract => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::designation::{Designation, DesignationType};
    use crate::layer1::execution::work_execution_system;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::resources::{ColonyResources, MiningProgress};
    use crate::layer1::utility_ai::{ActionType, PopAction};

    #[test]
    fn test_item_type_default() {
        assert_eq!(ItemType::default(), ItemType::None);
    }

    #[test]
    fn test_equipment_component_exists() {
        let mut world = World::new();
        let entity = world.spawn((Pop, Equipment::default())).id();
        let eq = world.get::<Equipment>(entity).unwrap();
        assert!(eq.tool.is_none());
    }

    #[test]
    fn test_tool_item_component() {
        let mut world = World::new();
        let tool = world
            .spawn((
                Item::default(),
                Tool {
                    tool_type: ToolType::Pickaxe,
                    durability: 100.0,
                    max_durability: 100.0,
                },
            ))
            .id();

        let t = world.get::<Tool>(tool).unwrap();
        assert_eq!(t.durability, 100.0);
    }

    #[test]
    fn test_work_reduces_durability() {
        let mut world = World::new();
        // Setup World Resources
        world.insert_resource(ColonyResources::default());
        world.insert_resource(crate::layer1::edicts::ColonyPolicies::default());
        world.insert_resource(crate::shared::log::MessageLog::default());
        world.insert_resource(crate::layer1::terrain::TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![crate::layer1::terrain::TerrainType::Grass; 100],
        });

        // Spawn Tool
        let tool = world
            .spawn(Tool {
                tool_type: ToolType::Pickaxe,
                durability: 10.0,
                max_durability: 100.0,
            })
            .id();

        // Spawn Designation (Mine)
        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
                MiningProgress::default(),
            ))
            .id();

        // Spawn Pop with Equipment
        world.spawn((
            Pop,
            Equipment {
                tool: Some(tool),
                ..Default::default()
            },
            GridPosition { x: 5, y: 5 },
            // Add work components
            crate::layer1::execution::MovementTarget {
                target_entity: designation,
                target_position: GridPosition { x: 5, y: 5 },
                for_action: ActionType::Work,
            },
            crate::layer1::execution::AtTarget,
            PopAction {
                current: ActionType::Work,
                ..Default::default()
            },
        ));

        // Run execution system
        work_execution_system(&mut world);

        // Check durability reduced
        let t = world.get::<Tool>(tool).unwrap();
        assert!(
            t.durability < 10.0,
            "Durability should decrease (was {})",
            t.durability
        );
    }

    #[test]
    fn test_tool_breakage_removes_item() {
        let mut world = World::new();
        // Setup
        world.insert_resource(ColonyResources::default());
        world.insert_resource(crate::layer1::edicts::ColonyPolicies::default());
        world.insert_resource(crate::shared::log::MessageLog::default());
        world.insert_resource(crate::layer1::terrain::TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![crate::layer1::terrain::TerrainType::Grass; 100],
        });

        // Spawn Tool with VERY low durability
        let tool = world
            .spawn(Tool {
                tool_type: ToolType::Pickaxe,
                durability: 0.00001, // Almost broken, effectively 0 after any work
                max_durability: 100.0,
            })
            .id();

        let designation = world
            .spawn((
                Designation {
                    designation_type: DesignationType::Mine,
                },
                GridPosition { x: 5, y: 5 },
                MiningProgress::default(),
            ))
            .id();

        let pop = world
            .spawn((
                Pop,
                Equipment {
                    tool: Some(tool),
                    ..Default::default()
                },
                GridPosition { x: 5, y: 5 },
                crate::layer1::execution::MovementTarget {
                    target_entity: designation,
                    target_position: GridPosition { x: 5, y: 5 },
                    for_action: ActionType::Work,
                },
                crate::layer1::execution::AtTarget,
                PopAction {
                    current: ActionType::Work,
                    ..Default::default()
                },
            ))
            .id();

        // Force durability to 0 or simulate enough work to break it
        work_execution_system(&mut world);

        // Tool entity should be despawned
        assert!(
            world.get_entity(tool).is_err(),
            "Tool entity should be despawned"
        );

        // Pop equipment should be None
        let eq = world.get::<Equipment>(pop).unwrap();
        assert!(eq.tool.is_none(), "Pop equipment should be cleared");
    }
}

#[cfg(test)]
mod bolt_tests {
    use super::*;

    #[test]
    fn test_item_type_is_copy() {
        fn assert_is_copy<T: Copy>() {}
        assert_is_copy::<ItemType>();
    }
}
