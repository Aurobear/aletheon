//! Unified TurnEngine contract — the single production execution path for
//! daemon, CLI, and native child-agent turns (Wave 1).

use crate::service::turn_runtime_ports::ResolvedTurnProfile;
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
        events: Arc<dyn TurnEngineEventSink>,
    ) -> Result<TurnEngineResult, TurnEngineError>;
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TurnEngineRequest {
    pub input: String,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
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
}

// ---------------------------------------------------------------------------
// Event sink
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TurnEngineEventSink: Send + Sync {
    async fn on_turn_started(&self, turn_id: TurnId);
    async fn on_turn_settled(&self, turn_id: TurnId, outcome: &TurnEngineResult);
}

// ---------------------------------------------------------------------------
// Result / Status / Error
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TurnEngineResult {
    pub turn_id: TurnId,
    pub output: String,
    pub status: TurnEngineStatus,
    pub tool_calls: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnEngineStatus {
    Completed,
    Cancelled,
    BudgetExhausted,
    DeadlineExceeded,
    Blocked,
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

// ---------------------------------------------------------------------------
// SessionTurnEngine — wraps TurnService for CLI exec paths
// ---------------------------------------------------------------------------

/// Adapter: wraps the existing `TurnService` behind the unified `TurnEngine`
/// contract.  `TurnService` remains an internal detail and will be inlined
/// when all three entry points have been migrated (W1-05).
pub struct SessionTurnEngine {
    inner: crate::service::turn_service::TurnService,
}

impl SessionTurnEngine {
    pub fn new(inner: crate::service::turn_service::TurnService) -> Self {
        Self { inner }
    }
}

struct FabricEventSink;

#[async_trait]
impl fabric::TurnEventSink for FabricEventSink {
    async fn emit(&self, _event: fabric::TurnEvent) {}
}

#[async_trait]
impl TurnEngine for SessionTurnEngine {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
        events: Arc<dyn TurnEngineEventSink>,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        let turn_id = fabric::TurnId::new();
        events.on_turn_started(turn_id).await;

        let principal_id = context.principal_id.clone();
        let workspace = (*context.workspace).clone();
        let model_policy = request
            .model_policy
            .or(context.profile.model_policy.clone());

        let turn_request = fabric::TurnRequest {
            operation_id: context.operation_id,
            process_id: context.process_id,
            context: fabric::PrincipalContext::new(
                principal_id.clone(),
                fabric::LocalOsPrincipal { uid: 0, gid: 0 },
                fabric::ConnectionId::default(),
                fabric::ThreadId(principal_id.0),
                workspace,
                fabric::PermissionProfileId("exec".into()),
                fabric::ApprovalPolicy::Never,
            ),
            input: request.input.clone(),
            model_policy,
            deadline: request.deadline,
        };

        let sink = FabricEventSink;
        let result = match self.inner.submit(turn_request, &sink).await {
            Ok(tr) => TurnEngineResult {
                turn_id,
                output: tr.output,
                status: TurnEngineStatus::Completed,
                tool_calls: tr.metrics.tool_calls_made,
                tokens_in: 0,
                tokens_out: 0,
                elapsed_ms: tr.metrics.elapsed_ms,
            },
            Err(_e) => TurnEngineResult {
                turn_id,
                output: String::new(),
                status: TurnEngineStatus::Blocked,
                tool_calls: 0,
                tokens_in: 0,
                tokens_out: 0,
                elapsed_ms: 0,
            },
        };

        events.on_turn_settled(turn_id, &result).await;
        Ok(result)
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
    pub status: TurnEngineStatus,
    pub tokens_in: u64,
    pub tokens_out: u64,
}
