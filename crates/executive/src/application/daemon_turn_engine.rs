//! Daemon adapter for the unified [`TurnEngine`] boundary.

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::post_turn_projection::{PostTurnDispatch, PostTurnOutcome};
use crate::application::turn_coordinator::TurnExecution;
use crate::application::turn_engine::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineEventSink, TurnEngineRequest,
    TurnEngineResult, TurnEngineStatus,
};
use crate::application::TurnPipeline;

/// Pure compatibility mapping used by parity tests and protocol adapters.
pub fn map_pipeline_response(
    turn_id: fabric::TurnId,
    response: &serde_json::Value,
) -> TurnEngineResult {
    let result = &response["result"];
    let succeeded =
        response.get("error").is_none() && result["succeeded"].as_bool().unwrap_or(false);
    let metrics = &result["metrics"];
    TurnEngineResult {
        turn_id,
        output: result["response"].as_str().unwrap_or_default().to_owned(),
        status: if succeeded {
            TurnEngineStatus::Completed
        } else {
            TurnEngineStatus::Blocked
        },
        tool_calls: metrics["tool_calls_made"].as_u64().unwrap_or(0) as usize,
        tokens_in: 0,
        tokens_out: 0,
        elapsed_ms: metrics["elapsed_ms"].as_u64().unwrap_or(0),
        coordinator_execution: None,
    }
}

pub struct DaemonTurnEngine {
    pipeline: Arc<TurnPipeline>,
}

impl DaemonTurnEngine {
    pub fn new(pipeline: Arc<TurnPipeline>) -> Self {
        Self { pipeline }
    }
}

/// Daemon lifecycle events are already persisted by
/// [`TurnCoordinator`](crate::application::turn_coordinator::TurnCoordinator).
pub struct NoopTurnEngineEventSink;

#[async_trait]
impl TurnEngineEventSink for NoopTurnEngineEventSink {
    async fn on_turn_started(&self, _turn_id: fabric::TurnId) {}

    async fn on_turn_settled(&self, _turn_id: fabric::TurnId, _outcome: &TurnEngineResult) {}
}

#[async_trait]
impl TurnEngine for DaemonTurnEngine {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
        events: Arc<dyn TurnEngineEventSink>,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        let turn_id = fabric::TurnId::new();
        events.on_turn_started(turn_id).await;

        let principal_context = context.principal_context.ok_or_else(|| {
            TurnEngineError::InvalidContext("daemon principal context is missing".into())
        })?;
        let turn_request = fabric::TurnRequest {
            operation_id: context.operation_id,
            process_id: context.process_id,
            context: principal_context,
            input: request.input,
            model_policy: request
                .model_policy
                .or(context.profile.model_policy.clone()),
            deadline: request.deadline,
        };

        {
            let mut guard = self.pipeline.current_scope.lock().await;
            *guard = Some(kernel::operation::OperationScope::new(context.operation_id));
        }
        let turn_cancel = context.cancel_token.clone();
        let principal = turn_request.context.principal_id.clone();
        let response = self
            .pipeline
            .run(
                serde_json::Value::Null,
                turn_request.input.clone(),
                turn_request.clone(),
                context.operation_id,
                context.process_id,
                context.cancel_token,
                principal,
            )
            .await?;
        if let Some(error) = response.get("error") {
            return Err(TurnEngineError::Internal(anyhow::anyhow!(
                "{}",
                error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("daemon turn failed")
            )));
        }

        let raw = &response["result"];
        let output = raw["response"].as_str().unwrap_or_default().to_owned();
        let items = serde_json::from_value(raw["canonical_items"].clone()).unwrap_or_default();
        let succeeded = raw["succeeded"].as_bool().unwrap_or(false);
        let metric = &raw["metrics"];
        let metrics = fabric::TurnMetrics {
            tool_calls_made: metric["tool_calls_made"].as_u64().unwrap_or(0) as usize,
            tool_errors: metric["tool_errors"].as_u64().unwrap_or(0) as usize,
            elapsed_ms: metric["elapsed_ms"].as_u64().unwrap_or(0),
            iterations: metric["iterations"].as_u64().unwrap_or(0) as usize,
            completed_normally: metric["completed_normally"].as_bool().unwrap_or(false),
        };
        let projection = PostTurnDispatch {
            projector: self.pipeline.post_turn_projection.clone(),
            outcome: PostTurnOutcome {
                session_id: raw["projection"]["session_id"]
                    .as_str()
                    .unwrap_or(&turn_request.context.thread_id.0)
                    .to_owned(),
                input: turn_request.input.clone(),
                output: output.clone(),
                turn: raw["turn"].as_u64().unwrap_or(0) as usize,
                succeeded,
                tool_calls_made: metrics.tool_calls_made,
                tool_errors: metrics.tool_errors,
                elapsed_ms: metrics.elapsed_ms,
                iterations: metrics.iterations,
                completed_normally: metrics.completed_normally,
                agora_start_version: raw["projection"]["agora_start_version"]
                    .as_u64()
                    .unwrap_or(0),
            },
        };
        let context_projection =
            serde_json::from_value(raw["projection"]["conscious_context"].clone()).ok();
        let status = if turn_cancel.is_cancelled() {
            TurnEngineStatus::Cancelled
        } else if succeeded {
            TurnEngineStatus::Completed
        } else {
            TurnEngineStatus::Blocked
        };
        let coordinator_execution = TurnExecution {
            result: fabric::TurnResult {
                output: output.clone(),
                stop: match status {
                    TurnEngineStatus::Cancelled => fabric::TurnStop::Cancelled,
                    TurnEngineStatus::Completed => fabric::TurnStop::Completed,
                    _ => fabric::TurnStop::Failed,
                },
                metrics: metrics.clone(),
            },
            items,
            projection: Some(projection),
            context_projection,
        };
        let result = TurnEngineResult {
            turn_id,
            output,
            status,
            tool_calls: metrics.tool_calls_made,
            tokens_in: 0,
            tokens_out: 0,
            elapsed_ms: metrics.elapsed_ms,
            coordinator_execution: Some(coordinator_execution),
        };
        events.on_turn_settled(turn_id, &result).await;
        Ok(result)
    }
}
