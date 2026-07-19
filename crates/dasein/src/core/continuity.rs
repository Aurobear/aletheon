//! ContinuityLayer — lineage records for identity continuity.
//!
//! Identity continuity is causal: each version names its parent and carries a
//! checksum. Wall time is retained for observability but never determines
//! whether the identity chain is continuous.

use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// A lineage record — a snapshot of identity at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageRecord {
    pub identity_name: String,
    pub identity_version: String,
    // TODO: migrate to WallTime (extensive chrono Duration arithmetic, SQLite rfc3339, tests)
    pub recorded_at: DateTime<Utc>,
    pub event: String,
    pub parent_version: Option<String>,
    pub mutation_id: Option<String>,
    pub approval_id: Option<String>,
    pub checksum: String,
}

/// ContinuityLayer — tracks identity lineage and checks for continuity gaps.
pub struct ContinuityLayer {
    records: RwLock<Vec<LineageRecord>>,
    /// Accepted for configuration compatibility; wall gaps are not identity breaks.
    _max_gap: Duration,
    clock: Arc<dyn fabric::Clock>,
}

impl ContinuityLayer {
    pub fn new(max_gap: Duration, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            _max_gap: max_gap,
            clock,
        }
    }

    /// Record a lineage event.
    pub fn record(&self, identity_name: &str, identity_version: &str, event: &str) {
        let parent_version = self
            .records
            .read()
            .last()
            .map(|record| record.identity_version.clone());
        self.record_internal(
            identity_name,
            identity_version,
            parent_version,
            event,
            None,
            None,
            true,
        )
        .expect("inferred lineage parent must match the current chain tip");
    }

    pub fn record_transition(
        &self,
        identity_name: &str,
        identity_version: &str,
        parent_version: Option<String>,
        event: &str,
        mutation_id: Option<String>,
        approval_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.record_internal(
            identity_name,
            identity_version,
            parent_version,
            event,
            mutation_id,
            approval_id,
            false,
        )
    }

    fn record_internal(
        &self,
        identity_name: &str,
        identity_version: &str,
        parent_version: Option<String>,
        event: &str,
        mutation_id: Option<String>,
        approval_id: Option<String>,
        allow_same_version_checkpoint: bool,
    ) -> anyhow::Result<()> {
        let mut records = self.records.write();
        let expected_parent = records.last().map(|record| record.identity_version.clone());
        anyhow::ensure!(
            parent_version == expected_parent,
            "identity lineage parent does not match the current chain tip"
        );
        if !allow_same_version_checkpoint {
            anyhow::ensure!(
                !records
                    .iter()
                    .any(|record| record.identity_version == identity_version),
                "identity lineage version already exists"
            );
        }
        let checksum = lineage_checksum(
            identity_name,
            identity_version,
            parent_version.as_deref(),
            event,
            mutation_id.as_deref(),
            approval_id.as_deref(),
        );
        let entry = LineageRecord {
            identity_name: identity_name.to_string(),
            identity_version: identity_version.to_string(),
            recorded_at: fabric::wall_to_datetime(self.clock.wall_now()),
            event: event.to_string(),
            parent_version,
            mutation_id,
            approval_id,
            checksum,
        };
        records.push(entry);
        Ok(())
    }

    /// Check that lineage is one checksum-valid causal chain.
    pub fn is_continuous(&self) -> bool {
        let records = self.records.read();
        for (index, record) in records.iter().enumerate() {
            let expected_parent = index
                .checked_sub(1)
                .map(|parent| records[parent].identity_version.as_str());
            if record.parent_version.as_deref() != expected_parent {
                return false;
            }
            let expected_checksum = lineage_checksum(
                &record.identity_name,
                &record.identity_version,
                record.parent_version.as_deref(),
                &record.event,
                record.mutation_id.as_deref(),
                record.approval_id.as_deref(),
            );
            if record.checksum != expected_checksum {
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
                event TEXT NOT NULL,
                parent_version TEXT,
                mutation_id TEXT,
                approval_id TEXT,
                checksum TEXT NOT NULL
            );",
        )?;
        for migration in [
            "ALTER TABLE continuity_records ADD COLUMN parent_version TEXT",
            "ALTER TABLE continuity_records ADD COLUMN mutation_id TEXT",
            "ALTER TABLE continuity_records ADD COLUMN approval_id TEXT",
            "ALTER TABLE continuity_records ADD COLUMN checksum TEXT NOT NULL DEFAULT ''",
        ] {
            let _ = conn.execute(migration, []);
        }
        conn.execute("DELETE FROM continuity_records", [])?;
        conn.execute("DELETE FROM self_lineage", [])?;
        let records = self.records.read();
        for r in records.iter() {
            conn.execute(
                "INSERT INTO continuity_records
                 (identity_name, identity_version, recorded_at, event, parent_version,
                  mutation_id, approval_id, checksum)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    r.identity_name,
                    r.identity_version,
                    r.recorded_at.to_rfc3339(),
                    r.event,
                    r.parent_version,
                    r.mutation_id,
                    r.approval_id,
                    r.checksum,
                ],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO self_lineage
                 (version, parent_version, mutation_id, approval_id, checksum)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    r.identity_version,
                    r.parent_version,
                    r.mutation_id,
                    r.approval_id,
                    r.checksum,
                ],
            )?;
        }
        Ok(())
    }

    /// Load lineage records from the SQLite store.
    pub fn load_from_store(
        &mut self,
        store: &crate::core::store::SelfFieldStore,
    ) -> anyhow::Result<()> {
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
        let has_parent_column = conn
            .prepare("PRAGMA table_info(continuity_records)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == "parent_version");
        let query = if has_parent_column {
            "SELECT identity_name, identity_version, recorded_at, event,
                    parent_version, mutation_id, approval_id, checksum
             FROM continuity_records"
        } else {
            "SELECT identity_name, identity_version, recorded_at, event,
                    NULL, NULL, NULL, '' FROM continuity_records"
        };
        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        let mut loaded = Vec::new();
        for row in rows {
            let (
                identity_name,
                identity_version,
                recorded_at_str,
                event,
                stored_parent,
                mutation_id,
                approval_id,
                stored_checksum,
            ) = row?;
            let recorded_at = chrono::DateTime::parse_from_rfc3339(&recorded_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now()));
            let parent_version = if has_parent_column {
                stored_parent
            } else {
                loaded
                    .last()
                    .map(|record: &LineageRecord| record.identity_version.clone())
            };
            let checksum = if stored_checksum.is_empty() {
                lineage_checksum(
                    &identity_name,
                    &identity_version,
                    parent_version.as_deref(),
                    &event,
                    mutation_id.as_deref(),
                    approval_id.as_deref(),
                )
            } else {
                stored_checksum
            };
            loaded.push(LineageRecord {
                identity_name,
                identity_version,
                recorded_at,
                event,
                parent_version,
                mutation_id,
                approval_id,
                checksum,
            });
        }
        *self.records.write() = loaded;
        Ok(())
    }
}

