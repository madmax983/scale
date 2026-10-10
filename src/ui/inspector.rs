#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::option_if_let_else)]

//! Inspector panel rendering.
//!
//! The inspector provides context-sensitive details about the currently selected entity or tile.
//! It updates dynamically based on the [`crate::ui::selection::Selection`] resource.

use bevy_ecs::prelude::*;
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Table},
};

#[cfg(feature = "nova")]
use crate::experimental::bureaucratic_martyrdom::Martyrdom;
#[cfg(feature = "nova")]
use crate::experimental::meme_plague::{MemeCarrier, MemeType};
use crate::layer1::biography::Biography;
use crate::layer1::biology::health::Health;
use crate::layer1::day_night::DayNightCycle;
use crate::layer1::energy::load_limits::PowerCable;
use crate::layer1::energy::{Battery, PowerConsumer, PowerSource};
use crate::layer1::environment::light_pollution::SkyGlow;
use crate::layer1::fauna::NocturnalFauna;
use crate::layer1::observatory::Observatory;
use crate::layer1::olfactory::{ScentEmitter, ScentMap};
use crate::layer1::psychology::dreams::DreamJournal;
use crate::layer1::purity::PurityMap;
use crate::layer1::rituals::{MachineSpirit, Quirk, QuirkType};
use crate::layer1::social::old_guard::{Arrival, Generation};
use crate::layer1::utility_types::UtilityWeights;
use crate::layer1::{
    building::Building,
    building::Material,
    building::MaterialType,
    needs::Needs,
    palette_fatigue::DietaryHistory,
    pop::{Pop, PopName},
    resources::RefiningProgress,
    stockpile::Stockpile,
    structure::Structure,
    ActionType, Biocompatibility, ColonyResources, Farm, GridPosition, Housing, PopAction,
    TerrainGrid,
};
use crate::ui::map::{get_building_color, get_terrain_char, get_terrain_color};
use crate::ui::selection::{Selection, SelectionTarget};

/// Helper to format `ActionType` into an icon and label.
const fn format_action_type(action: ActionType) -> (&'static str, &'static str, Color) {
    match action {
        ActionType::SatisfyHunger => ("🍖", "Eating", Color::Green),
        ActionType::SatisfyRest => ("💤", "Sleeping", Color::Blue),
        ActionType::Socialize => ("💬", "Socializing", Color::Yellow),
        ActionType::Explore => ("🔭", "Exploring", Color::Cyan),
        ActionType::Work => ("⚒", "Working", Color::White),
        ActionType::Repair => ("🔧", "Repairing", Color::White),
        ActionType::Research => ("📚", "Researching", Color::Magenta),
        ActionType::Haul => ("📦", "Hauling", Color::Gray),
        ActionType::SeekMedicalCare => ("🏥", "Healing", Color::Red),
        ActionType::BuryCorpse => ("⚰️", "Burying", Color::DarkGray),
        ActionType::FetchTool => ("🔧", "Fetching Tool", Color::Gray),
        ActionType::Idle => ("⏳", "Idle", Color::DarkGray),
        ActionType::Vandalize => ("🔨", "Vandalizing", Color::Red),
        ActionType::Binge => ("🍖", "Bingeing", Color::Red),
        ActionType::Daze => ("😵", "Dazed", Color::Magenta),
        ActionType::Fight => ("⚔️", "Fighting", Color::Red),
        ActionType::Refine => ("⚙️", "Refining", Color::White),
        ActionType::Farm => ("🌾", "Farming", Color::Green),
        ActionType::Warden => ("👮", "Arresting", Color::Blue),
        ActionType::Sleepwalking => ("💤", "Sleepwalking", Color::Magenta),
        ActionType::Tame => ("♥", "Taming", Color::LightGreen),
        ActionType::FireStarting => ("🔥", "Starting Fire", Color::Red),
        ActionType::HideInRoom => ("🚪", "Hiding", Color::DarkGray),
        ActionType::SadWander => ("😢", "Wandering Sadly", Color::Blue),
        ActionType::FetchClothing => ("👕", "Fetching Clothes", Color::Cyan),
        ActionType::Surgery => ("🏥", "Undergoing Surgery", Color::Red),
        ActionType::Charge => ("⚡", "Charging", Color::Cyan),
        ActionType::Hobby => ("🎨", "Hobby", Color::Magenta),
        ActionType::Admin => ("📝", "Administering", Color::Blue),
        ActionType::ScrawlMemeticSigil => ("👁", "Scrawling Sigil", Color::Red),
        ActionType::PreCrimeArrest => ("🛡", "Pre-Crime Arrest", Color::Blue),
        ActionType::ConsumeChemical => ("💊", "Consuming", Color::Magenta),
        ActionType::CollectSample => ("🧬", "Collecting", Color::Cyan),
        ActionType::UseShower => ("🚿", "Showering", Color::Cyan),
        ActionType::ListenToTheHum => ("🌀", "Listening", Color::Magenta),
        ActionType::Clean => ("🧹", "Cleaning", Color::Yellow),
        ActionType::PurgeResidue => ("🧹", "Purging Ghost Code", Color::Red),
        ActionType::VoidStare => ("👁", "Staring into Abyss", Color::Black),
        ActionType::VisitSanctuary => ("🧘", "Seeking Sanctuary", Color::Magenta),
        ActionType::ExtinguishFire => ("🧯", "Extinguishing", Color::Blue),
        ActionType::TreatWounds => ("🩹", "Treating Wounds", Color::Green),
        ActionType::Flee => ("🏃", "Fleeing", Color::Yellow),
        ActionType::Sabotage => ("💣", "Sabotaging", Color::Red),
        ActionType::Protest => ("🗣️", "Protesting", Color::Rgb(200, 50, 50)),
        ActionType::Gossip => ("🗣️", "Gossiping", Color::Magenta),
        ActionType::Philosophize => ("🤔", "Philosophizing", Color::Magenta),
        ActionType::RealityCollapse => ("🔥", "Reality Collapse", Color::Red),
        ActionType::PerformAncientRoutine => ("🗿", "Ancient Routine", Color::Magenta),
        ActionType::MemeticObsession => ("🌀", "Memetic Obsession", Color::Magenta),
        ActionType::Pollinate => ("🌸", "Pollinating", Color::LightGreen),
        ActionType::PhantomWork => ("🌙", "Phantom Shift", Color::Rgb(100, 100, 180)),
    }
}

