use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

pub struct HelpOverlay;

impl HelpOverlay {
    pub fn new() -> Self {
        Self
    }

    pub fn handle_key(&self, key: crossterm::event::KeyEvent) -> bool {
        // Close on q, Esc, ?, or Ctrl+H
        matches!(
            key.code,
            crossterm::event::KeyCode::Char('q')
                | crossterm::event::KeyCode::Esc
                | crossterm::event::KeyCode::Char('?')
        ) || (key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && matches!(key.code, crossterm::event::KeyCode::Char('h')))
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let header_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        let key_style = Style::default().fg(Color::Green);
        let desc_style = Style::default().fg(Color::White);
        let section_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);

        // Clear background
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::default().bg(Color::Black));
            }
        }

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            " Help ",
            header_style.bg(Color::DarkGray),
        )]));
        lines.push(Line::from(""));

        // Keyboard section
        lines.push(Line::from(Span::styled(
            "  Keyboard Shortcuts",
            section_style,
        )));
        lines.push(Line::from(""));

        let shortcuts: &[(&str, &str)] = &[
            ("Enter", "Send message"),
            ("Shift+Enter", "New line"),
            ("Esc", "Clear input"),
            ("Ctrl+C", "Cancel turn / Clear input / Quit"),
            ("Ctrl+D", "Quit"),
            ("Ctrl+L", "Clear screen"),
            ("Ctrl+T", "Open transcript pager"),
            ("Ctrl+O", "Toggle thinking display"),
            ("Ctrl+B", "Toggle last tool card"),
            ("Ctrl+A", "Cursor to line start"),
            ("Ctrl+E", "Cursor to line end"),
            ("Ctrl+W", "Delete word backward"),
            ("Ctrl+K", "Delete to end of line"),
            ("Ctrl+U", "Delete to start of line"),
            ("Up/Down", "Command history"),
            ("PgUp/PgDn", "Scroll chat"),
            ("Tab", "Command completion"),
            ("?", "This help"),
        ];

        for (key, desc) in shortcuts {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<16}", key), key_style),
                Span::styled(*desc, desc_style),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  Mouse", section_style)));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<16}", "Scroll"), key_style),
            Span::styled("Chat / Pager scroll", desc_style),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Slash Commands",
            section_style,
        )));
        lines.push(Line::from(""));

        let commands: &[(&str, &str)] = &[
            ("/help", "Show this help"),
            ("/clear", "Clear screen"),
            ("/copy", "Copy last response"),
            ("/status", "Show session status"),
            ("/compact", "Compact context"),
            ("/sessions", "List sessions"),
            ("/resume", "Resume session"),
            ("/quit", "Quit"),
        ];

        for (cmd, desc) in commands {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<16}", cmd), key_style),
                Span::styled(*desc, desc_style),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press q or Esc to close",
            Style::default().fg(Color::DarkGray),
        )));

        // Render lines centered vertically
        let content_height = lines.len() as u16;
        let y_offset = if content_height < area.height {
            (area.height - content_height) / 2
        } else {
            0
        };

        for (i, line) in lines.iter().enumerate() {
            let y = area.y + y_offset + i as u16;
            if y >= area.y + area.height {
                break;
            }
            // Center horizontally
            let line_width = line.width() as u16;
            let x_offset = if line_width < area.width {
                (area.width - line_width) / 2
            } else {
                0
            };
            let line_area = Rect {
                x: area.x + x_offset,
                y,
                width: area.width.saturating_sub(x_offset),
                height: 1,
            };
            line.render(line_area, buf);
        }
    }
}
