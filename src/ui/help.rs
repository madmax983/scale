//! Field Guide — onboarding notes for the newly landed keeper.
//!
//! The Field Guide is a shell pane (plugin type `field-guide`, palette
//! command "Open Field Guide") that teaches a new player what the map
//! glyphs mean, what to do in the first ten minutes, how air and vacuum
//! work, what the drones and the system above are doing, which keys do
//! what — and that the weird stuff is intentional.
//!
//! Content rules (do not break these when editing):
//! * Every glyph in the legend comes from the map renderer's own
//!   getters ([`crate::ui::map::get_terrain_char`] and friends), so the
//!   legend can never drift from what actually renders.
//! * Every key mentioned here is bound in [`crate::ui::input`] or the
//!   shell runtime. If a binding does not exist, the guide does not
//!   invent it — it teaches the command palette instead.
//! * Numbers (starting stores, season timing) come from setup/balance
//!   code, not from vibes.

use bevy_ecs::prelude::*;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::layer1::{
    AnomalyType, BuildingType, Chronicle, DesignationType, EventImportance, FaunaType, FloraType,
    MaterialType, Needs, ResourceType, TerrainType,
};
use crate::ui::map::{
    get_anomaly_char, get_building_char, get_designation_char, get_fauna_char, get_flora_char,
    get_pop_display, get_resource_char, get_terrain_char, get_terrain_color,
};

/// Scroll state for the Field Guide pane.
///
/// Lives in the world (like `TechUiState`) so the shell plugin stays a
/// thin adapter: rendering reads it, key events mutate it.
#[derive(Resource, Default)]
pub struct FieldGuideUiState {
    /// Index of the first guide line shown at the top of the pane.
    pub scroll: u16,
    /// The largest scroll offset that can still show guide content,
    /// as computed by the renderer from the WRAPPED row count at the
    /// pane's real width (see [`guide_max_scroll`]). The renderer
    /// cannot publish it here -- both app loops hold an immutable
    /// world borrow across rendering, so render must never mutate
    /// the world (item 122) -- so the shell plugin caches the bound
    /// itself, clamps keys against the cache, and mirrors the value
    /// into this field from the key handler. Zero until the first
    /// key press after the first render.
    pub max_scroll: u16,
}

impl FieldGuideUiState {
    /// Move the scroll offset by `delta` lines, clamped to `[0, max_scroll]`.
    pub fn scroll_by(&mut self, delta: i32, max_scroll: u16) {
        let next = i32::from(self.scroll) + delta;
        self.scroll = u16::try_from(next.clamp(0, i32::from(max_scroll))).unwrap_or(0);
    }

    /// Jump back to the first line.
    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    /// Jump so the last line is visible.
    pub fn scroll_to_bottom(&mut self, max_scroll: u16) {
        self.scroll = max_scroll;
    }
}

/// How far a PageUp/PageDown press moves the guide.
pub const PAGE_SCROLL_LINES: i32 = 10;

