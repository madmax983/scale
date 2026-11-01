use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::items::ItemType;
use crate::layer1::map::GridPosition;
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::pop::{Job, Pop};
use crate::layer1::social_stratification::{Prestige, SocialClass};
use bevy_ecs::prelude::*;
use std::collections::HashMap;

/// Tracks the current "hot" item in the colony.
#[derive(Resource, Default, Debug)]
pub struct Trend {
    /// The item currently in fashion.
    pub current_item: Option<ItemType>,
    /// Strength of the trend (0.0 to 1.0). Decays over time.
    pub strength: f32,
    /// Position where the trend was started (used for proximity spread).
    pub setter_pos: GridPosition,
}

/// Component for pops indicating they want to mimic the Elite.
#[derive(Component, Default, Debug)]
pub struct SocialMimicry {
    /// The item this pop wants to consume to be trendy.
    pub desired_item: Option<ItemType>,
}

/// Component to track consumption events this tick.
/// Added by `consume_food_system` and removed by `clear_just_consumed_system`.
#[derive(Component)]
pub struct JustConsumed {
    /// The item that was just consumed.
    pub item: ItemType,
}

/// System: Elites set the trend when they consume items.
pub fn trend_setting_system(
    mut trend: ResMut<Trend>,
    query: Query<(&SocialClass, &GridPosition, &JustConsumed)>,
) {
    // Decay existing trend
    if trend.strength > 0.0 {
        trend.strength -= 0.01;
        if trend.strength <= 0.0 {
            trend.current_item = None;
            trend.strength = 0.0;
        }
    }

    // Check for new trend setters
    for (class, pos, consumed) in &query {
        if *class == SocialClass::Elite {
            // Elite sets the trend!
            trend.current_item = Some(consumed.item);
            trend.strength = 1.0; // Max strength
            trend.setter_pos = *pos;
            // Only one elite needs to set it per tick (first one wins for MVP)
            break;
        }
    }
}

/// System: Trends spread to non-elites (Labor/Middle).
pub fn trend_spread_system(
    trend: Res<Trend>,
    mut query: Query<
        (&GridPosition, &mut SocialMimicry),
        Without<crate::layer1::social_stratification::Prestige>,
    >,
) {
    let weak_trend = trend.current_item.is_none() || trend.strength < 0.2;

    for (pos, mut mimicry) in &mut query {
        if weak_trend {
            // Trend died out, stop caring
            mimicry.desired_item = None;
            continue;
        }

        // Manhattan distance check
        let dist = pos.distance_manhattan(trend.setter_pos);

        // If near the trend source (let's say 20 tiles is "gossip range")
        if dist < 20 {
            mimicry.desired_item.clone_from(&trend.current_item);
        }
    }
}

/// System: Apply mood buff if Pop consumes the trendy item.
#[allow(clippy::collapsible_if)]
pub fn trend_satisfaction_system(
    _commands: Commands,
    mut query: Query<(Entity, &SocialMimicry, &JustConsumed, Option<&mut Morale>)>,
) {
    for (_entity, mimicry, consumed, morale_opt) in &mut query {
        if let Some(desired) = &mimicry.desired_item {
            if &consumed.item == desired {
                if let Some(mut morale) = morale_opt {
                    morale.add_modifier(MoodModifier {
                        label: "Feeling Trendy".to_string(),
                        value: 0.1,   // +10% morale
                        duration: 50, // Lasts for 50 ticks (short-ish)
                    });
                }
            }
        }
    }
}

/// System: Removes `JustConsumed` component at the end of the tick.
pub fn clear_just_consumed_system(
    mut commands: Commands,
    query: Query<Entity, With<JustConsumed>>,
) {
    for entity in query.iter() {
        commands.entity(entity).remove::<JustConsumed>();
    }
}

// ---------------------------------------------------------------------------
// Spec 386 Social Mimicry: per-pop consumable preferences spread from
// high-status pops to nearby lower-status pops.
//
// Grounding note: the spec sketch names `SocialStatus` / `Preferences` /
// `ItemType::{LuxuryFood, BasicFood}`; none of those exist as sketched.
// The real seams are reused instead: status = `Prestige` (falling back to
// `Prestige::from_job` via the pop's `Job`), items = the real `ItemType`
// (LuxuryMeal / Potato stand in for the sketch's luxury/basic foods), and
// position = `GridPosition` (Chebyshev distance, as elsewhere in layer 1).
// This extends the Spec 128 trend machinery in this module rather than
// duplicating it: trends are colony-wide fashion, preferences are the
// per-pop tastes those trends push around.
// ---------------------------------------------------------------------------

