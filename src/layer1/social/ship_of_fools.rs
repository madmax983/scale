//! The Ship of Fools (Spec 1375).
//!
//! Not every ship is a threat; some are just tragedies. A crippled
//! pleasure-cruiser limps into orbit carrying passengers with no useful
//! skills and strong opinions about the catering. They eat your surplus,
//! produce nothing, and grumble when the colony isn't abundant enough for
//! their tastes.
//!
//! ## Tension
//!
//! Altruism vs. parasitism: you save them, they complain about the food,
//! and you slowly realize they are more dangerous than the pirates because
//! they consume without contributing.

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::pop::{Pop, PopBundle};
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::skills::Skills;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::social::old_guard::Arrival;
use crate::shared::time::SimulationTime;

/// Ticks between entitled grumbling checks (once per "shift", not per tick).
pub const GRUMBLE_PERIOD_TICKS: u64 = 100;
/// Food fraction below which the entitled start grumbling about the catering.
pub const GRUMBLE_FOOD_FRACTION: f32 = 0.5;
/// Morale penalty applied by each grumbling bout.
pub const GRUMBLE_MORALE_PENALTY: f32 = -0.1;

/// Event: a ship of fools has arrived in orbit.
#[derive(Event, Debug, Clone)]
pub struct ShipOfFoolsArrivalEvent {
    /// How many useless passengers are coming down.
    pub count: u32,
}

/// Marker component for pops who arrived on the Ship of Fools.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FoolPassenger;

/// Spawns ship-of-fools passengers: full pops with zero skills and the
/// Entitled trait, landing at colony tiles like other arrivals.
pub fn process_ship_of_fools_arrival_system(
    mut commands: Commands,
    mut events: EventReader<ShipOfFoolsArrivalEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    sim_time: Option<Res<SimulationTime>>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    // Fools land where the colony already is, not in the vacuum.
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();

    for event in events.read() {
        let mut rng = rand::thread_rng();

        for _ in 0..event.count {
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });

            let mut bundle = PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            // Fools have no skills...
            bundle.skills = Skills::default();
            // ...and exactly one trait: Entitled.
            let mut traits = Traits::default();
            traits.add(Trait::Entitled);
            bundle.traits = traits;
            bundle.arrival = Arrival { tick };

            commands.spawn((bundle, FoolPassenger));
        }

        chronicle.send(AddChronicleEvent {
            text: format!(
                "A crippled pleasure-cruiser has limped into orbit. {} bedraggled passengers are coming down — they have no skills, but they have opinions about the catering.",
                event.count
            ),
            importance: EventImportance::Major,
        });
    }
}

