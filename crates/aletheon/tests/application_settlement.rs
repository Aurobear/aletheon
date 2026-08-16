use ::contracts::change_transaction::{
    ChangeTransactionId, ChangedRange, ValidationImpact, ValidationPlanOmission,
    ValidationPlanStep, ValidationRisk, VersionedValidationReceipt, WorkspaceVersion,
    WorkspaceVersionBasis,
};
use application::settlement::*;
use contracts::change_transaction::{ChangeTransactionPhase, ChangeTransactionSnapshot};
use contracts::{ReviewFinding, ReviewFindingStatus};
use corpus::tools::tools::change_transaction::ChangeTransactionRegistry;

#[derive(Clone)]
struct RegistryAuthority(ChangeTransactionRegistry);

#[async_trait::async_trait]
impl ChangeTransactionAuthority for RegistryAuthority {
    async fn snapshot(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
    ) -> anyhow::Result<Option<ChangeTransactionSnapshot>> {
        Ok(self.0.snapshot(transaction_id).await)
    }

    async fn accept(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        self.0
            .accept(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }

    async fn request_repair(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        self.0
            .request_repair(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }

    async fn rollback(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot> {
        self.0
            .rollback(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }
}

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
        mutation_coverage: ::contracts::change_transaction::MutationCoverage::Full,
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
        severity: ::contracts::ReviewFindingSeverity::Error,
        summary: "fixture".into(),
        location: Some(::contracts::ReviewFindingLocation {
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
            .evaluate(
                &transaction(ChangeTransactionPhase::Repair),
                &[finding(true)]
            )
            .decision,
        HostSettlementDecision::RepairRequired
    );
}

#[test]
fn unresolved_finding_returns_repair() {
    let receipt = HostSettlementService.evaluate(
        &transaction(ChangeTransactionPhase::Validated),
        &[finding(false)],
    );
    assert_eq!(receipt.decision, HostSettlementDecision::RepairRequired);
    assert_eq!(receipt.finding_ids, vec!["finding-1"]);
}

#[test]
fn u_verify_003_acceptance_receipt_carries_validation_artifact_and_version() {
    let receipt = HostSettlementService.evaluate(
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
    let receipt = HostSettlementService.evaluate(&tx, &[finding(true)]);
    assert_eq!(receipt.decision, HostSettlementDecision::RepairRequired);
    assert_eq!(receipt.validation_omissions.len(), 1);
}

#[test]
fn a_turn_002_failed_validation_returns_repair_not_completed() {
    let mut tx = transaction(ChangeTransactionPhase::DiffReviewed);
    tx.validation_receipts[0].terminal_status = "failed".into();
    assert_eq!(
        HostSettlementService
            .evaluate(&tx, &[finding(true)])
            .decision,
        HostSettlementDecision::RepairRequired
    );
}

#[tokio::test]
async fn host_review_service_executes_accept_repair_and_coverage_gated_rollback() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("tracked.txt"), "baseline").unwrap();
    let registry = ChangeTransactionRegistry::default();
    let accepted = registry.begin("accepted", temp.path()).await.unwrap();
    registry
        .record_apply(accepted.transaction_id, accepted.current.clone())
        .await
        .unwrap();
    registry
        .record_diff_review(accepted.transaction_id, "artifact://diff".into(), vec![])
        .await
        .unwrap();
    let service = TransactionReviewService::new(
        std::sync::Arc::new(RegistryAuthority(registry.clone())),
        std::sync::Arc::new(InMemoryTransactionSettlementStore::default()),
    );
    let accepted = service
        .review(
            TransactionReviewAction::Accept,
            accepted.transaction_id,
            "accepted",
            temp.path(),
            &[],
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        accepted.settlement.decision,
        HostSettlementDecision::Accepted
    );
    assert_eq!(accepted.transaction.phase, ChangeTransactionPhase::Accepted);

    let repair = registry.begin("repair", temp.path()).await.unwrap();
    let repair = service
        .review(
            TransactionReviewAction::Repair,
            repair.transaction_id,
            "repair",
            temp.path(),
            &[],
            false,
        )
        .await
        .unwrap();
    assert_eq!(repair.transaction.phase, ChangeTransactionPhase::Repair);

    let rollback = registry.begin("rollback", temp.path()).await.unwrap();
    let rejected = service
        .review(
            TransactionReviewAction::Rollback,
            rollback.transaction_id,
            "rollback",
            temp.path(),
            &[],
            false,
        )
        .await
        .unwrap_err();
    assert!(rejected
        .to_string()
        .contains("explicit risk acknowledgement"));
    let rolled_back = service
        .review(
            TransactionReviewAction::Rollback,
            rollback.transaction_id,
            "rollback",
            temp.path(),
            &[],
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        rolled_back.settlement.decision,
        HostSettlementDecision::RolledBack
    );
    assert_eq!(
        rolled_back.transaction.phase,
        ChangeTransactionPhase::RolledBack
    );
}
