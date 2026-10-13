use bevy_ecs::prelude::*;
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Row, Table},
};

use crate::layer1::{format_event_prefix, Chronicle, EventImportance};

/// Width of the Time column (`Y<year> [<tick>]`).
const TIME_COLUMN_WIDTH: u16 = 12;
/// Width of the importance-prefix column.
const IMPORTANCE_COLUMN_WIDTH: u16 = 4;

/// The width ratatui will actually give the Event column for a
/// Chronicle table rendered into `area_width`: the Min constraint
/// absorbs whatever the fixed Time/Imp columns, the two borders,
/// and the default 1-cell column spacing (2 gaps) leave behind.
fn event_column_width(area_width: u16) -> usize {
    usize::from(
        area_width
            .saturating_sub(2)
            .saturating_sub(TIME_COLUMN_WIDTH)
            .saturating_sub(IMPORTANCE_COLUMN_WIDTH)
            .saturating_sub(2),
    )
}

/// Wraps `text` to lines of at most `width` characters.
///
/// ratatui `Table` cells never wrap, so Chronicle event text is
/// wrapped here and emitted as continuation rows instead of being
/// clipped mid-word at the pane edge. Breaks on word boundaries;
/// a single word longer than `width` is hard-split. Always returns
/// at least one line (empty text yields one empty line), and a
/// `width` of 0 returns the text unwrapped rather than looping
/// forever.
///
/// # Examples
///
/// ```
/// use scale::ui::chronicle::wrap_event_text;
///
/// assert_eq!(
///     wrap_event_text("the old north road", 10),
///     vec!["the old".to_string(), "north road".to_string()]
/// );
/// ```
#[must_use]
pub fn wrap_event_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        // Hard-split words that cannot fit on a line of their own.
        let mut rest = word;
        while rest.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let split_at = rest
                .char_indices()
                .nth(width)
                .map_or(rest.len(), |(idx, _)| idx);
            lines.push(rest[..split_at].to_string());
            rest = &rest[split_at..];
        }
        if current.is_empty() {
            current = rest.to_string();
        } else if current.chars().count() + 1 + rest.chars().count() <= width {
            current.push(' ');
            current.push_str(rest);
        } else {
            lines.push(std::mem::take(&mut current));
            current = rest.to_string();
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Renders the chronicle pane into the provided area.
pub fn render_chronicle(frame: &mut Frame, area: Rect, world: &World) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let chronicle = world.resource::<Chronicle>();

    let block = Block::default()
        .title(" Chronicle ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Black));

    frame.render_widget(Clear, area);

    // Table Header
    let header = Row::new(vec!["Time", "Imp", "Event"])
        .style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )
        .bottom_margin(1);

    // Table Rows
    //
    // ratatui Table cells never wrap, so event text is pre-wrapped
    // to the Event column's real width and emitted as continuation
    // rows (blank Time/Imp cells) instead of being clipped mid-word
    // at the pane edge. Rows available between the header and the
    // bottom border: height - 2 (borders) - 1 (header) - 1 (header
    // bottom margin). The newest event is always shown at least in
    // part, even when it alone overflows the pane.
    let max_rows = usize::from(area.height.saturating_sub(4));
    let event_width = event_column_width(area.width);
    let mut rows: Vec<Row> = Vec::new();
    for evt in chronicle.events.iter().rev() {
        // Newest first
        let color = get_importance_color(evt.importance);
        let style = Style::default().fg(color);
        let time = if evt.year == 0 {
            format!("Pre [{}]", evt.tick)
        } else {
            format!("Y{} [{}]", evt.year, evt.tick)
        };
        let prefix = format_event_prefix(evt.importance).to_string();
        for (line_idx, line) in wrap_event_text(&evt.text, event_width)
            .into_iter()
            .enumerate()
        {
            if line_idx > 0 && rows.len() >= max_rows {
                break;
            }
            let (time_cell, prefix_cell) = if line_idx == 0 {
                (time.as_str(), prefix.as_str())
            } else {
                ("", "")
            };
            rows.push(
                Row::new(vec![time_cell.to_string(), prefix_cell.to_string(), line]).style(style),
            );
            if rows.len() >= max_rows {
                break;
            }
        }
        if rows.len() >= max_rows {
            break;
        }
    }

    // Column Widths
    let widths = [
        Constraint::Length(TIME_COLUMN_WIDTH),       // Time
        Constraint::Length(IMPORTANCE_COLUMN_WIDTH), // Type
        Constraint::Min(20),                         // Event
    ];

    let table = Table::new(rows, widths).header(header).block(block);

    frame.render_widget(table, area);
}

/// Returns the display color for an event based on its importance.
///
/// # Examples
///
/// ```
/// use scale::ui::chronicle::get_importance_color;
/// use scale::layer1::EventImportance;
/// use ratatui::style::Color;
///
/// assert_eq!(get_importance_color(EventImportance::Legendary), Color::Yellow);
/// assert_eq!(get_importance_color(EventImportance::Minor), Color::DarkGray);
/// ```
#[must_use]
pub const fn get_importance_color(importance: EventImportance) -> Color {
    match importance {
        EventImportance::Legendary => Color::Yellow,
        EventImportance::Major => Color::Magenta,
        EventImportance::Standard => Color::White,
        EventImportance::Minor => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::ChronicleEvent;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_rows(world: &World, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_chronicle(f, f.area(), world))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer.cell((x, y)).map_or(" ", |c| c.symbol()))
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn long_event_text_wraps_instead_of_clipping() {
        let mut world = World::new();
        world.insert_resource(Chronicle {
            events: vec![
                ChronicleEvent {
                    tick: 7,
                    year: 1,
                    text: "Colony founded.".to_string(),
                    importance: EventImportance::Standard,
                },
                ChronicleEvent {
                    tick: 42,
                    year: 2,
                    text: "A cradle-pod touches down humming three octaves below hearing"
                        .to_string(),
                    importance: EventImportance::Major,
                },
            ],
        });

        // At 44 cells wide the Event column gets 44 - 2 (borders)
        // - 12 (Time) - 4 (Imp) - 2 (column spacing) = 24 cells, so
        // the 61-char event must wrap onto three lines.
        let rows = buffer_rows(&world, 44, 12);
        let find_row = |needle: &str| rows.iter().position(|r| r.contains(needle));

        let first = find_row("A cradle-pod touches").expect("first wrapped line renders");
        let second = find_row("down humming three").expect("second wrapped line renders");
        let third =
            find_row("octaves below hearing").expect("final wrapped line renders, not clipped");
        let short = find_row("Colony founded.").expect("short event renders");

        // Newest event first, its wrapped lines in order, older
        // single-line event after them.
        assert!(first < second && second < third && third < short);
        assert!(
            rows[first].contains("Y2 [42]"),
            "first line keeps its Time cell: {:?}",
            rows[first]
        );
        assert!(
            !rows[third].contains("Y2"),
            "continuation line has an empty Time cell: {:?}",
            rows[third]
        );
        assert!(
            rows[short].contains("Y1 [7]"),
            "short event keeps its Time cell on its single row: {:?}",
            rows[short]
        );
    }
}
