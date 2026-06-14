use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::markdown;
use super::term_compat::{TermCaps, Theme};

/// Role of a chat message.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// A single chat message with cached rendered lines.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Pre-rendered styled lines (cached).
    rendered: Vec<Line<'static>>,
}

impl ChatMessage {
    pub fn new(role: Role, content: String, render_width: u16, caps: &TermCaps) -> Self {
        let theme = caps.theme();
        let rendered = Self::render_lines(&role, &content, render_width, caps, &theme);
        Self {
            role,
            content,
            rendered,
        }
    }

    /// Update content (for streaming) and re-render.
    pub fn update_content(&mut self, content: String, render_width: u16, caps: &TermCaps) {
        self.content = content;
        let theme = caps.theme();
        self.rendered = Self::render_lines(&self.role, &self.content, render_width, caps, &theme);
    }

    fn render_lines(
        role: &Role,
        content: &str,
        width: u16,
        caps: &TermCaps,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        // Border prefix: "  │ " for user/assistant, "  · " for system
        let (border_char, border_color) = match role {
            Role::User => ("│", theme.user_icon),
            Role::Assistant => ("│", theme.accent),
            Role::System => ("·", theme.system_icon),
        };
        let border_prefix = format!("  {} ", border_char);

        // Content area width: total - border prefix width (4 chars)
        let content_width = width.saturating_sub(4) as usize;

        let mut result = Vec::new();

        match role {
            Role::User => {
                // User messages get a subtle background tint
                let text_style = Style::default().fg(theme.text).bg(theme.bg_user);
                let border_style = Style::default().fg(border_color).bg(theme.bg_user);

                // Word-wrap the content
                let plain_line = Line::from(Span::styled(content.to_string(), text_style));
                let wrapped = word_wrap_line_with_prefix(&plain_line, content_width, &border_prefix, border_style);
                for line in wrapped {
                    result.push(line);
                }
            }
            Role::Assistant => {
                let md_lines = markdown::render_markdown(content, content_width as u16, caps);
                for (_i, md_line) in md_lines.into_iter().enumerate() {
                    let mut spans = vec![Span::styled(
                        border_prefix.clone(),
                        Style::default().fg(border_color),
                    )];
                    spans.extend(md_line.spans.into_iter().map(|s| {
                        // Preserve markdown styling, apply theme text color as fallback
                        let style = if s.style.fg.is_some() {
                            s.style
                        } else {
                            s.style.fg(theme.text)
                        };
                        Span::styled(s.content.to_string(), style)
                    }));
                    result.push(Line::from(spans));
                }
            }
            Role::System => {
                let text_style = Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::ITALIC);
                let mut spans = vec![Span::styled(
                    border_prefix.clone(),
                    Style::default().fg(border_color),
                )];
                spans.push(Span::styled(content.to_string(), text_style));
                result.push(Line::from(spans));
            }
        }

        result.push(Line::from(""));
        result
    }
}

/// Chat message display widget with scroll support.
pub struct ChatWidget {
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: u16,
    /// Whether the user has manually scrolled away from the bottom.
    pub user_scrolled: bool,
    render_width: u16,
    caps: TermCaps,
}

impl ChatWidget {
    pub fn new(caps: TermCaps) -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            user_scrolled: false,
            render_width: 80,
            caps,
        }
    }

    /// Add a new message.
    pub fn add_message(&mut self, role: Role, content: String) {
        let msg = ChatMessage::new(role, content, self.render_width, &self.caps);
        self.messages.push(msg);
        if !self.user_scrolled {
            self.scroll_offset = 0;
        }
    }

    /// Update the last message content (for streaming responses).
    pub fn update_last_message(&mut self, content: String) {
        if let Some(last) = self.messages.last_mut() {
            last.update_content(content, self.render_width, &self.caps);
        }
        // During streaming, follow the bottom unless user has manually scrolled
        if !self.user_scrolled {
            self.scroll_offset = 0;
        }
    }

    /// Scroll up by n lines.
    pub fn scroll_up(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
        self.user_scrolled = true;
    }

    /// Scroll down by n lines.
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        if self.scroll_offset == 0 {
            self.user_scrolled = false;
        }
    }

    /// Update render width (on resize).
    pub fn set_width(&mut self, width: u16) {
        if width != self.render_width {
            self.render_width = width;
            let theme = self.caps.theme();
            for msg in &mut self.messages {
                msg.rendered = ChatMessage::render_lines(
                    &msg.role,
                    &msg.content,
                    self.render_width,
                    &self.caps,
                    &theme,
                );
            }
        }
    }

    /// Get all rendered lines (original pre-wrapped).
    pub fn all_lines(&self) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for msg in &self.messages {
            lines.extend(msg.rendered.iter().cloned());
        }
        lines
    }

    /// Get all rendered lines, word-wrapped to fit the given width.
    pub fn all_lines_wrapped(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for msg in &self.messages {
            for line in &msg.rendered {
                lines.extend(word_wrap_line(line, width));
            }
        }
        lines
    }

    /// Render the chat widget.
    pub fn render_widget(&self) -> ChatWidgetRenderer<'_> {
        ChatWidgetRenderer { chat: self }
    }
}

