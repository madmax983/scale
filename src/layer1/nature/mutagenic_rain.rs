use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::lighting::{AmbientLight, LightMap};
use crate::layer1::map::GridPosition;
use crate::layer1::nature::weather::{WeatherState, WeatherType};
use crate::layer1::needs::Needs;
use crate::layer1::pop::{Pop, PopName};
use crate::layer1::structural_integrity::RoofGrid;
use crate::layer1::traits::{Trait, Traits};
use bevy_ecs::prelude::*;
use rand::Rng;

/// The four mutant traits granted by mutagenic rain (Spec 427).
pub const MUTANT_TRAITS: [Trait; 4] = [
    Trait::Photosynthesis,
    Trait::ThickSkin,
    Trait::BrittleBones,
    Trait::ExtremeHunger,
];

/// True when the pop carries at least one mutant trait.
#[must_use]
pub fn has_mutant_trait(traits: &Traits) -> bool {
    MUTANT_TRAITS.iter().any(|t| traits.has(*t))
}

/// If a pop has Photosynthesis and is exposed to light above this
/// threshold, it regenerates hunger (mirrors the experimental
/// `photosynthetic_nourishment` pattern, wired in layer1 for Spec 427).
pub const PHOTOSYNTHESIS_LIGHT_THRESHOLD: f32 = 0.5;
/// Hunger regenerated per tick while photosynthesizing.
pub const PHOTOSYNTHESIS_NOURISHMENT_RATE: f32 = 0.002;

/// Spec 427 Leg 2: pops with `Trait::Photosynthesis` slowly regain
/// hunger while standing in bright light (local LightMap tile or
/// ambient daylight, whichever is brighter). Runs in layer1 — the
/// experimental copy only runs under the `nova` feature.
pub fn photosynthesis_nourishment_system(
    light_map: Option<Res<LightMap>>,
    ambient_light: Option<Res<AmbientLight>>,
    mut query: Query<(&GridPosition, &Traits, &mut Needs), With<Pop>>,
) {
    let (Some(light_map), Some(ambient_light)) = (light_map, ambient_light) else {
        return;
    };
    for (pos, traits, mut needs) in query.iter_mut() {
        if !traits.has(Trait::Photosynthesis) {
            continue;
        }
        let (Ok(x), Ok(y)) = (u32::try_from(pos.x), u32::try_from(pos.y)) else {
            continue;
        };
        let total_light = light_map.get(x, y).max(ambient_light.level);
        if total_light > PHOTOSYNTHESIS_LIGHT_THRESHOLD {
            needs.hunger = (needs.hunger + PHOTOSYNTHESIS_NOURISHMENT_RATE).min(1.0);
        }
    }
}

/// Tag component to ensure pops don't mutate infinitely during a single storm.
#[derive(Component, Default)]
pub struct Mutated;

/// Applies random mutant traits to pops exposed to mutagenic rain.
type MutagenicRainQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GridPosition,
        &'static mut Traits,
        Option<&'static PopName>,
    ),
    (With<Pop>, Without<Mutated>),
>;
pub fn apply_mutagenic_rain_system(
    mut commands: Commands,
    weather: Res<WeatherState>,
    grid: Res<RoofGrid>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut query: MutagenicRainQuery<'_, '_>,
) {
    if weather.current_weather != WeatherType::MutagenicRain {
        return;
    }

    let mut rng = rand::thread_rng();

    for (entity, pos, mut traits, name) in query.iter_mut() {
        let is_under_roof = grid.has_roof(pos.x, pos.y);

        if !is_under_roof {
            // High chance to mutate (e.g., 5% per tick for testing, but in reality maybe 0.1%)
            if rng.gen_bool(0.05) {
                let new_trait = MUTANT_TRAITS[rng.gen_range(0..MUTANT_TRAITS.len())];

                if !traits.has(new_trait) {
                    traits.add(new_trait);
                    // Add Mutated tag to prevent infinite mutations in one storm
                    commands.entity(entity).insert(Mutated);
                    // Spec 427 Leg 3: every mutation is chronicle-visible.
                    // The Mutated once-per-storm cap bounds this to at
                    // most one entry per pop per storm.
                    let who = name.map_or("A colonist", |n| n.0.as_str());
                    chronicle.send(AddChronicleEvent {
                        text: format!(
                            "{who} mutated in the mutagenic rain, gaining {}.",
                            new_trait.label()
                        ),
                        importance: EventImportance::Minor,
                    });
                }
            }
        }
    }
}

