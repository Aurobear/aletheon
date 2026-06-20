//! ToolCard widget for tool call display.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

const COLLAPSE_LINES: usize = 3;

pub struct ToolCard {
    pub call_id: String,
    pub tool: String,
    pub args: String,
    pub output: String,
    pub is_error: bool,
    pub finished: bool,
    pub expanded: bool,
}

impl ToolCard {
    pub fn new(call_id: String, tool: String, args: String) -> Self {
        Self {
            call_id,
            tool,
            args,
            output: String::new(),
            is_error: false,
            finished: false,
            expanded: false,
        }
    }

    pub fn finish(&mut self, output: &str, is_error: bool) {
        self.output = output.to_string();
        self.is_error = is_error;
        self.finished = true;
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn render(&self) -> Vec<Line<'static>> {
        let dot_color = if self.is_error {
            Color::Red
        } else if !self.finished {
            Color::Yellow
        } else {
            tool_color(&self.tool)
        };

        let status = if !self.finished {
            " ⠋".to_string()
        } else if self.is_error {
            " ✗".to_string()
        } else {
            " ✓".to_string()
        };

        let header = format!("⏺ {}({}){}", self.tool, truncate_args(&self.args, 60), status);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("● ", Style::default().fg(dot_color)),
                Span::raw(header),
            ]),
        ];

        if self.expanded {
            let output_lines: Vec<&str> = self.output.lines().collect();
            let display = if output_lines.len() > 200 {
                &output_lines[..200]
            } else {
                &output_lines
            };
            for line in display {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::raw(line.to_string()),
                ]));
            }
            if output_lines.len() > 200 {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("... ({} lines total)", output_lines.len()), Style::default().fg(Color::DarkGray)),
                ]));
            }
        } else if self.finished {
            let line_count = self.output.lines().count();
            if line_count > COLLAPSE_LINES {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{} lines, Ctrl+B to expand", line_count), Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                for line in self.output.lines().take(COLLAPSE_LINES) {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
        }

        lines.push(Line::from(""));
        lines
    }

    /// Render tool card lines compatible with the chat widget's visual language.
    ///
    /// Uses braille spinner animation for in-progress tools and the same `│`
    /// border prefix as assistant messages.
    pub fn render_chat_lines(&self, frame_counter: u64, expanded: bool) -> Vec<Line<'static>> {
        const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        let dot_color = if self.is_error {
            Color::Red
        } else if !self.finished {
            Color::Yellow
        } else {
            tool_color(&self.tool)
        };

        let status = if !self.finished {
            SPINNER[frame_counter as usize % SPINNER.len()].to_string()
        } else if self.is_error {
            " ✗".to_string()
        } else {
            " ✓".to_string()
        };

        let header = format!(
            "  ⏺ {}({}){}",
            self.tool,
            truncate_args(&self.args, 60),
            status
        );
        let mut lines = vec![Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("● ", Style::default().fg(dot_color)),
            Span::raw(header),
        ])];

        if expanded && !self.output.is_empty() {
            let output_lines: Vec<&str> = self.output.lines().collect();
            let display = if output_lines.len() > 10 {
                &output_lines[..10]
            } else {
                &output_lines
            };
            for line in display {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::raw(line.to_string()),
                ]));
            }
            if output_lines.len() > 10 {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("... ({} lines total)", output_lines.len()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        } else if self.finished {
            let line_count = self.output.lines().count();
            if line_count > 3 {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} lines output, Ctrl+B to expand", line_count),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            } else if line_count > 0 {
                for line in self.output.lines().take(3) {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
        }

        lines
    }

    pub fn to_summary(&self) -> String {
        let status = if self.is_error { "failed" } else { "done" };
        format!("  ⏺ {}({}) — {}", self.tool, truncate_args(&self.args, 40), status)
    }
}

fn tool_color(tool: &str) -> Color {
    let lower = tool.to_lowercase();
    if lower.contains("read") || lower.contains("glob") || lower.contains("grep") {
        Color::Cyan
    } else if lower.contains("write") || lower.contains("edit") || lower.contains("apply") {
        Color::Green
    } else if lower.contains("bash") || lower.contains("shell") || lower.contains("exec") {
        Color::Yellow
    } else {
        Color::Magenta
    }
}

fn truncate_args(args: &str, max: usize) -> String {
    if args.len() <= max {
        args.to_string()
    } else {
        format!("{}...", &args[..max])
    }
}