pub struct ChatWidgetRenderer<'a> {
    chat: &'a ChatWidget,
}

impl<'a> Widget for ChatWidgetRenderer<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let all_lines = self.chat.all_lines_wrapped(area.width as usize);
        let total_lines = all_lines.len() as u16;
        let visible_height = area.height;

        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = self.chat.scroll_offset.min(max_scroll);
        let end = total_lines.saturating_sub(scroll);
        let start = end.saturating_sub(visible_height);

        let visible: Vec<Line> = all_lines[start as usize..end as usize].to_vec();

        Paragraph::new(visible)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

/// A character paired with its display style, used during word-wrapping.
#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    style: Style,
}

/// Check if a character is CJK (allows breaking after it).
fn is_cjk(ch: char) -> bool {
    let cp = ch as u32;
    // CJK Unified Ideographs Extension A
    (0x3400..=0x4DBF).contains(&cp)
        // CJK Unified Ideographs
        || (0x4E00..=0x9FFF).contains(&cp)
        // CJK Symbols and Punctuation
        || (0x3000..=0x303F).contains(&cp)
        // Hiragana
        || (0x3040..=0x309F).contains(&cp)
        // Katakana
        || (0x30A0..=0x30FF).contains(&cp)
        // Hangul Syllables
        || (0xAC00..=0xD7AF).contains(&cp)
        // CJK Compatibility Ideographs
        || (0xF900..=0xFAFF).contains(&cp)
        // Fullwidth Latin etc (FF01..FF5E are fullwidth ASCII)
        || (0xFF01..=0xFF5E).contains(&cp)
}

/// Measure the display width of a character (CJK = 2, others = 1).
fn char_display_width(ch: char) -> usize {
    if is_cjk(ch) { 2 } else { 1 }
}

/// Word-wrap a single `Line` with a repeating border prefix on every output line.
///
/// Each output line starts with `prefix` styled with `prefix_style`.
/// The `content_width` is the available width *after* the prefix.
fn word_wrap_line_with_prefix(
    line: &Line<'_>,
    content_width: usize,
    prefix: &str,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    let prefix_width = prefix.chars().count();
    if content_width == 0 || prefix_width >= content_width {
        // Degenerate case: just emit the line as-is with prefix
        let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
        for s in &line.spans {
            spans.push(Span::styled(s.content.to_string(), s.style));
        }
        return vec![Line::from(spans)];
    }

    // Flatten spans into (char, style) pairs
    let mut chars: Vec<StyledChar> = Vec::new();
    for span in &line.spans {
        for ch in span.content.chars() {
            chars.push(StyledChar { ch, style: span.style });
        }
    }

    if chars.is_empty() {
        return vec![Line::from(Span::styled(prefix.to_string(), prefix_style))];
    }

    let total_width: usize = chars.iter().map(|cs| char_display_width(cs.ch)).sum();
    if total_width <= content_width {
        // Fits on one line
        let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
        for sc in &chars {
            spans.push(Span::styled(sc.ch.to_string(), sc.style));
        }
        return vec![Line::from(group_spans(spans))];
    }

    // Greedy word-wrap
    let mut result: Vec<Line<'static>> = Vec::new();
    let mut pos = 0;

    while pos < chars.len() {
        let mut consumed_width = 0usize;
        let mut last_break: usize = pos;
        let mut found_break = false;
        let mut i = pos;

        while i < chars.len() {
            let cw = char_display_width(chars[i].ch);
            if consumed_width + cw > content_width {
                break;
            }
            consumed_width += cw;

            if chars[i].ch == ' ' || chars[i].ch == '\t' {
                last_break = i + 1;
                found_break = true;
            } else if is_cjk(chars[i].ch) {
                last_break = i + 1;
                found_break = true;
            }
            i += 1;
        }

        let end = if i >= chars.len() {
            chars.len()
        } else if found_break {
            last_break
        } else {
            if i == pos {
                (pos + 1).min(chars.len())
            } else {
                i
            }
        };

        let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
        for sc in &chars[pos..end] {
            spans.push(Span::styled(sc.ch.to_string(), sc.style));
        }
        result.push(Line::from(group_spans(spans)));

        pos = end;
        while pos < chars.len() && (chars[pos].ch == ' ' || chars[pos].ch == '\t') {
            pos += 1;
        }
    }

    if result.is_empty() {
        let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
        for sc in &chars {
            spans.push(Span::styled(sc.ch.to_string(), sc.style));
        }
        vec![Line::from(group_spans(spans))]
    } else {
        result
    }
}

