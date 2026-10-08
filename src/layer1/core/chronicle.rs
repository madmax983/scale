//! Chronicle system and historical records.
//!
//! The Chronicle is the "Memory" of the colony. It does not just log debug messages;
//! it records significant events that form the narrative of the player's playthrough.
//! This system bridges the gap between mechanical simulation (ticks, resources) and
//! player experience (stories, history).
//!
//! # Concepts
//!
//! * **`ChronicleEvent`**: An atomic piece of history (e.g., "Colony Founded", "First Winter").
//! * **`EventImportance`**: Determines how prominent the event is in the UI.
//! * **`Milestones`**: Automatic achievements tracked by the `check_milestones_system`.
//!
//! # Integration with Lore
//!
//! While the Chronicle stores *what* happened, the descriptions often come from the
//! narrative generator (see `src/shared/narrative.rs`). This separation allows for
//! flavor text to vary while the underlying event data remains consistent.

use crate::layer1::balance::TICKS_PER_YEAR;
use crate::layer1::building::{Building, BuildingType};
use crate::shared::colony::ColonyName;
use crate::shared::narrative::{NarrativeContext, NarrativeGenerator};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Importance level for chronicle events.
///
/// This enum dictates visual hierarchy in the UI (colors, prefixes).
///
/// # Examples
///
/// ```
/// use scale::layer1::chronicle::EventImportance;
///
/// let level = EventImportance::Legendary;
/// assert!(matches!(level, EventImportance::Legendary));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventImportance {
    /// Flavor text or minor notifications (e.g., "Bob ate a berry").
    Minor,
    /// Standard gameplay events (e.g., "Housing completed").
    Standard,
    /// Significant achievements or milestones (e.g., "Iron Age reached").
    Major,
    /// World-altering events or game start (e.g., "Colony Founded").
    Legendary,
}

/// Event triggered when a new chronicle entry should be added.
#[derive(Event, Debug, Clone)]
pub struct AddChronicleEvent {
    /// The text description of the event.
    pub text: String,
    /// The importance level of the event.
    pub importance: EventImportance,
}

/// System to process chronicle events and add them to the resource.
pub fn chronicle_event_handler_system(
    mut events: EventReader<AddChronicleEvent>,
    mut chronicle: ResMut<Chronicle>,
    time: Res<SimulationTime>,
) {
    for event in events.read() {
        chronicle.add_event(time.tick, event.text.clone(), event.importance);
    }
}

/// A single chronicle event.
#[derive(Clone, Debug)]
pub struct ChronicleEvent {
    /// The simulation tick when the event occurred.
    pub tick: u64,
    /// The year when the event occurred (derived from tick).
    pub year: u32,
    /// The event description.
    pub text: String,
    /// The importance level of the event.
    pub importance: EventImportance,
}

/// Chronicle resource - stores the colony's historical record.
pub const MAX_CHRONICLE_EVENTS: usize = 1000;

#[derive(Resource, Default)]
pub struct Chronicle {
    /// List of events in the chronicle.
    pub events: Vec<ChronicleEvent>,
}

impl Chronicle {
    /// Add a new event to the chronicle.
    ///
    /// The event is automatically timestamped with the current "Year" based on the tick.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::chronicle::{Chronicle, EventImportance};
    ///
    /// let mut chronicle = Chronicle::default();
    /// chronicle.add_event(100, "We built a fire.".to_string(), EventImportance::Standard);
    /// assert_eq!(chronicle.events.len(), 1);
    /// ```
    pub fn add_event(&mut self, tick: u64, text: String, importance: EventImportance) {
        self.events.push(ChronicleEvent {
            tick,
            year: 1 + u32::try_from(tick / TICKS_PER_YEAR).unwrap_or(u32::MAX), // Rough "year" approximation
            text,
            importance,
        });

        if self.events.len() > MAX_CHRONICLE_EVENTS {
            self.events.remove(0);
        }
    }

    /// Add a pre-history event (year 0, tick 0) to the chronicle.
    ///
    /// These represent events that occurred before the colony was founded,
    /// generated during world history creation.
    pub fn add_prehistory_event(&mut self, text: String, importance: EventImportance) {
        self.events.push(ChronicleEvent {
            tick: 0,
            year: 0,
            text,
            importance,
        });

        if self.events.len() > MAX_CHRONICLE_EVENTS {
            self.events.remove(0);
        }
    }
}

