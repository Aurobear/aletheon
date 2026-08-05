//! Host-owned review and settlement decision boundary (X8c).
//!
//! This module consumes typed transaction/validation evidence and never
//! trusts a model's textual claim of completion. It computes the only
//! accepted terminal decision; persistence and transport remain outside this
//! pure authority evaluator.

use fabric::change_transaction::{ChangeTransactionPhase, ChangeTransactionSnapshot};
use fabric::{ReviewFinding, ReviewFindingStatus};
pub use fabric::{
    TransactionReviewAction, TransactionReviewSnapshot as TransactionReviewOutcome,
    TransactionSettlementDecision as HostSettlementDecision,
    TransactionSettlementReceipt as HostSettlementReceipt,
};

#[async_trait::async_trait]
pub trait TransactionSettlementStore: Send + Sync {
    async fn append(&self, receipt: &HostSettlementReceipt) -> anyhow::Result<()>;
    async fn latest(
        &self,
        session_id: &str,
        transaction_id: &str,
    ) -> anyhow::Result<Option<HostSettlementReceipt>>;
}

#[derive(Default)]
pub struct InMemoryTransactionSettlementStore {
    receipts: tokio::sync::Mutex<std::collections::BTreeMap<String, HostSettlementReceipt>>,
}

#[async_trait::async_trait]
impl TransactionSettlementStore for InMemoryTransactionSettlementStore {
    async fn append(&self, receipt: &HostSettlementReceipt) -> anyhow::Result<()> {
        let mut receipts = self.receipts.lock().await;
        if let Some(existing) = receipts.get(&receipt.settlement_id) {
            anyhow::ensure!(existing == receipt, "settlement idempotency conflict");
            return Ok(());
        }
        receipts.insert(receipt.settlement_id.clone(), receipt.clone());
        Ok(())
    }

