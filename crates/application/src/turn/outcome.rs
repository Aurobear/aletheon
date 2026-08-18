use contracts::{
    ContextProjectionReceipt, InferenceUsage, ItemPayload, TurnFailure, TurnId, TurnResult,
    TurnStop,
};
use std::sync::Arc;

/// Provider-neutral rejection produced before cognitive execution starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnPipelineRejection {
    IntentDeniedBySelfField { reason: String },
    SelfFieldReviewFailed { reason: String },
    HookBlocked { reason: String },
}

impl TurnPipelineRejection {
    pub fn message(&self) -> String {
        match self {
            Self::IntentDeniedBySelfField { reason } => {
                format!("Intent denied by SelfField: {reason}")
            }
            Self::SelfFieldReviewFailed { reason } => {
                format!("SelfField review failed (fail-closed): {reason}")
            }
            Self::HookBlocked { reason } => format!("Blocked by hook: {reason}"),
        }
    }
}

pub struct TurnExecution {
    pub result: TurnResult,
    pub items: Vec<ItemPayload>,
    pub projection: Option<super::post_turn::PostTurnDispatch>,
    pub context_projection: Option<ContextProjectionReceipt>,
    pub evaluation_artifacts: crate::evaluation::TurnEvaluationArtifacts,
}

pub struct CompletedTurnProjection {
    pub projector: Arc<dyn super::post_turn::PostTurnProjection>,
    pub session_id: String,
    pub principal_id: contracts::PrincipalId,
    pub input: String,
    pub turn: usize,
    pub agora_start_version: u64,
}

impl TurnExecution {
    pub fn completed(
        result: TurnResult,
        items: Vec<ItemPayload>,
        projection: CompletedTurnProjection,
        context_projection: Option<ContextProjectionReceipt>,
        evaluation_artifacts: crate::evaluation::TurnEvaluationArtifacts,
    ) -> Self {
        let metrics = &result.metrics;
        let outcome = super::post_turn::PostTurnOutcome {
            session_id: projection.session_id,
            principal_id: projection.principal_id,
            input: projection.input,
            output: result.output.clone(),
            turn: projection.turn,
            succeeded: result.stop == TurnStop::Completed && metrics.completed_normally,
            tool_calls_made: metrics.tool_calls_made,
            tool_errors: metrics.tool_errors,
            elapsed_ms: metrics.elapsed_ms,
            iterations: metrics.iterations,
            completed_normally: metrics.completed_normally,
            agora_start_version: projection.agora_start_version,
        };
        Self {
            result,
            items,
            projection: Some(super::post_turn::PostTurnDispatch {
                projector: projection.projector,
                outcome,
            }),
            context_projection,
            evaluation_artifacts,
        }
    }
}

pub struct TurnServiceResult {
    pub turn_id: TurnId,
    pub output: String,
    pub stop: TurnStop,
    pub failure: Option<TurnFailure>,
    pub tool_calls: usize,
    pub usage: InferenceUsage,
    pub elapsed_ms: u64,
    pub coordinator_execution: Option<TurnExecution>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnServiceParitySnapshot {
    pub turn_id: TurnId,
    pub output_len: usize,
    pub tool_calls: usize,
    pub stop: TurnStop,
    pub failure: Option<TurnFailure>,
    pub usage: InferenceUsage,
}
