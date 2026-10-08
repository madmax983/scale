//! The Artifact Market (Spec 1376).
//!
//! One colony's trash is an empire's treasure. Everyday items that survive
//! long enough gain [`HistoricalArtifact`] status: the first pickaxe, a
//! handwritten diary, the founder's old spacesuit. Wealthy collectors on the
//! core worlds pay exorbitant credits for them — but selling a piece of your
//! history tanks colony morale, and the chronicle remembers what you gave up.
//!
//! ## Tension
//!
//! Preserving cultural heritage vs. immediate survival: you are starving and
//! need credits for a food shipment — selling the founder's spacesuit saves
//! the colony but permanently wounds its spirit.
//!
//! ## Mechanics
//!
//! 1. **Aging**: every [`AGING_SCAN_PERIOD_TICKS`] ticks, [`item_aging_system`]
//!    ages all [`Item`] entities (stamping new ones with [`ItemAge`]) and
//!    promotes items past [`ARTIFACT_AGE_THRESHOLD_TICKS`] to
//!    [`HistoricalArtifact`], with a procedurally generated provenance name.
//! 2. **Museum aura**: a building designated a [`Museum`] (headless `museum`
//!    command) grants pops within [`MUSEUM_RADIUS`] tiles a passive morale
//!    aura scaling with housed artifacts — making the sell-vs-keep choice
//!    harder.
//! 3. **Sale**: [`SellArtifactEvent`] (headless `sell <id>`) despawns the
//!    artifact, pays appraised credits into [`EmpireResources`], hits every
//!    pop with a long "Heritage Sold" grief modifier, and writes a
//!    [`AddChronicleEvent`] documenting the sacrifice.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::layer1::architecture::building::Building;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::inflation::EmpireResources;
use crate::layer1::economy::items::{Item, ItemType};
use crate::layer1::pop::Pop;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::time::SimulationTime;

/// Ticks an item must survive before collectors consider it history.
pub const ARTIFACT_AGE_THRESHOLD_TICKS: u64 = 800;
/// How often the aging scan runs (ticks).
pub const AGING_SCAN_PERIOD_TICKS: u64 = 50;
/// Base sale price — core-world collectors pay exorbitantly.
pub const BASE_ARTIFACT_VALUE: f32 = 100.0;
/// Extra appraised value per tick of age.
pub const VALUE_PER_AGE_TICK: f32 = 0.5;
/// Colony-wide morale hit when heritage is sold ("Heritage Sold").
pub const SALE_MORALE_HIT: f32 = -0.25;
/// How long the sale grief lasts (ticks) — near-permanent.
pub const SALE_GRIEF_DURATION_TICKS: u32 = 2000;
/// Museum aura radius (Chebyshev tiles).
pub const MUSEUM_RADIUS: i32 = 6;
/// Morale aura per housed artifact.
pub const MUSEUM_AURA_PER_ARTIFACT: f32 = 0.03;
/// Cap on the museum aura bonus.
pub const MUSEUM_AURA_CAP: f32 = 0.15;
/// How often the museum aura refreshes (ticks).
pub const MUSEUM_AURA_PERIOD_TICKS: u64 = 100;
/// Duration of each aura application (ticks) — refreshed before expiry.
pub const MUSEUM_AURA_DURATION_TICKS: u32 = 120;

/// Tracks how long an [`Item`] entity has existed (ticks).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ItemAge {
    /// Ticks since the item was first seen by the aging scan.
    pub age_ticks: u64,
}

/// Marks an item recognized as a piece of colony history.
#[derive(Component, Debug, Clone)]
pub struct HistoricalArtifact {
    /// Procedural provenance name, e.g. "the Founder's Pickaxe".
    pub name: String,
    /// Tick when the item was promoted.
    pub promoted_tick: u64,
}

/// Marker component: this building is a museum housing artifacts.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Museum;

/// Tuning knobs for the artifact market.
#[derive(Resource, Debug, Clone)]
pub struct ArtifactMarketConfig {
    /// Ticks an item must survive before promotion.
    pub age_threshold_ticks: u64,
    /// How often the aging scan runs.
    pub scan_period_ticks: u64,
    /// Base sale price in credits.
    pub base_value: f32,
    /// Extra value per tick of age.
    pub value_per_age_tick: f32,
    /// Morale hit applied colony-wide on sale.
    pub sale_morale_hit: f32,
    /// Duration of the sale grief (ticks).
    pub sale_grief_duration_ticks: u32,
    /// Museum aura radius (Chebyshev tiles).
    pub museum_radius: i32,
    /// Morale aura per housed artifact.
    pub museum_aura_per_artifact: f32,
    /// Cap on the museum aura bonus.
    pub museum_aura_cap: f32,
    /// How often the museum aura refreshes.
    pub museum_aura_period_ticks: u64,
}

