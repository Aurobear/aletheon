//! ThinkingBlock widget for collapsible thinking display.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub struct ThinkingBlock {
    pub collapsed: bool,
    pub elapsed: f64,
    pub text: String,
}

impl ThinkingBlock {
    pub fn new(elapsed: f64) -> Self {
        Self {
            collapsed: true,
            elapsed,
            text: String::new(),
        }
    }

    pub fn render_collapsed(&self) -> Vec<Line<'static>> {
        let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        vec![
            Line::from(vec![
                Span::styled(format!("✻ Thought for {:.1}s", self.elapsed), style),
            ]),
            Line::from(""),
        ]
    }

    pub fn render_expanded(&self) -> Vec<Line<'static>> {
        let header_style = Style::default().fg(Color::Cyan);
        let dim_style = Style::default().fg(Color::DarkGray);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("✻ Thinking...", header_style),
            ]),
        ];
        for line in self.text.lines() {
            lines.push(Line::from(vec![
                Span::styled("│ ", dim_style),
                Span::raw(line.to_string()),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("({:.1}s)", self.elapsed), dim_style),
        ]));
        lines.push(Line::from(""));
        lines
    }
}
