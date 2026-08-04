//! Keyboard-navigable session picker backed by the daemon's canonical list.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPickerAction {
    Continue,
    Close,
    Resume(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPicker {
    sessions: Vec<String>,
    selected: usize,
    current: Option<String>,
}

impl SessionPicker {
    pub fn from_json(value: &serde_json::Value, current: Option<String>) -> anyhow::Result<Self> {
        let entries = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("session list response is not an array"))?;
        let sessions = entries
            .iter()
            .filter_map(|entry| {
                entry.as_str().map(ToOwned::to_owned).or_else(|| {
                    entry
                        .get("session_id")
                        .or_else(|| entry.get("id"))
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
            })
            .filter(|id| !id.trim().is_empty())
            .collect::<Vec<_>>();
        anyhow::ensure!(!sessions.is_empty(), "no resumable sessions found");
        let selected = current
            .as_ref()
            .and_then(|id| sessions.iter().position(|candidate| candidate == id))
            .unwrap_or(0);
        Ok(Self {
            sessions,
            selected,
            current,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SessionPickerAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => SessionPickerAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                SessionPickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.sessions.len().saturating_sub(1));
                SessionPickerAction::Continue
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                SessionPickerAction::Continue
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.sessions.len().saturating_sub(1);
                SessionPickerAction::Continue
            }
            KeyCode::Enter => SessionPickerAction::Resume(self.sessions[self.selected].clone()),
            _ => SessionPickerAction::Continue,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Never construct a popup outside the authoritative frame. A minimum
        // visual width is desirable on normal terminals, but must not turn a
        // resize-to-tiny event into an out-of-bounds render.
        let width = area.width.saturating_sub(4).max(1).min(84);
        let visible_rows = self.sessions.len().min(15) as u16;
        let height = visible_rows.saturating_add(4).min(area.height);
        let popup = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        Clear.render(popup, buf);

        let content_height = popup.height.saturating_sub(3) as usize;
        let start = self
            .selected
            .saturating_add(1)
            .saturating_sub(content_height)
            .min(self.sessions.len().saturating_sub(content_height));
        let end = (start + content_height).min(self.sessions.len());
        let lines = self.sessions[start..end]
            .iter()
            .enumerate()
            .map(|(offset, id)| {
                let index = start + offset;
                let selected = index == self.selected;
                let current = self.current.as_deref() == Some(id.as_str());
                let marker = if selected { "›" } else { " " };
                let suffix = if current { "  (current)" } else { "" };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if current {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{marker} {id}{suffix}"), style))
            })
            .collect::<Vec<_>>();
        let block = Block::default()
            .title(format!(" Sessions ({}) ", self.sessions.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        Paragraph::new(lines).block(block).render(popup, buf);

        if popup.height >= 2 {
            let footer = Rect::new(
                popup.x + 1,
                popup.y + popup.height - 2,
                popup.width.saturating_sub(2),
                1,
            );
            Line::from(Span::styled(
                "↑/↓ select  Enter resume  Esc close",
                Style::default().fg(Color::DarkGray),
            ))
            .render(footer, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn parses_string_and_object_session_entries() {
        let picker = SessionPicker::from_json(
            &serde_json::json!(["recent", {"session_id":"older"}, {"id":"oldest"}]),
            Some("older".into()),
        )
        .unwrap();
        assert_eq!(picker.sessions, vec!["recent", "older", "oldest"]);
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn navigation_is_bounded_and_enter_returns_selected_id() {
        let mut picker = SessionPicker::from_json(&serde_json::json!(["a", "b"]), None).unwrap();
        assert_eq!(
            picker.handle_key(key(KeyCode::Up)),
            SessionPickerAction::Continue
        );
        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            SessionPickerAction::Resume("b".into())
        );
    }

    #[test]
    fn tiny_terminal_render_stays_inside_frame() {
        let picker = SessionPicker::from_json(&serde_json::json!(["a"]), None).unwrap();
        for area in [Rect::new(0, 0, 1, 1), Rect::new(0, 0, 8, 2)] {
            let mut buffer = ratatui::buffer::Buffer::empty(area);
            picker.render(area, &mut buffer);
            assert_eq!(buffer.area, area);
        }
    }
}
