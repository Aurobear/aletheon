use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use aletheon_kernel::chronos::TestClock;
use async_trait::async_trait;
use executive::r#impl::runtime::{
    AgentProfileRegistry, NativeCognitRuntime, NativeCognitRuntimeResources, ResolvedAgentProfile,
};
use executive::service::agent_control::{
    AgentContextProjection, AgentEventSink, AgentRuntimeEvent, AgentRuntimeInbox,
    AgentRuntimeInput, AgentRuntimeLauncher,
};
use executive::service::harness_factory::LinearCognitiveSessionFactory;
use executive::service::{CapabilityExecutionContext, CapabilityService};
use fabric::{
    AgentApprovalPolicy, AgentBudget, AgentContextFork, AgentControlErrorKind, AgentHandle,
    AgentId, AgentMessageKind, AgentMessagePayload, AgentProfile, AgentProfileId,
    AgentSpawnRequest, CapabilityCall, CapabilityResult, ContentBlock, LlmProvider, LlmResponse,
    LlmStream, OperationId, ParentRestriction, ProcessId, RiskTier, RuntimeId, StopReason,
    ToolDefinition, Usage, AGENT_MESSAGE_SCHEMA_V1,
};
use tokio_util::sync::CancellationToken;

struct ScriptedLlm {
    responses: Mutex<VecDeque<anyhow::Result<LlmResponse>>>,
    seen: Mutex<Vec<(Vec<fabric::Message>, Vec<ToolDefinition>)>>,
    block: bool,
}

impl ScriptedLlm {
    fn new(responses: Vec<anyhow::Result<LlmResponse>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            seen: Mutex::new(Vec::new()),
            block: false,
        })
    }

    fn blocked() -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
            block: true,
        })
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn complete(
        &self,
        messages: &[fabric::Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.seen
            .lock()
            .unwrap()
            .push((messages.to_vec(), tools.to_vec()));
        if self.block {
            std::future::pending().await
        } else {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted response")
        }
    }

    async fn complete_stream(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unreachable!("linear harness uses complete")
    }

    fn name(&self) -> &str {
        "scripted/model"
    }

    fn max_context_length(&self) -> usize {
        128_000
    }
}

#[derive(Default)]
struct RecordingCapability {
    calls: Mutex<Vec<(Option<CapabilityExecutionContext>, CapabilityCall)>>,
    block: bool,
}

#[async_trait]
impl CapabilityService for RecordingCapability {
    async fn invoke(
        &self,
        context: Option<CapabilityExecutionContext>,
        call: CapabilityCall,
        _cancel: CancellationToken,
    ) -> CapabilityResult {
        self.calls.lock().unwrap().push((context, call.clone()));
        if self.block {
            std::future::pending().await
        }
        CapabilityResult {
            call_id: call.call_id,
            output: "tool-ok".into(),
            is_error: false,
            usage: fabric::UsageReport::default(),
            audit_id: None,
        }
    }
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<AgentRuntimeEvent>>);

