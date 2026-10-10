#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops
)]
use bevy_ecs::prelude::*;
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph},
};
use std::collections::HashMap;

use crate::layer1::fire::Fire;
use crate::layer1::{
    Anomaly, AnomalyType, BuildMode, Building, BuildingType, Designation, DesignationMode,
    DesignationType, Fauna, FaunaType, Flora, FloraType, ForestryProgress, GridPosition, Material,
    MaterialType, Mentorship, MiningProgress, Needs, ResourceItem, ResourceType, TerrainGrid,
    TerrainType, Viewport, Visitor,
};
use crate::shared::time::WallTime;
use crate::ui::seasonal_gfx;

/// Represents a renderable entity on the map.
///
/// This enum abstracts the different types of entities that can appear on the map grid.
/// It carries just enough data to determine the character and color to render.
#[derive(Clone, Copy, Debug)]
pub enum RenderEntity {
    /// A player designation (e.g., [`DesignationType::Mine`], [`DesignationType::Chop`]).
    /// Includes optional progress (0.0 to 1.0) for active tasks.
    Designation(DesignationType, Option<f32>),
    /// A constructed building (e.g., [`BuildingType::Farm`], [`BuildingType::Housing`]).
    /// The `bool` flag indicates if the building is "Fragile" (Jury-Rigged).
    Building(BuildingType, MaterialType, bool),
    /// A hostile animal (e.g., Wolf, Space Rat).
    Fauna(FaunaType),
    /// Antagonistic flora (e.g., `XenoMoss`).
    Flora(FloraType),
    /// A colonist ([`crate::layer1::pop::Pop`]), carrying a display character and color based on status.
    Pop(&'static str, Color),
    /// An anomaly scan target (e.g., Ruins, Flora).
    Anomaly(AnomalyType),
    /// A loose resource item on the ground (e.g., [`ResourceType::Wood`]).
    Item(ResourceType),
    /// An active fire spreading across the map.
    Fire,
    /// The Blob (hazard).
    Blob,
    /// A visual particle effect (e.g., dust, sparks).
    Particle(char, Color),
}

impl RenderEntity {
    /// Returns the rendering priority (higher is drawn on top).
    ///
    /// The z-ordering is:
    /// 1. **Particles** (Top): Visual effects overlay everything.
    /// 2. **Fire**: Overlays designations.
    /// 3. **Designations**: Overlays like "Mine" need to be visible over buildings.
    /// 4. **Buildings**: Walls and structures cover pops.
    /// 5. **Pops**: Colonists move around on the ground.
    /// 6. **Anomalies**: Special sites (under pops).
    /// 7. **Items** (Bottom): Resources sit on the floor.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::ui::map::RenderEntity;
    /// use scale::layer1::DesignationType;
    /// use ratatui::style::Color;
    ///
    /// let des = RenderEntity::Designation(DesignationType::Mine, None);
    /// let pop = RenderEntity::Pop("☺", Color::Yellow);
    ///
    /// assert!(des.priority() > pop.priority());
    /// ```
    #[must_use]
    pub const fn priority(&self) -> u8 {
        match self {
            Self::Particle(_, _) => 7,
            Self::Fire | Self::Blob => 6,
            Self::Designation(_, _) => 5,
            Self::Building(_, _, _) => 4,
            Self::Fauna(_) | Self::Pop(_, _) => 3,
            Self::Flora(_) => 2,
            Self::Anomaly(_) => 1,
            Self::Item(_) => 0,
        }
    }
}

/// Cache for renderable entities to avoid repeated allocations and iterations.
///
/// Rendering the map requires determining what is at each (x, y) coordinate.
/// Iterating the entire ECS world for every tile would be O(N * W * H), which is too slow.
///
/// Instead, we iterate the ECS *once* per frame and populate this hash map:
/// `GridPosition -> RenderEntity`.
///
/// This reduces the per-tile lookup to O(1), making rendering O(W * H).
#[derive(Resource, Default)]
pub struct RenderCache {
    /// Cached entity data.
    pub entities: HashMap<GridPosition, RenderEntity>,
}

/// Updates the [`RenderCache`] by iterating the world once.
///
/// This system must run before rendering to ensure the cache reflects the latest frame state.
/// It respects the priority defined in [`RenderEntity::priority()`].
pub fn update_render_cache(world: &mut World) {
    let mut cache = world.remove_resource::<RenderCache>().unwrap_or_default();

    cache.entities.clear();

    for e in world.iter_entities() {
        if let Some(pos) = e.get::<GridPosition>() {
            // Check for Designation
            if let Some(designation) = e.get::<Designation>() {
                let progress = e.get::<MiningProgress>().map_or_else(
                    || e.get::<ForestryProgress>().map(|p| p.current / p.max),
                    |p| Some(p.current / p.max),
                );

                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Designation(designation.designation_type, progress),
                );
            }

            // Check for Building
            if let Some(building) = e.get::<Building>() {
                let material = e
                    .get::<Material>()
                    .map_or_else(MaterialType::default, |m| m.0);
                let is_fragile = e.contains::<crate::layer1::structure::Fragile>();
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Building(building.building_type, material, is_fragile),
                );
            }

            // Check for Fauna
            if let Some(fauna) = e.get::<Fauna>() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Fauna(fauna.fauna_type),
                );
            }

            // Check for Flora
            if let Some(flora) = e.get::<Flora>() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Flora(flora.flora_type),
                );
            }

            // Check for Visitor (Specific override)
            if e.get::<Visitor>().is_some() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Pop("V", Color::Magenta),
                );
            }
            // Check for Pop (via Needs)
            else if let Some(needs) = e.get::<Needs>() {
                let (mut text, color) = get_pop_display(needs);

                if e.get::<Mentorship>().is_some() {
                    text = "🎓";
                }

                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Pop(text, color),
                );
            }

            // Check for Fire
            if e.get::<Fire>().is_some() {
                insert_if_higher_priority(&mut cache.entities, *pos, RenderEntity::Fire);
            }

            // Check for Blob
            if e.get::<crate::layer1::entities::blob::Blob>().is_some() {
                insert_if_higher_priority(&mut cache.entities, *pos, RenderEntity::Blob);
            }

            // Check for Particle
            if let Some(particle) = e.get::<crate::layer1::Particle>() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Particle(particle.char, particle.color),
                );
            }

            // Check for Anomaly
            if let Some(anomaly) = e.get::<Anomaly>() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Anomaly(anomaly.anomaly_type),
                );
            }

            // Check for ResourceItem
            if let Some(item) = e.get::<ResourceItem>() {
                insert_if_higher_priority(
                    &mut cache.entities,
                    *pos,
                    RenderEntity::Item(item.resource_type),
                );
            }
        }
    }

    world.insert_resource(cache);
}

