//! MutationLayer — mutation request tracking and approval.
//!
//! Tracks mutation requests (changes to the agent's own configuration).
//! Irreversible mutations to core identity fields are auto-denied.

use aletheon_abi::{MutationIntent, Verdict};
use aletheon_abi::self_field::RiskLevel;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

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
const CORE_IDENTITY_FIELDS: &[&str] = &["name", "identity.name", "core_values", "fundamental_purpose"];

/// MutationLayer — tracks and reviews mutation requests.
pub struct MutationLayer {
    records: RwLock<Vec<MutationRecord>>,
}

impl MutationLayer {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(Vec::new()),
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
                reviewed_at: Some(Utc::now()),
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
                reason: format!("Non-reversible mutation to '{}' requires confirmation", mutation.target),
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
            reviewed_at: Some(Utc::now()),
            denial_reason: None,
        };
        self.records.write().push(record);
        Verdict::Allow
    }

    /// Check if a target field is a core identity field.
    fn is_core_identity(&self, target: &str) -> bool {
        CORE_IDENTITY_FIELDS.iter().any(|f| target == *f || target.starts_with(&format!("{}.", f)))
    }

    /// Get all mutation records.
    pub fn records(&self) -> Vec<MutationRecord> {
        self.records.read().clone()
    }

    /// Approve a pending mutation by target name. Returns true if found and approved.
    pub fn approve(&self, target: &str) -> bool {
        let mut records = self.records.write();
        if let Some(record) = records.iter_mut().find(|r| r.target == target && r.status == MutationStatus::Pending) {
            record.status = MutationStatus::Approved;
            record.reviewed_at = Some(Utc::now());
            true
        } else {
            false
        }
    }

    /// Deny a pending mutation by target name.
    pub fn deny(&self, target: &str, reason: &str) -> bool {
        let mut records = self.records.write();
        if let Some(record) = records.iter_mut().find(|r| r.target == target && r.status == MutationStatus::Pending) {
            record.status = MutationStatus::Denied;
            record.reviewed_at = Some(Utc::now());
            record.denial_reason = Some(reason.to_string());
            true
        } else {
            false
        }
    }
}

impl Default for MutationLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let layer = MutationLayer::new();
        let m = make_mutation("care_priorities", true);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::Allow));
        assert_eq!(layer.records().len(), 1);
        assert_eq!(layer.records()[0].status, MutationStatus::Approved);
    }

    #[test]
    fn irreversible_core_identity_denied() {
        let layer = MutationLayer::new();
        let m = make_mutation("name", false);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::Deny { .. }));
        assert_eq!(layer.records()[0].status, MutationStatus::Denied);
    }

    #[test]
    fn irreversible_non_core_requires_confirmation() {
        let layer = MutationLayer::new();
        let m = make_mutation("boundary_rules", false);
        let verdict = layer.review(&m);
        assert!(matches!(verdict, Verdict::RequireConfirmation { .. }));
        assert_eq!(layer.records()[0].status, MutationStatus::Pending);
    }

    #[test]
    fn approve_pending() {
        let layer = MutationLayer::new();
        let m = make_mutation("boundary_rules", false);
        layer.review(&m);
        assert!(layer.approve("boundary_rules"));
        assert_eq!(layer.records()[0].status, MutationStatus::Approved);
    }

    #[test]
    fn deny_pending() {
        let layer = MutationLayer::new();
        let m = make_mutation("boundary_rules", false);
        layer.review(&m);
        assert!(layer.deny("boundary_rules", "too risky"));
        assert_eq!(layer.records()[0].status, MutationStatus::Denied);
    }
}