/// A pop's consumable tastes (Spec 386 section 8: food and drinks ONLY —
/// equipment/clothing preferences are deliberately out of scope).
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    /// Favourite food item, if the pop has formed one.
    pub favorite_food: Option<ItemType>,
    /// Favourite drink item, if the pop has formed one.
    pub favorite_drink: Option<ItemType>,
}

/// Whether an item is a consumable whose preference may spread by mimicry.
#[must_use]
pub fn is_mimickable_consumable(item: ItemType) -> bool {
    use crate::layer1::economy::resources::ResourceType;
    matches!(item.as_resource_type(), Some(ResourceType::Food | ResourceType::Rations))
        || matches!(item, ItemType::NutrientPaste | ItemType::Alcohol)
}

/// Ticks remaining before a pop can change preference by mimicry again.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct MimicryCooldown {
    /// Remaining cooldown ticks (0 = eligible).
    pub remaining: u32,
}

/// Tuning for preference mimicry (Spec 386 section 5 REFACTOR).
#[derive(Resource, Debug, Clone, Copy)]
pub struct MimicryConfig {
    /// Chebyshev radius (tiles) within which tastes can spread.
    pub radius: u32,
    /// Per-tick probability that an eligible pop adopts a neighbour's taste.
    pub probability: f32,
    /// Cooldown imposed after a preference change.
    pub cooldown_ticks: u32,
    /// Minimum status gap (Prestige points) required for mimicry.
    pub min_status_gap: i32,
}

impl Default for MimicryConfig {
    fn default() -> Self {
        Self {
            radius: 6,
            probability: 0.08,
            cooldown_ticks: 240,
            min_status_gap: 3,
        }
    }
}

/// Emitted whenever a pop's preference changes by mimicry (observability).
#[derive(Event, Debug, Clone)]
pub struct PreferenceChangedEvent {
    /// The pop whose preference changed.
    pub pop: Entity,
    /// The higher-status pop they copied.
    pub source: Entity,
    /// Previous favourite (food or drink, per `drink`).
    pub old_item: Option<ItemType>,
    /// New favourite (food or drink, per `drink`).
    pub new_item: Option<ItemType>,
    /// True when the changed preference is the drink slot.
    pub drink: bool,
}

/// Running ledger of mimicry-driven preference changes (feeds STATS).
#[derive(Resource, Debug, Default, Clone)]
pub struct MimicryStats {
    /// Total preference changes caused by mimicry.
    pub changes: u64,
    /// Most recently copied item.
    pub last_item: Option<ItemType>,
}

/// Human-readable label for a newly fashionable consumable.
fn fashionable_label(item: ItemType) -> Option<&'static str> {
    match item {
        ItemType::LuxuryMeal => Some("a luxury meal"),
        ItemType::VoidAle => Some("void ale"),
        ItemType::Alcohol => Some("a strong spirit"),
        _ => None,
    }
}

/// Public-facing strings for Spec 386 (IP-guard test scans these).
#[must_use]
pub fn mimicry_public_strings() -> Vec<String> {
    vec![
        "A new taste spreads through the colony: a luxury meal is suddenly fashionable."
            .to_string(),
        "A new taste spreads through the colony: void ale is suddenly fashionable.".to_string(),
        "A new taste spreads through the colony: a strong spirit is suddenly fashionable."
            .to_string(),
    ]
}

/// STATS label for Spec 386: total mimicry-driven preference changes.
#[must_use]
pub fn mimicry_stats_label(world: &World) -> String {
    world
        .get_resource::<MimicryStats>()
        .map_or_else(|| "0".to_string(), |s| s.changes.to_string())
}

/// Effective social status of a pop: explicit `Prestige`, else derived
/// from its job, else the Labor floor (1).
fn effective_status(prestige: Option<&Prestige>, job: Option<&Job>) -> i32 {
    if let Some(p) = prestige {
        return i32::from(p.value);
    }
    if let Some(j) = job {
        return i32::from(Prestige::from_job(j.job_type).value);
    }
    1
}