fn insert_if_higher_priority(
    map: &mut HashMap<GridPosition, RenderEntity>,
    pos: GridPosition,
    entity: RenderEntity,
) {
    map.entry(pos)
        .and_modify(|e| {
            if entity.priority() > e.priority() {
                *e = entity;
            }
        })
        .or_insert(entity);
}

/// Context for rendering the map layer.
///
/// Bundles all the read-only references needed to draw the map, avoiding function signature bloat.
pub struct MapRenderContext<'a> {
    /// The area to render into.
    pub area: Rect,
    /// The terrain grid.
    pub terrain: &'a TerrainGrid,
    /// The water grid.
    pub water: &'a crate::layer1::water::WaterGrid,
    /// The viewport.
    pub viewport: &'a Viewport,
    /// Map of entity positions to their render data.
    pub entities_data: &'a HashMap<GridPosition, RenderEntity>,
    /// Current build mode state (cursor position, selected building, selected material, valid placement).
    pub build_mode: Option<(GridPosition, BuildingType, MaterialType, bool)>,
    /// Current designation mode state (cursor position, selected tool, valid placement, drag start).
    pub designation_mode: Option<(GridPosition, DesignationType, bool, Option<GridPosition>)>,
    /// The current season, if available (for visual overlays).
    pub season: Option<crate::layer1::seasons::Season>,
    /// The current wall time for UI animations.
    pub wall_time: f32,
}

impl Clone for MapRenderContext<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for MapRenderContext<'_> {}

/// Builds a vector of text lines to render the terrain within the given area.
///
/// Ignores entities and only draws the base terrain layer.
#[must_use]
pub fn build_terrain_spans(
    area: Rect,
    terrain: &TerrainGrid,
    viewport: &Viewport,
    season: Option<crate::layer1::seasons::Season>,
) -> Vec<Line<'static>> {
    let mut spans: Vec<Line> = Vec::with_capacity(area.height as usize);

    for screen_y in 0..area.height {
        let world_y = viewport.y.saturating_add(i32::from(screen_y));
        let mut line_spans = Vec::with_capacity(area.width as usize);

        for screen_x in 0..area.width {
            let world_x = viewport.x.saturating_add(i32::from(screen_x));

            let (text, color) =
                if let (Ok(ux), Ok(uy)) = (usize::try_from(world_x), usize::try_from(world_y)) {
                    terrain.get(ux, uy).map_or((" ", Color::Black), |tile| {
                        let color = seasonal_gfx::get_texture_override(tile, season)
                            .unwrap_or_else(|| get_terrain_color(tile));
                        (get_terrain_char(tile), color)
                    })
                } else {
                    (" ", Color::Black)
                };

            line_spans.push(Span::styled(text, Style::default().fg(color)));
        }
        spans.push(Line::from(line_spans));
    }
    spans
}

