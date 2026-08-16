//! Runtime-owned process registration port.

use ::contracts::{AgentControlError, RuntimeProcessId};
use async_trait::async_trait;

/// Durable binding between a Runtime Agent process and its kernel identity.
#[async_trait]
pub trait RuntimeProcessRegistrationPort: Send + Sync + std::fmt::Debug {
    async fn register(
        &self,
        os_pid: ::contracts::OsProcessId,
        start_time_ticks: u64,
    ) -> Result<RuntimeProcessId, AgentControlError>;

    async fn clear(&self, identity: RuntimeProcessId) -> Result<(), AgentControlError>;
}

#[derive(Debug, Default)]
pub struct NoopRuntimeProcessRegistration;

#[async_trait]
impl RuntimeProcessRegistrationPort for NoopRuntimeProcessRegistration {
    async fn register(
        &self,
        _os_pid: ::contracts::OsProcessId,
        _start_time_ticks: u64,
    ) -> Result<RuntimeProcessId, AgentControlError> {
        Err(AgentControlError {
            kind: ::contracts::AgentControlErrorKind::Runtime,
            message: "runtime process registration is unavailable at this composition boundary"
                .into(),
        })
    }

    async fn clear(&self, _identity: RuntimeProcessId) -> Result<(), AgentControlError> {
        Ok(())
    }
}
