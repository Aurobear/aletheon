//! Executive-owned operation creation, authorization, dispatch, and settlement.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    EmbodimentExecutionPort, SkillDispatchError, SkillOutcome, SkillRequest, SkillResult,
};
use hardware::{Broker, BrokerError};

use super::embodiment_authority::EmbodimentAuthorityPort;
use super::embodiment_progress::{BoundedProgressSink, EmbodimentProgressPort};

pub struct EmbodimentService {
    broker: Arc<Broker>,
    authority: Arc<dyn EmbodimentAuthorityPort>,
    progress: Arc<dyn EmbodimentProgressPort>,
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
        }
    }
}

#[async_trait]
impl EmbodimentExecutionPort for EmbodimentService {
    async fn execute_skill(
        &self,
        request: SkillRequest,
    ) -> Result<SkillResult, SkillDispatchError> {
        let operation_id = fabric::OperationId::new();
        let authorized = self.authority.authorize(operation_id, &request).await?;
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
        match (dispatched, settlement) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

fn map_broker_error(error: BrokerError) -> SkillDispatchError {
    match error {
        BrokerError::NoProvider(device) => SkillDispatchError::NoProvider(device),
        BrokerError::InvalidAuthority(reason) => SkillDispatchError::Rejected(reason),
        BrokerError::Provider(error) => SkillDispatchError::Rejected(error.to_string()),
    }
}