/// UI state for chronicle window.
#[derive(Resource, Default)]
pub struct ChronicleUiState {
    /// Whether the chronicle window is currently open.
    pub is_open: bool,
}

/// Tracks which building types have been built (for milestones).
#[derive(Resource, Default)]
pub struct BuildingTracker {
    /// Whether housing has been built at least once.
    pub has_built_housing: bool,
    /// Whether a farm has been built at least once.
    pub has_built_farm: bool,
}

/// Creates the initial "colony founded" event.
pub fn initial_chronicle_event(world: &mut World) {
    let text = {
        let generator = world.resource::<NarrativeGenerator>();
        let colony = world.resource::<ColonyName>();
        let mut ctx = NarrativeContext::new();
        ctx.insert("COLONY_NAME", &colony.name);
        ctx.insert("YEAR", "1");
        ctx.insert("FOUNDER_COUNT", "5");
        generator
            .generate("COLONY_FOUNDED", &ctx)
            .unwrap_or_else(|_| "Colony founded. The journey begins.".to_string())
    };
    world
        .resource_mut::<Chronicle>()
        .add_event(0, text, EventImportance::Legendary);
}

/// Checks for building milestones and records them in the chronicle.
pub fn check_milestones_system(
    time: Res<SimulationTime>,
    mut tracker: ResMut<BuildingTracker>,
    mut events: EventWriter<AddChronicleEvent>,
    buildings: Query<&Building>,
    generator: Res<NarrativeGenerator>,
    colony: Res<ColonyName>,
) {
    if tracker.has_built_housing && tracker.has_built_farm {
        return;
    }

    let current_tick = time.tick;
    let need_housing = !tracker.has_built_housing;
    let need_farm = !tracker.has_built_farm;

    let mut found_housing = false;
    let mut found_farm = false;

    for building in &buildings {
        if need_housing && building.building_type == BuildingType::Housing {
            found_housing = true;
        }
        if need_farm && building.building_type == BuildingType::Farm {
            found_farm = true;
        }
        if (found_housing || !need_housing) && (found_farm || !need_farm) {
            break;
        }
    }

    if found_housing && !tracker.has_built_housing {
        tracker.has_built_housing = true;
        let year = (1 + current_tick / crate::layer1::balance::TICKS_PER_YEAR).to_string();
        let mut ctx = NarrativeContext::new();
        ctx.insert("COLONY", &colony.name);
        ctx.insert("YEAR", &year);
        let text = generator
            .generate("FIRST_HOUSING", &ctx)
            .unwrap_or_else(|_| "First Housing constructed. A shelter from the void.".to_string());
        events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
    if found_farm && !tracker.has_built_farm {
        tracker.has_built_farm = true;
        let year = (1 + current_tick / crate::layer1::balance::TICKS_PER_YEAR).to_string();
        let mut ctx = NarrativeContext::new();
        ctx.insert("COLONY", &colony.name);
        ctx.insert("YEAR", &year);
        let text = generator
            .generate("FIRST_FARM", &ctx)
            .unwrap_or_else(|_| "First Farm operational. We shall not starve.".to_string());
        events.send(AddChronicleEvent {
            text,
            importance: EventImportance::Major,
        });
    }
}

/// Format the importance prefix for display.
#[must_use]
pub const fn format_event_prefix(importance: EventImportance) -> &'static str {
    match importance {
        EventImportance::Legendary => "!!!",
        EventImportance::Major => "!",
        _ => " ",
    }
}

