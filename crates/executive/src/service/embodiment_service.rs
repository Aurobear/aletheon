//! Executive-owned embodiment capability request shaping and query delegation.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::types::embodiment::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, SkillDescriptor, SkillDispatchError,
    SkillRequest, SkillResult,
};
use fabric::{
    CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityRequest, CapabilityScope,
    InvocationControl, LeaseRequest, OperationId, PrincipalId, ProcessId, SandboxRequirement,
};
use hardware::{Broker, BrokerError};
use tokio_util::sync::CancellationToken;

use super::embodiment_authority::ActiveEmbodimentOperations;

pub struct EmbodimentService {
    broker: Arc<Broker>,
    invoker: Arc<dyn CapabilityInvoker>,
    active: Arc<ActiveEmbodimentOperations>,
    process_id: ProcessId,
    principal: PrincipalId,
    workspace: fabric::WorkspacePolicy,
}

impl EmbodimentService {
    pub fn new(
        broker: Arc<Broker>,
        invoker: Arc<dyn CapabilityInvoker>,
        active: Arc<ActiveEmbodimentOperations>,
        process_id: ProcessId,
        principal: PrincipalId,
        workspace: fabric::WorkspacePolicy,
    ) -> Self {
        Self {
            broker,
            invoker,
            active,
            process_id,
            principal,
            workspace,
        }
    }
}

#[async_trait]
impl EmbodimentExecutionPort for EmbodimentService {
    async fn observe(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, SkillDispatchError> {
        self.broker.observe(device).await.map_err(map_broker_error)
    }

    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, SkillDispatchError> {
        self.broker
            .get_state(device)
            .await
            .map_err(map_broker_error)
    }

    async fn list_skills(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, SkillDispatchError> {
        self.broker
            .list_skills(device)
            .await
            .map_err(map_broker_error)
    }

    async fn execute_skill(
        &self,
        request: SkillRequest,
    ) -> Result<SkillResult, SkillDispatchError> {
        let operation_id = OperationId::new();
        let cancel = CancellationToken::new();
        let call_id = format!("embodiment:{operation_id:?}");
        let capability = CapabilityRequest {
            call: CapabilityCall {
                operation_id,
                process_id: self.process_id,
                name: "hardware.command".into(),
                input: serde_json::to_value(&request)
                    .map_err(|error| SkillDispatchError::Rejected(error.to_string()))?,
                call_id,
                deadline: None,
            },
            authority: CapabilityAuthority {
                agent: None,
                principal: self.principal.clone(),
                action: request.skill.0.clone(),
                requested_scope: CapabilityScope {
                    allowed_paths: vec![request.skill.0.clone()],
                    allowed_targets: vec![format!("device:{}", request.device.0)],
                    max_runtime_ms: Some(30_000),
                    max_output_bytes: Some(16 * 1024),
                },
                risk: RiskLevel::SystemModify,
                budget: None,
                lease: Some(LeaseRequest {
                    resource: format!("hardware:{}", request.device.0),
                    duration_ms: 30_000,
                }),
                sandbox: SandboxRequirement::NotRequired,
                connection_id: fabric::ConnectionId::default(),
                thread_id: fabric::ThreadId("embodiment".into()),
                turn_id: fabric::TurnId::new(),
                workspace: self.workspace.clone(),
                session_id: "embodiment".into(),
                working_dir: self.workspace.cwd().to_path_buf(),
            },
            control: InvocationControl {
                cancel,
                turn_event_sender: None,
            },
        };
        let result = self.invoker.invoke(capability).await;
        if result.is_error {
            return Err(SkillDispatchError::Rejected(result.output));
        }
        serde_json::from_str(&result.output)
            .map_err(|error| SkillDispatchError::Rejected(format!("invalid skill result: {error}")))
    }

    async fn cancel(&self, operation_id: &OperationId) -> Result<(), SkillDispatchError> {
        let active = self
            .active
            .get(operation_id)
            .await
            .ok_or_else(|| SkillDispatchError::Rejected("operation is not active".into()))?;
        active.cancel.cancel();
        self.broker
            .cancel(&active.device, &active.hardware_operation)
            .await
            .map_err(map_broker_error)
    }

    async fn safe_stop(&self, device: &DeviceId) -> Result<(), SkillDispatchError> {
        self.broker
            .safe_stop(device)
            .await
            .map_err(map_broker_error)
    }
}

fn map_broker_error(error: BrokerError) -> SkillDispatchError {
    match error {
        BrokerError::NoProvider(device) => SkillDispatchError::NoProvider(device),
        BrokerError::InvalidAuthority(reason) => SkillDispatchError::Rejected(reason),
        BrokerError::Provider(error) => SkillDispatchError::Rejected(error.to_string()),
    }
}
