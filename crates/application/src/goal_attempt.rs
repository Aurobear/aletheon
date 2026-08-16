//! Durable Goal attempt data contract.

use ::contracts::{
    AttemptEvidence, AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, GoalId, RuntimeFailure,
    RuntimeId, RuntimeResult,
};

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
