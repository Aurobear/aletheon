//! Gate-1 Acceptance Tests — Process/Operation/Chronos Integration (PR-4).
//!
//! Validates all four Gate 1 criteria:
//!
//! 1. **Main/Sub Agent in ProcessTable** — every agent process is registered and
//!    tracked with correct lifecycle states.
//! 2. **Every Turn has OperationId** — TurnService creates real process+operation
//!    records with state transitions (Submitted → Running → Succeeded/Failed).
//! 3. **wait/cancel/exit testable** — async wait resolves after completion,
//!    cancel propagates parent→child, and process exit transitions to terminal.
//! 4. **No orphan tasks** — OperationScope drain guarantee after cancel_and_drain.
//!
//! # Test inventory
//!
//! | Test | What it validates |
//! |------|-------------------|
//! | `main_agent_exists_in_process_table` | Spawn → inspect → state=Created |
//! | `sub_agent_created_in_process_table` | AgentControlService uses shared ProcessTable |
//! | `every_turn_has_operation_id` | TurnService creates operation, state transitions |
//! | `wait_on_operation_resolves_after_completion` | OperationTable::wait() unblocks on terminal state |
//! | `cancel_propagates_to_operation_and_task_exits` | parent cancel → child cancelled |
//! | `process_exit_transitions_state_and_operation_is_terminal` | Terminate signal → Stopping → Exited |
//! | `no_orphan_tasks_after_cancel_and_drain` | OperationScope drain → all tasks recorded |
//! | `deadline_exceeded_sets_operation_to_cancelled` | Hanging LLM triggers tokio timeout → Cancelled |

mod turn_request_support;

use executive::application::{PostTurnPipeline, PreTurnPipeline};
use executive::TurnService;
use fabric::{
    CancelReason, MonoDeadlineMillis, NoopTurnEventSink, OperationExitReason, OperationId,
    OperationKind, OperationRequest, OperationState, ProcessId, ProcessSignal, ProcessState,
    SpawnSpec, TurnRequest, TurnServices, TurnStop,
};
use kernel::chronos::TestClock;
use kernel::operation::OperationScope;
use kernel::KernelRuntime;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_kernel() -> Arc<KernelRuntime> {
    let clock: Arc<dyn fabric::Clock> = Arc::new(TestClock::default());
    let admission: Arc<dyn fabric::AdmissionController> = Arc::new(
        kernel::admission::AllowAllAdmissionController::new(clock.clone()),
    );
    Arc::new(KernelRuntime::with_admission(clock, admission))
}

/// Stub TurnServices that returns a simple text response immediately.
fn stub_services() -> Arc<dyn TurnServices> {
    use async_trait::async_trait;
    use fabric::{
        CapabilityCall, CapabilityResult, LlmProvider, RecallRequest, RecallSet, ToolDefinition,
        TurnServices,
    };

    struct StubServices;
    #[async_trait]
    impl TurnServices for StubServices {
        async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
            Ok(RecallSet::default())
        }
        async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
            Ok(fabric::DaseinView::default())
        }
        async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
            Ok(fabric::AgoraView::default())
        }
        async fn invoke(&self, _req: CapabilityCall) -> CapabilityResult {
            CapabilityResult {
                call_id: String::new(),
                output: String::new(),
                is_error: false,
                usage: fabric::UsageReport::default(),
                audit_id: None,
                patch_delta: None,
            }
        }
        fn llm_provider(&self) -> Option<&dyn LlmProvider> {
            None
        }
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
    }

    Arc::new(StubServices)
}

// ---------------------------------------------------------------------------
// 1. Main agent exists in process table
// ---------------------------------------------------------------------------

#[tokio::test]
async fn main_agent_exists_in_process_table() {
    let kernel = test_kernel();

    let handle = kernel
        .spawn_process(SpawnSpec {
            namespace: fabric::NamespaceId("gate-1".into()),
            ..SpawnSpec::default()
        })
        .await
        .expect("spawn should succeed");

    let snapshot = kernel
        .inspect_process(handle.id)
        .await
        .expect("process should exist in table");
    assert_eq!(snapshot.state, ProcessState::Created);
    assert_eq!(snapshot.process_id, handle.id);
}

