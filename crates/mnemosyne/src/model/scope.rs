use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Principal(String),
    Session(String),
    Goal(String),
    Agent(String),
    Task(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeAncestry {
    pub principal_id: Option<String>,
    pub session_id: Option<String>,
    pub goal_id: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
}

impl MemoryScope {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(id) = self.id() {
            anyhow::ensure!(!id.trim().is_empty(), "memory scope ID is required");
            anyhow::ensure!(id.len() <= 512, "memory scope ID exceeds byte limit");
        }
        Ok(())
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Principal(id)
            | Self::Session(id)
            | Self::Goal(id)
            | Self::Agent(id)
            | Self::Task(id) => Some(id),
        }
    }

    pub fn allows(&self, ancestry: &ScopeAncestry) -> bool {
        match self {
            Self::Global => true,
            Self::Principal(id) => ancestry.principal_id.as_deref() == Some(id),
            Self::Session(id) => ancestry.session_id.as_deref() == Some(id),
            Self::Goal(id) => ancestry.goal_id.as_deref() == Some(id),
            Self::Agent(id) => ancestry.agent_id.as_deref() == Some(id),
            Self::Task(id) => ancestry.task_id.as_deref() == Some(id),
        }
    }
}