/// The Field Guide's true scroll bound for a pane whose bordered
/// block has the given inner width and height.
///
/// The guide renders with `Wrap { trim: false }`, so at narrow pane
/// widths one logical line occupies several visual rows. The bound
/// therefore comes from the WRAPPED row count at the real content
/// width -- counted by ratatui itself, since `Paragraph::line_count`
/// runs the same `WordWrapper` the renderer uses -- minus the rows
/// that fit on screen. Counting unwrapped lines instead strands
/// every wrapped row past `lines - visible` (item 103).
#[must_use]
pub fn guide_max_scroll(lines: &[Line<'static>], inner_width: u16, inner_height: u16) -> u16 {
    if inner_width == 0 || inner_height == 0 {
        return 0;
    }
    let wrapped = Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(inner_width);
    u16::try_from(wrapped)
        .unwrap_or(u16::MAX)
        .saturating_sub(inner_height)
}

/// Marker text identifying the seeded guide entries in the Chronicle.
///
/// Used to keep seeding idempotent: if any chronicle event already
/// mentions the Field Guide, the entries are not added again.
const GUIDE_MARKER: &str = "Field Guide";

/// The guide entries seeded into a new colony's Chronicle.
///
/// In the game's own voice: one pointer at the guide itself, one set of
/// first orders, one promise that the weird stuff is coming. Added at
/// tick 0 by world setup, right after the founding entry — see
/// [`seed_field_guide_entries`].
pub const GUIDE_CHRONICLE_ENTRIES: [(&str, EventImportance); 3] = [
    (
        "The last keeper left a Field Guide open on the console: what the map's glyphs mean, what to do first, and every key that matters. Ctrl+K — 'Open Field Guide'.",
        EventImportance::Major,
    ),
    (
        "First orders, per the guide: keep both potato farms staffed, watch Food in the Info pane, and raise Housing before winter asks its questions.",
        EventImportance::Standard,
    ),
    (
        "Three origins will make planetfall before the year turns. The Chronicle will name them as they arrive. It forgets nothing.",
        EventImportance::Standard,
    ),
];

/// Seed the Field Guide entries into the colony's Chronicle.
///
/// Called once by world setup for a NEW colony, alongside
/// `initial_chronicle_event`. It cannot double-fire on save/load: the
/// save snapshot stores the whole Chronicle and restore replaces the
/// resource wholesale, so a loaded colony simply gets its original
/// entries back. The marker check additionally makes repeated calls
/// harmless.
pub fn seed_field_guide_entries(world: &mut World) {
    let mut chronicle = world.resource_mut::<Chronicle>();
    if chronicle
        .events
        .iter()
        .any(|event| event.text.contains(GUIDE_MARKER))
    {
        return;
    }
    for (text, importance) in GUIDE_CHRONICLE_ENTRIES {
        chronicle.add_event(0, text.to_string(), importance);
    }
}

/// Build the full Field Guide as styled lines.
///
/// Pure and world-free so it can be unit-tested directly. Wrapping to
/// the pane width happens at render time.
#[must_use]
pub fn guide_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(body("Notes for the newly landed, left by the last keeper."));
    lines.push(body(
        "You keep this colony. Nobody is coming to explain — so it falls to these pages.",
    ));
    lines.push(blank());
    legend_section(&mut lines);
    first_minutes_section(&mut lines);
    vacuum_section(&mut lines);
    layer2_drones_section(&mut lines);
    controls_section(&mut lines);
    grind_section(&mut lines);
    weird_stuff_section(&mut lines);
    lines.push(blank());
    lines.push(body(
        "Good luck, keeper. Losing is a story the Chronicle tells very well — but not today.",
    ));
    lines
}

