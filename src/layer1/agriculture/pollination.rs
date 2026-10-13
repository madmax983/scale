use crate::layer1::map::GridPosition;
use crate::layer1::seasons::SeasonState;
use bevy::prelude::*;

#[derive(Component)]
pub struct FarmCrop {
    pub growth_stage: FarmGrowthStage,
    pub requires_pollination: bool,
    /// Progress (0.0–1.0+) toward the next growth stage (Spec 344).
    /// Each `GrowthCycleEvent` adds the current season's growth
    /// modifier; a stage advances once progress reaches 1.0.
    pub growth_progress: f32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FarmGrowthStage {
    Seedling,
    Flowering,
    Harvestable,
    Failed,
}

#[derive(Component)]
pub struct PollinationStatus {
    pub is_pollinated: bool,
}

#[derive(Component)]
pub struct PollinatorDrone {
    pub range: i32,
}

#[derive(Event)]
pub struct GrowthCycleEvent;

pub fn drone_pollination_system(
    drones: Query<(&PollinatorDrone, &GridPosition)>,
    mut crops: Query<(&mut PollinationStatus, &GridPosition, &FarmCrop)>,
) {
    for (drone, drone_pos) in drones.iter() {
        for (mut status, crop_pos, crop) in crops.iter_mut() {
            if crop.growth_stage == FarmGrowthStage::Flowering && !status.is_pollinated {
                let dx = drone_pos.x as f32 - crop_pos.x as f32;
                let dy = drone_pos.y as f32 - crop_pos.y as f32;
                if (dx.abs() + dy.abs()) as i32 <= drone.range {
                    status.is_pollinated = true;
                }
            }
        }
    }
}

pub fn process_crop_growth_system(
    mut events: EventReader<GrowthCycleEvent>,
    season: Option<Res<SeasonState>>,
    mut crops: Query<(&mut FarmCrop, &PollinationStatus)>,
) {
    // Spec 344: seasons scale crop growth rates. No SeasonState (unit
    // tests, minimal worlds) means a neutral 1.0 rate.
    let growth_rate = season.map_or(1.0, |s| s.current_season.growth_modifier());
    for _ in events.read() {
        for (mut crop, status) in crops.iter_mut() {
            if crop.growth_stage == FarmGrowthStage::Harvestable
                || crop.growth_stage == FarmGrowthStage::Failed
            {
                continue;
            }
            crop.growth_progress += growth_rate;
            // Epsilon guards float accumulation (10 x 0.1 in winter).
            if crop.growth_progress < 0.999 {
                continue;
            }
            crop.growth_progress -= 1.0;
            match crop.growth_stage {
                FarmGrowthStage::Seedling => {
                    crop.growth_stage = FarmGrowthStage::Flowering;
                }
                FarmGrowthStage::Flowering => {
                    if crop.requires_pollination && !status.is_pollinated {
                        crop.growth_stage = FarmGrowthStage::Failed;
                    } else {
                        crop.growth_stage = FarmGrowthStage::Harvestable;
                    }
                }
                FarmGrowthStage::Harvestable | FarmGrowthStage::Failed => {}
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::seasons::Season;

    #[test]
    fn test_crop_fails_without_pollination() {
        // Arrange
        let mut app = App::new();
        app.add_systems(Update, process_crop_growth_system);
        app.add_event::<GrowthCycleEvent>();

        let crop = app
            .world_mut()
            .spawn((
                FarmCrop {
                    growth_stage: FarmGrowthStage::Flowering,
                    requires_pollination: true,
                    growth_progress: 0.0,
                },
                PollinationStatus {
                    is_pollinated: false,
                },
            ))
            .id();

        // Act - Simulate a growth cycle
        app.world_mut().send_event(GrowthCycleEvent);
        app.update();

        // Assert - Crop fails instead of progressing to Harvestable
        let crop_comp = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(crop_comp.growth_stage, FarmGrowthStage::Failed);
    }

    #[test]
    fn test_drone_pollination_overflow() {
        let mut app = App::new();
        app.add_systems(Update, drone_pollination_system);

        app.world_mut().spawn((
            FarmCrop {
                growth_stage: FarmGrowthStage::Flowering,
                requires_pollination: true,
                growth_progress: 0.0,
            },
            PollinationStatus {
                is_pollinated: false,
            },
            GridPosition { x: i32::MIN, y: 0 },
        ));

        app.world_mut().spawn((
            PollinatorDrone { range: 5 },
            GridPosition { x: i32::MAX, y: 0 },
        ));

        // This would panic before the fix
        app.update();
    }

    // --- Spec 344: seasonal crop growth rates ---

    fn seasonal_app(season: Season) -> App {
        let mut app = App::new();
        app.insert_resource(SeasonState {
            current_season: season,
        });
        app.add_systems(Update, process_crop_growth_system);
        app.add_event::<GrowthCycleEvent>();
        app
    }

    fn spawn_crop(app: &mut App, stage: FarmGrowthStage) -> Entity {
        app.world_mut()
            .spawn((
                FarmCrop {
                    growth_stage: stage,
                    requires_pollination: false,
                    growth_progress: 0.0,
                },
                PollinationStatus {
                    is_pollinated: true,
                },
            ))
            .id()
    }

    fn growth_cycle(app: &mut App) {
        app.world_mut().send_event(GrowthCycleEvent);
        app.update();
    }

    #[test]
    fn test_winter_slows_crop_growth_rate() {
        // Spec 344 RED test, adapted: winter growth is 0.1x, so a single
        // growth cycle must NOT advance a flowering crop.
        let mut app = seasonal_app(Season::Winter);
        let crop = spawn_crop(&mut app, FarmGrowthStage::Flowering);

        growth_cycle(&mut app);
        let c = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(c.growth_stage, FarmGrowthStage::Flowering);
        assert!((c.growth_progress - 0.1).abs() < 0.001);

        // Nine more cycles complete the stage (10 x 0.1 = 1.0).
        for _ in 0..9 {
            growth_cycle(&mut app);
        }
        let c = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(c.growth_stage, FarmGrowthStage::Harvestable);
    }

    #[test]
    fn test_summer_seedling_advances_and_carries_progress() {
        let mut app = seasonal_app(Season::Summer);
        let crop = spawn_crop(&mut app, FarmGrowthStage::Seedling);

        // 1.5x growth: one cycle advances Seedling -> Flowering, carrying 0.5.
        growth_cycle(&mut app);
        let c = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(c.growth_stage, FarmGrowthStage::Flowering);
        assert!((c.growth_progress - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_spring_seedling_advances_to_flowering() {
        let mut app = seasonal_app(Season::Spring);
        let crop = spawn_crop(&mut app, FarmGrowthStage::Seedling);

        growth_cycle(&mut app);
        let c = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(c.growth_stage, FarmGrowthStage::Flowering);
    }

    #[test]
    fn test_pollinator_drone_pollinates_nearby_crops() {
        // Arrange
        let mut app = App::new();
        app.add_systems(
            Update,
            (drone_pollination_system, process_crop_growth_system).chain(),
        );
        app.add_event::<GrowthCycleEvent>();

        let crop = app
            .world_mut()
            .spawn((
                FarmCrop {
                    growth_stage: FarmGrowthStage::Flowering,
                    requires_pollination: true,
                    growth_progress: 0.0,
                },
                PollinationStatus {
                    is_pollinated: false,
                },
                GridPosition { x: 10, y: 10 },
            ))
            .id();

        let _drone = app
            .world_mut()
            .spawn((
                PollinatorDrone { range: 5 },
                GridPosition { x: 12, y: 12 }, // Within range
            ))
            .id();

        // Act - Drone pollinates, then growth cycle happens
        app.update();
        app.world_mut().send_event(GrowthCycleEvent);
        app.update();

        // Assert - Crop successfully progresses to Harvestable
        let status = app.world().get::<PollinationStatus>(crop).unwrap();
        assert!(status.is_pollinated);

        let crop_comp = app.world().get::<FarmCrop>(crop).unwrap();
        assert_eq!(crop_comp.growth_stage, FarmGrowthStage::Harvestable);
    }
}