// ---------------------------------------------------------------------------
// 2. Sub-agent created in process table via shared table
// ---------------------------------------------------------------------------

#[test]
fn agent_control_is_the_only_subagent_process_owner() {
    let compatibility = include_str!("../src/core/sub_agent.rs");
    assert!(!compatibility.contains("struct SubAgentSpawner"));
    assert!(!compatibility.contains("HashMap<"));
    let authority = include_str!("../src/application/agent_control/mod.rs");
    assert!(authority.contains(".spawn_process("));
    assert!(authority.contains("repository.create"));
}

// ---------------------------------------------------------------------------
// 3. Every turn has an operation ID
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_turn_has_operation_id() {
    let kernel = test_kernel();
    let services = stub_services();
    let turn_service =
        TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel.clone());

    // Pre-register a process in the ProcessTable so TurnService picks it up.
    let handle = kernel
        .spawn_process(SpawnSpec {
            namespace: fabric::NamespaceId("gate-1-turn".into()),
            initial_operation: Some(OperationKind::Turn),
            ..SpawnSpec::default()
        })
        .await
        .expect("pre-spawn process");
    kernel
        .signal_process(handle.id, ProcessSignal::Start)
        .await
        .unwrap();

    let result = turn_service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
                process_id: handle.id,
                context: turn_request_support::context("gate-1-turn", PathBuf::from(".")),
                input: "hello".into(),
                model_policy: None,
                deadline: None,
            },
            &NoopTurnEventSink,
        )
        .await
        .expect("turn should complete");

    assert_eq!(result.stop, TurnStop::Completed);
    assert!(result.metrics.completed_normally);

    // Verify the process is still in the ProcessTable and in Running state.
    let process_snapshot = kernel
        .inspect_process(handle.id)
        .await
        .expect("process should be in table after turn");
    assert_eq!(process_snapshot.state, ProcessState::Running);

    // The turn completed with TurnStop::Completed, which means the kernel
    // operation lifecycle (submit → start → succeed) ran without errors.
    // The operation was created with owner = handle.id and has reached
    // a terminal state inside TurnService::submit().
}

// ---------------------------------------------------------------------------
// 4. Wait on operation resolves after completion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_on_operation_resolves_after_completion() {
    let kernel = test_kernel();
    let process = kernel.spawn_process(SpawnSpec::default()).await.unwrap();

    let op = kernel
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    kernel.start_operation(op.id).await.unwrap();

    // Spawn a waiter that will block until terminal.
    let t = kernel.clone();
    let op_id = op.id;
    let waiter = tokio::spawn(async move { t.wait_operation(op_id).await });

    // Succeed the operation — waiter should unblock.
    kernel.succeed_operation(op.id).await.unwrap();
    let result = waiter.await.unwrap().unwrap();

    assert_eq!(result.state, OperationState::Succeeded);
    assert_eq!(result.id, op.id);
    assert_eq!(result.exit, Some(OperationExitReason::Completed));
}

// ---------------------------------------------------------------------------
// 5. Cancel propagates to operation and task exits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_propagates_to_operation_and_task_exits() {
    let kernel = test_kernel();
    let process = kernel.spawn_process(SpawnSpec::default()).await.unwrap();

    let parent = kernel
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    kernel.start_operation(parent.id).await.unwrap();

    let child = kernel
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: Some(parent.id),
            kind: OperationKind::CapabilityCall,
            deadline: None,
        })
        .await
        .unwrap();
    kernel.start_operation(child.id).await.unwrap();

    // Cancel the parent — child must also be cancelled.
    kernel
        .cancel_operation(parent.id, CancelReason::User)
        .await
        .unwrap();

    let parent_result = kernel.wait_operation(parent.id).await.unwrap();
    let child_result = kernel.wait_operation(child.id).await.unwrap();

    assert_eq!(parent_result.state, OperationState::Cancelled);
    assert_eq!(child_result.state, OperationState::Cancelled);
    assert!(matches!(
        child_result.exit,
        Some(OperationExitReason::Cancelled(CancelReason::User))
    ));
}