/// Spec 386: nearby lower-status pops copy the consumable preferences of
/// higher-status pops. Uses a spatial hash over pops (section 5: no
/// O(N^2) pair scan), a per-tick probability roll, and a cooldown after
/// each change, so tastes spread gradually rather than instantly.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn preference_mimicry_system(
    mut commands: Commands,
    config: Option<Res<MimicryConfig>>,
    mut stats: Option<ResMut<MimicryStats>>,
    mut pref_events: Option<ResMut<Events<PreferenceChangedEvent>>>,
    mut chronicle_events: Option<ResMut<Events<AddChronicleEvent>>>,
    mut query: Query<
        (
            Entity,
            &GridPosition,
            &mut Preferences,
            Option<&Prestige>,
            Option<&Job>,
            Option<&mut MimicryCooldown>,
        ),
        With<Pop>,
    >,
) {
    let config = config.map_or_else(MimicryConfig::default, |c| *c);
    let radius = config.radius.max(1);

    // Snapshot every pop once; the spatial hash below restricts neighbour
    // checks to pops in adjacent grid cells.
    struct Snapshot {
        entity: Entity,
        pos: GridPosition,
        status: i32,
        food: Option<ItemType>,
        drink: Option<ItemType>,
        cooling: bool,
    }
    let mut snaps: Vec<Snapshot> = Vec::new();
    for (entity, pos, prefs, prestige, job, cooldown) in query.iter_mut() {
        let cooling = cooldown.as_ref().is_some_and(|c| c.remaining > 0);
        if let Some(mut cd) = cooldown {
            if cd.remaining > 0 {
                cd.remaining -= 1;
            }
        }
        snaps.push(Snapshot {
            entity,
            pos: *pos,
            status: effective_status(prestige, job),
            food: prefs.favorite_food,
            drink: prefs.favorite_drink,
            cooling,
        });
    }

    let cell = i32::try_from(radius).unwrap_or(i32::MAX).max(1);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (idx, snap) in snaps.iter().enumerate() {
        grid.entry((snap.pos.x.div_euclid(cell), snap.pos.y.div_euclid(cell)))
            .or_default()
            .push(idx);
    }

    // Decide changes from the snapshot (deterministic source choice:
    // highest status, then lowest entity index).
    struct Change {
        pop: Entity,
        source: Entity,
        old_item: Option<ItemType>,
        new_item: Option<ItemType>,
        drink: bool,
    }
    let mut changes: Vec<Change> = Vec::new();
    let mut rng = rand::thread_rng();
    for snap in &snaps {
        if snap.cooling {
            continue;
        }
        let cx = snap.pos.x.div_euclid(cell);
        let cy = snap.pos.y.div_euclid(cell);
        let mut best_food: Option<(i32, u32, Entity, ItemType)> = None;
        let mut best_drink: Option<(i32, u32, Entity, ItemType)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(bucket) = grid.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for &other_idx in bucket {
                    let other = &snaps[other_idx];
                    if other.entity == snap.entity
                        || other.status - snap.status < config.min_status_gap
                        || other.pos.distance_chebyshev(snap.pos) > radius
                    {
                        continue;
                    }
                    if let Some(item) = other.food {
                        if Some(item) != snap.food && is_mimickable_consumable(item) {
                            let cand = (other.status, other.entity.index(), other.entity, item);
                            let better = best_food.is_none_or(|b| {
                                cand.0 > b.0 || (cand.0 == b.0 && cand.1 < b.1)
                            });
                            if better {
                                best_food = Some(cand);
                            }
                        }
                    }
                    if let Some(item) = other.drink {
                        if Some(item) != snap.drink && is_mimickable_consumable(item) {
                            let cand = (other.status, other.entity.index(), other.entity, item);
                            let better = best_drink.is_none_or(|b| {
                                cand.0 > b.0 || (cand.0 == b.0 && cand.1 < b.1)
                            });
                            if better {
                                best_drink = Some(cand);
                            }
                        }
                    }
                }
            }
        }
        // Food trends lead; a pop adopts at most one taste per tick.
        let chosen = best_food
            .map(|(_, _, src, item)| (src, item, false))
            .or_else(|| best_drink.map(|(_, _, src, item)| (src, item, true)));
        let Some((source, item, drink)) = chosen else {
            continue;
        };
        // Probability gate (section 5): mimicry is a chance, not a certainty.
        let roll_ok = if config.probability >= 1.0 {
            true
        } else if config.probability <= 0.0 {
            false
        } else {
            rand::Rng::gen_bool(&mut rng, f64::from(config.probability))
        };
        if !roll_ok {
            continue;
        }
        changes.push(Change {
            pop: snap.entity,
            source,
            old_item: if drink { snap.drink } else { snap.food },
            new_item: Some(item),
            drink,
        });
    }

    // Apply decided changes.
    for change in changes {
        {
            let Ok((_, _, mut prefs, _, _, _)) = query.get_mut(change.pop) else {
                continue;
            };
            if change.drink {
                prefs.favorite_drink = change.new_item;
            } else {
                prefs.favorite_food = change.new_item;
            }
        }
        commands.entity(change.pop).insert(MimicryCooldown {
            remaining: config.cooldown_ticks,
        });
        if let Some(stats) = stats.as_deref_mut() {
            stats.changes += 1;
            stats.last_item = change.new_item;
        }
        if let Some(events) = pref_events.as_deref_mut() {
            events.send(PreferenceChangedEvent {
                pop: change.pop,
                source: change.source,
                old_item: change.old_item,
                new_item: change.new_item,
                drink: change.drink,
            });
        }
        // Only luxury tastes are news; everyday staples spread quietly.
        if let (Some(events), Some(item)) = (chronicle_events.as_deref_mut(), change.new_item) {
            if let Some(label) = fashionable_label(item) {
                events.send(AddChronicleEvent {
                    text: format!(
                        "A new taste spreads through the colony: {label} is suddenly fashionable."
                    ),
                    importance: EventImportance::Minor,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::ItemType;
    use crate::layer1::map::GridPosition;
    use crate::layer1::morale::Morale;
    use crate::layer1::pop::Pop;
    use crate::layer1::social_stratification::{Prestige, SocialClass};

    #[test]
    fn test_elite_sets_trend() {
        let mut world = World::new();
        world.init_resource::<Trend>();

        // Spawn Elite Pop consuming an item
        world.spawn((
            Pop,
            SocialClass::Elite,
            Prestige { value: 10 },
            GridPosition { x: 5, y: 5 },
            JustConsumed {
                item: ItemType::LuxuryMeal,
            },
        ));

        // Run system
        let mut schedule = Schedule::default();
        schedule.add_systems(trend_setting_system);
        schedule.run(&mut world);

        // Check Trend resource
        let trend = world.resource::<Trend>();
        assert_eq!(trend.current_item, Some(ItemType::LuxuryMeal));
        assert!(trend.strength > 0.0);
    }

    #[test]
    fn test_trend_spreads_to_nearby_labor() {
        let mut world = World::new();
        world.insert_resource(Trend {
            current_item: Some(ItemType::LuxuryMeal),
            strength: 1.0,
            setter_pos: GridPosition { x: 5, y: 5 },
        });

        // Spawn Laborer nearby
        let laborer = world
            .spawn((
                Pop,
                SocialClass::Labor,
                GridPosition { x: 6, y: 5 }, // Adjacent
                SocialMimicry::default(),
            ))
            .id();

        // Run spread system
        let mut schedule = Schedule::default();
        schedule.add_systems(trend_spread_system);
        schedule.run(&mut world);

        // Laborer should adopt the trend
        let mimicry = world.get::<SocialMimicry>(laborer).unwrap();
        assert_eq!(mimicry.desired_item, Some(ItemType::LuxuryMeal));
    }

    #[test]
    fn test_following_trend_grants_mood_buff() {
        let mut world = World::new();

        // Spawn Pop who wants LuxuryMeal and just ate it
        let pop = world
            .spawn((
                Pop,
                SocialMimicry {
                    desired_item: Some(ItemType::LuxuryMeal),
                },
                JustConsumed {
                    item: ItemType::LuxuryMeal,
                },
                Morale::default(),
            ))
            .id();

        // Run satisfaction system
        let mut schedule = Schedule::default();
        schedule.add_systems(trend_satisfaction_system);
        schedule.run(&mut world);

        // Check Morale modifiers
        let morale = world.get::<Morale>(pop).unwrap();
        assert!(morale.modifiers.iter().any(|m| m.label == "Feeling Trendy"));
    }

    #[test]
    fn test_clear_just_consumed() {
        let mut world = World::new();
        let entity = world
            .spawn(JustConsumed {
                item: ItemType::Potato,
            })
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(clear_just_consumed_system);
        schedule.run(&mut world);

        assert!(world.get::<JustConsumed>(entity).is_none());
    }
}

#[cfg(test)]
mod tests386 {
    use super::*;
    use crate::layer1::pop::Pop;
    use crate::layer1::social_stratification::Prestige;

    fn world_with_config(probability: f32) -> World {
        let mut world = World::new();
        world.insert_resource(MimicryConfig {
            radius: 6,
            probability,
            cooldown_ticks: 240,
            min_status_gap: 3,
        });
        world.init_resource::<MimicryStats>();
        world.init_resource::<Events<PreferenceChangedEvent>>();
        world
    }

    fn run(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(preference_mimicry_system);
        schedule.run(world);
    }

    fn spawn_pop(
        world: &mut World,
        x: i32,
        y: i32,
        prestige: u8,
        prefs: Preferences,
    ) -> Entity {
        world
            .spawn((Pop, GridPosition { x, y }, Prestige { value: prestige }, prefs))
            .id()
    }

    #[test]
    fn test386_low_status_mimics_high_status() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::LuxuryMeal)
        );
        assert_eq!(world.resource::<MimicryStats>().changes, 1);
    }

    #[test]
    fn test386_high_status_does_not_mimic_low_status() {
        let mut world = world_with_config(1.0);
        let high = spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(high).unwrap().favorite_food,
            Some(ItemType::LuxuryMeal)
        );
    }

    #[test]
    fn test386_mimicry_out_of_range() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            100,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
        assert_eq!(world.resource::<MimicryStats>().changes, 0);
    }

    #[test]
    fn test386_equal_status_no_mimicry() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            5,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let peer = spawn_pop(
            &mut world,
            1,
            0,
            5,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(peer).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
    }

    #[test]
    fn test386_small_status_gap_no_mimicry() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            7,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            5,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
    }

    #[test]
    fn test386_equipment_is_not_mimicked() {
        // Spec 386 section 8: consumables (food/drinks) only, no equipment.
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::Tool),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
        assert_eq!(world.resource::<MimicryStats>().changes, 0);
    }

    #[test]
    fn test386_drink_preference_mimicked() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: None,
                favorite_drink: Some(ItemType::Alcohol),
            },
        );
        let low = spawn_pop(
            &mut world,
            2,
            0,
            1,
            Preferences {
                favorite_food: None,
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_drink,
            Some(ItemType::Alcohol)
        );
    }

    #[test]
    fn test386_probability_zero_blocks_mimicry() {
        let mut world = world_with_config(0.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
    }

    #[test]
    fn test386_cooldown_blocks_immediate_second_change() {
        let mut world = world_with_config(1.0);
        let high = spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(world.resource::<MimicryStats>().changes, 1);
        // The elite changes their own taste; the mimic is on cooldown.
        world.get_mut::<Preferences>(high).unwrap().favorite_food = Some(ItemType::Meat);
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::LuxuryMeal)
        );
        assert_eq!(world.resource::<MimicryStats>().changes, 1);
        // Cooldown expiry re-enables mimicry.
        world.get_mut::<MimicryCooldown>(low).unwrap().remaining = 0;
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(low).unwrap().favorite_food,
            Some(ItemType::Meat)
        );
        assert_eq!(world.resource::<MimicryStats>().changes, 2);
    }

    #[test]
    fn test386_preference_change_event_emitted() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let low = spawn_pop(
            &mut world,
            1,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        let events = world.resource::<Events<PreferenceChangedEvent>>();
        let mut cursor = events.get_cursor();
        let emitted: Vec<&PreferenceChangedEvent> = cursor.read(events).collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].pop, low);
        assert_eq!(emitted[0].old_item, Some(ItemType::Potato));
        assert_eq!(emitted[0].new_item, Some(ItemType::LuxuryMeal));
        assert!(!emitted[0].drink);
    }

    #[test]
    fn test386_spatial_hash_radius_boundary() {
        let mut world = world_with_config(1.0);
        spawn_pop(
            &mut world,
            0,
            0,
            10,
            Preferences {
                favorite_food: Some(ItemType::LuxuryMeal),
                favorite_drink: None,
            },
        );
        let near = spawn_pop(
            &mut world,
            6,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        let far = spawn_pop(
            &mut world,
            7,
            0,
            1,
            Preferences {
                favorite_food: Some(ItemType::Potato),
                favorite_drink: None,
            },
        );
        run(&mut world);
        assert_eq!(
            world.get::<Preferences>(near).unwrap().favorite_food,
            Some(ItemType::LuxuryMeal)
        );
        assert_eq!(
            world.get::<Preferences>(far).unwrap().favorite_food,
            Some(ItemType::Potato)
        );
    }

    #[test]
    fn test386_ip_guard_public_strings_are_original() {
        let banned = [
            "sad king billy",
            "windsor",
            "rat-on-a-stick",
            "keeping up with",
            "influencer",
        ];
        for s in mimicry_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
