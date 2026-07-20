use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::{
    CognitErrorKind, CognitRetryDisposition, CognitiveSessionDependencies, HarnessConfig,
};
use fabric::{
    CapabilityCall, CapabilityResult, LlmProvider, LlmResponse, LlmStream, OperationId, ProcessId,
    ToolDefinition, TurnEvent, TurnEventSink, TurnRequest, TurnServices,
};
use tokio_util::sync::CancellationToken;

fn request() -> TurnRequest {
    let cwd = std::env::current_dir().unwrap();
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context: fabric::PrincipalContext::new(
            fabric::PrincipalId("test:facade".into()),
            fabric::LocalOsPrincipal { uid: 0, gid: 0 },
            fabric::ConnectionId::new(),
            fabric::ThreadId("facade".into()),
            fabric::WorkspacePolicy::from_resolved_roots(cwd, vec![]).unwrap(),
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
        ),
        input: "test facade".into(),
        model_policy: None,
        deadline: None,
    }
}

fn dependencies(cancel: CancellationToken) -> CognitiveSessionDependencies {
    CognitiveSessionDependencies {
        clock: Arc::new(kernel::chronos::TestClock::default()),
        cancellation: cancel,
        compactor: None,
        batch_planner: None,
        evicted_callback: None,
        verifier: None,
    }
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<TurnEvent>>);

#[async_trait]
impl TurnEventSink for RecordingEvents {
    async fn emit(&self, event: TurnEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct FailingProvider(&'static str);

#[async_trait]
impl LlmProvider for FailingProvider {
    async fn complete(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!(self.0)
    }

    async fn complete_stream(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        anyhow::bail!(self.0)
    }

    fn name(&self) -> &str {
        "failing"
    }

    fn max_context_length(&self) -> usize {
        128_000
    }
}

struct Services(FailingProvider);

#[async_trait]
impl TurnServices for Services {
    async fn recall(&self, _req: fabric::RecallRequest) -> anyhow::Result<fabric::RecallSet> {
        Ok(fabric::RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        CapabilityResult {
            call_id: call.call_id,
            output: "not reached".into(),
            is_error: true,
            usage: fabric::UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.0)
    }
}

#[test]
fn production_cognit_has_no_concrete_kernel_dependency() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    assert!(manifest["dependencies"].get("kernel").is_none());
    assert!(manifest["dev-dependencies"].get("kernel").is_some());

    let scheduler = include_str!("../src/impl/llm/scheduler.rs");
    let production = scheduler.split("#[cfg(test)]").next().unwrap();
    assert!(!production.contains("kernel::"));
}

#[test]
fn crate_root_exposes_session_facade_not_concrete_loop() {
    let root = include_str!("../src/lib.rs");
    assert!(root.contains("CognitiveSession"));
    assert!(!root.contains("ReActLoop"));
    assert!(!root.contains("build_harness"));
}
