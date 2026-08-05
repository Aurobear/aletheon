use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use fabric::{
    EvaluationReceipt, EvaluationReceiptId, OperationKind, OperationManager, OperationRequest,
    TaskEvaluationContract, TurnRequest, EVALUATION_SCHEMA_V1,
};
use kernel::KernelRuntime;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    coding_v2_rubric, CodingDimensionScorer, CodingEvidenceCollector,
    DefaultTaskEvaluationContractIssuer, EvaluationProjection, EvaluationProjectionContext,
    EvaluationProjectionRecord, EvaluationReceiptStore, EvaluationSettlementPolicy,
    TaskEvaluationContractIssuer, TurnEvaluationArtifacts,
};

pub struct EvaluationService {
    kernel: Arc<KernelRuntime>,
    issuer: Arc<dyn TaskEvaluationContractIssuer>,
    collector: Arc<dyn CodingEvidenceCollector>,
    scorer: Arc<dyn CodingDimensionScorer>,
    store: Arc<dyn EvaluationReceiptStore>,
    max_evaluation_ms: u64,
    projection: Option<Arc<EvaluationProjection>>,
    capability_rollups:
        Option<Arc<crate::application::capability_benchmark::CapabilityRollupProjectionSink>>,
}

impl EvaluationService {
    pub fn new(
        kernel: Arc<KernelRuntime>,
        settings: crate::composition::config::EvaluationSettings,
        store: Arc<dyn EvaluationReceiptStore>,
    ) -> anyhow::Result<Self> {
        let max_evaluation_ms = settings.max_evaluation_ms;
        Ok(Self {
            kernel,
            issuer: Arc::new(DefaultTaskEvaluationContractIssuer::new(settings)?),
            collector: Arc::new(super::DefaultCodingEvidenceCollector),
            scorer: Arc::new(super::CodingV2Scorer),
            store,
            max_evaluation_ms,
            projection: None,
            capability_rollups: None,
        })
    }

    pub fn with_projection(mut self, projection: Arc<EvaluationProjection>) -> Self {
        self.projection = Some(projection);
        self
    }

    pub fn with_capability_rollups(
        mut self,
        rollups: Arc<crate::application::capability_benchmark::CapabilityRollupProjectionSink>,
    ) -> Self {
        self.capability_rollups = Some(rollups);
        self
    }

    pub fn capability_rollup_snapshot(
        &self,
    ) -> std::collections::HashMap<
        crate::application::capability_benchmark::CapabilityRollupKey,
        crate::application::capability_benchmark::CapabilityReceiptRollup,
    > {
        self.capability_rollups
            .as_ref()
            .map(|rollups| rollups.snapshot())
            .unwrap_or_default()
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

    /// Read the persisted authoritative receipt. Authorization is deliberately
    /// enforced by the host-facing query use case before this store lookup.
    pub async fn receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<EvaluationReceipt>> {
        self.store.get_receipt(id).await
    }

    /// Read the evidence snapshot correlated to a persisted receipt. Callers
    /// must establish principal/session/workspace authority before requesting it.
    pub async fn evidence_snapshot_for_receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> anyhow::Result<Option<fabric::EvaluationEvidenceSnapshot>> {
        self.store.get_snapshot_for_receipt(id).await
    }

    pub async fn evaluate(
        &self,
        contract: &TaskEvaluationContract,
        artifacts: &TurnEvaluationArtifacts,
        owner: fabric::ProcessId,
        parent_turn_operation: fabric::OperationId,
    ) -> anyhow::Result<EvaluationReceipt> {
        let operation = self
            .kernel
            .submit(OperationRequest {
                owner,
                parent: Some(parent_turn_operation),
                kind: OperationKind::Evaluation,
                deadline: Some(fabric::MonoDeadline::after(
                    self.kernel.clock().mono_now(),
                    self.max_evaluation_ms,
                )),
            })
            .await?;
        self.kernel.start_operation(operation.id).await?;

        let settle = async {
            let snapshot = self.collector.collect(contract, artifacts)?;
            let scored = self.scorer.score(contract, &snapshot)?;
            let report = metacog::evaluation::DeterministicEvaluator::new()
                .evaluate_evidence_backed(
                    &coding_v2_rubric(),
                    scored.dimensions,
                    scored.gates,
                    &snapshot.evidence,
                    contract.thresholds,
                )
                .map_err(|error| anyhow::anyhow!(error))?;
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
                evaluation_operation_id: operation.id,
                subject: contract.subject.clone(),
                evidence_snapshot_sha256: snapshot.sha256.clone(),
                decision: EvaluationSettlementPolicy::decision(contract.mode, &report),
                report,
                failed_gates,
                evaluator: "metacog.deterministic/coding-v2@2".into(),
                execution: fabric::EvaluationExecutionContext {
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
                created_at_ms: Utc::now().timestamp_millis(),
            };
            receipt.validate(contract, &snapshot)?;
            self.store.append_evaluation(&snapshot, &receipt).await?;
            Ok::<_, anyhow::Error>(receipt)
        };

        match tokio::time::timeout(Duration::from_millis(self.max_evaluation_ms), settle).await {
            Ok(Ok(receipt)) => {
                self.kernel.succeed_operation(operation.id).await?;
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
                    // Complete the bounded observational fan-out before the
                    // Host applies its task-graph settlement. Otherwise the
                    // Agora observation can race the authoritative repair
                    // transition at the same workspace version.
                    let _ = projection.project(record).await;
                }
                Ok(receipt)
            }
            Ok(Err(error)) => {
                self.kernel
                    .fail_operation(operation.id, error.to_string())
                    .await?;
                Err(error)
            }
            Err(_) => {
                let error =
                    anyhow::anyhow!("evaluation exceeded {} ms timeout", self.max_evaluation_ms);
                self.kernel
                    .fail_operation(operation.id, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }
}

fn digest_json(value: &impl Serialize) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
