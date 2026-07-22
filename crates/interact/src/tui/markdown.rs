use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use super::term_compat::{TermCaps, Theme};

/// Render markdown text into styled ratatui lines.
///
/// Uses `pulldown-cmark` for CommonMark + GFM parsing and `syntect` for code
/// syntax highlighting. Supports headings, bold/italic/strikethrough, inline
/// code, fenced code blocks, bullet lists, blockquotes, tables, links, rules,
/// and task lists.
pub fn render_markdown(text: &str, width: u16, caps: &TermCaps) -> Vec<Line<'static>> {
    let theme = caps.theme();
    render_markdown_with_theme(text, width, caps, &theme)
}

fn render_markdown_with_theme(
    text: &str,
    width: u16,
    caps: &TermCaps,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, options);

    let ss = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    let syntect_theme = &ts.themes["base16-ocean.dark"];

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_highlighter: Option<HighlightLines> = None;
    let mut code_lines: Vec<Line<'static>> = Vec::new();
    let mut list_depth: usize = 0;
    let mut table_row: Vec<Vec<Span<'static>>> = Vec::new();
    // Whole-table buffer: all rows (header first) are collected, then emitted
    // aligned on TagEnd::Table once per-column widths are known.
    let mut table_rows: Vec<Vec<Vec<Span<'static>>>> = Vec::new();
    let mut table_header_count: usize = 0;

    let accent = theme.accent;
    let dim = theme.text_muted;
    let code_fg = if caps.true_color {
        Color::Rgb(180, 200, 220)
    } else {
        Color::Cyan
    };
    let code_bg = theme.code_bg;

    let wrap_width = width.saturating_sub(2) as usize;

    for event in parser {
        match event {
            Event::Start(tag) => match &tag {
                Tag::Heading { level, .. } => {
                    flush_spans(&mut current_spans, &mut lines);
                    let style = match level {
                        HeadingLevel::H1 | HeadingLevel::H2 => {
                            Style::default().fg(accent).add_modifier(Modifier::BOLD)
                        }
                        _ => Style::default().fg(accent).add_modifier(Modifier::ITALIC),
                    };
                    style_stack.push(style);
                }
                Tag::Paragraph => {
                    flush_spans(&mut current_spans, &mut lines);
                }
                Tag::CodeBlock(kind) => {
                    flush_spans(&mut current_spans, &mut lines);
                    in_code_block = true;
                    code_lines.clear();
                    match kind {
                        CodeBlockKind::Fenced(info) => {
                            let lang = info.split_whitespace().next().unwrap_or("");
                            code_block_lang = Some(lang.to_string());
                            code_highlighter = ss
                                .find_syntax_by_token(lang)
                                .map(|syntax| HighlightLines::new(syntax, syntect_theme));
                        }
                        CodeBlockKind::Indented => {
                            code_block_lang = None;
                            code_highlighter = None;
                        }
                    }
                }
                Tag::List(_) => {
                    flush_spans(&mut current_spans, &mut lines);
                    list_depth += 1;
                }
                Tag::Item => {
                    flush_spans(&mut current_spans, &mut lines);
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    current_spans.push(Span::raw(indent));
                    current_spans.push(Span::styled(
                        caps.bullet().to_string(),
                        Style::default().fg(accent),
                    ));
                    current_spans.push(Span::raw(" "));
                }
                Tag::BlockQuote(_) => {
                    flush_spans(&mut current_spans, &mut lines);
                    current_spans.push(Span::styled(
                        format!("{} ", caps.vline()),
                        Style::default().fg(dim),
                    ));
                }
                Tag::Table(_) => {
                    flush_spans(&mut current_spans, &mut lines);
                    table_rows.clear();
                    table_header_count = 0;
                }
                Tag::TableHead => {
                    table_row.clear();
                }
                Tag::TableRow => {
                    table_row.clear();
                }
                Tag::TableCell => {
                    current_spans.clear();
                }
                Tag::Emphasis => {
                    let mut style = *style_stack.last().unwrap_or(&Style::default());
                    style = style.add_modifier(Modifier::ITALIC);
                    style_stack.push(style);
                }
                Tag::Strong => {
                    let mut style = *style_stack.last().unwrap_or(&Style::default());
                    style = style.add_modifier(Modifier::BOLD);
                    style_stack.push(style);
                }
                Tag::Strikethrough => {
                    style_stack.push(Style::default().add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::Link { .. } => {
                    style_stack.push(
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                }
                _ => {}
            },
            Event::End(tag_end) => {
                match tag_end {
                    TagEnd::Heading(level) => {
                        flush_spans(&mut current_spans, &mut lines);
                        style_stack.pop();
                        // Add underline for H1
                        if level == HeadingLevel::H1 {
                            let hline = caps.hline().repeat(wrap_width.min(40));
                            lines
                                .push(Line::from(Span::styled(hline, Style::default().fg(accent))));
                        }
                        lines.push(Line::from(""));
                    }
                    TagEnd::Paragraph => {
                        flush_spans(&mut current_spans, &mut lines);
                        lines.push(Line::from(""));
                    }
                    TagEnd::CodeBlock => {
                        let gutter = Style::default().fg(dim);
                        let accent_style = Style::default().fg(accent);

                        // Top border with language label (accent color for dashes)
                        let lang = code_block_lang.as_deref().unwrap_or("");
                        let dash_count = wrap_width.min(60).saturating_sub(lang.len() + 2);
                        let mut top_spans = vec![
                            Span::raw("  "),
                            Span::styled(caps.hline().to_string(), accent_style),
                            Span::raw(" "),
                        ];
                        if !lang.is_empty() {
                            top_spans.push(Span::styled(format!("{lang} "), accent_style));
                        }
                        top_spans.push(Span::styled(caps.hline().repeat(dash_count), accent_style));
                        top_spans.push(Span::raw(" "));
                        lines.push(Line::from(top_spans));

                        // Code lines with line numbers: "  │  N │ code"
                        let line_num_width = format!("{}", code_lines.len()).len();
                        for (idx, line) in code_lines.iter().enumerate() {
                            let num = idx + 1;
                            let num_str = format!("{num:>line_num_width$}");
                            let mut spans = vec![
                                Span::raw("  "),
                                Span::styled(format!("{} ", caps.vline()), gutter),
                                Span::styled(format!("{num_str:>line_num_width$} "), gutter),
                                Span::styled(format!("{} ", caps.vline()), gutter),
                            ];
                            spans.extend(line.spans.clone());
                            lines.push(Line::from(spans));
                        }

                        // Bottom border (accent color for dashes)
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                format!("{} ", caps.hline().repeat(wrap_width.min(60))),
                                accent_style,
                            ),
                        ]));
                        lines.push(Line::from(""));

                        in_code_block = false;
                        code_block_lang = None;
                        code_highlighter = None;
                        code_lines.clear();
                    }
                    TagEnd::List(_) => {
                        list_depth = list_depth.saturating_sub(1);
                        if list_depth == 0 {
                            flush_spans(&mut current_spans, &mut lines);
                            lines.push(Line::from(""));
                        }
                    }
                    TagEnd::Item => {
                        flush_spans(&mut current_spans, &mut lines);
                    }
                    TagEnd::BlockQuote(_) => {
                        flush_spans(&mut current_spans, &mut lines);
                    }
                    TagEnd::TableHead => {
                        // Buffer the header row; the separator + alignment are
                        // emitted once at TagEnd::Table when all widths are known.
                        table_rows.push(std::mem::take(&mut table_row));
                        table_header_count = table_rows.len();
                    }
                    TagEnd::TableRow => {
                        table_rows.push(std::mem::take(&mut table_row));
                    }
                    TagEnd::TableCell => {
                        let cell_spans: Vec<Span<'static>> = std::mem::take(&mut current_spans);
                        table_row.push(cell_spans);
                    }
                    TagEnd::Table => {
                        // Two-pass layout: compute per-column widths across ALL
                        // rows, then emit aligned rows with borders + a header
                        // separator. Fixes misaligned columns / broken separator
                        // rows (bug T3).
                        let ncols = table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
                        let mut col_w = vec![0usize; ncols];
                        for row in &table_rows {
                            for (c, cell) in row.iter().enumerate() {
                                let w: usize = cell.iter().map(|s| s.width()).sum();
                                if w > col_w[c] {
                                    col_w[c] = w;
                                }
                            }
                        }
                        let table_width = 2 + col_w.iter().map(|width| width + 2).sum::<usize>();
                        if table_width > wrap_width {
                            let headers = table_rows.first().cloned().unwrap_or_default();
                            for row in table_rows.iter().skip(table_header_count) {
                                for (column, cell) in row.iter().enumerate() {
                                    let label = headers
                                        .get(column)
                                        .map(|spans| {
                                            spans
                                                .iter()
                                                .map(|span| span.content.as_ref())
                                                .collect::<String>()
                                        })
                                        .filter(|label| !label.trim().is_empty())
                                        .unwrap_or_else(|| format!("Column {}", column + 1));
                                    let mut spans = vec![
                                        Span::styled(
                                            if column == 0 { "• " } else { "  " }.to_string(),
                                            Style::default().fg(accent),
                                        ),
                                        Span::styled(
                                            format!("{}: ", label.trim()),
                                            Style::default().fg(dim).add_modifier(Modifier::BOLD),
                                        ),
                                    ];
                                    spans.extend(cell.clone());
                                    lines.push(Line::from(spans));
                                }
                                lines.push(Line::from(""));
                            }
                        } else {
                            for (ridx, row) in table_rows.iter().enumerate() {
                                let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
                                for (c, &col_width) in col_w.iter().enumerate() {
                                    let cell = row.get(c);
                                    let w: usize = cell
                                        .map(|cs| cs.iter().map(|s| s.width()).sum())
                                        .unwrap_or(0);
                                    if let Some(cell) = cell {
                                        spans.extend(cell.clone());
                                    }
                                    if w < col_width {
                                        spans.push(Span::raw(" ".repeat(col_width - w)));
                                    }
                                    if c + 1 < col_w.len() {
                                        spans.push(Span::raw("  "));
                                    }
                                }
                                lines.push(Line::from(spans));
                                // Emit the header/body separator right after the
                                // last header row.
                                if ridx + 1 == table_header_count {
                                    let mut sep: Vec<Span<'static>> = vec![Span::raw("  ")];
                                    for (c, &col_width) in col_w.iter().enumerate() {
                                        sep.push(Span::styled(
                                            "─".repeat(col_width),
                                            Style::default().fg(dim),
                                        ));
                                        if c + 1 < col_w.len() {
                                            sep.push(Span::raw("  "));
                                        }
                                    }
                                    lines.push(Line::from(sep));
                                }
                            }
                        }
                        table_rows.clear();
                        table_header_count = 0;
                    }
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                        style_stack.pop();
                    }
                    _ => {}
                }
            }
            Event::Text(text) => {
                if in_code_block {
                    // Syntax highlight or plain text
                    if let Some(ref mut highlighter) = code_highlighter {
                        for line_text in LinesWithEndings::from(&text) {
                            if let Ok(ranges) = highlighter.highlight_line(line_text, &ss) {
                                let spans: Vec<Span<'static>> = ranges
                                    .iter()
                                    .map(|(style, s)| {
                                        let fg = Color::Rgb(
                                            style.foreground.r,
                                            style.foreground.g,
                                            style.foreground.b,
                                        );
                                        Span::styled(s.to_string(), Style::default().fg(fg))
                                    })
                                    .collect();
                                code_lines.push(Line::from(spans));
                            } else {
                                code_lines.push(Line::from(Span::styled(
                                    line_text.to_string(),
                                    Style::default().fg(code_fg),
                                )));
                            }
                        }
                    } else {
                        for line_text in LinesWithEndings::from(&text) {
                            code_lines.push(Line::from(Span::styled(
                                line_text.trim_end_matches('\n').to_string(),
                                Style::default().fg(code_fg),
                            )));
                        }
                    }
                } else {
                    let style = *style_stack.last().unwrap_or(&Style::default());
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(code) => {
                let style = Style::default().fg(code_fg).bg(code_bg);
                current_spans.push(Span::styled(format!(" {code} "), style));
            }
            Event::Rule => {
                flush_spans(&mut current_spans, &mut lines);
                let hline = caps.hline().repeat(wrap_width.min(60));
                lines.push(Line::from(Span::styled(hline, Style::default().fg(dim))));
                lines.push(Line::from(""));
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    // handled by LinesWithEndings
                } else {
                    current_spans.push(Span::raw(" "));
                }
            }
            _ => {}
        }
    }

    flush_spans(&mut current_spans, &mut lines);

    // Remove trailing empty lines
    while lines
        .last()
        .is_some_and(|l| l.spans.is_empty() || l.spans.iter().all(|s| s.content.is_empty()))
    {
        lines.pop();
    }

    lines
}

