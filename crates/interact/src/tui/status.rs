//! Hybrid status bar using centralized AppState.

use super::presentation::CollaborationModePresentation;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::awareness::AwarenessWidget;
use super::state::AppState;
use super::term_compat::TermCaps;

/// Status bar shown at the bottom of the TUI.
pub struct StatusBar {
    pub caps: TermCaps,
    pub spinner_frame: usize,
    pub elapsed_secs: f64,
    /// Legacy fields kept for backward compatibility with existing callers.
    pub connected: bool,
    pub provider_info: String,
    pub model_name: String,
    pub waiting: bool,
    pub token_count: Option<u64>,
    pub total_tokens: u64,
    /// Estimated tokens currently present in the active model context.
    pub context_used_tokens: u64,
    pub context_window: u64,
    pub session_turns: u32,
}

impl StatusBar {
    pub fn new(caps: TermCaps) -> Self {
        Self {
            caps,
            spinner_frame: 0,
            elapsed_secs: 0.0,
            connected: false,
            provider_info: String::new(),
            model_name: String::new(),
            waiting: false,
            token_count: None,
            total_tokens: 0,
            context_used_tokens: 0,
            context_window: 0,
            session_turns: 0,
        }
    }

    pub fn tick_spinner(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        // The lifecycle calls this only while a request/turn is streaming.
        // `waiting` becomes false as soon as the first turn event arrives, so
        // gating elapsed time on it made long-running inference/tool rounds
        // appear frozen after admission.
        self.elapsed_secs += 0.06;
    }

    fn spinner_char(&self) -> &'static str {
        let frames = self.caps.spinner_frames();
        frames[self.spinner_frame % frames.len()]
    }

    /// Render using AppState (new path).
    pub fn render_widget_from_state<'a>(&'a self, state: &'a AppState) -> StatusBarStateWidget<'a> {
        StatusBarStateWidget {
            status: self,
            state,
        }
    }

    /// Legacy render using internal fields (backward compat).
    pub fn render_widget(&self) -> StatusBarWidget<'_> {
        StatusBarWidget { status: self }
    }
}

// ── New: AppState-based status bar widget ──────────────────────────

pub struct StatusBarStateWidget<'a> {
    status: &'a StatusBar,
    state: &'a AppState,
}

impl<'a> Widget for StatusBarStateWidget<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 {
            return;
        }

        let y = area.y;
        let bg_color = self.status.caps.color(30, 30, 30);

        let sep = if self.status.caps.unicode {
            " | "
        } else {
            " | "
        };

        let mut spans = Vec::new();

        // Spinner (when streaming)
        if self.state.streaming {
            let spinner_chars: &[&str] = if self.status.caps.unicode {
                &[
                    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}",
                    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
                ]
            } else {
                &["|", "/", "-", "\\", "|", "/", "-", "\\", "|", "/"]
            };
            let frame = spinner_chars[self.status.spinner_frame % spinner_chars.len()];
            spans.push(Span::styled(
                format!("{frame} "),
                Style::default().fg(Color::Yellow),
            ));
        }

        // Mode
        spans.push(Span::styled(
            format!(
                "{} {}",
                self.state.mode.icon(),
                self.state.mode.display_name()
            ),
            Style::default().fg(Color::Cyan),
        ));

        spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));

        // Model
        spans.push(Span::styled(
            self.state.model_name.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));

        // Context
        let ctx_color = if self.state.context_pressure_percent().unwrap_or_default() > 80.0 {
            Color::Red
        } else {
            Color::DarkGray
        };
        spans.push(Span::styled(
            self.state.context_status(),
            Style::default().fg(ctx_color),
        ));

        // Provider usage arrives at inference-round terminal boundaries. Keep
        // active-turn input/output separate from context occupancy and from
        // the Task-cumulative cost counter.
        if self.state.turn_input_tokens > 0 || self.state.turn_output_tokens > 0 {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!(
                    "turn {} in/{} out",
                    format_with_commas(self.state.turn_input_tokens),
                    format_with_commas(self.state.turn_output_tokens)
                ),
                Style::default().fg(Color::DarkGray),
            ));
        } else if self.state.total_tokens > 0 {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!(
                    "task total {} tok",
                    format_with_commas(self.state.total_tokens)
                ),
                Style::default().fg(Color::DarkGray),
            ));
        }

        if self.state.turn_tool_count > 0 {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!(
                    "tools {}/{}",
                    self.state.turn_activity.succeeded, self.state.turn_activity.tool_calls
                ),
                Style::default().fg(Color::DarkGray),
            ));
            if self.state.turn_activity.denied > 0 {
                spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    format!("denied {}", self.state.turn_activity.denied),
                    Style::default().fg(Color::Yellow),
                ));
            }
            if self.state.turn_activity.failed > 0 {
                spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    format!("failed {}", self.state.turn_activity.failed),
                    Style::default().fg(Color::Red),
                ));
            }
        }

        if self.state.current_iteration > 0 {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("infer {}", self.state.turn_activity.inference_rounds),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Awareness
        spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
        spans.push(AwarenessWidget::render_status_bar(
            &self.state.awareness,
            &self.status.caps,
        ));

        // Elapsed (when waiting)
        if self.state.streaming && self.status.elapsed_secs > 0.1 {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("{:.1}s", self.status.elapsed_secs),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Consistent keyboard footer. These actions are safe in both colour
        // and ASCII terminals, and do not imply that the client owns task state.
        spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            "Alt↑↓ activity · Ctrl+T transcript · Ctrl+D diff · Ctrl+C quit",
            Style::default().fg(Color::DarkGray),
        ));

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

