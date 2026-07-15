//! Chat widget with mixed text and executable tool entries.
//!
//! # Entry Types
//!
//! * `ChatEntry::Text` — Standard text messages (user, assistant, system).
//! * `ChatEntry::Exec` — Tool call + result, rendered inline with spinner animation.

use std::cell::{Cell, RefCell};

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
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
        // Keep user/system messages visually distinct, but render assistant
        // prose without a repeated rail (Codex-style text-first hierarchy).
        let (border_char, border_color) = match role {
            Role::User => ("│", theme.user_icon),
            Role::Assistant => ("", theme.accent),
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
                let wrapped = word_wrap_line_with_prefix(
                    &plain_line,
                    content_width,
                    &border_prefix,
                    border_style,
                );
                for line in wrapped {
                    result.push(line);
                }
            }
            Role::Assistant => {
                let md_lines = markdown::render_markdown(content, width, caps);
                for md_line in md_lines.into_iter() {
                    let spans = md_line
                        .spans
                        .into_iter()
                        .map(|s| {
                            // Preserve markdown styling, apply theme text color as fallback
                            let style = if s.style.fg.is_some() {
                                s.style
                            } else {
                                s.style.fg(theme.text)
                            };
                            Span::styled(s.content.to_string(), style)
                        })
                        .collect::<Vec<_>>();
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

    /// Render lines for a given width (used by ChatEntry::Text rendering).
    pub fn rendered_lines(&self, width: u16, caps: &TermCaps) -> Vec<Line<'static>> {
        let theme = caps.theme();
        Self::render_lines(&self.role, &self.content, width, caps, &theme)
    }
}

// ── ExecEntry — tool call inline rendering ────────────────────────────

/// Rendered tool execution entry in the chat history.
///
/// Displays a spinner during execution, then shows truncated output with
/// expand/collapse support.  Rendered in chat order alongside text messages.
pub struct ExecEntry {
    pub call_id: String,
    pub tool: String,
    pub args: String,
    pub output: String,
    pub is_error: bool,
    pub finished: bool,
    pub expanded: bool,
}

impl ExecEntry {
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

    pub fn update_args(&mut self, args: &str) {
        self.args = args.to_string();
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Build animated styled lines — copies the `render_chat_lines` logic from toolcard.rs.
    pub fn render_lines(&self, frame_counter: u64, _width: u16) -> Vec<Line<'static>> {
        const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        let dot_color = if self.is_error {
            Color::Red
        } else if !self.finished {
            Color::Yellow
        } else {
            tool_color(&self.tool)
        };

        let status = if !self.finished {
            format!(" {}", SPINNER[frame_counter as usize % SPINNER.len()])
        } else if self.is_error {
            " ✗".to_string()
        } else {
            String::new()
        };

        let header = format!("{}{}", tool_action(&self.tool, &self.args), status);
        let mut lines = vec![Line::from(vec![
            Span::styled("• ", Style::default().fg(dot_color)),
            Span::raw(header),
        ])];

        if self.expanded && !self.output.is_empty() {
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
        } else if self.finished && self.is_error && !self.output.is_empty() {
            for line in self.output.lines().take(3) {
                lines.push(Line::from(vec![
                    Span::styled("  └ ", Style::default().fg(Color::Red)),
                    Span::raw(line.to_string()),
                ]));
            }
        }

        lines
    }

    pub fn to_summary(&self) -> String {
        let status = if self.is_error { "failed" } else { "done" };
        format!(
            "  ⏺ {}({}) — {}",
            self.tool,
            truncate_args(&self.args, 40),
            status
        )
    }
}

fn tool_action(tool: &str, args: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(args).unwrap_or_default();
    let get = |key: &str| value.get(key).and_then(|item| item.as_str()).unwrap_or("");
    let lower = tool.to_ascii_lowercase();
    let (verb, detail) = if lower.contains("bash") || lower.contains("exec") {
        ("Ran", get("command"))
    } else if lower.contains("read") {
        ("Read", get("path"))
    } else if lower.contains("grep") || lower.contains("search") || lower.contains("glob") {
        let detail = if !get("pattern").is_empty() {
            get("pattern")
        } else {
            get("query")
        };
        ("Searched", detail)
    } else if lower.contains("write") || lower.contains("edit") || lower.contains("patch") {
        ("Updated", get("path"))
    } else {
        ("Used", tool)
    };
    let detail = if detail.is_empty() { tool } else { detail };
    format!("{verb} {}", truncate_args(detail, 76))
}

