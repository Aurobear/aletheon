//! NarrativeLayer — ring buffer decision log.
//!
//! The narrative is the agent's self-narrative: a record of every significant
//! decision and why it was made. Uses a fixed-capacity ring buffer so old
//! entries are evicted when the buffer is full.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// A single narrative entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeEntry {
    pub event: String,
    pub reason: String,
    pub action: Option<String>,
    pub verdict: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// NarrativeLayer — in-memory ring buffer of decision records.
pub struct NarrativeLayer {
    buffer: RwLock<Vec<NarrativeEntry>>,
    capacity: usize,
}

impl NarrativeLayer {
    /// Create with a given capacity (default 1000).
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(Vec::with_capacity(capacity)),
            capacity,
        }
    }

    /// Record a decision event.
    pub fn record(
        &self,
        event: &str,
        reason: &str,
        action: Option<&str>,
        verdict: &impl std::fmt::Debug,
    ) {
        let entry = NarrativeEntry {
            event: event.to_string(),
            reason: reason.to_string(),
            action: action.map(|s| s.to_string()),
            verdict: Some(format!("{:?}", verdict)),
            timestamp: Utc::now(),
        };
        let mut buffer = self.buffer.write();
        if buffer.len() >= self.capacity {
            buffer.remove(0);
        }
        buffer.push(entry);
    }

    /// Record a narrative event (for SelfFieldOps::narrate).
    pub fn narrate(&self, event: &str, reason: &str) {
        let entry = NarrativeEntry {
            event: event.to_string(),
            reason: reason.to_string(),
            action: None,
            verdict: None,
            timestamp: Utc::now(),
        };
        let mut buffer = self.buffer.write();
        if buffer.len() >= self.capacity {
            buffer.remove(0);
        }
        buffer.push(entry);
    }

    /// Get the N most recent entries (newest last).
    pub fn recent(&self, n: usize) -> Vec<NarrativeEntry> {
        let buffer = self.buffer.read();
        let skip = if n >= buffer.len() { 0 } else { buffer.len() - n };
        buffer[skip..].to_vec()
    }

    /// Total entries currently stored.
    pub fn len(&self) -> usize {
        self.buffer.read().len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.read().is_empty()
    }

    /// Buffer capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for NarrativeLayer {
    fn default() -> Self {
        Self::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::Verdict;

    #[test]
    fn record_and_retrieve() {
        let layer = NarrativeLayer::new(100);
        layer.record("boundary_check", "deny: rm", Some("rm -rf /"), &Verdict::Deny { reason: "bad".to_string() });

        let entries = layer.recent(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, "boundary_check");
        assert_eq!(entries[0].reason, "deny: rm");
        assert_eq!(entries[0].action, Some("rm -rf /".to_string()));
    }

    #[test]
    fn capacity_eviction() {
        let layer = NarrativeLayer::new(3);
        layer.narrate("e1", "r1");
        layer.narrate("e2", "r2");
        layer.narrate("e3", "r3");
        assert_eq!(layer.len(), 3);

        layer.narrate("e4", "r4");
        assert_eq!(layer.len(), 3);

        let entries = layer.recent(10);
        assert_eq!(entries[0].event, "e2");
        assert_eq!(entries[1].event, "e3");
        assert_eq!(entries[2].event, "e4");
    }

    #[test]
    fn recent_limits() {
        let layer = NarrativeLayer::new(100);
        for i in 0..10 {
            layer.narrate(&format!("e{}", i), "reason");
        }

        let entries = layer.recent(3);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].event, "e7");
        assert_eq!(entries[2].event, "e9");
    }
}