/// Removes the Mutated tag when the weather clears, allowing mutation in the next storm.
pub fn clear_mutation_immunity_system(
    mut commands: Commands,
    weather: Res<WeatherState>,
    query: Query<Entity, With<Mutated>>,
) {
    if weather.current_weather != WeatherType::MutagenicRain {
        for entity in query.iter() {
            commands.entity(entity).remove::<Mutated>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::nature::weather::{WeatherState, WeatherType};
    use crate::layer1::pop::Pop;
    use crate::layer1::structural_integrity::RoofGrid;
    use crate::layer1::traits::{Trait, Traits};
    use bevy_app::{App, Update};
    use bevy_ecs::system::RunSystemOnce;

    fn setup_app() -> App {
        let mut app = App::new();

        let mut grid = RoofGrid::new(10, 10);
        // Add a roof at (5, 5)
        grid.has_roof[5 * 10 + 5] = true;
        app.insert_resource(grid);

        app.init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>();
        app.insert_resource(WeatherState {
            current_weather: WeatherType::MutagenicRain,
            duration_remaining: 100,
        });

        app.add_systems(Update, apply_mutagenic_rain_system);

        app
    }

    #[test]
    fn test_indoor_pop_not_mutated() {
        let mut app = setup_app();

        let safe_pop = app
            .world_mut()
            .spawn((
                GridPosition { x: 5, y: 5 }, // Under roof
                Pop,
                Traits::default(),
            ))
            .id();

        // Run system enough times to statistically guarantee mutation if not protected
        for _ in 0..100 {
            app.update();
        }

        let traits = app.world().get::<Traits>(safe_pop).unwrap();
        assert!(traits.0.is_empty(), "Pop under roof should not mutate.");
    }

    #[test]
    fn test_outdoor_pop_mutates() {
        let mut app = setup_app();

        let exposed_pop = app
            .world_mut()
            .spawn((
                GridPosition { x: 2, y: 2 }, // No roof above
                Pop,
                Traits::default(),
            ))
            .id();

        // Run system enough times to statistically guarantee mutation (1000 iterations to avoid flakiness)
        for _ in 0..1000 {
            app.update();
        }

        let traits = app.world().get::<Traits>(exposed_pop).unwrap();
        assert!(
            !traits.0.is_empty(),
            "Pop outside during mutagenic rain should mutate."
        );

        // Ensure trait is a mutant trait
        let has_mutant_trait = traits.has(Trait::Photosynthesis)
            || traits.has(Trait::ThickSkin)
            || traits.has(Trait::BrittleBones)
            || traits.has(Trait::ExtremeHunger);
        assert!(has_mutant_trait, "Gained trait must be a mutant variant.");

        // Ensure they got tagged so they don't mutate infinitely
        assert!(
            app.world().get::<Mutated>(exposed_pop).is_some(),
            "Pop should be tagged as Mutated"
        );
    }

    #[test]
    fn test_mutation_writes_chronicle_entry() {
        // Spec 427 Leg 3: a mutation must be visible in the chronicle.
        let mut app = setup_app();
        app.init_resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>();
        app.world_mut().spawn((
            GridPosition { x: 2, y: 2 },
            Pop,
            Traits::default(),
            crate::layer1::pop::PopName("Test Subject".to_string()),
        ));
        for _ in 0..1000 {
            app.update();
        }
        let events = app
            .world()
            .resource::<bevy_ecs::event::Events<crate::layer1::chronicle::AddChronicleEvent>>();
        let texts: Vec<String> = events
            .get_cursor()
            .read(events)
            .map(|e| e.text.clone())
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("mutated")),
            "expected a chronicle entry about the mutation, got: {texts:?}"
        );
    }

    #[test]
    fn test_photosynthesis_nourishes_in_light() {
        // Spec 427 Leg 2: Photosynthesis pops regen hunger in bright light.
        let mut world = World::new();
        let mut map = crate::layer1::lighting::LightMap::new(10, 10);
        map.set(5, 5, 1.0);
        world.insert_resource(map);
        world.insert_resource(crate::layer1::lighting::AmbientLight { level: 0.0 });
        let mut traits = Traits::default();
        traits.add(Trait::Photosynthesis);
        let pop = world
            .spawn((
                Pop,
                traits,
                GridPosition { x: 5, y: 5 },
                crate::layer1::needs::Needs {
                    hunger: 0.5,
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(photosynthesis_nourishment_system)
            .unwrap();
        let needs = world.get::<crate::layer1::needs::Needs>(pop).unwrap();
        assert!(
            needs.hunger > 0.5,
            "Photosynthetic pop should gain hunger in light, got {}",
            needs.hunger
        );
    }

    #[test]
    fn test_photosynthesis_does_not_nourish_in_darkness() {
        let mut world = World::new();
        world.insert_resource(crate::layer1::lighting::LightMap::new(10, 10));
        world.insert_resource(crate::layer1::lighting::AmbientLight { level: 0.0 });
        let mut traits = Traits::default();
        traits.add(Trait::Photosynthesis);
        let pop = world
            .spawn((
                Pop,
                traits,
                GridPosition { x: 5, y: 5 },
                crate::layer1::needs::Needs {
                    hunger: 0.5,
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
            ))
            .id();
        world
            .run_system_once(photosynthesis_nourishment_system)
            .unwrap();
        let needs = world.get::<crate::layer1::needs::Needs>(pop).unwrap();
        assert!(
            (needs.hunger - 0.5).abs() < f32::EPSILON,
            "Photosynthetic pop should NOT gain hunger in darkness"
        );
    }

    #[test]
    fn test_mutated_tag_prevents_multiple_mutations() {
        let mut app = setup_app();

        let exposed_pop = app
            .world_mut()
            .spawn((
                GridPosition { x: 2, y: 2 }, // No roof above
                Pop,
                Traits::default(),
                Mutated, // Already mutated
            ))
            .id();

        for _ in 0..100 {
            app.update();
        }

        let traits = app.world().get::<Traits>(exposed_pop).unwrap();
        assert!(
            traits.0.is_empty(),
            "Pop with Mutated tag should not gain further traits."
        );
    }
}
