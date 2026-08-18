//! Durable Goal attempt data contract.

use ::contracts::{
    AttemptEvidence, AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, GoalId, RuntimeFailure,
    RuntimeId, RuntimeResult,
};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// Runtime-facing port for exactly one durable Goal attempt.
#[async_trait]
pub trait GoalAttemptPort: Send + Sync {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool;

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure>;
}

/// Fully materialized durable runtime attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalAttempt {
    pub id: AttemptId,
    pub goal_id: GoalId,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub role: CognitiveRole,
    pub status: AttemptStatus,
    pub input: serde_json::Value,
    pub output: Option<RuntimeResult>,
    pub failure: Option<RuntimeFailure>,
    pub evidence: Vec<AttemptEvidence>,
    pub usage: AttemptUsage,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// Typed retry/replan evidence derived only from a settled evaluation receipt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalEvaluationFeedback {
    pub receipt_id: contracts::EvaluationReceiptId,
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub decision: contracts::EvaluationDecision,
    pub failed_gates: Vec<String>,
    pub retry_replan_required: bool,
}
