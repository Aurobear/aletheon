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
                super::presentation::AwarenessLevel::Confident => "OK".to_string(),
                super::presentation::AwarenessLevel::Hesitant => "??".to_string(),
                super::presentation::AwarenessLevel::Confused => "!!".to_string(),
                super::presentation::AwarenessLevel::Curious => "??".to_string(),
                super::presentation::AwarenessLevel::Planning => "PL".to_string(),
                super::presentation::AwarenessLevel::Reflecting => "RF".to_string(),
                super::presentation::AwarenessLevel::Evolving => "EV".to_string(),
            }
        };

        let color = match awareness.level {
            super::presentation::AwarenessLevel::Confident => Color::Green,
            super::presentation::AwarenessLevel::Hesitant => Color::Yellow,
            super::presentation::AwarenessLevel::Confused => Color::Red,
            super::presentation::AwarenessLevel::Curious => Color::Cyan,
            super::presentation::AwarenessLevel::Planning => Color::Magenta,
            super::presentation::AwarenessLevel::Reflecting => Color::DarkGray,
            super::presentation::AwarenessLevel::Evolving => Color::Yellow,
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
        now: ::contracts::MonoTime,
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
