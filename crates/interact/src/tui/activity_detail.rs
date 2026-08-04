use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use super::{chat::ExecEntry, term_compat::TermCaps};

/// Wide-screen inspection panel for the selected authoritative tool activity.
pub struct ActivityDetail<'a> {
    pub entry: &'a ExecEntry,
    pub caps: &'a TermCaps,
}

impl Widget for ActivityDetail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 24 || area.height < 6 {
            return;
        }
        let status = if !self.entry.finished {
            ("RUNNING", Color::Yellow)
        } else if self.entry.is_error {
            ("FAILED / DENIED", Color::Red)
        } else {
            ("SUCCEEDED", Color::Green)
        };
        let muted = self.caps.color(120, 120, 130);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Status  ", Style::default().fg(muted)),
                Span::styled(
                    status.0,
                    Style::default().fg(status.1).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Tool    ", Style::default().fg(muted)),
                Span::raw(self.entry.tool.clone()),
            ]),
            Line::from(vec![
                Span::styled("Call ID ", Style::default().fg(muted)),
                Span::raw(self.entry.call_id.clone()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Arguments",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ];
        lines.extend(
            self.entry
                .args
                .lines()
                .take(8)
                .map(|line| Line::from(line.to_owned())),
        );
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Result",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if self.entry.output.is_empty() {
            lines.push(Line::from(Span::styled("—", Style::default().fg(muted))));
        } else {
            lines.extend(
                self.entry
                    .output
                    .lines()
                    .take(area.height.saturating_sub(10) as usize)
                    .map(|line| Line::from(line.to_owned())),
            );
        }
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Activity detail "),
            )
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}