/// The entitled grumble when the colony isn't abundant enough for their
/// tastes: periodic morale penalty while food is below half of capacity.
/// This is the "demand luxury" pressure — keeping them quiet costs surplus.
pub fn entitled_grumbling_system(
    mut query: Query<&mut Morale, With<FoolPassenger>>,
    resources: Option<Res<ColonyResources>>,
    sim_time: Option<Res<SimulationTime>>,
) {
    let tick = sim_time.map_or(0, |t| t.tick);
    if !tick.is_multiple_of(GRUMBLE_PERIOD_TICKS) {
        return;
    }

    let comfortable = resources
        .as_ref()
        .is_none_or(|r| r.max_food <= 0.0 || r.food / r.max_food >= GRUMBLE_FOOD_FRACTION);
    if comfortable {
        return;
    }

    for mut morale in &mut query {
        morale.add_modifier(MoodModifier {
            label: "Grumbling about the food".to_string(),
            value: GRUMBLE_MORALE_PENALTY,
            duration: GRUMBLE_PERIOD_TICKS as u32,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::skills::SkillType;

    fn setup() -> World {
        crate::setup::init_task_pools();
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<ShipOfFoolsArrivalEvent>::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    fn spawn_colonist(world: &mut World) {
        // One existing pop so the fools have a colony tile to land on.
        world.spawn((Pop, GridPosition { x: 10, y: 10 }));
    }

    fn run_arrival(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(process_ship_of_fools_arrival_system);
        schedule.run(world);
    }

    #[test]
    fn test_ship_of_fools_spawns_useless_pops() {
        let mut world = setup();
        spawn_colonist(&mut world);

        world
            .resource_mut::<Events<ShipOfFoolsArrivalEvent>>()
            .send(ShipOfFoolsArrivalEvent { count: 5 });
        run_arrival(&mut world);

        let mut query = world.query_filtered::<(&Skills, &Traits), With<FoolPassenger>>();
        let mut count = 0;
        for (skills, traits) in query.iter(&world) {
            count += 1;
            for skill in [
                SkillType::Mining,
                SkillType::Forestry,
                SkillType::Farming,
                SkillType::Construction,
                SkillType::Crafting,
                SkillType::Husbandry,
                SkillType::Engineering,
                SkillType::Science,
            ] {
                assert_eq!(
                    skills.get_xp(skill),
                    0.0,
                    "Fools should have zero skills"
                );
            }
            assert!(
                traits.has(Trait::Entitled),
                "Fools should have the Entitled trait"
            );
        }
        assert_eq!(count, 5, "Should spawn exact number of fools");
    }

    #[test]
    fn test_fools_land_at_colony_tiles() {
        let mut world = setup();
        spawn_colonist(&mut world);

        world
            .resource_mut::<Events<ShipOfFoolsArrivalEvent>>()
            .send(ShipOfFoolsArrivalEvent { count: 3 });
        run_arrival(&mut world);

        let mut query = world.query_filtered::<&GridPosition, With<FoolPassenger>>();
        for pos in query.iter(&world) {
            assert_eq!((pos.x, pos.y), (10, 10), "Fools land at colony tiles");
        }
    }

    #[test]
    fn test_fools_have_full_pop_bundle() {
        use crate::layer1::pop::PopName;
        use crate::layer1::psychology::needs::Needs;

        let mut world = setup();
        spawn_colonist(&mut world);

        world
            .resource_mut::<Events<ShipOfFoolsArrivalEvent>>()
            .send(ShipOfFoolsArrivalEvent { count: 2 });
        run_arrival(&mut world);

        // Full pops, not ghosts: they need names, health, needs, morale.
        let mut query = world.query_filtered::<
            (&PopName, &Needs, &Morale),
            With<FoolPassenger>,
        >();
        assert_eq!(query.iter(&world).count(), 2);
    }

    #[test]
    fn test_entitled_grumble_when_food_scarce() {
        let mut world = setup();
        let resources = ColonyResources {
            food: 10.0,
            max_food: 100.0,
            ..Default::default()
        };
        world.insert_resource(resources);
        // Tick 100: a grumble tick.
        world.resource_mut::<SimulationTime>().tick = GRUMBLE_PERIOD_TICKS;

        let fool = world
            .spawn((FoolPassenger, Morale::default()))
            .id();
        // A non-fool should not grumble.
        let normal = world.spawn(Morale::default()).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(entitled_grumbling_system);
        schedule.run(&mut world);

        let fool_morale = world.get::<Morale>(fool).unwrap();
        assert!(
            fool_morale
                .modifiers
                .iter()
                .any(|m| m.label == "Grumbling about the food"),
            "Entitled fool should grumble when food is scarce"
        );
        let normal_morale = world.get::<Morale>(normal).unwrap();
        assert!(
            normal_morale.modifiers.is_empty(),
            "Non-fools should not grumble"
        );
    }

    #[test]
    fn test_entitled_quiet_when_food_abundant() {
        let mut world = setup();
        let resources = ColonyResources {
            food: 90.0,
            max_food: 100.0,
            ..Default::default()
        };
        world.insert_resource(resources);
        world.resource_mut::<SimulationTime>().tick = GRUMBLE_PERIOD_TICKS;

        let fool = world
            .spawn((FoolPassenger, Morale::default()))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(entitled_grumbling_system);
        schedule.run(&mut world);

        let morale = world.get::<Morale>(fool).unwrap();
        assert!(
            morale.modifiers.is_empty(),
            "Fools stay quiet when the colony is abundant"
        );
    }

    #[test]
    fn test_grumble_only_on_period_ticks() {
        let mut world = setup();
        let resources = ColonyResources {
            food: 10.0,
            max_food: 100.0,
            ..Default::default()
        };
        world.insert_resource(resources);
        // Not a grumble tick.
        world.resource_mut::<SimulationTime>().tick = GRUMBLE_PERIOD_TICKS + 1;

        let fool = world
            .spawn((FoolPassenger, Morale::default()))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(entitled_grumbling_system);
        schedule.run(&mut world);

        let morale = world.get::<Morale>(fool).unwrap();
        assert!(
            morale.modifiers.is_empty(),
            "Grumbling only fires on period ticks"
        );
    }

    #[test]
    fn test_arrival_sends_chronicle() {
        let mut world = setup();
        spawn_colonist(&mut world);

        world
            .resource_mut::<Events<ShipOfFoolsArrivalEvent>>()
            .send(ShipOfFoolsArrivalEvent { count: 4 });
        run_arrival(&mut world);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_cursor();
        let entries: Vec<_> = reader.read(events).collect();
        assert_eq!(entries.len(), 1, "One chronicle entry per arrival");
        assert!(
            entries[0].text.contains("pleasure-cruiser"),
            "Chronicle mentions the pleasure-cruiser"
        );
    }
}