/// Assign a color to a tool based on its name.
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

/// Truncate tool args to `max` chars, appending "…" if needed.
fn truncate_args(args: &str, max: usize) -> String {
    if args.chars().count() <= max {
        args.to_string()
    } else {
        format!("{}...", args.chars().take(max).collect::<String>())
    }
}

// ── ChatEntry enum ────────────────────────────────────────────────────

use ratatui::style::Color;

/// A single entry in the chat history — either a text message or a tool execution.
pub enum ChatEntry {
    Text(ChatMessage),
    Exec(ExecEntry),
}

impl ChatEntry {
    /// Render this entry into styled lines for display.
    fn render_lines(&self, frame_counter: u64, width: u16, caps: &TermCaps) -> Vec<Line<'static>> {
        match self {
            ChatEntry::Text(msg) => msg.rendered_lines(width, caps),
            ChatEntry::Exec(entry) => entry.render_lines(frame_counter, width),
        }
    }
}

// ── ChatWidget ────────────────────────────────────────────────────────

/// Chat message display widget with scroll support.
pub struct ChatWidget {
    /// Mixed entries: text messages and tool executions in display order.
    pub entries: Vec<ChatEntry>,
    pub scroll_offset: u16,
    /// Whether the user has manually scrolled away from the bottom.
    pub user_scrolled: bool,
    render_width: u16,
    caps: TermCaps,
    revision: Cell<u64>,
    layout_cache: RefCell<Option<LayoutCache>>,
}

#[derive(Clone)]
struct LayoutCache {
    revision: u64,
    width: usize,
    animation_frame: Option<u64>,
    lines: Vec<Line<'static>>,
}

