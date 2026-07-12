//! MutationLayer — mutation request tracking and approval.
//!
//! Tracks mutation requests (changes to the agent's own configuration).
//! Irreversible mutations to core identity fields are auto-denied.

use chrono::{DateTime, Utc};
use fabric::self_field::RiskLevel;
use fabric::{MutationIntent, Verdict};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Status of a mutation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationStatus {
    Pending,
    Approved,
    Denied,
}

/// A tracked mutation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationRecord {
    pub target: String,
    pub change: serde_json::Value,
    pub reason: String,
    pub reversible: bool,
    pub status: MutationStatus,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub denial_reason: Option<String>,
}

/// Fields that are considered core identity — irreversible changes to these are auto-denied.
const CORE_IDENTITY_FIELDS: &[&str] = &[
    "name",
    "identity.name",
    "core_values",
    "fundamental_purpose",
];

/// MutationLayer — tracks and reviews mutation requests.
pub struct MutationLayer {
    records: RwLock<Vec<MutationRecord>>,
    clock: Arc<dyn fabric::Clock>,
}

impl MutationLayer {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            clock,
        }
    }

    /// Review a mutation request. Returns a Verdict.
    pub fn review(&self, mutation: &MutationIntent) -> Verdict {
        // Auto-deny irreversible changes to core identity fields
        if !mutation.reversible && self.is_core_identity(&mutation.target) {
            let record = MutationRecord {
                target: mutation.target.clone(),
                change: mutation.change.clone(),
                reason: mutation.reason.clone(),
                reversible: mutation.reversible,
                status: MutationStatus::Denied,
                reviewed_at: Some(fabric::wall_to_datetime(self.clock.wall_now())),
                denial_reason: Some("Irreversible change to core identity field".to_string()),
            };
            self.records.write().push(record);
            return Verdict::Deny {
                reason: format!(
                    "Cannot irreversibly mutate core identity field '{}'",
                    mutation.target
                ),
            };
        }

        // Non-reversible non-core: require confirmation
        if !mutation.reversible {
            let record = MutationRecord {
                target: mutation.target.clone(),
                change: mutation.change.clone(),
                reason: mutation.reason.clone(),
                reversible: mutation.reversible,
                status: MutationStatus::Pending,
                reviewed_at: None,
                denial_reason: None,
            };
            self.records.write().push(record);
            return Verdict::RequireConfirmation {
                reason: format!(
                    "Non-reversible mutation to '{}' requires confirmation",
                    mutation.target
                ),
                risk_level: RiskLevel::High,
            };
        }

        // Reversible mutations are allowed
        let record = MutationRecord {
            target: mutation.target.clone(),
            change: mutation.change.clone(),
            reason: mutation.reason.clone(),
            reversible: mutation.reversible,
            status: MutationStatus::Approved,
            reviewed_at: Some(fabric::wall_to_datetime(self.clock.wall_now())),
            denial_reason: None,
        };
        self.records.write().push(record);
        Verdict::Allow
    }

    /// Check if a target field is a core identity field.
    fn is_core_identity(&self, target: &str) -> bool {
        CORE_IDENTITY_FIELDS
            .iter()
            .any(|f| target == *f || target.starts_with(&format!("{}.", f)))
    }

    /// Get all mutation records.
    pub fn records(&self) -> Vec<MutationRecord> {
        self.records.read().clone()
    }

    /// Approve a pending mutation by target name. Returns true if found and approved.
    pub fn approve(&self, target: &str) -> bool {
        let mut records = self.records.write();
        if let Some(record) = records
            .iter_mut()
            .find(|r| r.target == target && r.status == MutationStatus::Pending)
        {
            record.status = MutationStatus::Approved;
            record.reviewed_at = Some(fabric::wall_to_datetime(self.clock.wall_now()));
            true
        } else {
            false
        }
    }

    /// Deny a pending mutation by target name.
    pub fn deny(&self, target: &str, reason: &str) -> bool {
        let mut records = self.records.write();
        if let Some(record) = records
            .iter_mut()
            .find(|r| r.target == target && r.status == MutationStatus::Pending)
        {
            record.status = MutationStatus::Denied;
            record.reviewed_at = Some(fabric::wall_to_datetime(self.clock.wall_now()));
            record.denial_reason = Some(reason.to_string());
            true
        } else {
            false
        }
    }

    /// Save all mutation records to the SQLite store.
    pub fn save_to_store(&self, store: &crate::core::store::SelfFieldStore) -> anyhow::Result<()> {
        let conn = store.conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS mutation_records (
                target TEXT NOT NULL,
                change TEXT NOT NULL,
                reason TEXT NOT NULL,
                reversible INTEGER NOT NULL,
                status TEXT NOT NULL,
                reviewed_at TEXT,
                denial_reason TEXT
            );",
        )?;
        conn.execute("DELETE FROM mutation_records", [])?;
        let records = self.records.read();
        for r in records.iter() {
            conn.execute(
                "INSERT INTO mutation_records (target, change, reason, reversible, status, reviewed_at, denial_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    r.target,
                    serde_json::to_string(&r.change).unwrap_or_default(),
                    r.reason,
                    r.reversible as i32,
                    serde_json::to_string(&r.status).unwrap_or_default(),
                    r.reviewed_at.map(|dt| dt.to_rfc3339()),
                    r.denial_reason,
                ],
            )?;
        }
        Ok(())
    }

    /// Load mutation records from the SQLite store.
    pub fn load_from_store(
        &mut self,
        store: &crate::core::store::SelfFieldStore,
    ) -> anyhow::Result<()> {
        let conn = store.conn();
        // Table may not exist yet on first load
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='mutation_records'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !table_exists {
            return Ok(());
        }
        let mut stmt = conn.prepare(
            "SELECT target, change, reason, reversible, status, reviewed_at, denial_reason FROM mutation_records",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut loaded = Vec::new();
        for row in rows {
            let (target, change_str, reason, reversible, status_str, reviewed_at, denial_reason) =
                row?;
            let change: serde_json::Value =
                serde_json::from_str(&change_str).unwrap_or(serde_json::Value::Null);
            let status: MutationStatus =
                serde_json::from_str(&status_str).unwrap_or(MutationStatus::Pending);
            let reviewed_at_parsed = reviewed_at
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            loaded.push(MutationRecord {
                target,
                change,
                reason,
                reversible: reversible != 0,
                status,
                reviewed_at: reviewed_at_parsed,
                denial_reason,
            });
        }
        *self.records.write() = loaded;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use serde_json::json;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_layer() -> MutationLayer {
        MutationLayer::new(test_clock())
    }

    fn make_mutation(target: &str, reversible: bool) -> MutationIntent {
        MutationIntent {
            target: target.to_string(),
            change: json!({"new": "value"}),
            reason: "test".to_string(),
            reversible,
        }
    }

    #[test]
    fn reversible_mutation_allowed() {
        let layer = test_layer();
        let m = make_mutation("care_priorities", true);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::Allow));
        assert_eq!(layer.records().len(), 1);
        assert_eq!(layer.records()[0].status, MutationStatus::Approved);
    }

    #[test]
    fn irreversible_core_identity_denied() {
        let layer = test_layer();
        let m = make_mutation("name", false);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::Deny { .. }));
        assert_eq!(layer.records()[0].status, MutationStatus::Denied);
    }

    #[test]
    fn irreversible_non_core_requires_confirmation() {
        let layer = test_layer();
        let m = make_mutation("boundary_rules", false);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::RequireConfirmation { .. }));
        assert_eq!(layer.records()[0].status, MutationStatus::Pending);
    }

    #[test]
    fn approve_pending() {
        let layer = test_layer();
        let m = make_mutation("boundary_rules", false);
        layer.review(&m);
        assert!(layer.approve("boundary_rules"));
        assert_eq!(layer.records()[0].status, MutationStatus::Approved);
    }

    #[test]
    fn deny_pending() {
        let layer = test_layer();
        let m = make_mutation("boundary_rules", false);
        layer.review(&m);
        assert!(layer.deny("boundary_rules", "too risky"));
        assert_eq!(layer.records()[0].status, MutationStatus::Denied);
    }
}
