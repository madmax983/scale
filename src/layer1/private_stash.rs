//! Private Stashes system (Spec 103, extended by Spec 350).
//!
//! Pops with `Greedy` or `Anxious` traits steal resources and hide them.
//!
//! Spec 350 ("Private Stashes"): in a shortage, pops hide food in their
//! rooms and the colony's inventory counts become inaccurate — stashed
//! goods are *not* part of [`ColonyResources`], so every stockpile count
//! (and [`calculate_visible_food`]) understates what the colony actually
//! holds. The owner can still eat from their own stash when hungry
//! ([`stash_access_system`]); nobody else can. The colony's counterplay
//! is [`execute_search_rooms`]: every stash is emptied back into the
//! stockpile, at a massive, long-lived morale penalty for everyone.

use crate::layer1::balance::{FOOD_HUNGER_THRESHOLD, FOOD_PER_MEAL, HUNGER_PER_MEAL};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::traits::{Trait, Traits};
use bevy_ecs::prelude::*;
use rand::Rng;
use std::collections::HashMap;

/// Colony food below this is a shortage: anxious pops start hiding food.
pub const SHORTAGE_FOOD_THRESHOLD: f32 = 10.0;

/// Mood label applied colony-wide when rooms are searched (Spec 350).
pub const SEARCH_ROOMS_MORALE_LABEL: &str = "Rooms Searched";

/// Morale hit per pop when rooms are searched — deliberately massive:
/// the search works, and everyone resents it for a long time.
pub const SEARCH_ROOMS_MORALE_PENALTY: f32 = -0.40;

/// How long the Search Rooms resentment lasts, in ticks.
pub const SEARCH_ROOMS_MORALE_DURATION: u32 = 2000;

/// Chronicle line written when a search actually recovers hidden goods.
pub const SEARCH_ROOMS_CHRONICLE: &str =
    "The rooms were searched. What was hidden is colony property again — nobody will forget the knock on the door.";

/// Component storing resources stolen by a Pop.
#[derive(Component, Debug, Clone, Default)]
pub struct PrivateStash {
    /// Entity ID of the Pop that owns this stash.
    pub owner: Option<Entity>,
    /// Map of resource type to amount stored.
    pub inventory: HashMap<ResourceType, f32>,
}

impl PrivateStash {
    /// Adds a resource to the stash.
    pub fn add(&mut self, res: ResourceType, amount: f32) {
        *self.inventory.entry(res).or_insert(0.0) += amount;
    }

    /// Gets the amount of a resource in the stash.
    #[must_use]
    pub fn get(&self, res: ResourceType) -> f32 {
        *self.inventory.get(&res).unwrap_or(&0.0)
    }

    /// Takes all resources from the stash, clearing it.
    pub fn take_all(&mut self) -> HashMap<ResourceType, f32> {
        std::mem::take(&mut self.inventory)
    }

    /// Total amount stored across all resource types.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.inventory.values().sum()
    }

    /// True when the stash holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inventory.values().all(|v| *v <= 0.0)
    }

    /// Removes up to `amount` of `res`, returning how much was removed.
    pub fn remove(&mut self, res: ResourceType, amount: f32) -> f32 {
        let entry = self.inventory.entry(res).or_insert(0.0);
        let taken = amount.min(*entry).max(0.0);
        *entry -= taken;
        if *entry <= 0.0 {
            self.inventory.remove(&res);
        }
        taken
    }
}

/// System where pops with Anxious traits create stashes when resources are low.
pub fn stash_creation_system(
    mut commands: Commands,
    mut resources: ResMut<ColonyResources>,
    query: Query<(Entity, &Traits), With<Pop>>,
) {
    let mut rng = rand::thread_rng();

    if resources.food < SHORTAGE_FOOD_THRESHOLD {
        for (entity, traits) in query.iter() {
            if (traits.has(Trait::Anxious) || traits.has(Trait::Hoarder)) && resources.food >= 1.0 {
                // Throttle stash creation with a probability.
                // Balance (2026-10-14 baseline): 0.001 produced 3 stashes
                // across 48 bot colonies (up to 2000 ticks each) — organic
                // hoarding was effectively invisible. 0.01 targets a few
                // visible stashes per shortage without noise.
                if rng.gen_bool(0.01) {
                    resources.food -= 1.0;
                    let mut stash = PrivateStash {
                        owner: Some(entity),
                        ..Default::default()
                    };
                    stash.add(ResourceType::Food, 1.0);
                    commands.spawn(stash);
                }
            }
        }
    }
}