fn legend_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("WHAT AM I LOOKING AT?"));
    lines.push(body(
        "The big pane is the colony map: one character per tile of world.",
    ));
    lines.push(body(
        "Your colonists are the faces. Everything else is ground, shelter, supplies — or trouble.",
    ));
    lines.push(blank());
    lines.push(subheader("— The ground —"));
    for (terrain, label) in [
        (TerrainType::Grass, "grass"),
        (TerrainType::Dirt, "dirt"),
        (TerrainType::Water, "water"),
        (TerrainType::Tree, "tree — chop it (C) for wood"),
        (TerrainType::Sapling, "sapling — a tree, eventually"),
        (TerrainType::Shrub, "shrub"),
        (TerrainType::Rock, "rock — mine it (M) for stone"),
        (
            TerrainType::DeepRock,
            "deep rock — the mountain, unconvinced",
        ),
        (TerrainType::Path, "path — much walked-on ground"),
        (TerrainType::Bridge, "bridge"),
        (TerrainType::Crater, "crater — something fell here"),
        (TerrainType::Ash, "ash — something burned here"),
        (
            TerrainType::MagmaRock,
            "magma rock — hot. It hurts. Keep off",
        ),
        (
            TerrainType::SporeBloom,
            "spore-bloom — toxic. Admire from a distance",
        ),
        (
            TerrainType::Artifact,
            "ancient artifact — old, gold, worth a look",
        ),
        (TerrainType::Void, "the void — no ground, no air, no colony"),
    ] {
        lines.push(legend(
            get_terrain_char(terrain),
            get_terrain_color(terrain),
            label,
        ));
    }
    lines.push(blank());
    lines.push(subheader("— The people —"));
    let (happy, happy_color) = get_pop_display(&needs(1.0));
    let (struggling, struggling_color) = get_pop_display(&needs(0.45));
    let (starving, starving_color) = get_pop_display(&needs(0.1));
    lines.push(legend(happy, happy_color, "colonist — fed and fine"));
    lines.push(legend(
        struggling,
        struggling_color,
        "colonist — struggling",
    ));
    lines.push(legend(
        starving,
        starving_color,
        "colonist — starving. Hunger is the need that kills",
    ));
    lines.push(legend(
        "V",
        Color::Magenta,
        "visitor — not yours; passing through",
    ));
    lines.push(legend(
        "🎓",
        Color::Yellow,
        "mentor — teaching another colonist",
    ));
    lines.push(blank());
    lines.push(subheader("— Shelter & work —"));
    for (building, label) in [
        (
            BuildingType::Lander,
            "the Lander — your ship, and first stockpile",
        ),
        (
            BuildingType::Housing,
            "housing — a real bed. Morale notices the difference",
        ),
        (
            BuildingType::Farm,
            "farm — grows food. (The tidy green ♣; spore-bloom is purple.)",
        ),
        (BuildingType::Stockpile, "stockpile — where hauling ends up"),
        (
            BuildingType::LifeSupport,
            "life support — makes the shelter's air breathable. Guard it",
        ),
        (BuildingType::Well, "well — water in, colony alive"),
        (BuildingType::Heater, "heater / solar panel"),
        (BuildingType::Hospital, "hospital"),
        (BuildingType::Generator, "generator"),
        (BuildingType::TradeDepot, "trade depot"),
        (
            BuildingType::Grave,
            "grave — someone the Chronicle remembers",
        ),
        (
            BuildingType::Wall,
            "wall — yes, the same glyph as rock. Context is everything",
        ),
    ] {
        lines.push(legend(
            get_building_char(building),
            crate::ui::map::get_building_color(building, MaterialType::default()),
            label,
        ));
    }
    lines.push(blank());
    lines.push(subheader("— On the ground —"));
    lines.push(body(format!(
        "  {} food    {} wood    {} stone",
        get_resource_char(ResourceType::Food),
        get_resource_char(ResourceType::Wood),
        get_resource_char(ResourceType::Stone),
    )));
    lines.push(body(format!(
        "  {} metal   {} planks   {} rations",
        get_resource_char(ResourceType::Metal),
        get_resource_char(ResourceType::Planks),
        get_resource_char(ResourceType::Rations),
    )));
    lines.push(blank());
    lines.push(subheader("— Marks & trouble —"));
    lines.push(legend(
        get_designation_char(DesignationType::Mine),
        Color::Red,
        "marked for mining — the mark warms from red to green as work goes",
    ));
    lines.push(legend(
        get_designation_char(DesignationType::Chop),
        Color::Red,
        "marked for chopping",
    ));
    lines.push(legend(
        get_designation_char(DesignationType::Demolish),
        Color::Red,
        "marked for demolition",
    ));
    lines.push(legend(
        get_fauna_char(FaunaType::Wolf),
        Color::Red,
        "wolf — it is not a pet",
    ));
    lines.push(legend(
        get_fauna_char(FaunaType::SpaceRat),
        Color::Rgb(105, 105, 105),
        "space rat",
    ));
    lines.push(legend(
        get_flora_char(FloraType::XenoMoss),
        Color::Rgb(0, 100, 0),
        "xeno-moss — the planet, growing back",
    ));
    lines.push(legend(
        get_anomaly_char(AnomalyType::Ruins),
        Color::Cyan,
        "ruins — old things, worth surveying",
    ));
    lines.push(legend(
        get_anomaly_char(AnomalyType::Geode),
        Color::Magenta,
        "geode",
    ));
    lines.push(legend(
        "^",
        Color::Rgb(255, 100, 0),
        "fire — self-explanatory, urgent",
    ));
    lines.push(body(
        "Colors matter: the same letter in a different color is a different thing.",
    ));
    lines.push(body(
        "Unsure about a tile? Click it — the Inspector pane names it. Right-click or Esc clears the selection.",
    ));
    lines.push(blank());
}

fn first_minutes_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("YOUR FIRST TEN MINUTES"));
    lines.push(body(
        "You landed with 5 founders, 10 food, 15 wood, 5 stone, and 2 tools.",
    ));
    lines.push(body(
        "Your shelter is already standing: metal walls, an airlock, the Lander, life support, a stockpile — and two potato farms.",
    ));
    lines.push(body(
        "Potatoes, because they endure winter. Trust the potatoes.",
    ));
    lines.push(body(
        "Ten food is a landing snack, not a supply line. Keep the farms staffed and watch Food in the Info pane — it is the number that decides whether this story is short.",
    ));
    lines.push(body(
        "Faces tell the truth faster than numbers: a struggling face is a warning, a starving face is an alarm with legs.",
    ));
    lines.push(body(
        "Build Housing early. 'Housing 0/5' in the Info pane means everyone sleeps rough, and morale keeps the receipts. Press B to build.",
    ));
    lines.push(body(
        "Grow with designations: C chops trees for wood, M mines rock for stone. Enter marks one corner of a rectangle; Enter again marks the other.",
    ));
    lines.push(body(
        "Winter is the last quarter of every year, and the first winter is the exam. Housing, heat, and a food buffer are the whole syllabus.",
    ));
    lines.push(blank());
}

