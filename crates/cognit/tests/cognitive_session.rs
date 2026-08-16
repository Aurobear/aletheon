use ::contracts::{
    CapabilityCall, CapabilityResult, ContentBlock, InferenceUsage, LlmProvider, LlmResponse,
    LlmStream, Message, NoopTurnEventSink, OperationId, ProcessId, Role, StopReason, StubTurnServices,
    ToolDefinition, TurnRequest, TurnServices, TurnStop,
};
use async_trait::async_trait;
use cognit::harness::session_log::{HarnessSessionEventKind, HarnessSessionId};
use cognit::harness::{
    CognitiveSession, CognitiveSessionDependencies, CognitiveStreamEvent, CognitiveStreamSink,
    HarnessCognitiveSession, HarnessConfig, LinearCognitiveSession,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn dependencies() -> CognitiveSessionDependencies {
    CognitiveSessionDependencies {
        clock: Arc::new(kernel::chronos::TestClock::default()),
        cancellation: CancellationToken::new(),
        compactor: None,
        batch_planner: None,
        evicted_callback: None,
        verifier: None,
        grounded_outcome_sink: None,
    }
}

fn request(input: &str) -> TurnRequest {
    let cwd = std::env::current_dir().unwrap();
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context: ::contracts::PrincipalContext::new(
            ::contracts::PrincipalId("test:cognitive-session".into()),
            ::contracts::LocalOsPrincipal { uid: 0, gid: 0 },
            ::contracts::ConnectionId::new(),
            ::contracts::ThreadId("test".into()),
            ::contracts::WorkspacePolicy::from_resolved_roots(cwd, vec![]).unwrap(),
            ::contracts::PermissionProfileId::workspace_write(),
            ::contracts::ApprovalPolicy::OnRequest,
        ),
        input: input.into(),
        execution_target: ::contracts::ExecutionTargetSelection::default(),
        model_policy: None,
        deadline: None,
        requirements: Vec::new(),
        requested_task_kind: None,
        evaluation_contract: None,
    }
}

