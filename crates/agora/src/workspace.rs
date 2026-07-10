//! Workspace — a single session's cognitive workspace, aggregating all
//! working-memory components (RFC-014).

use serde_json::{json, Value};

use crate::attention::Attention;
use crate::blackboard::Blackboard;
use crate::task_graph::TaskGraph;
use crate::trace::Trace;

/// One session's in-memory cognitive workspace.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub session_id: String,
    pub blackboard: Blackboard,
    pub attention: Attention,
    pub task_graph: TaskGraph,
    pub trace: Trace,
}

impl Workspace {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            blackboard: Blackboard::new(),
            attention: Attention::new(),
            task_graph: TaskGraph::new(),
            trace: Trace::new(),
        }
    }

    /// Snapshot the workspace to JSON (for debug / commit to Mnemosyne).
    pub fn snapshot(&self) -> Value {
        json!({
            "session_id": self.session_id,
            "blackboard": self.blackboard.to_json(),
            "attention": {
                "focus": self.attention.focus,
                "priorities": self.attention.priorities,
            },
            "task_count": self.task_graph.len(),
            "trace_len": self.trace.len(),
        })
    }

    /// Clear all workspace state (keeps the session id).
    pub fn clear(&mut self) {
        self.blackboard.clear();
        self.attention = Attention::new();
        self.task_graph = TaskGraph::new();
        self.trace.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_includes_session_and_blackboard() {
        let mut ws = Workspace::new("s1");
        ws.blackboard.set("goal", json!("ship it"));
        let snap = ws.snapshot();
        assert_eq!(snap["session_id"], json!("s1"));
        assert_eq!(snap["blackboard"]["goal"], json!("ship it"));
    }

    #[test]
    fn clear_resets_state() {
        let mut ws = Workspace::new("s1");
        ws.blackboard.set("k", json!(1));
        ws.clear();
        assert!(ws.blackboard.is_empty());
        assert_eq!(ws.session_id, "s1");
    }
}
