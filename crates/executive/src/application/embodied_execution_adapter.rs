//! Adapter from the fabric `EmbodimentExecutionPort` (owned by Executive /
//! hardware) to the cognit RobotHarness `EmbodiedExecutionPort`.
//!
//! The two ports differ: fabric carries `SkillDispatchError` and cancels by
//! `OperationId`; the cognit harness port preserves typed dispatch failures and cancels by
//! `DeviceId`. The adapter tracks the latest executed operation so a device-level
//! cancel can be forwarded to the fabric operation-level cancel.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::harness::robot::{EmbodiedExecutionPort, RobotExecutionError};
use fabric::types::embodiment::{DeviceId, SkillDispatchError, SkillRequest, SkillResult};
use fabric::OperationId;

pub struct EmbodiedExecutionAdapter {
    inner: Arc<dyn fabric::types::embodiment::EmbodimentExecutionPort>,
    last_operation: Mutex<Option<OperationId>>,
}

impl EmbodiedExecutionAdapter {
    pub fn new(inner: Arc<dyn fabric::types::embodiment::EmbodimentExecutionPort>) -> Self {
        Self {
            inner,
            last_operation: Mutex::new(None),
        }
    }
}

fn map_dispatch_error(error: SkillDispatchError) -> RobotExecutionError {
    match error {
        SkillDispatchError::NoProvider(reason) => RobotExecutionError::ProviderDisconnected(reason),
        SkillDispatchError::Rejected(reason) => RobotExecutionError::Rejected(reason),
    }
}

#[async_trait]
impl EmbodiedExecutionPort for EmbodiedExecutionAdapter {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
        let result = self
            .inner
            .execute_skill(request)
            .await
            .map_err(map_dispatch_error)?;
        *self.last_operation.lock().unwrap() = Some(result.operation_id);
        Ok(result)
    }

    async fn cancel(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        let operation = *self.last_operation.lock().unwrap();
        if let Some(operation) = operation {
            self.inner
                .cancel(&operation)
                .await
                .map_err(map_dispatch_error)?;
        }
        Ok(())
    }

    async fn safe_stop(&self, device: &DeviceId) -> Result<(), RobotExecutionError> {
        self.inner
            .safe_stop(device)
            .await
            .map_err(map_dispatch_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::{SkillId, SkillOutcome};
    use std::collections::VecDeque;

    struct FakeFabricPort {
        calls: Mutex<VecDeque<String>>,
    }
    #[async_trait::async_trait]
    impl fabric::types::embodiment::EmbodimentExecutionPort for FakeFabricPort {
        async fn observe(
            &self,
            _d: &DeviceId,
        ) -> Result<
            Vec<fabric::types::embodiment::EmbodiedObservation>,
            fabric::types::embodiment::SkillDispatchError,
        > {
            Ok(vec![])
        }
        async fn get_state(
            &self,
            _d: &DeviceId,
        ) -> Result<
            Option<fabric::types::embodiment::EmbodiedObservation>,
            fabric::types::embodiment::SkillDispatchError,
        > {
            Ok(None)
        }
        async fn list_skills(
            &self,
            _d: &DeviceId,
        ) -> Result<
            Vec<fabric::types::embodiment::SkillDescriptor>,
            fabric::types::embodiment::SkillDispatchError,
        > {
            Ok(vec![])
        }
        async fn execute_skill(
            &self,
            request: SkillRequest,
        ) -> Result<SkillResult, fabric::types::embodiment::SkillDispatchError> {
            self.calls.lock().unwrap().push_back("execute".into());
            Ok(SkillResult {
                operation_id: OperationId::new(),
                skill: request.skill,
                device: request.device,
                outcome: SkillOutcome::Succeeded,
                duration_ms: 0,
                evidence: vec![],
            })
        }
        async fn cancel(
            &self,
            _op: &OperationId,
        ) -> Result<(), fabric::types::embodiment::SkillDispatchError> {
            self.calls.lock().unwrap().push_back("cancel".into());
            Ok(())
        }
        async fn safe_stop(
            &self,
            _d: &DeviceId,
        ) -> Result<(), fabric::types::embodiment::SkillDispatchError> {
            self.calls.lock().unwrap().push_back("safe_stop".into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn execute_forwards_operation_and_cancel_uses_latest() {
        let inner = Arc::new(FakeFabricPort {
            calls: Mutex::new(VecDeque::new()),
        });
        let adapter = EmbodiedExecutionAdapter::new(inner.clone());
        let device = DeviceId("bot".into());
        let request = SkillRequest {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
        };
        let result = adapter.execute(request).await.unwrap();
        assert_eq!(result.outcome, SkillOutcome::Succeeded);
        adapter.cancel(&device).await.unwrap();
        let calls = inner.calls.lock().unwrap();
        assert_eq!(
            *calls,
            VecDeque::from(["execute".to_string(), "cancel".to_string()])
        );
    }

    #[tokio::test]
    async fn cancel_without_prior_execute_is_noop() {
        let inner = Arc::new(FakeFabricPort {
            calls: Mutex::new(VecDeque::new()),
        });
        let adapter = EmbodiedExecutionAdapter::new(inner.clone());
        adapter.cancel(&DeviceId("bot".into())).await.unwrap();
        assert!(inner.calls.lock().unwrap().is_empty());
    }
}
