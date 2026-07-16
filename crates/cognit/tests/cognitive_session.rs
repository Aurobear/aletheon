use async_trait::async_trait;
use cognit::harness::{
    CognitiveSession, CognitiveSessionDependencies, HarnessConfig, LinearCognitiveSession,
};
use fabric::{
    CapabilityCall, CapabilityResult, ContentBlock, LlmProvider, LlmResponse, LlmStream,
    NoopTurnEventSink, OperationId, ProcessId, StopReason, StubTurnServices, ToolDefinition,
    TurnRequest, TurnServices, TurnStop, Usage,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn dependencies() -> CognitiveSessionDependencies {
    CognitiveSessionDependencies {
        clock: Arc::new(aletheon_kernel::chronos::TestClock::default()),
        cancellation: CancellationToken::new(),
    }
}

fn request(input: &str) -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        session_id: "test".into(),
        input: input.into(),
        working_dir: PathBuf::from("."),
        model_policy: None,
        deadline: None,
    }
}

#[tokio::test]
async fn linear_session_returns_turn_result() {
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_turn(request("hello"), &StubTurnServices, &NoopTurnEventSink)
        .await
        .expect("turn should run");

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(result.output, "hello");
}

struct ScriptedLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn complete(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
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
        unimplemented!("not used by this test")
    }

    fn name(&self) -> &str {
        "scripted"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

struct ScriptedTurnServices {
    llm: ScriptedLlm,
    invoked: Mutex<Vec<String>>,
}

#[async_trait]
impl TurnServices for ScriptedTurnServices {
    async fn recall(&self, _req: fabric::RecallRequest) -> anyhow::Result<fabric::RecallSet> {
        Ok(fabric::RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        self.invoked.lock().unwrap().push(req.name.clone());
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
async fn linear_session_runs_react_with_turn_services() {
    let services = ScriptedTurnServices {
        llm: ScriptedLlm {
            calls: Mutex::new(0),
        },
        invoked: Mutex::new(Vec::new()),
    };
    let mut session = LinearCognitiveSession::new(
        HarnessConfig {
            max_iterations: 4,
            ..Default::default()
        },
        dependencies(),
    );

    let result = session
        .run_turn(request("use tool"), &services, &NoopTurnEventSink)
        .await
        .expect("turn should run through LLM and tool service");

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(result.output, "done: hi");
    assert_eq!(result.metrics.tool_calls_made, 1);
    assert_eq!(*services.invoked.lock().unwrap(), vec!["echo_tool"]);
}
