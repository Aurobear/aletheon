//! Concrete Metacog evaluation engine and Kernel operation adapters.

mod agent_sink;
mod coding_scorer;
mod host_acceptance;
pub use agent_sink::AgentEvaluationProjectionSink;
pub use coding_scorer::CodingV2Scorer;
pub use host_acceptance::AgoraEvaluationAcceptance;

use application::evaluation::{
    EvaluationEnginePort, EvaluationOperationPort, ScoredEvaluationInput,
};
use async_trait::async_trait;
use contracts::{OperationId, OperationKind, OperationRequest, ProcessId};
use kernel::{KernelRuntime, OperationManager};
use std::sync::Arc;

pub struct MetacogCodingV2Engine;

impl EvaluationEnginePort for MetacogCodingV2Engine {
    fn evaluate(
        &self,
        scored: ScoredEvaluationInput,
        evidence: &[contracts::types::metacognition_evidence::EvidenceItem],
        thresholds: contracts::EvaluationThresholds,
    ) -> anyhow::Result<contracts::types::metacognition_evaluation::EvaluationReport> {
        metacog::evaluation::DeterministicEvaluator::new()
            .evaluate_evidence_backed(
                &metacog::evaluation::coding_v2_rubric(),
                scored.dimensions,
                scored.gates,
                evidence,
                thresholds,
            )
            .map_err(anyhow::Error::msg)
    }
}

pub struct KernelEvaluationOperations {
    kernel: Arc<KernelRuntime>,
}

impl KernelEvaluationOperations {
    pub fn new(kernel: Arc<KernelRuntime>) -> Self {
        Self { kernel }
    }
}

#[async_trait]
impl EvaluationOperationPort for KernelEvaluationOperations {
    async fn start(
        &self,
        owner: ProcessId,
        parent: OperationId,
        timeout_ms: u64,
    ) -> anyhow::Result<OperationId> {
        let operation = self
            .kernel
            .submit(OperationRequest {
                owner,
                parent: Some(parent),
                kind: OperationKind::Evaluation,
                deadline: Some(contracts::MonoDeadline::after(
                    self.kernel.clock().mono_now(),
                    timeout_ms,
                )),
            })
            .await?;
        self.kernel.start_operation(operation.id).await?;
        Ok(operation.id)
    }

    async fn succeed(&self, operation: OperationId) -> anyhow::Result<()> {
        self.kernel
            .succeed_operation(operation)
            .await
            .map_err(Into::into)
    }

    async fn fail(&self, operation: OperationId, error: String) -> anyhow::Result<()> {
        self.kernel
            .fail_operation(operation, error)
            .await
            .map_err(Into::into)
    }
}
