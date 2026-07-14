use fabric::wall_to_datetime;
use fabric::Clock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// State shared across graph execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphState {
    /// Key-value data store.
    pub data: HashMap<String, serde_json::Value>,
    /// Execution log entries.
    pub log: Vec<LogEntry>,
}

/// A single log entry in the graph execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub node_id: String,
    pub status: String,
    pub timestamp: String,
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            log: Vec::new(),
        }
    }

    /// Set a value in the state.
    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.data.insert(key.to_string(), value);
    }

    /// Get a value from the state.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    /// Record a log entry.
    pub fn record(&mut self, node_id: &str, status: &str, clock: &dyn Clock) {
        self.log.push(LogEntry {
            node_id: node_id.to_string(),
            status: status.to_string(),
            timestamp: wall_to_datetime(clock.wall_now()).to_rfc3339(),
        });
    }
}

impl Default for GraphState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    #[test]
    fn test_state_set_get() {
        let mut state = GraphState::new();
        state.set("key", serde_json::json!("value"));
        assert_eq!(state.get("key"), Some(&serde_json::json!("value")));
        assert_eq!(state.get("missing"), None);
    }

    #[test]
    fn test_state_record() {
        let clock = TestClock::default();
        let mut state = GraphState::new();
        state.record("node1", "completed", &clock);
        assert_eq!(state.log.len(), 1);
        assert_eq!(state.log[0].node_id, "node1");
        assert_eq!(state.log[0].status, "completed");
    }
}