impl ChatWidget {
    pub fn new(caps: TermCaps) -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            user_scrolled: false,
            render_width: 80,
            caps,
            revision: Cell::new(0),
            layout_cache: RefCell::new(None),
        }
    }

    fn invalidate_layout(&self) {
        self.revision.set(self.revision.get().wrapping_add(1));
        self.layout_cache.borrow_mut().take();
    }

    /// Add a text message entry.
    pub fn add_text(&mut self, role: Role, content: String) {
        let msg = ChatMessage::new(role, content, self.render_width, &self.caps);
        self.entries.push(ChatEntry::Text(msg));
        self.invalidate_layout();
        if !self.user_scrolled {
            self.scroll_offset = 0;
        }
    }

    /// Add a tool execution entry (in-progress).
    pub fn add_exec(&mut self, call_id: String, tool: String, args: String) {
        let entry = ExecEntry::new(call_id, tool, args);
        self.entries.push(ChatEntry::Exec(entry));
        self.invalidate_layout();
        if !self.user_scrolled {
            self.scroll_offset = 0;
        }
    }

    /// Set the current assistant response content, creating a trailing
    /// assistant text entry if the last entry isn't already one.
    ///
    /// Unlike a reverse search, this always writes to the END of the list, so
    /// the assistant entry is appended AFTER any Exec (tool) and System
    /// (reflection) entries that arrived during the turn. This keeps the final
    /// answer rendering chronologically *after* the analysis log, and prevents
    /// streamed/final text from overwriting an earlier placeholder or a System
    /// notice (bugs T1 + "final answer above the log").
    pub fn set_assistant_stream(&mut self, content: String) {
        let trailing_is_assistant = matches!(
            self.entries.last(),
            Some(ChatEntry::Text(m)) if m.role == Role::Assistant
        );
        if !trailing_is_assistant {
            self.add_text(Role::Assistant, String::new());
        }
        if let Some(ChatEntry::Text(msg)) = self.entries.last_mut() {
            msg.update_content(content, self.render_width, &self.caps);
        }
        self.invalidate_layout();
        if !self.user_scrolled {
            self.scroll_offset = 0;
        }
    }

    /// Remove a transient assistant draft when the same model iteration
    /// proceeds to a tool call. Only the final, tool-free iteration should be
    /// retained as the user-facing answer.
    pub fn discard_trailing_assistant_draft(&mut self) {
        if matches!(
            self.entries.last(),
            Some(ChatEntry::Text(message)) if message.role == Role::Assistant
        ) {
            self.entries.pop();
            self.invalidate_layout();
        }
    }

    /// Update a tool execution entry by call_id (mark as finished with output).
    pub fn update_exec(&mut self, call_id: &str, output: &str, is_error: bool) {
        let mut changed = false;
        for entry in self.entries.iter_mut() {
            if let ChatEntry::Exec(ref mut ee) = entry {
                if ee.call_id == call_id {
                    ee.finish(output, is_error);
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.invalidate_layout();
        }
    }

    /// Update a tool execution entry's args by call_id.
    pub fn update_exec_args(&mut self, call_id: &str, args: &str) {
        let mut changed = false;
        for entry in self.entries.iter_mut() {
            if let ChatEntry::Exec(ref mut ee) = entry {
                if ee.call_id == call_id {
                    ee.update_args(args);
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.invalidate_layout();
        }
    }

    /// Toggle expand/collapse on a tool execution entry by call_id.
    pub fn toggle_exec(&mut self, call_id: &str) -> bool {
        let mut changed = false;
        for entry in self.entries.iter_mut() {
            if let ChatEntry::Exec(ref mut ee) = entry {
                if ee.call_id == call_id {
                    ee.toggle();
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.invalidate_layout();
        }
        changed
    }

    /// Count unfinished exec entries (still running).
    pub fn active_exec_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Exec(ee) if !ee.finished))
            .count()
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

    /// Update render width (on resize). Re-renders Text entries only.
    pub fn set_width(&mut self, width: u16) {
        if width != self.render_width {
            self.render_width = width;
            for entry in &mut self.entries {
                if let ChatEntry::Text(msg) = entry {
                    let theme = self.caps.theme();
                    msg.rendered = ChatMessage::render_lines(
                        &msg.role,
                        &msg.content,
                        self.render_width,
                        &self.caps,
                        &theme,
                    );
                }
            }
            self.invalidate_layout();
        }
    }

    /// Get all rendered lines (for the current frame, with animations).
    pub fn all_lines(&self, frame_counter: u64) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for entry in &self.entries {
            lines.extend(entry.render_lines(frame_counter, self.render_width, &self.caps));
        }
        lines
    }

    /// Get all rendered lines, word-wrapped to fit the given width.
    pub fn all_lines_wrapped(&self, frame_counter: u64, width: usize) -> Vec<Line<'static>> {
        self.ensure_cache(frame_counter, width);
        self.layout_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.lines.clone())
            .unwrap_or_default()
    }

    fn ensure_cache(&self, frame_counter: u64, width: usize) {
        let animation_frame = self
            .active_exec_count()
            .gt(&0)
            .then_some(frame_counter % 10);
        let revision = self.revision.get();
        if let Some(cache) = self.layout_cache.borrow().as_ref() {
            if cache.revision == revision
                && cache.width == width
                && cache.animation_frame == animation_frame
            {
                return;
            }
        }

        let mut lines = Vec::new();
        for entry in &self.entries {
            for line in entry.render_lines(frame_counter, self.render_width, &self.caps) {
                lines.extend(word_wrap_line(&line, width));
            }
        }
        *self.layout_cache.borrow_mut() = Some(LayoutCache {
            revision,
            width,
            animation_frame,
            lines: lines.clone(),
        });
    }

    pub fn visible_lines(
        &self,
        frame_counter: u64,
        width: usize,
        height: u16,
    ) -> Vec<Line<'static>> {
        self.ensure_cache(frame_counter, width);
        let cache = self.layout_cache.borrow();
        let lines = &cache.as_ref().expect("layout cache initialized").lines;
        let total = lines.len();
        let scroll = usize::from(self.scroll_offset).min(total.saturating_sub(height as usize));
        let end = total.saturating_sub(scroll);
        let start = end.saturating_sub(height as usize);
        lines[start..end].to_vec()
    }

    /// Render the chat widget.
    pub fn render_widget<'a>(&'a self, frame_counter: u64) -> ChatWidgetRenderer<'a> {
        ChatWidgetRenderer {
            chat: self,
            frame_counter,
        }
    }
}

pub struct ChatWidgetRenderer<'a> {
    chat: &'a ChatWidget,
    frame_counter: u64,
}

