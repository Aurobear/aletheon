use ::contracts::RuntimeId;
use async_trait::async_trait;
use std::sync::Arc;

struct ReportingBackend;

#[async_trait]
impl runtime::DelegateBackend for ReportingBackend {
    async fn spawn(
        &self,
        _request: &runtime::DelegateSpawnRequest,
        identity: &runtime::DelegateReceipt,
    ) -> Result<runtime::DelegateReceipt, runtime::RuntimeError> {
        Ok(identity.clone())
    }

    async fn cancel(&self, _agent_run: &runtime::AgentRunId) -> Result<(), runtime::RuntimeError> {
        Ok(())
    }

    async fn wait(
        &self,
        _agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::event::TurnTerminal, runtime::RuntimeError> {
        Ok(runtime::event::TurnTerminal::Completed)
    }
}

#[test]
fn registry_rejects_duplicate_missing_and_blank_ids_without_owning_runs() {
    let id = RuntimeId("worker".into());
    let registry = runtime::DelegateBackendRegistry::new();
    registry
        .register(
            runtime::DelegateBackendId(id.0.clone()),
            Arc::new(ReportingBackend),
        )
        .unwrap();
    assert!(registry
        .register(
            runtime::DelegateBackendId(id.0.clone()),
            Arc::new(ReportingBackend),
        )
        .is_err());
    assert!(registry
        .resolve(&runtime::DelegateBackendId("missing".into()))
        .is_none());
    assert!(registry
        .register(
            runtime::DelegateBackendId(" ".into()),
            Arc::new(ReportingBackend)
        )
        .is_err());
}
