//! Keyboard-navigable projection of Aletheon-owned child Agent sessions.

use ::contracts::protocol::client::AgentSessionSnapshot;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentInspectorAction {
    Continue,
    Close,
    Refresh,
}

#[derive(Debug, Clone)]
pub struct AgentInspector {
    agents: Vec<AgentSessionSnapshot>,
    selected: usize,
    detail: bool,
    scroll: u16,
}

impl AgentInspector {
    pub fn from_json(value: &serde_json::Value, focus: Option<&str>) -> anyhow::Result<Self> {
        let agents = serde_json::from_value::<Vec<AgentSessionSnapshot>>(value.clone())?;
        anyhow::ensure!(!agents.is_empty(), "no child Agent sessions found");
        let selected = focus
            .and_then(|id| agents.iter().position(|agent| agent.id == id))
            .unwrap_or(0);
        Ok(Self {
            agents,
            selected,
            detail: focus.is_some(),
            scroll: 0,
        })
    }

    pub fn focus_id(&self) -> Option<String> {
        self.agents.get(self.selected).map(|agent| agent.id.clone())
    }

    pub fn replace(&mut self, value: &serde_json::Value) -> anyhow::Result<()> {
        let focus = self.focus_id();
        let mut next = Self::from_json(value, focus.as_deref())?;
        next.detail = self.detail;
        next.scroll = self.scroll;
        *self = next;
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AgentInspectorAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') if self.detail => {
                self.detail = false;
                self.scroll = 0;
                AgentInspectorAction::Continue
            }
            KeyCode::Esc | KeyCode::Char('q') => AgentInspectorAction::Close,
            KeyCode::Char('r') => AgentInspectorAction::Refresh,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                self.detail = true;
                self.scroll = 0;
                AgentInspectorAction::Continue
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.detail = false;
                self.scroll = 0;
                AgentInspectorAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') if self.detail => {
                self.scroll = self.scroll.saturating_sub(1);
                AgentInspectorAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') if self.detail => {
                self.scroll = self.scroll.saturating_add(1);
                AgentInspectorAction::Continue
            }
            KeyCode::PageUp if self.detail => {
                self.scroll = self.scroll.saturating_sub(8);
                AgentInspectorAction::Continue
            }
            KeyCode::PageDown if self.detail => {
                self.scroll = self.scroll.saturating_add(8);
                AgentInspectorAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                AgentInspectorAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.agents.len().saturating_sub(1));
                AgentInspectorAction::Continue
            }
            _ => AgentInspectorAction::Continue,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        Clear.render(area, buf);
        let block = Block::default()
            .title(" Agent sessions ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);
        if self.detail {
            self.render_detail(inner, buf);
        } else {
            self.render_list(inner, buf);
        }
    }

    fn render_list(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let rows = self
            .agents
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let selected = index == self.selected;
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if agent.status == "running" || agent.status == "waiting" {
                    Style::default().fg(Color::Yellow)
                } else if agent.status == "failed" {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(
                    format!(
                        "{} {:12} {:10} {} · {}",
                        if selected { "›" } else { " " },
                        short(&agent.id, 12),
                        agent.status,
                        agent.runtime_id,
                        one_line(&agent.task, 72)
                    ),
                    style,
                ))
            })
            .collect::<Vec<_>>();
        let footer = area.height.saturating_sub(1);
        Paragraph::new(rows).wrap(Wrap { trim: true }).render(
            Rect {
                height: footer,
                ..area
            },
            buf,
        );
        if area.height > 0 {
            Line::from(Span::styled(
                "↑/↓ select · Enter inspect · live auto-refresh · r refresh · Esc close",
                Style::default().fg(Color::DarkGray),
            ))
            .render(Rect::new(area.x, area.y + footer, area.width, 1), buf);
        }
    }

    fn render_detail(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let Some(agent) = self.agents.get(self.selected) else {
            return;
        };
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        let snapshot = &agent.snapshot;
        let elapsed = elapsed_ms(
            snapshot.started_at_ms,
            snapshot.ended_at_ms,
            i64::try_from(crate::tui::test_infra::now_ms()).unwrap_or(i64::MAX),
        );
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Agent ", Style::default().fg(Color::Cyan)),
                Span::styled(&agent.id, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("  [{}]", agent.status)),
            ]),
            Line::from(format!(
                "runtime {} · profile {} · process {} · operation {}",
                agent.runtime_id,
                agent.profile_id,
                snapshot.handle.process_id.0,
                snapshot.handle.operation_id.0
            )),
            Line::from(format!(
                "parent {} · elapsed {}",
                snapshot
                    .handle
                    .parent_agent_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_else(|| "root".into()),
                elapsed.map_or_else(|| "pending".into(), |ms| format!("{ms}ms"))
            )),
            Line::from(format!("task {}", one_line(&agent.task, 160))),
            Line::from(
                snapshot
                    .last_error
                    .as_deref()
                    .map(|error| format!("error {}", one_line(error, 160)))
                    .unwrap_or_default(),
            ),
        ])
        .wrap(Wrap { trim: true })
        .render(sections[0], buf);

        let mut lines = vec![Line::from(Span::styled(
            "Authoritative timeline",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))];
        for entry in &agent.timeline {
            lines.push(Line::from(format!(
                "{:>4} {:10} {}",
                entry.sequence,
                entry.kind,
                one_line(&public_detail(&entry.detail), 180)
            )));
        }
        if let Some(result) = snapshot.result.as_ref() {
            lines.push(Line::from(Span::styled(
                "Terminal result",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(format!(
                "usage {} in / {} out · {} evidence · {} artifacts",
                result.usage.input_tokens,
                result.usage.output_tokens,
                result.evidence.len(),
                result.artifacts.len()
            )));
            lines.extend(
                result
                    .output
                    .lines()
                    .map(|line| Line::from(line.to_owned())),
            );
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0))
            .block(Block::default().borders(Borders::TOP))
            .render(sections[1], buf);
        Line::from(Span::styled(
            "↑/↓ scroll · h/Esc back · live auto-refresh · r refresh · q close",
            Style::default().fg(Color::DarkGray),
        ))
        .render(sections[2], buf);
    }
}

