//! Trace — append-only reasoning trace: tool outputs and sub-agent results (RFC-014).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Audit fact kind. Runtime reasoning and tool payloads are not accepted.
    pub kind: String,
    pub content: Value,
    /// Agora audit is diagnostic and never an authority for replay.
    pub authoritative: bool,
    /// Audit content is sensitive unless a separate projection redacts it.
    pub sensitive: bool,
}

/// Append-only, best-effort audit trace for a session.
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

    pub fn push(&mut self, kind: impl Into<String>, content: Value) -> bool {
        let kind = kind.into();
        if !matches!(
            kind.as_str(),
            "proposal_rejected" | "evidence" | "candidate_admitted" | "selection" | "broadcast"
        ) {
            return false;
        }
        self.entries.push(TraceEntry {
            kind,
            content,
            authoritative: false,
            sensitive: true,
        });
        true
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
        assert!(!t.push("tool_output", json!({"tool": "bash", "ok": true})));
        assert!(t.push("selection", json!({"candidate_ids": ["c1"]})));
        assert_eq!(t.len(), 1);
        assert_eq!(t.entries()[0].kind, "selection");
        assert!(!t.entries()[0].authoritative);
        assert!(t.entries()[0].sensitive);
    }
}
