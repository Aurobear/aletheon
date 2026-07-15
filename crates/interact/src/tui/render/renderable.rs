//! Composable rendering via the `Renderable` trait.
//!
//! Each UI component (header, chat, input, status) implements `Renderable`, and
//! `LayoutHelper` composes them vertically, replacing the ad-hoc Layout split
//! previously hardcoded in `draw.rs`.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Widget, Wrap},
};

use super::super::chat::ChatWidget;
use super::super::completion::CompletionPopup;
use super::super::status::StatusBar;
use super::super::term_compat::TermCaps;

// ── Renderable trait ────────────────────────────────────────────────

/// Trait for any UI component that can render itself to a ratatui Buffer.
pub trait Renderable {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

// ── LayoutHelper ────────────────────────────────────────────────────

/// Simple vertical layout that renders children top-to-bottom.
///
/// Children are registered with either a fixed height (via `push_fixed`) or
/// marked as flex (via `push_flex`). Flex children split remaining space
/// equally. The lifetime `'a` matches the borrows each child holds.
pub struct LayoutHelper<'a> {
    /// (height, child).  height == 0 means flex; otherwise fixed.
    children: Vec<(u16, Box<dyn Renderable + 'a>)>,
}

impl<'a> LayoutHelper<'a> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Add a child with a fixed pixel height.
    pub fn push_fixed(&mut self, height: u16, child: impl Renderable + 'a) {
        self.children.push((height, Box::new(child)));
    }

    /// Add a child that takes any remaining space (split equally among flex children).
    pub fn push_flex(&mut self, child: impl Renderable + 'a) {
        self.children.push((0, Box::new(child)));
    }
}

impl Renderable for LayoutHelper<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        // Distribute remaining space among flex children
        let mut total_fixed: u16 = 0;
        let mut flex_count: u16 = 0;
        for (h, _) in &self.children {
            if *h == 0 {
                flex_count += 1;
            } else {
                total_fixed += *h;
            }
        }

        let flex_height = if flex_count > 0 {
            area.height
                .saturating_sub(total_fixed)
                .saturating_div(flex_count)
        } else {
            0
        };

        let mut y = area.y;
        let max_y = area.y + area.height;
        for (h, child) in &self.children {
            if y >= max_y {
                break;
            }
            let child_height = if *h == 0 { flex_height } else { *h }.min(max_y - y);
            if child_height == 0 {
                continue;
            }
            let child_area = Rect::new(area.x, y, area.width, child_height);
            child.render(child_area, buf);
            y += child_height;
        }
    }
}

// ── HeaderRenderable ────────────────────────────────────────────────

/// Renders the top header bar (1 or 3 rows depending on first-render state).
pub struct HeaderRenderable<'a> {
    pub caps: &'a TermCaps,
}

impl Renderable for HeaderRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let bg = self.caps.color(20, 20, 60);

        let line = Line::from(Span::styled(
            "  aletheon",
            Style::default().fg(Color::White),
        ));
        Paragraph::new(line)
            .style(Style::default().bg(bg))
            .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}

// ── ChatRenderable ──────────────────────────────────────────────────

/// Renders the chat area with inline tool cards and scroll support.
pub struct ChatRenderable<'a> {
    pub chat: &'a ChatWidget,
    pub frame_counter: u64,
    pub caps: &'a TermCaps,
}

impl Renderable for ChatRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let chat_block = Block::default()
            .borders(Borders::NONE)
            .padding(Padding::horizontal(1));
        let chat_inner = chat_block.inner(area);
        chat_block.render(area, buf);

        let visible = self.chat.visible_lines(
            self.frame_counter,
            chat_inner.width as usize,
            chat_inner.height,
        );
        Paragraph::new(visible)
            .wrap(Wrap { trim: false })
            .render(chat_inner, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0 // flex — takes remaining space
    }
}

// ── InputRenderable ─────────────────────────────────────────────────

/// Renders the 3-row input area (separator, input text with cursor, hint line).
pub struct InputRenderable<'a> {
    pub buf: &'a str,
    pub cursor: usize,
    pub has_cjk: bool,
    pub caps: &'a TermCaps,
    pub completion: &'a CompletionPopup,
}

impl Renderable for InputRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let border_h = self.caps.hline();
        let prompt = if self.caps.unicode { "❯ " } else { "> " };

        // Row 0: separator line
        let sep = format!(
            "  {}",
            border_h.repeat(area.width.saturating_sub(4) as usize)
        );
        let sep_line = Line::from(Span::styled(sep, Style::default().fg(Color::DarkGray)));
        Paragraph::new(sep_line).render(Rect { height: 1, ..area }, buf);

        // Row 1: input text with cursor
        let input_area = Rect {
            y: area.y + 1,
            height: 1,
            ..area
        };
        let mut spans = vec![Span::styled(prompt, Style::default().fg(Color::Green))];

        let byte_pos = self.cursor.min(self.buf.len());
        let before = &self.buf[..byte_pos];
        let after = &self.buf[byte_pos..];

        if !before.is_empty() {
            spans.push(Span::styled(before, Style::default().fg(Color::White)));
        }

        let cursor_char = after
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_else(|| " ".to_string());
        spans.push(Span::styled(
            cursor_char,
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));

        let rest = if after.chars().count() > 1 {
            &after[after
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(after.len())..]
        } else {
            ""
        };
        if !rest.is_empty() {
            spans.push(Span::styled(rest, Style::default().fg(Color::White)));
        }

        Paragraph::new(Line::from(spans)).render(input_area, buf);

        // Render completion popup over the input area (port from completion.rs)
        self.render_completion(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        2
    }
}

impl InputRenderable<'_> {
    /// Render the tab-completion popup above the input area.
    fn render_completion(&self, area: Rect, buf: &mut Buffer) {
        let comp = self.completion;
        if !comp.visible || comp.candidates.is_empty() {
            return;
        }

        // Build ListItems mirroring CompletionPopup::render
        let items: Vec<ListItem> = comp
            .candidates
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let item_style = if i == comp.selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(format!("  {} ", cmd), item_style)))
            })
            .collect();

        let height = (comp.candidates.len() as u16 + 2).min(10);
        let popup = Rect {
            x: area.x + 2,
            y: area.y.saturating_sub(height),
            width: 30.min(area.width.saturating_sub(4)),
            height,
        };

        Clear.render(popup, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let list = List::new(items).block(block);
        list.render(popup, buf);
    }
}

// ── StatusRenderable ────────────────────────────────────────────────

/// Renders the single-row status bar at the bottom of the screen.
pub struct StatusRenderable<'a> {
    pub status: &'a StatusBar,
}

impl Renderable for StatusRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.status.render_widget().render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}
