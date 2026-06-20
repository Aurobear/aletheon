//! Ctrl+R reverse history search overlay.
//!
//! A popup that filters command history with fuzzy subsequence matching,
//! similar to bash Ctrl+R.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// History search overlay state.
pub struct HistorySearchOverlay {
    /// Current search query.
    query: String,
    /// Cursor position in query (byte index).
    cursor: usize,
    /// All history entries (in original order).
    history: Vec<String>,
    /// Filtered results: (original_index, entry_text).
    results: Vec<(usize, String)>,
    /// Currently highlighted result index.
    selected: usize,
    /// Scroll offset for the results list.
    scroll: usize,
    /// Whether the overlay is closed.
    pub closed: bool,
    /// The chosen entry (set on Enter).
    chosen: Option<String>,
}

impl HistorySearchOverlay {
    /// Create a new history search overlay with the given history entries.
    pub fn new(history: Vec<String>) -> Self {
        // Show all entries reversed (newest first) as initial results
        let results: Vec<(usize, String)> = history
            .iter()
            .enumerate()
            .rev()
            .map(|(i, s)| (i, s.clone()))
            .collect();

        Self {
            query: String::new(),
            cursor: 0,
            history,
            results,
            selected: 0,
            scroll: 0,
            closed: false,
            chosen: None,
        }
    }

    /// Handle a key event. Returns true when the overlay should close.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // Enter: select current entry and close
            KeyCode::Enter => {
                if let Some((_, entry)) = self.results.get(self.selected) {
                    self.chosen = Some(entry.clone());
                }
                self.closed = true;
                return true;
            }

            // Esc: close without selection
            KeyCode::Esc => {
                self.closed = true;
                return true;
            }