fn elapsed_ms(started_at_ms: Option<i64>, ended_at_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    started_at_ms.map(|started| ended_at_ms.unwrap_or(now_ms).saturating_sub(started).max(0))
}

fn public_detail(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

fn short(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        value.chars().take(max).collect()
    }
}

fn one_line(value: &str, max: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max {
        compact
    } else {
        format!("{}…", compact.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn fixture() -> serde_json::Value {
        serde_json::json!([{
            "id": "agent-1",
            "task": "inspect repository",
            "status": "running",
            "runtime_id": "pi-rpc",
            "profile_id": "code-agent",
            "snapshot": {
                "handle": {
                    "agent_id": "00000000-0000-0000-0000-000000000001",
                    "root_agent_id": "00000000-0000-0000-0000-000000000002",
                    "parent_agent_id": null,
                    "process_id": "00000000-0000-0000-0000-000000000003",
                    "operation_id": "00000000-0000-0000-0000-000000000004",
                    "runtime_id": "pi-rpc",
                    "profile_id": "code-agent"
                },
                "status": "running",
                "result": null,
                "created_at_ms": 1,
                "started_at_ms": 2,
                "ended_at_ms": null,
                "last_error": null
            },
            "timeline": [{"sequence":1,"kind":"started","detail":null}]
        }])
    }

    #[test]
    fn enter_opens_session_and_escape_returns_to_list() {
        let mut inspector = AgentInspector::from_json(&fixture(), None).unwrap();
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        inspector.handle_key(key(KeyCode::Enter));
        assert!(inspector.detail);
        assert_eq!(
            inspector.handle_key(key(KeyCode::Esc)),
            AgentInspectorAction::Continue
        );
        assert!(!inspector.detail);
    }

    #[test]
    fn running_elapsed_uses_refresh_time_and_terminal_elapsed_uses_end_time() {
        assert_eq!(elapsed_ms(Some(100), None, 350), Some(250));
        assert_eq!(elapsed_ms(Some(100), Some(220), 350), Some(120));
        assert_eq!(elapsed_ms(Some(400), None, 350), Some(0));
        assert_eq!(elapsed_ms(None, None, 350), None);
    }
}