/// Renders the inspector panel content based on current selection.
///
/// Dispatches rendering to specific helpers based on the [`SelectionTarget`]:
/// - [`SelectionTarget::None`] -> Colony stats (global overview).
/// - [`SelectionTarget::Tile`] -> Tile inspector (terrain info).
/// - [`SelectionTarget::Entity`] -> Entity inspector (pop/building details).
pub fn render_inspector(frame: &mut Frame, area: Rect, world: &World) {
    let selection = world.resource::<Selection>();

    match selection.target() {
        SelectionTarget::None => render_colony_stats(frame, area, world),
        SelectionTarget::Tile(x, y) => render_tile_inspector(frame, area, world, x, y),
        SelectionTarget::Entity(entity) => render_entity_inspector(frame, area, world, entity),
    }
}

fn get_resource_color(current: f32, max: f32, inverse: bool) -> Color {
    if max <= f32::EPSILON {
        return Color::Gray;
    }
    let pct = current / max;
    if inverse {
        if pct < 0.2 {
            Color::Green
        } else if pct < 0.8 {
            Color::Yellow
        } else {
            Color::Red
        }
    } else if pct < 0.2 {
        Color::Red
    } else if pct < 0.5 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn format_mini_bar(current: f32, max: f32, width: usize) -> String {
    if max <= 0.0 {
        return format!("[{}]", "─".repeat(width));
    }
    let pct = (current / max).clamp(0.0, 1.0);
    let filled = (pct * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn create_resource_row<'a>(
    label: &'a str,
    label_color: Color,
    current: f32,
    max: f32,
    inverse: bool,
) -> Row<'a> {
    Row::new(vec![
        Cell::from(label).style(Style::default().fg(label_color)),
        Cell::from(format!(
            "{:.0}/{:.0} {}",
            current,
            max,
            format_mini_bar(current, max, 10)
        ))
        .style(Style::default().fg(get_resource_color(current, max, inverse))),
    ])
}

fn calculate_pop_stats(world: &World) -> (usize, f32) {
    let (pop_count, total_morale) = world
        .iter_entities()
        .filter_map(|e| e.get::<Needs>())
        .fold((0, 0.0), |(count, sum), needs| {
            (count + 1, sum + needs.morale())
        });

    let avg_morale = if pop_count > 0 {
        total_morale / pop_count as f32
    } else {
        0.0
    };

    (pop_count, avg_morale)
}

fn calculate_housing_stats(world: &World) -> (usize, usize, usize) {
    world
        .iter_entities()
        .filter_map(|e| e.get::<Housing>())
        .fold((0, 0, 0), |(count, cap, used), h| {
            (count + 1, cap + h.capacity, used + h.residents.len())
        })
}

fn render_colony_stats(frame: &mut Frame, area: Rect, world: &World) {
    let resources = world.resource::<ColonyResources>();

    let (pop_count, avg_morale) = calculate_pop_stats(world);
    let (_housing_count, housing_capacity, housing_used) = calculate_housing_stats(world);

    // Dashboard Layout
    // 1. Status (Top)
    // 2. Survival
    // 3. Industry
    // 4. Economy
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Status (Pop + Morale)
            Constraint::Length(6), // Survival
            Constraint::Length(6), // Industry
            Constraint::Min(6),    // Economy
        ])
        .split(area);

    render_status_module(
        frame,
        chunks[0],
        pop_count,
        avg_morale,
        housing_used,
        housing_capacity,
    );
    render_survival_module(frame, chunks[1], resources);
    render_industry_module(frame, chunks[2], resources);
    render_economy_module(frame, chunks[3], resources, world);
}

