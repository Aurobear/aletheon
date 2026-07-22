//! Executive-owned operation creation, authorization, dispatch, and settlement.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, OperationId, SkillDescriptor,
    SkillDispatchError, SkillOutcome, SkillRequest, SkillResult,
};
use hardware::{Broker, BrokerError};

use super::embodiment_authority::EmbodimentAuthorityPort;
use super::embodiment_progress::{BoundedProgressSink, EmbodimentProgressPort};

pub struct EmbodimentService {
    broker: Arc<Broker>,
    authority: Arc<dyn EmbodimentAuthorityPort>,
    progress: Arc<dyn EmbodimentProgressPort>,
    active_operations: tokio::sync::Mutex<std::collections::HashMap<OperationId, DeviceId>>,
}

impl EmbodimentService {
    pub fn new(
        broker: Arc<Broker>,
        authority: Arc<dyn EmbodimentAuthorityPort>,
        progress: Arc<dyn EmbodimentProgressPort>,
    ) -> Self {
        Self {
            broker,
            authority,
            progress,
            active_operations: tokio::sync::Mutex::new(Default::default()),
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
        let operation_id = fabric::OperationId::new();
        let authorized = self.authority.authorize(operation_id, &request).await?;
        self.active_operations
            .lock()
            .await
            .insert(operation_id, request.device.clone());
        let sink = Arc::new(BoundedProgressSink::new(
            operation_id,
            self.progress.clone(),
            64,
        ));
        let dispatched = self
            .broker
            .execute(authorized, sink)
            .await
            .map_err(map_broker_error);
        let succeeded = matches!(
            dispatched.as_ref().map(|result| &result.outcome),
            Ok(SkillOutcome::Succeeded)
        );
        let settlement = self.authority.settle(operation_id, succeeded).await;
        self.active_operations.lock().await.remove(&operation_id);
        match (dispatched, settlement) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    async fn cancel(&self, operation_id: &OperationId) -> Result<(), SkillDispatchError> {
        let device = self
            .active_operations
            .lock()
            .await
            .get(operation_id)
            .cloned()
            .ok_or_else(|| SkillDispatchError::Rejected("operation is not active".into()))?;
        self.broker
            .cancel(&device, &hardware::OperationId(operation_id.0.to_string()))
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
