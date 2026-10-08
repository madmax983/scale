//! The 'Recalled' Product (Spec 1373).
//!
//! Corporate negligence on a galactic scale: manufacturers issue recall
//! notices for defective products. Using a recalled item risks critical
//! failure — poisoned rations or exploding tools. Returning stock grants
//! credits but leaves you without the goods.
//!
//! Safety (return it) vs. necessity (use it anyway).

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use rand::Rng;
use std::collections::HashSet;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::inflation::EmpireResources;use crate::layer1::economy::items::ItemType;
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::shields::DamageEvent;

/// Default chance a recalled item critically fails when used.
pub const DEFAULT_FAILURE_CHANCE: f32 = 0.05;
/// Default credits granted per returned recalled item.
pub const DEFAULT_CREDITS_PER_ITEM: f32 = 5.0;
/// Damage from tainted food (poison) — low velocity, bypasses kinetic barriers.
pub const POISON_DAMAGE: f32 = 15.0;
/// Damage from defective non-food goods (explosion) — high velocity, shields help.
pub const EXPLOSION_DAMAGE: f32 = 25.0;
/// Velocity tag for explosions (kinetic barriers absorb high-velocity hits).
pub const EXPLOSION_VELOCITY: f32 = 100.0;

/// Chance per tick of a spontaneous manufacturer recall.
pub const AUTO_RECALL_CHANCE: f32 = 0.0005;

/// Manufactured products subject to corporate recalls.
/// (Negligence hits factory goods, not raw crops.)
const RECALLABLE_PRODUCTS: &[ItemType] = &[
    ItemType::Rations,
    ItemType::NutrientPaste,
    ItemType::Stim,
    ItemType::Sedative,
    ItemType::Alcohol,
    ItemType::LuxuryMeal,
];

/// Tracks which product types are currently under manufacturer recall.
#[derive(Resource, Default, Debug)]
pub struct RecallManager {
    /// Item types currently recalled.
    pub recalled: HashSet<ItemType>,
}

impl RecallManager {
    /// Returns true if the item type is currently recalled.
    pub fn is_recalled(&self, item: &ItemType) -> bool {
        self.recalled.contains(item)
    }

    /// Issues a recall. Returns true if this is a new recall.
    pub fn issue(&mut self, item: ItemType) -> bool {
        self.recalled.insert(item)
    }

    /// Lifts a recall. Returns true if one was active.
    pub fn lift(&mut self, item: &ItemType) -> bool {
        self.recalled.remove(item)
    }
}

/// Tuning knobs (and deterministic test hook) for the recall mechanic.
#[derive(Resource, Debug, Clone)]
pub struct RecallConfig {
    /// Chance [0,1] a recalled item critically fails on use.
    pub failure_chance: f32,
    /// Credits granted per returned recalled item.
    pub credits_per_item: f32,
    /// Chance [0,1] per tick of a spontaneous manufacturer recall.
    pub auto_recall_chance: f32,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            failure_chance: DEFAULT_FAILURE_CHANCE,
            credits_per_item: DEFAULT_CREDITS_PER_ITEM,
            auto_recall_chance: AUTO_RECALL_CHANCE,
        }
    }
}

/// A manufacturer issues a recall notice for a product type.
#[derive(Event, Debug, Clone)]
pub struct IssueRecallEvent {
    /// The product type being recalled.
    pub item_type: ItemType,
    /// Who made it (for the chronicle notice).
    pub manufacturer: String,
    /// Why it's defective.
    pub reason: String,
}

/// A pop uses or consumes an item (eats food, wields a tool, ...).
#[derive(Event, Debug, Clone)]
pub struct UseItemEvent {
    /// The pop using the item.
    pub user: Entity,
    /// What they're using.
    pub item_type: ItemType,
}

/// Return recalled stock to the manufacturer for credits.
#[derive(Event, Debug, Clone)]
pub struct ReturnRecalledItemEvent {
    /// The product type being returned.
    pub item_type: ItemType,
    /// How many units to return.
    pub quantity: u32,
}

/// Human-readable name for an item type (headless UI + chronicles).
pub fn item_display_name(item: &ItemType) -> String {
    format!("{item:?}")
}

/// True for anything a pop might eat (poison vector); false for tools etc.
/// (explosion vector).
fn is_foodlike(item: &ItemType) -> bool {
    matches!(
        item.as_resource_type(),
        Some(ResourceType::Food | ResourceType::Rations)
    ) || matches!(item, ItemType::NutrientPaste)
}

