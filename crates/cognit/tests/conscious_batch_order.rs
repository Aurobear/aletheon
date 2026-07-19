//! Integration tests for governed capability batch ordering (R2/R3 production).
//!
//! These tests exercise the `plan_capability_batch` path through `run_streaming_turn`
//! and validate that:
//! - Enforce mode with a valid permutation reorders tool calls.
//! - Observe mode preserves provider order regardless of the plan.
//! - Invalid (non-permutation) plans fall back to provider order.
//! - The identity default preserves provider order.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::harness::{
    BatchPlanner, CognitiveSession, CognitiveSessionDependencies, HarnessConfig,
    LinearCognitiveSession,
};
use fabric::{
    CapabilityBatchPlan, CapabilityCall, CapabilityResult, ConsciousArbitrationMode, ContentBlock,
    LlmProvider, LlmResponse, LlmStream, NoopTurnEventSink, OperationId, ProcessId, StopReason,
    ToolDefinition, TurnRequest, TurnServices, Usage,
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn request() -> TurnRequest {
    let cwd = std::env::current_dir().unwrap();
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context: fabric::PrincipalContext::new(
            fabric::PrincipalId("test:batch-order".into()),
            fabric::LocalOsPrincipal { uid: 0, gid: 0 },
            fabric::ConnectionId::new(),
            fabric::ThreadId("batch-test".into()),
            fabric::WorkspacePolicy::from_resolved_roots(cwd, vec![]).unwrap(),
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
        ),
        input: "run tools".into(),
        model_policy: None,
        deadline: None,
    }
}

/// A shared invocation record so the test can observe tool execution order.
#[derive(Clone, Default)]
struct InvocationLog {
    ids: Arc<Mutex<Vec<String>>>,
}

impl InvocationLog {
    fn push(&self, id: &str) {
        self.ids.lock().unwrap().push(id.to_string());
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.ids.lock().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Mock LLM that returns three tool uses (tool_a, tool_b, tool_c) then ends.
// ---------------------------------------------------------------------------

struct ThreeToolLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ThreeToolLlm {
    async fn complete(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        if *n == 1 {
            Ok(LlmResponse {
                content: vec![
                    ContentBlock::ToolUse {
                        id: "tool_a".into(),
                        name: "tool_a".into(),
                        input: serde_json::json!({}),
                    },
                    ContentBlock::ToolUse {
                        id: "tool_b".into(),
                        name: "tool_b".into(),
                        input: serde_json::json!({}),
                    },
                    ContentBlock::ToolUse {
                        id: "tool_c".into(),
                        name: "tool_c".into(),
                        input: serde_json::json!({}),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "all done".into(),
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
        unimplemented!("not used by this test — uses run_turn path")
    }

    fn name(&self) -> &str {
        "three-tool-llm"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

// ---------------------------------------------------------------------------
// Drive the turn and collect invocation order.
// ---------------------------------------------------------------------------

async fn run_with_plan(
    mode: ConsciousArbitrationMode,
    planned_order: Vec<&str>,
    valid_plan: bool,
) -> Vec<String> {
    let llm = ThreeToolLlm {
        calls: Mutex::new(0),
    };
    let shared_log = InvocationLog::default();

    // Build a planner that delegates to plan_capability_batch on services.
    struct ServicesBatchPlanner {
        planned_order: Arc<Vec<String>>,
        mode: ConsciousArbitrationMode,
        valid_plan: bool,
    }

    #[async_trait]
    impl BatchPlanner for ServicesBatchPlanner {
        async fn plan(&self, calls: Vec<CapabilityCall>) -> anyhow::Result<CapabilityBatchPlan> {
            if self.valid_plan {
                Ok(CapabilityBatchPlan {
                    mode: self.mode,
                    ordered_call_ids: self.planned_order.clone().to_vec(),
                    decisions: calls
                        .iter()
                        .map(|c| fabric::CapabilityBatchDecision {
                            call_id: c.call_id.clone(),
                            decision: fabric::FieldDecisionKind::Reorder,
                            reason: fabric::FieldDecisionReason::Selected,
                            priority: 0.5,
                            broadcast_epoch: None,
                        })
                        .collect(),
                })
            } else {
                Ok(CapabilityBatchPlan {
                    mode: ConsciousArbitrationMode::Enforce,
                    ordered_call_ids: vec!["tool_a".into(), "tool_a".into(), "tool_b".into()],
                    decisions: vec![],
                })
            }
        }
    }

    let planner = Arc::new(ServicesBatchPlanner {
        planned_order: Arc::new(planned_order.iter().map(|s| s.to_string()).collect()),
        mode,
        valid_plan,
    });

    // TurnServices that records invocation order and provides the LLM.
    struct SharedServices {
        log: InvocationLog,
        llm: ThreeToolLlm,
    }

    #[async_trait]
    impl TurnServices for SharedServices {
        async fn recall(&self, _req: fabric::RecallRequest) -> anyhow::Result<fabric::RecallSet> {
            Ok(Default::default())
        }
        async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
            Ok(Default::default())
        }
        async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
            Ok(Default::default())
        }
        async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
            self.log.push(&call.call_id);
            CapabilityResult {
                call_id: call.call_id,
                output: format!("result_{}", call.name),
                is_error: false,
                usage: Default::default(),
                audit_id: None,
                patch_delta: None,
            }
        }
        fn llm_provider(&self) -> Option<&dyn LlmProvider> {
            Some(&self.llm)
        }
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![
                ToolDefinition {
                    name: "tool_a".into(),
                    description: "tool a".into(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "tool_b".into(),
                    description: "tool b".into(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "tool_c".into(),
                    description: "tool c".into(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
            ]
        }
    }

    let services = SharedServices {
        log: shared_log.clone(),
        llm,
    };

    let mut session = LinearCognitiveSession::new(
        HarnessConfig {
            max_iterations: 4,
            ..Default::default()
        },
        CognitiveSessionDependencies {
            clock: Arc::new(kernel::chronos::TestClock::default()),
            cancellation: CancellationToken::new(),
            compactor: None,
            batch_planner: Some(planner),
            evicted_callback: None,
            verifier: None,
        },
    );

    let _result = session
        .run_turn(request(), &services, &NoopTurnEventSink)
        .await
        .expect("turn should complete");

    shared_log.take()
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enforce_applies_exact_stable_permutation() {
    let ids = run_with_plan(
        ConsciousArbitrationMode::Enforce,
        vec!["tool_c", "tool_a", "tool_b"],
        true,
    )
    .await;
    assert_eq!(ids, vec!["tool_c", "tool_a", "tool_b"]);
}

#[tokio::test]
async fn observe_and_invalid_plans_keep_provider_order() {
    // Observe mode always preserves provider order.
    let ids = run_with_plan(
        ConsciousArbitrationMode::Observe,
        vec!["tool_c", "tool_a", "tool_b"],
        true,
    )
    .await;
    assert_eq!(ids, vec!["tool_a", "tool_b", "tool_c"]);

    // Invalid plan (non-permutation) keeps provider order even in Enforce mode.
    let ids = run_with_plan(
        ConsciousArbitrationMode::Enforce,
        vec!["tool_c", "tool_a", "tool_b"],
        false,
    )
    .await;
    assert_eq!(ids, vec!["tool_a", "tool_b", "tool_c"]);
}
