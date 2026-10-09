//! Spec 276: The Flesh Famine.
//!
//! When plant agriculture is destroyed (blight), pops degrade through a
//! unified [`DietPriority`]: crops -> hunted meat -> cannibalism.
//! Cannibalism inserts taboo and trauma memories, lowers morale, and shifts
//! the civic ideology to survivalist.

use crate::layer1::agriculture::farm::Farm;
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::entities::pop::Pop;
use crate::layer1::fauna::Fauna;
use crate::layer1::health::Dead;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::nature::fertility::FertilityGrid;
use crate::layer1::needs::Needs;
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::social::civic_ideology::{ActiveIdeology, IdeologyType};
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Hunger level at which pops are desperate enough to hunt or cannibalize.
pub const FAMINE_DESPERATION_HUNGER: f32 = 0.2;

/// Hunger restored by a famine meal (hunted meat or corpse).
pub const FAMINE_MEAL_SATIETY: f32 = 0.6;

/// Event that triggers a plant blight, destroying crops on all farms.
#[derive(Event, Debug, Clone)]
pub struct PlantBlight;

/// Marker for farms whose crops were destroyed by blight.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Blighted;

/// Unified diet priority: degrades gracefully as food sources vanish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DietPriority {
    /// Normal: eat from colony food stores (crops).
    Plant,
    /// Plant agriculture destroyed: hunt live fauna for meat.
    Meat,
    /// No fauna either: eat the dead.
    Cannibalism,
}

impl DietPriority {
    /// Determines the colony's diet priority from available food sources.
    #[must_use]
    pub fn for_colony(has_plant_food: bool, has_live_fauna: bool) -> Self {
        if has_plant_food {
            Self::Plant
        } else if has_live_fauna {
            Self::Meat
        } else {
            Self::Cannibalism
        }
    }
}

/// Tracks flesh-famine state (e.g. whether the chronicle entry fired).
#[derive(Resource, Default, Debug)]
pub struct FleshFamineState {
    /// True once cannibalism has occurred (chronicle fires once).
    pub cannibalism_has_occurred: bool,
}

/// Blight destroys crops: marks farms [`Blighted`] and ruins soil fertility.
pub fn trigger_blight_system(
    mut events: EventReader<PlantBlight>,
    mut commands: Commands,
    farms: Query<(Entity, &GridPosition), With<Farm>>,
    mut fertility: Option<ResMut<FertilityGrid>>,
    mut log: ResMut<MessageLog>,
) {
    for _event in events.read() {
        for (entity, pos) in &farms {
            commands.entity(entity).insert(Blighted);
            if let Some(grid) = fertility.as_deref_mut() {
                grid.set(pos.x.max(0) as usize, pos.y.max(0) as usize, 0.0);
            }
        }
        log.add("A blight sweeps through the fields! Crops wither and die.".to_string());
    }
}

/// Starving pops hunt live fauna when plant agriculture is gone.
pub fn hunt_fauna_during_famine_system(
    mut commands: Commands,
    mut pops: Query<(Entity, &mut Needs), (With<Pop>, Without<Dead>)>,
    fauna: Query<Entity, (With<Fauna>, Without<Dead>)>,
    farms: Query<&Farm, Without<Blighted>>,
    resources: Res<ColonyResources>,
) {
    let has_plant_food = !farms.is_empty() || resources.food > f32::EPSILON;
    if has_plant_food {
        return;
    }
    for (pop_entity, mut needs) in &mut pops {
        if needs.hunger > FAMINE_DESPERATION_HUNGER {
            continue;
        }
        if let Some(prey) = fauna.iter().next() {
            commands.entity(prey).despawn();
            needs.hunger = (needs.hunger + FAMINE_MEAL_SATIETY).min(1.0);
            let _ = pop_entity;
        }
    }
}

