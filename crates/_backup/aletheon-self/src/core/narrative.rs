//! NarrativeLayer — ring buffer decision log.
//!
//! The narrative is the agent's self-narrative: a record of every significant
//! decision and why it was made. Uses a fixed-capacity ring buffer so old
//! entries are evicted when the buffer is full.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::core::store::SelfFieldStore;

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
    buffer: RwLock<VecDeque<NarrativeEntry>>,
    capacity: usize,
}

impl NarrativeLayer {
    /// Create with a given capacity (default 1000).
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
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
            buffer.pop_front();
        }
        buffer.push_back(entry);
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
            buffer.pop_front();
        }
        buffer.push_back(entry);
    }

    /// Get the N most recent entries (newest last).
    pub fn recent(&self, n: usize) -> Vec<NarrativeEntry> {
        let buffer = self.buffer.read();
        let skip = if n >= buffer.len() { 0 } else { buffer.len() - n };
        buffer.iter().skip(skip).cloned().collect()
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

    /// Persist all current buffer entries to the store (replaces existing rows).
    pub fn save_to_store(&self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        conn.execute("DELETE FROM narrative_entries", [])
            .context("Failed to clear narrative_entries")?;

        let buffer = self.buffer.read();
        let mut stmt = conn
            .prepare(
                "INSERT INTO narrative_entries (event, reason, action, verdict, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("Failed to prepare narrative insert")?;
        for entry in buffer.iter() {
            stmt.execute(rusqlite::params![
                entry.event,
                entry.reason,
                entry.action,
                entry.verdict,
                entry.timestamp.to_rfc3339(),
            ])
            .context("Failed to insert narrative entry")?;
        }
        Ok(())
    }

    /// Load entries from the store into the buffer, replacing current contents.
    pub fn load_from_store(&mut self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT event, reason, action, verdict, timestamp
                 FROM narrative_entries ORDER BY id ASC",
            )
            .context("Failed to prepare narrative select")?;

        let rows = stmt
            .query_map([], |row| {
                let ts_str: String = row.get(4)?;
                Ok(NarrativeEntry {
                    event: row.get(0)?,
                    reason: row.get(1)?,
                    action: row.get(2)?,
                    verdict: row.get(3)?,
                    timestamp: DateTime::parse_from_rfc3339(&ts_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .context("Failed to query narrative_entries")?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.context("Failed to read narrative row")?);
        }

        // Trim to capacity if needed
        let start = if entries.len() > self.capacity {
            entries.len() - self.capacity
        } else {
            0
        };
        let entries: Vec<_> = entries[start..].to_vec();

        let mut buffer = self.buffer.write();
        *buffer = entries.into();
        Ok(())
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

    #[test]
    fn save_and_load_roundtrip() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = NarrativeLayer::new(100);
        layer.narrate("evt1", "reason1");
        layer.narrate("evt2", "reason2");

        layer.save_to_store(&store).unwrap();

        let mut loaded = NarrativeLayer::new(100);
        loaded.load_from_store(&store).unwrap();

        assert_eq!(loaded.len(), 2);
        let entries = loaded.recent(10);
        assert_eq!(entries[0].event, "evt1");
        assert_eq!(entries[1].reason, "reason2");
    }

    #[test]
    fn save_clears_previous() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = NarrativeLayer::new(100);
        layer.narrate("old", "old_reason");
        layer.save_to_store(&store).unwrap();

        let layer2 = NarrativeLayer::new(100);
        layer2.narrate("new", "new_reason");
        layer2.save_to_store(&store).unwrap();

        let mut loaded = NarrativeLayer::new(100);
        loaded.load_from_store(&store).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.recent(10)[0].event, "new");
    }
}
