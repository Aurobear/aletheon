//! Canonical Kernel capability execution adapter for embodiment commands.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use fabric::{
    CapabilityInvoker, CapabilityRequest, CapabilityResult, ExecutionPermit, OperationId,
    UsageReport,
};
use hardware::{
    AuthorizedSkillRequest, Broker, ControlLease, ControlPermit, MonotonicInstant,
    SkillProgressSink,
};
use kernel::capability::ToolExecutor;
use tokio_util::sync::CancellationToken;

use super::embodiment_progress::{BoundedProgressSink, EmbodimentProgressPort};

#[derive(Clone)]
pub struct ActiveEmbodimentOperation {
    pub device: fabric::types::embodiment::DeviceId,
    pub hardware_operation: hardware::OperationId,
    pub cancel: CancellationToken,
}

#[derive(Default)]
pub struct ActiveEmbodimentOperations {
    inner: tokio::sync::Mutex<HashMap<OperationId, ActiveEmbodimentOperation>>,
}

impl ActiveEmbodimentOperations {
    pub async fn insert(&self, id: OperationId, operation: ActiveEmbodimentOperation) {
        self.inner.lock().await.insert(id, operation);
    }

    pub async fn get(&self, id: &OperationId) -> Option<ActiveEmbodimentOperation> {
        self.inner.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &OperationId) {
        self.inner.lock().await.remove(id);
    }
}

struct EmbodimentCapabilityExecutor {
    broker: Arc<Broker>,
    progress: Arc<dyn EmbodimentProgressPort>,
    active: Arc<ActiveEmbodimentOperations>,
}

#[async_trait]
impl ToolExecutor for EmbodimentCapabilityExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        let call_id = request.call.call_id.clone();
        let skill_request = match serde_json::from_value::<fabric::types::embodiment::SkillRequest>(
            request.call.input.clone(),
        ) {
            Ok(request) => request,
            Err(error) => {
                return capability_error(call_id, format!("invalid skill request: {error}"))
            }
        };
        let Some(lease_id) = permit.lease else {
            return capability_error(call_id, "admission omitted hardware lease".into());
        };
        let operation = hardware::OperationId(request.call.operation_id.0.to_string());
        let principal = hardware::PrincipalId(request.authority.principal.0.clone());
        let scope: std::collections::BTreeSet<String> =
            permit.granted_scope.allowed_paths.iter().cloned().collect();
        let expires_at = MonotonicInstant(permit.expires_at.0 .0);
        let authorized = AuthorizedSkillRequest {
            request: skill_request.clone(),
            permit: ControlPermit {
                permit_id: permit.id.0.to_string(),
                operation: operation.clone(),
                principal: principal.clone(),
                device: skill_request.device.clone(),
                scope: scope.clone(),
                expires_at,
                revoked: false,
            },
            lease: ControlLease {
                lease_id: lease_id.0.to_string(),
                operation: operation.clone(),
                device: skill_request.device.clone(),
                holder: principal,
                scope,
                expires_at,
                exclusive: true,
            },
        };
        self.active
            .insert(
                request.call.operation_id,
                ActiveEmbodimentOperation {
                    device: skill_request.device,
                    hardware_operation: operation,
                    cancel: request.control.cancel.clone(),
                },
            )
            .await;
        let sink: Arc<dyn SkillProgressSink> = Arc::new(BoundedProgressSink::new(
            request.call.operation_id,
            self.progress.clone(),
            64,
        ));
        let result = self.broker.execute(authorized, sink).await;
        self.active.remove(&request.call.operation_id).await;
        match result {
            Ok(result) => match serde_json::to_string(&result) {
                Ok(output) => {
                    let usage = UsageReport {
                        output_bytes: output.len() as u64,
                        ..Default::default()
                    };
                    CapabilityResult {
                        call_id,
                        output,
                        is_error: false,
                        usage,
                        audit_id: None,
                        patch_delta: None,
                    }
                }
                Err(error) => capability_error(call_id, error.to_string()),
            },
            Err(error) => capability_error(call_id, error.to_string()),
        }
    }
}

fn capability_error(call_id: String, output: String) -> CapabilityResult {
    CapabilityResult {
        call_id,
        output,
        is_error: true,
        usage: UsageReport::default(),
        audit_id: None,
        patch_delta: None,
    }
}

pub fn build_embodiment_invoker(
    admission: Arc<dyn fabric::AdmissionController>,
    broker: Arc<Broker>,
    progress: Arc<dyn EmbodimentProgressPort>,
) -> (Arc<dyn CapabilityInvoker>, Arc<ActiveEmbodimentOperations>) {
    let active = Arc::new(ActiveEmbodimentOperations::default());
    let executor = Arc::new(EmbodimentCapabilityExecutor {
        broker,
        progress,
        active: active.clone(),
    });
    (
        crate::application::governed_capability::canonical_capability_invoker(admission, executor),
        active,
    )
}
