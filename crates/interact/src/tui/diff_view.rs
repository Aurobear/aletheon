use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

#[derive(Clone, Debug)]
pub struct DiffView {
    pub text: String,
    pub scroll: u16,
}

impl DiffView {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            scroll: 0,
        }
    }
    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }
}

impl Widget for &DiffView {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let mut old_line = 0u64;
        let mut new_line = 0u64;
        let lines = self
            .text
            .lines()
            .map(|text| {
                if let Some(header) = text.strip_prefix("@@") {
                    if let Some((old, new)) = parse_hunk_header(header) {
                        old_line = old;
                        new_line = new;
                    }
                }
                let (marker, old, new, color) = if text.starts_with('+') && !text.starts_with("+++")
                {
                    let current = new_line;
                    new_line += 1;
                    ('+', None, Some(current), Color::Green)
                } else if text.starts_with('-') && !text.starts_with("---") {
                    let current = old_line;
                    old_line += 1;
                    ('-', Some(current), None, Color::Red)
                } else {
                    let current_old = old_line;
                    let current_new = new_line;
                    if !text.starts_with("@@")
                        && !text.starts_with("---")
                        && !text.starts_with("+++")
                    {
                        old_line += 1;
                        new_line += 1;
                    }
                    (
                        ' ',
                        Some(current_old),
                        Some(current_new),
                        if text.starts_with("@@") {
                            Color::Cyan
                        } else {
                            Color::Gray
                        },
                    )
                };
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{:>4} {:>4} {marker} ",
                            old.map_or(String::new(), |v| v.to_string()),
                            new.map_or(String::new(), |v| v.to_string())
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(text.to_string(), Style::default().fg(color)),
                ])
            })
            .collect::<Vec<_>>();
        Paragraph::new(lines)
            .block(Block::default().title(" Diff ").borders(Borders::ALL))
            .scroll((self.scroll, 0))
            .render(area, buffer);
    }
}

fn parse_hunk_header(header: &str) -> Option<(u64, u64)> {
    let body = header.trim().trim_end_matches('@').trim();
    let mut parts = body.split_whitespace();
    let old = parts
        .next()?
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = parts
        .next()?
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_hunk_coordinates() {
        assert_eq!(parse_hunk_header(" -12,2 +20,3 "), Some((12, 20)));
    }
}
