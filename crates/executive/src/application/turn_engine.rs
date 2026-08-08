//! Unified TurnEngine contract — the single production execution path for
//! daemon, CLI, and native child-agent turns (Wave 1).

use crate::application::turn_runtime_ports::ResolvedTurnProfile;
use async_trait::async_trait;
use fabric::{MonoDeadlineMillis, OperationId, PrincipalId, ProcessId, TurnId};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// TurnEngine trait
// ---------------------------------------------------------------------------

/// The single, authoritative turn execution entry point.
#[async_trait]
pub trait TurnEngine: Send + Sync {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
    ) -> Result<TurnEngineResult, TurnEngineError>;
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TurnEngineRequest {
    pub input: String,
    pub execution_target: fabric::ExecutionTargetSelection,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
    pub requirements: Vec<fabric::TurnRequirement>,
    pub requested_task_kind: Option<fabric::TaskKind>,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TurnEngineContext {
    pub principal_id: PrincipalId,
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub workspace: Arc<fabric::WorkspacePolicy>,
    pub profile: ResolvedTurnProfile,
    pub cancel_token: CancellationToken,
    /// Notification stream owned by the connection that admitted this Turn.
    pub notification_sender: Option<tokio::sync::mpsc::Sender<String>>,
    /// Exact host-authenticated context resolved at the trusted transport edge.
    /// Execution must fail closed when this is absent.
    pub principal_context: Option<fabric::PrincipalContext>,
}

impl TurnEngineContext {
    /// Return the host-authenticated authority context or reject execution
    /// before any capability can be invoked. There is deliberately no local,
    /// root, permission-profile, or approval-policy fallback.
    pub fn require_principal_context(&self) -> Result<fabric::PrincipalContext, TurnEngineError> {
        let context = self.principal_context.clone().ok_or_else(|| {
            TurnEngineError::InvalidContext("authenticated principal context is missing".into())
        })?;
        if context.principal_id != self.principal_id {
            return Err(TurnEngineError::InvalidContext(
                "authenticated principal does not match engine principal".into(),
            ));
        }
        Ok(context)
    }
}

// ---------------------------------------------------------------------------
// Result / Status / Error
// ---------------------------------------------------------------------------

pub struct TurnEngineResult {
    pub turn_id: TurnId,
    pub output: String,
    /// The existing domain stop value is the one authoritative outcome. Do not
    /// introduce an adapter-local status that can drift from `TurnResult.stop`.
    pub stop: fabric::TurnStop,
    pub failure: Option<fabric::TurnFailure>,
    pub tool_calls: usize,
    /// Provider-reported usage aggregated from durable inference receipts.
    /// Missing provider telemetry remains `None`, never a fabricated zero.
    pub usage: fabric::InferenceUsage,
    pub elapsed_ms: u64,
    /// Coordinator-owned durable artifacts produced by daemon execution.
    /// Non-daemon engines leave this empty.
    pub coordinator_execution: Option<crate::application::turn_coordinator::TurnExecution>,
}

#[derive(Debug, thiserror::Error)]
pub enum TurnEngineError {
    #[error("turn engine not available: {0}")]
    Unavailable(String),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("operation rejected: {0}")]
    AdmissionRejected(String),
    #[error("context missing required field: {0}")]
    InvalidContext(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl TurnEngineError {
    /// Stable machine-readable error identity used at the command boundary.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "execution_target_unavailable",
            Self::ProfileNotFound(_) => "turn_profile_not_found",
            Self::AdmissionRejected(_) => "turn_admission_rejected",
            Self::InvalidContext(_) => "turn_context_invalid",
            Self::Internal(_) => "turn_runtime_failed",
        }
    }

    /// Whether the same typed request can be retried without correction.
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

// ---------------------------------------------------------------------------
// Parity types (shared between production and test harness)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEngineParitySnapshot {
    pub turn_id: TurnId,
    pub output_len: usize,
    pub tool_calls: usize,
    pub stop: fabric::TurnStop,
    pub failure: Option<fabric::TurnFailure>,
    pub usage: fabric::InferenceUsage,
}