/// Builds a vector of text lines to render the map layer (terrain, pops, buildings, cursor).
///
/// This is the core rendering logic for the map view. It iterates screen coordinates
/// and resolves the character/color for each cell based on:
/// 1. Active cursors (Build/Designate).
/// 2. Drag selection rectangles.
/// 3. Cached entities ([`RenderCache`]).
/// 4. Base terrain.
pub fn build_map_layer_spans(ctx: MapRenderContext<'_>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::with_capacity(ctx.area.height as usize);

    for screen_y in 0..ctx.area.height {
        let world_y = ctx.viewport.y.saturating_add(i32::from(screen_y));
        let line_spans = build_map_layer_line(&ctx, world_y);
        lines.push(Line::from(line_spans));
    }
    lines
}

fn build_map_layer_line(ctx: &MapRenderContext<'_>, world_y: i32) -> Vec<Span<'static>> {
    let mut line_spans = Vec::with_capacity(ctx.area.width as usize);

    for screen_x in 0..ctx.area.width {
        let world_x = ctx.viewport.x.saturating_add(i32::from(screen_x));

        // Build mode cursor (highest priority)
        if let Some(span) = render_build_mode_cursor(ctx, world_x, world_y) {
            line_spans.push(span);
            continue;
        }

        // Designation mode cursor (highest priority, shared with build mode)
        if let Some(span) = render_designation_mode(ctx, world_x, world_y) {
            line_spans.push(span);
            continue;
        }

        // Check for entity in cache
        if let Some(entity) = ctx.entities_data.get(&GridPosition {
            x: world_x,
            y: world_y,
        }) {
            if let Some(span) = render_cached_entity(ctx, entity) {
                line_spans.push(span);
                continue;
            }
        }

        // Otherwise render terrain
        line_spans.push(render_base_terrain(ctx, world_x, world_y));
    }
    line_spans
}

fn render_build_mode_cursor(
    ctx: &MapRenderContext<'_>,
    world_x: i32,
    world_y: i32,
) -> Option<Span<'static>> {
    let (_, selected, material, can_place) = ctx
        .build_mode
        .filter(|(cursor, _, _, _)| cursor.x == world_x && cursor.y == world_y)?;

    // Ludwig: Pulsing cursor effect
    let pulse = (ctx.wall_time * 5.0).sin().abs(); // 0.0 to 1.0
    let intensity = (50.0 + 100.0 * pulse) as u8; // 50 to 150

    let bg = if can_place {
        Color::Rgb(0, intensity + 50, 0) // Green pulse
    } else {
        Color::Rgb(intensity + 100, 0, 0) // Red pulse
    };

    let text = get_building_char(selected);
    let fg = get_building_color(selected, material);
    Some(Span::styled(text, Style::default().fg(fg).bg(bg)))
}

fn render_designation_mode(
    ctx: &MapRenderContext<'_>,
    world_x: i32,
    world_y: i32,
) -> Option<Span<'static>> {
    let (cursor, selected, can_place, drag_start) = ctx.designation_mode?;

    if cursor.x == world_x && cursor.y == world_y {
        // Ludwig: Pulsing cursor effect
        let pulse = (ctx.wall_time * 5.0).sin().abs();
        let intensity = (50.0 + 100.0 * pulse) as u8;

        let bg = if can_place {
            Color::Rgb(0, intensity + 50, 0)
        } else {
            Color::Rgb(intensity + 100, 0, 0)
        };

        let text = get_designation_char(selected);
        return Some(Span::styled(text, Style::default().fg(Color::White).bg(bg)));
    }

    // Rectangle preview: highlight tiles from drag_start to cursor
    if let Some(start) = drag_start {
        let min_x = start.x.min(cursor.x);
        let max_x = start.x.max(cursor.x);
        let min_y = start.y.min(cursor.y);
        let max_y = start.y.max(cursor.y);

        if world_x >= min_x && world_x <= max_x && world_y >= min_y && world_y <= max_y {
            // Render terrain underneath with a highlight background
            let (text, fg) =
                if let (Ok(ux), Ok(uy)) = (usize::try_from(world_x), usize::try_from(world_y)) {
                    ctx.terrain.get(ux, uy).map_or((" ", Color::Black), |tile| {
                        let color = seasonal_gfx::get_texture_override(tile, ctx.season)
                            .unwrap_or_else(|| get_terrain_color(tile));
                        (get_terrain_char(tile), color)
                    })
                } else {
                    (" ", Color::Black)
                };

            // Ludwig: Gentle pulsing highlight for selection area
            let pulse = (ctx.wall_time * 2.0).sin().abs(); // Slower pulse
            let blue = (50.0 + 30.0 * pulse) as u8;

            return Some(Span::styled(
                text,
                Style::default().fg(fg).bg(Color::Rgb(50, 50, blue)),
            ));
        }
    }

    None
}

