use super::{render_with_frame, SharedWorld};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_hypertile_extras::HypertilePlugin;

use crate::shared::log::MessageLog;

pub struct LogPlugin {
    world: SharedWorld,
}

impl LogPlugin {
    pub const fn new(world: SharedWorld) -> Self {
        Self { world }
    }
}

impl HypertilePlugin for LogPlugin {
    fn render(&mut self, area: Rect, buf: &mut Buffer, _is_focused: bool) {
        let world = self.world.borrow();
        render_with_frame(area, buf, |frame| {
            let block = Block::default()
                .title(ratatui::text::Span::styled(
                    " Message Log ",
                    ratatui::style::Style::default()
                        .fg(ratatui::style::Color::Cyan)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray));

            if let Some(log) = world.get_resource::<MessageLog>() {
                if log.messages.is_empty() {
                    let paragraph = Paragraph::new("(No messages)")
                        .style(ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray))
                        .block(block)
                        .alignment(ratatui::layout::Alignment::Center);
                    frame.render_widget(paragraph, frame.area());
                } else {
                    use ratatui::style::{Color, Modifier, Style};
                    use ratatui::text::{Line, Span};

                    let items: Vec<ListItem> = log
                        .messages
                        .iter()
                        .enumerate()
                        .map(|(i, msg)| {
                            let (prefix, prefix_style) = match msg.color {
                                Color::Red | Color::LightRed => (
                                    "[ERR] ",
                                    Style::default().fg(msg.color).add_modifier(Modifier::BOLD),
                                ),
                                Color::Yellow | Color::LightYellow => (
                                    "[WRN] ",
                                    Style::default().fg(msg.color).add_modifier(Modifier::BOLD),
                                ),
                                Color::Green | Color::LightGreen => (
                                    "[OK ] ",
                                    Style::default().fg(msg.color).add_modifier(Modifier::BOLD),
                                ),
                                Color::Cyan | Color::LightCyan => (
                                    "[INF] ",
                                    Style::default().fg(msg.color).add_modifier(Modifier::BOLD),
                                ),
                                _ => ("[LOG] ", Style::default().fg(msg.color)),
                            };

                            let line = Line::from(vec![
                                Span::styled(prefix, prefix_style),
                                Span::styled(msg.text.clone(), Style::default().fg(msg.color)),
                            ]);

                            // Alternating row background for Z-pattern readability
                            let bg_color = if i % 2 == 0 {
                                Color::Reset
                            } else {
                                Color::DarkGray
                            };

                            ListItem::new(line).style(Style::default().bg(bg_color))
                        })
                        .collect();

                    let list = List::new(items).block(block);
                    frame.render_widget(list, frame.area());
                }
            } else {
                let paragraph = Paragraph::new("MessageLog not initialized.")
                    .style(ratatui::style::Style::default().fg(ratatui::style::Color::Red))
                    .block(block);
                frame.render_widget(paragraph, frame.area());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui_hypertile_extras::HypertilePlugin;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn setup_world() -> Rc<RefCell<World>> {
        let mut world = World::new();
        world.insert_resource(MessageLog::new(10));
        Rc::new(RefCell::new(world))
    }

    fn buffer_text(buffer: &Buffer) -> String {
        // ⚡ Bolt Optimization: Replace intermediate .collect::<String>() chain with pre-allocated String loop
        let mut s = String::with_capacity(buffer.area.area() as usize);
        for cell in &buffer.content {
            s.push_str(cell.symbol());
        }
        s
    }

    #[test]
    fn test_log_plugin_empty() {
        let world = setup_world();
        let mut plugin = LogPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 40, 10);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(text.contains("Message Log"));
        assert!(text.contains("(No messages)"));
    }

    #[test]
    fn test_log_plugin_with_messages() {
        let world = setup_world();
        {
            let mut w = world.borrow_mut();
            let mut log = w.resource_mut::<MessageLog>();
            log.add_colored("Error Message", Color::Red);
            log.add_colored("Warning Message", Color::Yellow);
            log.add_colored("Info Message", Color::Cyan);
            log.add_colored("Success Message", Color::Green);
            log.add_colored("Normal Message", Color::White);
        }

        let mut plugin = LogPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 40, 10);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(text.contains("Message Log"));
        assert!(text.contains("[ERR] Error Message"));
        assert!(text.contains("[WRN] Warning Message"));
        assert!(text.contains("[INF] Info Message"));
        assert!(text.contains("[OK ] Success Message"));
        assert!(text.contains("[LOG] Normal Message"));
    }

    #[test]
    fn test_log_plugin_uninitialized() {
        let world = Rc::new(RefCell::new(World::new()));
        let mut plugin = LogPlugin::new(Rc::clone(&world));
        let area = Rect::new(0, 0, 40, 10);
        let mut buffer = Buffer::empty(area);

        plugin.render(area, &mut buffer, false);

        let text = buffer_text(&buffer);
        assert!(text.contains("Message Log"));
        assert!(text.contains("MessageLog not initialized."));
    }
}