fn render_status_module(
    frame: &mut Frame,
    area: Rect,
    pop_count: usize,
    avg_morale: f32,
    housing_used: usize,
    housing_capacity: usize,
) {
    let morale_color = if avg_morale > 0.7 {
        Color::Green
    } else if avg_morale > 0.4 {
        Color::Yellow
    } else {
        Color::Red
    };

    let status_rows = vec![
        Row::new(vec![
            Cell::from("👥 Pop").style(Style::default().fg(Color::Cyan)),
            Cell::from(format!("{pop_count}")),
        ]),
        Row::new(vec![
            Cell::from("😃 Morale").style(Style::default().fg(Color::Cyan)),
            Cell::from(format!("{:.0}%", avg_morale * 100.0))
                .style(Style::default().fg(morale_color)),
        ]),
        Row::new(vec![
            Cell::from("🏠 Housing").style(Style::default().fg(Color::Cyan)),
            Cell::from(format!("{housing_used}/{housing_capacity}")),
        ]),
    ];

    let status_table = Table::new(
        status_rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .title(" Status ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(status_table, area);
}

fn render_survival_module(frame: &mut Frame, area: Rect, resources: &ColonyResources) {
    let survival_rows = vec![
        create_resource_row(
            "🍖 Food",
            Color::Green,
            resources.total_food(),
            resources.max_food,
            false,
        ),
        create_resource_row(
            "💧 Water",
            Color::Blue,
            resources.water,
            resources.max_water,
            false,
        ),
        create_resource_row(
            "⛽ Fuel",
            Color::Yellow,
            resources.fuel,
            resources.max_fuel,
            false,
        ),
        create_resource_row(
            "🗑 Waste",
            Color::DarkGray,
            resources.waste,
            resources.max_waste,
            true,
        ),
    ];

    let survival_table = Table::new(
        survival_rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .title(" Survival ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Green)),
    );
    frame.render_widget(survival_table, area);
}

fn render_industry_module(frame: &mut Frame, area: Rect, resources: &ColonyResources) {
    let industry_rows = vec![
        create_resource_row(
            "🌲 Wood",
            Color::White,
            resources.wood,
            resources.max_wood,
            false,
        ),
        create_resource_row(
            "🪨 Stone",
            Color::Gray,
            resources.stone,
            resources.max_stone,
            false,
        ),
        create_resource_row(
            "⚙ Metal",
            Color::LightBlue,
            resources.metal,
            resources.max_metal,
            false,
        ),
        create_resource_row(
            "🔧 Tools",
            Color::Cyan,
            resources.tools,
            resources.max_tools,
            false,
        ),
    ];

    let industry_table = Table::new(
        industry_rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .title(" Industry ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Gray)),
    );
    frame.render_widget(industry_table, area);
}

fn render_economy_module(
    frame: &mut Frame,
    area: Rect,
    resources: &ColonyResources,
    world: &World,
) {
    let economy_rows = vec![
        create_resource_row(
            "🔬 Tech",
            Color::Magenta,
            resources.knowledge,
            resources.max_knowledge,
            false,
        ),
        create_resource_row(
            "👕 Clothes",
            Color::LightMagenta,
            resources.clothing,
            resources.max_clothing,
            false,
        ),
        create_resource_row(
            "🍺 Alcohol",
            Color::Yellow,
            resources.alcohol,
            resources.max_alcohol,
            false,
        ),
        Row::new(vec![
            Cell::from("📜 Permits").style(Style::default().fg(Color::White)),
            Cell::from(format!("{:.0}", resources.building_permits)),
        ]),
        Row::new(vec![
            Cell::from("✨ Sky Glow").style(Style::default().fg(Color::Cyan)),
            Cell::from(format!(
                "{:.1}",
                world
                    .get_resource::<crate::layer1::environment::light_pollution::SkyGlow>()
                    .map_or(0.0, |g| g.global_level)
            )),
        ]),
    ];

    let economy_table = Table::new(
        economy_rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .title(" Economy ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta)),
    );
    frame.render_widget(economy_table, area);
}

fn get_purity_line(
    world: &World,
    x: i32,
    y: i32,
    terrain_type: Option<crate::layer1::terrain::TerrainType>,
) -> Option<Line<'static>> {
    if terrain_type != Some(crate::layer1::terrain::TerrainType::Rock) {
        return None;
    }
    let map = world.get_resource::<PurityMap>()?;
    let purity = map.get(x, y);
    let pct = (purity * 100.0) as u32;
    let color = if purity > 0.8 {
        Color::Green
    } else if purity > 0.4 {
        Color::Yellow
    } else {
        Color::Red
    };
    Some(Line::from(vec![
        Span::raw("Purity: "),
        Span::styled(format!("{pct}%"), Style::default().fg(color)),
    ]))
}

fn get_scent_line(world: &World, x: i32, y: i32) -> Option<Line<'static>> {
    let scent_map = world.get_resource::<ScentMap>()?;
    let tile_scent = scent_map.get_scent(GridPosition { x, y });

    if tile_scent.pleasant <= 0.0 && tile_scent.foul <= 0.0 {
        return None;
    }

    Some(Line::from(vec![
        Span::raw("Scent: "),
        Span::styled(
            format!("🌸 {:.1}", tile_scent.pleasant),
            Style::default().fg(Color::LightMagenta),
        ),
        Span::raw(" / "),
        Span::styled(
            format!("🤢 {:.1}", tile_scent.foul),
            Style::default().fg(Color::Rgb(150, 200, 50)),
        ),
    ]))
}

fn get_echo_line(world: &World, x: i32, y: i32) -> Option<Line<'static>> {
    use crate::layer1::anomalies::echo::{Echo, EchoSource, EchoType};

    let target_pos = GridPosition { x, y };

    let mut echo_type_found = None;
    let mut is_source = false;

    // Archetype iteration for read-only querying without QueryState
    let echo_component_id = world.component_id::<Echo>()?;
    let source_component_id = world.component_id::<EchoSource>()?;
    let pos_component_id = world.component_id::<GridPosition>()?;

    for archetype in world.archetypes().iter() {
        if archetype.contains(pos_component_id) {
            for entity in archetype.entities() {
                if let Some(pos) = world.get::<GridPosition>(entity.id()) {
                    if *pos == target_pos {
                        if archetype.contains(source_component_id) {
                            if let Some(source) = world.get::<EchoSource>(entity.id()) {
                                echo_type_found = Some(source.event_type);
                                is_source = true;
                                break;
                            }
                        } else if archetype.contains(echo_component_id) {
                            if let Some(echo) = world.get::<Echo>(entity.id()) {
                                echo_type_found = Some(echo.event_type);
                                break;
                            }
                        }
                    }
                }
            }
        }
        if echo_type_found.is_some() && is_source {
            break;
        }
    }

    echo_type_found.map(|echo_type| {
        let (label, color) = match echo_type {
            EchoType::Tragedy => ("Tragedy", Color::Red),
            EchoType::Triumph => ("Triumph", Color::Yellow),
            EchoType::Mystery => ("Mystery", Color::Cyan),
        };

        let prefix = if is_source { "Echo Source: " } else { "Echo: " };

        Line::from(vec![
            Span::styled(prefix, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(label, Style::default().fg(color)),
        ])
    })
}

fn render_tile_inspector(frame: &mut Frame, area: Rect, world: &World, x: i32, y: i32) {
    let terrain = world.resource::<TerrainGrid>();

    if x < 0 || y < 0 {
        let text = Paragraph::new("Outside Map").style(Style::default().fg(Color::Red));
        frame.render_widget(text, area);
        return;
    }

    let (name, char, color, terrain_type) = terrain
        .get(x as usize, y as usize)
        .map_or(("Unknown", "?", Color::Red, None), |t| {
            (t.name(), get_terrain_char(t), get_terrain_color(t), Some(t))
        });

    let purity_line = get_purity_line(world, x, y, terrain_type);
    let echo_line = get_echo_line(world, x, y);
    let scent_line = get_scent_line(world, x, y);

    let mut constraints = vec![
        Constraint::Length(1), // Header
        Constraint::Length(1), // Coords
    ];

    if purity_line.is_some() {
        constraints.push(Constraint::Length(1));
    }

    if echo_line.is_some() {
        constraints.push(Constraint::Length(1));
    }

    if scent_line.is_some() {
        constraints.push(Constraint::Length(1));
    }

    constraints.push(Constraint::Min(1)); // Visual

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Terrain: "),
            Span::styled(
                name,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])),
        layout[0],
    );

    frame.render_widget(
        Paragraph::new(format!("Position: ({x}, {y})")).style(Style::default().fg(Color::DarkGray)),
        layout[1],
    );

    let mut current_idx = 2;
    if let Some(line) = purity_line {
        frame.render_widget(Paragraph::new(line), layout[current_idx]);
        current_idx += 1;
    }

    if let Some(line) = echo_line {
        frame.render_widget(Paragraph::new(line), layout[current_idx]);
        current_idx += 1;
    }

    if let Some(line) = scent_line {
        frame.render_widget(Paragraph::new(line), layout[current_idx]);
        current_idx += 1;
    }

    let visual_idx = current_idx;
    render_tile_visual(frame, layout[visual_idx], char, color);
}

fn render_tile_visual(frame: &mut Frame, area: Rect, char: &str, color: Color) {
    let visual_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Visual ");
    let visual_inner = visual_block.inner(area);
    frame.render_widget(visual_block, area);

    let visual = Paragraph::new(char)
        .style(Style::default().fg(color))
        .alignment(Alignment::Center);

    // Center the char vertically
    let v_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ])
        .split(visual_inner);

    frame.render_widget(visual, v_layout[1]);
}

fn get_entity_header(world: &World, entity: Entity) -> (String, Color) {
    if let Some(pop_name) = world.get::<PopName>(entity) {
        (pop_name.0.clone(), Color::Yellow)
    } else if world.get::<Pop>(entity).is_some() {
        ("Colonist".to_string(), Color::Yellow)
    } else if let Some(b) = world.get::<Building>(entity) {
        let material = world
            .get::<Material>(entity)
            .map_or_else(MaterialType::default, |m| m.0);
        (
            b.building_type.label().to_string(),
            get_building_color(b.building_type, material),
        )
    } else {
        ("Entity".to_string(), Color::White)
    }
}