fn render_cached_entity(
    ctx: &MapRenderContext<'_>,
    entity: &RenderEntity,
) -> Option<Span<'static>> {
    match entity {
        RenderEntity::Particle(c, color) => {
            Some(Span::styled(c.to_string(), Style::default().fg(*color)))
        }
        RenderEntity::Fire => Some(Span::styled(
            "^",
            Style::default()
                .fg(Color::Rgb(255, 100, 0))
                .add_modifier(Modifier::BOLD),
        )),
        RenderEntity::Blob => Some(Span::styled(
            "%",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )),
        RenderEntity::Designation(tool, progress) => {
            let color = progress.map_or(Color::Red, |p| {
                if p < 0.33 {
                    Color::Red
                } else if p < 0.66 {
                    Color::Yellow
                } else {
                    Color::Green
                }
            });

            Some(Span::styled(
                get_designation_char(*tool),
                Style::default().fg(color),
            ))
        }
        RenderEntity::Building(b, m, fragile) => {
            let mut fg = get_building_color(*b, *m);
            if *fragile {
                // Pulsing red effect for Jury-Rigged buildings
                let pulse = (ctx.wall_time * 8.0).sin();
                if pulse > 0.0 {
                    fg = Color::LightRed;
                }
            }
            Some(Span::styled(get_building_char(*b), Style::default().fg(fg)))
        }
        RenderEntity::Fauna(ft) => Some(Span::styled(
            get_fauna_char(*ft),
            Style::default().fg(get_fauna_color(*ft)),
        )),
        RenderEntity::Flora(ft) => Some(Span::styled(
            get_flora_char(*ft),
            Style::default().fg(get_flora_color(*ft)),
        )),
        RenderEntity::Pop(text, color) => Some(Span::styled(*text, Style::default().fg(*color))),
        RenderEntity::Anomaly(a) => Some(Span::styled(
            get_anomaly_char(*a),
            Style::default().fg(get_anomaly_color(*a)),
        )),
        RenderEntity::Item(r) => Some(Span::styled(
            get_resource_char(*r),
            Style::default().fg(get_resource_color(*r)),
        )),
    }
}

fn render_base_terrain(ctx: &MapRenderContext<'_>, world_x: i32, world_y: i32) -> Span<'static> {
    let (text, color) =
        if let (Ok(ux), Ok(uy)) = (usize::try_from(world_x), usize::try_from(world_y)) {
            ctx.terrain.get(ux, uy).map_or((" ", Color::Black), |tile| {
                let mut color = seasonal_gfx::get_texture_override(tile, ctx.season)
                    .unwrap_or_else(|| get_terrain_color(tile));

                // Hydration visualization
                let hydration = ctx.water.get(ux, uy);
                #[allow(
                    clippy::collapsible_if,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::suboptimal_flops
                )]
                if hydration > 0 && tile != TerrainType::Water {
                    if let Color::Rgb(r, g, b) = color {
                        // Mix with Blue (80, 140, 255) based on hydration level (0-100)
                        let factor = f32::from(hydration) / 200.0; // Max 50% mix
                        let r = (f32::from(r) * (1.0 - factor) + 80.0 * factor) as u8;
                        let g = (f32::from(g) * (1.0 - factor) + 140.0 * factor) as u8;
                        let b = (f32::from(b) * (1.0 - factor) + 255.0 * factor) as u8;
                        color = Color::Rgb(r, g, b);
                    }
                }

                (get_terrain_char(tile), color)
            })
        } else {
            (" ", Color::Black)
        };

    Span::styled(text, Style::default().fg(color))
}

/// Render terrain grid, buildings, and pops to the given frame area with viewport offset.
pub fn render_map_layer(frame: &mut Frame, ctx: MapRenderContext<'_>) {
    let area = ctx.area;
    let lines = build_map_layer_spans(ctx);
    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

/// Renders the full map with borders and simulation state.
///
/// This is the top-level function for drawing the map component. It:
/// 1. Draws the border block.
/// 2. Assembles the [`MapRenderContext`] from World resources.
/// 3. Delegates pixel generation to [`render_map_layer`].
pub fn render_map(frame: &mut Frame, area: Rect, world: &World) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Colony ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Render terrain inside
    let terrain = world.resource::<TerrainGrid>();
    let water = world.resource::<crate::layer1::water::WaterGrid>();
    let viewport = world.resource::<Viewport>();
    let build_mode = world.resource::<BuildMode>();
    let designation_mode = world.resource::<DesignationMode>();
    let render_cache = world.resource::<RenderCache>();
    let season = world
        .get_resource::<crate::layer1::seasons::SeasonState>()
        .map(|s| s.current_season);

    let wall_time = world.resource::<WallTime>().0;

    // Fetch ScreenShake
    let shake_offset = world
        .get_resource::<crate::layer1::map::ScreenShake>()
        .map_or((0, 0), |s| s.offset);

    let effective_viewport = Viewport {
        x: viewport.x + shake_offset.0,
        y: viewport.y + shake_offset.1,
    };

    // Build mode cursor info
    let build_mode_cursor = if build_mode.active {
        let can_place =
            crate::layer1::can_place_building(world, build_mode.cursor.x, build_mode.cursor.y);
        Some((
            build_mode.cursor,
            build_mode.selected,
            build_mode.selected_material,
            can_place,
        ))
    } else {
        None
    };

    // Designation mode cursor info
    let designation_mode_cursor = if designation_mode.active {
        let can = crate::layer1::can_designate(
            world,
            designation_mode.cursor.x,
            designation_mode.cursor.y,
            designation_mode.tool,
        );
        Some((
            designation_mode.cursor,
            designation_mode.tool,
            can,
            designation_mode.drag_start,
        ))
    } else {
        None
    };

    let ctx = MapRenderContext {
        area: inner,
        terrain,
        water,
        viewport: &effective_viewport,
        entities_data: &render_cache.entities,
        build_mode: build_mode_cursor,
        designation_mode: designation_mode_cursor,
        season,
        wall_time,
    };

    render_map_layer(frame, ctx);
}

