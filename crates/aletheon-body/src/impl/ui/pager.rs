//! Pager overlay for scrolling through conversation history.
//!
//! Opened with Ctrl+T, renders the full chat transcript in an alternate screen
//! with vim-like scrolling keys. Closes on q/Esc.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::chat::ChatWidget;

/// Full-screen pager overlay for browsing conversation history.
pub struct PagerOverlay {
    /// All rendered lines from the chat transcript.
    lines: Vec<Line<'static>>,
    /// Current scroll offset (0 = bottom/latest).
    scroll_offset: usize,
    /// Title shown at the top.
    title: String,
}

impl PagerOverlay {
    /// Create a pager overlay from the current chat widget state.
    pub fn from_chat(chat: &ChatWidget, title: &str) -> Self {
        let lines = chat.all_lines_wrapped(200); // wide enough to avoid wrapping in pager
        Self {
            lines,
            scroll_offset: 0,
            title: title.to_string(),
        }
    }

    /// Render the pager overlay into the given area.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Header: title
        let header = format!(" {} ", self.title);
        let header_line = Line::from(Span::styled(
            header,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let sep = Line::from(Span::styled(
            "─".repeat(area.width as usize),
            Style::default().add_modifier(Modifier::DIM),
        ));

        // Content area (exclude header + separator + footer)
        let content_height = area.height.saturating_sub(3) as usize;
        let total_lines = self.lines.len();

        // scroll_offset: 0 = bottom (latest), increases = scroll up
        let max_scroll = total_lines.saturating_sub(content_height);
        let scroll = self.scroll_offset.min(max_scroll);
        let end = total_lines.saturating_sub(scroll);
        let start = end.saturating_sub(content_height);

        // Clamp start to valid range
        let start = start.min(total_lines);
        let end = end.min(total_lines);

        let visible: Vec<Line> = if start < end {
            self.lines[start..end].to_vec()
        } else {
            vec![]
        };

        // Footer: scroll percentage + key hints
        let percent = if max_scroll == 0 {
            100
        } else {
            (((scroll as f32 / max_scroll as f32) * 100.0).round()) as u8
        };
        let footer = format!(
            " {}%  ↑/↓ scroll  PgUp/PgDn  g top  G bottom  q close ",
            percent
        );

        // Render header
        let header_area = Rect::new(area.x, area.y, area.width, 1);
        header_line.render(header_area, buf);

        // Render separator
        let sep_area = Rect::new(area.x, area.y + 1, area.width, 1);
        sep.render(sep_area, buf);

        // Render content
        let content_area = Rect::new(area.x, area.y + 2, area.width, content_height as u16);
        Paragraph::new(visible)
            .wrap(Wrap { trim: false })
            .render(content_area, buf);

        // Fill remaining space with '~'
        let drawn = (end.saturating_sub(start)) as u16;
        for row in drawn..content_height as u16 {
            let y = content_area.y + row;
            if y < area.y + area.height.saturating_sub(1) {
                ratatui::widgets::Clear.render(
                    Rect::new(area.x, y, area.width, 1),
                    buf,
                );
                let tilde = Line::from(Span::styled(
                    "~",
                    Style::default().add_modifier(Modifier::DIM),
                ));
                tilde.render(Rect::new(area.x, y, area.width, 1), buf);
            }
        }

        // Render footer
        let footer_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        Line::from(Span::styled(
            footer,
            Style::default().add_modifier(Modifier::DIM),
        ))
        .render(footer_area, buf);
    }

    /// Handle a key event. Returns true if the pager should close.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            // Close
            KeyCode::Char('q') | KeyCode::Esc => true,
            // Scroll up
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                false
            }
            // Scroll down
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                false
            }
            // Page up
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
                false
            }
            // Page down
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
                false
            }
            // Jump to top (oldest)
            KeyCode::Char('g') => {
                self.scroll_offset = usize::MAX;
                false
            }
            // Jump to bottom (latest)
            KeyCode::Char('G') => {
                self.scroll_offset = 0;
                false
            }
            _ => false,
        }
    }
}