fn get_action_line(world: &World, entity: Entity) -> Option<Line<'static>> {
    world.get::<PopAction>(entity).map(|action| {
        let (icon, label, color) = format_action_type(action.current);
        Line::from(vec![
            Span::raw("Action: "),
            Span::styled(format!("{icon} {label}"), Style::default().fg(color)),
        ])
    })
}

fn get_generation_line(world: &World, entity: Entity) -> Option<Line<'static>> {
    world.get::<Generation>(entity).map(|generation| {
        let label = match generation {
            Generation::Founder => "Founder",
            Generation::Immigrant => "Immigrant",
        };
        let year_str = if let Some(arrival) = world.get::<Arrival>(entity) {
            let year = 1 + arrival.tick / crate::layer1::balance::TICKS_PER_YEAR;
            format!(" (Year {year})")
        } else {
            String::new()
        };

        let color = match generation {
            Generation::Founder => Color::LightYellow,
            Generation::Immigrant => Color::Gray,
        };

        Line::from(vec![
            Span::raw("Status: "),
            Span::styled(format!("{label}{year_str}"), Style::default().fg(color)),
        ])
    })
}

type InspectorWidget<'a> = Box<dyn FnOnce(&mut Frame, Rect) + 'a>;

struct InspectorLayout<'a> {
    constraints: Vec<Constraint>,
    widgets: Vec<InspectorWidget<'a>>,
}

impl<'a> InspectorLayout<'a> {
    fn new() -> Self {
        Self {
            constraints: Vec::new(),
            widgets: Vec::new(),
        }
    }

    fn push(&mut self, height: u16, render: impl FnOnce(&mut Frame, Rect) + 'a) {
        if height > 0 {
            self.constraints.push(Constraint::Length(height));
            self.widgets.push(Box::new(render));
        }
    }

    fn push_min(&mut self, min_height: u16, render: impl FnOnce(&mut Frame, Rect) + 'a) {
        self.constraints.push(Constraint::Min(min_height));
        self.widgets.push(Box::new(render));
    }
}

fn push_entity_header<'a>(layout: &mut InspectorLayout<'a>, world: &'a World, entity: Entity) {
    let (name, color) = get_entity_header(world, entity);
    layout.push(1, move |f, a| {
        f.render_widget(
            Paragraph::new(Span::styled(
                name,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            a,
        );
    });

    if let Some(pos) = world.get::<GridPosition>(entity) {
        let pos_str = format!("Position: ({}, {})", pos.x, pos.y);
        layout.push(1, move |f, a| {
            f.render_widget(
                Paragraph::new(pos_str).style(Style::default().fg(Color::DarkGray)),
                a,
            );
        });
    }

    if let Some(line) = get_generation_line(world, entity) {
        layout.push(1, move |f, a| f.render_widget(Paragraph::new(line), a));
    }

    if let Some(line) = get_action_line(world, entity) {
        layout.push(1, move |f, a| f.render_widget(Paragraph::new(line), a));
    }

    layout.push(1, |_, _| {}); // Spacer
}

fn push_needs_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(needs) = world.get::<Needs>(entity) {
        let bio_opt = world.get::<Biocompatibility>(entity);
        let health_opt = world.get::<Health>(entity);
        let has_rust_lung = health_opt.is_some_and(|h| h.has_rust_lung);
        let mut details_height = 6;
        if has_rust_lung {
            details_height += 1;
        }
        layout.push(details_height, move |f, a| {
            render_bio_monitor(f, a, needs, bio_opt, health_opt);
        });
        return true;
    }
    false
}

fn push_housing_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(housing) = world.get::<Housing>(entity) {
        layout.push(3, move |f, a| render_housing_details(f, a, housing));
        return true;
    }
    false
}

fn push_farm_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(farm) = world.get::<Farm>(entity) {
        layout.push(3, move |f, a| render_farm_details(f, a, farm));
        return true;
    }
    false
}

fn push_stockpile_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(stockpile) = world.get::<Stockpile>(entity) {
        layout.push(6, move |f, a| render_stockpile_details(f, a, stockpile));
        return true;
    }
    false
}

fn push_refining_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(progress) = world.get::<RefiningProgress>(entity) {
        layout.push(3, move |f, a| render_refining_details(f, a, progress));
        return true;
    }
    false
}

fn push_observatory_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(obs) = world.get::<Observatory>(entity) {
        layout.push(3, move |f, a| render_observatory_details(f, a, obs, world));
        return true;
    }
    false
}

fn push_nocturnal_fauna_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) -> bool {
    if let Some(fauna) = world.get::<NocturnalFauna>(entity) {
        layout.push(3, move |f, a| {
            render_nocturnal_fauna_details(f, a, fauna, world)
        });
        return true;
    }
    false
}

fn push_structure_component<'a>(
    layout: &mut InspectorLayout<'a>,
    world: &'a World,
    entity: Entity,
) {
    if let Some(structure) = world.get::<Structure>(entity) {
        let pct = if structure.max_hp > 0.0 {
            ((structure.current_hp / structure.max_hp * 100.0).clamp(0.0, 100.0)) as u16
        } else {
            0
        };
        let color = if pct > 66 {
            Color::Green
        } else if pct > 33 {
            Color::Yellow
        } else {
            Color::Red
        };
        let hp_str = format!("{:.0}/{:.0}", structure.current_hp, structure.max_hp);
        layout.push(1, move |f, a| {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw("HP: "),
                    Span::styled(hp_str, Style::default().fg(color)),
                ])),
                a,
            );
        });
    }
}

fn push_entity_components<'a>(layout: &mut InspectorLayout<'a>, world: &'a World, entity: Entity) {
    let _ = push_needs_component(layout, world, entity)
        || push_housing_component(layout, world, entity)
        || push_farm_component(layout, world, entity)
        || push_stockpile_component(layout, world, entity)
        || push_refining_component(layout, world, entity)
        || push_observatory_component(layout, world, entity)
        || push_nocturnal_fauna_component(layout, world, entity);

    push_structure_component(layout, world, entity);
}

fn push_power_diagnostics<'a>(layout: &mut InspectorLayout<'a>, world: &'a World, entity: Entity) {
    let spirit_opt = world.get::<MachineSpirit>(entity);
    let quirk_opt = world.get::<Quirk>(entity);
    if spirit_opt.is_some() || quirk_opt.is_some() {
        let diag_height = 2 + u16::from(spirit_opt.is_some()) + u16::from(quirk_opt.is_some());
        layout.push(diag_height, move |f, a| {
            render_diagnostics(f, a, spirit_opt, quirk_opt);
        });
    }

    if let Some(cable) = world.get::<PowerCable>(entity) {
        layout.push(1, move |f, a| render_power_cable_info(f, a, cable));
    }
    if let Some(battery) = world.get::<Battery>(entity) {
        layout.push(1, move |f, a| render_battery_info(f, a, battery));
    }
    if let Some(consumer) = world.get::<PowerConsumer>(entity) {
        layout.push(1, move |f, a| render_power_consumer_info(f, a, consumer));
    }
    if let Some(source) = world.get::<PowerSource>(entity) {
        layout.push(1, move |f, a| render_power_source_info(f, a, source));
    }
    if let Some(emitter) = world.get::<ScentEmitter>(entity) {
        layout.push(1, move |f, a| render_scent_emitter_info(f, a, emitter));
    }
}

