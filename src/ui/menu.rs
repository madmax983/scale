use crate::setup::{start_scenario_definition, StartScenarioDifficulty};
use crate::ui::menu_state::{MenuSelector, MenuState};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

const LOGO: &str = r#"
  ____   ____    _    _     _____
 / ___| / ___|  / \  | |   | ____|
 \___ \| |     / _ \ | |   |  _|
  ___) | |___ / ___ \| |___| |___
 |____/ \____/_/   \_\_____|_____|
"#;

const SUBTITLE: &str = "From pebble to empire. Every world remembers.";

/// Renders the main menu screen.
///
/// Displays the game title and a selectable list of options (Start Game, Quit).
#[allow(clippy::cast_possible_truncation)]
pub fn render_main_menu(frame: &mut Frame, area: Rect, state: &MenuState) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(2),                                 // Top spacing
            Constraint::Length(9),                              // Title and Subtitle
            Constraint::Length(state.options.len() as u16 + 2), // Menu items
            Constraint::Length(3),                              // Scenario info
            Constraint::Length(3),                              // Origin info
            Constraint::Min(1),                                 // Bottom spacing
            Constraint::Length(3),                              // Footer
        ])
        .split(area);

    // Title Block
    let mut title_text: Vec<Line> = LOGO
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                line,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    title_text.push(Line::from(""));
    title_text.push(Line::from(Span::styled(
        SUBTITLE,
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
    )));

    let title = Paragraph::new(title_text).alignment(Alignment::Center);
    frame.render_widget(title, layout[1]);

    // Options Block
    let options_block = Block::default().borders(Borders::NONE);
    let menu_area = options_block.inner(layout[2]);
    frame.render_widget(options_block, layout[2]);

    let constraints: Vec<Constraint> = state
        .options
        .iter()
        .map(|_| Constraint::Length(1))
        .collect();

    let menu_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(menu_area);

    for (i, option) in state.options.iter().enumerate() {
        let style = if i == state.selected_index {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::White)
        };

        let text = if i == state.selected_index {
            format!("  ▶ {option}  ")
        } else {
            format!("    {option}  ")
        };

        let p = Paragraph::new(text)
            .style(style)
            .alignment(Alignment::Center);
        frame.render_widget(p, menu_layout[i]);
    }

    let scenario = start_scenario_definition(state.selected_scenario);
    let scenario_text = format!(
        "Scenario: {} [{}]",
        scenario.name,
        difficulty_label(scenario.difficulty)
    );
    let scenario_info = Paragraph::new(vec![
        Line::from(Span::styled(
            scenario_text,
            setup_row_style(state.selector == MenuSelector::Scenario),
        )),
        Line::from(Span::styled(
            "Use ←/→ to change scenario",
            Style::default().fg(Color::Gray),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(scenario_info, layout[3]);

    // Adventurer origin row: the player can pick one origin to guarantee
    // in this run, or leave it on "Surprise me" for a random roster.
    let origin_text = format!("Origin: {}", state.origin_label());
    let origin_info = Paragraph::new(vec![
        Line::from(Span::styled(
            origin_text,
            setup_row_style(state.selector == MenuSelector::Origin),
        )),
        Line::from(Span::styled(
            "Tab switches row · ←/→ changes origin",
            Style::default().fg(Color::Gray),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(origin_info, layout[4]);

    // Footer
    let footer_text = "Use ↑/↓ to select | ←/→ change | Tab switch row | Enter to confirm";
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(footer, layout[6]);
}

/// Highlight style for the setup row (scenario / origin) the ←/→ keys
/// currently edit.
fn setup_row_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    }
}

fn difficulty_label(difficulty: StartScenarioDifficulty) -> &'static str {
    match difficulty {
        StartScenarioDifficulty::Hard => "Hard",
        StartScenarioDifficulty::Medium => "Medium",
        StartScenarioDifficulty::Easy => "Easy",
        StartScenarioDifficulty::Standard => "Standard",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::scenario::StartScenarioId;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_render_main_menu_shows_selected_scenario_and_difficulty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = MenuState {
            selected_scenario: StartScenarioId::GroundSurvival,
            ..Default::default()
        };

        terminal
            .draw(|frame| render_main_menu(frame, frame.area(), &state))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for cell in &buffer.content {
            full_text.push_str(cell.symbol());
        }

        assert!(full_text.contains("Ground Survival"));
        assert!(full_text.contains("Hard"));
        // The adventurer-origin setup row renders too.
        assert!(full_text.contains("Origin:"));
        assert!(full_text.contains("Surprise me"));
    }

    #[test]
    fn test_render_main_menu_shows_chosen_origin() {
        use crate::layer1::culture::origins::OriginKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = MenuState {
            selected_origin: Some(OriginKind::Lawbound),
            ..Default::default()
        };

        terminal
            .draw(|frame| render_main_menu(frame, frame.area(), &state))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut full_text = String::with_capacity(buffer.area.area() as usize);
        for cell in &buffer.content {
            full_text.push_str(cell.symbol());
        }

        assert!(full_text.contains("Lawbound"));
    }
}