/// Calculates total visible food, ignoring private stashes.
pub fn calculate_visible_food(world: &World) -> f32 {
    let resources = world.resource::<ColonyResources>();
    resources.food
}

/// Number of non-empty private stashes in the world (headless STATS).
/// Counts both stashes carried on a pop and standalone stash entities
/// spawned by [`stash_creation_system`].
pub fn stash_count(world: &mut World) -> usize {
    let mut query = world.query::<&PrivateStash>();
    query.iter(world).filter(|s| !s.is_empty()).count()
}

/// Total food hidden across all private stashes — the gap between what
/// the colony *has* and what its inventory says it has (headless STATS).
pub fn stashed_food_total(world: &mut World) -> f32 {
    let mut query = world.query::<&PrivateStash>();
    query.iter(world).map(|s| s.get(ResourceType::Food)).sum()
}

/// Lets a hungry pop eat from their *own* stash (Spec 350 REFACTOR).
///
/// Two stash shapes exist: a `PrivateStash` component carried on the pop
/// itself (hoarding / pirate pension), and a standalone stash entity
/// whose `owner` points at the pop (spawned by `stash_creation_system`).
/// Both are private: a pop only ever eats from a stash they own, one
/// standard meal ([`FOOD_PER_MEAL`] food for [`HUNGER_PER_MEAL`] hunger)
/// per tick, and only once they are actually hungry
/// ([`FOOD_HUNGER_THRESHOLD`] — the same threshold the colony kitchen
/// uses in `consume_food_system`).
#[allow(clippy::type_complexity)]
pub fn stash_access_system(
    mut set: ParamSet<(
        Query<(&mut Needs, &mut PrivateStash), With<Pop>>,
        Query<(Entity, &mut Needs), With<Pop>>,
        Query<&mut PrivateStash, Without<Pop>>,
    )>,
) {
    // Stashes carried on the pop's own entity.
    for (mut needs, mut stash) in &mut set.p0() {
        if needs.hunger < FOOD_HUNGER_THRESHOLD {
            let eaten = stash.remove(ResourceType::Food, FOOD_PER_MEAL);
            if eaten > 0.0 {
                needs.hunger = (needs.hunger + HUNGER_PER_MEAL * (eaten / FOOD_PER_MEAL)).min(1.0);
            }
        }
    }
    // Standalone stashes: collect the hungry owners first (the pop and
    // stash queries cannot be live at once), drain one meal from each
    // owned stash, then apply the hunger gains.
    let hungry_owners: Vec<Entity> = set
        .p1()
        .iter()
        .filter(|(_, needs)| needs.hunger < FOOD_HUNGER_THRESHOLD)
        .map(|(entity, _)| entity)
        .collect();
    if hungry_owners.is_empty() {
        return;
    }
    let mut fed: Vec<Entity> = Vec::new();
    for mut stash in &mut set.p2() {
        let Some(owner) = stash.owner else { continue };
        if !hungry_owners.contains(&owner) || fed.contains(&owner) {
            continue;
        }
        if stash.remove(ResourceType::Food, FOOD_PER_MEAL) > 0.0 {
            fed.push(owner);
        }
    }
    if fed.is_empty() {
        return;
    }
    for (entity, mut needs) in &mut set.p1() {
        if fed.contains(&entity) {
            needs.hunger = (needs.hunger + HUNGER_PER_MEAL).min(1.0);
        }
    }
}

/// Returns confiscated goods to the colony stockpile. Uses the generic
/// `add_resource` path so *every* resource type comes back (the old
/// hand-written match in `inspect_pop` silently dropped Ore and more);
/// `HyperValuable` is special-cased because `add_resource` subtracts it
/// (a pre-existing bug in resources.rs, left alone here).
fn return_goods_to_colony(resources: &mut ColonyResources, goods: HashMap<ResourceType, f32>) {
    for (res_type, amount) in goods {
        if amount <= 0.0 {
            continue;
        }
        if res_type == ResourceType::HyperValuable {
            resources.hyper_valuable =
                (resources.hyper_valuable + amount).min(resources.max_hyper_valuable);
        } else {
            resources.add_resource(&res_type, amount);
        }
    }
}

/// Outcome of a Search Rooms action (Spec 350).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SearchRoomsReport {
    /// Non-empty stashes found and emptied.
    pub stashes_searched: usize,
    /// Food recovered into the stockpile.
    pub food_recovered: f32,
    /// Total goods recovered across all resource types.
    pub total_recovered: f32,
}