/// Parse a headless-console item name into an ItemType.
pub fn parse_recallable_item(name: &str) -> Option<ItemType> {
    match name.to_lowercase().as_str() {
        "rations" => Some(ItemType::Rations),
        "nutrient_paste" | "paste" => Some(ItemType::NutrientPaste),
        "potato" => Some(ItemType::Potato),
        "wheat" => Some(ItemType::Wheat),
        "rice" => Some(ItemType::Rice),
        "corn" => Some(ItemType::Corn),
        "soy" => Some(ItemType::Soy),
        "meat" => Some(ItemType::Meat),
        "fish" => Some(ItemType::Fish),
        "fruit" => Some(ItemType::Fruit),
        "luxury_meal" | "luxurymeal" => Some(ItemType::LuxuryMeal),
        "mystery_meal" | "mysterymeal" => Some(ItemType::MysteryMeal),
        "stim" => Some(ItemType::Stim),
        "sedative" => Some(ItemType::Sedative),
        "alcohol" => Some(ItemType::Alcohol),
        "xenoflora" => Some(ItemType::Xenoflora),
        "tool" | "tools" => Some(ItemType::Tool),
        _ => None,
    }
}

/// Remove up to `quantity` units of stock from the matching colony pool.
/// Returns the actual number of units removed.
pub fn remove_stock(resources: &mut ColonyResources, item_type: &ItemType, quantity: u32) -> u32 {
    let pool: &mut f32 = match item_type {
        ItemType::Rations => &mut resources.rations,
        ItemType::NutrientPaste => &mut resources.nutrient_paste,
        ItemType::Tool => &mut resources.tools,
        t if matches!(t.as_resource_type(), Some(ResourceType::Food)) => &mut resources.food,
        _ => return 0,
    };
    let removed = (*pool).min(quantity as f32).max(0.0);
    *pool -= removed;
    removed as u32
}

/// Records recall notices in the RecallManager and the chronicle.
pub fn process_recall_issuance_system(
    mut events: EventReader<IssueRecallEvent>,
    mut manager: ResMut<RecallManager>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        if manager.issue(ev.item_type) {
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "RECALL NOTICE: {} has recalled all {} — {}. Use at your own risk; return stock for credits.",
                    ev.manufacturer,
                    item_display_name(&ev.item_type),
                    ev.reason
                ),
                importance: EventImportance::Major,
            });
        }
    }
}

/// Applies the failure chance when recalled items are used.
///
/// Food-like items poison (low velocity: kinetic barriers can't stop
/// ingested toxins); manufactured goods explode (high velocity: shields
/// help). Failure hooks into the existing DamageEvent stream.
pub fn process_item_usage_system(
    mut events: EventReader<UseItemEvent>,
    manager: Res<RecallManager>,
    config: Res<RecallConfig>,
    mut damage: EventWriter<DamageEvent>,
) {
    let mut rng = rand::thread_rng();
    for ev in events.read() {
        if !manager.is_recalled(&ev.item_type) {
            continue;
        }
        if rng.gen_bool(config.failure_chance as f64) {
            let (amount, velocity) = if is_foodlike(&ev.item_type) {
                (POISON_DAMAGE, 0.0)
            } else {
                (EXPLOSION_DAMAGE, EXPLOSION_VELOCITY)
            };
            damage.send(DamageEvent {
                target: ev.user,
                amount,
                velocity,
            });
        }
    }
}

/// Returns recalled stock for credits. Only pays for stock actually on hand.
pub fn process_item_return_system(
    mut events: EventReader<ReturnRecalledItemEvent>,
    manager: Res<RecallManager>,
    config: Res<RecallConfig>,
    mut empires: Query<&mut EmpireResources>,
    mut stockpile: Option<ResMut<ColonyResources>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        if !manager.is_recalled(&ev.item_type) {
            continue;
        }
        let returned = stockpile
            .as_mut()
            .map(|s| remove_stock(s, &ev.item_type, ev.quantity))
            .unwrap_or(0);
        if returned == 0 {
            continue;
        }
        let payout = returned as f32 * config.credits_per_item;
        for mut empire in empires.iter_mut() {
            empire.credits += payout;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Returned {} recalled {} for {:.0} credits.",
                returned,
                item_display_name(&ev.item_type),
                payout
            ),
            importance: EventImportance::Minor,
        });
    }
}