#[async_trait]
impl AgentEventSink for RecordingEvents {
    async fn emit(&self, event: AgentRuntimeEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn response(content: Vec<ContentBlock>, stop_reason: StopReason) -> anyhow::Result<LlmResponse> {
    Ok(LlmResponse {
        content,
        stop_reason,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 4,
        },
        cache_hit_tokens: 0,
        cache_miss_tokens: 0,
    })
}

fn profile() -> AgentProfile {
    AgentProfile {
        id: AgentProfileId("worker".into()),
        system_prompt: "PROFILE SYSTEM".into(),
        model: "scripted/model".into(),
        allowed_tools: vec!["echo".into()],
        max_iterations: 4,
        max_input_tokens: 8_000,
        max_output_tokens: 1_000,
        max_tool_calls: 4,
        max_elapsed_ms: 5_000,
        profile_name: "worker".into(),
        risk_tier: RiskTier::Sandboxed,
        approval_policy: AgentApprovalPolicy::PromptUser,
        tool_timeout_ms: 5_000,
        inheritable: false,
        parent_restriction: ParentRestriction::None,
    }
}

fn input(cancel: CancellationToken) -> AgentRuntimeInput {
    let root = AgentId::new();
    let process = ProcessId::new();
    let request = AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("worker".into()),
        runtime_id: RuntimeId("native-cognit".into()),
        trusted_workspace: None,
        task: "perform the task".into(),
        context: AgentContextFork::SelectedProjection {
            items: vec!["reference context".into()],
        },
        broadcast_refs: vec![],
        allowed_tools: vec!["echo".into()],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 4_000,
            max_output_tokens: 500,
            max_tool_calls: 2,
            max_elapsed_ms: 2_000,
            max_cost_usd: None,
            max_depth: 2,
        },
    };
    AgentRuntimeInput {
        workspace: None,
        context: AgentContextProjection::from_fork(&request.context).unwrap(),
        memory_context: mnemosyne::AgentMemoryContext::verified(
            process,
            root,
            fabric::AgentTaskId("test-task".into()),
            "sha256:test-projection",
        )
        .unwrap(),
        handle: AgentHandle {
            agent_id: root,
            root_agent_id: root,
            parent_agent_id: None,
            process_id: process,
            operation_id: OperationId::new(),
            runtime_id: request.runtime_id.clone(),
            profile_id: request.profile_id.clone(),
        },
        workspace_id: fabric::AgoraSpaceId(format!("agent:{}", root.0)),
        root_workspace_id: fabric::AgoraSpaceId(format!("root:{}", root.0)),
        root_process_id: process,
        inbox: AgentRuntimeInbox::empty(),
        request,
        cancellation: cancel,
        background_cancellations: std::collections::HashMap::new(),
        background_registrations: std::collections::HashMap::new(),
    }
}

fn runtime(llm: Arc<ScriptedLlm>, capability: Arc<RecordingCapability>) -> NativeCognitRuntime {
    let clock = Arc::new(TestClock::default());
    let profiles = Arc::new(AgentProfileRegistry::default());
    profiles
        .register(ResolvedAgentProfile {
            profile: profile(),
            llm,
            tools: vec![ToolDefinition {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }],
        })
        .unwrap();
    NativeCognitRuntime::new(NativeCognitRuntimeResources {
        sessions: Arc::new(LinearCognitiveSessionFactory::new(
            cognit::harness::HarnessConfig::default(),
            clock.clone(),
        )),
        capabilities: capability,
        profiles,
        clock,
        conscious_actions: None,
        conscious_candidates: None,
    })
}