fn flush_spans(spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>) {
    if !spans.is_empty() {
        let line_spans: Vec<Span<'static>> = std::mem::take(spans);
        lines.push(Line::from(line_spans));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_caps() -> TermCaps {
        TermCaps {
            true_color: true,
            unicode: true,
            width: 80,
            height: 24,
        }
    }

    #[test]
    fn test_heading() {
        let caps = test_caps();
        let lines = render_markdown("# Hello", 80, &caps);
        // H1 produces heading text + underline + blank = at least 2 lines
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_bold() {
        let caps = test_caps();
        let lines = render_markdown("Hello **world**", 80, &caps);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_code_block() {
        let caps = test_caps();
        let input = "```rust\nfn main() {}\n```";
        let lines = render_markdown(input, 80, &caps);
        // top border + code line + bottom border + blank = 4
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_bullet_list() {
        let caps = test_caps();
        let lines = render_markdown("- item 1\n- item 2", 80, &caps);
        // 2 items + trailing blank from list end
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_inline_code() {
        let caps = test_caps();
        let lines = render_markdown("Use `foo` here", 80, &caps);
        assert!(!lines.is_empty());
        let spans = &lines[0].spans;
        // Should have at least 3 spans: "Use ", "`foo` styled", " here"
        assert!(spans.len() >= 3);
    }

    #[test]
    fn test_strikethrough() {
        let caps = test_caps();
        let lines = render_markdown("~~deleted~~", 80, &caps);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_table_aligned() {
        let caps = test_caps();
        let input = "| Lang | Use |\n|------|-----|\n| Rust | sys |\n| Go | web |";
        let lines = render_markdown(input, 80, &caps);
        let text = |l: &Line| -> String {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        };
        let tbl: Vec<String> = lines.iter().map(&text).filter(|t| !t.is_empty()).collect();
        // header + separator + 2 body rows.
        assert_eq!(tbl.len(), 4, "table lines: {tbl:?}");
        assert!(!tbl.iter().any(|line| line.contains('│')));
        // All rows align to the same display width (ignoring trailing space).
        let w0 = tbl[0].trim_end().chars().count();
        for t in &tbl {
            assert_eq!(t.trim_end().chars().count(), w0, "misaligned: {tbl:?}");
        }
        // Row index 1 is the header separator: only box-drawing chars, no text.
        let sep = &tbl[1];
        assert!(sep.contains('─'), "no separator dashes: {sep:?}");
        assert!(
            !sep.chars().any(|c| c.is_alphabetic()),
            "separator has text: {sep:?}"
        );
        // No raw markdown table syntax leaked as literal characters.
        assert!(
            !tbl.iter().any(|t| t.contains('|') || t.contains("---")),
            "raw markdown leaked: {tbl:?}"
        );
    }

    #[test]
    fn test_wide_table_falls_back_to_stacked_rows() {
        let caps = test_caps();
        let input = "| Sender | Subject | Summary |\n|---|---|---|\n| Anthropic | Your account has been suspended | A very long explanation that cannot fit in a narrow terminal |";
        let lines = render_markdown(input, 40, &caps);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text.iter().any(|line| line.contains("Sender: Anthropic")));
        assert!(text
            .iter()
            .any(|line| line.contains("Subject: Your account")));
        assert!(!text.iter().any(|line| line.contains('│')));
    }

    #[test]
    fn test_rule() {
        let caps = test_caps();
        let lines = render_markdown("---", 80, &caps);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let caps = test_caps();
        let lines = render_markdown("", 80, &caps);
        assert!(lines.is_empty());
    }
}
