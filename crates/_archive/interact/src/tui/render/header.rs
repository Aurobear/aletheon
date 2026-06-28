use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::super::term_compat::TermCaps;

pub fn render_header(
    f: &mut ratatui::Frame,
    area: Rect,
    caps: &TermCaps,
    model_name: &str,
    show_full: bool,
) {
    let bg = caps.color(20, 20, 60);

    if show_full {
        let vsep = if caps.unicode { "  │  " } else { "  |  " };
        let line1 = Line::from(Span::styled(
            "  aletheon v0.1.0",
            Style::default().fg(Color::White),
        ));
        let line2 = Line::from(Span::styled(
            format!("  model: {model_name}{vsep}connected"),
            Style::default().fg(Color::DarkGray),
        ));
        let hints = if caps.unicode {
            "  Shift+Enter 换行 │ Enter 发送 │ Ctrl+C 退出 │ /help"
        } else {
            "  Shift+Enter newline | Enter send | Ctrl+C quit | /help"
        };
        let line3 = Line::from(Span::styled(hints, Style::default().fg(Color::DarkGray)));

        let header = Paragraph::new(vec![line1, line2, line3]).style(Style::default().bg(bg));
        f.render_widget(header, area);
    } else {
        let title = format!("  aletheon  │  {model_name}");
        let line = Line::from(Span::styled(title, Style::default().fg(Color::White)));
        let header = Paragraph::new(line).style(Style::default().bg(bg));
        f.render_widget(header, area);
    }
}
