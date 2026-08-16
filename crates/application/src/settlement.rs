//! Host-owned review and settlement decision boundary (X8c).
//!
//! This module consumes typed transaction/validation evidence and never
//! trusts a model's textual claim of completion. It computes the only
//! accepted terminal decision; persistence and transport remain outside this
//! pure authority evaluator.

use ::contracts::change_transaction::{ChangeTransactionPhase, ChangeTransactionSnapshot};
use ::contracts::{ReviewFinding, ReviewFindingStatus};
pub use ::contracts::{
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
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
    ) -> anyhow::Result<Option<ChangeTransactionSnapshot>>;

    async fn accept(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot>;

    async fn request_repair(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<ChangeTransactionSnapshot>;

    async fn rollback(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
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
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
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
                use ::contracts::change_transaction::MutationCoverage;
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