impl Default for ArtifactMarketConfig {
    fn default() -> Self {
        Self {
            age_threshold_ticks: ARTIFACT_AGE_THRESHOLD_TICKS,
            scan_period_ticks: AGING_SCAN_PERIOD_TICKS,
            base_value: BASE_ARTIFACT_VALUE,
            value_per_age_tick: VALUE_PER_AGE_TICK,
            sale_morale_hit: SALE_MORALE_HIT,
            sale_grief_duration_ticks: SALE_GRIEF_DURATION_TICKS,
            museum_radius: MUSEUM_RADIUS,
            museum_aura_per_artifact: MUSEUM_AURA_PER_ARTIFACT,
            museum_aura_cap: MUSEUM_AURA_CAP,
            museum_aura_period_ticks: MUSEUM_AURA_PERIOD_TICKS,
        }
    }
}

/// Provenance serials so each artifact of a type gets a distinct name.
#[derive(Resource, Debug, Default)]
pub struct ArtifactRegistry {
    /// How many artifacts of each item type have been promoted.
    pub promoted_per_type: HashMap<ItemType, u32>,
}

/// Request to sell a historical artifact to core-world collectors.
#[derive(Event, Debug, Clone)]
pub struct SellArtifactEvent {
    /// The artifact entity to sell.
    pub artifact: Entity,
}

/// Request to designate a building as a museum.
#[derive(Event, Debug, Clone)]
pub struct DesignateMuseumEvent {
    /// The building entity to designate.
    pub building: Entity,
}

/// Flavor title for an item type's artifacts (original names, concept-only).
pub fn artifact_title(item_type: &ItemType) -> &'static str {
    match item_type {
        ItemType::Tool => "Pickaxe",
        ItemType::Clothing => "Spacesuit",
        ItemType::Manual => "Handwritten Diary",
        ItemType::Prosthetic => "Prosthetic",
        ItemType::Rations => "Ration Tin",
        ItemType::LuxuryMeal => "Banquet Plate",
        ItemType::Potato => "Potato",
        ItemType::Wheat => "Wheat Sheaf",
        _ => "Curio",
    }
}

/// Ordinal word for the nth artifact of a type (n >= 2).
fn ordinal_word(n: u32) -> String {
    match n {
        2 => "Second".to_string(),
        3 => "Third".to_string(),
        4 => "Fourth".to_string(),
        5 => "Fifth".to_string(),
        6 => "Sixth".to_string(),
        7 => "Seventh".to_string(),
        8 => "Eighth".to_string(),
        9 => "Ninth".to_string(),
        _ => format!("#{n}"),
    }
}

/// Generates a provenance name and bumps the registry serial for the type.
pub fn artifact_name(registry: &mut ArtifactRegistry, item_type: &ItemType) -> String {
    let count = registry.promoted_per_type.entry(*item_type).or_insert(0);
    *count += 1;
    let title = artifact_title(item_type);
    if *count == 1 {
        format!("the Founder's {title}")
    } else {
        format!("the {} {title}", ordinal_word(*count))
    }
}

/// Appraised sale value of an artifact of the given age.
#[allow(clippy::cast_precision_loss)]
pub fn artifact_sale_value(age_ticks: u64, config: &ArtifactMarketConfig) -> f32 {
    config.base_value + config.value_per_age_tick * age_ticks as f32
}

/// Counts historical artifacts in the world (STATS helper).
pub fn artifact_count(world: &mut World) -> usize {
    let mut query = world.query_filtered::<Entity, With<HistoricalArtifact>>();
    query.iter(world).count()
}
/// Ages all [`Item`] entities and promotes items that have survived past
/// [`ArtifactMarketConfig::age_threshold_ticks`] to [`HistoricalArtifact`]
/// status, with a procedural provenance name and a chronicle entry.
/// Items never seen before are stamped with age zero first.
pub fn item_aging_system(
    mut commands: Commands,
    mut registry: ResMut<ArtifactRegistry>,
    config: Res<ArtifactMarketConfig>,
    sim_time: Option<Res<SimulationTime>>,
    items: Query<(Entity, &Item, Option<&ItemAge>, Option<&HistoricalArtifact>)>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    if !tick.is_multiple_of(config.scan_period_ticks) {
        return;
    }
    for (entity, item, age, artifact) in &items {
        let Some(age) = age else {
            commands.entity(entity).insert(ItemAge { age_ticks: 0 });
            continue;
        };
        let new_age = age.age_ticks.saturating_add(config.scan_period_ticks);
        commands.entity(entity).insert(ItemAge {
            age_ticks: new_age,
        });
        if new_age >= config.age_threshold_ticks && artifact.is_none() {
            let title = artifact_title(&item.item_type);
            let name = artifact_name(&mut registry, &item.item_type);
            commands.entity(entity).insert(HistoricalArtifact {
                name: name.clone(),
                promoted_tick: tick,
            });
            chronicle.send(AddChronicleEvent {
                text: format!(
                    "Recognized as a historical artifact: {name}. The old {title} has survived {new_age} ticks — core-world collectors would pay dearly for it."
                ),
                importance: EventImportance::Standard,
            });
        }
    }
}

