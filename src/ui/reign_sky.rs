//! Reign & Sky panels: terminal UI for the headless-console-only mechanics.
//!
//! The swarm's week of landings (Propaganda Constellation, Degraded
//! God-Mind, Fallen Sovereign, adventurer possession) shipped with a
//! headless console interface only; `src/ui/` had zero references to any
//! of them. This module gives them first-class ratatui panels:
//!
//! - [`render_constellation_panel`]: satellite link status, the active sky
//!   message, its morale effect, and rival-hack exposure.
//! - [`render_godmind_panel`]: the uploaded Eternal Ruler, bit-rot,
//!   reign/edict bookkeeping, tribute monuments.
//! - [`render_sovereign_panel`]: legitimacy/melancholy meters, decree
//!   cooldown, in-flight decrees/edicts, commissioned artworks.
//! - [`render_reign_sky`]: all three stacked in one pane.
//! - [`render_possess_hud`]: adventurer-mode overlay (possessed pop readout
//!   + available actions), drawn over the fullscreen map while
//!   `UiState::suppress_global_ui` is set.
//! - [`build_reign_spans`]: compact status-bar segments for the same data.
//!
//! All queries are read-only; the sim is untouched.

use bevy_ecs::prelude::*;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::layer1::culture::corsair::{CorsairCaptain, CorsairState, SkiffHold, SkiffHull};
use crate::layer1::culture::sovereign::{
    DentedCrown, Sovereign, SovereignArtwork, SovereignState, EDICT_BANISHMENT_THRESHOLD,
    EDICT_FEAST_THRESHOLD, EDICT_SILENCE_THRESHOLD,
};
use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
use crate::layer1::diplomacy::factions::rivals::RivalColony;
use crate::layer1::direct_link::Possessed;
use crate::layer1::entities::pop::{PopName, Role};
use crate::layer1::{GridPosition, Health, Needs};
use crate::layer2::propaganda::{
    Constellation, SloganMessage, CONSTELLATION_MIN_SATELLITES,
    HACK_PROBABILITY_PER_TICK_PER_RIVAL,
};
use crate::layer3::diplomacy::god_mind::{
    GodMind, GodMindMonument, CENTURIES_TO_FULL_ROT, ROT_TICK_INTERVAL,
};

/// Width of the little ASCII progress bars used in these panels.
const BAR_WIDTH: usize = 12;