fn with_coding_contract(
    mut request: TurnRequest,
    mode: ::contracts::EvaluationMode,
) -> TurnRequest {
    let turn_id = ::contracts::TurnId::new();
    request.context.turn_id = Some(turn_id);
    request.requested_task_kind = Some(::contracts::TaskKind::Coding);
    request.evaluation_contract = Some(::contracts::TaskEvaluationContract {
        schema_version: ::contracts::EVALUATION_SCHEMA_V1,
        contract_id: ::contracts::EvaluationContractId::new(),
        task_kind: ::contracts::TaskKind::Coding,
        subject: ::contracts::EvaluationSubject::Turn {
            turn_id,
            operation_id: request.operation_id,
        },
        rubric: ::contracts::types::metacognition_evaluation::RubricId("coding-v2".into()),
        rubric_version: 2,
        mode,
        objective_ref: ::contracts::EvidenceRef("test:objective".into()),
        requirement_refs: Vec::new(),
        required_evidence: vec![::contracts::RequiredEvidence {
            kind: ::contracts::types::metacognition_evidence::EvidenceKind::VerificationResult,
            minimum_count: 1,
            authoritative: true,
        }],
        required_gates: vec![
            ::contracts::RequiredGate {
                name: "required_verification_passed".into(),
            },
            ::contracts::RequiredGate {
                name: "change_within_scope".into(),
            },
        ],
        thresholds: ::contracts::EvaluationThresholds {
            min_score_millis: 70_000,
            min_evidence_coverage_millis: 600,
            min_confidence_millis: 700,
        },
        issued_by: "test".into(),
        issued_at_ms: 1,
    });
    request
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
        _messages: &[::contracts::Message],
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
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(::contracts::RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(::contracts::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(::contracts::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        self.invoked.lock().unwrap().push(req.name.clone());
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

#[tokio::test]
async fn core_harness_session_adapts_public_turn_services_without_a_second_history() {
    let services = ScriptedTurnServices {
        llm: ScriptedLlm {
            calls: Mutex::new(0),
        },
        invoked: Mutex::new(Vec::new()),
    };
    let mut session = HarnessCognitiveSession::new(
        HarnessSessionId("core-adapter".into()),
        Arc::new(kernel::chronos::TestClock::default()),
        CancellationToken::new(),
    )
    .unwrap();
    let log = session.event_log();
    let result = session
        .run_turn(request("use echo"), &services, &NoopTurnEventSink)
        .await
        .unwrap();

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(result.output, "done: hi");
    assert_eq!(result.metrics.iterations, 2);
    assert_eq!(result.metrics.tool_calls_made, 1);
    assert_eq!(services.invoked.lock().unwrap().as_slice(), ["echo_tool"]);
    let log = log.lock().unwrap();
    assert_eq!(log.derive_messages().len(), 4);
    assert!(matches!(
        log.events().last().unwrap().kind,
        HarnessSessionEventKind::TurnEnd { .. }
    ));
}

struct SeedRecordingLlm {
    requests: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl LlmProvider for SeedRecordingLlm {
    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.requests.lock().unwrap().push(messages.to_vec());
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "seeded".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("collecting Harness adapter uses complete")
    }

    fn name(&self) -> &str {
        "seed-recording"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

struct SeededTurnServices {
    llm: SeedRecordingLlm,
    seed: Vec<Message>,
}

#[async_trait]
impl TurnServices for SeededTurnServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(Default::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        CapabilityResult {
            call_id: call.call_id,
            output: String::new(),
            is_error: false,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.llm)
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        self.seed.clone()
    }
}

#[tokio::test]
async fn core_harness_preserves_typed_seed_messages_without_json_system_reencoding() {
    let seed = vec![
        Message::system("system instructions"),
        Message::user("older question"),
        Message::assistant("older answer"),
        Message::user("effective hello with projected context"),
    ];
    let services = SeededTurnServices {
        llm: SeedRecordingLlm {
            requests: Mutex::new(Vec::new()),
        },
        seed: seed.clone(),
    };
    let mut session = HarnessCognitiveSession::new(
        HarnessSessionId("seed-adapter".into()),
        Arc::new(kernel::chronos::TestClock::default()),
        CancellationToken::new(),
    )
    .unwrap();

    session
        .run_turn(request("hello"), &services, &NoopTurnEventSink)
        .await
        .unwrap();

    let requests = services.llm.requests.lock().unwrap();
    assert_eq!(requests.as_slice(), &[seed]);
    assert_eq!(requests[0][0].role, Role::System);
}

struct ActivatingLlm {
    calls: Mutex<usize>,
    visible_tools: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl LlmProvider for ActivatingLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.visible_tools
            .lock()
            .unwrap()
            .push(tools.iter().map(|tool| tool.name.clone()).collect());
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        Ok(if *calls == 1 {
            LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "discover".into(),
                    name: "tool_search".into(),
                    input: serde_json::json!({}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            }
        } else {
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "activated".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            }
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("collecting Harness adapter uses complete")
    }

    fn name(&self) -> &str {
        "activating"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

struct ActivatingServices {
    llm: ActivatingLlm,
    activated: tokio::sync::Mutex<Vec<ToolDefinition>>,
}

#[async_trait]
impl TurnServices for ActivatingServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(Default::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        if call.name == "tool_search" {
            self.activated.lock().await.push(ToolDefinition {
                name: "activated_tool".into(),
                description: "discovered tool".into(),
                input_schema: serde_json::json!({"type":"object"}),
            });
        }
        CapabilityResult {
            call_id: call.call_id,
            output: "found".into(),
            is_error: false,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    async fn drain_activated_tool_definitions(&self) -> Vec<ToolDefinition> {
        std::mem::take(&mut *self.activated.lock().await)
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.llm)
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "tool_search".into(),
            description: "discover tools".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }]
    }
}

#[tokio::test]
async fn core_harness_exposes_discovered_tool_schema_on_the_next_inference() {
    let services = ActivatingServices {
        llm: ActivatingLlm {
            calls: Mutex::new(0),
            visible_tools: Mutex::new(Vec::new()),
        },
        activated: tokio::sync::Mutex::new(Vec::new()),
    };
    let mut session = HarnessCognitiveSession::new(
        HarnessSessionId("tool-activation".into()),
        Arc::new(kernel::chronos::TestClock::default()),
        CancellationToken::new(),
    )
    .unwrap();

    session
        .run_turn(request("discover"), &services, &NoopTurnEventSink)
        .await
        .unwrap();

    assert_eq!(
        services.llm.visible_tools.lock().unwrap().as_slice(),
        [
            vec!["tool_search".to_string()],
            vec!["activated_tool".to_string(), "tool_search".to_string()],
        ]
    );
}

struct StreamingServices {
    llm: cognit::testing::mock_llm::MockLlmProvider,
}

#[async_trait]
impl TurnServices for StreamingServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(::contracts::DaseinView {
            text: Some("calm".into()),
        })
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        CapabilityResult {
            call_id: call.call_id,
            output: "unused".into(),
            is_error: true,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.llm)
    }
}

#[derive(Default)]
struct RecordingStream(Mutex<Vec<CognitiveStreamEvent>>);

impl CognitiveStreamSink for RecordingStream {
    fn emit(&self, event: CognitiveStreamEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn streaming_session_preserves_interactive_events_behind_the_facade() {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new("streaming");
    llm.push_text_response("streamed answer", StopReason::EndTurn);
    let services = StreamingServices { llm };
    let stream = RecordingStream::default();
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_streaming_turn(
            request("stream this"),
            &services,
            &NoopTurnEventSink,
            &stream,
        )
        .await
        .unwrap();

    assert_eq!(result.output, "streamed answer");
    let events = stream.0.lock().unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event, CognitiveStreamEvent::GoalSet { .. })));
    assert!(events.iter().any(
        |event| matches!(event, CognitiveStreamEvent::TextDelta { delta } if delta == "streamed answer")
    ));
    assert!(events
        .iter()
        .any(|event| matches!(event, CognitiveStreamEvent::TurnDone { result: Ok(text) } if text == "streamed answer")));
}

#[tokio::test]
async fn streaming_session_keeps_thinking_out_of_visible_and_final_text() {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new("thinking");
    llm.push_response(LlmResponse {
        content: vec![
            ContentBlock::Thinking {
                text: "internal reasoning".into(),
                signature: None,
            },
            ContentBlock::Text {
                text: "visible answer".into(),
            },
        ],
        stop_reason: StopReason::EndTurn,
        usage: InferenceUsage::default(),
    });
    let services = StreamingServices { llm };
    let stream = RecordingStream::default();
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_streaming_turn(
            request("think privately"),
            &services,
            &NoopTurnEventSink,
            &stream,
        )
        .await
        .unwrap();

    assert_eq!(result.output, "visible answer");
    assert!(!stream.0.lock().unwrap().iter().any(
        |event| matches!(event, CognitiveStreamEvent::TextDelta { delta } if delta.contains("internal reasoning"))
    ));
}

struct CodingContractServices {
    llm: cognit::testing::mock_llm::MockLlmProvider,
    terminal_validation: bool,
}

#[async_trait]
impl TurnServices for CodingContractServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(Default::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        let output = if self.terminal_validation && call.name == "validation_run" {
            serde_json::json!({
                "session_id": "validation-session",
                "terminal": {"status": "exited", "exit_code": 0},
                "output_artifact_ref": "artifact://sha256/validation"
            })
            .to_string()
        } else {
            "unused".into()
        };
        CapabilityResult {
            call_id: call.call_id,
            output,
            is_error: false,
            usage: Default::default(),
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
            name: "validation_run".into(),
            description: "run a deterministic validation".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }]
    }

    fn turn_requirements(&self, request: &TurnRequest) -> Vec<::contracts::TurnRequirement> {
        request.requirements.clone()
    }
}

