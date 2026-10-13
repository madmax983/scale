use crate::layer1::balance::{
    SEASON_GROWTH_MODIFIER_AUTUMN, SEASON_GROWTH_MODIFIER_SPRING, SEASON_GROWTH_MODIFIER_SUMMER,
    SEASON_GROWTH_MODIFIER_WINTER, SEASON_MODIFIER_AUTUMN, SEASON_MODIFIER_SPRING,
    SEASON_MODIFIER_SUMMER, SEASON_MODIFIER_WINTER, SEASON_MOVE_MODIFIER_AUTUMN,
    SEASON_MOVE_MODIFIER_SPRING, SEASON_MOVE_MODIFIER_SUMMER, SEASON_MOVE_MODIFIER_WINTER,
    TICKS_PER_YEAR,
};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// The four seasons of the year, influencing gameplay mechanics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Season {
    /// The season of rebirth and growth.
    #[default]
    Spring,
    /// The season of warmth and abundance.
    Summer,
    /// The season of harvest and preparation.
    Autumn,
    /// The season of cold and dormancy.
    Winter,
}

impl Season {
    /// Returns the next season in the cycle.
    #[must_use]
    pub const fn next(&self) -> Self {
        match self {
            Self::Spring => Self::Summer,
            Self::Summer => Self::Autumn,
            Self::Autumn => Self::Winter,
            Self::Winter => Self::Spring,
        }
    }

    /// Returns the food production modifier for this season.
    #[must_use]
    pub const fn food_modifier(&self) -> f32 {
        match self {
            Self::Spring => SEASON_MODIFIER_SPRING,
            Self::Summer => SEASON_MODIFIER_SUMMER,
            Self::Autumn => SEASON_MODIFIER_AUTUMN,
            Self::Winter => SEASON_MODIFIER_WINTER,
        }
    }

    /// Returns the display name of the season.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Spring => "Spring",
            Self::Summer => "Summer",
            Self::Autumn => "Autumn",
            Self::Winter => "Winter",
        }
    }

    /// Returns the base ambient temperature for this season (Celsius).
    #[must_use]
    pub const fn base_temperature(&self) -> f32 {
        match self {
            Self::Spring => 15.0,
            Self::Summer => 30.0,
            Self::Autumn => 10.0,
            Self::Winter => -5.0,
        }
    }

    /// Movement speed multiplier for this season (Spec 344): winter snow
    /// (0.6) and spring mud (0.85) slow pops outdoors; summer is full speed.
    #[must_use]
    pub const fn movement_modifier(&self) -> f32 {
        match self {
            Self::Spring => SEASON_MOVE_MODIFIER_SPRING,
            Self::Summer => SEASON_MOVE_MODIFIER_SUMMER,
            Self::Autumn => SEASON_MOVE_MODIFIER_AUTUMN,
            Self::Winter => SEASON_MOVE_MODIFIER_WINTER,
        }
    }

    /// Crop growth-rate multiplier for this season (Spec 344): growth
    /// nearly stops in winter (0.1) and peaks in summer (1.5). Distinct
    /// from `food_modifier`, which scales harvested yield in `farm.rs`.
    #[must_use]
    pub const fn growth_modifier(&self) -> f32 {
        match self {
            Self::Spring => SEASON_GROWTH_MODIFIER_SPRING,
            Self::Summer => SEASON_GROWTH_MODIFIER_SUMMER,
            Self::Autumn => SEASON_GROWTH_MODIFIER_AUTUMN,
            Self::Winter => SEASON_GROWTH_MODIFIER_WINTER,
        }
    }

    /// Short label for the season's ground condition (Spec 344).
    #[must_use]
    pub const fn ground_condition(&self) -> &'static str {
        match self {
            Self::Spring => "Mud",
            Self::Summer => "Dry ground",
            Self::Autumn => "Firm ground",
            Self::Winter => "Snow",
        }
    }
}

/// Ticks in one season (a quarter of the year).
pub const TICKS_PER_SEASON: u64 = TICKS_PER_YEAR / 4;