// ── Legacy: backward-compatible widget using internal fields ──────

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
            " \u{2502} "
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
            right_parts.push(format!(
                "task total {} tok",
                format_with_commas(self.status.total_tokens)
            ));
        }
        if self.status.context_window > 0 {
            // `total_tokens` is cumulative provider usage across every
            // inference round in the session. It is useful as a cost counter,
            // but it is not the active context size. Use the harness context
            // estimate for pressure so a long-lived session does not
            // appear to fill a 1M window merely by accumulating billed tokens.
            let active_tokens = self.status.context_used_tokens;
            let pct = ((active_tokens as f64) / (self.status.context_window as f64) * 100.0)
                .clamp(0.0, 99.0) as u32;
            right_parts.push(format!("{pct}% ctx"));
        }
        if self.status.session_turns > 0 {
            right_parts.push(format!("turn {}", self.status.session_turns));
        }
        let right = right_parts.join(sep);

        // ── Assemble spans: left (spinner+model) | elapsed | right (tok+ctx+turn) ──
        let mut spans = Vec::new();

        // Left: activity spinner only. Model routing is daemon-owned and may
        // vary per turn, so the client must not display a guessed model name.
        if self.status.waiting {
            spans.push(Span::styled(
                format!(" {} ", self.status.spinner_char()),
                Style::default().fg(Color::Yellow),
            ));
        }

        // Center: elapsed time with separator
        if !center.is_empty() {
            spans.push(Span::styled(sep, Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(center, Style::default().fg(Color::DarkGray)));
        }

        // Right: push to right edge with separator, then tokens + ctx + turn
        if !right.is_empty() {
            let used: usize = spans.iter().map(|s: &Span| s.width()).sum();
            let right_with_sep = format!("{sep}{right}");
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
fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, widgets::Widget};

    #[test]
    fn elapsed_time_advances_after_the_first_turn_event() {
        let mut status = StatusBar::new(TermCaps::detect());
        status.waiting = false;
        status.tick_spinner();
        assert!((status.elapsed_secs - 0.06).abs() < f64::EPSILON);
    }

    #[test]
    fn legacy_context_percent_uses_active_request_not_cumulative_usage() {
        let mut status = StatusBar::new(TermCaps::detect());
        status.total_tokens = 400_000;
        status.token_count = Some(20_000);
        status.context_used_tokens = 20_000;
        status.context_window = 1_000_000;

        let area = Rect::new(0, 0, 100, 1);
        let mut buffer = Buffer::empty(area);
        status.render_widget().render(area, &mut buffer);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("task total 400,000 tok"));
        assert!(rendered.contains("2% ctx"));
        assert!(!rendered.contains("40% ctx"));
    }

    #[test]
    fn state_widget_separates_context_from_active_turn_usage() {
        let status = StatusBar::new(TermCaps::detect());
        let mut state = AppState::default();
        state.context.used = Some(8_000);
        state.context.max = Some(1_000_000);
        state.turn_input_tokens = 300;
        state.turn_output_tokens = 30;
        state.total_tokens = 10_500;

        let area = Rect::new(0, 0, 180, 1);
        let mut buffer = Buffer::empty(area);
        status
            .render_widget_from_state(&state)
            .render(area, &mut buffer);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("ctx: 8k / 1,000k (1%)"), "{rendered}");
        assert!(rendered.contains("turn 300 in/30 out"), "{rendered}");
        assert!(!rendered.contains("task total 10,500 tok"), "{rendered}");
    }
}
