use std::sync::Arc;

use async_trait::async_trait;
use fabric::{AgentControlError, AgentControlErrorKind, AgentSpawnRequest};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub trait AgentAdmissionLease: Send {
    fn release_completed(&mut self);
    fn revoke(&mut self);
}

#[async_trait]
pub trait AgentAdmissionPort: Send + Sync {
    async fn reserve(
        &self,
        request: &AgentSpawnRequest,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError>;
}

#[derive(Debug)]
pub struct BoundedAgentAdmission {
    permits: Arc<Semaphore>,
}

impl BoundedAgentAdmission {
    pub fn new(max_concurrent: usize) -> Result<Self, AgentControlError> {
        if max_concurrent == 0 {
            return Err(AgentControlError::invalid(
                "Agent concurrency limit must be nonzero",
            ));
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(max_concurrent)),
        })
    }

    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }
}

#[async_trait]
impl AgentAdmissionPort for BoundedAgentAdmission {
    async fn reserve(
        &self,
        _request: &AgentSpawnRequest,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| AgentControlError {
                kind: AgentControlErrorKind::Capacity,
                message: "Agent concurrency capacity is exhausted".into(),
            })?;
        Ok(Box::new(BoundedAdmissionLease {
            permit: Some(permit),
        }))
    }
}

struct BoundedAdmissionLease {
    permit: Option<OwnedSemaphorePermit>,
}

impl AgentAdmissionLease for BoundedAdmissionLease {
    fn release_completed(&mut self) {
        self.permit.take();
    }

    fn revoke(&mut self) {
        self.permit.take();
    }
}
