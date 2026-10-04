use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::psychology::void_stare::VoidExposure;
use crate::layer2::ship::OffWorldDuty;
use bevy::prelude::Time;
use bevy_ecs::prelude::*;

// Define PopStats as it's missing in the main codebase
#[derive(Component, Default, Debug, Clone, PartialEq)]
pub struct PopStats {
    pub intellect: f32,
    pub empathy: f32,
    pub perception: f32,
}

pub fn process_void_exposure_system(
    mut query: Query<&mut VoidExposure, With<OffWorldDuty>>,
    time: Res<Time>,
) {
    for mut exposure in query.iter_mut() {
        exposure.current += time.delta_secs() * 0.1; // Base rate
    }
}

pub fn apply_void_touched_trait_system(mut query: Query<(&VoidExposure, &mut Traits)>) {
    for (exposure, mut traits) in query.iter_mut() {
        if exposure.current >= 100.0 && !traits.has(Trait::VoidTouched) {
            traits.add(Trait::VoidTouched);
        }
    }
}

/// Marker component to indicate that VoidTouched stat modifiers have been applied.
#[derive(Component)]
pub struct VoidTouchedStatsApplied;

#[allow(clippy::type_complexity)]
pub fn apply_trait_stat_modifiers_system(
    mut commands: Commands,
    mut query: Query<
        (Entity, &Traits, &mut PopStats),
        (Changed<Traits>, Without<VoidTouchedStatsApplied>),
    >,
) {
    for (entity, traits, mut stats) in query.iter_mut() {
        if traits.has(Trait::VoidTouched) {
            stats.intellect += 1.0;
            stats.perception += 1.0;
            stats.empathy -= 1.0;
            commands.entity(entity).insert(VoidTouchedStatsApplied);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::mind::evaluate_single_pop;
    use crate::layer1::mind::utility_eval_types::{
        PopEvalData, ScorableCandidate, UtilityAIBuffer, WorldContext,
    };
    use crate::layer1::mind::utility_types::ActionType;
    use crate::layer1::pop::Pop;
    use crate::layer1::psychology::needs::Needs;
    use bevy::prelude::Update;

    #[test]
    fn test_void_exposure_accumulation() {
        let mut app = bevy_app::App::new();
        app.add_systems(Update, process_void_exposure_system);

        // Add required resource
        app.init_resource::<Time>();
        // Initialize the time explicitly to have delta time
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(1.0));

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                VoidExposure {
                    current: 0.0,
                    susceptibility: 1.0,
                    check_timer: 0,
                },
                OffWorldDuty,
            ))
            .id();

        app.update();

        let exposure = app.world().get::<VoidExposure>(pop).unwrap();
        assert!(
            exposure.current > 0.0,
            "Exposure should increase when on off-world duty"
        );
    }

    #[test]
    fn test_void_touched_trait_application() {
        let mut app = bevy_app::App::new();
        app.add_systems(Update, apply_void_touched_trait_system);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Traits::default(),
                VoidExposure {
                    current: 100.0,
                    susceptibility: 1.0,
                    check_timer: 0,
                }, // High exposure
            ))
            .id();

        app.update();

        let traits = app.world().get::<Traits>(pop).unwrap();
        assert!(
            traits.has(Trait::VoidTouched),
            "High exposure should apply the VoidTouched trait"
        );
    }

    #[test]
    fn test_void_touched_stat_modifiers() {
        let mut app = bevy_app::App::new();
        app.add_systems(Update, apply_trait_stat_modifiers_system);

        let mut base_traits = Traits::default();
        base_traits.add(Trait::VoidTouched);

        let pop = app
            .world_mut()
            .spawn((Pop, base_traits, PopStats::default()))
            .id();

        app.update();

        let stats = app.world().get::<PopStats>(pop).unwrap();
        assert!(
            stats.intellect > PopStats::default().intellect,
            "VoidTouched should boost Intellect"
        );
        assert!(
            stats.empathy < PopStats::default().empathy,
            "VoidTouched should penalize Empathy"
        );
    }

    #[test]
    fn test_void_touched_refuses_surface_sleep() {
        let mut app = bevy_app::App::new();

        let mut traits = Traits::default();
        traits.add(Trait::VoidTouched);

        let needs = Needs {
            rest: 0.1,
            ..Default::default()
        };

        let mut data = PopEvalData::test_instance();
        data.needs = needs;
        data.traits = Some(traits);

        let mut buffer = UtilityAIBuffer::default();
        // Add a house to sleep in
        let house_entity = app.world_mut().spawn_empty().id();
        buffer.housing.push(ScorableCandidate {
            entity: house_entity,
            pos: GridPosition { x: 0, y: 0 },
            capacity: 1,
            usage: 0,
            score_bonus: 0.0,
            resource_type: None,
            item_type: None,
            is_advanced_tech: false,
        });

        // Initialize world context
        app.insert_resource(crate::layer1::resources::ColonyResources::default());
        app.insert_resource(crate::layer1::day_night::DayNightCycle::default());
        app.insert_resource(crate::layer1::taboo::TabooState::default());
        app.insert_resource(crate::layer1::zone::ZoneGrid::new(10, 10));

        let world_ctx = WorldContext {
            resources: app
                .world()
                .resource::<crate::layer1::resources::ColonyResources>(),
            cycle: app
                .world()
                .resource::<crate::layer1::day_night::DayNightCycle>(),
            taboo: app.world().resource::<crate::layer1::taboo::TabooState>(),
            factions: None,
            zone_grid: app.world().resource::<crate::layer1::zone::ZoneGrid>(),
            temperature_grid: None,
            work_fervor: 0.0,
            quota_fervor: 0.0,
        };

        let (action, utility, _) = evaluate_single_pop(&buffer, &data, &world_ctx);

        // Even though tired, shouldn't choose to SatisfyRest (or if it does, utility should be very low)
        if action == ActionType::SatisfyRest {
            assert!(
                utility < 0.2,
                "VoidTouched Pop on surface should refuse/heavily penalize sleep"
            );
        }
    }
}