fn coding_contract_services(name: &str, responses: usize) -> CodingContractServices {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new(name);
    for _ in 0..responses {
        llm.push_text_response("claimed complete", StopReason::EndTurn);
    }
    CodingContractServices {
        llm,
        terminal_validation: false,
    }
}

fn projected_model_text(messages: &[::contracts::Message]) -> String {
    messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn coding_contract_configures_code_change_without_turn_requirements() {
    let services = coding_contract_services("coding-contract-shadow", 1);
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_turn(
            with_coding_contract(request("change code"), ::contracts::EvaluationMode::Shadow),
            &services,
            &NoopTurnEventSink,
        )
        .await
        .expect("shadow contract remains observable without blocking completion");

    assert_eq!(result.stop, TurnStop::Completed);
    let calls = services.llm.call_log.lock().unwrap();
    let projected = projected_model_text(calls.first().expect("model call"));
    assert!(projected.contains("explicitly typed coding task"));
    assert!(projected.contains("authoritative verification_result"));
    assert!(projected.contains("change_within_scope"));
}

#[tokio::test]
async fn coding_contract_configures_enforced_completion_gate() {
    let services = coding_contract_services("coding-contract-enforce", 3);
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_turn(
            with_coding_contract(request("change code"), ::contracts::EvaluationMode::Enforce),
            &services,
            &NoopTurnEventSink,
        )
        .await
        .expect("missing typed evidence becomes a blocked outcome");

    assert_eq!(result.stop, TurnStop::Blocked);
    assert!(result.output.contains("verification_result"));
    let calls = services.llm.call_log.lock().unwrap();
    assert!(projected_model_text(calls.first().expect("model call")).contains("validation_run"));
}

#[tokio::test]
async fn coding_contract_merges_explicit_turn_requirements() {
    let services = coding_contract_services("coding-contract-merged", 3);
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());
    let mut turn = with_coding_contract(
        request("delegate code change"),
        ::contracts::EvaluationMode::Shadow,
    );
    turn.requirements = vec![::contracts::TurnRequirement::InvokeAgentRuntime {
        runtime_id: "pi-rpc".into(),
    }];

    let result = session
        .run_turn(turn, &services, &NoopTurnEventSink)
        .await
        .expect("explicit turn requirements retain their enforced authority");

    assert_eq!(result.stop, TurnStop::Blocked);
    assert!(result.output.contains("pi-rpc"));
    let calls = services.llm.call_log.lock().unwrap();
    let projected = projected_model_text(calls.first().expect("model call"));
    assert!(projected.contains("explicitly typed coding task"));
    assert!(projected.contains("`agent_spawn`"));
    assert!(projected.contains("`pi-rpc`"));
}