/// "Search Rooms" (Spec 350 REFACTOR): empties every private stash in the
/// colony back into the stockpile — and every pop, owner or not, takes a
/// massive, long-lived morale hit ([`SEARCH_ROOMS_MORALE_PENALTY`] for
/// [`SEARCH_ROOMS_MORALE_DURATION`] ticks). A search that finds nothing
/// costs nothing: no stashes, no search, no penalty. Writes a Major
/// chronicle entry when goods are recovered and the chronicle event
/// channel exists.
pub fn execute_search_rooms(world: &mut World) -> SearchRoomsReport {
    // 1. Drain every stash (carried and standalone alike).
    let mut report = SearchRoomsReport::default();
    let mut recovered: HashMap<ResourceType, f32> = HashMap::new();
    {
        let mut query = world.query::<&mut PrivateStash>();
        for mut stash in query.iter_mut(world) {
            if stash.is_empty() {
                continue;
            }
            report.stashes_searched += 1;
            for (res, amount) in stash.take_all() {
                report.total_recovered += amount;
                if res == ResourceType::Food {
                    report.food_recovered += amount;
                }
                *recovered.entry(res).or_insert(0.0) += amount;
            }
        }
    }
    if report.stashes_searched == 0 {
        return report;
    }

    // 2. Goods return to the visible stockpile.
    if world.get_resource::<ColonyResources>().is_some() {
        let mut resources = world.resource_mut::<ColonyResources>();
        return_goods_to_colony(&mut resources, recovered);
    }

    // 3. The whole colony resents the knock on the door.
    {
        let mut query = world.query::<&mut Morale>();
        for mut morale in query.iter_mut(world) {
            morale.add_modifier(MoodModifier {
                label: SEARCH_ROOMS_MORALE_LABEL.to_string(),
                value: SEARCH_ROOMS_MORALE_PENALTY,
                duration: SEARCH_ROOMS_MORALE_DURATION,
            });
        }
    }

    // 4. Chronicle, when the event channel exists.
    if world
        .get_resource::<bevy_ecs::event::Events<AddChronicleEvent>>()
        .is_some()
    {
        world
            .resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: SEARCH_ROOMS_CHRONICLE.to_string(),
                importance: EventImportance::Major,
            });
    }

    report
}

/// Public-facing strings, for the IP-guard test.
pub fn stash_public_strings() -> Vec<String> {
    vec![
        SEARCH_ROOMS_MORALE_LABEL.to_string(),
        SEARCH_ROOMS_CHRONICLE.to_string(),
        "Rooms Searched".to_string(),
        "Private stash".to_string(),
    ]
}

/// System where Pops with specific traits steal resources.
pub fn hoarding_system(
    mut resources: ResMut<ColonyResources>,
    mut query: Query<(&Traits, &mut PrivateStash)>,
) {
    let mut rng = rand::thread_rng();

    for (traits, mut stash) in &mut query {
        // Small chance to steal per tick (e.g., 0.1%)
        if !rng.gen_bool(0.001) {
            continue;
        }

        if traits.has(Trait::Anxious) {
            // Steal Food
            // Using direct check/subtraction as spec implies simple theft logic
            if resources.food >= 1.0 {
                resources.food -= 1.0;
                stash.add(ResourceType::Food, 1.0);
            }
        }

        if traits.has(Trait::Greedy) {
            // Steal Metal
            if resources.metal >= 1.0 {
                resources.metal -= 1.0;
                stash.add(ResourceType::Metal, 1.0);
            }
        }
    }
}

/// Inspects a pop and confiscates their stash.
pub fn inspect_pop(world: &mut World, pop_entity: Entity) {
    // 1. Get stash content
    let stolen_goods = if let Some(mut stash) = world.get_mut::<PrivateStash>(pop_entity) {
        stash.take_all()
    } else {
        return;
    };

    if stolen_goods.is_empty() {
        return;
    }

    // 2. Return to global resources (every resource type — see
    // `return_goods_to_colony`; the old hand-written match dropped Ore).
    let mut resources = world.resource_mut::<ColonyResources>();
    return_goods_to_colony(&mut resources, stolen_goods);

    // Log/Notification would go here.
}

#[cfg(test)]
mod tests {
    use super::{hoarding_system, inspect_pop, PrivateStash};
    use crate::layer1::pop::Pop;
    use crate::layer1::resources::{ColonyResources, ResourceType};
    use crate::layer1::traits::{Trait, Traits};
    use bevy_ecs::prelude::*;