            // Up: navigate results
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.adjust_scroll();
                }
            }

            // Down: navigate results
            KeyCode::Down => {
                if self.selected + 1 < self.results.len() {
                    self.selected += 1;
                    self.adjust_scroll();
                }
            }

            // Backspace: delete char before cursor
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let prev = self.query[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.query.replace_range(prev..self.cursor, "");
                    self.cursor = prev;
                    self.filter();
                }
            }

            // Delete: delete char at cursor
            KeyCode::Delete => {
                if self.cursor < self.query.len() {
                    let next = self.query[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.query.len());
                    self.query.replace_range(self.cursor..next, "");
                    self.filter();
                }
            }

            // Left: move cursor
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = self.query[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }

            // Right: move cursor
            KeyCode::Right => {
                if self.cursor < self.query.len() {
                    self.cursor = self.query[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.query.len());
                }
            }

            // Home
            KeyCode::Home => self.cursor = 0,

            // End
            KeyCode::End => self.cursor = self.query.len(),

            // Character input (skip control characters)
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.query.insert(self.cursor, c);
                    self.cursor += c.len_utf8();
                    self.filter();
                }
            }

            _ => {}
        }

        false
    }

    /// Returns the chosen entry if one was selected on Enter.
    pub fn selected_entry(&self) -> Option<String> {
        self.chosen.clone()
    }

    /// Filter results using fuzzy subsequence match (case-insensitive).
    fn filter(&mut self) {
        if self.query.is_empty() {
            self.results = self
                .history
                .iter()
                .enumerate()
                .rev()
                .map(|(i, s)| (i, s.clone()))
                .collect();
            self.selected = 0;
            self.scroll = 0;
            return;
        }

        let query_lower: Vec<char> = self.query.to_lowercase().chars().collect();
        let mut matched: Vec<(usize, String, usize)> = self
            .history
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                fuzzy_subsequence_match(&query_lower, entry).map(|score| (i, entry.clone(), score))
            })
            .collect();

        // Sort: shorter entries with same match rank higher (lower score = better)
        matched.sort_by_key(|&(_, ref entry, score)| (score, entry.len()));

        self.results = matched.into_iter().map(|(i, s, _)| (i, s)).collect();
        self.selected = 0;
        self.scroll = 0;
    }

    /// Adjust scroll so selected item is visible.
    fn adjust_scroll(&mut self) {
        // We'll use a fixed max visible height; the render call will tell us the actual height.
        // Use a generous default here; the render clamps anyway.
        let max_visible = 20;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + max_visible {
            self.scroll = self.selected - max_visible + 1;
        }
    }

    /// Render the overlay into a full-screen area using a Buffer.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Popup dimensions
        let popup_width = (area.width * 3 / 4).min(80).max(40);
        let popup_height = (area.height * 2 / 3).min(30).max(10);
        let popup_x = area.x + (area.width - popup_width) / 2;
        let popup_y = area.y + (area.height - popup_height) / 2;
        let popup = Rect {
            x: popup_x,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        // Dim the background behind the popup
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_style(Style::default().bg(Color::Black));
            }
        }

        // Draw popup border
        self.draw_border(popup, buf);

        // Inner area (inside border)
        let inner = Rect {
            x: popup.x + 1,
            y: popup.y + 1,
            width: popup.width.saturating_sub(2),
            height: popup.height.saturating_sub(2),
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // Row 0: Header
        let header_text = " History Search ";
        let header_x = inner.x + (inner.width.saturating_sub(header_text.len() as u16)) / 2;
        self.render_text(
            buf,
            header_x,
            inner.y,
            header_text,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        // Row 1: separator
        if inner.height > 1 {
            for x in inner.x..inner.x + inner.width {
                buf[(x, inner.y + 1)]
                    .set_symbol("-")
                    .set_style(Style::default().fg(Color::DarkGray));
            }
        }

        // Row 2: query line
        if inner.height > 2 {
            let query_y = inner.y + 2;
            let mut x = inner.x;

            // Green prompt
            buf[(x, query_y)]
                .set_symbol(">")
                .set_style(Style::default().fg(Color::Green));
            x += 1;
            buf[(x, query_y)]
                .set_symbol(" ")
                .set_style(Style::default());
            x += 1;

            // Query text
            for ch in self.query.chars() {
                if x >= inner.x + inner.width - 1 {
                    break;
                }
                buf[(x, query_y)]
                    .set_symbol(&ch.to_string())
                    .set_style(Style::default().fg(Color::White));
                x += 1;
            }

            // Block cursor
            if x < inner.x + inner.width - 1 {
                buf[(x, query_y)]
                    .set_symbol("\u{2588}")
                    .set_style(Style::default().fg(Color::White));
            }
        }

        // Row 3: separator
        if inner.height > 3 {
            for x in inner.x..inner.x + inner.width {
                buf[(x, inner.y + 3)]
                    .set_symbol("-")
                    .set_style(Style::default().fg(Color::DarkGray));
            }
        }

        // Rows 4..height-2: results
        let results_start = inner.y + 4;
        let results_end = inner.y + inner.height.saturating_sub(1);
        let max_visible = results_end.saturating_sub(results_start) as usize;

        if max_visible > 0 {
            for vi in 0..max_visible {
                let result_idx = self.scroll + vi;
                let y = results_start + vi as u16;
                if y >= results_end {
                    break;
                }

                if let Some((_, entry)) = self.results.get(result_idx) {
                    let is_selected = result_idx == self.selected;

                    // Fill background for selected item
                    if is_selected {
                        for x in inner.x..inner.x + inner.width {
                            buf[(x, y)]
                                .set_symbol(" ")
                                .set_style(Style::default().bg(Color::DarkGray));
                        }
                    }

                    // Render entry text (truncate to fit)
                    let max_text_width = inner.width.saturating_sub(2) as usize;
                    let display_text: String = entry.chars().take(max_text_width).collect();
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    self.render_text(buf, inner.x + 1, y, &display_text, style);
                }
            }
        }

        // Last row: footer
        if inner.height > 1 {
            let footer_y = inner.y + inner.height - 1;
            let footer_text = " \u{2191}/\u{2193}  Enter select  Esc cancel ";
            let footer_x = inner.x + (inner.width.saturating_sub(footer_text.chars().count() as u16)) / 2;
            self.render_text(
                buf,
                footer_x,
                footer_y,
                footer_text,
                Style::default().fg(Color::DarkGray),
            );
        }
    }

    fn draw_border(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let x0 = area.x;
        let y0 = area.y;
        let x1 = area.x + area.width.saturating_sub(1);
        let y1 = area.y + area.height.saturating_sub(1);

        let border_style = Style::default().fg(Color::DarkGray);

        // Top and bottom edges
        for x in x0 + 1..x1 {
            buf[(x, y0)]
                .set_symbol("\u{2500}")
                .set_style(border_style);
            buf[(x, y1)]
                .set_symbol("\u{2500}")
                .set_style(border_style);
        }
        // Left and right edges
        for y in y0 + 1..y1 {
            buf[(x0, y)]
                .set_symbol("\u{2502}")
                .set_style(border_style);
            buf[(x1, y)]
                .set_symbol("\u{2502}")
                .set_style(border_style);
        }
        // Corners
        buf[(x0, y0)]
            .set_symbol("\u{256D}")
            .set_style(border_style);
        buf[(x1, y0)]
            .set_symbol("\u{256E}")
            .set_style(border_style);
        buf[(x0, y1)]
            .set_symbol("\u{2570}")
            .set_style(border_style);
        buf[(x1, y1)]
            .set_symbol("\u{256F}")
            .set_style(border_style);
    }

    fn render_text(&self, buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, text: &str, style: Style) {
        let mut cx = x;
        for ch in text.chars() {
            if cx >= buf.area.x + buf.area.width {
                break;
            }
            buf[(cx, y)]
                .set_symbol(&ch.to_string())
                .set_style(style);
            cx += 1;
        }
    }
}

