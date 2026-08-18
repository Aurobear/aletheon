//! Evaluation use-case coordination with all authority/effects behind ports.

use super::{
    CodingDimensionScorer, CodingEvidenceCollector, EvaluationProjection,
    EvaluationProjectionContext, EvaluationProjectionRecord, ScoredEvaluationInput,
    TaskEvaluationContractIssuer, TurnEvaluationArtifacts,
};
use async_trait::async_trait;
use contracts::{
    Clock, EvaluationDecision, EvaluationMode, EvaluationReceipt, EvaluationReceiptId,
    EvaluationSubject, EvaluationThresholds, OperationId, ProcessId, TaskEvaluationContract,
    TurnRequest, EVALUATION_SCHEMA_V1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

#[async_trait]
pub trait EvaluationReceiptStore: Send + Sync {
    async fn append_contract(&self, contract: &TaskEvaluationContract) -> anyhow::Result<()>;
    async fn append_evaluation(
        &self,
        snapshot: &contracts::EvaluationEvidenceSnapshot,
        receipt: &EvaluationReceipt,
    ) -> anyhow::Result<()>;
    async fn get_receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<EvaluationReceipt>>;
    async fn get_snapshot(
        &self,
        id: &contracts::EvaluationSnapshotId,
    ) -> anyhow::Result<Option<contracts::EvaluationEvidenceSnapshot>>;
    async fn get_snapshot_for_receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<contracts::EvaluationEvidenceSnapshot>>;
    async fn latest_for_subject(
        &self,
        subject: &EvaluationSubject,
    ) -> anyhow::Result<Option<EvaluationReceipt>>;
}

pub trait EvaluationEnginePort: Send + Sync {
    fn evaluate(
        &self,
        scored: ScoredEvaluationInput,
        evidence: &[contracts::types::metacognition_evidence::EvidenceItem],
        thresholds: EvaluationThresholds,
    ) -> anyhow::Result<contracts::types::metacognition_evaluation::EvaluationReport>;
}

#[async_trait]
pub trait EvaluationOperationPort: Send + Sync {
    async fn start(
        &self,
        owner: ProcessId,
        parent: OperationId,
        timeout_ms: u64,
    ) -> anyhow::Result<OperationId>;
    async fn succeed(&self, operation: OperationId) -> anyhow::Result<()>;
    async fn fail(&self, operation: OperationId, error: String) -> anyhow::Result<()>;
}

pub struct EvaluationService {
    issuer: Arc<dyn TaskEvaluationContractIssuer>,
    collector: Arc<dyn CodingEvidenceCollector>,
    scorer: Arc<dyn CodingDimensionScorer>,
    engine: Arc<dyn EvaluationEnginePort>,
    operations: Arc<dyn EvaluationOperationPort>,
    store: Arc<dyn EvaluationReceiptStore>,
    clock: Arc<dyn Clock>,
    max_evaluation_ms: u64,
    projection: Option<Arc<EvaluationProjection>>,
}

impl EvaluationService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer: Arc<dyn TaskEvaluationContractIssuer>,
        collector: Arc<dyn CodingEvidenceCollector>,
        scorer: Arc<dyn CodingDimensionScorer>,
        engine: Arc<dyn EvaluationEnginePort>,
        operations: Arc<dyn EvaluationOperationPort>,
        store: Arc<dyn EvaluationReceiptStore>,
        clock: Arc<dyn Clock>,
        max_evaluation_ms: u64,
    ) -> anyhow::Result<Self> {
        if max_evaluation_ms == 0 {
            anyhow::bail!("evaluation timeout must be positive");
        }
        Ok(Self {
            issuer,
            collector,
            scorer,
            engine,
            operations,
            store,
            clock,
            max_evaluation_ms,
            projection: None,
        })
    }

    pub fn with_projection(mut self, projection: Arc<EvaluationProjection>) -> Self {
        self.projection = Some(projection);
        self
    }

    pub async fn issue_contract(
        &self,
        request: &TurnRequest,
    ) -> anyhow::Result<Option<TaskEvaluationContract>> {
        let contract = self.issuer.issue(request)?;
        if let Some(contract) = &contract {
            self.store.append_contract(contract).await?;
        }
        Ok(contract)
    }

    pub async fn receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<EvaluationReceipt>> {
        self.store.get_receipt(id).await
    }

    pub async fn evidence_snapshot_for_receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<contracts::EvaluationEvidenceSnapshot>> {
        self.store.get_snapshot_for_receipt(id).await
    }

    pub async fn evaluate(
        &self,
        contract: &TaskEvaluationContract,
        artifacts: &TurnEvaluationArtifacts,
        owner: ProcessId,
        parent_turn_operation: OperationId,
    ) -> anyhow::Result<EvaluationReceipt> {
        let operation = self
            .operations
            .start(owner, parent_turn_operation, self.max_evaluation_ms)
            .await?;
        let settle = async {
            let snapshot = self.collector.collect(contract, artifacts)?;
            let scored = self.scorer.score(contract, &snapshot)?;
            let report = self
                .engine
                .evaluate(scored, &snapshot.evidence, contract.thresholds)?;
            let failed_gates = report
                .gates
                .iter()
                .filter(|gate| !gate.passed)
                .map(|gate| gate.name.clone())
                .collect::<Vec<_>>();
            let receipt = EvaluationReceipt {
                schema_version: EVALUATION_SCHEMA_V1,
                receipt_id: EvaluationReceiptId::new(),
                contract_id: contract.contract_id,
                evaluation_operation_id: operation,
                subject: contract.subject.clone(),
                evidence_snapshot_sha256: snapshot.sha256.clone(),
                decision: decision(contract.mode, &report),
                report,
                failed_gates,
                evaluator: "metacog.deterministic/coding-v2@2".into(),
                execution: contracts::EvaluationExecutionContext {
                    runtime_id: artifacts.runtime_id.clone(),
                    agent_profile: artifacts.profile_name.clone(),
                    effective_model_id: artifacts.effective_model_id.clone(),
                    model_display_name: artifacts.model_display_name.clone(),
                    workspace_boundary_sha256: digest_json(&artifacts.workspace)?,
                    verification_selection_sha256: digest_json(
                        &artifacts
                            .capability_receipts
                            .iter()
                            .filter(|receipt| receipt.capability == "validation_run")
                            .collect::<Vec<_>>(),
                    )?,
                },
                created_at_ms: self.clock.wall_now().0,
            };
            receipt.validate(contract, &snapshot)?;
            self.store.append_evaluation(&snapshot, &receipt).await?;
            Ok::<_, anyhow::Error>(receipt)
        };
        match tokio::time::timeout(Duration::from_millis(self.max_evaluation_ms), settle).await {
            Ok(Ok(receipt)) => {
                self.operations.succeed(operation).await?;
                if let Some(projection) = self.projection.clone() {
                    let record = EvaluationProjectionRecord {
                        receipt: receipt.reference(),
                        context: EvaluationProjectionContext {
                            session_id: artifacts.session_id.clone(),
                            runtime_id: artifacts.runtime_id.clone(),
                            profile_id: artifacts.profile_name.clone(),
                            effective_model_id: receipt.execution.effective_model_id.clone(),
                            model_display_name: receipt.execution.model_display_name.clone(),
                            workspace_boundary_sha256: receipt
                                .execution
                                .workspace_boundary_sha256
                                .clone(),
                            verification_selection_sha256: receipt
                                .execution
                                .verification_selection_sha256
                                .clone(),
                            rubric_id: contract.rubric.0.clone(),
                            rubric_version: contract.rubric_version,
                            process_id: owner,
                            metrics: artifacts.projection_metrics.clone(),
                        },
                    };
                    let _ = projection.project(record).await;
                }
                Ok(receipt)
            }
            Ok(Err(error)) => {
                self.operations.fail(operation, error.to_string()).await?;
                Err(error)
            }
            Err(_) => {
                let error =
                    anyhow::anyhow!("evaluation exceeded {} ms timeout", self.max_evaluation_ms);
                self.operations.fail(operation, error.to_string()).await?;
                Err(error)
            }
        }
    }
}

fn decision(
    mode: EvaluationMode,
    report: &contracts::types::metacognition_evaluation::EvaluationReport,
) -> EvaluationDecision {
    match (mode, report.eligible) {
        (EvaluationMode::Shadow, true) => EvaluationDecision::ObservedPass,
        (EvaluationMode::Shadow, false) => EvaluationDecision::ObservedFail,
        (EvaluationMode::Enforce, true) => EvaluationDecision::Accepted,
        (EvaluationMode::Enforce, false) => EvaluationDecision::Rejected,
    }
}

fn digest_json(value: &impl Serialize) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