/// Occasionally, a manufacturer issues a recall on its own — the
/// "recall notice during a siege" emergence. Only targets manufactured
/// goods, never raw crops.
pub fn auto_recall_system(
    mut events: EventWriter<IssueRecallEvent>,
    manager: Res<RecallManager>,
    config: Res<RecallConfig>,
) {
    let mut rng = rand::thread_rng();
    if !rng.gen_bool(config.auto_recall_chance as f64) {
        return;
    }
    let candidates: Vec<ItemType> = RECALLABLE_PRODUCTS
        .iter()
        .copied()
        .filter(|t| !manager.is_recalled(t))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let item_type = candidates[rng.gen_range(0..candidates.len())];
    let (manufacturer, reason) = match item_type {
        ItemType::Rations => ("OmniNutri Corp", "bacterial contamination in vat 7"),
        ItemType::NutrientPaste => ("OmniNutri Corp", "miscalibrated nutrient mix"),
        ItemType::Stim => ("Ferrum Works", "adrenal overstimulation risk"),
        ItemType::Sedative => ("Ferrum Works", "excessive drowsiness reports"),
        ItemType::Alcohol => ("Void Distilleries", "methanol contamination"),
        _ => ("Generic Manufacturing", "quality control failure"),
    };
    events.send(IssueRecallEvent {
        item_type,
        manufacturer: manufacturer.to_string(),
        reason: reason.to_string(),
    });
}

