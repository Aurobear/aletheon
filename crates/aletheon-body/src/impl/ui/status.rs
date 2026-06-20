use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::term_compat::TermCaps;

/// Status bar shown at the bottom of the TUI.
pub struct StatusBar {
    pub connected: bool,
    pub provider_info: String,
    pub model_name: String,
    pub waiting: bool,
    pub elapsed_secs: f64,
    pub token_count: Option<u32>,
    /// Cumulative tokens across all turns in this session.
    pub total_tokens: u32,
    /// Maximum context window size (default 128000).
    pub context_window: u32,
    /// Number of completed turns in this session.
    pub session_turns: u32,
    spinner_frame: u8,
    caps: TermCaps,
}

impl StatusBar {
    pub fn new(caps: TermCaps) -> Self {
        Self {
            connected: false,
            provider_info: String::new(),
            model_name: String::new(),
            waiting: false,
            elapsed_secs: 0.0,
            token_count: None,
            total_tokens: 0,
            context_window: 128_000,
            session_turns: 0,
            spinner_frame: 0,
            caps,
        }
    }

    pub fn tick_spinner(&mut self) {
        let len = self.caps.spinner_frames().len() as u8;
        self.spinner_frame = (self.spinner_frame + 1) % len;
        if self.waiting {
            self.elapsed_secs += 0.06; // ~60ms per tick
        }
    }

    fn spinner_char(&self) -> &'static str {
        self.caps.spinner_frames()[self.spinner_frame as usize]
    }

    pub fn render_widget(&self) -> StatusBarWidget<'_> {
        StatusBarWidget { status: self }
    }
}

pub struct StatusBarWidget<'a> {
    status: &'a StatusBar,
}

impl<'a> Widget for StatusBarWidget<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 {
            return;
        }

        let y = area.y;
        let bg_color = self.status.caps.color(30, 30, 30);
        let total_w = area.width as usize;

        let sep = if self.status.caps.unicode {
            " │ "
        } else {
            " | "
        };

        // ── Center section: elapsed time (only when waiting & > 0.1s) ──
        let center = if self.status.waiting && self.status.elapsed_secs > 0.1 {
            format!("{:.1}s", self.status.elapsed_secs)
        } else {
            String::new()
        };

        // ── Right section: tokens + context % + turn count ──
        let mut right_parts = Vec::new();
        if self.status.total_tokens > 0 {
            right_parts.push(format!("{} tok", format_with_commas(self.status.total_tokens)));
        }
        if self.status.context_window > 0 {
            let pct = ((self.status.total_tokens as f64)
                / (self.status.context_window as f64)
                * 100.0)
                .clamp(0.0, 99.0) as u32;
            right_parts.push(format!("{}% ctx", pct));
        }
        if self.status.session_turns > 0 {
            right_parts.push(format!("turn {}", self.status.session_turns));
        }
        let right = right_parts.join(sep);

        // ── Assemble spans: left (spinner+model) | elapsed | right (tok+ctx+turn) ──
        let mut spans = Vec::new();

        // Left: spinner (yellow when waiting) + model name (dim)
        if self.status.waiting {
            spans.push(Span::styled(
                format!(" {} ", self.status.spinner_char()),
                Style::default().fg(Color::Yellow),
            ));
        }
        spans.push(Span::styled(
            if self.status.waiting {
                self.status.model_name.clone()
            } else {
                format!(" {}", self.status.model_name)
            },
            Style::default().fg(Color::DarkGray),
        ));

        // Center: elapsed time with separator
        if !center.is_empty() {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(center, Style::default().fg(Color::DarkGray)));
        }

        // Right: push to right edge with separator, then tokens + ctx + turn
        if !right.is_empty() {
            let used: usize = spans.iter().map(|s: &Span| s.width()).sum();
            let right_with_sep = format!("{}{}", sep, right);
            let right_pad = total_w.saturating_sub(used + right_with_sep.len());
            spans.push(Span::raw(" ".repeat(right_pad)));
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(right, Style::default().fg(Color::DarkGray)));
        }

        let line = Line::from(spans);

        // Fill background
        for x in area.left()..area.right() {
            buf[(x, y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(bg_color));
        }

        let mut x = area.left();
        for span in &line.spans {
            let content = span.content.as_ref();
            let style = span.style;
            for ch in content.chars() {
                if x >= area.right() {
                    break;
                }
                buf[(x, y)]
                    .set_symbol(&ch.to_string())
                    .set_style(style.bg(bg_color));
                x += 1;
            }
        }
    }
}

/// Format a number with comma separators (e.g. 12345 -> "12,345").
fn format_with_commas(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}