fn vacuum_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("THE AIR IS THE HULL"));
    lines.push(body(
        "There is no sky here. Outside the sealed hull the pressure is zero — the planet does not hold air for you, the walls do.",
    ));
    lines.push(body(
        "Air comes from Life Support, and Life Support runs on power: the Lander makes it, a Power Pole carries it, and a broken or unpowered chain means the colony is breathing whatever is already in the rooms.",
    ));
    lines.push(body(
        "A colonist caught outside — chopping, mining, fleeing — starts suffocating within moments and loses health every heartbeat. A trip outside and back is an errand; a long work party out there is a funeral with extra steps, roughly a hundred heartbeats of grace.",
    ));
    lines.push(body(
        "Panic makes it worse: a frightened colonist runs, and out here running usually means running out of air. Keep the scared ones inside until they settle.",
    ));
    lines.push(body(
        "An EVA suit changes the arithmetic. A suited colonist outside breathes from the bottle on their back — one day of air for every day outside — and takes no harm while the bottle lasts. Bottles hold 50 days, 100 once your researchers work out Closed-Cycle Breathing, and 200 with Expedition Systems. Colonists dress on their own before stepping out for work when a suit is available — the Tailor sews pool suits from cloth and metal — and hang the suit back up when they come in. The Chronicle calls out a bottle at a quarter, and again when it runs dry. Migrants walk in already wearing expedition rigs, their bottles partly spent by the crossing. A suit is a round trip, not a home: when the bottle empties, the old rules return.",
    ));
    lines.push(body(
        "Watch the status bar: it counts anyone standing in vacuum and names the thinnest air any colonist is breathing. When someone is suffocating, the Chronicle says so — once, by name, while there is still time to matter.",
    ));
    lines.push(body(
        "Life Support power speaks in three voices. A brief brownout — the grid flickering under load — says nothing: a flicker is weather, not news. A unit that stays dark earns one Chronicle warning, and the status bar counts the damage — 'LifeSupport 1/2 unpowered' means one of two units is down and the survivor is carrying the colony alone. A unit dark for a long stretch escalates: a STILL-without-power Chronicle entry, because an outage that long is no longer an accident, it is the colony's condition. Answer all three the same way: more generation, or a shorter pole line to the unit that went dark.",
    ));
    lines.push(body(
        "New arrivals walk in from the beacon with nothing but their skills. The roster marks them NEW ARRIVAL while they are still finding their feet.",
    ));
    lines.push(blank());
}

