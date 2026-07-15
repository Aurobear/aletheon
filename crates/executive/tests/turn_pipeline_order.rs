use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::service::ServicePorts;
use async_trait::async_trait;
use executive::service::{PostTurnPipeline, PreTurnPipeline, TurnService};
use fabric::{
    CapabilityCall, CapabilityResult, ContentBlock, LlmProvider, LlmResponse, LlmStream,
    NoopTurnEventSink, OperationId, ProcessId, RecallRequest, RecallSet, StopReason,
    ToolDefinition, TurnRequest, TurnServices, Usage,
};

fn test_ports() -> Arc<ServicePorts> {
    let clock: Arc<dyn fabric::Clock> = Arc::new(TestClock::default());
    let admission: Arc<dyn fabric::AdmissionController> =
        Arc::new(aletheon_kernel::admission::AllowAllAdmissionController::new(clock.clone()));
    Arc::new(ServicePorts::for_testing(clock, admission))
}

fn request() -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        session_id: "pipeline".into(),
        input: "use tool".into(),
        working_dir: PathBuf::from("."),
        model_policy: None,
        deadline: None,
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
        _messages: &[fabric::Message],
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
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "done: hi".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
    }

    async fn complete_stream(
        &self,
        _messages: &[fabric::Message],
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
    block_pre_turn: bool,
}

impl PipelineServices {
    fn new(events: Arc<Mutex<Vec<String>>>, block_pre_turn: bool) -> Self {
        Self {
            llm: ScriptedLlm {
                events: events.clone(),
                calls: Mutex::new(0),
            },
            events,
            block_pre_turn,
        }
    }
}

#[async_trait]
impl TurnServices for PipelineServices {
    async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
        self.events.lock().unwrap().push("pre:recall".into());
        if self.block_pre_turn {
            anyhow::bail!("blocked by pre-turn recall")
        }
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
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
            usage: fabric::UsageReport::default(),
            audit_id: None,
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
    let services = Arc::new(PipelineServices::new(events.clone(), false));
    let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, test_ports());

    let result = turn_service
        .submit(request(), &NoopTurnEventSink)
        .await
        .expect("turn should complete");

    assert_eq!(result.output, "done: hi");
    assert_eq!(
        *events.lock().unwrap(),
        vec!["pre:recall", "llm", "invoke:echo_tool:call_1", "llm"]
    );
}

#[tokio::test]
async fn pre_turn_error_blocks_model_call() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let services = Arc::new(PipelineServices::new(events.clone(), true));
    let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, test_ports());

    let err = turn_service
        .submit(request(), &NoopTurnEventSink)
        .await
        .expect_err("pre-turn error should abort submit");

    assert!(err.to_string().contains("blocked by pre-turn recall"));
    assert_eq!(*events.lock().unwrap(), vec!["pre:recall"]);
}