#[tokio::test]
async fn final_text_uses_profile_and_labelled_projection() {
    let llm = ScriptedLlm::new(vec![response(
        vec![ContentBlock::Text {
            text: "finished".into(),
        }],
        StopReason::EndTurn,
    )]);
    let events = Arc::new(RecordingEvents::default());
    let result = runtime(llm.clone(), Arc::new(RecordingCapability::default()))
        .launch(input(CancellationToken::new()), events.clone())
        .await
        .unwrap();
    assert_eq!(result.output, "finished");
    let seen = llm.seen.lock().unwrap();
    let rendered = format!("{:?}", seen[0].0);
    assert!(rendered.contains("PROFILE SYSTEM"));
    assert!(rendered.contains("untrusted reference data"));
    assert!(rendered.contains("reference context"));
    assert_eq!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AgentRuntimeEvent::Terminal { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn queued_start_turn_input_runs_in_the_same_cognit_session() {
    let llm = ScriptedLlm::new(vec![
        response(
            vec![ContentBlock::Text {
                text: "first turn".into(),
            }],
            StopReason::EndTurn,
        ),
        response(
            vec![ContentBlock::Text {
                text: "second turn".into(),
            }],
            StopReason::EndTurn,
        ),
    ]);
    let (sender, inbox) = AgentRuntimeInbox::bounded_channel(2).unwrap();
    sender
        .send(AgentMessagePayload {
            schema_version: AGENT_MESSAGE_SCHEMA_V1,
            kind: AgentMessageKind::Input,
            content: "follow-up input".into(),
            start_turn: true,
            correlation_id: None,
            deadline_mono_ms: None,
        })
        .await
        .unwrap();
    let mut runtime_input = input(CancellationToken::new());
    runtime_input.inbox = inbox;
    let result = runtime(llm.clone(), Arc::new(RecordingCapability::default()))
        .launch(runtime_input, Arc::new(RecordingEvents::default()))
        .await
        .unwrap();
    assert_eq!(result.output, "second turn");
    let seen = llm.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(format!("{:?}", seen[1].0).contains("follow-up input"));
}

#[tokio::test]
async fn tool_calls_use_persisted_lifecycle_context_and_evidence() {
    let llm = ScriptedLlm::new(vec![
        response(
            vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "echo".into(),
                input: serde_json::json!({"value": 1}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text {
                text: "after tool".into(),
            }],
            StopReason::EndTurn,
        ),
    ]);
    let capability = Arc::new(RecordingCapability::default());
    let expected = input(CancellationToken::new());
    let result = runtime(llm, capability.clone())
        .launch(expected.clone(), Arc::new(RecordingEvents::default()))
        .await
        .unwrap();
    let calls = capability.calls.lock().unwrap();
    let context = calls[0].0.as_ref().unwrap();
    assert_eq!(context.process_id, expected.handle.process_id);
    assert_eq!(context.operation_id, expected.handle.operation_id);
    assert_eq!(result.evidence.len(), 1);
}

#[tokio::test]
async fn unknown_profile_and_disallowed_tool_fail_before_provider_call() {
    let llm = ScriptedLlm::new(vec![]);
    let runtime = runtime(llm.clone(), Arc::new(RecordingCapability::default()));
    let mut unknown = input(CancellationToken::new());
    unknown.request.profile_id = AgentProfileId("missing".into());
    assert_eq!(
        runtime
            .launch(unknown, Arc::new(RecordingEvents::default()))
            .await
            .unwrap_err()
            .kind,
        AgentControlErrorKind::NotFound
    );
    let mut disallowed = input(CancellationToken::new());
    disallowed.request.allowed_tools.push("shell".into());
    assert_eq!(
        runtime
            .launch(disallowed, Arc::new(RecordingEvents::default()))
            .await
            .unwrap_err()
            .kind,
        AgentControlErrorKind::Forbidden
    );
    assert!(llm.seen.lock().unwrap().is_empty());
}

#[test]
fn profile_registration_rejects_model_mismatch_before_session_creation() {
    let profiles = AgentProfileRegistry::default();
    let mut mismatched = profile();
    mismatched.model = "different/model".into();
    let error = profiles
        .register(ResolvedAgentProfile {
            profile: mismatched,
            llm: ScriptedLlm::new(vec![]),
            tools: vec![ToolDefinition {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }],
        })
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
    assert!(error.message.contains("does not match"));
}

#[tokio::test]
async fn cancellation_interrupts_provider_and_emits_one_terminal() {
    let llm = ScriptedLlm::blocked();
    let runtime = runtime(llm, Arc::new(RecordingCapability::default()));
    let cancellation = CancellationToken::new();
    let events = Arc::new(RecordingEvents::default());
    let task = tokio::spawn({
        let events = events.clone();
        let input = input(cancellation.clone());
        async move { runtime.launch(input, events).await }
    });
    tokio::task::yield_now().await;
    cancellation.cancel();
    assert_eq!(
        task.await.unwrap().unwrap_err().kind,
        AgentControlErrorKind::Terminal
    );
    assert_eq!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AgentRuntimeEvent::Terminal { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn multiple_tools_are_governed_and_unknown_tools_never_reach_capability() {
    let llm = ScriptedLlm::new(vec![
        response(
            vec![
                ContentBlock::ToolUse {
                    id: "call-1".into(),
                    name: "echo".into(),
                    input: serde_json::json!({"value": 1}),
                },
                ContentBlock::ToolUse {
                    id: "call-2".into(),
                    name: "echo".into(),
                    input: serde_json::json!({"value": 2}),
                },
            ],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text {
                text: "two tools complete".into(),
            }],
            StopReason::EndTurn,
        ),
    ]);
    let capability = Arc::new(RecordingCapability::default());
    let result = runtime(llm, capability.clone())
        .launch(
            input(CancellationToken::new()),
            Arc::new(RecordingEvents::default()),
        )
        .await
        .unwrap();
    assert_eq!(capability.calls.lock().unwrap().len(), 2);
    assert_eq!(result.evidence.len(), 2);

    let llm = ScriptedLlm::new(vec![
        response(
            vec![ContentBlock::ToolUse {
                id: "bad".into(),
                name: "shell".into(),
                input: serde_json::json!({}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text {
                text: "handled rejection".into(),
            }],
            StopReason::EndTurn,
        ),
    ]);
    let capability = Arc::new(RecordingCapability::default());
    let result = runtime(llm, capability.clone())
        .launch(
            input(CancellationToken::new()),
            Arc::new(RecordingEvents::default()),
        )
        .await
        .unwrap();
    assert!(capability.calls.lock().unwrap().is_empty());
    assert!(result.evidence[0].content.contains("not allowed"));
}

#[tokio::test]
async fn provider_failure_and_iteration_exhaustion_are_bounded_runtime_errors() {
    let llm = ScriptedLlm::new(vec![Err(anyhow::anyhow!("provider unavailable"))]);
    let error = runtime(llm, Arc::new(RecordingCapability::default()))
        .launch(
            input(CancellationToken::new()),
            Arc::new(RecordingEvents::default()),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::Runtime);
    assert!(error.message.contains("provider unavailable"));
    assert!(error.message.len() <= 4 * 1024);

    let llm = ScriptedLlm::new(vec![response(
        vec![ContentBlock::ToolUse {
            id: "repeat".into(),
            name: "echo".into(),
            input: serde_json::json!({}),
        }],
        StopReason::ToolUse,
    )]);
    let mut limited = profile();
    limited.max_iterations = 1;
    let clock = Arc::new(TestClock::default());
    let profiles = Arc::new(AgentProfileRegistry::default());
    profiles
        .register(ResolvedAgentProfile {
            profile: limited,
            llm,
            tools: vec![ToolDefinition {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }],
        })
        .unwrap();
    let runtime = NativeCognitRuntime::new(NativeCognitRuntimeResources {
        sessions: Arc::new(LinearCognitiveSessionFactory::new(
            cognit::harness::HarnessConfig::default(),
            clock.clone(),
        )),
        capabilities: Arc::new(RecordingCapability::default()),
        profiles,
        clock,
        conscious_actions: None,
        conscious_candidates: None,
    });
    let error = runtime
        .launch(
            input(CancellationToken::new()),
            Arc::new(RecordingEvents::default()),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::Runtime);
}

#[tokio::test]
async fn cancellation_interrupts_a_live_tool_call() {
    let llm = ScriptedLlm::new(vec![
        response(
            vec![ContentBlock::ToolUse {
                id: "blocked-tool".into(),
                name: "echo".into(),
                input: serde_json::json!({}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text {
                text: "must still cancel".into(),
            }],
            StopReason::EndTurn,
        ),
    ]);
    let capability = Arc::new(RecordingCapability {
        calls: Mutex::new(Vec::new()),
        block: true,
    });
    let runtime = runtime(llm, capability.clone());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn({
        let input = input(cancellation.clone());
        async move {
            runtime
                .launch(input, Arc::new(RecordingEvents::default()))
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while capability.calls.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    assert_eq!(
        task.await.unwrap().unwrap_err().kind,
        AgentControlErrorKind::Terminal
    );
}
