use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame,
};

/// Decision returned when the user presses a key in the approval dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogDecision {
    Approve,
    ApproveForSession,
    ApprovePathForSession,
    Deny,
}

/// A centered modal dialog asking the user to approve or deny a tool invocation.
pub struct ApprovalDialog {
    pub approval_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    /// Full command / diff context for the user to inspect.
    pub detail: Option<String>,
    pub scope_subject: Option<fabric::protocol::client::TransientApprovalScopeSubject>,
    pub scroll: u16,
}

impl ApprovalDialog {
    pub fn new(
        approval_id: impl Into<String>,
        tool: impl Into<String>,
        action_summary: impl Into<String>,
        risk_level: impl Into<String>,
    ) -> Self {
        Self {
            approval_id: approval_id.into(),
            tool: tool.into(),
            action_summary: action_summary.into(),
            risk_level: risk_level.into(),
            detail: None,
            scope_subject: None,
            scroll: 0,
        }
    }

    pub fn with_scope_subject(
        mut self,
        subject: Option<fabric::protocol::client::TransientApprovalScopeSubject>,
    ) -> Self {
        self.scope_subject = subject;
        self
    }

    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }

    /// Map a key press to a dialog decision.
    ///
    /// - `y` -> Approve
    /// - `a` -> ApproveForSession
    /// - `n` or `d` -> Deny
    /// - anything else -> None
    pub fn key_to_decision(c: char) -> Option<DialogDecision> {
        match c {
            'y' => Some(DialogDecision::Approve),
            'a' => Some(DialogDecision::ApproveForSession),
            'p' => Some(DialogDecision::ApprovePathForSession),
            'n' | 'd' => Some(DialogDecision::Deny),
            _ => None,
        }
    }

    /// Render the approval dialog as a centered modal over the given area.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let detail_lines: Vec<&str> = self
            .detail
            .as_deref()
            .map(|d| d.lines().collect())
            .unwrap_or_default();
        let has_detail = !detail_lines.is_empty();

        let popup_w = 72.min((area.width * 80) / 100).max(40);
        let detail_h = (detail_lines.len() as u16).min(14);
        let base_h: u16 = if has_detail { 6 + detail_h } else { 10 };
        let popup_h = base_h.min((area.height * 80) / 100).max(7);
        let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect {
            x,
            y,
            width: popup_w,
            height: popup_h,
        };

        // Clear background underneath
        f.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Approval Required ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .padding(Padding::horizontal(1));

        let inner = block.inner(popup);

        let risk_color = match self.risk_level.as_str() {
            "high" | "critical" => Color::Red,
            "medium" => Color::Yellow,
            _ => Color::Green,
        };

        if has_detail {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(4),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            let header = vec![
                Line::from(vec![
                    Span::styled("Tool: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(&self.tool, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::styled("Risk: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(&self.risk_level, Style::default().fg(risk_color)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    &self.action_summary,
                    Style::default().fg(Color::White),
                )),
            ];
            f.render_widget(Paragraph::new(header), chunks[0]);

            let detail_spans: Vec<Line> = detail_lines
                .iter()
                .skip(self.scroll as usize)
                .take(chunks[1].height as usize)
                .map(|line| {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ))
                })
                .collect();
            f.render_widget(Paragraph::new(detail_spans), chunks[1]);

            let hints = Line::from(vec![
                Span::styled(
                    "[y]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("es  "),
                Span::styled(
                    "[a]",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("lways  "),
                Span::styled(
                    if self.scope_subject.is_some() {
                        "[p]ath  "
                    } else {
                        ""
                    },
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "[N]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("o"),
            ]);
            f.render_widget(Paragraph::new(hints), chunks[2]);
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);

            let lines = vec![
                Line::from(vec![
                    Span::styled("Tool: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(&self.tool, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::styled("Risk: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(&self.risk_level, Style::default().fg(risk_color)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    &self.action_summary,
                    Style::default().fg(Color::White),
                )),
            ];
            f.render_widget(Paragraph::new(lines), chunks[0]);

            let hints = Line::from(vec![
                Span::styled(
                    "[y]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("es  "),
                Span::styled(
                    "[a]",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("lways  "),
                Span::styled(
                    "[N]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("o"),
            ]);
            f.render_widget(Paragraph::new(hints), chunks[1]);
        }

        // Render border (must be last to draw on top)
        f.render_widget(block, popup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_maps_to_decision() {
        assert_eq!(
            ApprovalDialog::key_to_decision('y'),
            Some(DialogDecision::Approve)
        );
        assert_eq!(
            ApprovalDialog::key_to_decision('a'),
            Some(DialogDecision::ApproveForSession)
        );
        assert_eq!(
            ApprovalDialog::key_to_decision('n'),
            Some(DialogDecision::Deny)
        );
        assert_eq!(
            ApprovalDialog::key_to_decision('d'),
            Some(DialogDecision::Deny)
        );
        assert_eq!(ApprovalDialog::key_to_decision('x'), None);
        assert_eq!(ApprovalDialog::key_to_decision(' '), None);
        assert_eq!(ApprovalDialog::key_to_decision('\n'), None);
    }
}
