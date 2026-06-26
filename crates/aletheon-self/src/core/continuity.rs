//! ContinuityLayer — lineage records for identity continuity.
//!
//! Tracks identity changes over time. `is_continuous()` returns true
//! if there are no gaps longer than a configured threshold (default 24h)
//! between consecutive identity records.

use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// A lineage record — a snapshot of identity at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageRecord {
    pub identity_name: String,
    pub identity_version: String,
    pub recorded_at: DateTime<Utc>,
    pub event: String,
}

/// ContinuityLayer — tracks identity lineage and checks for continuity gaps.
pub struct ContinuityLayer {
    records: RwLock<Vec<LineageRecord>>,
    /// Maximum allowed gap between records (default 24 hours).
    max_gap: Duration,
}

impl ContinuityLayer {
    pub fn new(max_gap: Duration) -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            max_gap,
        }
    }

    /// Record a lineage event.
    pub fn record(
        &self,
        identity_name: &str,
        identity_version: &str,
        event: &str,
    ) {
        let entry = LineageRecord {
            identity_name: identity_name.to_string(),
            identity_version: identity_version.to_string(),
            recorded_at: Utc::now(),
            event: event.to_string(),
        };
        self.records.write().push(entry);
    }

    /// Check if the lineage is continuous (no gap > max_gap between records).
    pub fn is_continuous(&self) -> bool {
        let records = self.records.read();
        if records.len() < 2 {
            return true;
        }
        for window in records.windows(2) {
            let gap = window[1].recorded_at - window[0].recorded_at;
            if gap > self.max_gap {
                return false;
            }
        }
        true
    }

    /// Get all lineage records.
    pub fn all_records(&self) -> Vec<LineageRecord> {
        self.records.read().clone()
    }

    /// Number of lineage records.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Whether there are no records.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }

    /// Save all lineage records to the SQLite store.
    pub fn save_to_store(&self, store: &crate::core::store::SelfFieldStore) -> anyhow::Result<()> {
        let conn = store.conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS continuity_records (
                identity_name TEXT NOT NULL,
                identity_version TEXT NOT NULL,
                recorded_at TEXT NOT NULL,
                event TEXT NOT NULL
            );"
        )?;
        conn.execute("DELETE FROM continuity_records", [])?;
        let records = self.records.read();
        for r in records.iter() {
            conn.execute(
                "INSERT INTO continuity_records (identity_name, identity_version, recorded_at, event)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    r.identity_name,
                    r.identity_version,
                    r.recorded_at.to_rfc3339(),
                    r.event,
                ],
            )?;
        }
        Ok(())
    }

    /// Load lineage records from the SQLite store.
    pub fn load_from_store(&mut self, store: &crate::core::store::SelfFieldStore) -> anyhow::Result<()> {
        let conn = store.conn();
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='continuity_records'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !table_exists {
            return Ok(());
        }
        let mut stmt = conn.prepare(
            "SELECT identity_name, identity_version, recorded_at, event FROM continuity_records",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut loaded = Vec::new();
        for row in rows {
            let (identity_name, identity_version, recorded_at_str, event) = row?;
            let recorded_at = chrono::DateTime::parse_from_rfc3339(&recorded_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            loaded.push(LineageRecord {
                identity_name,
                identity_version,
                recorded_at,
                event,
            });
        }
        *self.records.write() = loaded;
        Ok(())
    }
}

impl Default for ContinuityLayer {
    fn default() -> Self {
        Self::new(Duration::hours(24))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn single_record_is_continuous() {
        let layer = ContinuityLayer::default();
        layer.record("aurb", "0.1.0", "initialized");
        assert!(layer.is_continuous());
    }

    #[test]
    fn empty_is_continuous() {
        let layer = ContinuityLayer::default();
        assert!(layer.is_continuous());
    }

    #[test]
    fn no_gap_is_continuous() {
        let layer = ContinuityLayer::new(Duration::hours(1));
        layer.record("aurb", "0.1.0", "init");
        layer.record("aurb", "0.2.0", "upgrade");
        assert!(layer.is_continuous());
    }

    #[test]
    fn gap_exceeds_threshold() {
        // We can't easily simulate time passing in a unit test,
        // but we can test the structure by manually constructing records.
        let layer = ContinuityLayer {
            records: RwLock::new(vec![
                LineageRecord {
                    identity_name: "aurb".to_string(),
                    identity_version: "0.1.0".to_string(),
                    recorded_at: Utc::now() - Duration::hours(48),
                    event: "init".to_string(),
                },
                LineageRecord {
                    identity_name: "aurb".to_string(),
                    identity_version: "0.2.0".to_string(),
                    recorded_at: Utc::now(),
                    event: "upgrade".to_string(),
                },
            ]),
            max_gap: Duration::hours(24),
        };
        assert!(!layer.is_continuous());
    }
}
