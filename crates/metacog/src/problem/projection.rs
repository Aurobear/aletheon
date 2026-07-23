//! Problem projection — rebuilds current ProblemRecord state by replaying events.
//!
//! Corrections append new facts; they do not rewrite history.

use std::collections::HashMap;

use super::ledger::LedgerEvent;
use super::model::{ProblemRecord, ProblemState};

/// An in-memory projection of the current problem state.
///
/// Rebuilt from scratch by replaying all ledger events in append order.
pub struct Projection {
    records: HashMap<String, ProblemRecord>,
}

impl Projection {
    /// Create an empty projection.
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Apply a ledger event to update the projection.
    pub fn apply_event(&mut self, event: &LedgerEvent) {
        match event {
            LedgerEvent::ProblemObserved {
                finding,
                fingerprint: _fp,
                ..
            } => {
                // Create a new record if not already present (idempotency)
                self.records
                    .entry(finding.problem_id.clone())
                    .or_insert_with(|| ProblemRecord {
                        problem_id: finding.problem_id.clone(),
                        category: finding.category.clone(),
                        subtype: finding.subtype.clone(),
                        domain: finding.domain.clone(),
                        subject: finding.subject.clone(),
                        severity: finding.severity,
                        confidence_millis: finding.confidence_millis,
                        state: ProblemState::Observed,
                        first_seen_at_ms: finding.observed_at_ms,
                        last_seen_at_ms: finding.observed_at_ms,
                        occurrence_count: 1,
                        affected_versions: finding.affected_versions.clone(),
                        expected_summary: finding.expected_summary.clone(),
                        observed_summary: finding.observed_summary.clone(),
                        failure_signature: finding.failure_signature.clone(),
                        evidence_ids: finding.evidence_ids.clone(),
                        causal_hypotheses: Vec::new(),
                        related_problem_ids: Vec::new(),
                        proposed_mitigations: Vec::new(),
                        resolution_evidence: Vec::new(),
                        regression_evidence: Vec::new(),
                    });
            }
            LedgerEvent::ProblemTransitioned { transition, .. } => {
                if let Some(record) = self.records.get_mut(&transition.problem_id) {
                    record.state = transition.new_state;
                    record.last_seen_at_ms = transition.timestamp_ms;
                    record.occurrence_count = record.occurrence_count.saturating_add(1);
                }
            }
        }
    }

    /// Get a record by ID.
    pub fn get(&self, id: &str) -> Option<&ProblemRecord> {
        self.records.get(id)
    }

    /// Check if a problem ID is present.
    pub fn contains(&self, id: &str) -> bool {
        self.records.contains_key(id)
    }

    /// List all active problems (non-resolved, non-disputed, non-accepted-risk).
    pub fn active(&self) -> Vec<ProblemRecord> {
        self.records
            .values()
            .filter(|r| {
                matches!(
                    r.state,
                    ProblemState::Observed
                        | ProblemState::Confirmed
                        | ProblemState::Active
                        | ProblemState::Mitigated
                        | ProblemState::Regressed
                )
            })
            .cloned()
            .collect()
    }

    /// Return the total number of records.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Return true if there are no records.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for Projection {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::ledger::ProblemFinding;
    use crate::problem::model::ProblemSeverity;

    fn make_finding(id: &str, domain: &str) -> ProblemFinding {
        ProblemFinding {
            problem_id: id.to_string(),
            category: "correctness".to_string(),
            subtype: "type_error".to_string(),
            domain: domain.to_string(),
            subject: "rustc".to_string(),
            severity: ProblemSeverity::Medium,
            confidence_millis: 800,
            observed_at_ms: 100,
            affected_versions: vec!["v1".to_string()],
            expected_summary: "expected clean compile".to_string(),
            observed_summary: "type error".to_string(),
            failure_signature: "E0308".to_string(),
            evidence_ids: vec!["ev-1".to_string()],
            rubric_version: 1,
        }
    }

    #[test]
    fn projection_rebuilds_from_events() {
        let mut proj = Projection::new();

        let finding = make_finding("p1", "coding");
        let event = LedgerEvent::ProblemObserved {
            event_id: "evt-1".into(),
            finding,
            fingerprint: "abc".into(),
            timestamp_ms: 100,
        };
        proj.apply_event(&event);

        let record = proj.get("p1").unwrap();
        assert_eq!(record.state, ProblemState::Observed);
        assert_eq!(record.occurrence_count, 1);

        // Apply a transition
        let transition = crate::problem::model::ProblemTransition {
            problem_id: "p1".into(),
            event_id: "evt-2".into(),
            old_state: ProblemState::Observed,
            new_state: ProblemState::Confirmed,
            reason: "verified".into(),
            evidence_ids: vec!["ev-2".into()],
            timestamp_ms: 200,
        };
        let event = LedgerEvent::ProblemTransitioned {
            event_id: "evt-2".into(),
            transition,
            timestamp_ms: 200,
        };
        proj.apply_event(&event);

        let record = proj.get("p1").unwrap();
        assert_eq!(record.state, ProblemState::Confirmed);
        assert_eq!(record.occurrence_count, 2);
    }

    #[test]
    fn projection_duplicate_observed_is_idempotent() {
        let mut proj = Projection::new();

        let finding = make_finding("p1", "coding");
        let event = LedgerEvent::ProblemObserved {
            event_id: "evt-1".into(),
            finding: finding.clone(),
            fingerprint: "abc".into(),
            timestamp_ms: 100,
        };
        proj.apply_event(&event);

        // Same ID observed again — should not duplicate
        let event2 = LedgerEvent::ProblemObserved {
            event_id: "evt-2".into(),
            finding,
            fingerprint: "abc".into(),
            timestamp_ms: 200,
        };
        proj.apply_event(&event2);

        assert_eq!(proj.len(), 1);
        let record = proj.get("p1").unwrap();
        assert_eq!(record.state, ProblemState::Observed);
    }

    #[test]
    fn active_filters_non_active_states() {
        let mut proj = Projection::new();

        // Create two problems
        let f1 = make_finding("p1", "coding");
        let f2 = make_finding("p2", "coding");
        proj.apply_event(&LedgerEvent::ProblemObserved {
            event_id: "evt-1".into(),
            finding: f1,
            fingerprint: "abc".into(),
            timestamp_ms: 100,
        });
        proj.apply_event(&LedgerEvent::ProblemObserved {
            event_id: "evt-2".into(),
            finding: f2,
            fingerprint: "def".into(),
            timestamp_ms: 200,
        });

        // Transition p1 to Resolved
        proj.apply_event(&LedgerEvent::ProblemTransitioned {
            event_id: "evt-3".into(),
            transition: crate::problem::model::ProblemTransition {
                problem_id: "p1".into(),
                event_id: "evt-3".into(),
                old_state: ProblemState::Observed,
                new_state: ProblemState::Disputed,
                reason: "not a real problem".into(),
                evidence_ids: vec![],
                timestamp_ms: 300,
            },
            timestamp_ms: 300,
        });

        let active = proj.active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].problem_id, "p2");
    }
}
