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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};

use super::super::completion::CompletionPopup;
use super::super::state::AppState;
use super::super::status::StatusBar;
use super::super::task_console::TaskConsole;
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
    pub state: &'a AppState,
}

impl Renderable for HeaderRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let bg = self.caps.color(20, 20, 60);

        let line = Line::from(vec![
            Span::styled(
                "  ALETHEON  ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Task Console  ·  mode {}", self.state.mode.display_name()),
                Style::default().fg(Color::Cyan),
            ),
        ]);
        Paragraph::new(line)
            .style(Style::default().bg(bg))
            .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}

// ── ChatRenderable ──────────────────────────────────────────────────

/// Renders the canonical task console. Conversation is intentionally
/// separate from daemon-projected Activity and Changes panels.
pub struct TaskConsoleRenderable<'a> {
    pub caps: &'a TermCaps,
    pub state: &'a AppState,
    pub workspace_name: &'a str,
    pub selected_activity: Option<usize>,
    pub next_agent_runtime: Option<&'a str>,
}

impl Renderable for TaskConsoleRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        TaskConsole {
            state: self.state,
            caps: self.caps,
            workspace_name: self.workspace_name,
            selected_activity: self.selected_activity,
            next_agent_runtime: self.next_agent_runtime,
        }
        .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

// ── InputRenderable ─────────────────────────────────────────────────

/// Renders a separator plus a bounded, cursor-following input viewport.
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

        let byte_pos = self.cursor.min(self.buf.len());
        let cursor_line = self.buf[..byte_pos]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        let cursor_in_line = self.buf[..byte_pos]
            .rsplit_once('\n')
            .map_or(byte_pos, |(_, tail)| tail.len());
        let lines = self.buf.split('\n').collect::<Vec<_>>();
        let visible_rows = usize::from(area.height.saturating_sub(1)).max(1);
        let first_line = cursor_line.saturating_sub(visible_rows.saturating_sub(1));
        let last_line = (first_line + visible_rows).min(lines.len());

        for (visible_index, line_index) in (first_line..last_line).enumerate() {
            let line_area = Rect {
                y: area.y + 1 + visible_index as u16,
                height: 1,
                ..area
            };
            let prefix = if line_index == first_line {
                if lines.len() > visible_rows {
                    format!("{prompt}[{}/{}] ", cursor_line + 1, lines.len())
                } else {
                    prompt.to_owned()
                }
            } else {
                "  ".to_owned()
            };
            let prefix_width = Line::from(prefix.as_str())
                .width()
                .min(usize::from(area.width));
            Paragraph::new(Line::from(Span::styled(
                prefix,
                Style::default().fg(Color::Green),
            )))
            .render(
                Rect {
                    width: prefix_width as u16,
                    ..line_area
                },
                buf,
            );

            let content_area = Rect {
                x: line_area.x.saturating_add(prefix_width as u16),
                width: line_area.width.saturating_sub(prefix_width as u16),
                ..line_area
            };
            if content_area.width == 0 {
                continue;
            }
            let line = lines[line_index];
            if line_index == cursor_line {
                render_cursor_line(line, cursor_in_line, content_area, buf);
            } else {
                Paragraph::new(Line::from(Span::styled(
                    line,
                    Style::default().fg(Color::White),
                )))
                .render(content_area, buf);
            }
        }

        // Render completion popup over the input area (port from completion.rs)
        self.render_completion(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        4
    }
}

fn render_cursor_line(line: &str, cursor: usize, area: Rect, buf: &mut Buffer) {
    let cursor = cursor.min(line.len());
    let before = &line[..cursor];
    let after = &line[cursor..];
    let cursor_char = after
        .chars()
        .next()
        .map(|value| value.to_string())
        .unwrap_or_else(|| " ".to_owned());
    let rest_offset = after
        .char_indices()
        .nth(1)
        .map_or(after.len(), |(offset, _)| offset);
    let line = Line::from(vec![
        Span::styled(before, Style::default().fg(Color::White)),
        Span::styled(
            cursor_char,
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&after[rest_offset..], Style::default().fg(Color::White)),
    ]);
    let cursor_column = Line::from(before).width();
    let horizontal_scroll = cursor_column.saturating_sub(usize::from(area.width).saturating_sub(1));
    Paragraph::new(line)
        .scroll((0, horizontal_scroll.min(u16::MAX as usize) as u16))
        .render(area, buf);
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
                let disabled = cmd
                    .disabled_reason
                    .as_deref()
                    .map_or(String::new(), |reason| format!(" · {reason}"));
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "  {}  {} · {}{} ",
                        cmd.label, cmd.description, cmd.metadata, disabled
                    ),
                    item_style,
                )))
            })
            .collect();

        let height = (comp.candidates.len() as u16 + 2).min(10);
        let popup = Rect {
            x: area.x + 2,
            y: area.y.saturating_sub(height),
            width: 72.min(area.width.saturating_sub(4)),
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
    pub state: &'a AppState,
}

impl Renderable for StatusRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.status
            .render_widget_from_state(self.state)
            .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}
