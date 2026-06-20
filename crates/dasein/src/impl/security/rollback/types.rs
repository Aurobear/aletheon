//! Types for the rollback engine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Rollback capability tiers (higher = better).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RollbackTier {
    /// Tier 1: Audit log + manual rollback guidance (always available)
    AuditOnly,
    /// Tier 2: File-level backup + service state recording
    FileBackup,
    /// Tier 3: btrfs atomic snapshot (best)
    AtomicSnapshot,
}

/// Unique identifier for a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SnapshotId {
    pub id: String,
    pub tier: RollbackTier,
    pub created_at: DateTime<Utc>,
}

impl SnapshotId {
    pub fn new(tier: RollbackTier) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tier,
            created_at: Utc::now(),
        }
    }
}

/// Context for snapshot creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackContext {
    /// Description of what operation triggered the snapshot
    pub operation: String,
    /// Paths to include in the snapshot
    pub paths: Vec<String>,
    /// Associated tool name
    pub tool: Option<String>,
    /// Risk level of the operation
    pub risk_level: Option<String>,
}

/// Result of a rollback operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackResult {
    pub success: bool,
    pub snapshot_id: SnapshotId,
    pub restored_paths: Vec<String>,
    pub message: String,
}

/// Configuration for rollback engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackConfig {
    /// Enable rollback engine
    pub enabled: bool,
    /// Preferred tier (auto selects best available)
    pub preference: RollbackPreference,
    /// Maximum snapshot age before cleanup
    pub max_snapshot_age_hours: u64,
    /// Maximum number of snapshots to keep
    pub max_snapshots: usize,
    /// Paths to always include in snapshots
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RollbackPreference {
    Auto,
    Require,
    BestEffort,
    Forbid,
}

impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            preference: RollbackPreference::Auto,
            max_snapshot_age_hours: 168, // 7 days
            max_snapshots: 50,
            protected_paths: vec!["/etc".to_string(), "/var/lib".to_string()],
        }
    }
}