/// Hook emitted when the season changes (Spec 344 §7), alongside the
/// chronicle entry. Other systems (UI, events) can react to this.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonTransitionEvent {
    /// The season that just began.
    pub season: Season,
    /// Simulation tick at which the transition happened.
    pub tick: u64,
}

/// The season for a world, defaulting to Spring when no state exists.
#[must_use]
pub fn current_season(world: &World) -> Season {
    world
        .get_resource::<SeasonState>()
        .map_or(Season::Spring, |s| s.current_season)
}

/// One-line season summary for the headless console (Spec 344).
#[must_use]
pub fn season_status_label(world: &World) -> String {
    let season = current_season(world);
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);
    let into = tick % TICKS_PER_SEASON;
    format!(
        "{} (tick {} of {}), next: {}, ambient {:.0}C, food x{:.2}, growth x{:.2}, movement x{:.2} ({})",
        season.name(),
        into,
        TICKS_PER_SEASON,
        season.next().name(),
        season.base_temperature(),
        season.food_modifier(),
        season.growth_modifier(),
        season.movement_modifier(),
        season.ground_condition(),
    )
}

/// Tracks the current season of the colony.
#[derive(Resource, Default)]
pub struct SeasonState {
    /// The current active season.
    pub current_season: Season,
}

/// Updates the current season based on the simulation tick.
///
/// This system calculates the season by dividing the current tick by `TICKS_PER_YEAR / 4`.
/// It updates `SeasonState` and adds a `Chronicle` event when the season changes.
pub fn advance_season_system(
    time: Res<SimulationTime>,
    mut state: ResMut<SeasonState>,
    mut events: EventWriter<AddChronicleEvent>,
    mut transitions: EventWriter<SeasonTransitionEvent>,
) {
    let tick = time.tick;
    let ticks_per_season = TICKS_PER_YEAR / 4;

    let season_index = (tick / ticks_per_season) % 4;
    let new_season = match season_index {
        0 => Season::Spring,
        1 => Season::Summer,
        2 => Season::Autumn,
        _ => Season::Winter,
    };

    if state.current_season != new_season {
        state.current_season = new_season;
        events.send(AddChronicleEvent {
            text: format!("The season turns. {} has arrived.", new_season.name()),
            importance: EventImportance::Standard,
        });
        transitions.send(SeasonTransitionEvent {
            season: new_season,
            tick,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::time::SimulationTime;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_season_enum_cycling() {
        assert_eq!(Season::Spring.next(), Season::Summer);
        assert_eq!(Season::Summer.next(), Season::Autumn);
        assert_eq!(Season::Autumn.next(), Season::Winter);
        assert_eq!(Season::Winter.next(), Season::Spring);
    }

    #[test]
    fn test_season_state_default() {
        let state = SeasonState::default();
        assert_eq!(state.current_season, Season::Spring);
    }

    #[test]
    fn test_advance_season_system_initial() {
        let mut world = World::new();
        world.insert_resource(SeasonState::default());
        world.insert_resource(SimulationTime {
            tick: 0,
            ..Default::default()
        });
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<SeasonTransitionEvent>>();

        world.run_system_once(advance_season_system).unwrap();

        let state = world.resource::<SeasonState>();
        assert_eq!(state.current_season, Season::Spring);
    }

    #[test]
    fn test_advance_season_system_transition() {
        let mut world = World::new();
        world.insert_resource(SeasonState::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<SeasonTransitionEvent>>();

        // Ticks per season = 1000 / 4 = 250
        // Spring: 0-249, Summer: 250-499
        world.insert_resource(SimulationTime {
            tick: 250,
            ..Default::default()
        });

        world.run_system_once(advance_season_system).unwrap();

        let state = world.resource::<SeasonState>();
        assert_eq!(state.current_season, Season::Summer);

        // Verify Event was sent
        let events = world.resource::<Events<AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();
        assert_eq!(emitted.len(), 1);
        assert!(emitted[0].text.contains("Summer"));
    }

    #[test]
    fn test_advance_season_system_no_spam() {
        let mut world = World::new();
        world.insert_resource(SeasonState {
            current_season: Season::Summer,
        });
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<SeasonTransitionEvent>>();
        world.insert_resource(SimulationTime {
            tick: 251,
            ..Default::default()
        });

        world.run_system_once(advance_season_system).unwrap();

        // Verify NO Event was sent
        let events = world.resource::<Events<AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();
        assert!(
            emitted.is_empty(),
            "Should not add event if season hasn't changed"
        );
    }

    #[test]
    fn test_get_food_modifier() {
        assert!((Season::Spring.food_modifier() - 1.0).abs() < f32::EPSILON);
        assert!((Season::Summer.food_modifier() - 1.2).abs() < f32::EPSILON);
        assert!((Season::Autumn.food_modifier() - 1.5).abs() < f32::EPSILON);
        assert!((Season::Winter.food_modifier() - 0.5).abs() < f32::EPSILON);
    }

    // --- Spec 344 RED tests (adapted to the real Season/SeasonState types) ---

    #[test]
    fn test_season_movement_modifiers_snow_and_mud() {
        // Winter snow is the slowest going, Spring mud drags, Summer is full speed.
        assert!((Season::Winter.movement_modifier() - 0.6).abs() < f32::EPSILON);
        assert!((Season::Spring.movement_modifier() - 0.85).abs() < f32::EPSILON);
        assert!((Season::Autumn.movement_modifier() - 0.95).abs() < f32::EPSILON);
        assert!((Season::Summer.movement_modifier() - 1.0).abs() < f32::EPSILON);
        assert!(Season::Winter.movement_modifier() < Season::Spring.movement_modifier());
        assert!(Season::Spring.movement_modifier() < Season::Summer.movement_modifier());
    }

    #[test]
    fn test_season_growth_modifiers() {
        // Spec 344: winter nearly stops growth (0.1x), summer peaks (1.5x).
        assert!((Season::Winter.growth_modifier() - 0.1).abs() < f32::EPSILON);
        assert!((Season::Summer.growth_modifier() - 1.5).abs() < f32::EPSILON);
        assert!(Season::Winter.growth_modifier() < Season::Spring.growth_modifier());
        assert!(Season::Spring.growth_modifier() < Season::Summer.growth_modifier());
        assert!(Season::Autumn.growth_modifier() < Season::Summer.growth_modifier());
    }

    #[test]
    fn test_advance_season_emits_transition_event() {
        let mut world = World::new();
        world.insert_resource(SeasonState::default());
        world.init_resource::<Events<AddChronicleEvent>>();
        world.init_resource::<Events<SeasonTransitionEvent>>();
        world.insert_resource(SimulationTime {
            tick: 250,
            ..Default::default()
        });

        world.run_system_once(advance_season_system).unwrap();

        let events = world.resource::<Events<SeasonTransitionEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].season, Season::Summer);
        assert_eq!(emitted[0].tick, 250);
    }

    #[test]
    fn test_season_public_strings_are_original() {
        // IP guard (Spec 344): season names / ground-condition labels are
        // plain generic words, no borrowed IP.
        for season in [
            Season::Spring,
            Season::Summer,
            Season::Autumn,
            Season::Winter,
        ] {
            let text = format!("{} {}", season.name(), season.ground_condition());
            for banned in ["SeasonTracker", "CropGrowth"] {
                assert!(!text.contains(banned), "leaked mock type name: {banned}");
            }
        }
        assert_eq!(Season::Winter.ground_condition(), "Snow");
        assert_eq!(Season::Spring.ground_condition(), "Mud");
    }

    #[test]
    fn test_base_temperature() {
        assert!((Season::Spring.base_temperature() - 15.0).abs() < f32::EPSILON);
        assert!((Season::Summer.base_temperature() - 30.0).abs() < f32::EPSILON);
        assert!((Season::Autumn.base_temperature() - 10.0).abs() < f32::EPSILON);
        assert!((Season::Winter.base_temperature() + 5.0).abs() < f32::EPSILON);
    }
}