fn push_mind_info<'a>(layout: &mut InspectorLayout<'a>, world: &'a World, entity: Entity) {
    #[cfg(feature = "nova")]
    if let Some(meme) = world.get::<MemeCarrier>(entity) {
        layout.push(1, move |f, a| render_meme_carrier_info(f, a, meme));
    }
    #[cfg(feature = "nova")]
    if let Some(martyrdom) = world.get::<Martyrdom>(entity) {
        if martyrdom.active {
            layout.push(1, move |f, a| render_martyrdom_info(f, a, martyrdom));
        }
    }

    if let Some(weights) = world.get::<UtilityWeights>(entity) {
        layout.push(2, move |f, a| render_personality(f, a, *weights));
    }

    if let Some(journal) = world.get::<DreamJournal>(entity) {
        layout.push(1, move |f, a| render_dream_journal(f, a, journal));
    }

    if let Some(history) = world.get::<DietaryHistory>(entity) {
        layout.push(1, move |f, a| render_dietary_history(f, a, history));
    }

    if let Some(bio) = world.get::<Biography>(entity) {
        layout.push_min(1, move |f, a| render_biography(f, a, bio, world));
    }
}

fn render_entity_inspector(frame: &mut Frame, area: Rect, world: &World, entity: Entity) {
    if !world.entities().contains(entity) {
        frame.render_widget(
            Paragraph::new("Entity Despawned").style(Style::default().fg(Color::Red)),
            area,
        );
        return;
    }

    let mut layout = InspectorLayout::new();

    push_entity_header(&mut layout, world, entity);
    push_entity_components(&mut layout, world, entity);
    push_power_diagnostics(&mut layout, world, entity);
    push_mind_info(&mut layout, world, entity);

    if layout.constraints.is_empty() {
        return;
    }
    let layout_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(layout.constraints)
        .split(area);
    for (widget, chunk) in layout.widgets.into_iter().zip(layout_rects.iter()) {
        widget(frame, *chunk);
    }
}

fn render_bio_monitor(
    frame: &mut Frame,
    details_area: Rect,
    needs: &Needs,
    bio_opt: Option<&Biocompatibility>,
    health_opt: Option<&Health>,
) {
    let bio_block = Block::default()
        .title(" Bio-Monitor ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));

    let bio_inner = bio_block.inner(details_area);
    frame.render_widget(bio_block, details_area);

    let has_rust_lung = health_opt.is_some_and(|h: &Health| h.has_rust_lung);

    let constraints = if bio_opt.is_some() && has_rust_lung {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else if bio_opt.is_some() || has_rust_lung {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![Constraint::Length(1), Constraint::Length(1)]
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(bio_inner);

    render_needs_gauges(frame, rows[0], needs);
    render_morale_gauge(frame, rows[1], needs);

    let mut current_row = 2;

    if let Some(bio) = bio_opt {
        render_bio_status(frame, rows[current_row], bio);
        current_row += 1;
    }

    if has_rust_lung {
        let rust_lung_label = Paragraph::new(Span::styled(
            "⚠️ Condition: Rust-Lung",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(rust_lung_label, rows[current_row]);
    }
}

fn render_needs_gauges(frame: &mut Frame, area: Rect, needs: &Needs) {
    let needs_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(1), // Gap
            Constraint::Percentage(50),
        ])
        .split(area);

    let hunger_percent = ((needs.hunger * 100.0).clamp(0.0, 100.0)) as u16;
    let rest_percent = ((needs.rest * 100.0).clamp(0.0, 100.0)) as u16;

    let hunger_color = if needs.hunger < 0.3 {
        Color::Red
    } else {
        Color::Green
    };
    let rest_color = if needs.rest < 0.3 {
        Color::Red
    } else {
        Color::Cyan
    };

    let hunger_gauge = Gauge::default()
        .gauge_style(Style::default().fg(hunger_color))
        .label(format!("🍖 {hunger_percent}%"))
        .percent(hunger_percent);

    let rest_gauge = Gauge::default()
        .gauge_style(Style::default().fg(rest_color))
        .label(format!("💤 {rest_percent}%"))
        .percent(rest_percent);

    frame.render_widget(hunger_gauge, needs_layout[0]);
    frame.render_widget(rest_gauge, needs_layout[2]);
}

fn render_morale_gauge(frame: &mut Frame, area: Rect, needs: &Needs) {
    let morale = needs.morale();
    let morale_percent = ((morale * 100.0).clamp(0.0, 100.0)) as u16;
    let morale_color = if morale < 0.3 {
        Color::Red
    } else if morale < 0.7 {
        Color::Yellow
    } else {
        Color::Green
    };

    let morale_gauge = Gauge::default()
        .gauge_style(Style::default().fg(morale_color))
        .label(format!("😃 Morale: {morale_percent}%"))
        .percent(morale_percent);

    frame.render_widget(morale_gauge, area);
}

fn render_bio_status(frame: &mut Frame, area: Rect, bio: &Biocompatibility) {
    let bio_percent = ((bio.value * 100.0).clamp(0.0, 100.0)) as u16;
    let bio_color = if bio.value < 0.4 {
        Color::Red
    } else if bio.value < 0.7 {
        Color::Yellow
    } else {
        Color::Green
    };

    let bio_gauge = Gauge::default()
        .gauge_style(Style::default().fg(bio_color))
        .label(format!("🧬 Bio-Comp: {bio_percent}%"))
        .percent(bio_percent);

    frame.render_widget(bio_gauge, area);
}

fn render_power_cable_info(frame: &mut Frame, area: Rect, cable: &PowerCable) {
    let load_color = if cable.current_load > cable.capacity {
        Color::Red
    } else {
        Color::Cyan
    };
    let load_pct = if cable.capacity > 0.0 {
        (cable.current_load / cable.capacity) * 100.0
    } else {
        0.0
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("⚡ Cable Load: "),
            Span::styled(
                format!(
                    "{:.0}/{:.0} ({:.0}%)",
                    cable.current_load, cable.capacity, load_pct
                ),
                Style::default().fg(load_color),
            ),
        ])),
        area,
    );
}