/// Renders the terrain grid to the provided frame.
pub fn render_terrain(
    frame: &mut Frame,
    area: Rect,
    terrain: &TerrainGrid,
    viewport: &Viewport,
    season: Option<crate::layer1::seasons::Season>,
) {
    let spans = build_terrain_spans(area, terrain, viewport, season);
    let paragraph = Paragraph::new(spans);
    frame.render_widget(paragraph, area);
}

// --- Visual Helpers ---

/// Returns the ASCII character for the given terrain type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_terrain_char;
/// use scale::layer1::TerrainType;
///
/// assert_eq!(get_terrain_char(TerrainType::Grass), ".");
/// assert_eq!(get_terrain_char(TerrainType::Water), "~");
/// ```
#[must_use]
pub const fn get_terrain_char(terrain: TerrainType) -> &'static str {
    match terrain {
        TerrainType::Grass => ".",
        TerrainType::Dirt => ",",
        TerrainType::Rock => "#",
        TerrainType::Water => "~",
        TerrainType::Tree => "↑",
        TerrainType::Path => "░",
        TerrainType::Shrub => "\"",
        TerrainType::Sapling => "t",
        TerrainType::DeepRock => "▓",
        TerrainType::Crater => "o",
        TerrainType::IndestructibleStump => "I",
        TerrainType::MagmaRock => "≈",
        TerrainType::SporeBloom => "♣",
        TerrainType::Artifact => "Ω",
        TerrainType::FaultLine(true) => "≈",
        TerrainType::FaultLine(false) => "–",
        TerrainType::Bridge => "=",
        TerrainType::Void => " ",
    }
}

/// Returns the color for the given terrain type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_terrain_color;
/// use scale::layer1::TerrainType;
/// use ratatui::style::Color;
///
/// // Water is blue-ish
/// assert_eq!(get_terrain_color(TerrainType::Water), Color::Rgb(80, 140, 255));
/// ```
#[must_use]
pub const fn get_terrain_color(terrain: TerrainType) -> Color {
    match terrain {
        TerrainType::Grass => Color::Rgb(100, 200, 100),
        TerrainType::Dirt => Color::Rgb(205, 150, 80),
        TerrainType::Rock => Color::Rgb(160, 160, 170),
        TerrainType::Water => Color::Rgb(80, 140, 255),
        TerrainType::Tree => Color::Rgb(50, 180, 50),
        TerrainType::Path => Color::Rgb(180, 130, 70),
        TerrainType::Shrub => Color::Rgb(80, 180, 80),
        TerrainType::Sapling => Color::Rgb(100, 220, 100),
        TerrainType::DeepRock => Color::Rgb(100, 100, 110),
        TerrainType::Crater => Color::Rgb(80, 80, 90),
        TerrainType::IndestructibleStump => Color::White,
        TerrainType::MagmaRock => Color::Rgb(255, 60, 20),
        TerrainType::SporeBloom => Color::Rgb(150, 60, 200),
        TerrainType::Artifact => Color::Rgb(255, 215, 0), // Gold
        TerrainType::FaultLine(true) => Color::Rgb(255, 60, 20),
        TerrainType::FaultLine(false) => Color::Rgb(100, 100, 110),
        TerrainType::Bridge => Color::Rgb(150, 100, 50),
        TerrainType::Void => Color::Black,
    }
}

