//! Rich completion popup for slash commands.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
    Frame,
};

use super::registry::{CommandRegistry, CommandSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub value: String,
    pub label: String,
    pub description: String,
    pub metadata: String,
    pub disabled_reason: Option<String>,
}

pub struct CompletionPopup {
    pub visible: bool,
    pub candidates: Vec<CompletionCandidate>,
    pub selected: usize,
    pub input_prefix: String,
}

impl Default for CompletionPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionPopup {
    pub fn new() -> Self {
        Self {
            visible: false,
            candidates: Vec::new(),
            selected: 0,
            input_prefix: String::new(),
        }
    }

    /// Compatibility entry point used by non-command completion callers.
    pub fn show(&mut self, prefix: &str, commands: &[String]) {
        let candidates = commands
            .iter()
            .filter(|command| command.starts_with(prefix))
            .map(|command| CompletionCandidate {
                value: command.clone(),
                label: command.clone(),
                description: String::new(),
                metadata: String::new(),
                disabled_reason: None,
            })
            .collect();
        self.show_candidates(prefix, candidates);
    }

    pub fn show_commands(&mut self, input: &str, registry: &CommandRegistry, turn_active: bool) {
        let query = input.trim_start_matches('/');
        if query.contains(char::is_whitespace) {
            self.hide();
            return;
        }
        let candidates = registry
            .find(query)
            .into_iter()
            .map(|command| {
                let source = match &command.source {
                    CommandSource::Builtin => "builtin".to_string(),
                    CommandSource::Skill { extension_id, .. } => extension_id
                        .as_ref()
                        .map_or_else(|| "skill".to_string(), |id| format!("skill · {id}")),
                };
                CompletionCandidate {
                    value: format!("/{}", command.name),
                    label: command.usage.clone(),
                    description: command.description.clone(),
                    metadata: source,
                    disabled_reason: (!command.available(turn_active)).then(|| {
                        match command.availability {
                            super::registry::CommandAvailability::IdleOnly => {
                                "当前任务运行中不可用".to_string()
                            }
                            super::registry::CommandAvailability::ActiveTurnOnly => {
                                "仅在任务运行中可用".to_string()
                            }
                            super::registry::CommandAvailability::Always => String::new(),
                        }
                    }),
                }
            })
            .collect();
        self.show_candidates(input, candidates);
    }

    fn show_candidates(&mut self, prefix: &str, candidates: Vec<CompletionCandidate>) {
        self.candidates = candidates;
        if self.candidates.is_empty() {
            self.hide();
            return;
        }
        self.visible = true;
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
        self.input_prefix = prefix.to_string();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.candidates.clear();
        self.selected = 0;
    }

    pub fn next(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = (self.selected + 1) % self.candidates.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = if self.selected == 0 {
                self.candidates.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.candidates
            .get(self.selected)
            .filter(|candidate| candidate.disabled_reason.is_none())
            .map(|candidate| candidate.value.as_str())
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }

        let visible = self.candidates.iter().take(8);
        let items: Vec<ListItem> = visible
            .enumerate()
            .map(|(index, candidate)| {
                let selected = index == self.selected;
                let disabled = candidate.disabled_reason.is_some();
                let base = if disabled {
                    Style::default().fg(Color::DarkGray)
                } else if selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                let description_style = if selected && !disabled {
                    base
                } else {
                    Style::default().fg(Color::Gray)
                };
                let suffix = candidate
                    .disabled_reason
                    .as_deref()
                    .unwrap_or(&candidate.metadata);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", candidate.label),
                        base.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {}  ", candidate.description), description_style),
                    Span::styled(
                        suffix.to_string(),
                        description_style.add_modifier(Modifier::ITALIC),
                    ),
                ]))
            })
            .collect();

        let height = (items.len() as u16 + 2).min(10);
        let popup = Rect {
            x: area.x + 1,
            y: area.y.saturating_sub(height),
            width: area.width.saturating_sub(2).max(1),
            height,
        };
        frame.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(List::new(items).block(block), popup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_opens_automatically_with_descriptions() {
        let registry = CommandRegistry::new();
        let mut popup = CompletionPopup::new();
        popup.show_commands("/", &registry, false);
        assert!(popup.visible);
        assert!(popup.candidates.iter().any(|item| item.value == "/compact"));
        assert!(popup
            .candidates
            .iter()
            .all(|item| !item.description.is_empty()));
    }

    #[test]
    fn active_turn_disables_idle_only_commands() {
        let registry = CommandRegistry::new();
        let mut popup = CompletionPopup::new();
        popup.show_commands("/clear", &registry, true);
        assert!(popup.candidates[0].disabled_reason.is_some());
        assert!(popup.selected().is_none());
    }
}