fn render_battery_info(frame: &mut Frame, area: Rect, battery: &Battery) {
    let charge_pct = if battery.capacity > 0.0 {
        (battery.charge / battery.capacity) * 100.0
    } else {
        0.0
    };
    let charge_color = if charge_pct < 20.0 {
        Color::Red
    } else if charge_pct < 80.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("🔋 Battery Charge: "),
            Span::styled(
                format!(
                    "{:.0}/{:.0} ({:.0}%)",
                    battery.charge, battery.capacity, charge_pct
                ),
                Style::default().fg(charge_color),
            ),
        ])),
        area,
    );
}

fn render_power_consumer_info(frame: &mut Frame, area: Rect, consumer: &PowerConsumer) {
    let status = if consumer.active {
        "Active"
    } else {
        "Inactive"
    };
    let color = if consumer.active {
        Color::Green
    } else {
        Color::Red
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("🔌 Consumer Demand: "),
            Span::styled(
                format!("{:.0} ({})", consumer.demand, status),
                Style::default().fg(color),
            ),
        ])),
        area,
    );
}

fn render_power_source_info(frame: &mut Frame, area: Rect, source: &PowerSource) {
    let status = if source.active { "Active" } else { "Inactive" };
    let color = if source.active {
        Color::Green
    } else {
        Color::Red
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("🏭 Source Output: "),
            Span::styled(
                format!("{:.0} ({})", source.output, status),
                Style::default().fg(color),
            ),
        ])),
        area,
    );
}

fn render_scent_emitter_info(frame: &mut Frame, area: Rect, emitter: &ScentEmitter) {
    let color = if emitter.is_pleasant {
        Color::LightMagenta
    } else {
        Color::Rgb(150, 200, 50)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("💨 Emits Scent: "),
            Span::styled(
                format!(
                    "{} ({:.1})",
                    if emitter.is_pleasant {
                        "Pleasant"
                    } else {
                        "Foul"
                    },
                    emitter.strength
                ),
                Style::default().fg(color),
            ),
        ])),
        area,
    );
}

#[cfg(feature = "nova")]
fn render_meme_carrier_info(frame: &mut Frame, area: Rect, meme: &MemeCarrier) {
    let (label, color) = match meme.meme_type {
        MemeType::WorkCult => ("Work Cult", Color::Yellow),
        MemeType::DanceMeme => ("Dance Fever", Color::Magenta),
        MemeType::ParanoiaMeme => ("Paranoia", Color::Red),
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("🧠 Meme Infection: "),
            Span::styled(
                format!("{} ({}t remaining)", label, meme.duration),
                Style::default().fg(color),
            ),
        ])),
        area,
    );
}

#[cfg(feature = "nova")]
fn render_martyrdom_info(frame: &mut Frame, area: Rect, martyrdom: &Martyrdom) {
    if martyrdom.active {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "⚠️ BUREAUCRATIC MARTYRDOM ACTIVE ⚠️",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::RAPID_BLINK),
            )])),
            area,
        );
    }
}

fn render_diagnostics(
    frame: &mut Frame,
    diag_area: Rect,
    spirit_opt: Option<&MachineSpirit>,
    quirk_opt: Option<&Quirk>,
) {
    let block = Block::default()
        .title(" Diagnostics ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(diag_area);
    frame.render_widget(block, diag_area);

    let constraints = if spirit_opt.is_some() && quirk_opt.is_some() {
        vec![Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Length(1)]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut current_chunk = 0;
    if let Some(spirit) = spirit_opt {
        render_machine_spirit(frame, chunks[current_chunk], spirit);
        current_chunk += 1;
    }
    if let Some(quirk) = quirk_opt {
        render_quirk(frame, chunks[current_chunk], quirk);
    }
}

fn render_machine_spirit(frame: &mut Frame, area: Rect, spirit: &MachineSpirit) {
    let pct = spirit.anger.clamp(0.0, 100.0) as u16;
    let color = if spirit.anger > 70.0 {
        Color::Red
    } else if spirit.anger > 30.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(color))
        .label(format!("Spirit Anger: {:.0}%", spirit.anger))
        .percent(pct);

    frame.render_widget(gauge, area);
}

fn render_observatory_details(frame: &mut Frame, area: Rect, obs: &Observatory, world: &World) {
    let pct = obs.efficiency.clamp(0.0, 100.0) as u16;
    let color = if pct < 50 {
        Color::Red
    } else if pct < 80 {
        Color::Yellow
    } else {
        Color::Cyan
    };

    let label = if let Some(glow) = world.get_resource::<SkyGlow>() {
        if glow.global_level > 0.0 {
            format!("{pct}% (Pollution: {:.1})", glow.global_level)
        } else {
            format!("{pct}%")
        }
    } else {
        format!("{pct}%")
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Lens Efficiency ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(Style::default().fg(color))
        .label(label)
        .percent(pct);

    frame.render_widget(gauge, area);
}

fn render_nocturnal_fauna_details(
    frame: &mut Frame,
    area: Rect,
    fauna: &NocturnalFauna,
    world: &World,
) {
    let max_aggression = 10.0; // Assume a reasonable max for UI display purposes
    let pct = ((fauna.aggression / max_aggression) * 100.0).clamp(0.0, 100.0) as u16;
    let color = if pct > 70 {
        Color::Red
    } else if pct > 30 {
        Color::Yellow
    } else {
        Color::Green
    };

    let title = if let Some(glow) = world.get_resource::<SkyGlow>() {
        if glow.global_level > 0.0 {
            " Agitated by Light "
        } else {
            " Aggression "
        }
    } else {
        " Aggression "
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Red)),
        )
        .gauge_style(Style::default().fg(color))
        .label(format!("Aggression: {:.1}", fauna.aggression))
        .percent(pct);

    frame.render_widget(gauge, area);
}

fn render_quirk(frame: &mut Frame, area: Rect, quirk: &Quirk) {
    let text = match quirk.quirk_type {
        QuirkType::Glitchy => "⚠ Glitchy (Stops Production)",
        QuirkType::Overheating => "⚠ Overheating (Fire Risk)",
        QuirkType::Demanding => "⚠ Demanding (Anger++)",
        QuirkType::Haunted => "⚠ Haunted (Stress++)",
        QuirkType::GasLeak => "⚠ Gas Leak (Hazard)",
        QuirkType::Pests => "⚠ Pests (Disease Risk)",
    };

    let p =
        Paragraph::new(text).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
    frame.render_widget(p, area);
}

fn render_dream_journal(frame: &mut Frame, area: Rect, journal: &DreamJournal) {
    if let Some(last_dream) = &journal.last_dream {
        let color = if last_dream.is_nightmare {
            Color::Red
        } else {
            Color::LightBlue
        };
        let p = Paragraph::new(Line::from(vec![
            Span::raw("Dream: "),
            Span::styled(&last_dream.content, Style::default().fg(color)),
        ]));
        frame.render_widget(p, area);
    }
}