/// Returns the display character for a building type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_building_char;
/// use scale::layer1::BuildingType;
///
/// assert_eq!(get_building_char(BuildingType::Housing), "⌂");
/// ```
#[must_use]
pub const fn get_building_char(building: BuildingType) -> &'static str {
    match building {
        BuildingType::Housing => "⌂",
        BuildingType::Farm | BuildingType::TrashCannon => "♣",
        BuildingType::Well => "U",
        BuildingType::Stockpile | BuildingType::Vent => "≡",
        BuildingType::Smokehouse => "♨",
        BuildingType::LumberMill => "L",
        BuildingType::StoneMason => "M",
        BuildingType::Smelter => "S",
        BuildingType::Smithy => "⚒",
        BuildingType::Tavern => "T",
        BuildingType::Library => "K",
        BuildingType::Plantation => "P",
        BuildingType::Weaver => "W",
        BuildingType::Tailor => "t",
        BuildingType::FlowerBed | BuildingType::PersonalGarden => "*",
        BuildingType::Statue => "I",
        BuildingType::Hospital | BuildingType::Gate => "+",
        BuildingType::Landfill => "%",
        BuildingType::Grave => "†",
        BuildingType::TradeDepot => "$",
        BuildingType::Generator => "⚡",
        BuildingType::PowerPole => "|",
        BuildingType::Wall => "#",
        BuildingType::Window => "□",
        BuildingType::Tower | BuildingType::Observatory | BuildingType::Office => "O",
        BuildingType::AncientReactor | BuildingType::Refinery => "R",
        BuildingType::AncientFabricator => "F",
        BuildingType::Greenhouse => "G",
        BuildingType::PersonalShed => "s",
        BuildingType::PersonalShrine => "☗",
        BuildingType::ConveyorBelt => ">",
        BuildingType::Hopper => "V",
        BuildingType::HydroponicsBay => "Y",
        BuildingType::LifeSupport => "♼",
        BuildingType::Airlock => "⌷",
        BuildingType::Battery => "B",
        BuildingType::Heater | BuildingType::SolarPanel => "☼",
        BuildingType::ServerBank => "▥",
        BuildingType::Lander => "Λ",
        BuildingType::CommandCenter => "C",
        BuildingType::AICore => "A",
        BuildingType::DroneHub => "D",
        BuildingType::CryoPod => "❄",
        BuildingType::AuroralCollector => "Ψ",
        BuildingType::AtmosphericProcessor => "@",
        BuildingType::GeneBank => "🧬",
        BuildingType::CloneVat => "⚗",
        BuildingType::HypnoPod => "H",
        BuildingType::Shower => "🚿",
        BuildingType::Recycler => "♻",
        BuildingType::BulletinBoard => "B",
        BuildingType::HoloProjector => "O",
        BuildingType::Nanoforge => "N",
        BuildingType::School => "S",
        BuildingType::MediaStation => "M",
        BuildingType::Spaceport => "P",
        BuildingType::Mainframe => "M",
        BuildingType::CommsRelay => "C",
        BuildingType::Billboard => "B",
        BuildingType::BiomassExtractor => "X",
        BuildingType::TetherArray => "\u{2301}",
        BuildingType::HarpoonLauncher => "\u{2693}",
    }
}

