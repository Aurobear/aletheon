//! Plan mode visualization widget.
//!
//! Renders the plan-critique-revise cycle during plan mode.
//! Shows plan versions, critiques, and approval status.

use super::term_compat::TermCaps;
use base::brain::{CriticismSeverity, Critique, Plan};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// A plan version with its critique history.
#[derive(Debug, Clone)]
pub struct PlanVersion {
    pub version: usize,
    pub plan: Plan,
    pub critique: Option<Vec<Critique>>,
}

/// State for the plan view widget.
#[derive(Debug, Default)]
pub struct PlanViewState {
    pub versions: Vec<PlanVersion>,
    pub ready_for_approval: bool,
    pub visible: bool,
}

impl PlanViewState {
    pub fn add_version(&mut self, version: PlanVersion) {
        self.versions.push(version);
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.ready_for_approval = ready;
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// Renders the plan view.
pub struct PlanViewWidget;

impl PlanViewWidget {
    /// Render plan versions as chat lines (inline in the chat area).
    pub fn render_chat_lines(state: &PlanViewState, _caps: &TermCaps) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for ver in &state.versions {
            // Plan header
            lines.push(Line::from(Span::styled(
                format!("+-- Plan v{} --", ver.version),
                Style::default().fg(Color::Cyan),
            )));

            // Plan steps
            for (i, step) in ver.plan.steps.iter().enumerate() {
                lines.push(Line::from(Span::raw(format!(
                    "| {}. {}",
                    i + 1,
                    step.action.name
                ))));
            }

            // Critique (if any)
            if let Some(ref critiques) = ver.critique {
                lines.push(Line::from(Span::styled(
                    "+-- Critique --",
                    Style::default().fg(Color::Yellow),
                )));

                if critiques.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "| No issues found",
                        Style::default().fg(Color::Green),
                    )));
                } else {
                    for c in critiques {
                        let (icon, color) = match c.severity {
                            CriticismSeverity::Fatal | CriticismSeverity::Error => {
                                ("x", Color::Red)
                            }
                            CriticismSeverity::Warning => ("!", Color::Yellow),
                            CriticismSeverity::Info => ("i", Color::Cyan),
                        };
                        lines.push(Line::from(vec![
                            Span::styled(format!("| {} ", icon), Style::default().fg(color)),
                            Span::raw(format!("{:?}: {}", c.dimension, c.description)),
                        ]));
                    }
                }
            }

            lines.push(Line::from(Span::styled(
                "+---------------------",
                Style::default().fg(Color::DarkGray),
            )));
        }

        // Approval prompt
        if state.ready_for_approval {
            lines.push(Line::from(Span::styled(
                "Plan ready. Type /approve to execute, or continue editing.",
                Style::default().fg(Color::Green),
            )));
        }

        lines
    }
}
