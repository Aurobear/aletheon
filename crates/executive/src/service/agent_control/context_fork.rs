use fabric::{AgentContextFork, AgentControlError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentContextItemKind {
    Selected,
    RecentTurns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContextItem {
    pub kind: AgentContextItemKind,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentContextProjection {
    pub items: Vec<AgentContextItem>,
    pub omitted_count: usize,
}

impl AgentContextProjection {
    pub fn from_fork(fork: &AgentContextFork) -> Result<Self, AgentControlError> {
        fork.validate()?;
        Ok(match fork {
            AgentContextFork::None => Self::default(),
            AgentContextFork::LastTurns { count } => Self {
                items: vec![AgentContextItem {
                    kind: AgentContextItemKind::RecentTurns,
                    label: "requested_recent_turn_count".into(),
                    content: count.to_string(),
                }],
                omitted_count: 0,
            },
            AgentContextFork::SelectedProjection { items } => Self {
                items: items
                    .iter()
                    .enumerate()
                    .map(|(index, content)| AgentContextItem {
                        kind: AgentContextItemKind::Selected,
                        label: format!("selected_context_{}", index + 1),
                        content: content.clone(),
                    })
                    .collect(),
                omitted_count: 0,
            },
        })
    }
}
