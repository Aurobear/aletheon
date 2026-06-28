use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::super::term_compat::TermCaps;

pub fn render_input(
    f: &mut ratatui::Frame,
    area: Rect,
    caps: &TermCaps,
    buf: &str,
    cursor: usize,
    has_cjk: bool,
) {
    let border_h = caps.hline();
    let prompt = if caps.unicode { "❯ " } else { "> " };

    // Row 0: separator line
    let sep = format!(
        "  {}",
        border_h.repeat(area.width.saturating_sub(4) as usize)
    );
    let sep_line = Line::from(Span::styled(sep, Style::default().fg(Color::DarkGray)));
    f.render_widget(Paragraph::new(sep_line), Rect { height: 1, ..area });

    // Row 1: input text with cursor
    let input_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    let mut spans = vec![Span::styled(prompt, Style::default().fg(Color::Green))];

    // Split buffer at cursor for cursor display
    let before = &buf[..cursor.min(buf.len())];
    let after = &buf[cursor.min(buf.len())..];

    if !before.is_empty() {
        spans.push(Span::styled(before, Style::default().fg(Color::White)));
    }

    // Cursor character (reverse video)
    let cursor_char = after
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    spans.push(Span::styled(
        cursor_char,
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));

    let rest = if after.chars().count() > 1 {
        &after[after
            .char_indices()
            .nth(1)
            .map(|(i, _)| i)
            .unwrap_or(after.len())..]
    } else {
        ""
    };
    if !rest.is_empty() {
        spans.push(Span::styled(rest, Style::default().fg(Color::White)));
    }

    let input_line = Paragraph::new(Line::from(spans));
    f.render_widget(input_line, input_area);

    // Row 2: hint line (Claude/Codex style - minimal)
    let hint_area = Rect {
        y: area.y + 2,
        height: 1,
        ..area
    };
    let hint = "  \\ + Enter 换行 │ /help 帮助";
    let hint_line = Paragraph::new(Line::from(Span::styled(
        hint,
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(hint_line, hint_area);
}