/// Registers the recall events, resources, and systems on an App (tests).
pub fn register_recall(app: &mut App) {
    app.init_resource::<RecallManager>();
    app.init_resource::<RecallConfig>();
    app.add_event::<IssueRecallEvent>();
    app.add_event::<UseItemEvent>();
    app.add_event::<ReturnRecalledItemEvent>();
    app.add_event::<DamageEvent>();
    app.add_event::<AddChronicleEvent>();
    app.add_systems(
        Update,
        (
            process_recall_issuance_system,
            process_item_usage_system,
            process_item_return_system,
            auto_recall_system,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_app() -> App {
        let mut app = App::new();
        register_recall(&mut app);
        // EmpireResources is a Component; spawn one holder.
        app.world_mut().spawn(EmpireResources {
            credits: 0.0,
            alloys: 0,
        });
        app.world_mut().insert_resource(ColonyResources::zeroed());
        app
    }

    fn empire_credits(app: &App) -> f32 {
        let world = app.world();
        let mut found = 0.0;
        // EmpireResources is a Component; find the holder via direct entity scan.
        for entity_ref in world.iter_entities() {
            if let Some(res) = world.get::<EmpireResources>(entity_ref.id()) {
                found = res.credits;
                break;
            }
        }
        found
    }

    #[test]
    fn test_issuing_recall_marks_item_recalled() {
        let mut app = setup_app();
        app.world_mut().send_event(IssueRecallEvent {
            item_type: ItemType::Rations,
            manufacturer: "OmniNutri Corp".to_string(),
            reason: "bacterial contamination".to_string(),
        });
        app.update();
        let manager = app.world().resource::<RecallManager>();
        assert!(manager.is_recalled(&ItemType::Rations));
        assert!(!manager.is_recalled(&ItemType::Potato));
    }

    #[test]
    fn test_using_recalled_item_can_fail() {
        let mut app = setup_app();
        // Deterministic: force the failure.
        app.world_mut().resource_mut::<RecallConfig>().failure_chance = 1.0;
        let pop = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(IssueRecallEvent {
            item_type: ItemType::Rations,
            manufacturer: "OmniNutri Corp".to_string(),
            reason: "bacterial contamination".to_string(),
        });
        app.update();
        app.world_mut().send_event(UseItemEvent {
            user: pop,
            item_type: ItemType::Rations,
        });
        app.update();
        // A DamageEvent must have been emitted for the user.
        let damage = app.world_mut().resource_mut::<Events<DamageEvent>>();
        let mut reader = damage.get_cursor();
        let hits: Vec<_> = reader.read(&damage).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, pop);
    }

    #[test]
    fn test_using_safe_item_never_fails() {
        let mut app = setup_app();
        app.world_mut().resource_mut::<RecallConfig>().failure_chance = 1.0;
        let pop = app.world_mut().spawn_empty().id();
        // Potato is NOT recalled.
        app.world_mut().send_event(UseItemEvent {
            user: pop,
            item_type: ItemType::Potato,
        });
        app.update();
        let damage = app.world_mut().resource_mut::<Events<DamageEvent>>();
        let mut reader = damage.get_cursor();
        assert_eq!(reader.read(&damage).count(), 0);
    }

    #[test]
    fn test_food_failure_is_poison_low_velocity() {
        let mut app = setup_app();
        app.world_mut().resource_mut::<RecallConfig>().failure_chance = 1.0;
        let pop = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(IssueRecallEvent {
            item_type: ItemType::Rations,
            manufacturer: "OmniNutri Corp".to_string(),
            reason: "bacterial contamination".to_string(),
        });
        app.update();
        app.world_mut().send_event(UseItemEvent {
            user: pop,
            item_type: ItemType::Rations,
        });
        app.update();
        let damage = app.world_mut().resource_mut::<Events<DamageEvent>>();
        let mut reader = damage.get_cursor();
        let hits: Vec<_> = reader.read(&damage).collect();
        assert_eq!(hits.len(), 1);
        // Poison: low velocity so kinetic barriers don't absorb it.
        assert!(hits[0].velocity < 1.0);
        assert!((hits[0].amount - POISON_DAMAGE).abs() < f32::EPSILON);
    }

    #[test]
    fn test_tool_failure_is_explosion() {
        let mut app = setup_app();
        app.world_mut().resource_mut::<RecallConfig>().failure_chance = 1.0;
        let pop = app.world_mut().spawn_empty().id();
        app.world_mut().send_event(IssueRecallEvent {
            item_type: ItemType::Tool,
            manufacturer: "Ferrum Works".to_string(),
            reason: "fractured power cells".to_string(),
        });
        app.update();
        app.world_mut().send_event(UseItemEvent {
            user: pop,
            item_type: ItemType::Tool,
        });
        app.update();
        let damage = app.world_mut().resource_mut::<Events<DamageEvent>>();
        let mut reader = damage.get_cursor();
        let hits: Vec<_> = reader.read(&damage).collect();
        assert_eq!(hits.len(), 1);
        // Explosion: high velocity, shields can absorb.
        assert!(hits[0].velocity >= EXPLOSION_VELOCITY);
        assert!((hits[0].amount - EXPLOSION_DAMAGE).abs() < f32::EPSILON);
    }

    #[test]
    fn test_returning_recalled_stock_grants_credits() {
        let mut app = setup_app();
        // Stock the pool first: 10 rations.
        app.world_mut()
            .resource_mut::<ColonyResources>()
            .rations = 10.0;
        app.world_mut().send_event(IssueRecallEvent {
            item_type: ItemType::Rations,
            manufacturer: "OmniNutri Corp".to_string(),
            reason: "bacterial contamination".to_string(),
        });
        app.update();
        app.world_mut().send_event(ReturnRecalledItemEvent {
            item_type: ItemType::Rations,
            quantity: 10,
        });
        app.update();
        // 10 items × 5 credits.
        assert!((empire_credits(&app) - 50.0).abs() < f32::EPSILON);
        // Stock is gone.
        assert!(app.world().resource::<ColonyResources>().rations < f32::EPSILON);
    }

    #[test]
    fn test_returning_unrecalled_stock_grants_nothing() {
        let mut app = setup_app();
        app.world_mut()
            .resource_mut::<ColonyResources>()
            .rations = 10.0;
        // No recall issued for Rations.
        app.world_mut().send_event(ReturnRecalledItemEvent {
            item_type: ItemType::Rations,
            quantity: 10,
        });
        app.update();
        assert!((empire_credits(&app) - 0.0).abs() < f32::EPSILON);
        // Stock untouched.
        assert!((app.world().resource::<ColonyResources>().rations - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_recallable_item() {
        assert_eq!(parse_recallable_item("rations"), Some(ItemType::Rations));
        assert_eq!(parse_recallable_item("RATIONS"), Some(ItemType::Rations));
        assert_eq!(parse_recallable_item("paste"), Some(ItemType::NutrientPaste));
        assert_eq!(parse_recallable_item("stim"), Some(ItemType::Stim));
        assert_eq!(parse_recallable_item("nonsense"), None);
    }

    #[test]
    fn test_auto_recall_issues_manufactured_goods() {
        let mut app = setup_app();
        // Force the auto-recall to fire.
        app.world_mut()
            .resource_mut::<RecallConfig>()
            .auto_recall_chance = 1.0;
        app.update();
        let manager = app.world().resource::<RecallManager>();
        // Exactly one new recall, and it's a manufactured good.
        assert_eq!(manager.recalled.len(), 1);
        let recalled = manager.recalled.iter().next().unwrap();
        assert!(RECALLABLE_PRODUCTS.contains(recalled));
    }

    #[test]
    fn test_auto_recall_respects_existing_recalls() {
        let mut app = setup_app();
        app.world_mut()
            .resource_mut::<RecallConfig>()
            .auto_recall_chance = 1.0;
        // Recall everything first.
        for item in RECALLABLE_PRODUCTS {
            app.world_mut().send_event(IssueRecallEvent {
                item_type: *item,
                manufacturer: "Test".to_string(),
                reason: "test".to_string(),
            });
        }
        app.update();
        let before = app.world().resource::<RecallManager>().recalled.len();
        // Auto-recall fires but finds nothing new to recall.
        app.update();
        let after = app.world().resource::<RecallManager>().recalled.len();
        assert_eq!(before, after);
        assert_eq!(after, RECALLABLE_PRODUCTS.len());
    }
}
