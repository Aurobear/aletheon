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

/// The single, authoritative turn execution entry point. Every production
/// turn — daemon JSON-RPC, CLI exec, native child agent — flows through one
/// implementation of this trait.
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

/// Normalised input payload. All turn variations are reduced to this before
/// entering the engine.
#[derive(Clone, Debug)]
pub struct TurnEngineRequest {
    pub input: String,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Immutable turn-lifetime context. Everything the engine needs to authorise,
/// track, cap, and settle a turn is held here.
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
// Event sink (caller-supplied observer)
// ---------------------------------------------------------------------------

/// Lightweight event observer. Callers (daemon / CLI / child adapter) inject
/// their own streaming or logging behaviour.
#[async_trait]
pub trait TurnEngineEventSink: Send + Sync {
    async fn on_turn_started(&self, turn_id: TurnId);
    async fn on_turn_settled(&self, turn_id: TurnId, outcome: &TurnEngineResult);
}

// ---------------------------------------------------------------------------
// Result
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

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

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
// Parity types (shared between production and test harness)
// ---------------------------------------------------------------------------

/// A snapshot of the observable outputs that the parity harness compares
/// when validating that daemon + TurnEngine paths are semantically identical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEngineParitySnapshot {
    pub turn_id: TurnId,
    pub output_len: usize,
    pub tool_calls: usize,
    pub status: TurnEngineStatus,
    pub tokens_in: u64,
    pub tokens_out: u64,
}
