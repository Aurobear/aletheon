//! Kernel admission and Hardware authority projection for embodiment skills.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use fabric::types::admission::{RiskLevel, UsageReport};
use fabric::{
    AdmissionController, AdmissionRequest, CapabilityId, CapabilityScope, LeaseRequest,
    OperationId, PrincipalId, ProcessId, SandboxRequirement, SkillDispatchError, SkillRequest,
};
use hardware::{AuthorizedSkillRequest, ControlLease, ControlPermit, MonotonicClock};

#[async_trait]
pub trait EmbodimentAuthorityPort: Send + Sync {
    async fn authorize(
        &self,
        operation_id: OperationId,
        request: &SkillRequest,
    ) -> Result<AuthorizedSkillRequest, SkillDispatchError>;

    async fn settle(
        &self,
        operation_id: OperationId,
        succeeded: bool,
    ) -> Result<(), SkillDispatchError>;
}

pub struct KernelEmbodimentAuthority {
    admission: Arc<dyn AdmissionController>,
    clock: Arc<dyn MonotonicClock>,
    process_id: ProcessId,
    principal: PrincipalId,
    permits: tokio::sync::Mutex<std::collections::HashMap<OperationId, fabric::PermitId>>,
}

impl KernelEmbodimentAuthority {
    pub fn new(
        admission: Arc<dyn AdmissionController>,
        clock: Arc<dyn MonotonicClock>,
        process_id: ProcessId,
        principal: PrincipalId,
    ) -> Self {
        Self {
            admission,
            clock,
            process_id,
            principal,
            permits: tokio::sync::Mutex::new(Default::default()),
        }
    }
}

#[async_trait]
impl EmbodimentAuthorityPort for KernelEmbodimentAuthority {
    async fn authorize(
        &self,
        operation_id: OperationId,
        request: &SkillRequest,
    ) -> Result<AuthorizedSkillRequest, SkillDispatchError> {
        let duration_ms = 30_000;
        let admission_request = AdmissionRequest {
            operation_id,
            process_id: self.process_id,
            principal: self.principal.clone(),
            capability: CapabilityId("hardware.command".into()),
            action: request.skill.0.clone(),
            input_summary: format!("embodiment skill for {}", request.device.0),
            risk: RiskLevel::SystemModify,
            requested_scope: CapabilityScope {
                allowed_paths: vec![request.skill.0.clone()],
                allowed_targets: vec![format!("device:{}", request.device.0)],
                max_runtime_ms: Some(duration_ms),
                max_output_bytes: Some(16 * 1024),
            },
            budget: None,
            lease: Some(LeaseRequest {
                resource: format!("hardware:{}", request.device.0),
                duration_ms,
            }),
            sandbox: SandboxRequirement::NotRequired,
        };
        let permit = self
            .admission
            .admit(admission_request)
            .await
            .map_err(|error| SkillDispatchError::Rejected(error.to_string()))?;
        let lease_id = permit.lease.ok_or_else(|| {
            SkillDispatchError::Rejected("admission omitted control lease".into())
        })?;
        let hardware_operation = hardware::OperationId(operation_id.0.to_string());
        let hardware_principal = hardware::PrincipalId(self.principal.0.clone());
        let scope = BTreeSet::from([request.skill.0.clone()]);
        let expires_at = hardware::MonotonicInstant(self.clock.now().0.saturating_add(duration_ms));
        self.permits.lock().await.insert(operation_id, permit.id);
        Ok(AuthorizedSkillRequest {
            request: request.clone(),
            permit: ControlPermit {
                permit_id: permit.id.0.to_string(),
                operation: hardware_operation.clone(),
                principal: hardware_principal.clone(),
                device: request.device.clone(),
                scope: scope.clone(),
                expires_at,
                revoked: false,
            },
            lease: ControlLease {
                lease_id: lease_id.0.to_string(),
                operation: hardware_operation,
                device: request.device.clone(),
                holder: hardware_principal,
                scope,
                expires_at,
                exclusive: true,
            },
        })
    }

    async fn settle(
        &self,
        operation_id: OperationId,
        _succeeded: bool,
    ) -> Result<(), SkillDispatchError> {
        let permit = self
            .permits
            .lock()
            .await
            .remove(&operation_id)
            .ok_or_else(|| SkillDispatchError::Rejected("operation already settled".into()))?;
        self.admission
            .settle(permit, UsageReport::default())
            .await
            .map_err(|error| SkillDispatchError::Rejected(error.to_string()))
    }
}
