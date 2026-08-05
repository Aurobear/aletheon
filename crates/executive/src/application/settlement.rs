//! Host-owned review and settlement decision boundary (X8c).
//!
//! This module consumes typed transaction/validation evidence and never
//! trusts a model's textual claim of completion. It computes the only
//! accepted terminal decision; persistence and transport remain outside this
//! pure authority evaluator.

use fabric::change_transaction::{
    ChangeTransactionPhase, ChangeTransactionSnapshot, ValidationPlanOmission,
};
use fabric::{ReviewFinding, ReviewFindingStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSettlementDecision {
    Accepted,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettlementReceipt {
    pub transaction_id: String,
    pub session_id: String,
    pub workspace_version: String,
    pub decision: HostSettlementDecision,
    pub finding_ids: Vec<String>,
    pub validation_receipt_refs: Vec<String>,
    pub validation_omissions: Vec<ValidationPlanOmission>,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HostSettlementService;

impl HostSettlementService {
    pub fn settle(
        &self,
        transaction: &ChangeTransactionSnapshot,
        findings: &[ReviewFinding],
    ) -> HostSettlementReceipt {
        let finding_ids = findings
            .iter()
            .map(|finding| finding.finding_id.clone())
            .collect();
        let validation_receipt_refs = transaction
            .validation_receipts
            .iter()
            .enumerate()
            .map(|(index, receipt)| {
                receipt.output_ref.clone().unwrap_or_else(|| {
                    format!("validation://{}/{}", transaction.transaction_id.0, index)
                })
            })
            .collect();
        let decision = if self.acceptable(transaction, findings) {
            HostSettlementDecision::Accepted
        } else {
            HostSettlementDecision::Repair
        };
        let reason = match decision {
            HostSettlementDecision::Accepted => "host validation and review evidence passed".into(),
            HostSettlementDecision::Repair => self.repair_reason(transaction, findings),
        };
        HostSettlementReceipt {
            transaction_id: transaction.transaction_id.0.to_string(),
            session_id: transaction.owner_session_id.clone(),
            workspace_version: transaction.current.digest.clone(),
            decision,
            finding_ids,
            validation_receipt_refs,
            validation_omissions: transaction.validation_omissions.clone(),
            reason,
        }
    }

    fn acceptable(
        &self,
        transaction: &ChangeTransactionSnapshot,
        findings: &[ReviewFinding],
    ) -> bool {
        transaction.phase == ChangeTransactionPhase::Validated
            && transaction
                .validation_plan
                .iter()
                .filter(|step| step.required)
                .all(|step| {
                    transaction.validation_receipts.iter().any(|receipt| {
                        receipt.validation_kind == step.validation_kind
                            && receipt.command == step.command
                            && receipt.workspace_version == transaction.current.digest
                            && receipt.terminal_status == "succeeded"
                    })
                })
            && findings
                .iter()
                .all(|finding| finding.status == ReviewFindingStatus::Resolved)
            && transaction.validation_omissions.is_empty()
    }

    fn repair_reason(
        &self,
        transaction: &ChangeTransactionSnapshot,
        findings: &[ReviewFinding],
    ) -> String {
        if transaction.phase != ChangeTransactionPhase::Validated {
            return format!("transaction phase {:?} is not validated", transaction.phase);
        }
        if findings
            .iter()
            .any(|finding| finding.status != ReviewFindingStatus::Resolved)
        {
            return "unresolved review finding requires repair".into();
        }
        if !transaction.validation_omissions.is_empty() {
            return "validation omission requires explicit repair or risk disposition".into();
        }
        "required validation evidence is incomplete or failed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::change_transaction::{
        ChangeTransactionId, ChangedRange, ValidationImpact, ValidationPlanStep, ValidationRisk,
        VersionedValidationReceipt, WorkspaceVersion, WorkspaceVersionBasis,
    };

    fn version(digest: &str) -> WorkspaceVersion {
        WorkspaceVersion {
            digest: digest.into(),
            basis: WorkspaceVersionBasis::BoundedTree,
            root: "/workspace".into(),
            head: None,
            changed_paths: vec!["src/lib.rs".into()],
        }
    }

    fn transaction(phase: ChangeTransactionPhase) -> ChangeTransactionSnapshot {
        ChangeTransactionSnapshot {
            transaction_id: ChangeTransactionId::new(),
            owner_session_id: "session".into(),
            owner_turn_id: Some("turn".into()),
            owner_agent: None,
            root: "/workspace".into(),
            baseline: version("before"),
            current: version("after"),
            phase,
            mutation_coverage: fabric::change_transaction::MutationCoverage::Full,
            compensation_ref: None,
            changed_paths: vec!["src/lib.rs".into()],
            changed_ranges: vec![ChangedRange {
                path: "src/lib.rs".into(),
                start_line: 1,
                end_line: 2,
            }],
            diff_artifact_ref: Some("artifact://diff".into()),
            validation_plan: vec![ValidationPlanStep {
                id: "test".into(),
                validation_kind: "test".into(),
                command: "cargo test".into(),
                reason: "changed Rust source".into(),
                source: "host".into(),
                required: true,
            }],
            validation_omissions: vec![],
            validation_impact: ValidationImpact::PackageLocal,
            validation_risk: ValidationRisk::Moderate,
            validation_receipts: vec![VersionedValidationReceipt {
                validation_kind: "test".into(),
                command: "cargo test".into(),
                workspace_version: "after".into(),
                terminal_status: "succeeded".into(),
                output_ref: Some("artifact://test".into()),
            }],
            accepted_workspace_version: None,
            active_command: None,
            failure: None,
        }
    }

    fn finding(resolved: bool) -> ReviewFinding {
        ReviewFinding {
            finding_id: "finding-1".into(),
            severity: fabric::ReviewFindingSeverity::Error,
            summary: "fixture".into(),
            location: Some(fabric::ReviewFindingLocation {
                path: "src/lib.rs".into(),
                line: Some(1),
                column: None,
            }),
            evidence_refs: vec!["artifact://review".into()],
            status: if resolved {
                ReviewFindingStatus::Resolved
            } else {
                ReviewFindingStatus::Open
            },
            repair_link: Some("root:repair".into()),
        }
    }

    #[test]
    fn u_verify_001_model_completion_cannot_accept_failed_phase() {
        assert_eq!(
            HostSettlementService
                .settle(
                    &transaction(ChangeTransactionPhase::Repair),
                    &[finding(true)]
                )
                .decision,
            HostSettlementDecision::Repair
        );
    }

    #[test]
    fn unresolved_finding_returns_repair() {
        let receipt = HostSettlementService.settle(
            &transaction(ChangeTransactionPhase::Validated),
            &[finding(false)],
        );
        assert_eq!(receipt.decision, HostSettlementDecision::Repair);
        assert_eq!(receipt.finding_ids, vec!["finding-1"]);
    }

    #[test]
    fn u_verify_003_acceptance_receipt_carries_validation_artifact_and_version() {
        let receipt = HostSettlementService.settle(
            &transaction(ChangeTransactionPhase::Validated),
            &[finding(true)],
        );
        assert_eq!(receipt.decision, HostSettlementDecision::Accepted);
        assert_eq!(receipt.workspace_version, "after");
        assert_eq!(receipt.validation_receipt_refs, vec!["artifact://test"]);
    }

    #[test]
    fn u_verify_004_omitted_validation_cannot_be_silent() {
        let mut tx = transaction(ChangeTransactionPhase::Validated);
        tx.validation_omissions.push(ValidationPlanOmission {
            validation_kind: "integration".into(),
            reason: "external service unavailable".into(),
        });
        let receipt = HostSettlementService.settle(&tx, &[finding(true)]);
        assert_eq!(receipt.decision, HostSettlementDecision::Repair);
        assert_eq!(receipt.validation_omissions.len(), 1);
    }

    #[test]
    fn a_turn_002_failed_validation_returns_repair_not_completed() {
        let mut tx = transaction(ChangeTransactionPhase::DiffReviewed);
        tx.validation_receipts[0].terminal_status = "failed".into();
        assert_eq!(
            HostSettlementService.settle(&tx, &[finding(true)]).decision,
            HostSettlementDecision::Repair
        );
    }
}