fn render_dietary_history(frame: &mut Frame, area: Rect, history: &DietaryHistory) {
    if history.recent_meals.is_empty() {
        return;
    }
    // ⚡ Bolt Optimization: Replace intermediate `.collect::<Vec<String>>()` allocation with pre-allocated String
    let mut text = String::with_capacity(15 + history.recent_meals.len() * 10);
    text.push_str("Recent Meals: ");
    for (i, meal) in history.recent_meals.iter().enumerate() {
        if i > 0 {
            text.push_str(", ");
        }
        text.push_str(&format!("{meal:?}"));
    }
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::Gray)),
        area,
    );
}

fn render_housing_details(frame: &mut Frame, area: Rect, housing: &Housing) {
    let residents_count = housing.residents.len();
    let capacity = housing.capacity;
    let percent = (residents_count as f32 / capacity as f32).clamp(0.0, 1.0);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Housing ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(Style::default().fg(if residents_count >= capacity {
            Color::Yellow
        } else {
            Color::Green
        }))
        .label(format!("{residents_count}/{capacity}"))
        .percent((percent * 100.0) as u16);

    frame.render_widget(gauge, area);
}

fn render_farm_details(frame: &mut Frame, area: Rect, farm: &Farm) {
    let workers_count = farm.workers.len();
    let capacity = farm.capacity;
    let percent = (workers_count as f32 / capacity as f32).clamp(0.0, 1.0);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Farm ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .gauge_style(Style::default().fg(if workers_count >= capacity {
            Color::Green
        } else {
            Color::Yellow
        }))
        .label(format!("{workers_count}/{capacity}"))
        .percent((percent * 100.0) as u16);

    frame.render_widget(gauge, area);
}

