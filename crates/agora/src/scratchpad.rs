//! Scratchpad — task-level ephemeral workspace (migrated from mnemosyne, RFC-014).

use serde::{Deserialize, Serialize};

/// Retention policy for a scratchpad when the task completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionPolicy {
    /// Discard all entries immediately.
    Discard,
    /// Archive entries into the owning agent's private memory.
    ArchiveToAgent,
    /// Archive entries into the session-scoped memory (visible to parent).
    ArchiveToSession,
}

/// A single entry in a scratchpad.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    pub key: String,
    pub value: String,
}

/// Task-level scratch space for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scratchpad {
    pub agent_id: String,
    pub task_id: String,
    pub entries: Vec<ScratchpadEntry>,
    pub retention: RetentionPolicy,
}

impl Scratchpad {
    pub fn new(
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        retention: RetentionPolicy,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            entries: Vec::new(),
            retention,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
        } else {
            self.entries.push(ScratchpadEntry { key, value });
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.value.as_str())
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.key != key);
        self.entries.len() < len_before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn format_entries(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("[{}]: {}", e.key, e.value))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scratchpad_basic_operations() {
        let mut sp = Scratchpad::new("agent-1", "task-42", RetentionPolicy::Discard);
        sp.set("step", "1");
        sp.set("result", "ok");
        assert_eq!(sp.get("step"), Some("1"));
        assert_eq!(sp.get("result"), Some("ok"));
        assert_eq!(sp.len(), 2);
        sp.set("step", "2");
        assert_eq!(sp.get("step"), Some("2"));
        assert_eq!(sp.len(), 2);
        assert!(sp.remove("result"));
        assert!(!sp.remove("nonexistent"));
        assert_eq!(sp.len(), 1);
        sp.clear();
        assert!(sp.is_empty());
    }

    #[test]
    fn test_scratchpad_format_entries() {
        let mut sp = Scratchpad::new("agent-1", "task-1", RetentionPolicy::ArchiveToAgent);
        sp.set("a", "1");
        sp.set("b", "2");
        let formatted = sp.format_entries();
        assert!(formatted.contains("[a]: 1"));
        assert!(formatted.contains("[b]: 2"));
    }
}
