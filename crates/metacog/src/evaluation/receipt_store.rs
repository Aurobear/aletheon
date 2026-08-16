//! Durable evaluation receipt persistence port.

use async_trait::async_trait;

#[async_trait]
pub trait EvaluationReceiptStore: Send + Sync {
    async fn append_contract(
        &self,
        contract: &::contracts::TaskEvaluationContract,
    ) -> anyhow::Result<()>;
    async fn append_evaluation(
        &self,
        snapshot: &::contracts::EvaluationEvidenceSnapshot,
        receipt: &::contracts::EvaluationReceipt,
    ) -> anyhow::Result<()>;
    async fn get_receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationReceipt>>;
    async fn get_snapshot(
        &self,
        id: &::contracts::EvaluationSnapshotId,
    ) -> anyhow::Result<Option<::contracts::EvaluationEvidenceSnapshot>>;
    async fn get_snapshot_for_receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationEvidenceSnapshot>>;
    async fn latest_for_subject(
        &self,
        subject: &::contracts::EvaluationSubject,
    ) -> anyhow::Result<Option<::contracts::EvaluationReceipt>>;
}