    async fn latest(
        &self,
        session_id: &str,
        transaction_id: &str,
    ) -> anyhow::Result<Option<HostSettlementReceipt>> {
        Ok(self
            .receipts
            .lock()
            .await
            .values()
            .rev()
            .find(|receipt| {
                receipt.session_id == session_id && receipt.transaction_id == transaction_id
            })
            .cloned())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HostSettlementService;

impl HostSettlementService {
    pub fn evaluate(
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
            HostSettlementDecision::RepairRequired
        };
        let reason = match decision {
            HostSettlementDecision::Accepted => "host validation and review evidence passed".into(),
            HostSettlementDecision::RepairRequired => self.repair_reason(transaction, findings),
            HostSettlementDecision::RolledBack => "host restored the transaction baseline".into(),
        };
        HostSettlementReceipt {
            settlement_id: settlement_id(transaction, decision),
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

fn settlement_id(
    transaction: &ChangeTransactionSnapshot,
    decision: HostSettlementDecision,
) -> String {
    let material = format!(
        "{}\0{}\0{}\0{decision:?}",
        transaction.transaction_id.0, transaction.owner_session_id, transaction.current.digest
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, material.as_bytes()).to_string()
}

#[async_trait::async_trait]
pub trait ChangeTransactionAuthority: Send + Sync {
    async fn snapshot(
        &self,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
    ) -> anyhow::Result<Option<ChangeTransactionSnapshot>>;

    async fn accept(
        &self,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot>;

    async fn request_repair(
        &self,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot>;

    async fn rollback(
        &self,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<fabric::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot>;
}

#[derive(Clone)]
pub struct TransactionReviewService {
    transactions: std::sync::Arc<dyn ChangeTransactionAuthority>,
    store: std::sync::Arc<dyn TransactionSettlementStore>,
}

impl TransactionReviewService {
    pub fn new(
        transactions: std::sync::Arc<dyn ChangeTransactionAuthority>,
        store: std::sync::Arc<dyn TransactionSettlementStore>,
    ) -> Self {
        Self {
            transactions,
            store,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn review(
        &self,
        action: TransactionReviewAction,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
        session_id: &str,
        root: &std::path::Path,
        findings: &[ReviewFinding],
        risk_acknowledged: bool,
    ) -> anyhow::Result<TransactionReviewOutcome> {
        let current = self
            .transactions
            .snapshot(transaction_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown change transaction"))?;
        anyhow::ensure!(
            current.owner_session_id == session_id,
            "transaction belongs to another session"
        );
        anyhow::ensure!(
            std::path::Path::new(&current.root) == root,
            "transaction workspace differs from host authority"
        );
        let (transaction, settlement) = match action {
            TransactionReviewAction::Accept => {
                let settlement = HostSettlementService.evaluate(&current, findings);
                anyhow::ensure!(
                    settlement.decision == HostSettlementDecision::Accepted,
                    "{}",
                    settlement.reason
                );
                let transaction = self
                    .transactions
                    .accept(transaction_id, session_id, None, root)
                    .await?;
                (transaction, settlement)
            }
            TransactionReviewAction::Repair => {
                let transaction = self
                    .transactions
                    .request_repair(transaction_id, session_id, None, root)
                    .await?;
                let settlement = HostSettlementService.evaluate(&transaction, findings);
                (transaction, settlement)
            }
            TransactionReviewAction::Rollback => {
                use fabric::change_transaction::MutationCoverage;
                match current.mutation_coverage {
                    MutationCoverage::Full => {}
                    MutationCoverage::BestEffort if risk_acknowledged => {}
                    MutationCoverage::BestEffort => {
                        anyhow::bail!("best-effort rollback requires explicit risk acknowledgement")
                    }
                    MutationCoverage::NonRollbackable => {
                        anyhow::bail!("transaction declares non-rollbackable effects")
                    }
                }
                let transaction = self
                    .transactions
                    .rollback(transaction_id, session_id, None, root)
                    .await?;
                let mut settlement = HostSettlementService.evaluate(&transaction, findings);
                settlement.decision = HostSettlementDecision::RolledBack;
                settlement.reason = "host restored the transaction baseline".into();
                (transaction, settlement)
            }
        };
        self.store.append(&settlement).await?;
        Ok(TransactionReviewOutcome {
            transaction,
            settlement,
        })
    }

    pub async fn latest(
        &self,
        session_id: &str,
        transaction_id: &str,
    ) -> anyhow::Result<Option<HostSettlementReceipt>> {
        self.store.latest(session_id, transaction_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::tools::tools::change_transaction::ChangeTransactionRegistry;
    use fabric::change_transaction::{
        ChangeTransactionId, ChangedRange, ValidationImpact, ValidationPlanOmission,
        ValidationPlanStep, ValidationRisk, VersionedValidationReceipt, WorkspaceVersion,
        WorkspaceVersionBasis,
    };

    #[derive(Clone)]
    struct RegistryAuthority(ChangeTransactionRegistry);

    #[async_trait::async_trait]
    impl ChangeTransactionAuthority for RegistryAuthority {
        async fn snapshot(
            &self,
            transaction_id: fabric::change_transaction::ChangeTransactionId,
        ) -> anyhow::Result<Option<ChangeTransactionSnapshot>> {
            Ok(self.0.snapshot(transaction_id).await)
        }

        async fn accept(
            &self,
            transaction_id: fabric::change_transaction::ChangeTransactionId,
            owner_session_id: &str,
            owner_agent: Option<fabric::AgentToolContext>,
            root: &std::path::Path,
        ) -> anyhow::Result<ChangeTransactionSnapshot> {
            self.0
                .accept(transaction_id, owner_session_id, owner_agent, root)
                .await
                .map_err(|failure| anyhow::anyhow!(failure.summary))
        }

        async fn request_repair(
            &self,
            transaction_id: fabric::change_transaction::ChangeTransactionId,
            owner_session_id: &str,
            owner_agent: Option<fabric::AgentToolContext>,
            root: &std::path::Path,
        ) -> anyhow::Result<ChangeTransactionSnapshot> {
            self.0
                .request_repair(transaction_id, owner_session_id, owner_agent, root)
                .await
                .map_err(|failure| anyhow::anyhow!(failure.summary))
        }

        async fn rollback(
            &self,
            transaction_id: fabric::change_transaction::ChangeTransactionId,
            owner_session_id: &str,
            owner_agent: Option<fabric::AgentToolContext>,
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
            HostSettlementService.evaluate(&tx, &[finding(true)]).decision,
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
}
