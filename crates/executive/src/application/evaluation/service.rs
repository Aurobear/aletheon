use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use fabric::{
    EvaluationReceipt, EvaluationReceiptId, OperationKind, OperationManager, OperationRequest,
    TaskEvaluationContract, TurnRequest, EVALUATION_SCHEMA_V1,
};
use kernel::KernelRuntime;

use super::{
    coding_v2_rubric, CodingDimensionScorer, CodingEvidenceCollector,
    DefaultTaskEvaluationContractIssuer, EvaluationReceiptStore, EvaluationSettlementPolicy,
    TaskEvaluationContractIssuer, TurnEvaluationArtifacts,
};

pub struct EvaluationService {
    kernel: Arc<KernelRuntime>,
    issuer: Arc<dyn TaskEvaluationContractIssuer>,
    collector: Arc<dyn CodingEvidenceCollector>,
    scorer: Arc<dyn CodingDimensionScorer>,
    store: Arc<dyn EvaluationReceiptStore>,
    max_evaluation_ms: u64,
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
        })
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
                created_at_ms: Utc::now().timestamp_millis(),
            };
            receipt.validate(contract, &snapshot)?;
            self.store.append_evaluation(&snapshot, &receipt).await?;
            Ok::<_, anyhow::Error>(receipt)
        };

        match tokio::time::timeout(Duration::from_millis(self.max_evaluation_ms), settle).await {
            Ok(Ok(receipt)) => {
                self.kernel.succeed_operation(operation.id).await?;
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