fn layer2_drones_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("LAYER 2 & DRONES"));
    lines.push(body(
        "The colony's second workforce does not breathe. Drones are battery-driven hands: no hunger, no morale, no sleep. They work until the battery says otherwise, and then they go and charge.",
    ));
    lines.push(body(
        "They choose their own work, in this order, always: charge, then repair, then haul. A drone below a fifth of its battery drops everything — even a repair half-welded — and makes for the nearest powered DroneHub.",
    ));
    lines.push(body(format!(
        "On the colony map the hub wears a '{}', and a powered hub replaces lost drones one at a time, up to three per hub.",
        get_building_char(BuildingType::DroneHub)
    )));
    lines.push(body(
        "Repairs outrank hauling, and a building standing in vacuum outranks every other repair — then the most damaged, then the nearest. That ordering is mercy with a serial number: every patch a drone welds outside is a patch no colonist has to make in a suit, breathing bottle air. Drones are tireless, not faster: they weld at a colonist's pace, standing in vacuum forever.",
    ));
    lines.push(body(
        "They will not touch ancient structures, nor anything the colony has flagged to leave unmaintained. Some things are not theirs to fix.",
    ));
    lines.push(body(
        "The hub only feeds a drone while the hub itself has power, and a drone answers through that link. Leave the hub dead long enough — thousands of heartbeats — and the drone forgets whose it is. That is a feral drone: it repairs nothing, hoards what it can carry, and attacks colonists. Keep the power on.",
    ));
    lines.push(body(
        "Above the colony, the system turns whether you watch it or not. Press Tab to flip between the colony and the system, or open the map as a pane: Ctrl+K, then 'Open System Map'. The System Survey workspace keeps it in front of you. The system shows itself only to a colony with a powered Command Center — lose the deck's power and the view snaps back to the dirt.",
    ));
    lines.push(body(
        "What you will see: the sun at the center, the system's bodies on their orbits, and fleets — drawn at the world they orbit, or strung between worlds while they cross. A grey '?' is a contact your sensors cannot name, which out there is its own kind of sentence.",
    ));
    lines.push(body(
        "Read the traffic like weather. Traders come down to trade. A blockade means nobody is trading with anyone. Repo fleets collect on debts the colony left unpaid, and they collect in buildings. Orphan fleets drift, looking for a new allegiance; ghost fleets keep older appointments. And some things out there eat worlds, given time.",
    ));
    lines.push(body(
        "Here is the part the last keeper underlined twice: the fleets fly themselves. No helm out there takes your orders — your levers are down here: stores laid in, debts paid, suits sewn, drones charged, walls standing. The system map is not a helm. It is how you see it coming.",
    ));
    lines.push(blank());
}

fn controls_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("CONTROLS"));
    lines.push(control(
        "Camera",
        "WASD or arrow keys. Click a tile to select it; right-click or Esc clears the selection; the Inspector explains it.",
    ));
    lines.push(control(
        "Space",
        "pause / resume.  1 · 2 · 3 — simulation speed (1x · 3x · 5x); . cycles speed.",
    ));
    lines.push(control(
        "Ctrl+K",
        "the command palette. Every pane, workspace, and command lives there, searchable. When in doubt, Ctrl+K.",
    ));
    lines.push(control(
        "H or L",
        "Chronicle pane.  T — Tech Tree (↑/↓ choose, Enter unlocks).",
    ));
    lines.push(control(
        "B",
        "build mode: WASD/arrows move the cursor, Tab cycles buildings, M cycles material, Enter or Space places, Esc leaves.",
    ));
    lines.push(control(
        "M · C · X",
        "designate mining / chopping / demolition: Enter anchors a corner, Enter again confirms the rectangle, Backspace cancels a mark, right-click clears the selection, Esc leaves.",
    ));
    lines.push(control(
        "Tab",
        "Colony view ↔ System view, once the system beyond is revealed.",
    ));
    lines.push(control(
        "P",
        "possess the selected colonist and walk in their skin. WASD moves them. Esc lets go.",
    ));
    lines.push(control(
        "Workspaces",
        "Colony Ops, System Survey, Director — switch from the palette.",
    ));
    lines.push(control(
        "Layout mode",
        "palette: 'Enter Layout Mode'. Then: hjkl focus · Shift+HJKL move · [ ] resize · s/v split · d close · Enter interact · Esc back to play.",
    ));
    lines.push(control(
        "Q",
        "quit. Esc clears the selection; with nothing selected, Esc opens Layout mode instead — Q is the quit key.",
    ));
    lines.push(control(
        "This guide",
        "Ctrl+K, then 'Open Field Guide'. ↑/↓ scrolls it while it is focused.",
    ));
    lines.push(blank());
}

fn grind_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("THE GRIND"));
    lines.push(body(
        "Misery here is structural, not a mood: wages that don't reach, bunks that don't exist, paste for every meal. It accrues while those conditions hold — and it lingers long after they lift.",
    ));
    lines.push(body(
        "Inspect a colonist: the inspector names their top grievances in their own words. The status bar counts the simmering when enough of the colony is there.",
    ));
    lines.push(body(
        "Counterplays are the conditions themselves: housing for everyone, variety on the plate, rest that is actually rest, wages that reach the food.",
    ));
    lines.push(blank());
}

