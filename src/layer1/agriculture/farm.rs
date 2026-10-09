#![allow(clippy::cast_sign_loss, clippy::unnecessary_map_or)]
#![allow(clippy::collapsible_if, clippy::type_complexity)]
use crate::layer1::actions::AssignmentType;
use crate::layer1::balance::{
    FOOD_HUNGER_THRESHOLD, FOOD_PER_MEAL, FOOD_PER_WORKER_PER_TICK, HUNGER_PER_MEAL,
};
use crate::layer1::building::{Building, BuildingType};
use crate::layer1::economy::{get_wage_for_job, ColonyPrices, Wallet};
use crate::layer1::energy::PowerConsumer;
use crate::layer1::eureka::{check_for_eureka, EurekaConfig};
use crate::layer1::factions::{FactionMember, FactionState, Factions};
use crate::layer1::fauna::{Fauna, FaunaType};
use crate::layer1::fertility::FertilityGrid;
use crate::layer1::agriculture::compost::CompostSoil;
use crate::layer1::economy::recall::UseItemEvent;
use crate::layer1::husbandry::Tame;
use crate::layer1::items::ItemType;
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::palette_fatigue::{record_meal, DietaryHistory};
use crate::layer1::pop::Job;
use crate::layer1::pop::Pop;
use crate::layer1::resources::ColonyResources;
use crate::layer1::seasons::{Season, SeasonState};
use crate::layer1::skills::{get_skill_efficiency, SkillType, Skills};
use crate::layer1::social_mimicry::JustConsumed;
use crate::layer1::social::flesh_famine::Blighted;
use crate::layer1::tech::Tech;
use crate::layer1::traits::Trait;
use crate::layer1::utility_ai::{ActionType, PopAction};
use crate::layer1::GridPosition;
use bevy_ecs::prelude::*;
use crate::layer1::culture::sovereign::FarmProductionLedger;
use rand::seq::SliceRandom;

/// Water cost per tick per worker for Hydroponics.
const HYDROPONICS_WATER_COST: f32 = 0.1;
/// Production multiplier for Hydroponics.
const HYDROPONICS_MULTIPLIER: f32 = 2.0;

/// Farm component - produces food when worked.
#[derive(Component)]
pub struct Farm {
    /// Maximum number of workers.
    pub capacity: usize,
    /// List of workers assigned to this farm.
    /// **Legacy**: Used for UI/Capacity checks, but production uses `PopAction`.
    pub workers: Vec<Entity>,
    /// The crop selected for this farm.
    pub selected_crop: ItemType,
}

impl Default for Farm {
    fn default() -> Self {
        Self {
            capacity: 2,
            workers: Vec::new(),
            selected_crop: ItemType::Wheat,
        }
    }
}

struct CropStats {
    base_yield: f32,
    winter_modifier: f32,
}

const fn get_crop_stats(crop: &ItemType) -> CropStats {
    match crop {
        ItemType::Wheat => CropStats {
            base_yield: 0.006,
            winter_modifier: 0.2,
        },
        ItemType::Potato => CropStats {
            base_yield: 0.004,
            winter_modifier: 0.8,
        },
        ItemType::Rice => CropStats {
            base_yield: 0.005,
            winter_modifier: 0.5,
        },
        _ => CropStats {
            base_yield: FOOD_PER_WORKER_PER_TICK,
            winter_modifier: 0.5,
        },
    }
}

