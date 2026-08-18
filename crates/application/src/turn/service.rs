use async_trait::async_trait;
use contracts::{NoopTurnEventSink, OperationId, TurnEventSink, TurnId};

use super::command::{TurnCommand, TurnContext};
use super::outcome::TurnServiceResult;

#[async_trait]
pub trait TurnNotificationPort: std::fmt::Debug + Send + Sync {
    async fn send(&self, payload: String) -> Result<(), ()>;
}

#[async_trait]
pub trait TurnService: Send + Sync {
    async fn execute(
        &self,
        request: TurnCommand,
        context: TurnContext,
    ) -> Result<TurnServiceResult, TurnServiceError>;

    async fn execute_with_events(
        &self,
        request: TurnCommand,
        context: TurnContext,
        _events: &dyn TurnEventStream,
    ) -> Result<TurnServiceResult, TurnServiceError> {
        self.execute(request, context).await
    }
}

pub trait RuntimeIdentityBinder: Send + Sync {
    fn bind_runtime_identity(&self, operation_id: OperationId, turn_id: &TurnId);
}

pub trait TurnEventStream: TurnEventSink + RuntimeIdentityBinder {}

impl<T> TurnEventStream for T where T: TurnEventSink + RuntimeIdentityBinder {}

impl RuntimeIdentityBinder for NoopTurnEventSink {
    fn bind_runtime_identity(&self, _operation_id: OperationId, _turn_id: &TurnId) {}
}

#[derive(Debug, thiserror::Error)]
pub enum TurnServiceError {
    #[error("turn service not available: {0}")]
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

impl TurnServiceError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "execution_target_unavailable",
            Self::ProfileNotFound(_) => "turn_profile_not_found",
            Self::AdmissionRejected(_) => "turn_admission_rejected",
            Self::InvalidContext(_) => "turn_context_invalid",
            Self::Internal(_) => "turn_runtime_failed",
        }
    }

    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}