/// Fuzzy subsequence match: each char of query must appear in order in the entry.
/// Returns a score (lower = better match) or None if no match.
fn fuzzy_subsequence_match(query: &[char], entry: &str) -> Option<usize> {
    let entry_lower: Vec<char> = entry.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut first_match_pos = None;

    for (ei, &ec) in entry_lower.iter().enumerate() {
        if qi < query.len() && ec == query[qi] {
            if first_match_pos.is_none() {
                first_match_pos = Some(ei);
            }
            qi += 1;
        }
    }

    if qi == query.len() {
        // Score based on: position of first match (earlier = better) and entry length
        let first = first_match_pos.unwrap_or(0);
        Some(first + entry.len())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_subsequence_match() {
        let query: Vec<char> = "git".chars().collect();
        assert!(fuzzy_subsequence_match(&query, "git status").is_some());
        assert!(fuzzy_subsequence_match(&query, "git commit -m").is_some());
        assert!(fuzzy_subsequence_match(&query, "ls -la").is_none());
        assert!(fuzzy_subsequence_match(&query, "do git push").is_some());
    }

    #[test]
    fn test_fuzzy_case_insensitive() {
        let query: Vec<char> = "git".chars().collect();
        assert!(fuzzy_subsequence_match(&query, "GIT STATUS").is_some());
    }

    #[test]
    fn test_filter_empty_query_shows_all() {
        let history = vec!["first".to_string(), "second".to_string(), "third".to_string()];
        let overlay = HistorySearchOverlay::new(history);
        assert_eq!(overlay.results.len(), 3);
        // Newest first
        assert_eq!(overlay.results[0].1, "third");
        assert_eq!(overlay.results[2].1, "first");
    }

    #[test]
    fn test_filter_with_query() {
        let history = vec![
            "git status".to_string(),
            "ls -la".to_string(),
            "git commit".to_string(),
        ];
        let mut overlay = HistorySearchOverlay::new(history);
        overlay.query = "git".to_string();
        overlay.filter();
        assert_eq!(overlay.results.len(), 2);
    }

    #[test]
    fn test_enter_selects() {
        let history = vec!["hello".to_string(), "world".to_string()];
        let mut overlay = HistorySearchOverlay::new(history);
        overlay.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(overlay.closed);
        // With empty query, all results shown, first (newest) selected
        assert_eq!(overlay.selected_entry(), Some("world".to_string()));
    }

    #[test]
    fn test_esc_closes_without_selection() {
        let history = vec!["hello".to_string()];
        let mut overlay = HistorySearchOverlay::new(history);
        overlay.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(overlay.closed);
        assert!(overlay.selected_entry().is_none());
    }

    #[test]
    fn test_typing_filters() {
        let history = vec![
            "git status".to_string(),
            "ls -la".to_string(),
            "git push".to_string(),
        ];
        let mut overlay = HistorySearchOverlay::new(history);
        // Type 'g'
        overlay.handle_key(KeyEvent::from(KeyCode::Char('g')));
        assert_eq!(overlay.results.len(), 2);
        // Type 'i'
        overlay.handle_key(KeyEvent::from(KeyCode::Char('i')));
        assert_eq!(overlay.results.len(), 2);
        // Type 't'
        overlay.handle_key(KeyEvent::from(KeyCode::Char('t')));
        assert_eq!(overlay.results.len(), 2);
        // Type 'p'
        overlay.handle_key(KeyEvent::from(KeyCode::Char('p')));
        assert_eq!(overlay.results.len(), 1);
        assert_eq!(overlay.results[0].1, "git push");
    }

    #[test]
    fn test_backspace_refilters() {
        let history = vec![
            "git status".to_string(),
            "ls -la".to_string(),
        ];
        let mut overlay = HistorySearchOverlay::new(history);
        overlay.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(overlay.results.len(), 1); // "ls -la"
        overlay.handle_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(overlay.results.len(), 2); // all again
    }
}