/// Render a `[████░░░░░░]` style bar for a 0.0..=1.0 value.
fn bar(value: f32, width: usize) -> String {
    let clamped = value.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let filled = (clamped * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

/// Render the Propaganda Constellation panel.
pub fn render_constellation_panel(frame: &mut Frame, area: Rect, world: &World) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Propaganda Constellation ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(constellation) = world.get_resource::<Constellation>() else {
        render_lines(
            frame,
            inner,
            vec![Line::styled(
                "no constellation uplink",
                Style::default().fg(Color::DarkGray),
            )],
        );
        return;
    };

    let linked = constellation.satellites.len();
    let is_night = world
        .get_resource::<DayNightCycle>()
        .is_some_and(|c| c.time_of_day == TimeOfDay::Night);
    let rivals = world
        .iter_entities()
        .filter(|e| e.get::<RivalColony>().is_some())
        .count();

    let mut lines = Vec::new();
    let link_style = if constellation.active() {
        Style::default().fg(Color::LightYellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("linked {linked}/{CONSTELLATION_MIN_SATELLITES} "),
            link_style,
        ),
        Span::styled(
            if constellation.active() {
                "ACTIVE"
            } else {
                "dark"
            },
            link_style.add_modifier(Modifier::BOLD),
        ),
    ]));

    match &constellation.message {
        SloganMessage::Hope(text) => {
            lines.push(Line::from(vec![
                Span::raw("sky: "),
                Span::styled(
                    format!("\"{text}\""),
                    Style::default()
                        .fg(Color::LightYellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            let effect = if is_night {
                "+0.35 morale (night)"
            } else {
                "+0.35 morale at night"
            };
            lines.push(Line::styled(
                effect,
                Style::default().fg(Color::Green),
            ));
        }
        SloganMessage::Despair(text) => {
            lines.push(Line::from(vec![
                Span::raw("sky: "),
                Span::styled(
                    format!("\"{text}\""),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::styled(
                "HACKED: −0.40 morale",
                Style::default().fg(Color::Red),
            ));
        }
        SloganMessage::Dark => {
            lines.push(Line::styled(
                "sky dark — link 5+ satellites",
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    lines.push(Line::styled(
        format!(
            "hack risk: {rivals} rival(s) × {HACK_PROBABILITY_PER_TICK_PER_RIVAL}/tick"
        ),
        Style::default().fg(Color::DarkGray),
    ));
    render_lines(frame, inner, lines);
}

/// Render the Degraded God-Mind reign panel.
pub fn render_godmind_panel(frame: &mut Frame, area: Rect, world: &World) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Degraded God-Mind ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mind: Option<GodMind> = world
        .iter_entities()
        .filter_map(|e| e.get::<GodMind>().cloned())
        .next();

    let Some(mind) = mind else {
        render_lines(
            frame,
            inner,
            vec![Line::styled(
                "no god-mind uploaded",
                Style::default().fg(Color::DarkGray),
            )],
        );
        return;
    };

    #[allow(clippy::cast_precision_loss)]
    let century = (mind.reign_ticks / ROT_TICK_INTERVAL) as u32;
    let rot_color = if mind.bit_rot >= 0.85 {
        Color::Red
    } else if mind.bit_rot >= 0.5 {
        Color::Yellow
    } else {
        Color::Cyan
    };

    let mut lines = vec![
        Line::from(vec![
            Span::raw("Eternal Ruler: "),
            Span::styled(
                mind.leader_name.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw(format!("bit-rot {} ", bar(mind.bit_rot, BAR_WIDTH))),
            Span::styled(
                format!("{:.0}%", mind.bit_rot * 100.0),
                Style::default().fg(rot_color),
            ),
            Span::styled(
                format!("  century {century}/{CENTURIES_TO_FULL_ROT}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(format!(
            "reign {} ticks · {} edicts · stability +{:.1}",
            mind.reign_ticks, mind.edict_count, mind.stability_bonus
        )),
    ];

    let check = |seen: bool| if seen { "✓" } else { "·" };
    lines.push(Line::from(vec![
        Span::raw("edict arc: "),
        Span::styled(
            format!("{} erratic  ", check(mind.seen_erratic_edict)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} bizarre tribute  ", check(mind.seen_bizarre_tribute)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} corrupted war", check(mind.seen_corrupted_war)),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    let monuments: Vec<GodMindMonument> = world
        .iter_entities()
        .filter_map(|e| e.get::<GodMindMonument>().cloned())
        .collect();
    if monuments.is_empty() {
        lines.push(Line::styled(
            "no tribute monuments",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        for monument in monuments.iter().take(3) {
            lines.push(Line::styled(
                format!(
                    "☖ \"{}\" (upkeep {:.1} stone)",
                    monument.name, monument.upkeep
                ),
                Style::default().fg(Color::Yellow),
            ));
        }
    }

    render_lines(frame, inner, lines);
}

/// Render the Fallen Sovereign reign panel.
pub fn render_sovereign_panel(frame: &mut Frame, area: Rect, world: &World) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Fallen Sovereign ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let state = world.get_resource::<SovereignState>();
    let sovereign_entity = state.and_then(|s| s.sovereign);
    let sovereign = sovereign_entity.and_then(|e| world.get::<Sovereign>(e));

    let Some(sovereign) = sovereign else {
        // No crowned pop — is the crown lying around?
        let crown: Option<GridPosition> = world
            .iter_entities()
            .filter(|e| e.get::<DentedCrown>().is_some())
            .filter_map(|e| e.get::<GridPosition>().copied())
            .next();
        let lines = match crown {
            Some(pos) => vec![Line::from(vec![
                Span::styled(
                    "a dented crown lies unclaimed",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!(" at ({}, {})", pos.x, pos.y),
                    Style::default().fg(Color::DarkGray),
                ),
            ])],
            None => vec![Line::styled(
                "no sovereign — the crown is lost",
                Style::default().fg(Color::DarkGray),
            )],
        };
        render_lines(frame, inner, lines);
        return;
    };

    let state = state.expect("sovereign implies SovereignState exists");
    let name = sovereign_entity
        .and_then(|e| world.get::<PopName>(e))
        .map_or_else(|| "the crowned".to_string(), |n| n.0.clone());

    let legitimacy_color = if sovereign.legitimacy >= 0.6 {
        Color::Green
    } else if sovereign.legitimacy >= 0.3 {
        Color::Yellow
    } else {
        Color::Red
    };
    let melancholy_color = if sovereign.melancholy >= EDICT_SILENCE_THRESHOLD {
        Color::Red
    } else if sovereign.melancholy >= EDICT_FEAST_THRESHOLD {
        Color::Magenta
    } else if sovereign.melancholy >= EDICT_BANISHMENT_THRESHOLD {
        Color::Yellow
    } else {
        Color::Cyan
    };

    let mut lines = vec![
        Line::from(vec![
            Span::raw("reign of "),
            Span::styled(
                name,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("legitimacy "),
            Span::styled(
                format!("{} {:.0}%", bar(sovereign.legitimacy, BAR_WIDTH), sovereign.legitimacy * 100.0),
                Style::default().fg(legitimacy_color),
            ),
        ]),
        Line::from(vec![
            Span::raw("melancholy "),
            Span::styled(
                format!("{} {:.0}%", bar(sovereign.melancholy, BAR_WIDTH), sovereign.melancholy * 100.0),
                Style::default().fg(melancholy_color),
            ),
        ]),
    ];

    if state.decree_cooldown > 0 {
        lines.push(Line::styled(
            format!("decree cooldown: {} ticks", state.decree_cooldown),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut active = Vec::new();
    if let Some(labor) = &state.labor {
        active.push(format!("labor decree ({}t)", labor.remaining));
    }
    if let Some(revel) = &state.revel {
        active.push(format!("revel ({}t)", revel.remaining));
    }
    if let Some(feast) = &state.feast {
        active.push(format!("grand feast ({}t)", feast.remaining));
    }
    if let Some(banishment) = &state.banishment {
        active.push(format!("banishment ({}t)", banishment.idle_remaining));
    }
    if active.is_empty() {
        lines.push(Line::styled(
            "no active decrees",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        lines.push(Line::styled(
            format!("active: {}", active.join(" · ")),
            Style::default().fg(Color::Green),
        ));
    }

    if state.fired_edicts.is_empty() {
        lines.push(Line::styled(
            format!(
                "edicts unlock at melancholy {:.0}/{:.0}/{:.0}",
                EDICT_BANISHMENT_THRESHOLD * 100.0,
                EDICT_FEAST_THRESHOLD * 100.0,
                EDICT_SILENCE_THRESHOLD * 100.0
            ),
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        let names: Vec<&str> = state
            .fired_edicts
            .iter()
            .map(|e| e.name())
            .collect();
        lines.push(Line::styled(
            format!("edicts fired: {}", names.join(", ")),
            Style::default().fg(Color::Magenta),
        ));
    }

    let artworks = world
        .iter_entities()
        .filter(|e| e.get::<SovereignArtwork>().is_some())
        .count();
    lines.push(Line::styled(
        format!("patron's gallery: {artworks} commissioned"),
        Style::default().fg(Color::DarkGray),
    ));

    render_lines(frame, inner, lines);
}

/// Render all three Reign & Sky panels stacked vertically.
///
/// Each panel gets a fixed-ish share; leftover space goes to the last one.
pub fn render_reign_sky(frame: &mut Frame, area: Rect, world: &World) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Min(8),
        ])
        .split(area);
    render_constellation_panel(frame, chunks[0], world);
    render_godmind_panel(frame, chunks[1], world);
    render_sovereign_panel(frame, chunks[2], world);
}

/// Render the adventurer-mode possession HUD.
///
/// Drawn as a bottom-anchored overlay while `suppress_global_ui` is set.
/// Renders nothing when no pop is possessed.
pub fn render_possess_hud(frame: &mut Frame, area: Rect, world: &World) {
    let entity = world
        .iter_entities()
        .filter(|e| e.get::<Possessed>().is_some())
        .map(|e| e.id())
        .next();
    let Some(entity) = entity else {
        return;
    };

    let name = world
        .get::<PopName>(entity)
        .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
    let role = world
        .get::<Role>(entity)
        .map_or_else(|| "—".to_string(), |r| format!("{r:?}"));
    let pos = world
        .get::<GridPosition>(entity)
        .map_or_else(|| "(?, ?)".to_string(), |p| format!("({}, {})", p.x, p.y));
    let health = world
        .get::<Health>(entity)
        .map_or_else(|| "—".to_string(), |h| format!("{:.0}%", h.current));
    let hunger = world
        .get::<Needs>(entity)
        .map_or_else(|| "—".to_string(), |n| format!("{:.0}%", n.hunger * 100.0));

    let mut lines = vec![Line::from(vec![
        Span::styled(
            "ADVENTURER ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" · {role} · {pos} · HP {health} · hunger {hunger}"),
            Style::default().fg(Color::White),
        ),
    ])];

    // Origin-specific readout.
    if let Some(sovereign) = world.get::<Sovereign>(entity) {
        lines.push(Line::from(vec![
            Span::styled(
                "♛ ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(
                    "legitimacy {:.0}% · melancholy {:.0}%",
                    sovereign.legitimacy * 100.0,
                    sovereign.melancholy * 100.0
                ),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                " — decrees/edicts are console commands",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    if world.get::<CorsairCaptain>(entity).is_some() {
        let corsair = world.get_resource::<CorsairState>();
        let band = corsair.map_or(0, |c| c.heat_band);
        let band_name = match band {
            0 => "calm",
            1 => "patrol",
            2 => "hunters",
            _ => "?",
        };
        let heat = corsair.map_or(0.0, |c| c.heat);
        let skiff_line = corsair
            .and_then(|c| c.skiff)
            .and_then(|skiff| {
                let hull = world.get::<SkiffHull>(skiff).map(|h| h.hp);
                let hold = world.get::<SkiffHold>(skiff);
                hull.map(|hp| {
                    let hold_str = hold.map_or(String::new(), |h| {
                            format!(
                                " · hold {:.1} food / {:.0} cr / {} art",
                                h.food, h.credits, h.artifacts
                            )
                        },
                    );
                    format!("skiff hull {hp:.0}{hold_str}")
                })
            })
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled("⚓ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("heat {heat:.0} ({band_name})"),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("  {skiff_line}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::styled(
        "WASD/arrows move · interact is a console command · Esc release",
        Style::default().fg(Color::DarkGray),
    ));

    let height = (lines.len() + 2).min(area.height as usize).max(3) as u16;
    let width = (area.width.saturating_sub(4)).clamp(20, 78);
    let hud_area = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height),
        width,
        height,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" possess ")
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(hud_area);
    frame.render_widget(block, hud_area);
    render_lines(frame, inner, lines);
}

/// Compact status-bar segments for the reign & sky mechanics.
///
/// Only emits a segment when the mechanic is live (active constellation,
/// uploaded god-mind, crowned sovereign, possessed pop), so the bar stays
/// quiet on colonies that haven't touched these systems.
pub fn build_reign_spans(world: &World) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let sep = || Span::styled("  ║  ", Style::default().fg(Color::DarkGray));

    if let Some(constellation) = world.get_resource::<Constellation>() {
        if constellation.active() && constellation.message.is_lit() {
            let (label, color) = match &constellation.message {
                SloganMessage::Hope(text) => (
                    format!(" ✦ {text} +0.35 "),
                    Color::LightYellow,
                ),
                SloganMessage::Despair(text) => (
                    format!(" ✦ {text} −0.40 "),
                    Color::Red,
                ),
                SloganMessage::Dark => (String::new(), Color::Reset),
            };
            if !label.is_empty() {
                spans.push(sep());
                spans.push(Span::styled(label, Style::default().fg(color)));
            }
        }
    }

    let mind: Option<GodMind> = world
        .iter_entities()
        .filter_map(|e| e.get::<GodMind>().cloned())
        .next();
    if let Some(mind) = mind {
        spans.push(sep());
        spans.push(Span::styled(
            format!(" 🧠 rot {:.0}% ", mind.bit_rot * 100.0),
            Style::default().fg(Color::Magenta),
        ));
    }

    if let Some(state) = world.get_resource::<SovereignState>() {
        if let Some(entity) = state.sovereign {
            if let Some(sovereign) = world.get::<Sovereign>(entity) {
                spans.push(sep());
                spans.push(Span::styled(
                    format!(
                        " ♛ leg {:.0}% mel {:.0}% ",
                        sovereign.legitimacy * 100.0,
                        sovereign.melancholy * 100.0
                    ),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }

    let possessed = world
        .iter_entities()
        .filter(|e| e.get::<Possessed>().is_some())
        .map(|e| e.id())
        .next();
    if let Some(entity) = possessed {
        let name = world
            .get::<PopName>(entity)
            .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
        spans.push(sep());
        spans.push(Span::styled(
            format!(" 🎮 {name} "),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans
}

/// Render up to `area.height` lines into `area`, clipping the rest.
fn render_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let paragraph = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::Pop;
    use crate::setup::{setup_world_with_config, SetupConfig};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_text(world: &World, render: impl Fn(&mut Frame, Rect, &World)) -> String {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), world))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn empty_world_renders_all_panels_gracefully() {
        let world = World::new();
        let text = buffer_text(&world, render_reign_sky);
        assert!(text.contains("Constellation"), "constellation panel renders");
        assert!(text.contains("God-Mind"), "god-mind panel renders");
        assert!(text.contains("Sovereign"), "sovereign panel renders");
    }

    #[test]
    fn possess_hud_renders_nothing_without_possession() {
        let world = World::new();
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_possess_hud(f, f.area(), &world))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!text.contains("ADVENTURER"));
    }

    #[test]
    fn constellation_panel_shows_lit_hope_message() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation
            .satellites
            .extend((0..5).map(|_| world.spawn_empty().id()));
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);
        let text = buffer_text(&world, render_constellation_panel);
        assert!(text.contains("WE ENDURE"), "lit message is shown");
        assert!(text.contains("ACTIVE"), "constellation reads active");
        assert!(text.contains("+0.35"), "morale effect is shown");

        let spans = build_reign_spans(&world);
        let bar: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(bar.contains("WE ENDURE"), "status bar carries the slogan");
    }

    #[test]
    fn constellation_panel_shows_linked_count() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        // The setup path doesn't insert the resource; the sim initializes it
        // lazily. Insert it the way `update_constellation` expects.
        world.insert_resource(Constellation::default());
        let text = buffer_text(&world, render_constellation_panel);
        assert!(text.contains("linked"), "panel shows link status");
        assert!(text.contains("hack risk"), "panel shows hack exposure");
    }

    #[test]
    fn reign_spans_quiet_without_mechanics() {
        let world = World::new();
        assert!(build_reign_spans(&world).is_empty());
    }

    #[test]
    fn reign_spans_show_possessed_pop() {
        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });
        // Find any pop and possess it the way the headless console does.
        let pop = world
            .iter_entities()
            .filter(|e| e.get::<Pop>().is_some())
            .map(|e| e.id())
            .next()
            .expect("headless setup spawns pops");
        world.entity_mut(pop).insert(Possessed);
        let spans = build_reign_spans(&world);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("🎮"), "possess segment appears");
    }

}