impl std::fmt::Display for Chronicle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(target_arch = "wasm32"))]
use comfy_table::{presets::UTF8_FULL, Attribute as TableAttribute, Cell, Color as TableColor, Table};
#[cfg(target_arch = "wasm32")]
use comfy_table::{presets::UTF8_FULL, Cell, Table};
#[cfg(target_arch = "wasm32")]
use crate::wasm_style::{CellStyle, TableAttribute, TableColor};
        #[cfg(not(target_arch = "wasm32"))]
        use crossterm::style::{Color, Stylize};
        #[cfg(target_arch = "wasm32")]
        use crate::wasm_style::{Color, Stylize};

        writeln!(
            f,
            "{}",
            "╭── Colony Chronicle (History) ───────────────────╮".with(Color::Cyan)
        )?;

        if self.events.is_empty() {
            let text = format!("{:<47}", "No events currently recorded.");
            writeln!(f, "│ {} │", text.with(Color::DarkGrey))?;
            writeln!(
                f,
                "{}",
                "╰─────────────────────────────────────────────────╯".with(Color::Cyan)
            )?;
            return Ok(());
        } else {
            let text = format!("{:<47}", format!("{} events recorded.", self.events.len()));
            writeln!(f, "│ {} │", text.with(Color::White))?;
            writeln!(
                f,
                "{}",
                "╰─────────────────────────────────────────────────╯".with(Color::Cyan)
            )?;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("Tick").add_attribute(TableAttribute::Bold),
                Cell::new("Year").add_attribute(TableAttribute::Bold),
                Cell::new("Importance").add_attribute(TableAttribute::Bold),
                Cell::new("Event Description").add_attribute(TableAttribute::Bold),
            ]);

        for event in &self.events {
            let importance_str = format!("{:?}", event.importance);
            let importance_color = match event.importance {
                EventImportance::Legendary => TableColor::Yellow,
                EventImportance::Major => TableColor::Red,
                EventImportance::Standard => TableColor::White,
                EventImportance::Minor => TableColor::DarkGrey,
            };

            table.add_row(vec![
                Cell::new(event.tick.to_string()).fg(TableColor::DarkGrey),
                Cell::new(event.year.to_string()).fg(TableColor::Cyan),
                Cell::new(&importance_str).fg(importance_color),
                Cell::new(&event.text).fg(importance_color),
            ]);
        }

        write!(f, "{}", table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::GridPosition;
    use crate::shared::colony::ColonyName;
    use crate::shared::narrative::NarrativeGenerator;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_event_importance_variants() {
        // Just verify all variants exist
        let _ = EventImportance::Minor;
        let _ = EventImportance::Standard;
        let _ = EventImportance::Major;
        let _ = EventImportance::Legendary;
    }

    #[test]
    fn test_chronicle_event_creation() {
        let event = ChronicleEvent {
            tick: 100,
            year: 1,
            text: "Test event".to_string(),
            importance: EventImportance::Standard,
        };

        assert_eq!(event.tick, 100);
        assert_eq!(event.year, 1);
        assert_eq!(event.text, "Test event");
        assert_eq!(event.importance, EventImportance::Standard);
    }

    #[test]
    fn test_chronicle_default() {
        let chronicle = Chronicle::default();
        assert!(chronicle.events.is_empty());
    }

    #[test]
    fn test_chronicle_add_event() {
        let mut chronicle = Chronicle::default();

        chronicle.add_event(0, "First event".to_string(), EventImportance::Legendary);
        chronicle.add_event(100, "Second event".to_string(), EventImportance::Standard);

        assert_eq!(chronicle.events.len(), 2);
        assert_eq!(chronicle.events[0].text, "First event");
        assert_eq!(chronicle.events[1].text, "Second event");
    }

    #[test]
    fn test_chronicle_year_calculation() {
        let mut chronicle = Chronicle::default();

        chronicle.add_event(0, "Year 1".to_string(), EventImportance::Standard);
        chronicle.add_event(
            TICKS_PER_YEAR,
            "Year 2".to_string(),
            EventImportance::Standard,
        );
        chronicle.add_event(
            TICKS_PER_YEAR * 5,
            "Year 6".to_string(),
            EventImportance::Standard,
        );

        assert_eq!(chronicle.events[0].year, 1);
        assert_eq!(chronicle.events[1].year, 2);
        assert_eq!(chronicle.events[2].year, 6);
    }

    #[test]
    fn test_chronicle_ui_state_default() {
        let ui_state = ChronicleUiState::default();
        assert!(!ui_state.is_open);
    }

    #[test]
    fn test_chronicle_ui_state_toggle() {
        let mut ui_state = ChronicleUiState::default();
        assert!(!ui_state.is_open);

        ui_state.is_open = true;
        assert!(ui_state.is_open);

        ui_state.is_open = !ui_state.is_open;
        assert!(!ui_state.is_open);
    }

    #[test]
    fn test_building_tracker_default() {
        let tracker = BuildingTracker::default();
        assert!(!tracker.has_built_housing);
        assert!(!tracker.has_built_farm);
    }

    #[test]
    fn test_check_milestones_system_housing() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(BuildingTracker::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());
        world.insert_resource(ColonyName::default());

        // Place housing
        world.spawn((
            Building {
                building_type: BuildingType::Housing,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.run_system_once(check_milestones_system).unwrap();

        let events = world.resource::<Events<AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();
        assert_eq!(emitted.len(), 1);
        assert!(!emitted[0].text.is_empty());

        let tracker = world.resource::<BuildingTracker>();
        assert!(tracker.has_built_housing);
    }

    #[test]
    fn test_check_milestones_system_farm() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(BuildingTracker::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());
        world.insert_resource(ColonyName::default());

        // Place farm
        world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));

        world.run_system_once(check_milestones_system).unwrap();

        let events = world.resource::<Events<AddChronicleEvent>>();
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();
        assert_eq!(emitted.len(), 1);
        assert!(!emitted[0].text.is_empty());

        let tracker = world.resource::<BuildingTracker>();
        assert!(tracker.has_built_farm);
    }

    #[test]
    fn test_check_milestones_system_only_once() {
        let mut world = World::new();
        world.init_resource::<Events<AddChronicleEvent>>();
        world.insert_resource(BuildingTracker::default());
        world.insert_resource(SimulationTime::default());
        world.insert_resource(NarrativeGenerator::from_embedded());
        world.insert_resource(ColonyName::default());

        // Place two farms
        world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 5, y: 5 },
        ));
        world.spawn((
            Building {
                building_type: BuildingType::Farm,
            },
            GridPosition { x: 10, y: 10 },
        ));

        world.run_system_once(check_milestones_system).unwrap();

        {
            let events = world.resource::<Events<AddChronicleEvent>>();
            #[allow(deprecated)]
            let mut reader = events.get_reader();
            assert_eq!(
                reader.read(events).count(),
                1,
                "Should only record first farm"
            );
        }

        // Run twice
        world.run_system_once(check_milestones_system).unwrap();

        {
            let events = world.resource::<Events<AddChronicleEvent>>();
            #[allow(deprecated)]
            let mut reader = events.get_reader();
            // Reader tracks read events, so if we read again it should be empty?
            // Actually get_reader creates a NEW reader every time (if it's not stored in Local or ResMut).
            // `events.get_reader()` is deprecated and returns a ManualEventReader.
            // If I create a new reader, it might read from the beginning of the buffer?
            // No, ManualEventReader default constructor starts at 0?
            // Wait, Bevy's manual reader usually needs to be updated or initialized correctly.
            // But here I'm creating a new reader each block.
            // If I want to check total count, I should probably check emitted count per run.
            // The system should NOT emit again.
            // So count should be 0 in the second run.

            // However, since I'm creating a new reader, I might re-read the OLD event if it wasn't cleared.
            // `Events::update()` clears old events. But I'm not calling update() here.
            // So events persist.
            // So a new reader will see ALL events.
            // So I expect count to still be 1 (the first event).
            assert_eq!(
                reader.read(events).count(),
                1,
                "Should still see only 1 event total"
            );
        }
    }

    #[test]
    fn test_initial_chronicle_event() {
        let mut world = World::new();
        world.insert_resource(Chronicle::default());
        world.insert_resource(NarrativeGenerator::from_embedded());
        world.insert_resource(ColonyName::default());

        initial_chronicle_event(&mut world);

        let chronicle = world.resource::<Chronicle>();
        assert_eq!(chronicle.events.len(), 1);
        assert!(!chronicle.events[0].text.is_empty());
        assert_eq!(chronicle.events[0].importance, EventImportance::Legendary);
    }

    #[test]
    fn test_format_event_prefix() {
        assert_eq!(format_event_prefix(EventImportance::Legendary), "!!!");
        assert_eq!(format_event_prefix(EventImportance::Major), "!");
        assert_eq!(format_event_prefix(EventImportance::Standard), " ");
        assert_eq!(format_event_prefix(EventImportance::Minor), " ");
    }
}