/// Refreshes the museum morale aura: each [`Museum`] building grants pops
/// within radius a "Museum of Wonders" mood modifier scaling with the number
/// of housed artifacts (artifacts on tiles within the radius), capped.
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
pub fn museum_aura_system(
    sim_time: Option<Res<SimulationTime>>,
    config: Res<ArtifactMarketConfig>,
    museums: Query<&GridPosition, (With<Museum>, With<Building>)>,
    artifacts: Query<&GridPosition, With<HistoricalArtifact>>,
    mut pops: Query<(&GridPosition, &mut Morale), With<Pop>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    if !tick.is_multiple_of(config.museum_aura_period_ticks) {
        return;
    }
    let artifact_positions: Vec<GridPosition> = artifacts.iter().copied().collect();
    for museum_pos in &museums {
        let housed = artifact_positions
            .iter()
            .filter(|p| museum_pos.distance_chebyshev(**p) <= config.museum_radius as u32)
            .count();
        if housed == 0 {
            continue;
        }
        let bonus = (housed as f32 * config.museum_aura_per_artifact).min(config.museum_aura_cap);
        for (pop_pos, mut morale) in &mut pops {
            if museum_pos.distance_chebyshev(*pop_pos) <= config.museum_radius as u32 {
                morale.add_modifier(MoodModifier {
                    label: "Museum of Wonders".to_string(),
                    value: bonus,
                    duration: MUSEUM_AURA_DURATION_TICKS,
                });
            }
        }
    }
}

/// Processes [`SellArtifactEvent`]: pays the appraised value into
/// [`EmpireResources`] (spawning a holder if none exists), hits every pop
/// with a long "Heritage Sold" grief modifier, writes an [`AddChronicleEvent`]
/// documenting the sacrifice, and despawns the artifact. Missing entities are
/// skipped so re-delivery is harmless.
pub fn process_artifact_sale_system(
    mut commands: Commands,
    mut events: EventReader<SellArtifactEvent>,
    artifacts: Query<(&HistoricalArtifact, &ItemAge)>,
    config: Res<ArtifactMarketConfig>,
    mut empires: Query<&mut EmpireResources>,
    mut pops: Query<&mut Morale, With<Pop>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let mut total_payout = 0.0f32;
    for ev in events.read() {
        let Ok((artifact, age)) = artifacts.get(ev.artifact) else {
            continue;
        };
        let value = artifact_sale_value(age.age_ticks, &config);
        total_payout += value;
        for mut morale in &mut pops {
            morale.add_modifier(MoodModifier {
                label: "Heritage Sold".to_string(),
                value: config.sale_morale_hit,
                duration: config.sale_grief_duration_ticks,
            });
        }
        let name = artifact.name.clone();
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Sold {name} to core-world collectors for {value:.0} credits. The colony eats tonight; a piece of its history is gone forever."
            ),
            importance: EventImportance::Major,
        });
        commands.entity(ev.artifact).despawn();
    }
    if total_payout > 0.0 {
        let mut paid = false;
        for mut empire in &mut empires {
            empire.credits += total_payout;
            paid = true;
        }
        if !paid {
            commands.spawn(EmpireResources {
                credits: total_payout,
                alloys: 0,
            });
        }
    }
}

/// Processes [`DesignateMuseumEvent`]: marks the building a [`Museum`].
pub fn process_museum_designation_system(
    mut commands: Commands,
    mut events: EventReader<DesignateMuseumEvent>,
    buildings: Query<Entity, With<Building>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        if buildings.get(ev.building).is_err() {
            continue;
        }
        commands.entity(ev.building).insert(Museum);
        chronicle.send(AddChronicleEvent {
            text: "A museum wing has opened — the colony's oldest relics are now on display, and the people remember who they were."
                .to_string(),
            importance: EventImportance::Standard,
        });
    }
}