fn lineage_checksum(
    identity_name: &str,
    identity_version: &str,
    parent_version: Option<&str>,
    event: &str,
    mutation_id: Option<&str>,
    approval_id: Option<&str>,
) -> String {
    let material = serde_json::json!({
        "identity_name": identity_name,
        "identity_version": identity_version,
        "parent_version": parent_version,
        "event": event,
        "mutation_id": mutation_id,
        "approval_id": approval_id,
    });
    let digest =
        Sha256::digest(serde_json::to_vec(&material).expect("lineage JSON is serializable"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_layer(max_gap: Duration) -> ContinuityLayer {
        ContinuityLayer::new(max_gap, test_clock())
    }

    #[test]
    fn single_record_is_continuous() {
        let layer = test_layer(Duration::hours(24));
        layer.record("aurb", "0.1.0", "initialized");
        assert!(layer.is_continuous());
    }

    #[test]
    fn empty_is_continuous() {
        let layer = test_layer(Duration::hours(24));
        assert!(layer.is_continuous());
    }

    #[test]
    fn no_gap_is_continuous() {
        let layer = test_layer(Duration::hours(1));
        layer.record("aurb", "0.1.0", "init");
        layer.record("aurb", "0.2.0", "upgrade");
        assert!(layer.is_continuous());
    }

    #[test]
    fn wall_gap_does_not_break_causal_continuity() {
        let layer = test_layer(Duration::hours(1));
        layer.record("aurb", "0.1.0", "init");
        layer.record("aurb", "0.2.0", "upgrade");
        let now = fabric::wall_to_datetime(layer.clock.wall_now());
        layer.records.write()[0].recorded_at = now - Duration::hours(48);
        assert!(layer.is_continuous());
    }

    #[test]
    fn wrong_parent_or_checksum_breaks_continuity() {
        let layer = test_layer(Duration::hours(24));
        layer.record("aurb", "0.1.0", "init");
        layer.record("aurb", "0.2.0", "upgrade");
        layer.records.write()[1].parent_version = Some("missing".into());
        assert!(!layer.is_continuous());
    }
}