/// Group consecutive spans with the same style to reduce span count.
fn group_spans(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    if spans.is_empty() {
        return spans;
    }
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut current_style = spans[0].style;
    let mut current_text = String::new();

    for s in &spans {
        if s.style == current_style {
            current_text.push_str(&s.content);
        } else {
            if !current_text.is_empty() {
                result.push(Span::styled(current_text.clone(), current_style));
            }
            current_style = s.style;
            current_text.clear();
            current_text.push_str(&s.content);
        }
    }
    if !current_text.is_empty() {
        result.push(Span::styled(current_text, current_style));
    }
    result
}

/// Word-wrap a single `Line` into multiple lines that fit within `width`.
///
/// - Breaks at word boundaries (spaces)
/// - CJK characters allow breaking after each character
/// - Continuation lines are indented by 2 spaces
/// - Preserves styling from original `Span`s
pub fn word_wrap_line(line: &Line<'_>, width: usize) -> Vec<Line<'static>> {
    // Helper: convert a borrowed Line to owned (Line<'static>)
    fn to_owned_line(line: &Line<'_>) -> Line<'static> {
        Line::from(
            line.spans
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect::<Vec<_>>(),
        )
    }

    if width == 0 {
        return vec![to_owned_line(line)];
    }

    // Flatten spans into (char, style) pairs
    let mut chars: Vec<StyledChar> = Vec::new();
    for span in &line.spans {
        for ch in span.content.chars() {
            chars.push(StyledChar {
                ch,
                style: span.style,
            });
        }
    }

    if chars.is_empty() {
        return vec![to_owned_line(line)];
    }

    // Calculate total display width
    let total_width: usize = chars.iter().map(|cs| char_display_width(cs.ch)).sum();
    if total_width <= width {
        return vec![to_owned_line(line)];
    }

    // Greedy word-wrap
    let indent_width: usize = 2;
    let mut result: Vec<Line<'static>> = Vec::new();
    let mut pos = 0;
    let mut first_line = true;

    while pos < chars.len() {
        let available = if first_line {
            width
        } else {
            width.saturating_sub(indent_width)
        };

        if available == 0 {
            // Width too small; just output everything on one line
            let spans = build_spans(&chars[pos..], 0);
            result.push(Line::from(spans));
            break;
        }

        // Find the best break point within `available` width
        let mut consumed_width = 0usize;
        let mut last_break: usize = pos; // last position we can break at
        let mut found_break = false;
        let mut i = pos;

        while i < chars.len() {
            let cw = char_display_width(chars[i].ch);
            if consumed_width + cw > available {
                break;
            }
            consumed_width += cw;

            // Mark break opportunities: after spaces or after CJK chars
            if chars[i].ch == ' ' || chars[i].ch == '\t' {
                last_break = i + 1;
                found_break = true;
            } else if is_cjk(chars[i].ch) {
                last_break = i + 1;
                found_break = true;
            }
            i += 1;
        }

        let end = if i >= chars.len() {
            // All remaining chars fit on this line
            chars.len()
        } else if found_break {
            last_break
        } else {
            // No break opportunity; force break at current position
            if i == pos {
                // At least consume one character to avoid infinite loop
                (pos + 1).min(chars.len())
            } else {
                i
            }
        };

        let indent = if first_line { 0 } else { indent_width };
        let spans = build_spans(&chars[pos..end], indent);
        result.push(Line::from(spans));

        // Advance past the break point, skipping trailing spaces
        pos = end;
        while pos < chars.len() && (chars[pos].ch == ' ' || chars[pos].ch == '\t') {
            pos += 1;
        }

        first_line = false;
    }

    if result.is_empty() {
        vec![to_owned_line(line)]
    } else {
        result
    }
}

/// Build styled spans from a slice of `StyledChar` with optional indent.
fn build_spans(chars: &[StyledChar], indent: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    if indent > 0 {
        spans.push(Span::raw(" ".repeat(indent)));
    }

    if chars.is_empty() {
        return spans;
    }

    // Group consecutive chars with the same style into spans
    let mut current_style = chars[0].style;
    let mut current_text = String::new();

    for sc in chars {
        if sc.style == current_style {
            current_text.push(sc.ch);
        } else {
            if !current_text.is_empty() {
                spans.push(Span::styled(current_text.clone(), current_style));
            }
            current_style = sc.style;
            current_text.clear();
            current_text.push(sc.ch);
        }
    }
    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, current_style));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_wrap_short_line_unchanged() {
        let line = Line::from("hello");
        let result = word_wrap_line(&line, 80);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_word_wrap_long_line() {
        let line = Line::from("hello world this is a very long line that should be wrapped at word boundaries");
        let result = word_wrap_line(&line, 20);
        assert!(result.len() > 1);
        // Each line should fit within 20 columns
        for wrapped in &result {
            let width: usize = wrapped
                .spans
                .iter()
                .flat_map(|s| s.content.chars())
                .map(char_display_width)
                .sum();
            assert!(width <= 20, "line too wide: {} chars", width);
        }
    }

    #[test]
    fn test_word_wrap_cjk_breaks() {
        let line = Line::from("这是一个很长的中文句子，需要在字符边界处断行");
        let result = word_wrap_line(&line, 10);
        assert!(result.len() > 1);
    }

    #[test]
    fn test_word_wrap_preserves_style() {
        let line = Line::from(vec![
            Span::styled("hello ", Style::default().fg(Color::Red)),
            Span::styled("world", Style::default().fg(Color::Blue)),
        ]);
        let result = word_wrap_line(&line, 80);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].spans.len(), 2);
    }

    #[test]
    fn test_word_wrap_continuation_indent() {
        let line = Line::from("first second third fourth fifth");
        let result = word_wrap_line(&line, 10);
        assert!(result.len() >= 2);
        // Second line should start with 2-space indent
        let second = &result[1];
        assert!(!second.spans.is_empty());
        let first_span_content = second.spans[0].content.as_ref();
        assert!(
            first_span_content.starts_with("  "),
            "continuation should be indented, got: {:?}",
            first_span_content
        );
    }

    #[test]
    fn test_all_lines_wrapped() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_message(Role::User, "hello world".to_string());
        let wrapped = widget.all_lines_wrapped(80);
        assert!(!wrapped.is_empty());
    }

    #[test]
    fn test_user_scrolled_flag() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_message(Role::User, "msg1".to_string());
        assert!(!widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 0);

        widget.scroll_up(5);
        assert!(widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 5);

        // New message should not reset scroll when user has scrolled
        widget.add_message(Role::User, "msg2".to_string());
        assert_eq!(widget.scroll_offset, 5);

        // Scroll to bottom should clear user_scrolled
        widget.scroll_down(5);
        assert!(!widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 0);
    }

    #[test]
    fn test_word_wrap_empty_line() {
        let line = Line::from("");
        let result = word_wrap_line(&line, 20);
        assert_eq!(result.len(), 1);
    }
}