/// Returns the display color for a building type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_building_color;
/// use scale::layer1::{BuildingType, MaterialType};
/// use ratatui::style::Color;
///
/// assert_eq!(get_building_color(BuildingType::Farm, MaterialType::default()), Color::Rgb(218, 165, 32));
/// ```
#[must_use]
pub const fn get_building_color(building: BuildingType, material: MaterialType) -> Color {
    if building.supports_material() {
        match material {
            MaterialType::Wood => Color::Rgb(139, 90, 43), // Brown
            MaterialType::Stone => Color::Rgb(169, 169, 169), // DarkGray
            MaterialType::Metal => Color::Cyan,
            MaterialType::Gold => Color::Rgb(255, 215, 0), // Gold
        }
    } else {
        match building {
            BuildingType::Housing | BuildingType::PersonalShed => Color::Rgb(139, 90, 43), // Fallback (should be covered above)
            BuildingType::Farm => Color::Rgb(218, 165, 32), // Goldenrod
            BuildingType::Well | BuildingType::Tailor => Color::Blue,
            BuildingType::Stockpile
            | BuildingType::Wall
            | BuildingType::Gate
            | BuildingType::Tower
            | BuildingType::PersonalShrine
            | BuildingType::Hopper
            | BuildingType::Vent => Color::Rgb(169, 169, 169), // Fallback (should be covered above)
            BuildingType::Smokehouse => Color::Rgb(200, 200, 200), // Smoky
            BuildingType::LumberMill => Color::Rgb(205, 133, 63),  // Peru
            BuildingType::StoneMason => Color::Rgb(119, 136, 153), // LightSlateGray
            BuildingType::Smelter | BuildingType::AncientReactor => Color::Rgb(255, 69, 0), // Red-Orange
            BuildingType::Smithy | BuildingType::PowerPole | BuildingType::Airlock => {
                Color::Rgb(192, 192, 192)
            } // Silver
            BuildingType::Tavern | BuildingType::FlowerBed | BuildingType::AICore => Color::Magenta,
            BuildingType::Library
            | BuildingType::Window
            | BuildingType::AncientFabricator
            | BuildingType::Observatory
            | BuildingType::ConveyorBelt
            | BuildingType::LifeSupport
            | BuildingType::Battery
            | BuildingType::DroneHub
            | BuildingType::AuroralCollector
            | BuildingType::GeneBank
            | BuildingType::Shower => Color::Cyan,
            BuildingType::AtmosphericProcessor
            | BuildingType::Recycler
            | BuildingType::ServerBank
            | BuildingType::CloneVat
            | BuildingType::HypnoPod => Color::Rgb(0, 255, 100), // Toxic/Data Green
            BuildingType::Plantation | BuildingType::PersonalGarden => Color::Green,
            BuildingType::Weaver | BuildingType::Statue | BuildingType::Lander => Color::White,
            BuildingType::Hospital => Color::Red,
            BuildingType::Landfill => Color::Rgb(105, 105, 105), // DimGray
            BuildingType::Grave => Color::Rgb(128, 128, 128),    // Gray
            BuildingType::TradeDepot | BuildingType::SolarPanel => Color::Yellow,
            BuildingType::Generator => Color::Rgb(255, 215, 0), // Gold
            BuildingType::Refinery => Color::Rgb(100, 200, 255), // Chemical Blue
            BuildingType::Greenhouse | BuildingType::HydroponicsBay => Color::Rgb(200, 255, 255), // Glass/Cyan
            BuildingType::TrashCannon => Color::Rgb(100, 100, 100),
            BuildingType::Heater => Color::Rgb(255, 140, 0), // DarkOrange
            BuildingType::CommandCenter | BuildingType::Office | BuildingType::CryoPod => {
                Color::Rgb(0, 0, 255)
            } // Blue
            BuildingType::BulletinBoard => Color::Rgb(210, 180, 140), // Tan
            BuildingType::HoloProjector => Color::Rgb(200, 200, 255),
            BuildingType::Nanoforge => Color::Rgb(150, 0, 255),
            BuildingType::School => Color::Blue,
            BuildingType::MediaStation => Color::Cyan,
            BuildingType::Spaceport => Color::LightBlue,
            BuildingType::Mainframe => Color::LightMagenta,
            BuildingType::CommsRelay => Color::LightCyan,
            BuildingType::Billboard => Color::Magenta,
            BuildingType::BiomassExtractor => Color::Rgb(139, 0, 0),
            BuildingType::TetherArray => Color::Rgb(120, 180, 255),
            BuildingType::HarpoonLauncher => Color::Rgb(255, 180, 120),
        }
    }
}

/// Returns the display string for a designation tool.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_designation_char;
/// use scale::layer1::DesignationType;
///
/// assert_eq!(get_designation_char(DesignationType::Mine), "%");
/// ```
#[must_use]
pub const fn get_designation_char(tool: DesignationType) -> &'static str {
    match tool {
        DesignationType::Mine => "%",
        DesignationType::Demolish => "X",
        DesignationType::Chop => "/",
        DesignationType::Repair => "+",
        DesignationType::SetZone(_) => "Z",
        DesignationType::Tame => "♥",
        DesignationType::ClearFlora => "F",
        DesignationType::JuryRig => "J",
        DesignationType::Cannibalize => "C",
        DesignationType::Destroy => "D",
        DesignationType::CollectSample => "S",
        DesignationType::Consume => "E",
        DesignationType::ExcavateDust => "U",
    }
}

/// Returns the display string for a loose resource type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_resource_char;
/// use scale::layer1::ResourceType;
///
/// assert_eq!(get_resource_char(ResourceType::Wood), "t");
/// ```
#[must_use]
pub const fn get_resource_char(resource: ResourceType) -> &'static str {
    match resource {
        ResourceType::Food => "%",
        ResourceType::Wood => "t",
        ResourceType::Stone => "*",
        ResourceType::Ore => "o",
        ResourceType::Metal => "m",
        ResourceType::Planks => "=",
        ResourceType::Blocks => "■",
        ResourceType::Waste => "x",
        ResourceType::Rations => "R",
        ResourceType::Fuel => "F",
        ResourceType::Alcohol => "D",
        ResourceType::Scrap => "S",
        ResourceType::Tools => "T",
        ResourceType::BuildingPermit => "P",
        ResourceType::MemoryCore => "M",
        ResourceType::VoidAle => "V",
        ResourceType::HyperValuable => "$",
        ResourceType::BiologicalWaste => "B",
        ResourceType::NutrientPaste => "P",
        ResourceType::Organs => "Q",
    }
}