fn weird_stuff_section(lines: &mut Vec<Line<'static>>) {
    lines.push(header("THE WEIRD STUFF"));
    lines.push(body(
        "Origins — three arrive every run, staggered over the first year. The menu lets you pick one, or choose 'Surprise me.'",
    ));
    lines.push(body(
        "A fallen sovereign. A corsair. A lawbound automaton. The landing roster is never quite sane, and that is the point.",
    ));
    lines.push(body(
        "Possession — any colonist can be ridden (select them, press P). Origins are stranger from the inside.",
    ));
    lines.push(body(
        "The Chronicle — the colony's memory: foundings, firsts, feuds, deaths, and rumors from the deep. If it happened, it is written there. Press H.",
    ));
    lines.push(body(
        "And above it all, the System turns. Press Tab to look up — once the colony has earned the view.",
    ));
}

/// Render the Field Guide into `area` (used by the shell pane plugin).
pub fn render_field_guide(frame: &mut Frame, area: Rect, world: &World) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let block = Block::default()
        .title(" Field Guide ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Black));
    // Wrap width and visible height are the block's INNER dimensions.
    let inner = block.inner(chunks[0]);

    let lines = guide_lines();
    // The bound must count WRAPPED rows at the inner width (see
    // `guide_max_scroll`); the unwrapped line count strands the tail
    // of the guide (item 103). The caller (the shell plugin) caches
    // the returned bound and the key handler clamps against that
    // same value; the plugin also mirrors it into
    // `FieldGuideUiState::max_scroll` from the key handler.
    let max_scroll = guide_max_scroll(&lines, inner.width, inner.height);
    let scroll = world
        .get_resource::<FieldGuideUiState>()
        .map_or(0, |state| state.scroll)
        .min(max_scroll);

    frame.render_widget(Clear, chunks[0]);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, chunks[0]);

    let footer = Paragraph::new(Line::from(Span::styled(
        " ↑/↓ scroll · PgUp/PgDn page · lost? Ctrl+K opens every command ",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(footer, chunks[1]);

    max_scroll
}

/// A colonist whose worst need is `worst` (drives the pop legend glyphs).
fn needs(worst: f32) -> Needs {
    Needs {
        hunger: worst,
        rest: 1.0,
        leisure: 1.0,
        hygiene: 1.0,
    }
}

fn header(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn subheader(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn body(text: impl Into<String>) -> Line<'static> {
    Line::from(text.into())
}

fn control(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<12}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(description.to_string()),
    ])
}

fn legend(glyph: &str, color: Color, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(glyph.to_string(), Style::default().fg(color)),
        Span::raw(format!("  {label}")),
    ])
}