impl<'a> Widget for ChatWidgetRenderer<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let visible = self
            .chat
            .visible_lines(self.frame_counter, area.width as usize, area.height);

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
    if is_cjk(ch) {
        2
    } else {
        1
    }
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
            chars.push(StyledChar {
                ch,
                style: span.style,
            });
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
    use ratatui::style::Color;

    #[test]
    fn test_word_wrap_short_line_unchanged() {
        let line = Line::from("hello");
        let result = word_wrap_line(&line, 80);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_word_wrap_long_line() {
        let line = Line::from(
            "hello world this is a very long line that should be wrapped at word boundaries",
        );
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
        widget.add_text(Role::User, "hello world".to_string());
        let wrapped = widget.all_lines_wrapped(0, 80);
        assert!(!wrapped.is_empty());
    }

    #[test]
    fn test_scrolling_reuses_wrapped_layout() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_text(Role::Assistant, "one two three four five".repeat(50));
        let _ = widget.visible_lines(0, 30, 5);
        let first_ptr = widget
            .layout_cache
            .borrow()
            .as_ref()
            .unwrap()
            .lines
            .as_ptr();
        widget.scroll_up(3);
        let _ = widget.visible_lines(99, 30, 5);
        let second_ptr = widget
            .layout_cache
            .borrow()
            .as_ref()
            .unwrap()
            .lines
            .as_ptr();
        assert_eq!(first_ptr, second_ptr, "scrolling must reuse cached layout");
    }

    #[test]
    fn test_assistant_message_has_no_repeated_vertical_rail() {
        let caps = TermCaps::detect();
        let message = ChatMessage::new(Role::Assistant, "first\n\nsecond".into(), 80, &caps);
        let text = message
            .rendered
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!text.contains('│'));
    }

    #[test]
    fn test_user_scrolled_flag() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_text(Role::User, "msg1".to_string());
        assert!(!widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 0);

        widget.scroll_up(5);
        assert!(widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 5);

        // New message should not reset scroll when user has scrolled
        widget.add_text(Role::User, "msg2".to_string());
        assert_eq!(widget.scroll_offset, 5);

        // Scroll to bottom should clear user_scrolled
        widget.scroll_down(5);
        assert!(!widget.user_scrolled);
        assert_eq!(widget.scroll_offset, 0);
    }

    #[test]
    fn test_exec_entry_spinner() {
        let mut entry = ExecEntry::new(
            "call_1".to_string(),
            "bash_exec".to_string(),
            r#"{"command": "ls"}"#.to_string(),
        );
        let lines = entry.render_lines(0, 80);
        assert!(!lines.is_empty());
        // Should contain the spinner animation
        let header_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            header_text.contains("⠋"),
            "should have spinner, got: {}",
            header_text
        );
        assert!(!entry.finished);

        entry.finish("file1.txt\nfile2.txt", false);
        assert!(entry.finished);
        let lines2 = entry.render_lines(0, 80);
        let header2: String = lines2[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(header2, "• Ran ls");
    }

    #[test]
    fn test_exec_entry_toggle() {
        let mut entry = ExecEntry::new(
            "call_2".to_string(),
            "file_read".to_string(),
            r#"{"path": "/tmp/x"}"#.to_string(),
        );
        assert!(!entry.expanded);
        entry.toggle();
        assert!(entry.expanded);
        entry.toggle();
        assert!(!entry.expanded);
    }

    #[test]
    fn test_exec_entry_collapses_large_single_line_output() {
        let mut entry = ExecEntry::new(
            "call_json".to_string(),
            "google_gmail_search".to_string(),
            r#"{"account":"aurobear-gmail"}"#.to_string(),
        );
        let raw = format!(r#"{{"messages":["{}"]}}"#, "x".repeat(4_000));
        entry.finish(&raw, false);

        let rendered = entry.render_lines(0, 80);
        let text = rendered
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(text, "• Searched google_gmail_search");
        assert!(!text.contains(&"x".repeat(100)));
    }

    #[test]
    fn test_exec_entry_shows_failure_excerpt() {
        let mut entry = ExecEntry::new(
            "call_git".to_string(),
            "bash_exec".to_string(),
            r#"{"command":"git status"}"#.to_string(),
        );
        entry.finish("fatal: repository unavailable\nmore detail", true);
        let text = entry
            .render_lines(0, 80)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("• Ran git status ✗"));
        assert!(text.contains("fatal: repository unavailable"));
    }

    #[test]
    fn test_truncate_args_respects_utf8_boundaries() {
        let truncated = truncate_args("看看我最新7天的gmail", 6);
        assert_eq!(truncated, "看看我最新7...");
    }

    #[test]
    fn test_update_last_text_skips_exec() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_exec("c1".to_string(), "bash".to_string(), "{}".to_string());
        widget.set_assistant_stream("new content".to_string());
        // Exec entry stays; a fresh assistant entry is appended AFTER it.
        assert!(matches!(widget.entries.first(), Some(ChatEntry::Exec(_))));
        assert!(matches!(
            widget.entries.last(),
            Some(ChatEntry::Text(m)) if m.role == Role::Assistant && m.content == "new content"
        ));
    }

    #[test]
    fn test_assistant_stream_lands_after_logs() {
        // Bug: final answer must render AFTER exec + reflection entries, and a
        // mid-stream Reflection (System) must not capture assistant text.
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_text(Role::User, "do it".to_string());
        widget.add_exec("c1".to_string(), "bash".to_string(), "{}".to_string());
        widget.set_assistant_stream("partial".to_string()); // first delta after tool
        widget.add_text(Role::System, "Reflection: 10 tool calls".to_string());
        widget.set_assistant_stream("final answer".to_string()); // more deltas

        let kinds: Vec<(&str, String)> = widget
            .entries
            .iter()
            .map(|e| match e {
                ChatEntry::Text(m) => (
                    match m.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System => "system",
                    },
                    m.content.clone(),
                ),
                ChatEntry::Exec(_) => ("exec", String::new()),
            })
            .collect();
        // Order: user, exec, assistant(partial), system(reflection), assistant(final).
        // A NEW assistant entry is appended after the reflection (System is not
        // a trailing assistant), so the final answer sits at the very end.
        assert_eq!(kinds.first().map(|k| k.0), Some("user"));
        assert_eq!(kinds.get(1).map(|k| k.0), Some("exec"));
        assert_eq!(
            kinds.last(),
            Some(&("assistant", "final answer".to_string()))
        );
    }

    #[test]
    fn test_tool_call_discards_transient_assistant_draft() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.set_assistant_stream("I will inspect that first".into());
        widget.discard_trailing_assistant_draft();
        widget.add_exec("c1".into(), "file_read".into(), "{}".into());
        assert_eq!(widget.entries.len(), 1);
        assert!(matches!(widget.entries[0], ChatEntry::Exec(_)));
    }

    #[test]
    fn test_update_exec_by_call_id() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_exec("c1".to_string(), "bash".to_string(), "{}".to_string());
        widget.update_exec("c1", "output text", false);
        // Verify the entry was finished
        match &widget.entries[0] {
            ChatEntry::Exec(ee) => {
                assert!(ee.finished);
                assert!(!ee.is_error);
                assert_eq!(ee.output, "output text");
            }
            _ => panic!("expected Exec entry"),
        }
    }

    #[test]
    fn test_active_exec_count() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        assert_eq!(widget.active_exec_count(), 0);
        widget.add_exec("c1".to_string(), "bash".to_string(), "{}".to_string());
        assert_eq!(widget.active_exec_count(), 1);
        widget.add_exec("c2".to_string(), "read".to_string(), "{}".to_string());
        assert_eq!(widget.active_exec_count(), 2);
        widget.update_exec("c1", "done", false);
        assert_eq!(widget.active_exec_count(), 1);
    }

    #[test]
    fn test_toggle_exec_by_call_id() {
        let caps = TermCaps::detect();
        let mut widget = ChatWidget::new(caps);
        widget.add_exec("c1".to_string(), "bash".to_string(), "{}".to_string());
        // Toggle should work
        assert!(widget.toggle_exec("c1"));
        match &widget.entries[0] {
            ChatEntry::Exec(ee) => assert!(ee.expanded),
            _ => panic!("expected Exec entry"),
        }
        // Non-existent call_id returns false
        assert!(!widget.toggle_exec("no_such_call"));
    }

    #[test]
    fn test_word_wrap_empty_line() {
        let line = Line::from("");
        let result = word_wrap_line(&line, 20);
        assert_eq!(result.len(), 1);
    }
}