/// Returns the display color for a loose resource type.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_resource_color;
/// use scale::layer1::ResourceType;
/// use ratatui::style::Color;
///
/// assert_eq!(get_resource_color(ResourceType::Food), Color::Green);
/// ```
#[must_use]
pub const fn get_resource_color(resource: ResourceType) -> Color {
    match resource {
        ResourceType::Food => Color::Green,
        ResourceType::Wood => Color::Rgb(139, 69, 19), // SaddleBrown
        ResourceType::Stone => Color::Gray,
        ResourceType::Ore => Color::Rgb(165, 42, 42), // Brown
        ResourceType::Metal | ResourceType::Tools => Color::Cyan,
        ResourceType::Planks => Color::Yellow,
        ResourceType::Blocks | ResourceType::BuildingPermit => Color::White,
        ResourceType::Waste => Color::Rgb(85, 107, 47), // DarkOliveGreen
        ResourceType::Rations => Color::Rgb(255, 105, 180), // HotPink
        ResourceType::Fuel => Color::Rgb(255, 140, 0),  // DarkOrange
        ResourceType::Alcohol => Color::Rgb(148, 0, 211), // DarkViolet
        ResourceType::Scrap => Color::Rgb(105, 105, 105), // DimGray
        ResourceType::MemoryCore => Color::Rgb(148, 0, 211), // DarkViolet
        ResourceType::VoidAle => Color::Yellow,
        ResourceType::HyperValuable => Color::Rgb(255, 215, 0),
        ResourceType::BiologicalWaste => Color::Rgb(100, 100, 0),
        ResourceType::NutrientPaste => Color::Rgb(200, 200, 200), // Gold
        ResourceType::Organs => Color::Rgb(220, 20, 60), // Crimson
    }
}

const HEALTHY_THRESHOLD: f32 = 0.6;
const WARNING_THRESHOLD: f32 = 0.3;

/// Returns the character and color for rendering a pop.
///
/// Display is driven primarily by hunger (the only lethal need).
/// Leisure/rest affect mood but not survival, so they only downgrade
/// from happy to neutral — never to the "dying" indicator.
///
/// # Examples
///
/// ```
/// use scale::ui::map::get_pop_display;
/// use scale::layer1::Needs;
/// use ratatui::style::Color;
///
/// let happy = Needs { hunger: 1.0, rest: 1.0, leisure: 1.0, hygiene: 1.0 };
/// let (char, color) = get_pop_display(&happy);
/// assert_eq!(char, "☺");
/// assert_eq!(color, Color::Yellow);
/// ```
#[must_use]
pub fn get_pop_display(needs: &Needs) -> (&'static str, Color) {
    let worst = needs.worst();
    if worst > HEALTHY_THRESHOLD {
        ("☺", Color::Yellow)
    } else if worst > WARNING_THRESHOLD {
        ("☻", Color::Rgb(255, 165, 0)) // fed but tired/bored
    } else {
        ("☹", Color::Red) // starving
    }
}

/// Returns the display character for an anomaly.
#[must_use]
pub const fn get_anomaly_char(anomaly: AnomalyType) -> &'static str {
    match anomaly {
        AnomalyType::Ruins => "R",
        AnomalyType::StrangeFlora => "F",
        AnomalyType::Geode => "G",
    }
}

/// Returns the display color for an anomaly.
#[must_use]
pub const fn get_anomaly_color(anomaly: AnomalyType) -> Color {
    match anomaly {
        AnomalyType::Ruins => Color::Cyan,
        AnomalyType::StrangeFlora => Color::Green,
        AnomalyType::Geode => Color::Magenta,
    }
}

/// Returns the display character for a fauna type.
#[must_use]
pub const fn get_fauna_char(fauna: FaunaType) -> &'static str {
    match fauna {
        FaunaType::Wolf => "w",
        FaunaType::SpaceRat => "r",
        FaunaType::Mascot => "M",
    }
}

/// Returns the display color for a fauna type.
#[must_use]
pub const fn get_fauna_color(fauna: FaunaType) -> Color {
    match fauna {
        FaunaType::Wolf => Color::Red,
        FaunaType::SpaceRat => Color::Rgb(105, 105, 105), // DimGray
        FaunaType::Mascot => Color::Yellow,
    }
}

/// Returns the display character for a flora type.
#[must_use]
pub const fn get_flora_char(flora: FloraType) -> &'static str {
    match flora {
        FloraType::XenoMoss => "▒",
        FloraType::StrangleVines => "§",
        FloraType::SilentFlora => "♠",
        FloraType::FireWeed => "w",
        FloraType::Ironwood => "T",
        FloraType::Lumiflora => "❀",
    }
}

/// Returns the display color for a flora type.
#[must_use]
pub const fn get_flora_color(flora: FloraType) -> Color {
    match flora {
        FloraType::XenoMoss => Color::Rgb(0, 100, 0), // DarkGreen
        FloraType::StrangleVines => Color::Rgb(139, 0, 139), // DarkMagenta
        FloraType::SilentFlora => Color::Rgb(255, 255, 255), // White
        FloraType::FireWeed => Color::Rgb(255, 100, 0), // Orange-red
        FloraType::Ironwood => Color::Rgb(100, 100, 100), // Grey
        FloraType::Lumiflora => Color::Rgb(255, 182, 193), // Light Pink
    }
}