#[tokio::test]
async fn terminal_validation_satisfies_enforced_coding_contract() {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new("coding-contract-terminal");
    llm.push_response(LlmResponse {
        content: vec![ContentBlock::ToolUse {
            id: "validation-1".into(),
            name: "validation_run".into(),
            input: serde_json::json!({}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: InferenceUsage::default(),
    });
    llm.push_text_response("verified change", StopReason::EndTurn);
    let services = CodingContractServices {
        llm,
        terminal_validation: true,
    };
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_turn(
            with_coding_contract(request("change code"), ::contracts::EvaluationMode::Enforce),
            &services,
            &NoopTurnEventSink,
        )
        .await
        .expect("authoritative terminal validation satisfies the cognitive gate");

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(result.output, "verified change");
}

struct RequiredCapabilityServices {
    llm: cognit::testing::mock_llm::MockLlmProvider,
    projections: Mutex<Vec<::contracts::model_projection::ModelContextProjectionReceipt>>,
}

#[async_trait]
impl TurnServices for RequiredCapabilityServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(Default::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        CapabilityResult {
            call_id: call.call_id,
            output: "unused".into(),
            is_error: false,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(&self.llm)
    }

    fn turn_requirements(&self, request: &TurnRequest) -> Vec<::contracts::TurnRequirement> {
        if request.requirements.is_empty() {
            vec![::contracts::TurnRequirement::InvokeCapability {
                name: "file_read".into(),
            }]
        } else {
            request.requirements.clone()
        }
    }

    async fn record_model_context_projection(
        &self,
        receipt: ::contracts::model_projection::ModelContextProjectionReceipt,
    ) {
        self.projections.lock().unwrap().push(receipt);
    }
}

fn required_capability_services(name: &str) -> RequiredCapabilityServices {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new(name);
    for _ in 0..3 {
        llm.push_text_response("claimed complete", StopReason::EndTurn);
    }
    RequiredCapabilityServices {
        llm,
        projections: Mutex::new(Vec::new()),
    }
}

#[tokio::test]
async fn collecting_session_cannot_bypass_typed_completion_requirement() {
    let services = required_capability_services("collecting-gate");
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_turn(request("inspect the target"), &services, &NoopTurnEventSink)
        .await
        .expect("missing evidence is a typed blocked outcome");

    assert_eq!(result.stop, TurnStop::Blocked);
    assert!(!result.metrics.completed_normally);
    assert!(result.output.contains("file_read"));
    assert_eq!(services.llm.call_log.lock().unwrap().len(), 3);
    let projections = services.projections.lock().unwrap();
    assert_eq!(projections.len(), 3);
    assert!(projections
        .iter()
        .all(|receipt| !receipt.fragments.is_empty() && receipt.message_bytes > 0));
}

#[tokio::test]
async fn streaming_session_cannot_bypass_typed_completion_requirement() {
    let services = required_capability_services("streaming-gate");
    let stream = RecordingStream::default();
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_streaming_turn(
            request("inspect the target"),
            &services,
            &NoopTurnEventSink,
            &stream,
        )
        .await
        .expect("missing evidence is a typed blocked outcome");

    assert_eq!(result.stop, TurnStop::Blocked);
    assert!(!result.metrics.completed_normally);
    assert!(result.output.contains("file_read"));
    assert!(stream.0.lock().unwrap().iter().any(|event| {
        matches!(event, CognitiveStreamEvent::TurnDone { result: Err(message) } if message.contains("file_read"))
    }));
    assert_eq!(services.llm.call_log.lock().unwrap().len(), 3);
    assert_eq!(services.projections.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn explicit_request_agent_requirement_reaches_the_enforced_gate() {
    let services = required_capability_services("request-agent-gate");
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());
    let mut turn = request("inspect through the selected runtime");
    turn.requirements = vec![::contracts::TurnRequirement::InvokeAgentRuntime {
        runtime_id: "pi-rpc".into(),
    }];

    let result = session
        .run_turn(turn, &services, &NoopTurnEventSink)
        .await
        .expect("missing current-turn Agent receipt is a typed blocked outcome");

    assert_eq!(result.stop, TurnStop::Blocked);
    assert!(!result.metrics.completed_normally);
    assert!(result.output.contains("pi-rpc"));
}

struct InterjectingServices {
    llm: cognit::testing::mock_llm::MockLlmProvider,
    drains: Mutex<VecDeque<Vec<String>>>,
    drain_calls: AtomicUsize,
    invoked: Mutex<usize>,
}

#[async_trait]
impl TurnServices for InterjectingServices {
    async fn recall(
        &self,
        _req: ::contracts::RecallRequest,
    ) -> anyhow::Result<::contracts::RecallSet> {
        Ok(Default::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<::contracts::DaseinView> {
        Ok(Default::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<::contracts::AgoraView> {
        Ok(Default::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        assert_eq!(
            self.drain_calls.load(Ordering::SeqCst),
            0,
            "interjections must not drain while a tool/approval is in flight"
        );
        *self.invoked.lock().unwrap() += 1;
        CapabilityResult {
            call_id: call.call_id,
            output: "tool-finished".into(),
            is_error: false,
            usage: Default::default(),
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
            description: "test".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }]
    }

    async fn drain_interjections(&self) -> anyhow::Result<Vec<String>> {
        self.drain_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.drains.lock().unwrap().pop_front().unwrap_or_default())
    }
}

fn single_text(message: &::contracts::Message) -> Option<&str> {
    match message.content.as_slice() {
        [ContentBlock::Text { text }] => Some(text),
        _ => None,
    }
}

#[tokio::test]
async fn react_completion_injects_fifo_interjections_as_independent_user_messages() {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new("interjection");
    llm.push_text_response("draft", StopReason::EndTurn);
    llm.push_text_response("final", StopReason::EndTurn);
    let services = InterjectingServices {
        llm,
        drains: Mutex::new(VecDeque::from([vec!["first".into(), "second".into()]])),
        drain_calls: AtomicUsize::new(0),
        invoked: Mutex::new(0),
    };
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    let result = session
        .run_streaming_turn(
            request("start"),
            &services,
            &NoopTurnEventSink,
            &RecordingStream::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "final");
    let log = services.llm.call_log.lock().unwrap();
    let second_call = &log[1];
    assert_eq!(
        single_text(&second_call[second_call.len() - 3]),
        Some("draft")
    );
    assert_eq!(
        single_text(&second_call[second_call.len() - 2]),
        Some("first")
    );
    assert_eq!(
        single_text(&second_call[second_call.len() - 1]),
        Some("second")
    );
    assert!(second_call[second_call.len() - 2..]
        .iter()
        .all(|message| message.role == ::contracts::Role::User));
}

#[tokio::test]
async fn tool_interjection_is_injected_only_after_tool_result_is_absorbed() {
    let llm = cognit::testing::mock_llm::MockLlmProvider::new("tool-interjection");
    llm.push_response(LlmResponse {
        content: vec![ContentBlock::ToolUse {
            id: "call-1".into(),
            name: "echo_tool".into(),
            input: serde_json::json!({}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: InferenceUsage::default(),
    });
    llm.push_text_response("final", StopReason::EndTurn);
    let services = InterjectingServices {
        llm,
        drains: Mutex::new(VecDeque::from([vec!["after-tool".into()]])),
        drain_calls: AtomicUsize::new(0),
        invoked: Mutex::new(0),
    };
    let mut session = LinearCognitiveSession::new(HarnessConfig::default(), dependencies());

    session
        .run_streaming_turn(
            request("use tool"),
            &services,
            &NoopTurnEventSink,
            &RecordingStream::default(),
        )
        .await
        .unwrap();

    assert_eq!(*services.invoked.lock().unwrap(), 1);
    assert!(services.drain_calls.load(Ordering::SeqCst) >= 1);
    let log = services.llm.call_log.lock().unwrap();
    let second_call = &log[1];
    let interjection_index = second_call
        .iter()
        .position(|message| single_text(message) == Some("after-tool"))
        .unwrap();
    assert!(second_call[..interjection_index].iter().any(|message| {
        message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
    }));
}