/// Starving pops eat the dead when no plants and no fauna remain.
/// Inserts taboo/trauma memories and shifts ideology to survivalist.
#[allow(clippy::too_many_arguments)]
pub fn cannibalism_during_famine_system(
    mut commands: Commands,
    mut pops: Query<
        (
            Entity,
            &mut Needs,
            Option<&mut Memories>,
            Option<&Traits>,
        ),
        (With<Pop>, Without<Dead>),
    >,
    corpses: Query<Entity, (With<Pop>, With<Dead>)>,
    fauna: Query<Entity, (With<Fauna>, Without<Dead>)>,
    farms: Query<&Farm, Without<Blighted>>,
    resources: Res<ColonyResources>,
    mut ideology: ResMut<ActiveIdeology>,
    mut famine_state: ResMut<FleshFamineState>,
    mut chronicle_events: EventWriter<AddChronicleEvent>,
    mut log: ResMut<MessageLog>,
    time: Res<SimulationTime>,
) {
    let has_plant_food = !farms.is_empty() || resources.food > f32::EPSILON;
    let diet = DietPriority::for_colony(has_plant_food, !fauna.is_empty());
    if diet != DietPriority::Cannibalism {
        return;
    }
    for (pop_entity, mut needs, memories_opt, traits_opt) in &mut pops {
        if needs.hunger > FAMINE_DESPERATION_HUNGER {
            continue;
        }
        let Some(corpse) = corpses.iter().next() else {
            continue;
        };
        commands.entity(corpse).despawn();
        needs.hunger = (needs.hunger + FAMINE_MEAL_SATIETY).min(1.0);

        // The desensitized feel nothing; everyone else gains taboo + trauma.
        let is_desensitized = traits_opt.is_some_and(|t| t.has(Trait::Cannibal));
        if !is_desensitized {
            let tick = time.tick;
            if let Some(mut memories) = memories_opt {
                memories.add(MemoryType::CannibalismTaboo, tick);
                memories.add(MemoryType::CannibalismTrauma, tick);
            } else {
                let mut memories = Memories::default();
                memories.add(MemoryType::CannibalismTaboo, tick);
                memories.add(MemoryType::CannibalismTrauma, tick);
                commands.entity(pop_entity).insert(memories);
            }
        }

        // Civility is abandoned for survival.
        ideology.0 = IdeologyType::Survivalist;

        if !famine_state.cannibalism_has_occurred {
            famine_state.cannibalism_has_occurred = true;
            chronicle_events.send(AddChronicleEvent {
                text: "Driven by starvation, the colony has turned to eating its own dead."
                    .to_string(),
                importance: EventImportance::Major,
            });
            log.add("The flesh famine begins: the starving eat the dead.".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};

    fn starving_needs() -> Needs {
        Needs {
            hunger: 0.05,
            rest: 1.0,
            leisure: 1.0,
            hygiene: 1.0,
        }
    }

    /// Colony resources with no food stores (famine conditions).
    fn empty_resources() -> ColonyResources {
        ColonyResources {
            food: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_diet_priority_degrades_gracefully() {
        assert_eq!(
            DietPriority::for_colony(true, true),
            DietPriority::Plant,
            "With plant food, diet stays plant-based"
        );
        assert_eq!(
            DietPriority::for_colony(true, false),
            DietPriority::Plant,
            "Plant food outranks meat even without fauna"
        );
        assert_eq!(
            DietPriority::for_colony(false, true),
            DietPriority::Meat,
            "Without plants but with fauna, pops hunt"
        );
        assert_eq!(
            DietPriority::for_colony(false, false),
            DietPriority::Cannibalism,
            "Without plants or fauna, pops turn to cannibalism"
        );
    }

    #[test]
    fn test_plant_blight_destroys_crops() {
        // Arrange: a farm with fertile soil.
        let mut app = App::new();
        app.add_event::<PlantBlight>();
        app.add_systems(Update, trigger_blight_system);
        app.insert_resource(FertilityGrid::new(10, 10));
        app.insert_resource(MessageLog::default());

        let farm = app
            .world_mut()
            .spawn((Farm::default(), GridPosition { x: 2, y: 3 }))
            .id();
        assert!(
            (app.world().resource::<FertilityGrid>().get(2, 3) - 1.0).abs() < f32::EPSILON,
            "Soil starts fertile"
        );

        // Act: trigger the blight.
        app.world_mut().send_event(PlantBlight);
        app.update();

        // Assert: crops dead (farm blighted), soil fertility ruined.
        assert!(
            app.world().get::<Blighted>(farm).is_some(),
            "Farm should be marked Blighted after blight"
        );
        assert!(
            app.world()
                .resource::<FertilityGrid>()
                .get(2, 3)
                .abs()
                < f32::EPSILON,
            "Soil fertility should be ruined by blight"
        );
    }

    #[test]
    fn test_flesh_famine_shifts_diet() {
        // Arrange: starving colony, no crops, some fauna.
        let mut app = App::new();
        app.add_systems(Update, hunt_fauna_during_famine_system);
        app.insert_resource(empty_resources());

        let pop = app.world_mut().spawn((Pop, starving_needs())).id();
        let prey = app.world_mut().spawn(Fauna::default()).id();

        // Act: advance time.
        app.update();

        // Assert: pops hunt and eat fauna.
        assert!(
            app.world().get_entity(prey).is_err(),
            "Fauna should be hunted (despawned)"
        );
        let needs = app.world().get::<Needs>(pop).unwrap();
        assert!(
            needs.hunger > 0.3,
            "Starving pop should have eaten the fauna"
        );
    }

    #[test]
    fn test_famine_hunt_ignored_when_plants_available() {
        // Arrange: starving pop, live fauna, but a working farm exists.
        let mut app = App::new();
        app.add_systems(Update, hunt_fauna_during_famine_system);
        app.insert_resource(ColonyResources::default());

        let pop = app.world_mut().spawn((Pop, starving_needs())).id();
        let prey = app.world_mut().spawn(Fauna::default()).id();
        app.world_mut().spawn(Farm::default()); // healthy farm

        // Act
        app.update();

        // Assert: no hunting while plant agriculture stands.
        assert!(
            app.world().get_entity(prey).is_ok(),
            "Fauna should NOT be hunted while farms produce"
        );
        let needs = app.world().get::<Needs>(pop).unwrap();
        assert!(
            needs.hunger < 0.2,
            "Pop should still be hungry (normal food system handles it)"
        );
    }

    #[test]
    fn test_cannibalism_generates_taboo_and_trauma() {
        // Arrange: starving colony, no crops, no fauna, one corpse.
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(Update, cannibalism_during_famine_system);
        app.insert_resource(empty_resources());
        app.insert_resource(FleshFamineState::default());
        app.insert_resource(ActiveIdeology::default());
        app.insert_resource(MessageLog::default());
        app.insert_resource(SimulationTime::default());

        let eater = app
            .world_mut()
            .spawn((Pop, starving_needs(), Memories::default()))
            .id();
        let corpse = app.world_mut().spawn((Pop, Dead)).id();

        // Act: advance time.
        app.update();

        // Assert: the dead are eaten; taboo + trauma memories inserted.
        assert!(
            app.world().get_entity(corpse).is_err(),
            "Corpse should be consumed"
        );
        let needs = app.world().get::<Needs>(eater).unwrap();
        assert!(needs.hunger > 0.3, "Eater should be fed by the corpse");
        let memories = app.world().get::<Memories>(eater).unwrap();
        assert!(
            memories
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::CannibalismTaboo),
            "Eater should gain a cannibalism taboo memory"
        );
        assert!(
            memories
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::CannibalismTrauma),
            "Eater should gain a cannibalism trauma memory"
        );
        // Ideology abandons civility for survival.
        assert_eq!(
            app.world().resource::<ActiveIdeology>().0,
            IdeologyType::Survivalist,
            "Cannibalism should shift civic ideology to survivalist"
        );
        // Chronicle fires once.
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(
            cursor.read(events).count(),
            1,
            "First cannibalism should trigger a chronicle event"
        );
    }

    #[test]
    fn test_cannibal_trait_pops_skip_trauma() {
        // Arrange: a desensitized cannibal-trait pop, one corpse.
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_systems(Update, cannibalism_during_famine_system);
        app.insert_resource(empty_resources());
        app.insert_resource(FleshFamineState::default());
        app.insert_resource(ActiveIdeology::default());
        app.insert_resource(MessageLog::default());
        app.insert_resource(SimulationTime::default());

        let mut traits = Traits::default();
        traits.add(Trait::Cannibal);
        let eater = app
            .world_mut()
            .spawn((Pop, starving_needs(), Memories::default(), traits))
            .id();
        let corpse = app.world_mut().spawn((Pop, Dead)).id();

        // Act
        app.update();

        // Assert: eats without trauma (already a cannibal).
        assert!(
            app.world().get_entity(corpse).is_err(),
            "Corpse should be consumed"
        );
        let memories = app.world().get::<Memories>(eater).unwrap();
        assert!(
            !memories
                .items
                .iter()
                .any(|m| m.memory_type == MemoryType::CannibalismTrauma),
            "Cannibal-trait pops should not gain trauma"
        );
    }

    #[test]
    fn test_cannibalism_memories_lower_morale() {
        assert!(
            MemoryType::CannibalismTaboo.base_mood_impact() < 0.0,
            "Taboo memory should lower morale"
        );
        assert!(
            MemoryType::CannibalismTrauma.base_mood_impact() < 0.0,
            "Trauma memory should lower morale"
        );
        assert!(
            MemoryType::CannibalismTrauma.base_mood_impact()
                < MemoryType::CannibalismTaboo.base_mood_impact(),
            "Trauma should hit harder than taboo"
        );
    }
}