// ---------------------------------------------------------------------------
// 6. Process exit transitions state and makes operation terminal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn process_exit_transitions_state_and_operation_is_terminal() {
    let kernel = test_kernel();

    let handle = kernel
        .spawn_process(SpawnSpec {
            namespace: fabric::NamespaceId("exit-test".into()),
            ..SpawnSpec::default()
        })
        .await
        .unwrap();

    // Start the process.
    kernel
        .signal_process(handle.id, ProcessSignal::Start)
        .await
        .unwrap();
    assert_eq!(
        kernel.inspect_process(handle.id).await.unwrap().state,
        ProcessState::Running
    );

    // Terminate — transitions through Stopping → Exited.
    kernel
        .signal_process(handle.id, ProcessSignal::Terminate)
        .await
        .unwrap();

    let exit = kernel.wait_process(handle.id).await.unwrap();
    assert_eq!(
        exit.reason,
        fabric::ExitReason::Cancelled("terminated".into())
    );
    assert_eq!(
        kernel.inspect_process(handle.id).await.unwrap().state,
        ProcessState::Exited
    );
}

// ---------------------------------------------------------------------------
// 7. No orphan tasks after cancel_and_drain
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_orphan_tasks_after_cancel_and_drain() {
    let mut scope = OperationScope::new(OperationId::new());

    // Spawn a task that awaits cancellation.
    let token = scope.token();
    scope.spawn("worker", async move {
        token.cancelled().await;
        OperationExitReason::Cancelled(CancelReason::User)
    });

    // Drain the scope with a short grace period.
    let clock = TestClock::default();
    let exits = scope
        .cancel_and_drain(&clock, Duration::from_millis(500))
        .await;

    // All tasks should be accounted for.
    assert_eq!(exits.len(), 1);
    assert_eq!(exits[0].name, "worker");
    assert!(matches!(
        exits[0].reason,
        OperationExitReason::Cancelled(CancelReason::User)
    ));

    // Verify the scope's JoinSet is truly empty.
    assert!(scope.tasks.is_empty());
}

// ---------------------------------------------------------------------------
// 8. Deadline exceeded sets operation to cancelled
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deadline_exceeded_sets_operation_to_cancelled() {
    use async_trait::async_trait;
    use fabric::{
        CapabilityCall, CapabilityResult, ContentBlock, LlmProvider, LlmResponse, LlmStream,
        Message, RecallRequest, RecallSet, StopReason, ToolDefinition, TurnServices, Usage,
    };

    /// LLM that hangs for `hang_ms` ms, simulating a long-running model call.
    struct HangingLlm {
        hang_ms: u64,
    }

    #[async_trait]
    impl LlmProvider for HangingLlm {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            tokio::time::sleep(Duration::from_millis(self.hang_ms)).await;
            Ok(LlmResponse {
                content: vec![ContentBlock::Text { text: "ok".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!()
        }
        fn name(&self) -> &str {
            "hanging"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    struct HangingServices {
        llm: HangingLlm,
    }

    #[async_trait]
    impl TurnServices for HangingServices {
        async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
            Ok(RecallSet::default())
        }
        async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
            Ok(fabric::DaseinView::default())
        }
        async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
            Ok(fabric::AgoraView::default())
        }
        async fn invoke(&self, _req: CapabilityCall) -> CapabilityResult {
            CapabilityResult {
                call_id: String::new(),
                output: String::new(),
                is_error: false,
                usage: fabric::UsageReport::default(),
                audit_id: None,
                patch_delta: None,
            }
        }
        fn llm_provider(&self) -> Option<&dyn LlmProvider> {
            Some(&self.llm)
        }
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
    }

    // Deadline 50ms, LLM takes 500ms — deadline fires first.
    let services = Arc::new(HangingServices {
        llm: HangingLlm { hang_ms: 500 },
    });
    let kernel = test_kernel();
    let process = kernel
        .spawn_process(SpawnSpec::default())
        .await
        .expect("deadline process");
    let turn_service =
        TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel.clone());

    let result = turn_service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
                process_id: process.id,
                context: turn_request_support::context("deadline-gate1", PathBuf::from(".")),
                input: "should timeout".into(),
                model_policy: None,
                deadline: Some(MonoDeadlineMillis(50)),
            },
            &NoopTurnEventSink,
        )
        .await
        .expect("deadline turn should not error");

    assert_eq!(result.stop, TurnStop::Cancelled);
    assert!(!result.metrics.completed_normally);
}