/// Produces food from all farms with active workers.
#[allow(clippy::too_many_arguments)]
pub fn produce_food_system(
    farm_query: Query<(
        &Building,
        &GridPosition,
        Option<&PowerConsumer>,
        &Farm,
        Option<&Blighted>,
    )>,
    mut pop_query: Query<
        (
            Entity,
            &GridPosition,
            &PopAction,
            Option<&mut Skills>,
            Option<&FactionMember>,
            Option<&crate::layer1::traits::Traits>,
            Option<&mut Wallet>,
            Option<&Job>,
        ),
        With<Pop>,
    >,
    season_state: Option<Res<SeasonState>>,
    mut resources: ResMut<ColonyResources>,
    factions: Option<Res<Factions>>,
    // We need mut access to TechState for Corruption Check
    tech_state_mut: Option<ResMut<crate::layer1::tech::TechState>>,
    fertility_grid: Option<Res<FertilityGrid>>,
    compost_soil: Option<Res<CompostSoil>>,
    eureka_config: Option<Res<EurekaConfig>>,
    mut eureka_events: EventWriter<crate::layer1::eureka::EurekaEvent>,
    // Fallen Sovereign: cumulative farm-production ledger for the labor decree.
    mut ledger: Option<ResMut<FarmProductionLedger>>,
) {
    let modifier = season_state
        .as_ref()
        .map_or(1.0, |s| s.current_season.food_modifier());

    let current_season = season_state
        .as_ref()
        .map_or(Season::Spring, |s| s.current_season);

    let farm_map: bevy::utils::HashMap<GridPosition, (BuildingType, bool, ItemType)> = farm_query
        .iter()
        // Spec 276: blighted farms produce nothing.
        .filter(|(_, _, _, _, blighted)| blighted.is_none())
        .map(|(b, p, pc, farm, _)| {
            (
                *p,
                (
                    b.building_type,
                    pc.is_some_and(|c| c.active),
                    farm.selected_crop,
                ),
            )
        })
        .collect();

    for (_, pos, action, skills_opt, faction_member_opt, traits, mut wallet_opt, job_opt) in
        &mut pop_query
    {
        if action.current != ActionType::Farm {
            continue;
        }
        // Check for strikes
        if let Some(factions) = &factions {
            if let Some(member) = faction_member_opt {
                if let Some(fid) = member.faction_id {
                    if factions
                        .get(fid)
                        .is_some_and(|d| d.state == FactionState::Striking)
                    {
                        continue;
                    }
                }
            }
        }

        if let Some((building_type, is_powered, selected_crop)) = farm_map.get(pos) {
            process_single_farmer(
                building_type,
                *is_powered,
                selected_crop,
                pos,
                skills_opt,
                traits,
                &mut wallet_opt,
                job_opt,
                &mut resources,
                tech_state_mut.as_deref(),
                fertility_grid.as_deref(),
                compost_soil.as_deref(),
                eureka_config.as_deref(),
                &mut eureka_events,
                modifier,
                current_season,
                ledger.as_deref_mut(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_single_farmer(
    building_type: &BuildingType,
    is_powered: bool,
    selected_crop: &ItemType,
    pos: &GridPosition,
    mut skills_opt: Option<Mut<'_, Skills>>,
    traits: Option<&crate::layer1::traits::Traits>,
    wallet_opt: &mut Option<Mut<'_, Wallet>>,
    job_opt: Option<&Job>,
    resources: &mut ColonyResources,
    tech_state_mut: Option<&crate::layer1::tech::TechState>,
    fertility_grid: Option<&FertilityGrid>,
    compost_soil: Option<&CompostSoil>,
    eureka_config: Option<&EurekaConfig>,
    eureka_events: &mut EventWriter<crate::layer1::eureka::EurekaEvent>,
    modifier: f32,
    current_season: Season,
    // Fallen Sovereign: cumulative farm-production ledger (None = don't track).
    ledger: Option<&mut FarmProductionLedger>,
) {
    // Tech Corruption Check
    if let Some(tech) = building_type.required_tech() {
        let tech_active = tech_state_mut.map_or(true, |ts| ts.is_active(tech));
        if !tech_active {
            return;
        }
    }

    let skill_type = SkillType::Farming;

    // Calculate efficiency
    let efficiency = get_skill_efficiency(skills_opt.as_deref(), skill_type);

    // Add XP
    if let Some(ref mut skills) = skills_opt {
        skills.add_xp(skill_type, 1.0);
    }

    let crop_stats = get_crop_stats(selected_crop);

    // Determine yield and modifiers
    let (base_production, water_cost, effective_modifier) = match building_type {
        BuildingType::HydroponicsBay => {
            if is_powered {
                (
                    crop_stats.base_yield,
                    HYDROPONICS_WATER_COST,
                    HYDROPONICS_MULTIPLIER,
                )
            } else {
                (0.0, 0.0, 0.0)
            }
        }
        BuildingType::Plantation => (FOOD_PER_WORKER_PER_TICK, 0.0, modifier),
        _ => {
            let season_mod = if building_type.seasonal_immunity() {
                1.0
            } else if current_season == Season::Winter {
                crop_stats.winter_modifier
            } else {
                modifier
            };
            (crop_stats.base_yield, 0.0, season_mod)
        }
    };

    // Integrate Fertility
    let mut fertility_modifier = if *building_type == BuildingType::HydroponicsBay {
        1.0
    } else if let Some(grid) = fertility_grid {
        if pos.x >= 0 && pos.y >= 0 {
            grid.get(pos.x as usize, pos.y as usize)
        } else {
            0.0
        }
    } else {
        1.0
    };
    // Spec 1370 Civilizational Compost: corpse-enriched soil boosts yields
    // beyond the grid baseline (hydroponics excluded — soilless).
    if *building_type != BuildingType::HydroponicsBay {
        if let Some(compost) = compost_soil {
            fertility_modifier += compost.enrichment_at(pos.x, pos.y);
        }
    }

    // Check water availability
    if water_cost > 0.0 && resources.water < water_cost {
        return;
    }

    // Deduct water
    if water_cost > 0.0 {
        resources.water -= water_cost;
    }

    let production = efficiency * base_production * effective_modifier * fertility_modifier;
    if production > 0.0 {
        match building_type {
            BuildingType::Plantation => {
                resources.add_fiber(production);
            }
            _ => {
                resources.add_food(production);
                if let Some(ledger) = ledger {
                    ledger.cumulative += production;
                }
            }
        }

        // Pay Wages
        if let Some(wallet) = wallet_opt.as_deref_mut() {
            let job_type = job_opt.map_or(AssignmentType::FarmWorker, |j| j.job_type);
            let base_wage = get_wage_for_job(job_type);
            let wage = base_wage * 0.01;
            wallet.credits += wage;
        }

        // Eureka Check
        if let Some(config) = eureka_config {
            check_for_eureka(
                eureka_events,
                config,
                ActionType::Farm,
                Some(Tech::Hydroponics),
                traits,
            );
        }
    }
}

/// Pops eat food when hungry.
#[allow(clippy::too_many_arguments)] // Spec 1373 adds the recall event writer (8th param).
pub fn consume_food_system(
    mut commands: Commands,
    mut pop_query: Query<
        (
            Entity,
            &mut Needs,
            Option<&mut DietaryHistory>,
            Option<&mut Wallet>,
            Option<&mut Morale>,
            Option<&crate::layer1::traits::Traits>,
            Option<&mut crate::layer1::gut_biome::GutBiome>,
        ),
        With<Pop>,
    >,
    mut resources: ResMut<ColonyResources>,
    farm_query: Query<&Farm>, // Query Farm instead of Crop
    animal_query: Query<&Fauna, With<Tame>>,
    prices: Option<Res<ColonyPrices>>,
    // Planetary Governor: active rationing schedules shrink meals (None = no rationing).
    ration: Option<Res<crate::layer1::culture::governor::RationOrder>>,
    // Spec 1373: report food consumption so recalled items can fail.
    // Optional: farm unit tests run this system without event resources.
    mut item_use_events: Option<ResMut<bevy_ecs::event::Events<UseItemEvent>>>,
) {
    // Use total_food() logic for check
    let total_food = resources.total_food();
    if total_food < f32::EPSILON {
        return;
    }

    let food_price = prices.map_or(0.0, |p| p.food_price);

    // Collect available food types from active sources
    let mut available_items = Vec::new();
    for farm in &farm_query {
        available_items.push(farm.selected_crop);
    }
    for fauna in &animal_query {
        match fauna.fauna_type {
            FaunaType::SpaceRat | FaunaType::Wolf => {
                available_items.push(ItemType::Meat);
            }
            FaunaType::Mascot => {}
        }
    }

    // Collect hungry pop entities first to avoid borrow issues with mut iteration
    let hungry_pops: Vec<Entity> = pop_query
        .iter()
        .filter(|(_, needs, _, wallet, _, _, _)| {
            if needs.hunger >= FOOD_HUNGER_THRESHOLD {
                return false;
            }
            // Check affordability (if wallet exists)
            if let Some(w) = wallet {
                if w.credits < food_price {
                    return false;
                }
            }
            true
        })
        .map(|(e, _, _, _, _, _, _)| e)
        .collect();

    let mut rng = rand::thread_rng();

    for entity in hungry_pops {
        // Try to eat from food pool first, then rations
        let mut eaten_item = ItemType::None;
        let mut ate = false;

        // Planetary Governor: an active rationing schedule shrinks every meal
        // (smaller portions, same official satisfaction).
        let meal_cost = if ration.as_ref().is_some_and(|r| r.remaining > 0) {
            FOOD_PER_MEAL * crate::layer1::culture::governor::RATION_MEAL_FRACTION
        } else {
            FOOD_PER_MEAL
        };

        // Simplify consumption: Just check food pool
        if resources.food >= meal_cost {
            resources.food -= meal_cost;
            ate = true;

            // Determine flavor for Palette Fatigue
            // Prefer picking from active production
            if !available_items.is_empty() {
                eaten_item = available_items
                    .choose(&mut rng)
                    .cloned()
                    .unwrap_or(ItemType::Potato);
            } else {
                // If no farms active, default to Potato (The Universal Tuber)
                eaten_item = ItemType::Potato;
            }
        } else if resources.rations >= meal_cost {
            resources.rations -= meal_cost;
            eaten_item = ItemType::Rations;
            ate = true;
        } else if resources.nutrient_paste >= meal_cost {
            resources.nutrient_paste -= meal_cost;
            eaten_item = ItemType::NutrientPaste;
            ate = true;
        }

        if ate {
            // Spec 1373: eating a recalled product risks critical failure.
            if let Some(events) = item_use_events.as_mut() {
                events.send(UseItemEvent {
                    user: entity,
                    item_type: eaten_item,
                });
            }
            if let Ok((
                _,
                mut needs,
                mut history_opt,
                mut wallet_opt,
                mut morale_opt,
                traits_opt,
                biome_opt,
            )) = pop_query.get_mut(entity)
            {
                apply_food_consumption_effects(
                    &mut commands,
                    entity,
                    &mut needs,
                    &mut history_opt,
                    &mut wallet_opt,
                    &mut morale_opt,
                    traits_opt,
                    biome_opt,
                    eaten_item,
                    food_price,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_food_consumption_effects(
    commands: &mut Commands,
    entity: Entity,
    needs: &mut Needs,
    history_opt: &mut Option<Mut<'_, DietaryHistory>>,
    wallet_opt: &mut Option<Mut<'_, Wallet>>,
    morale_opt: &mut Option<Mut<'_, Morale>>,
    traits_opt: Option<&crate::layer1::traits::Traits>,
    biome_opt: Option<Mut<'_, crate::layer1::gut_biome::GutBiome>>,
    eaten_item: ItemType,
    food_price: f32,
) {
    // Determine GutBiome category
    let category = crate::layer1::gut_biome::get_biome_category(&eaten_item);

    // Get Biome Data
    let (efficiency, mood_effect) = if let Some(mut biome) = biome_opt {
        let fam = biome.get_familiarity(category);
        biome.adapt(category);

        if fam > 0.8 {
            (1.0, Some("Gut Comfort"))
        } else if fam < 0.3 {
            (0.6, Some("Indigestion"))
        } else {
            (1.0, None)
        }
    } else {
        (1.0, None)
    };

    needs.hunger = (needs.hunger + (HUNGER_PER_MEAL * efficiency)).min(1.0);

    // Apply Gut Mood Effect
    if let Some(label) = mood_effect {
        if let Some(morale) = morale_opt.as_deref_mut() {
            let val = if label == "Gut Comfort" { 0.05 } else { -0.1 };
            morale.add_modifier(MoodModifier {
                label: label.to_string(),
                value: val,
                duration: 200,
            });
        }
    }

    // Deduct Cost
    if let Some(wallet) = wallet_opt.as_deref_mut() {
        wallet.credits -= food_price;
    }

    // Rations Mood Logic
    // Nutrient Paste Mood Logic
    if eaten_item == ItemType::NutrientPaste {
        let is_immune = traits_opt
            .is_some_and(|t| t.0.contains(&Trait::Cannibal) || t.0.contains(&Trait::Pragmatist));
        if !is_immune {
            if let Some(morale) = morale_opt.as_deref_mut() {
                morale.add_modifier(MoodModifier {
                    label: "Ate Nutrient Paste".to_string(),
                    value: -0.1, // -10% mood
                    duration: 100,
                });
            }
        }
    }

    if eaten_item == ItemType::Rations {
        let is_immune =
            traits_opt.is_some_and(|t| t.has(Trait::Cannibal) || t.has(Trait::Pragmatist));

        if !is_immune {
            if let Some(morale) = morale_opt.as_deref_mut() {
                morale.add_modifier(MoodModifier {
                    label: "Ate Slop".to_string(),
                    value: -0.1, // -10% mood (0.1 in 0.0-1.0 scale, spec said -10 but scale is usually 0-1 or 0-100? Morale struct says value 0.0-1.0. Modifier sum added to value. MoodModifier value is f32. Let's assume 0.1 means 10%)
                    duration: 250, // 24h
                });
            }
        }
    }

    // Palette Fatigue Logic
    if let Some(ref mut history) = history_opt {
        record_meal(history, eaten_item);
    } else {
        let mut history = DietaryHistory::default();
        record_meal(&mut history, eaten_item);
        commands.entity(entity).insert(history);
    }

    // Mimicry Integration
    commands
        .entity(entity)
        .insert(JustConsumed { item: eaten_item });
}

/// Removes dead workers from farms.
pub fn clean_dead_workers_system(mut farm_query: Query<&mut Farm>, pop_query: Query<&Pop>) {
    for mut farm in &mut farm_query {
        farm.workers.retain(|&worker| pop_query.get(worker).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::building::{Building, BuildingType};
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::GridPosition;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_farm_default() {
        let farm = Farm::default();
        assert_eq!(farm.capacity, 2);
        assert!(farm.workers.is_empty());
        assert_eq!(farm.selected_crop, ItemType::Wheat);
    }

    fn setup_test_world() -> World {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        world.init_resource::<Events<crate::layer1::eureka::EurekaEvent>>();
        world
    }

    #[test]
    fn test_produce_food_system() {
        let mut world = setup_test_world();

        world.spawn((
            Farm::default(),
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn worker at farm doing Farm action
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));

        world.run_system_once(produce_food_system).unwrap();

        let resources = world.resource::<ColonyResources>();
        // Check total food
        assert!(resources.food > 0.0, "Total food should be updated");
    }

    #[test]
    fn test_produce_food_system_skills_xp() {
        let mut world = setup_test_world();

        world.spawn((
            Farm::default(),
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        let worker = world
            .spawn((
                Pop,
                Skills::default(),
                GridPosition { x: 5, y: 5 },
                PopAction {
                    current: ActionType::Farm,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(produce_food_system).unwrap();

        // Check XP
        let skills = world.get::<Skills>(worker).unwrap();
        assert_eq!(skills.get_xp(SkillType::Farming), 1.0);
    }

    #[test]
    fn test_produce_food_system_skills_efficiency() {
        let mut world = setup_test_world();

        // Worker with Level 1 Farming (100 XP) -> 1.1 efficiency
        let mut skills = Skills::default();
        skills.add_xp(SkillType::Farming, 100.0);

        world.spawn((
            Farm::default(),
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.spawn((
            Pop,
            skills,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));

        world.run_system_once(produce_food_system).unwrap();

        let resources = world.resource::<ColonyResources>();
        // Base Wheat = 0.006
        // With skill = 0.006 * 1.1 = 0.0066
        // Food starts at 10.0
        assert!(
            (resources.food - 10.0066).abs() < 0.0001,
            "Food production should reflect skill efficiency"
        );
    }

    #[test]
    fn test_produce_food_multiple_workers() {
        let mut world = setup_test_world();

        world.spawn((
            Farm::default(),
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Two workers
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));

        world.run_system_once(produce_food_system).unwrap();

        let resources = world.resource::<ColonyResources>();
        // 2 * 0.006 = 0.012. Initial 10.0.
        assert!(
            (resources.food - 10.012).abs() < 0.0001,
            "Two workers should produce more food"
        );
    }

    #[test]
    fn test_consume_food_system() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 1.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        world.spawn((
            Pop,
            Needs {
                hunger: 0.3,
                rest: 0.8,
                ..Default::default()
            },
        ));

        let food_before = world.resource::<ColonyResources>().food;
        world.run_system_once(consume_food_system).unwrap();
        let food_after = world.resource::<ColonyResources>().food;

        assert!(food_after < food_before, "Food should be consumed");
    }

    #[test]
    fn test_consume_food_adds_dietary_history() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 1.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.3,
                    rest: 0.8,
                    ..Default::default()
                },
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let history = world.get::<DietaryHistory>(pop);
        assert!(
            history.is_some(),
            "DietaryHistory should be added when eating"
        );
        assert_eq!(history.unwrap().recent_meals.len(), 1);
        // Default fallback is Potato
        assert_eq!(history.unwrap().recent_meals[0], ItemType::Potato);
    }

    #[test]
    fn test_consume_food_picks_active_crop() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 10.0,
            ..Default::default()
        });

        // Spawn a Wheat Farm
        world.spawn((
            Farm {
                selected_crop: ItemType::Wheat,
                ..Default::default()
            },
            GridPosition { x: 0, y: 0 },
        ));

        // Spawn a Pop
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                DietaryHistory::default(),
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let history = world.get::<DietaryHistory>(pop).unwrap();
        assert_eq!(history.recent_meals[0], ItemType::Wheat);
    }

    #[test]
    fn test_farm_has_selected_crop() {
        let farm = Farm::default();
        // Default crop should be Wheat (standard)
        assert_eq!(farm.selected_crop, ItemType::Wheat);
    }

    #[test]
    fn test_produce_food_wheat_yield() {
        let mut world = setup_test_world();
        world.insert_resource(SeasonState {
            current_season: Season::Spring,
        }); // Good weather

        // Spawn Farm with Wheat
        world.spawn((
            Farm {
                selected_crop: ItemType::Wheat,
                ..Default::default()
            },
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn Worker
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));

        world.run_system_once(produce_food_system).unwrap();

        let res = world.resource::<ColonyResources>();
        // Wheat base yield is high (e.g. 0.006 vs standard 0.005)
        // Initial 10.0. Yield > 0.005. Total > 10.005
        assert!(res.food > 10.005);
    }

    #[test]
    fn test_produce_food_potato_winter_resistance() {
        let mut world = setup_test_world();
        world.insert_resource(SeasonState {
            current_season: Season::Winter,
        });

        // Spawn Farm with Potato
        world.spawn((
            Farm {
                selected_crop: ItemType::Potato,
                ..Default::default()
            },
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        // Spawn Worker
        world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            PopAction {
                current: ActionType::Farm,
                ..Default::default()
            },
        ));

        world.run_system_once(produce_food_system).unwrap();

        let res = world.resource::<ColonyResources>();

        // Potato in winter (0.8 modifier) vs Wheat in winter (0.2 modifier)
        // Potato base (0.004) * 0.8 = 0.0032
        // Wheat base (0.006) * 0.2 = 0.0012
        // Total should increase by at least 0.003
        assert!(res.food > 10.003);
    }

    #[test]
    fn test_change_crop_selection() {
        let mut farm = Farm::default();
        assert_eq!(farm.selected_crop, ItemType::Wheat);

        farm.selected_crop = ItemType::Rice;
        assert_eq!(farm.selected_crop, ItemType::Rice);
    }

    #[test]
    fn test_consume_food_rations_mood_penalty() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            rations: 10.0,
            food: 0.0, // Ensure no other food
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale.modifiers.iter().find(|m| m.label == "Ate Slop");
        assert!(modifier.is_some(), "Should apply Ate Slop modifier");
        assert_eq!(modifier.unwrap().value, -0.1);
    }

    #[test]
    fn test_eating_paste_causes_gloom() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            rations: 10.0,
            food: 0.0, // Ensure no other food
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale.modifiers.iter().find(|m| m.label == "Ate Slop");
        assert!(modifier.is_some(), "Should apply Ate Slop (gloom) modifier");
    }

    #[test]
    fn test_pragmatist_ignores_gloom() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            rations: 10.0,
            food: 0.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        // Pragmatist Pop
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
                {
                    let mut t = crate::layer1::traits::Traits::default();
                    t.add(crate::layer1::traits::Trait::Pragmatist);
                    t
                },
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale.modifiers.iter().find(|m| m.label == "Ate Slop");
        assert!(
            modifier.is_none(),
            "Pragmatist should be immune to Ate Slop (gloom)"
        );
    }

    #[test]
    fn test_consume_food_rations_immunity() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            rations: 10.0,
            food: 0.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        // Cannibal Pop
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
                {
                    let mut t = crate::layer1::traits::Traits::default();
                    t.add(crate::layer1::traits::Trait::Cannibal);
                    t
                },
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale.modifiers.iter().find(|m| m.label == "Ate Slop");
        assert!(modifier.is_none(), "Cannibal should be immune to Ate Slop");
    }
}

#[cfg(test)]
mod recycler_tests {
    use super::*;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::morale::Morale;
    use crate::layer1::traits::{Trait, Traits};
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_eating_paste_causes_gloom() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            nutrient_paste: 10.0,
            food: 0.0,
            rations: 0.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                crate::layer1::entities::pop::Pop,
                crate::layer1::psychology::needs::Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale
            .modifiers
            .iter()
            .find(|m| m.label == "Ate Nutrient Paste");
        assert!(
            modifier.is_some(),
            "Should apply Ate Nutrient Paste gloom modifier"
        );
    }

    #[test]
    fn test_pragmatist_ignores_gloom() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            nutrient_paste: 10.0,
            food: 0.0,
            rations: 0.0,
            ..Default::default()
        });
        world.insert_resource(crate::shared::time::SimulationTime::default());

        let pop = world
            .spawn((
                crate::layer1::entities::pop::Pop,
                crate::layer1::psychology::needs::Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                Morale::default(),
                {
                    let mut t_set = bevy::utils::HashSet::default();
                    t_set.insert(Trait::Pragmatist);
                    Traits(t_set)
                },
            ))
            .id();

        world.run_system_once(consume_food_system).unwrap();

        let morale = world.get::<Morale>(pop).unwrap();
        let modifier = morale
            .modifiers
            .iter()
            .find(|m| m.label == "Ate Nutrient Paste");
        assert!(
            modifier.is_none(),
            "Pragmatist should be immune to Ate Nutrient Paste gloom"
        );
    }
}
