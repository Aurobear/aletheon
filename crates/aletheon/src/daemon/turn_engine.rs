//! Daemon adapter for the unified [`TurnEngine`] boundary.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::host::turn_pipeline::TurnPipeline;
use application::turn::coordinator::TurnExecution;
use application::turn::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineRequest, TurnEngineResult,
};

#[derive(Debug)]
pub struct MpscTurnNotificationPort(pub tokio::sync::mpsc::Sender<String>);

#[async_trait]
impl application::turn::service::TurnNotificationPort for MpscTurnNotificationPort {
    async fn send(&self, payload: String) -> Result<(), ()> {
        self.0.send(payload).await.map_err(|_| ())
    }
}

#[derive(Debug)]
pub struct SharedMpscTurnNotificationPort(
    pub Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>>>,
);

#[async_trait]
impl application::turn::service::TurnNotificationPort for SharedMpscTurnNotificationPort {
    async fn send(&self, payload: String) -> Result<(), ()> {
        let sender = self.0.lock().await.clone().ok_or(())?;
        sender.send(payload).await.map_err(|_| ())
    }
}

/// Map the typed coordinator execution into the authoritative engine outcome.
/// This is intentionally exhaustive over the existing `TurnStop` domain value:
/// an adapter-local status must never reinterpret `Ok(TurnResult)` as success.
pub fn map_turn_execution(
    turn_id: ::contracts::TurnId,
    execution: TurnExecution,
) -> TurnEngineResult {
    TurnEngineResult {
        turn_id,
        output: execution.result.output.clone(),
        stop: execution.result.stop.clone(),
        failure: execution.result.failure.clone(),
        tool_calls: execution.result.metrics.tool_calls_made,
        usage: execution.result.usage.clone(),
        elapsed_ms: execution.result.metrics.elapsed_ms,
        coordinator_execution: Some(execution),
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

#[async_trait]
impl TurnEngine for DaemonTurnEngine {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        self.pipeline
            .cognitive_sessions
            .validate_target(&request.execution_target)
            .map_err(|error| TurnEngineError::Unavailable(error.to_string()))?;
        // Fail closed before starting execution or exposing any capability.
        // The coordinator persists this typed refusal on its error settlement
        // path, so production denial remains auditable.
        let principal_context = context.require_principal_context()?;
        let turn_id = principal_context.turn_id.ok_or_else(|| {
            TurnEngineError::InvalidContext("authoritative turn id is missing".into())
        })?;
        let turn_request = ::contracts::TurnRequest {
            operation_id: context.operation_id,
            process_id: context.process_id,
            context: principal_context,
            input: request.input,
            execution_target: request.execution_target,
            model_policy: request
                .model_policy
                .or(context.profile.model_policy.clone()),
            deadline: request.deadline,
            requirements: request.requirements,
            requested_task_kind: request.requested_task_kind,
            evaluation_contract: None,
        };

        let mut scope = kernel::operation::OperationScope::with_cancellation(
            context.operation_id,
            context.cancel_token,
        );
        let principal = turn_request.context.principal_id.clone();
        let coordinator_execution = match self
            .pipeline
            .run(
                turn_request.input.clone(),
                turn_request.clone(),
                context.process_id,
                &mut scope,
                principal,
                context.notification,
            )
            .await
        {
            Ok(crate::host::turn_pipeline::TurnPipelineOutcome::Completed(execution)) => *execution,
            Ok(crate::host::turn_pipeline::TurnPipelineOutcome::Rejected(rejection)) => {
                let cleanup = scope
                    .abort_and_drain(self.pipeline.clock.as_ref(), Duration::from_secs(5))
                    .await;
                log_scope_cleanup(&cleanup, false);
                return Err(TurnEngineError::Internal(anyhow::anyhow!(
                    "{}",
                    rejection.message()
                )));
            }
            Err(error) => {
                let cleanup = scope
                    .abort_and_drain(self.pipeline.clock.as_ref(), Duration::from_secs(5))
                    .await;
                log_scope_cleanup(&cleanup, false);
                return Err(error.into());
            }
        };
        let result = map_turn_execution(turn_id, coordinator_execution);
        let cleanup = if matches!(
            result.stop,
            ::contracts::TurnStop::Cancelled | ::contracts::TurnStop::Failed
        ) {
            scope
                .abort_and_drain(self.pipeline.clock.as_ref(), Duration::from_secs(5))
                .await
        } else {
            scope
                .settle_and_drain(self.pipeline.clock.as_ref(), Duration::from_secs(5))
                .await
        };
        log_scope_cleanup(&cleanup, result.stop == ::contracts::TurnStop::Cancelled);
        Ok(result)
    }
}

fn log_scope_cleanup(
    report: &kernel::operation::OperationScopeCleanupReport,
    expected_cancel: bool,
) {
    let failed = report.exits.iter().any(|exit| {
        matches!(
            exit.reason,
            ::contracts::OperationExitReason::Failed(_)
                | ::contracts::OperationExitReason::Panic(_)
        )
    });
    if failed || (report.forced_abort && !expected_cancel) {
        tracing::warn!(
            operation = %report.operation_id.0,
            kind = ?report.kind,
            forced_abort = report.forced_abort,
            exits = ?report.exits,
            "turn operation scope required abnormal cleanup"
        );
    } else if expected_cancel {
        tracing::debug!(
            operation = %report.operation_id.0,
            kind = ?report.kind,
            forced_abort = report.forced_abort,
            resources = report.exits.len(),
            "cancelled turn operation scope drained"
        );
    } else {
        tracing::debug!(
            operation = %report.operation_id.0,
            kind = ?report.kind,
            resources = report.exits.len(),
            "turn operation scope drained"
        );
    }
}
