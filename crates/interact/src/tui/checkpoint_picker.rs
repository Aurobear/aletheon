//! Keyboard picker for daemon-owned workspace checkpoints.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointPickerAction {
    Continue,
    Close,
    RewindCode {
        prompt_index: u64,
    },
    ForkSession {
        through_sequence: u64,
    },
    ForkAndRewind {
        prompt_index: u64,
        through_sequence: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointPicker {
    entries: Vec<::contracts::CheckpointListEntry>,
    selected: usize,
}

impl CheckpointPicker {
    pub fn from_snapshot(snapshot: ::contracts::CheckpointListSnapshot) -> anyhow::Result<Self> {
        anyhow::ensure!(
            snapshot.schema_version == ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
            "unsupported checkpoint list schema {}",
            snapshot.schema_version
        );
        anyhow::ensure!(
            !snapshot.checkpoints.is_empty(),
            "no workspace checkpoints found"
        );
        Ok(Self {
            entries: snapshot.checkpoints,
            selected: 0,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CheckpointPickerAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => CheckpointPickerAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                CheckpointPickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                CheckpointPickerAction::Continue
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                CheckpointPickerAction::Continue
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.entries.len().saturating_sub(1);
                CheckpointPickerAction::Continue
            }
            KeyCode::Char('r') => CheckpointPickerAction::RewindCode {
                prompt_index: self.entries[self.selected].prompt_index,
            },
            KeyCode::Char('f') => CheckpointPickerAction::ForkSession {
                through_sequence: self.entries[self.selected].through_sequence,
            },
            KeyCode::Enter | KeyCode::Char('b') => CheckpointPickerAction::ForkAndRewind {
                prompt_index: self.entries[self.selected].prompt_index,
                through_sequence: self.entries[self.selected].through_sequence,
            },
            _ => CheckpointPickerAction::Continue,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width.saturating_sub(4).max(1).min(96);
        let height = (self.entries.len() as u16)
            .saturating_add(4)
            .min(area.height);
        let popup = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        Clear.render(popup, buf);
        let visible = popup.height.saturating_sub(3) as usize;
        let start = self
            .selected
            .saturating_add(1)
            .saturating_sub(visible)
            .min(self.entries.len().saturating_sub(visible));
        let end = (start + visible).min(self.entries.len());
        let lines = self.entries[start..end]
            .iter()
            .enumerate()
            .map(|(offset, entry)| {
                let index = start + offset;
                let selected = index == self.selected;
                let marker = if selected { "›" } else { " " };
                let state = if entry.finalized { "finalized" } else { "open" };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(
                    format!(
                        "{marker} prompt {} · {} · {state}",
                        entry.prompt_index, entry.turn_id
                    ),
                    style,
                ))
            })
            .collect::<Vec<_>>();
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Workspace checkpoints ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .render(popup, buf);
        if popup.height >= 2 {
            let footer = Rect::new(
                popup.x + 1,
                popup.y + popup.height - 2,
                popup.width.saturating_sub(2),
                1,
            );
            Line::from(Span::styled(
                "↑/↓ select  r rewind code  f fork only  Enter/b fork + rewind  Esc close",
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
    fn picker_returns_host_prompt_index_not_local_position() {
        let mut picker = CheckpointPicker::from_snapshot(::contracts::CheckpointListSnapshot {
            schema_version: ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
            session_id: "session".into(),
            checkpoints: vec![::contracts::CheckpointListEntry {
                checkpoint_id: "checkpoint".into(),
                turn_id: "turn".into(),
                prompt_index: 42,
                through_sequence: 17,
                created_at_ms: 1,
                finalized: true,
            }],
        })
        .unwrap();
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            CheckpointPickerAction::ForkAndRewind {
                prompt_index: 42,
                through_sequence: 17,
            }
        );
    }

    #[test]
    fn picker_exposes_each_recovery_mode_without_guessing_event_sequence() {
        let snapshot = ::contracts::CheckpointListSnapshot {
            schema_version: ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
            session_id: "session".into(),
            checkpoints: vec![::contracts::CheckpointListEntry {
                checkpoint_id: "checkpoint".into(),
                turn_id: "turn".into(),
                prompt_index: 42,
                through_sequence: 17,
                created_at_ms: 1,
                finalized: true,
            }],
        };
        let mut picker = CheckpointPicker::from_snapshot(snapshot).unwrap();
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('r'))),
            CheckpointPickerAction::RewindCode { prompt_index: 42 }
        );
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('f'))),
            CheckpointPickerAction::ForkSession {
                through_sequence: 17,
            }
        );
    }
}
