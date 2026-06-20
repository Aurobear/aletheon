//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call.
//! Their status is tracked and emitted to the TUI via UiEvent.

use std::collections::HashMap;
use aletheon_abi::ui_event::{SubAgentHandle, SubAgentStatus};

/// Spawns and tracks sub-agents.
#[derive(Debug)]
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentHandle>,
    next_id: usize,
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a new sub-agent and return its handle.
    pub fn spawn(&mut self, task: String, parent_turn_id: String) -> SubAgentHandle {
        self.next_id += 1;
        let id = format!("agent-{}", self.next_id);
        let handle = SubAgentHandle {
            id: id.clone(),
            task,
            status: SubAgentStatus::Planning,
            parent_turn_id,
            spawned_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.agents.insert(id, handle.clone());
        handle
    }

    /// Update an agent's status.
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.status = status;
        }
    }

    /// Remove a completed/failed agent.
    pub fn remove(&mut self, id: &str) -> bool {
        self.agents.remove(id).is_some()
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().collect()
    }

    /// Get a specific agent.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id)
    }
}

impl Default for SubAgentSpawner {
    fn default() -> Self {
        Self::new()
    }
}
