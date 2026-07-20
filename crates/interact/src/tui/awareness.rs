//! Awareness signal indicator widget.
//!
//! Renders awareness state in the status bar and inline messages
//! for notable state transitions.

use super::state::AwarenessState;
use super::term_compat::TermCaps;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Renders awareness indicators.
pub struct AwarenessWidget;

impl AwarenessWidget {
    /// Render the awareness section for the status bar.
    pub fn render_status_bar(awareness: &AwarenessState, caps: &TermCaps) -> Span<'static> {
        let icon = if caps.unicode {
            awareness.level.icon().to_string()
        } else {
            match awareness.level {
                fabric::ui_event::AwarenessLevel::Confident => "OK".to_string(),
                fabric::ui_event::AwarenessLevel::Hesitant => "??".to_string(),
                fabric::ui_event::AwarenessLevel::Confused => "!!".to_string(),
                fabric::ui_event::AwarenessLevel::Curious => "??".to_string(),
                fabric::ui_event::AwarenessLevel::Planning => "PL".to_string(),
                fabric::ui_event::AwarenessLevel::Reflecting => "RF".to_string(),
                fabric::ui_event::AwarenessLevel::Evolving => "EV".to_string(),
            }
        };

        let color = match awareness.level {
            fabric::ui_event::AwarenessLevel::Confident => Color::Green,
            fabric::ui_event::AwarenessLevel::Hesitant => Color::Yellow,
            fabric::ui_event::AwarenessLevel::Confused => Color::Red,
            fabric::ui_event::AwarenessLevel::Curious => Color::Cyan,
            fabric::ui_event::AwarenessLevel::Planning => Color::Magenta,
            fabric::ui_event::AwarenessLevel::Reflecting => Color::DarkGray,
            fabric::ui_event::AwarenessLevel::Evolving => Color::Yellow,
        };

        Span::styled(
            format!("{} {}", icon, awareness.level.display_name()),
            Style::default().fg(color),
        )
    }

    /// Render an inline message for notable awareness transitions.
    /// Returns None if no inline message should be shown.
    pub fn render_inline(
        awareness: &AwarenessState,
        caps: &TermCaps,
        now: fabric::MonoTime,
    ) -> Option<Line<'static>> {
        if !awareness.should_show_inline(now) {
            return None;
        }

        let prefix = if caps.unicode { ">>" } else { ">>" };
        let msg = format!("{} {}", prefix, awareness.context);

        Some(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::Yellow),
        )))
    }
}