/// Registers the artifact market events, resources, and systems on an App.
pub fn register_artifact_market(app: &mut App) {
    app.init_resource::<ArtifactMarketConfig>();
    app.init_resource::<ArtifactRegistry>();
    app.add_event::<SellArtifactEvent>();
    app.add_event::<DesignateMuseumEvent>();
    app.add_event::<AddChronicleEvent>();
    app.add_systems(
        Update,
        (
            item_aging_system,
            museum_aura_system,
            process_artifact_sale_system,
            process_museum_designation_system,
        )
            .chain(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_app() -> App {
        let mut app = App::new();
        register_artifact_market(&mut app);
        app.world_mut().insert_resource(SimulationTime {
            tick: 100,
            ..Default::default()
        });
        // EmpireResources is a Component; spawn one holder.
        app.world_mut().spawn(EmpireResources {
            credits: 0.0,
            alloys: 0,
        });
        app
    }

    fn chronicle_texts(app: &mut App) -> Vec<String> {
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).map(|e| e.text.clone()).collect()
    }

    fn spawn_artifact(app: &mut App, item_type: ItemType, age_ticks: u64, name: &str) -> Entity {
        app.world_mut()
            .spawn((
                Item { item_type },
                ItemAge { age_ticks },
                HistoricalArtifact {
                    name: name.to_string(),
                    promoted_tick: 0,
                },
            ))
            .id()
    }

    #[test]
    fn test_aging_promotes_item_past_threshold() {
        let mut app = setup_app();
        let item = app
            .world_mut()
            .spawn((Item { item_type: ItemType::Tool }, ItemAge { age_ticks: 900 }))
            .id();
        app.update();
        let world = app.world();
        assert!(
            world.get::<HistoricalArtifact>(item).is_some(),
            "item past the age threshold should be promoted to HistoricalArtifact"
        );
    }

    #[test]
    fn test_aging_does_not_promote_young_item() {
        let mut app = setup_app();
        let item = app
            .world_mut()
            .spawn((Item { item_type: ItemType::Tool }, ItemAge { age_ticks: 100 }))
            .id();
        app.update();
        assert!(
            app.world().get::<HistoricalArtifact>(item).is_none(),
            "young item must not be promoted"
        );
    }

    #[test]
    fn test_aging_stamps_new_items_with_zero_age() {
        let mut app = setup_app();
        let item = app
            .world_mut()
            .spawn(Item {
                item_type: ItemType::Tool,
            })
            .id();
        app.update();
        let age = app.world().get::<ItemAge>(item);
        assert!(age.is_some(), "aging scan should stamp new items");
        assert_eq!(age.unwrap().age_ticks, 0);
    }

    #[test]
    fn test_promotion_names_first_tool_the_founders_pickaxe() {
        let mut app = setup_app();
        let item = app
            .world_mut()
            .spawn((Item { item_type: ItemType::Tool }, ItemAge { age_ticks: 900 }))
            .id();
        app.update();
        let artifact = app.world().get::<HistoricalArtifact>(item).unwrap();
        assert_eq!(artifact.name, "the Founder's Pickaxe");
    }

    #[test]
    fn test_promotion_writes_chronicle() {
        let mut app = setup_app();
        app.world_mut().spawn((
            Item {
                item_type: ItemType::Manual,
            },
            ItemAge { age_ticks: 900 },
        ));
        app.update();
        let texts = chronicle_texts(&mut app);
        assert!(
            texts.iter().any(|t| t.contains("Handwritten Diary")),
            "promotion should be chronicled, got: {texts:?}"
        );
    }

    #[test]
    fn test_selling_artifact_increases_credits() {
        let mut app = setup_app();
        // Freeze aging for this test: tick 101 is not a scan multiple, so the
        // artifact's age (and appraised value) stays exactly as spawned.
        app.world_mut().resource_mut::<SimulationTime>().tick = 101;
        let artifact = spawn_artifact(&mut app, ItemType::Tool, 800, "the Founder's Pickaxe");
        let expected = artifact_sale_value(800, &ArtifactMarketConfig::default());
        app.world_mut().send_event(SellArtifactEvent { artifact });
        app.update();
        let credits: f32 = {
            let world = app.world_mut();
            let mut query = world.query::<&EmpireResources>();
            query.iter(world).map(|e| e.credits).sum()
        };
        assert!(
            (credits - expected).abs() < 0.01,
            "credits should increase by the appraised value {expected}, got {credits}"
        );
    }

    #[test]
    fn test_selling_artifact_decreases_morale() {
        let mut app = setup_app();
        let pop = app.world_mut().spawn((Pop, Morale::default())).id();
        let artifact = spawn_artifact(&mut app, ItemType::Tool, 800, "the Founder's Pickaxe");
        app.world_mut().send_event(SellArtifactEvent { artifact });
        app.update();
        let morale = app.world().get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == "Heritage Sold" && m.value < 0.0),
            "sale should apply a negative Heritage Sold modifier, got {:?}",
            morale.modifiers
        );
    }

    #[test]
    fn test_selling_artifact_despawns_it() {
        let mut app = setup_app();
        let artifact = spawn_artifact(&mut app, ItemType::Tool, 800, "the Founder's Pickaxe");
        app.world_mut().send_event(SellArtifactEvent { artifact });
        app.update();
        assert!(
            app.world().get_entity(artifact).is_err(),
            "sold artifact should be despawned"
        );
    }

    #[test]
    fn test_selling_unknown_entity_is_noop() {
        let mut app = setup_app();
        let ghost = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(ghost);
        app.world_mut().send_event(SellArtifactEvent { artifact: ghost });
        app.update(); // must not panic
        let credits: f32 = {
            let world = app.world_mut();
            let mut query = world.query::<&EmpireResources>();
            query.iter(world).map(|e| e.credits).sum()
        };
        assert_eq!(credits, 0.0, "selling nothing should pay nothing");
    }

    #[test]
    fn test_sale_writes_chronicle_of_sacrifice() {
        let mut app = setup_app();
        let artifact = spawn_artifact(
            &mut app,
            ItemType::Manual,
            800,
            "the Founder's Handwritten Diary",
        );
        app.world_mut().send_event(SellArtifactEvent { artifact });
        app.update();
        let texts = chronicle_texts(&mut app);
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Founder's Handwritten Diary") && t.contains("credits")),
            "sale should be chronicled as a sacrifice, got: {texts:?}"
        );
    }

    #[test]
    fn test_museum_aura_boosts_nearby_morale() {
        let mut app = setup_app();
        app.world_mut().spawn((
            Building {
                building_type: crate::layer1::architecture::building::BuildingType::Tavern,
            },
            Museum,
            GridPosition { x: 0, y: 0 },
        ));
        app.world_mut().spawn((
            Item {
                item_type: ItemType::Tool,
            },
            HistoricalArtifact {
                name: "the Founder's Pickaxe".to_string(),
                promoted_tick: 0,
            },
            GridPosition { x: 1, y: 1 },
        ));
        let pop = app
            .world_mut()
            .spawn((Pop, Morale::default(), GridPosition { x: 2, y: 2 }))
            .id();
        app.update();
        let morale = app.world().get::<Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == "Museum of Wonders" && m.value > 0.0),
            "pop near a museum with a housed artifact should get an aura, got {:?}",
            morale.modifiers
        );
    }

    #[test]
    fn test_museum_aura_scales_with_housed_count() {
        let mut app = setup_app();
        app.world_mut().spawn((
            Building {
                building_type: crate::layer1::architecture::building::BuildingType::Tavern,
            },
            Museum,
            GridPosition { x: 0, y: 0 },
        ));
        for i in 0..2 {
            app.world_mut().spawn((
                Item {
                    item_type: ItemType::Tool,
                },
                HistoricalArtifact {
                    name: format!("artifact {i}"),
                    promoted_tick: 0,
                },
                GridPosition { x: 1, y: 1 },
            ));
        }
        let pop = app
            .world_mut()
            .spawn((Pop, Morale::default(), GridPosition { x: 2, y: 2 }))
            .id();
        app.update();
        let morale = app.world().get::<Morale>(pop).unwrap();
        let bonus: f32 = morale
            .modifiers
            .iter()
            .filter(|m| m.label == "Museum of Wonders")
            .map(|m| m.value)
            .sum();
        let expected = 2.0 * MUSEUM_AURA_PER_ARTIFACT;
        assert!(
            (bonus - expected).abs() < 0.001,
            "two housed artifacts should give {expected}, got {bonus}"
        );
    }

    #[test]
    fn test_artifact_value_grows_with_age() {
        let config = ArtifactMarketConfig::default();
        let young = artifact_sale_value(800, &config);
        let old = artifact_sale_value(1600, &config);
        assert!(old > young, "older artifacts appraise higher");
        assert!((young - (BASE_ARTIFACT_VALUE + VALUE_PER_AGE_TICK * 800.0)).abs() < 0.01);
    }
}