fn blank() -> Line<'static> {
    Line::from("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn all_text() -> String {
        guide_lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn guide_covers_all_four_sections() {
        let text = all_text();
        assert!(
            text.contains("WHAT AM I LOOKING AT?"),
            "legend section missing"
        );
        assert!(
            text.contains("YOUR FIRST TEN MINUTES"),
            "first-minutes section missing"
        );
        assert!(text.contains("CONTROLS"), "controls section missing");
        assert!(
            text.contains("THE WEIRD STUFF"),
            "weird-stuff section missing"
        );
    }

    #[test]
    fn guide_teaches_vacuum_and_air() {
        // Item 77 Phase 2: the guide must teach the death chain — vacuum
        // outside the hull, the power chain behind Life Support, and
        // suffocation — in plain language, before it happens to the reader.
        let text = all_text();
        assert!(
            text.contains("THE AIR IS THE HULL"),
            "vacuum section missing"
        );
        assert!(text.contains("pressure is zero"), "vacuum danger missing");
        assert!(text.contains("Life Support"), "air source missing");
        assert!(text.contains("Power Pole"), "power chain missing");
        assert!(text.contains("suffocating"), "suffocation warning missing");
    }

    #[test]
    fn guide_teaches_unpowered_lifesupport_telegraphs() {
        // Item 99: the guide was silent on what the power telegraphs
        // mean — a keeper could not learn that a flicker is forgiven,
        // a sustained outage is chronicled, a long one escalates, or
        // that the status bar names a partial outage by count.
        let text = all_text();
        assert!(
            text.contains("1/2 unpowered"),
            "guide must teach the partial-outage status segment"
        );
        assert!(
            text.contains("brownout"),
            "guide must distinguish brownout flicker from a real outage"
        );
        assert!(
            text.contains("STILL"),
            "guide must name the sustained-outage escalation"
        );
    }

    #[test]
    fn guide_teaches_layer2_and_drones() {
        // Layer 2 legibility pass: the guide must teach the drone
        // priority ladder, the powered-hub rule, the feral warning, and
        // how to open the system map — systems a keeper (above all a
        // Layer 2 Ready keeper, who starts with a live fleet) is handed
        // with no other explanation anywhere in the game.
        let text = all_text();
        assert!(
            text.contains("LAYER 2 & DRONES"),
            "layer 2 & drones section missing"
        );
        assert!(
            text.contains("charge, then repair, then haul"),
            "drone priority ladder missing"
        );
        assert!(text.contains("DroneHub"), "drone hub missing");
        assert!(
            text.contains("the hub wears a"),
            "drone hub glyph teaching missing"
        );
        assert!(text.contains("feral"), "feral drone warning missing");
        assert!(
            text.contains("Open System Map"),
            "guide must name the system map palette command"
        );
        assert!(
            text.contains("System Survey"),
            "guide must name the system survey workspace"
        );
        assert!(
            text.contains("Command Center"),
            "system view power gate missing"
        );
        assert!(
            text.contains("fly themselves"),
            "guide must be honest that fleets are not player-flown"
        );
    }

    #[test]
    fn guide_legend_uses_renderer_glyphs() {
        let text = all_text();
        // The legend is built from the renderer's own getters; if any of
        // these go missing, the guide no longer teaches the real map.
        assert!(
            text.contains(get_terrain_char(TerrainType::Tree)),
            "tree glyph missing"
        );
        assert!(
            text.contains(get_terrain_char(TerrainType::Water)),
            "water glyph missing"
        );
        assert!(
            text.contains(get_terrain_char(TerrainType::Rock)),
            "rock glyph missing"
        );
        assert!(
            text.contains(get_building_char(BuildingType::Housing)),
            "housing glyph missing"
        );
        assert!(
            text.contains(get_building_char(BuildingType::LifeSupport)),
            "life support glyph missing"
        );
        assert!(
            text.contains(get_pop_display(&needs(1.0)).0),
            "happy pop glyph missing"
        );
        assert!(
            text.contains(get_pop_display(&needs(0.1)).0),
            "starving pop glyph missing"
        );
        assert!(
            text.contains(get_designation_char(DesignationType::Mine)),
            "mine designation glyph missing"
        );
        assert!(
            text.contains(get_resource_char(ResourceType::Food)),
            "food item glyph missing"
        );
        assert!(
            text.contains(get_fauna_char(FaunaType::Wolf)),
            "wolf glyph missing"
        );
    }

    #[test]
    fn guide_teaches_palette_and_real_bindings() {
        let text = all_text();
        assert!(
            text.contains("Ctrl+K"),
            "guide must teach the command palette"
        );
        assert!(
            text.contains("Open Field Guide"),
            "guide must name its own palette command"
        );
        assert!(text.contains("possess"), "guide must teach possession");
    }

    #[test]
    fn guide_teaches_right_click_and_esc_selection_clear() {
        // Item 98: right-click support (item 93) and the layered Esc
        // (item 90) must be taught where a player will actually read
        // them, worded only as far as src/ui/input.rs proves:
        // route_mouse_input clears Selection on a right click in both
        // Normal and DesignationMode (never selecting / never starting
        // a drag), and route_root_input clears the selection on Esc in
        // Normal — while a bare Esc with nothing selected flips the
        // shell into Layout mode instead of quitting.
        let text = all_text();
        assert!(
            text.contains("Click a tile to select it; right-click or Esc clears the selection"),
            "camera line must teach right-click / Esc selection clear"
        );
        assert!(
            text.contains("Right-click or Esc clears the selection"),
            "legend line must teach right-click / Esc selection clear"
        );
        assert!(
            text.contains("right-click clears the selection"),
            "designation line must teach right-click selection clear"
        );
        assert!(
            text.contains("with nothing selected, Esc opens Layout mode instead"),
            "Q line must not claim a second Esc quits; it opens Layout mode"
        );
        assert!(
            !text.contains("press it again to quit"),
            "stale Q wording (second Esc quits) must be gone"
        );
    }

    #[test]
    fn render_field_guide_paints_title_and_sections() {
        let world = World::new();
        let backend = TestBackend::new(90, 34);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_field_guide(frame, frame.area(), &world);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for cell in buffer.content() {
            text.push_str(cell.symbol());
        }
        assert!(text.contains("Field Guide"), "pane title should render");
        assert!(
            text.contains("WHAT AM I LOOKING AT?"),
            "first section should be visible without scrolling"
        );
        assert!(text.contains("Ctrl+K"), "footer hint should render");
    }

    #[test]
    fn scroll_offsets_clamp_to_bounds() {
        let mut state = FieldGuideUiState::default();
        state.scroll_by(-5, 40);
        assert_eq!(state.scroll, 0, "scroll must not go below zero");
        state.scroll_by(12, 40);
        assert_eq!(state.scroll, 12);
        state.scroll_by(100, 40);
        assert_eq!(state.scroll, 40, "scroll must clamp at max_scroll");
        state.scroll_to_top();
        assert_eq!(state.scroll, 0);
        state.scroll_to_bottom(40);
        assert_eq!(state.scroll, 40);
    }

    /// Render the guide at the given terminal size with the world's
    /// current scroll state and return the whole buffer as text.
    fn render_guide_text(world: &World, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_field_guide(frame, frame.area(), world);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for cell in buffer.content() {
            text.push_str(cell.symbol());
        }
        text
    }

    #[test]
    fn guide_tail_is_reachable_at_narrow_width() {
        // Item 103 (Mark, WASM playtest): at a narrow pane width the
        // guide's wrapped rows far outnumber its raw line count, and
        // the scroll bound derived from the UNWRAPPED count stranded
        // everything below the EVA section. Pin the scroll past the
        // end -- the renderer clamps to its bound, and that bound must
        // reach the guide's final line.
        let mut world = World::new();
        world.insert_resource(FieldGuideUiState {
            scroll: u16::MAX,
            ..Default::default()
        });
        let text = render_guide_text(&world, 44, 14);
        assert!(
            text.contains("Good luck, keeper"),
            "at max scroll the final guide line must be visible"
        );
        assert!(
            text.contains("today."),
            "at max scroll the final wrapped row must be visible"
        );
    }

    #[test]
    fn guide_max_scroll_matches_empirical_wrapped_rows() {
        // The shared bound must equal (wrapped rows - visible rows)
        // with the wrapped rows counted EMPIRICALLY from a real
        // render, so if ratatui's wrap counting ever drifts from its
        // rendering, this fails instead of the guide's tail quietly
        // becoming unreachable again.
        let world = World::new();
        let backend = TestBackend::new(44, 900);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_field_guide(frame, frame.area(), &world);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut last_text_row = 0u16;
        for y in 1..buffer.area.height - 2 {
            let row_has_text = (1..43).any(|x| buffer[(x, y)].symbol().trim() != "");
            if row_has_text {
                last_text_row = y;
            }
        }
        // Pane 44x14: content chunk 13 rows, block inner 42 wide x 11.
        assert_eq!(
            guide_max_scroll(&guide_lines(), 42, 11),
            last_text_row - 11,
            "guide_max_scroll must equal empirical wrapped rows minus visible rows"
        );
        // A wide pane wraps far less, so its bound is strictly smaller.
        assert!(guide_max_scroll(&guide_lines(), 118, 30) < last_text_row - 11);
    }

    #[test]
    fn seed_adds_guide_entries_exactly_once() {
        let mut world = World::new();
        world.insert_resource(Chronicle::default());

        seed_field_guide_entries(&mut world);
        seed_field_guide_entries(&mut world);

        let chronicle = world.resource::<Chronicle>();
        assert_eq!(
            chronicle.events.len(),
            GUIDE_CHRONICLE_ENTRIES.len(),
            "seeding twice must not duplicate the guide entries"
        );
        assert!(
            chronicle.events[0].text.contains("Field Guide"),
            "first seeded entry should point at the guide"
        );
        assert_eq!(chronicle.events[0].importance, EventImportance::Major);
    }

    #[test]
    fn fresh_setup_world_carries_guide_entries() {
        let world = crate::setup::setup_world_with_config(crate::setup::SetupConfig {
            headless: true,
            ..Default::default()
        });
        let chronicle = world.resource::<Chronicle>();
        let guide_entries = chronicle
            .events
            .iter()
            .filter(|event| event.text.contains("Field Guide"))
            .count();
        assert_eq!(
            guide_entries, 1,
            "exactly one entry should name the Field Guide"
        );
        assert!(
            chronicle
                .events
                .iter()
                .any(|event| event.text.contains("First orders, per the guide")),
            "first-orders entry should be seeded"
        );
        assert!(
            chronicle
                .events
                .iter()
                .any(|event| event.text.contains("Three origins will make planetfall")),
            "origins entry should be seeded"
        );
    }
}
