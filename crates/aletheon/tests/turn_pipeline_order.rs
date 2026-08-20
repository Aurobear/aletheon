#![cfg(feature = "test-support")]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ::contracts::{
    CapabilityCall, CapabilityResult, ContentBlock, InferenceUsage, LlmProvider, LlmResponse,
    LlmStream, NoopTurnEventSink, OperationId, ProcessId, RecallRequest, RecallSet, StopReason,
    ToolDefinition, TurnRequest, TurnServices,
};
use async_trait::async_trait;
use runtime::{post_turn::PostTurnPipeline, pre_turn::PreTurnPipeline};
#[path = "support/turn_service.rs"]
mod test_turn_service;
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use test_turn_service::TurnService;

fn test_kernel() -> Arc<KernelRuntime> {
    let clock: Arc<dyn ::contracts::Clock> = Arc::new(TestClock::default());
    let admission: Arc<dyn kernel::AdmissionController> = Arc::new(
        kernel::admission::AllowAllAdmissionController::new(clock.clone()),
    );
    Arc::new(KernelRuntime::with_admission(clock, admission))
}

fn request(process_id: ProcessId) -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id,
        context: turn_request_support::context("pipeline", PathBuf::from(".")),
        input: "use tool".into(),
        execution_target: ::contracts::ExecutionTargetSelection::default(),
        model_policy: None,
        deadline: None,
        requirements: Vec::new(),
        requested_task_kind: None,
        evaluation_contract: None,
    }
}

struct ScriptedLlm {
    events: Arc<Mutex<Vec<String>>>,
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn complete(
        &self,
        _messages: &[::contracts::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.events.lock().unwrap().push("llm".into());
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Ok(LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "echo_tool".into(),
                    input: serde_json::json!({"text": "hi"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: InferenceUsage::default(),
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "done: hi".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
            })
        }
    }

    async fn complete_stream(
        &self,
        _messages: &[::contracts::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("streaming not used by TurnService unit tests")
    }

    fn name(&self) -> &str {
        "scripted"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

struct PipelineServices {
    events: Arc<Mutex<Vec<String>>>,
    llm: ScriptedLlm,
}

impl PipelineServices {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            llm: ScriptedLlm {
                events: events.clone(),
                calls: Mutex::new(0),
            },
            events,
        }
    }
}

#[async_trait]
impl TurnServices for PipelineServices {
    async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
        anyhow::bail!("legacy turn recall must not be called")
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(::contracts::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(::contracts::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        self.events
            .lock()
            .unwrap()
            .push(format!("invoke:{}:{}", req.name, req.call_id));
        CapabilityResult {
            call_id: req.call_id,
            output: req.input["text"].as_str().unwrap_or_default().to_string(),
            is_error: false,
            usage: ::contracts::UsageReport::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.llm)
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "echo_tool".into(),
            description: "echo text".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }]
    }
}

#[tokio::test]
async fn turn_pipeline_runs_pre_cognit_capability_in_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let services = Arc::new(PipelineServices::new(events.clone()));
    let kernel = test_kernel();
    let process = kernel
        .spawn_process(::contracts::SpawnSpec::default())
        .await
        .unwrap();
    let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel);

    let result = turn_service
        .submit(request(process.id), &NoopTurnEventSink)
        .await
        .expect("turn should complete");

    assert_eq!(result.output, "done: hi");
    assert_eq!(
        *events.lock().unwrap(),
        vec!["llm", "invoke:echo_tool:call_1", "llm"]
    );
}

#[tokio::test]
async fn legacy_turn_recall_cannot_bypass_workspace_projection() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let services = Arc::new(PipelineServices::new(events.clone()));
    let kernel = test_kernel();
    let process = kernel
        .spawn_process(::contracts::SpawnSpec::default())
        .await
        .unwrap();
    let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel);

    let result = turn_service
        .submit(request(process.id), &NoopTurnEventSink)
        .await
        .expect("legacy recall hook must not participate in prompt assembly");

    assert_eq!(result.output, "done: hi");
    assert_eq!(
        *events.lock().unwrap(),
        vec!["llm", "invoke:echo_tool:call_1", "llm"]
    );
}
mod turn_request_support;
