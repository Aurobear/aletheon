//! Sub-agent inline display widget.
//!
//! Renders sub-agent status cards inline in the chat area.

use ratatui::text::{Line, Span};
use ratatui::style::{Color, Style};
use base::ui_event::{SubAgentHandle, SubAgentStatus};
use super::term_compat::TermCaps;

/// Renders sub-agent status cards.
pub struct SubAgentViewWidget;

impl SubAgentViewWidget {
    /// Render a sub-agent card as chat lines.
    pub fn render_card(agent: &SubAgentHandle, _caps: &TermCaps) -> Vec<Line<'static>> {
        let (status_str, color) = match &agent.status {
            SubAgentStatus::Planning => ("planning".to_string(), Color::Cyan),
            SubAgentStatus::Executing { current_step } => {
                let bar = format!("executing: {}", current_step);
                (bar, Color::Yellow)
            }
            SubAgentStatus::WaitingApproval => ("waiting approval".to_string(), Color::Magenta),
            SubAgentStatus::Completed { summary } => {
                let s = format!("completed: {}", summary);
                (s, Color::Green)
            }
            SubAgentStatus::Failed { error } => {
                let s = format!("failed: {}", error);
                (s, Color::Red)
            }
        };

        vec![
            Line::from(Span::styled(
                format!("+-- SubAgent: {} -- {} --", agent.id, status_str),
                Style::default().fg(color),
            )),
            Line::from(Span::styled(
                format!("| Task: {}", agent.task),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "+--------------------------------------",
                Style::default().fg(color),
            )),
        ]
    }

    /// Render all active sub-agents as chat lines.
    pub fn render_all(agents: &[SubAgentHandle], caps: &TermCaps) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for agent in agents {
            lines.extend(Self::render_card(agent, caps));
        }
        lines
    }
}
