//! Asynchronous provider contract for long-running, cancellable skills.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{DeviceId, SkillDescriptor, SkillProgress, SkillRequest, SkillResult};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("provider disconnected")]
    Disconnected,
    #[error("provider rejected request: {0}")]
    Rejected(String),
    #[error("provider timed out")]
    Timeout,
}

#[async_trait]
pub trait SkillProgressSink: Send + Sync {
    async fn progress(&self, update: SkillProgress);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelAck {
    pub device: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopReceipt {
    pub device: DeviceId,
}

/// Authority projected by Executive after Kernel admission.
pub struct AuthorizedSkillRequest {
    pub request: SkillRequest,
    pub permit: crate::ControlPermit,
    pub lease: crate::ControlLease,
}

pub struct ValidatedSkillCommand<'a>(pub(crate) &'a AuthorizedSkillRequest);

impl<'a> ValidatedSkillCommand<'a> {
    pub fn request(&self) -> &SkillRequest {
        &self.0.request
    }

    pub fn permit(&self) -> &crate::ControlPermit {
        &self.0.permit
    }

    pub fn lease(&self) -> &crate::ControlLease {
        &self.0.lease
    }
}

#[async_trait]
pub trait EmbodimentProvider: Send + Sync {
    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError>;
    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError>;
    async fn cancel(
        &self,
        device: &DeviceId,
        operation: &crate::OperationId,
    ) -> Result<CancelAck, ProviderError>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError>;
}

#[cfg(test)]
pub(crate) fn authorized_fixture(request: SkillRequest) -> AuthorizedSkillRequest {
    let device = request.device.clone();
    let operation = crate::OperationId(fabric::OperationId::new().0.to_string());
    let principal = crate::PrincipalId("test-principal".into());
    let scope = std::collections::BTreeSet::from([request.skill.0.clone()]);
    AuthorizedSkillRequest {
        request,
        permit: crate::ControlPermit {
            permit_id: "test-permit".into(),
            operation: operation.clone(),
            principal: principal.clone(),
            device: device.clone(),
            scope: scope.clone(),
            expires_at: crate::MonotonicInstant(u64::MAX),
            revoked: false,
        },
        lease: crate::ControlLease {
            lease_id: "test-lease".into(),
            operation,
            device,
            holder: principal,
            scope,
            expires_at: crate::MonotonicInstant(u64::MAX),
            exclusive: true,
        },
    }
}
