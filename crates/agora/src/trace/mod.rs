//! Trace — append-only reasoning trace: tool outputs and sub-agent results (RFC-014).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Kind of trace event, e.g. "reasoning", "tool_output", "sub_agent".
    pub kind: String,
    pub content: Value,
}

/// Append-only reasoning trace for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trace {
    entries: Vec<TraceEntry>,
}

impl Trace {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, kind: impl Into<String>, content: Value) {
        self.entries.push(TraceEntry {
            kind: kind.into(),
            content,
        });
    }

    pub fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_and_read() {
        let mut t = Trace::new();
        t.push("tool_output", json!({"tool": "bash", "ok": true}));
        assert_eq!(t.len(), 1);
        assert_eq!(t.entries()[0].kind, "tool_output");
    }
}