    #[test]
    fn test_private_stash_component() {
        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, 5.0);
        assert_eq!(stash.get(ResourceType::Food), 5.0);
        assert_eq!(stash.get(ResourceType::Wood), 0.0);
    }

    #[test]
    fn test_hoarding_system_anxious_steals_food() {
        let mut world = World::new();
        // Setup resources
        let resources = ColonyResources {
            food: 100.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        // Setup Anxious Pop
        let pop = world
            .spawn((
                Pop,
                {
                    let mut t = Traits::default();
                    t.add(Trait::Anxious);
                    t
                }, // New trait
                PrivateStash::default(),
            ))
            .id();

        // Create a schedule to run the system
        let mut schedule = Schedule::default();
        schedule.add_systems(hoarding_system);

        // Run system multiple times to ensure probability hits
        // We need to loop enough times because probability is 0.001 (1/1000)
        // 100 times might not be enough if it's 0.1%.
        // The spec said 0.1% (0.001) but the test loop was 100.
        // 100 * 0.001 = 0.1 expected hits. That's too low for a deterministic test unless we mock RNG.
        // I'll increase the loop count for the test to ensure it hits at least once.
        // Or I'll just accept that with standard RNG it might flake, but the spec test had 100.
        // Maybe the spec meant 1% (0.01)? "Small chance to steal per tick (e.g., 0.1%)"
        // I will trust the code 0.001. I'll bump the loop to 50000 to be safe-ish, or use a seed if possible.
        // Since I can't easily inject a seeded RNG into the system without changing its signature significantly...
        // I will bump the loop count.

        // Wait, if I want deterministic tests, I should probably check for > 0 but with a VERY high loop count it slows down.
        // Alternatively, for the sake of the test "Minimal Implementation", I could expose the probability or make it configurable.
        // BUT, I can also just patch the RNG in the system? No.

        // I will run it many times.
        for _ in 0..50000 {
            schedule.run(&mut world);
            // Optimization: Break early if theft occurred
            if world
                .get::<PrivateStash>(pop)
                .expect("Missing resource or component")
                .get(ResourceType::Food)
                > 0.0
            {
                break;
            }
        }

        // Check global resources decreased
        let res = world.resource::<ColonyResources>();
        assert!(res.food < 100.0, "Food should be stolen");

        // Check stash increased
        let stash = world
            .get::<PrivateStash>(pop)
            .expect("Missing resource or component");
        assert!(
            stash.get(ResourceType::Food) > 0.0,
            "Stash should contain Food"
        );
    }

    #[test]
    fn test_hoarding_system_greedy_steals_valuables() {
        let mut world = World::new();
        let resources = ColonyResources {
            metal: 50.0,
            ..Default::default()
        };
        world.insert_resource(resources);

        let pop = world
            .spawn((
                Pop,
                {
                    let mut t = Traits::default();
                    t.add(Trait::Greedy);
                    t
                }, // New trait
                PrivateStash::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(hoarding_system);

        for _ in 0..50000 {
            schedule.run(&mut world);
            if world
                .get::<PrivateStash>(pop)
                .expect("Missing resource or component")
                .get(ResourceType::Metal)
                > 0.0
            {
                break;
            }
        }

        let res = world.resource::<ColonyResources>();
        assert!(res.metal < 50.0, "Metal should be stolen");

        let stash = world
            .get::<PrivateStash>(pop)
            .expect("Missing resource or component");
        assert!(
            stash.get(ResourceType::Metal) > 0.0,
            "Stash should contain Metal"
        );
    }

    #[test]
    fn test_discovery_returns_resources() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, 10.0);

        let pop = world.spawn((Pop, stash, Traits::default())).id();

        // Inspect the pop
        inspect_pop(&mut world, pop);

        // Stash should be empty
        let stash = world
            .get::<PrivateStash>(pop)
            .expect("Missing resource or component");
        assert_eq!(stash.get(ResourceType::Food), 0.0);

        // Global resources should have the food back
        let res = world.resource::<ColonyResources>();
        // Default starts at 10.0 + 10.0 recovered = 20.0
        assert_eq!(res.food, 20.0);
    }

    #[test]
    fn test_inspect_pop_empty_stash() {
        let mut world = World::new();
        let res_before = ColonyResources::default();
        world.insert_resource(res_before);

        let pop = world
            .spawn((Pop, PrivateStash::default(), Traits::default()))
            .id();

        inspect_pop(&mut world, pop);

        let res_after = world.resource::<ColonyResources>();
        // Should be identical (no panic, no change)
        assert!((res_after.food - res_before.food).abs() < f32::EPSILON);
    }

    #[test]
    fn test_anxious_pop_creates_stash() {
        // Arrange
        let mut world = World::new();
        // Pop has Anxious trait and there is a global food shortage
        let res = ColonyResources {
            food: 5.0,
            ..Default::default()
        };
        world.insert_resource(res);

        let _pop = world
            .spawn((Pop, {
                let mut t = Traits::default();
                t.add(Trait::Anxious);
                t
            }))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(super::stash_creation_system);

        // Run system many times to ensure probability hits
        for _ in 0..50000 {
            schedule.run(&mut world);
            // Optimization: Break early if stash created
            if world.query::<&PrivateStash>().iter(&world).count() > 0 {
                break;
            }
        }

        // Assert
        let stashes = world.query::<&PrivateStash>().iter(&world).count();
        assert_eq!(
            stashes, 1,
            "Anxious pop should create a stash during a shortage"
        );

        // Assert it took resources
        let res = world.resource::<ColonyResources>();
        assert!(res.food < 5.0, "Food should have decreased");
    }

    #[test]
    fn test_stash_hides_from_global_inventory() {
        // Arrange
        let mut world = World::new();
        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, 10.0);
        world.spawn(stash);

        let res = ColonyResources {
            food: 20.0,
            ..Default::default()
        };
        world.insert_resource(res);

        // Act
        let total_visible_food = super::calculate_visible_food(&world);

        // Assert
        assert_eq!(
            total_visible_food, 20.0,
            "Stashed food should not be visible to the colony"
        );
    }

    #[test]
    fn test_inspect_pop_mixed_resources() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());

        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, 5.0);
        stash.add(ResourceType::Metal, 2.0);
        stash.add(ResourceType::Wood, 3.0);
        stash.add(ResourceType::Ore, 1.0);

        let pop = world.spawn((Pop, stash, Traits::default())).id();

        inspect_pop(&mut world, pop);

        let res = world.resource::<ColonyResources>();
        // Food: 10 + 5 = 15
        assert!((res.food - 15.0).abs() < f32::EPSILON);
        // Metal: 0 + 2 = 2
        assert!((res.metal - 2.0).abs() < f32::EPSILON);
        // Wood: 15 + 3 = 18
        assert!((res.wood - 18.0).abs() < f32::EPSILON);
        // Ore: 0 + 1 = 1 (Spec 350: return_goods_to_colony covers every type;
        // the old hand-written match silently dropped Ore.)
        assert!((res.ore - 1.0).abs() < f32::EPSILON);
    }

    // ---- Spec 350: owner access, Search Rooms, visibility counts ----

    use crate::layer1::needs::Needs;
    use crate::layer1::social::morale::Morale;
    use bevy_ecs::system::RunSystemOnce;

    fn hungry_pop_with_stash(world: &mut World, hunger: f32, food: f32) -> Entity {
        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, food);
        world
            .spawn((
                Pop,
                Needs {
                    hunger,
                    ..Default::default()
                },
                stash,
                Traits::default(),
            ))
            .id()
    }

    #[test]
    fn test_hungry_owner_eats_from_own_stash() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        let pop = hungry_pop_with_stash(&mut world, 0.1, 1.0);

        world.run_system_once(super::stash_access_system).unwrap();

        let needs = world.get::<Needs>(pop).unwrap();
        assert!(
            (needs.hunger - 0.4).abs() < 0.001,
            "owner should eat one meal from own stash: hunger 0.1 + 0.3, got {}",
            needs.hunger
        );
        let stash = world.get::<PrivateStash>(pop).unwrap();
        assert!(
            (stash.get(ResourceType::Food) - 0.9).abs() < 0.001,
            "one meal (0.1 food) should leave the stash, got {}",
            stash.get(ResourceType::Food)
        );
        // Colony stockpile untouched: eating from a stash is private.
        assert!((world.resource::<ColonyResources>().food - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sated_owner_does_not_touch_stash() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        let pop = hungry_pop_with_stash(&mut world, 0.9, 1.0);

        world.run_system_once(super::stash_access_system).unwrap();

        assert!((world.get::<Needs>(pop).unwrap().hunger - 0.9).abs() < f32::EPSILON);
        assert!(
            (world
                .get::<PrivateStash>(pop)
                .unwrap()
                .get(ResourceType::Food)
                - 1.0)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn test_hungry_owner_eats_from_owned_standalone_stash() {
        // stash_creation_system spawns the stash as its own entity whose
        // `owner` points at the pop — the owner must be able to eat from it,
        // and nobody else may.
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        let owner = hungry_pop_with_stash(&mut world, 0.8, 0.0);
        let bystander = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.1,
                    ..Default::default()
                },
                Traits::default(),
            ))
            .id();
        let mut stash = PrivateStash {
            owner: Some(owner),
            ..Default::default()
        };
        stash.add(ResourceType::Food, 1.0);
        world.spawn(stash);
        // Make the owner hungry too.
        world.get_mut::<Needs>(owner).unwrap().hunger = 0.1;

        world.run_system_once(super::stash_access_system).unwrap();

        assert!(
            (world.get::<Needs>(owner).unwrap().hunger - 0.4).abs() < 0.001,
            "owner eats from their standalone stash"
        );
        assert!(
            (world.get::<Needs>(bystander).unwrap().hunger - 0.1).abs() < f32::EPSILON,
            "a non-owner must not eat from someone else's stash"
        );
    }

    #[test]
    fn test_search_rooms_returns_goods_and_tanks_morale() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        world.insert_resource(bevy_ecs::event::Events::<
            crate::layer1::core::chronicle::AddChronicleEvent,
        >::default());
        let pop = hungry_pop_with_stash(&mut world, 0.8, 0.0);
        world
            .get_mut::<PrivateStash>(pop)
            .unwrap()
            .add(ResourceType::Food, 4.0);
        world
            .get_mut::<PrivateStash>(pop)
            .unwrap()
            .add(ResourceType::Metal, 2.0);
        world.entity_mut(pop).insert(Morale::default());
        let other = world
            .spawn((Pop, Morale::default(), Traits::default()))
            .id();

        let report = super::execute_search_rooms(&mut world);

        assert_eq!(report.stashes_searched, 1);
        assert!((report.food_recovered - 4.0).abs() < f32::EPSILON);
        assert!((report.total_recovered - 6.0).abs() < f32::EPSILON);
        let res = world.resource::<ColonyResources>();
        assert!(
            (res.food - 14.0).abs() < f32::EPSILON,
            "food back in the stockpile"
        );
        assert!((res.metal - 2.0).abs() < f32::EPSILON);
        assert_eq!(
            super::stash_count(&mut world),
            0,
            "stash emptied by the search"
        );
        // Massive morale penalty on every pop, stash owner or not.
        for e in [pop, other] {
            let morale = world.get::<Morale>(e).unwrap();
            assert_eq!(morale.modifiers.len(), 1);
            assert_eq!(morale.modifiers[0].label, super::SEARCH_ROOMS_MORALE_LABEL);
            assert!(
                morale.modifiers[0].value <= -0.35,
                "penalty must be massive, got {}",
                morale.modifiers[0].value
            );
            assert!(morale.modifiers[0].duration >= 1000);
        }
    }

    #[test]
    fn test_search_rooms_with_no_stashes_is_quiet() {
        let mut world = World::new();
        world.insert_resource(ColonyResources::default());
        let pop = world
            .spawn((Pop, Morale::default(), Traits::default()))
            .id();

        let report = super::execute_search_rooms(&mut world);

        assert_eq!(report.stashes_searched, 0);
        assert!((report.total_recovered).abs() < f32::EPSILON);
        assert!(
            world.get::<Morale>(pop).unwrap().modifiers.is_empty(),
            "no stashes, no search, no penalty"
        );
    }

    #[test]
    fn test_stash_counts_and_hidden_food_total() {
        let mut world = World::new();
        world.insert_resource(ColonyResources {
            food: 20.0,
            ..Default::default()
        });
        let mut stash = PrivateStash::default();
        stash.add(ResourceType::Food, 10.0);
        world.spawn(stash);
        world.spawn(PrivateStash::default()); // empty stash entity: not counted

        assert_eq!(super::stash_count(&mut world), 1);
        assert!((super::stashed_food_total(&mut world) - 10.0).abs() < f32::EPSILON);
        // Spec RED test 2: visible food is the stockpile only.
        assert_eq!(super::calculate_visible_food(&world), 20.0);
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Spec 350 is concept-only: generic shortage/hoarding language,
        // nothing lifted from outside fiction.
        let banned = [
            "sad king billy",
            "windsor",
            "windsor-in-exile",
            "dune",
            "arrakis",
            "hitchhiker",
            "improbability drive",
            "shimmer",
            "area x",
            "foundation",
            "psychohistory",
        ];
        for s in super::stash_public_strings() {
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
