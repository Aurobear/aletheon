//! Awareness signal indicator widget.
//!
//! Renders awareness state in the status bar and inline messages
//! for notable state transitions.

use ratatui::text::{Line, Span};
use ratatui::style::{Color, Style};
use super::state::AwarenessState;
use super::term_compat::TermCaps;

/// Renders awareness indicators.
pub struct AwarenessWidget;

impl AwarenessWidget {
    /// Render the awareness section for the status bar.
    pub fn render_status_bar(awareness: &AwarenessState, caps: &TermCaps) -> Span<'static> {
        let icon = if caps.unicode {
            awareness.level.icon().to_string()
        } else {
            match awareness.level {
                base::ui_event::AwarenessLevel::Confident => "OK".to_string(),
                base::ui_event::AwarenessLevel::Hesitant => "??".to_string(),
                base::ui_event::AwarenessLevel::Confused => "!!".to_string(),
                base::ui_event::AwarenessLevel::Curious => "??".to_string(),
                base::ui_event::AwarenessLevel::Planning => "PL".to_string(),
                base::ui_event::AwarenessLevel::Reflecting => "RF".to_string(),
                base::ui_event::AwarenessLevel::Evolving => "EV".to_string(),
            }
        };

        let color = match awareness.level {
            base::ui_event::AwarenessLevel::Confident => Color::Green,
            base::ui_event::AwarenessLevel::Hesitant => Color::Yellow,
            base::ui_event::AwarenessLevel::Confused => Color::Red,
            base::ui_event::AwarenessLevel::Curious => Color::Cyan,
            base::ui_event::AwarenessLevel::Planning => Color::Magenta,
            base::ui_event::AwarenessLevel::Reflecting => Color::DarkGray,
            base::ui_event::AwarenessLevel::Evolving => Color::Yellow,
        };

        Span::styled(
            format!("{} {}", icon, awareness.level.display_name()),
            Style::default().fg(color),
        )
    }

    /// Render an inline message for notable awareness transitions.
    /// Returns None if no inline message should be shown.
    pub fn render_inline(awareness: &AwarenessState, caps: &TermCaps) -> Option<Line<'static>> {
        if !awareness.should_show_inline() {
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
