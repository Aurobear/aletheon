use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::event::Action;

use super::term_compat::TermCaps;

/// Text input area with cursor, history navigation.
pub struct InputArea {
    /// Current input buffer.
    buffer: String,
    /// Cursor position within the buffer (byte index).
    cursor: usize,
    /// Command history for up/down navigation.
    history: Vec<String>,
    /// Current position in history (None = not navigating).
    history_idx: Option<usize>,
    /// Temporary storage for the current input when navigating history.
    saved_input: String,
    /// Model name displayed in the hint line.
    pub model_name: String,
}

impl InputArea {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: None,
            saved_input: String::new(),
            model_name: String::new(),
        }
    }

    /// Handle a key event, returning an Action.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        // Ctrl+C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }

        // Ctrl+L: enter IME input mode (for Chinese/Japanese/Korean input)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
            return Action::Command("/input".to_string());
        }

        match key.code {
            // Submit
            KeyCode::Enter => {
                let text = self.buffer.trim().to_string();
                if text.is_empty() {
                    return Action::None;
                }
                self.history.push(text.clone());
                self.history_idx = None;
                self.buffer.clear();
                self.cursor = 0;
                // Intercept /commands
                if text.starts_with('/') {
                    return Action::Command(text);
                }
                Action::Submit(text)
            }

            // Character input
            KeyCode::Char(c) => {
                self.insert_char(c);
                Action::None
            }

            // Backspace
            KeyCode::Backspace => {
                self.delete_before_cursor();
                Action::None
            }

            // Delete
            KeyCode::Delete => {
                self.delete_after_cursor();
                Action::None
            }

            // Cursor movement
            KeyCode::Left => {
                self.move_cursor_left();
                Action::None
            }
            KeyCode::Right => {
                self.move_cursor_right();
                Action::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                Action::None
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
                Action::None
            }

            // History navigation
            KeyCode::Up => {
                self.history_prev();
                Action::None
            }
            KeyCode::Down => {
                self.history_next();
                Action::None
            }

            // Page up/down for chat scroll
            KeyCode::PageUp => Action::ScrollUp(5),
            KeyCode::PageDown => Action::ScrollDown(5),

            _ => Action::None,
        }
    }

    fn insert_char(&mut self, c: char) {
        // Ensure cursor is at a valid char boundary
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the previous char boundary
        let prev = self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buffer.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    fn delete_after_cursor(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        // Find the next char boundary
        let next = self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buffer.len());
        self.buffer.replace_range(self.cursor..next, "");
    }

    fn move_cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.cursor = prev;
    }

    fn move_cursor_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buffer.len());
        self.cursor = next;
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            Some(i) => {
                if i == 0 {
                    return;
                }
                i - 1
            }
            None => {
                self.saved_input = self.buffer.clone();
                self.history.len() - 1
            }
        };
        self.history_idx = Some(idx);
        self.buffer = self.history[idx].clone();
        self.cursor = self.buffer.len();
    }

    fn history_next(&mut self) {
        match self.history_idx {
            Some(i) => {
                if i + 1 >= self.history.len() {
                    self.history_idx = None;
                    self.buffer = self.saved_input.clone();
                } else {
                    self.history_idx = Some(i + 1);
                    self.buffer = self.history[i + 1].clone();
                }
                self.cursor = self.buffer.len();
            }
            None => {}
        }
    }

    /// Get the current buffer content (for display).
    #[allow(dead_code)]
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Render the input area widget.
    pub fn render_widget<'a>(&'a self, caps: &'a TermCaps) -> InputWidget<'a> {
        InputWidget { input: self, caps }
    }
}

pub struct InputWidget<'a> {
    input: &'a InputArea,
    caps: &'a TermCaps,
}

impl<'a> Widget for InputWidget<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Need at least 3 rows: border, input, hint
        if area.height < 3 {
            // Fallback: render just the input line
            self.render_input_line(area, buf, area.y);
            return;
        }

        let border_y = area.y;
        let input_y = area.y + 1;
        let hint_y = area.y + 2;

        // Row 1: top border line
        let hline = if self.caps.unicode { "─" } else { "-" };
        let prefix = "  ";
        let line_w = area.width.saturating_sub(prefix.len() as u16) as usize;
        let border_str = format!("{}{}", prefix, hline.repeat(line_w));
        for (i, ch) in border_str.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.x + area.width {
                break;
            }
            buf[(x, border_y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::DarkGray));
        }

        // Row 2: input line
        self.render_input_line(area, buf, input_y);

        // Row 3: hint line (model name + key hints)
        let prompt_indent = if self.caps.unicode { "  ❯ " } else { "  > " };
        let hint_offset = prompt_indent.len() as u16;

        let model_str = if !self.input.model_name.is_empty() {
            format!("model: {}", self.input.model_name)
        } else {
            String::new()
        };
        let sep = if self.caps.unicode { "   " } else { "   " };
        let hints = if self.caps.unicode {
            "Ctrl+L 中文 │ /help"
        } else {
            "Ctrl+L CJK | /help"
        };

        // Render hint text at the same indent as input content
        let mut hx = area.x + hint_offset;
        // Model name in dim
        for ch in model_str.chars() {
            if hx >= area.x + area.width {
                break;
            }
            buf[(hx, hint_y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::DarkGray));
            hx += 1;
        }
        // Separator
        for ch in sep.chars() {
            if hx >= area.x + area.width {
                break;
            }
            buf[(hx, hint_y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::DarkGray));
            hx += 1;
        }
        // Hints in dim
        for ch in hints.chars() {
            if hx >= area.x + area.width {
                break;
            }
            buf[(hx, hint_y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::DarkGray));
            hx += 1;
        }
    }
}

impl<'a> InputWidget<'a> {
    fn render_input_line(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, y: u16) {
        let prompt = if self.caps.unicode { "❯ " } else { "> " };
        let prompt_len = prompt.chars().count() as u16;
        let indent = "  ";
        let indent_len = indent.len() as u16;

        // Render indent
        for (i, ch) in indent.chars().enumerate() {
            if i as u16 >= area.width {
                break;
            }
            buf[(area.x + i as u16, y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default());
        }

        // Render prompt
        for (i, ch) in prompt.chars().enumerate() {
            let x = area.x + indent_len + i as u16;
            if x >= area.x + area.width {
                break;
            }
            buf[(x, y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::Green));
        }

        // Render buffer content
        let content_x = area.x + indent_len + prompt_len;
        let buffer_display = self.input.buffer.as_str();

        for (i, ch) in buffer_display.chars().enumerate() {
            let x = content_x + i as u16;
            if x >= area.x + area.width {
                break;
            }
            buf[(x, y)]
                .set_symbol(&ch.to_string())
                .set_style(Style::default().fg(Color::White));
        }

        // Render cursor (block cursor)
        let cursor_char_idx = buffer_display[..self.input.cursor.min(buffer_display.len())]
            .chars()
            .count();
        let cursor_x = content_x + cursor_char_idx as u16;
        if cursor_x < area.x + area.width {
            buf[(cursor_x, y)]
                .set_symbol(
                    buffer_display[self.input.cursor..]
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| " ".to_string())
                        .as_str(),
                )
                .set_style(Style::default().fg(Color::Black).bg(Color::White));
        }
    }
}
