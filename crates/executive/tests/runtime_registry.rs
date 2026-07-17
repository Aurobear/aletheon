use async_trait::async_trait;
use executive::core::sub_agent::SubAgentRuntime;
use executive::core::RuntimeRegistry;
use fabric::RuntimeId;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct ReportingRuntime(&'static str);

#[async_trait]
impl SubAgentRuntime for ReportingRuntime {
    async fn run(&self, _task: &str, _cancel: CancellationToken) -> Result<String, String> {
        Ok(self.0.into())
    }
}

#[test]
fn registry_rejects_duplicate_missing_and_blank_ids_without_owning_runs() {
    let id = RuntimeId("worker".into());
    let mut registry = RuntimeRegistry::new();
    registry
        .register(id.clone(), Arc::new(ReportingRuntime("one")))
        .unwrap();
    assert!(registry
        .register(id.clone(), Arc::new(ReportingRuntime("two")))
        .is_err());
    assert!(registry.resolve(&RuntimeId("missing".into())).is_err());
    assert!(registry
        .register(RuntimeId(" ".into()), Arc::new(ReportingRuntime("blank")))
        .is_err());

    let source = include_str!("../src/core/runtime_registry.rs");
    for forbidden in [
        "AgentRunRecord",
        "ProcessId",
        "OperationId",
        "SubAgentHandle",
        "HashMap<String",
    ] {
        assert!(
            !source.contains(forbidden),
            "runtime catalog owns forbidden {forbidden}"
        );
    }
}
