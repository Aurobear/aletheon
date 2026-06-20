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

        let mut spans = Vec::new();

        // Left: spinner when waiting
        if self.status.waiting {
            spans.push(Span::styled(
                format!(" {} ", self.status.spinner_char()),
                Style::default().fg(Color::Yellow),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // Center: elapsed time when waiting
        if self.status.waiting && self.status.elapsed_secs > 0.1 {
            let elapsed_text = format!("{:.1}s", self.status.elapsed_secs);
            // Pad to center
            let total_w = area.width as usize;
            let left_pad = total_w.saturating_sub(elapsed_text.len()) / 2;
            let pad = " ".repeat(
                left_pad.saturating_sub(spans.iter().map(|s: &Span| s.width()).sum::<usize>()),
            );
            spans.push(Span::raw(pad));
            spans.push(Span::styled(
                elapsed_text,
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Right: token count if available
        if let Some(tokens) = self.status.token_count {
            let token_text = format!("{tokens} tok");
            // Push to the right with padding
            let used: usize = spans.iter().map(|s: &Span| s.width()).sum();
            let right_pad = (area.width as usize).saturating_sub(used + token_text.len() + 1);
            spans.push(Span::raw(" ".repeat(right_pad)));
            spans.push(Span::styled(
                token_text,
                Style::default().fg(Color::DarkGray),
            ));
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