fn render_stockpile_details(frame: &mut Frame, area: Rect, stockpile: &Stockpile) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Storage Bonus ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Gray));

    // ⚡ Bolt Optimization: Pre-allocate capacity for `lines`
    let mut lines = Vec::with_capacity(4);
    if stockpile.food_bonus > 0.0 {
        lines.push(Line::from(vec![
            Span::raw("Food: +"),
            Span::styled(
                format!("{:.0}", stockpile.food_bonus),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }
    if stockpile.wood_bonus > 0.0 {
        lines.push(Line::from(vec![
            Span::raw("Wood: +"),
            Span::styled(
                format!("{:.0}", stockpile.wood_bonus),
                Style::default().fg(Color::Green),
            ),
        ]));
    }
    if stockpile.stone_bonus > 0.0 {
        lines.push(Line::from(vec![
            Span::raw("Stone: +"),
            Span::styled(
                format!("{:.0}", stockpile.stone_bonus),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    if stockpile.waste_bonus > 0.0 {
        lines.push(Line::from(vec![
            Span::raw("Waste: +"),
            Span::styled(
                format!("{:.0}", stockpile.waste_bonus),
                Style::default().fg(Color::Rgb(85, 107, 47)),
            ),
        ]));
    }

    let p = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(p, area);
}

fn render_refining_details(frame: &mut Frame, area: Rect, progress: &RefiningProgress) {
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Production ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .gauge_style(Style::default().fg(Color::LightGreen))
        .percent(((progress.current / progress.max) * 100.0).clamp(0.0, 100.0) as u16);

    frame.render_widget(gauge, area);
}

fn render_biography(frame: &mut Frame, area: Rect, bio: &Biography, world: &World) {
    let bio_block = Block::default()
        .borders(Borders::TOP)
        .title(" Biography ")
        .title_style(Style::default().fg(Color::Blue));

    let cycle = world.resource::<DayNightCycle>();
    let ticks_per_day = cycle.ticks_per_day.max(1); // Avoid div by zero

    // Show last 5 events reversed
    // ⚡ Bolt Optimization: Use an iterator instead of collecting into an intermediate `Vec<ListItem>`
    // to prevent allocations when rendering the biography list.
    let events = bio.events.iter().rev().take(5).map(|e| {
        let day = e.tick / ticks_per_day;
        let day_tick = e.tick % ticks_per_day;
        #[allow(clippy::cast_precision_loss)]
        let pct = day_tick as f32 / ticks_per_day as f32;

        // Approximate phase for past events (since we don't store phase history)
        // Using same thresholds as day_night.rs
        let phase = if pct < 0.1 {
            "Dawn"
        } else if pct < 0.75 {
            "Day"
        } else if pct < 0.85 {
            "Dusk"
        } else {
            "Night"
        };

        let phase_color = match phase {
            "Dawn" => Color::LightYellow,
            "Day" => Color::Yellow,
            "Dusk" => Color::Rgb(255, 165, 0), // Orange-ish
            "Night" => Color::Blue,
            _ => Color::White,
        };

        ListItem::new(Line::from(vec![
            Span::styled(format!("Day {day} "), Style::default().fg(Color::White)),
            Span::styled(format!("{phase:5} "), Style::default().fg(phase_color)),
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            Span::raw(&e.text),
        ]))
    });

    let list = List::new(events).block(bio_block);

    frame.render_widget(list, area);
}

fn render_personality(frame: &mut Frame, area: Rect, weights: UtilityWeights) {
    if area.height < 2 {
        return;
    }

    // ⚡ Bolt Optimization: Pre-allocate vector for traits
    let mut traits = Vec::with_capacity(3);

    // Distance
    if weights.distance_weight > 1.2 {
        traits.push(Span::styled(
            "Homebody",
            Style::default().fg(Color::LightBlue),
        ));
    } else if weights.distance_weight < 0.8 {
        traits.push(Span::styled(
            "Nomad",
            Style::default().fg(Color::LightGreen),
        ));
    }

    // Availability
    if weights.availability_weight > 1.2 {
        traits.push(Span::styled(
            "Introvert",
            Style::default().fg(Color::LightMagenta),
        ));
    } else if weights.availability_weight < 0.8 {
        traits.push(Span::styled(
            "Socialite",
            Style::default().fg(Color::Yellow),
        ));
    }

    // Default if boring
    if traits.is_empty() {
        traits.push(Span::styled(
            "Average Joe",
            Style::default().fg(Color::Gray),
        ));
    }

    // Intersperse with commas
    // ⚡ Bolt Optimization: Use `Vec::with_capacity` to prevent multiple allocations
    // while building the list of trait spans.
    let mut spans = Vec::with_capacity(traits.len() * 2 + 1);
    spans.push(Span::raw("Traits: "));
    for (i, t) in traits.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(t);
    }

    let p = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::NONE)); // No block to save space

    frame.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_inspector_render_pop() {
        use crate::layer1::pop::PopName;

        let mut world = World::new();
        // Setup world
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Pop,
                PopName("Ada".to_string()),
                GridPosition { x: 1, y: 1 },
                Needs {
                    hunger: 0.5,
                    rest: 0.8,
                    ..Default::default()
                },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        // Setup Terminal
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();

        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        // Check pop name appears instead of generic "Colonist"
        assert!(full_text.contains("Ada"));
        assert!(full_text.contains("Bio-Monitor"));
    }

    #[test]
    fn test_inspector_render_pop_morale() {
        use crate::layer1::pop::PopName;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Pop,
                PopName("Bryn".to_string()),
                GridPosition { x: 2, y: 3 },
                Needs {
                    hunger: 0.8,
                    rest: 0.9,
                    leisure: 0.6,
                    hygiene: 0.8,
                },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Bryn"));
        assert!(full_text.contains("Morale"));
    }

    #[test]
    fn test_inspector_render_stockpile() {
        use crate::layer1::building::BuildingType;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Building {
                    building_type: BuildingType::Stockpile,
                },
                Stockpile {
                    food_bonus: 0.0,
                    wood_bonus: 100.0,
                    stone_bonus: 50.0,
                    waste_bonus: 0.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Stockpile"));
        assert!(full_text.contains("Storage Bonus"));
        assert!(full_text.contains("Wood: +100"));
        assert!(full_text.contains("Stone: +50"));
        // Food is 0, so it should NOT be there
        assert!(!full_text.contains("Food: +0"));
    }

    #[test]
    fn test_inspector_render_refining() {
        use crate::layer1::building::BuildingType;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Building {
                    building_type: BuildingType::LumberMill,
                },
                RefiningProgress {
                    current: 50.0,
                    max: 100.0,
                },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Lumber Mill"));
        // "Production" is the title of the gauge block
        assert!(full_text.contains("Production"));
    }

    #[test]
    fn test_inspector_render_dream() {
        use crate::layer1::pop::PopName;
        use crate::layer1::psychology::dreams::{Dream, DreamJournal};

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Pop,
                PopName("Dreamer".to_string()),
                GridPosition { x: 1, y: 1 },
                DreamJournal {
                    last_dream: Some(Dream {
                        content: "dreamed of flying pigs".to_string(),
                        tick: 100,
                        impact: 0.1,
                        is_nightmare: false,
                    }),
                    history: vec![],
                },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Dreamer"));
        assert!(full_text.contains("Dream:"));
        assert!(full_text.contains("flying pigs"));
    }

    #[test]
    fn test_inspector_render_rock_purity() {
        use crate::layer1::purity::PurityMap;
        use crate::layer1::terrain::{TerrainGrid, TerrainType};

        let mut world = World::new();
        // Setup Rock tile
        let mut tiles = vec![TerrainType::Grass; 100];
        tiles[55] = TerrainType::Rock; // (5, 5)
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });

        // Setup PurityMap
        let mut purity_map = PurityMap::default();
        purity_map.set_override(5, 5, 0.85);
        world.insert_resource(purity_map);

        // Select Tile
        world.insert_resource(Selection::default());
        world.resource_mut::<Selection>().select_tile(5, 5);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Rock"));
        assert!(full_text.contains("Purity: 85%"));
    }

    #[test]
    fn test_inspector_render_pop_biocompatibility() {
        use crate::layer1::biocompatibility::Biocompatibility;
        use crate::layer1::pop::PopName;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Pop,
                PopName("Cade".to_string()),
                GridPosition { x: 2, y: 3 },
                Needs::default(),
                Biocompatibility { value: 0.85 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Cade"));
        assert!(full_text.contains("Bio-Comp: 85%"));
    }

    #[test]
    fn test_inspector_render_machine_spirit() {
        use crate::layer1::building::BuildingType;
        use crate::layer1::rituals::{MachineSpirit, Quirk, QuirkType};

        let mut world = World::new();
        world.insert_resource(Selection::default());
        let entity = world
            .spawn((
                Building {
                    building_type: BuildingType::AncientReactor,
                },
                MachineSpirit { anger: 80.0 },
                Quirk {
                    quirk_type: QuirkType::Glitchy,
                },
                GridPosition { x: 0, y: 0 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Spirit Anger: 80%"));
        assert!(full_text.contains("Glitchy"));
    }

    #[test]
    fn test_inspector_render_colony_stats_skyglow() {
        use crate::layer1::environment::light_pollution::SkyGlow;

        let mut world = World::new();
        world.insert_resource(Selection::default()); // SelectionTarget::None triggers render_colony_stats
        world.insert_resource(ColonyResources::default());
        world.insert_resource(SkyGlow { global_level: 42.5 });

        let backend = TestBackend::new(40, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Sky Glow"));
        assert!(full_text.contains("42.5"));
    }

    #[test]
    fn test_inspector_render_observatory() {
        use crate::layer1::building::BuildingType;
        use crate::layer1::environment::light_pollution::SkyGlow;
        use crate::layer1::observatory::Observatory;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        world.insert_resource(SkyGlow { global_level: 15.5 });

        let entity = world
            .spawn((
                Building {
                    building_type: BuildingType::Observatory,
                },
                Observatory { efficiency: 84.5 },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Observatory"));
        assert!(full_text.contains("Lens Efficiency"));
        assert!(full_text.contains("84% (Pollution: 15.5)"));
    }

    #[test]
    fn test_inspector_render_nocturnal_fauna() {
        use crate::layer1::environment::light_pollution::SkyGlow;
        use crate::layer1::fauna::NocturnalFauna;

        let mut world = World::new();
        world.insert_resource(Selection::default());
        world.insert_resource(SkyGlow { global_level: 20.0 });

        let entity = world
            .spawn((
                NocturnalFauna { aggression: 2.0 },
                GridPosition { x: 5, y: 5 },
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Agitated by Light"));
        assert!(full_text.contains("Aggression: 2.0"));
    }

    #[test]
    fn test_inspector_render_dietary_history() {
        use crate::layer1::items::ItemType;
        use crate::layer1::palette_fatigue::DietaryHistory;
        use crate::layer1::pop::PopName;

        let mut world = World::new();
        world.insert_resource(Selection::default());

        let mut history = DietaryHistory::default();
        history.recent_meals.push_back(ItemType::Potato);
        history.recent_meals.push_back(ItemType::Meat);

        let entity = world
            .spawn((
                Pop,
                PopName("Gourmand".to_string()),
                GridPosition { x: 1, y: 1 },
                history,
            ))
            .id();

        world.resource_mut::<Selection>().select_entity(entity);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                render_inspector(f, f.area(), &world);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for c in &buffer.content {
            full_text.push_str(c.symbol());
        }

        assert!(full_text.contains("Gourmand"));
        assert!(full_text.contains("Recent Meals:"));
        assert!(full_text.contains("Potato"));
        assert!(full_text.contains("Meat"));
    }
}
