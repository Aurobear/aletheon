use std::sync::Arc;

use async_trait::async_trait;
use executive::application::harness_factory::CognitiveSessionFactory;
use executive::application::turn_coordinator::TurnExecution;
use executive::application::turn_policy::*;
use executive::application::{PostTurnPipeline, PreTurnPipeline};
use executive::composition::config::GrokHardeningConfig;
use executive::runtime::events::{EventReadFilter, SqliteEventSpine};
use executive::runtime::session::canonical_store::CanonicalSessionStore;
use executive::TurnService;
use fabric::{
    ItemPayload, OperationState, SessionId, TurnMetrics, TurnRequest, TurnResult, TurnStop,
};
use kernel::KernelRuntime;

fn request(session: &str, process_id: fabric::ProcessId) -> TurnRequest {
    TurnRequest {
        operation_id: fabric::OperationId::default(),
        process_id,
        context: turn_request_support::context(session, std::env::temp_dir()),
        input: "hello".into(),
        execution_target: fabric::ExecutionTargetSelection::default(),
        model_policy: None,
        deadline: None,
        requirements: Vec::new(),
        requested_task_kind: None,
        evaluation_contract: None,
    }
}

#[test]
fn policy_contains_all_mode_differences() {
    let daemon = TurnPolicy::daemon();
    let exec = TurnPolicy::exec();
    assert_eq!(daemon.persistence, PersistenceMode::Durable);
    assert_eq!(exec.persistence, PersistenceMode::Durable);
    assert_ne!(daemon.reviewer, exec.reviewer);
    assert_ne!(daemon.memory_eligible, exec.memory_eligible);
    assert_ne!(daemon.agora_available, exec.agora_available);
    assert_ne!(daemon.event_delivery, exec.event_delivery);
    assert_ne!(daemon.environment, exec.environment);
}

#[tokio::test]
async fn coordinator_owns_turn_operation_and_ordered_canonical_items() {
    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = executive::testing::turn_coordinator::compose_with_event_spine(
        kernel.clone(),
        read_store,
        event_spine.clone(),
        executive::composition::config::GrokHardeningConfig::default(),
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();
    let runner_store = store.clone();
    let mut submitted = request("success", process.id);
    submitted.execution_target = fabric::ExecutionTargetSelection::robot(
        "robot-1",
        fabric::types::embodiment::ExecutionEnvironment::Simulation,
        fabric::ExecutionTargetSource::UserCommand,
    )
    .unwrap();
    let result = coordinator
        .submit_with(
            submitted,
            &TurnPolicy::daemon(),
            move |request, _| async move {
                *capture.lock().await = Some(request.operation_id);
                let turn_id = request.context.turn_id.expect("coordinator turn id");
                let session_id = SessionId(request.context.thread_id.0.clone());
                runner_store
                    .append(
                        &session_id,
                        2,
                        fabric::ItemRecord {
                            schema_version: fabric::SESSION_SCHEMA_VERSION,
                            id: fabric::ItemId::new(),
                            session_id: session_id.clone(),
                            turn_id,
                            sequence: 2,
                            created_at_ms: 2,
                            payload: ItemPayload::TaskProjection {
                                fact: fabric::TaskProjectionFact::default(),
                            },
                        },
                    )
                    .await?;
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "answer".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![
                        ItemPayload::ToolCall {
                            call_id: "c".into(),
                            name: "tool".into(),
                            input: serde_json::json!({}),
                        },
                        ItemPayload::ToolResult {
                            call_id: "c".into(),
                            content: "ok".into(),
                            is_error: false,
                            permit_id: None,
                            audit_id: None,
                        },
                    ],
                    projection: None,
                    context_projection: Some(fabric::ContextProjectionReceipt {
                        space: fabric::AgoraSpaceId("session-a".into()),
                        broadcast_epoch: Some(fabric::BroadcastEpoch(2)),
                        workspace_version: Some(3),
                        dasein_version: fabric::dasein::SelfVersion(4),
                        content_ids: vec![fabric::ContentId(uuid::Uuid::from_u128(5))],
                    }),
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();
    assert_eq!(result.output, "answer");
    let operation = kernel
        .inspect_operation(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(operation.kind, fabric::OperationKind::Turn);
    assert_eq!(operation.state, OperationState::Succeeded);
    let items = store
        .load_items(&SessionId("success".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 6);
    assert!(matches!(
        &items[0].payload,
        ItemPayload::UserMessage {
            execution_target: fabric::ExecutionTargetSelection {
                target: fabric::ExecutionTarget::Robot { device_id, .. },
                source: fabric::ExecutionTargetSource::UserCommand,
            },
            ..
        } if device_id.0 == "robot-1"
    ));
    assert!(matches!(
        items[1].payload,
        ItemPayload::TaskProjection { .. }
    ));
    assert!(matches!(
        items[2].payload,
        ItemPayload::ContextProjection {
            broadcast_epoch: Some(2),
            workspace_version: Some(3),
            dasein_version: 4,
            ..
        }
    ));
    assert!(matches!(items[3].payload, ItemPayload::ToolCall { .. }));
    assert!(matches!(items[4].payload, ItemPayload::ToolResult { .. }));
    assert!(matches!(
        items[5].payload,
        ItemPayload::AssistantMessage { .. }
    ));
    let events = event_spine
        .read_tree(
            fabric::EventTreeId::for_root_session("success"),
            EventReadFilter {
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(events.len(), items.len() + 2);
    assert_eq!(
        events[0].schema.0,
        fabric::SchemaId::EVENT_SESSION_CREATED_V1
    );
    assert_eq!(
        events[1].schema.0,
        fabric::SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1
    );
    assert!(events
        .iter()
        .skip(2)
        .all(|event| event.schema.0 == fabric::SchemaId::TURN_EVENT_V1));
    for (index, event) in events.iter().enumerate() {
        assert_eq!(
            event.position.sequence,
            fabric::TreeSequence(index as u64 + 1)
        );
    }
}

#[tokio::test]
async fn general_robot_general_turn_starts_remain_ordered_and_target_scoped() {
    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
        kernel.clone(),
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let targets = [
        fabric::ExecutionTargetSelection::default(),
        fabric::ExecutionTargetSelection::robot(
            "robot-1",
            fabric::types::embodiment::ExecutionEnvironment::Simulation,
            fabric::ExecutionTargetSource::UserCommand,
        )
        .unwrap(),
        fabric::ExecutionTargetSelection::general(fabric::ExecutionTargetSource::UserCommand),
    ];

    for (index, target) in targets.iter().cloned().enumerate() {
        let mut turn = request("mixed-targets", process.id);
        turn.input = format!("turn-{index}");
        turn.execution_target = target;
        coordinator
            .submit_with(turn, &TurnPolicy::daemon(), move |_request, _| async move {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: format!("answer-{index}"),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: Vec::new(),
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            })
            .await
            .unwrap();
    }

    let items = store
        .load_items(&SessionId("mixed-targets".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 6);
    for (index, pair) in items.chunks_exact(2).enumerate() {
        assert_eq!(pair[0].turn_id, pair[1].turn_id);
        assert!(matches!(
            pair[1].payload,
            ItemPayload::AssistantMessage { .. }
        ));
        let ItemPayload::UserMessage {
            execution_target, ..
        } = &pair[0].payload
        else {
            panic!("turn {index} did not start with a UserMessage target fact")
        };
        assert_eq!(execution_target, &targets[index]);
    }
}

#[tokio::test]
async fn concurrent_general_and_robot_turns_do_not_share_target_or_settlement() {
    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
            kernel.clone(),
            Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
        ),
    );
    let store = coordinator.store();
    let general_process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let robot_process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let observed = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let mut general = request("concurrent-general", general_process.id);
    general.execution_target = fabric::ExecutionTargetSelection::default();
    let mut robot = request("concurrent-robot", robot_process.id);
    robot.execution_target = fabric::ExecutionTargetSelection::robot(
        "robot-1",
        fabric::types::embodiment::ExecutionEnvironment::Simulation,
        fabric::ExecutionTargetSource::TrustedClient,
    )
    .unwrap();

    let submit = |request: TurnRequest| {
        let coordinator = coordinator.clone();
        let barrier = barrier.clone();
        let observed = observed.clone();
        async move {
            coordinator
                .submit_with(
                    request,
                    &TurnPolicy::daemon(),
                    move |request, _| async move {
                        observed
                            .lock()
                            .await
                            .push((request.operation_id, request.execution_target.clone()));
                        barrier.wait().await;
                        Ok(TurnExecution {
                            result: TurnResult {
                                output: request.context.thread_id.0,
                                stop: TurnStop::Completed,
                                failure: None,
                                usage: Default::default(),
                                metrics: TurnMetrics {
                                    completed_normally: true,
                                    ..Default::default()
                                },
                            },
                            items: Vec::new(),
                            projection: None,
                            context_projection: None,
                            evaluation_artifacts: Default::default(),
                        })
                    },
                )
                .await
        }
    };
    let (general_result, robot_result) = tokio::join!(submit(general), submit(robot));
    assert!(general_result.is_ok());
    assert!(robot_result.is_ok());

    for (session, expected) in [
        (
            "concurrent-general",
            fabric::ExecutionTargetSelection::default(),
        ),
        (
            "concurrent-robot",
            fabric::ExecutionTargetSelection::robot(
                "robot-1",
                fabric::types::embodiment::ExecutionEnvironment::Simulation,
                fabric::ExecutionTargetSource::TrustedClient,
            )
            .unwrap(),
        ),
    ] {
        let items = store
            .load_items(&SessionId(session.into()), None)
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0].payload,
            ItemPayload::UserMessage { execution_target, .. } if execution_target == &expected
        ));
        assert_eq!(items[0].turn_id, items[1].turn_id);
    }
    let observed = observed.lock().await;
    assert_eq!(observed.len(), 2);
    assert_ne!(observed[0].0, observed[1].0);
    assert_ne!(observed[0].1, observed[1].1);
    for (operation_id, _) in observed.iter() {
        assert_eq!(
            kernel.inspect_operation(*operation_id).await.unwrap().state,
            OperationState::Succeeded
        );
    }
}

#[tokio::test]
async fn failure_is_terminal_and_remains_replayable() {
    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
        kernel.clone(),
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let result = coordinator
        .submit_with(
            request("failed", process.id),
            &TurnPolicy::exec(),
            |_request, _| async { anyhow::bail!("model unavailable") },
        )
        .await;
    assert!(result.is_err());
    let items = store
        .load_items(&SessionId("failed".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::SystemNotice { .. }));
}

#[tokio::test]
async fn compatibility_cancel_reaches_active_turn_for_principal() {
    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
            kernel.clone(),
            Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let request = request("cancelled", process.id);
    let principal_id = request.context.principal_id.clone();
    let running = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .submit_with(
                    request,
                    &TurnPolicy::daemon(),
                    |_request, cancel| async move {
                        cancel.cancelled().await;
                        anyhow::bail!("cancelled")
                    },
                )
                .await
        })
    };

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while coordinator.active_turn_count().await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(
        coordinator.cancel_active_for_principal(&principal_id).await,
        1
    );
    // Cancellation is authoritative: the active turn settles as a typed
    // cancelled result, not a generic error.
    let settled = running.await.unwrap();
    assert!(
        matches!(settled, Ok(turn) if turn.stop == TurnStop::Cancelled),
        "cancel must settle the active turn with TurnStop::Cancelled"
    );
    assert_eq!(coordinator.active_turn_count().await, 0);
}

struct SeedCapturingFactory(Arc<tokio::sync::Mutex<Vec<usize>>>);

#[async_trait]
impl CognitiveSessionFactory for SeedCapturingFactory {
    async fn create(
        &self,
        _session: &fabric::SessionRecord,
        _policy: &TurnPolicy,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(SeedCapturingSession(self.0.clone())))
    }
}

struct SeedCapturingSession(Arc<tokio::sync::Mutex<Vec<usize>>>);

#[async_trait]
impl cognit::harness::CognitiveSession for SeedCapturingSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn fabric::TurnServices,
        _events: &dyn fabric::TurnEventSink,
    ) -> Result<TurnResult, cognit::CognitError> {
        self.0
            .lock()
            .await
            .push(services.seed_messages(&request).len());
        Ok(TurnResult {
            output: format!("answer: {}", request.input),
            stop: TurnStop::Completed,
            failure: None,
            usage: Default::default(),
            metrics: TurnMetrics {
                completed_normally: true,
                ..Default::default()
            },
        })
    }
}

struct EmptyServices;

#[async_trait]
impl fabric::TurnServices for EmptyServices {
    async fn recall(&self, _request: fabric::RecallRequest) -> anyhow::Result<fabric::RecallSet> {
        Ok(Default::default())
    }
    async fn dasein_view(&self, _process: fabric::ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(Default::default())
    }
    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(Default::default())
    }
    async fn invoke(&self, call: fabric::CapabilityCall) -> fabric::CapabilityResult {
        fabric::CapabilityResult {
            call_id: call.call_id,
            output: "unused".into(),
            is_error: true,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }
}

#[tokio::test]
async fn daemon_then_exec_restart_projects_prior_canonical_context() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sessions.db");
    let captures = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    for policy in [TurnPolicy::daemon(), TurnPolicy::exec()] {
        let kernel = Arc::new(KernelRuntime::new());
        let coordinator = Arc::new(
            executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
                kernel.clone(),
                Arc::new(CanonicalSessionStore::open(&db).unwrap()),
            ),
        );
        let process = kernel
            .spawn_process(fabric::SpawnSpec::default())
            .await
            .unwrap();
        TurnService::new(
            Arc::new(EmptyServices),
            PreTurnPipeline,
            PostTurnPipeline,
            kernel,
        )
        .with_coordinator(coordinator)
        .with_policy(policy)
        .with_session_factory(Arc::new(SeedCapturingFactory(captures.clone())))
        .submit(request("restart", process.id), &fabric::NoopTurnEventSink)
        .await
        .unwrap();
    }
    let captures = captures.lock().await.clone();
    assert_eq!(captures[0], 0);
    assert_eq!(
        captures[1], 2,
        "second mode must receive prior user+assistant context"
    );
}
mod turn_request_support;

#[tokio::test]
async fn deadline_cancels_a_never_ending_turn_and_settles_exactly_once() {
    use executive::application::turn_coordinator::TurnExecution;
    use executive::application::turn_policy::*;
    use fabric::TurnStop;

    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = executive::testing::turn_coordinator::compose_with_event_spine(
        kernel.clone(),
        read_store,
        event_spine.clone(),
        GrokHardeningConfig::default(),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let mut turn = request("deadline-turn", process.id);
    turn.deadline = Some(fabric::MonoDeadlineMillis(60));
    let captured_operation = Arc::new(tokio::sync::Mutex::new(None::<fabric::OperationId>));
    let captured = captured_operation.clone();

    let result = coordinator
        .submit_with(
            turn,
            &TurnPolicy::daemon(),
            move |request, cancel| async move {
                *captured.lock().await = Some(request.operation_id);
                // Never-ending runner: loop until the deadline cancels the token.
                let mut iterations = 0u32;
                loop {
                    if cancel.is_cancelled() {
                        return Ok(TurnExecution {
                            result: TurnResult {
                                output: "cancelled by deadline".into(),
                                stop: TurnStop::Cancelled,
                                failure: None,
                                usage: Default::default(),
                                metrics: TurnMetrics {
                                    completed_normally: false,
                                    ..Default::default()
                                },
                            },
                            items: Vec::new(),
                            projection: None,
                            context_projection: None,
                            evaluation_artifacts: Default::default(),
                        });
                    }
                    iterations += 1;
                    if iterations > 100_000 {
                        panic!("deadline watchdog never cancelled the never-ending runner");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            },
        )
        .await;

    assert!(matches!(result, Ok(turn) if turn.stop == TurnStop::Cancelled));
    assert_eq!(
        coordinator.active_turn_count().await,
        0,
        "active index must be empty after deadline settlement"
    );
    let operation_id = captured_operation
        .lock()
        .await
        .expect("runner captured operation id");
    let record = kernel
        .inspect_operation(operation_id)
        .await
        .expect("operation should exist");
    assert!(
        matches!(
            record.state,
            fabric::OperationState::Cancelled | fabric::OperationState::Failed
        ),
        "kernel operation must be terminal after deadline, got {:?}",
        record.state
    );
    let metrics = kernel::operation::operation_scope_metrics();
    assert_eq!(
        metrics.active_scopes, 0,
        "no active scopes after deadline settlement"
    );
    assert_eq!(
        metrics.active_resources, 0,
        "no active resources after deadline settlement"
    );
}

#[tokio::test]
async fn deadline_forcibly_drops_an_uncooperative_runner_after_bounded_grace() {
    use executive::application::turn_coordinator::TurnExecution;
    use executive::application::turn_policy::*;
    use fabric::{OperationExitReason, TurnStop};

    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let clock = Arc::new(kernel::chronos::TestClock::default());
    let timer = Arc::new(kernel::chronos::TestTimer::new(clock));
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            read_store,
            event_spine,
            GrokHardeningConfig::default(),
        )
        .with_shared_test_timer(timer.clone()),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let mut turn = request("uncooperative-deadline", process.id);
    turn.deadline = Some(fabric::MonoDeadlineMillis(60));
    let captured_operation = Arc::new(tokio::sync::Mutex::new(None));
    let captured = captured_operation.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let submitted = coordinator.clone();
    let handle = tokio::spawn(async move {
        submitted
            .submit_with(
                turn,
                &TurnPolicy::daemon(),
                move |request, _cancel| async move {
                    *captured.lock().await = Some(request.operation_id);
                    let _ = started_tx.send(());
                    std::future::pending::<anyhow::Result<TurnExecution>>().await
                },
            )
            .await
    });

    started_rx.await.expect("uncooperative runner started");
    timer.advance(60);
    // The first advance wakes the deadline. Subsequent bounded advances wake
    // the coordinator's five-second abort-drain grace regardless of the exact
    // scheduling point at which that sleeper is registered.
    for _ in 0..8 {
        tokio::task::yield_now().await;
        if handle.is_finished() {
            break;
        }
        timer.advance(1_000);
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("bounded abort protocol should finish")
        .expect("submit task should not panic")
        .expect("deadline cancellation should be a terminal result");
    assert_eq!(result.stop, TurnStop::Cancelled);
    assert_eq!(coordinator.active_turn_count().await, 0);

    let operation_id = captured_operation.lock().await.unwrap();
    let operation = kernel.inspect_operation(operation_id).await.unwrap();
    assert_eq!(operation.state, fabric::OperationState::Cancelled);
    assert_eq!(
        operation.exit,
        Some(OperationExitReason::Cancelled(
            fabric::CancelReason::DeadlineExceeded
        ))
    );
}

#[tokio::test]
async fn user_cancel_on_deadline_bearing_turn_preserves_user_reason() {
    use executive::application::turn_coordinator::TurnExecution;
    use executive::application::turn_policy::*;
    use fabric::{OperationExitReason, TurnStop};

    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            read_store,
            event_spine,
            GrokHardeningConfig::default(),
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let mut turn = request("user-cancel-with-deadline", process.id);
    turn.deadline = Some(fabric::MonoDeadlineMillis(10_000));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let operation = Arc::new(tokio::sync::Mutex::new(None));
    let captured = operation.clone();
    let submitted = coordinator.clone();
    let handle = tokio::spawn(async move {
        submitted
            .submit_with(
                turn,
                &TurnPolicy::daemon(),
                move |request, cancel| async move {
                    *captured.lock().await = Some(request.operation_id);
                    let _ = started_tx.send(());
                    cancel.cancelled().await;
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: "cancelled".into(),
                            stop: TurnStop::Cancelled,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics {
                                completed_normally: false,
                                ..Default::default()
                            },
                        },
                        items: Vec::new(),
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                },
            )
            .await
    });

    started_rx.await.expect("runner started");
    let operation_id = operation.lock().await.unwrap();
    assert!(coordinator.cancel_operation(operation_id).await);
    let result = handle
        .await
        .expect("submit task should not panic")
        .expect("user cancellation should be a terminal result");
    assert_eq!(result.stop, TurnStop::Cancelled);
    let operation = kernel.inspect_operation(operation_id).await.unwrap();
    assert_eq!(
        operation.exit,
        Some(OperationExitReason::Cancelled(fabric::CancelReason::User))
    );
}

#[tokio::test]
async fn cancellation_refreshes_the_terminal_sequence_after_runner_fragments() {
    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            read_store,
            event_spine,
            GrokHardeningConfig::default(),
        ),
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let operation = Arc::new(tokio::sync::Mutex::new(None));
    let captured = operation.clone();
    let runner_store = store.clone();
    let (fragment_tx, fragment_rx) = tokio::sync::oneshot::channel();
    let submitted = coordinator.clone();
    let handle = tokio::spawn(async move {
        submitted
            .submit_with(
                request("cancel-after-runner-fragment", process.id),
                &TurnPolicy::daemon(),
                move |request, cancel| async move {
                    *captured.lock().await = Some(request.operation_id);
                    let turn_id = request.context.turn_id.expect("coordinator turn id");
                    let session_id = SessionId(request.context.thread_id.0.clone());
                    runner_store
                        .append(
                            &session_id,
                            2,
                            fabric::ItemRecord {
                                schema_version: fabric::SESSION_SCHEMA_VERSION,
                                id: fabric::ItemId::new(),
                                session_id: session_id.clone(),
                                turn_id,
                                sequence: 2,
                                created_at_ms: 2,
                                payload: ItemPayload::TaskProjection {
                                    fact: fabric::TaskProjectionFact::default(),
                                },
                            },
                        )
                        .await?;
                    let _ = fragment_tx.send(());
                    cancel.cancelled().await;
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: "runner observed cancellation".into(),
                            stop: TurnStop::Cancelled,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics::default(),
                        },
                        items: Vec::new(),
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                },
            )
            .await
    });

    fragment_rx.await.expect("runner fragment persisted");
    let operation_id = operation.lock().await.expect("operation captured");
    assert!(coordinator.cancel_operation(operation_id).await);
    let result = handle
        .await
        .expect("submit task should not panic")
        .expect("cancellation terminal should follow the runner fragment");
    assert_eq!(result.stop, TurnStop::Cancelled);

    let items = store
        .load_items(&SessionId("cancel-after-runner-fragment".into()), None)
        .await
        .unwrap();
    assert_eq!(
        items.iter().map(|item| item.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(matches!(
        items[1].payload,
        ItemPayload::TaskProjection { .. }
    ));
    assert!(matches!(items[2].payload, ItemPayload::SystemNotice { .. }));
    assert_eq!(items[2].id.0, items[2].turn_id.0);
}

#[tokio::test]
async fn connection_scoped_cancel_touches_only_that_connections_turn() {
    use executive::application::turn_coordinator::TurnExecution;
    use executive::application::turn_policy::*;
    use fabric::TurnStop;

    let kernel = Arc::new(KernelRuntime::new());
    let read_store = Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            read_store,
            event_spine.clone(),
            GrokHardeningConfig::default(),
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    // Turn A owned by connection "conn-a".
    let conn_a = fabric::ConnectionId::new();
    let conn_b = fabric::ConnectionId::new();
    let mut turn_a = request("conn-a-turn", process.id);
    turn_a.context.connection_id = conn_a.clone();
    let coordinator_a = coordinator.clone();
    let handle_a = tokio::spawn(async move {
        coordinator_a
            .submit_with(
                turn_a,
                &TurnPolicy::daemon(),
                move |_request, cancel| async move {
                    loop {
                        if cancel.is_cancelled() {
                            return Ok(TurnExecution {
                                result: TurnResult {
                                    output: "cancelled".into(),
                                    stop: TurnStop::Cancelled,
                                    failure: None,
                                    usage: Default::default(),
                                    metrics: TurnMetrics {
                                        completed_normally: false,
                                        ..Default::default()
                                    },
                                },
                                items: Vec::new(),
                                projection: None,
                                context_projection: None,
                                evaluation_artifacts: Default::default(),
                            });
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                },
            )
            .await
    });

    // Turn B owned by connection "conn-b".
    let mut turn_b = request("conn-b-turn", process.id);
    turn_b.context.connection_id = conn_b.clone();
    let coordinator_b = coordinator.clone();
    let handle_b = tokio::spawn(async move {
        coordinator_b
            .submit_with(
                turn_b,
                &TurnPolicy::daemon(),
                move |_request, cancel| async move {
                    let mut iterations = 0u32;
                    loop {
                        if cancel.is_cancelled() {
                            return Ok(TurnExecution {
                                result: TurnResult {
                                    output: "conn-b never cancelled".into(),
                                    stop: TurnStop::Completed,
                                    failure: None,
                                    usage: Default::default(),
                                    metrics: TurnMetrics {
                                        completed_normally: true,
                                        ..Default::default()
                                    },
                                },
                                items: Vec::new(),
                                projection: None,
                                context_projection: None,
                                evaluation_artifacts: Default::default(),
                            });
                        }
                        iterations += 1;
                        if iterations > 50 {
                            // conn-b is never cancelled; complete normally.
                            return Ok(TurnExecution {
                                result: TurnResult {
                                    output: "conn-b completed".into(),
                                    stop: TurnStop::Completed,
                                    failure: None,
                                    usage: Default::default(),
                                    metrics: TurnMetrics {
                                        completed_normally: true,
                                        ..Default::default()
                                    },
                                },
                                items: Vec::new(),
                                projection: None,
                                context_projection: None,
                                evaluation_artifacts: Default::default(),
                            });
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                },
            )
            .await
    });

    // Give both turns time to register, then cancel only conn-a.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let cancelled = coordinator.cancel_active_for_connection(&conn_a).await;
    assert_eq!(cancelled, 1, "exactly one turn belongs to conn-a");

    let result_a = handle_a.await.unwrap().unwrap();
    assert_eq!(result_a.stop, TurnStop::Cancelled);
    let result_b = handle_b.await.unwrap().unwrap();
    assert_eq!(result_b.stop, TurnStop::Completed);
    assert_eq!(coordinator.active_turn_count().await, 0);
}

#[tokio::test]
async fn daemon_shutdown_cancels_and_settles_every_active_turn() {
    use executive::application::turn_coordinator::TurnExecution;
    use executive::application::turn_policy::*;
    use fabric::{OperationExitReason, TurnStop};

    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
            Arc::new(SqliteEventSpine::open(":memory:").unwrap()),
            GrokHardeningConfig::default(),
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let submitted = coordinator.clone();
    let handle = tokio::spawn(async move {
        submitted
            .submit_with(
                request("shutdown-turn", process.id),
                &TurnPolicy::daemon(),
                move |request, cancel| async move {
                    let _ = started_tx.send(request.operation_id);
                    cancel.cancelled().await;
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: "shutdown".into(),
                            stop: TurnStop::Cancelled,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics {
                                completed_normally: false,
                                ..Default::default()
                            },
                        },
                        items: Vec::new(),
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                },
            )
            .await
    });

    let operation_id = started_rx.await.expect("runner started");
    assert_eq!(coordinator.cancel_all_active().await, 1);
    let result = handle.await.unwrap().unwrap();
    assert_eq!(result.stop, TurnStop::Cancelled);
    assert_eq!(coordinator.active_turn_count().await, 0);
    let operation = kernel.inspect_operation(operation_id).await.unwrap();
    assert_eq!(operation.state, fabric::OperationState::Cancelled);
    assert_eq!(
        operation.exit,
        Some(OperationExitReason::Cancelled(
            fabric::CancelReason::Shutdown
        ))
    );
}

#[tokio::test]
async fn panicking_runner_is_failed_and_guard_removes_active_turn() {
    use executive::application::turn_policy::*;

    let kernel = Arc::new(KernelRuntime::new());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel.clone(),
            Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
            Arc::new(SqliteEventSpine::open(":memory:").unwrap()),
            GrokHardeningConfig::default(),
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let submitted = coordinator.clone();
    let handle = tokio::spawn(async move {
        submitted
            .submit_with(
                request("panic-turn", process.id),
                &TurnPolicy::daemon(),
                move |request, _cancel| async move {
                    let _ = started_tx.send(request.operation_id);
                    panic!("injected runner panic");
                    #[allow(unreachable_code)]
                    Ok(executive::application::turn_coordinator::TurnExecution {
                        result: TurnResult {
                            output: String::new(),
                            stop: fabric::TurnStop::Failed,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics::default(),
                        },
                        items: Vec::new(),
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                },
            )
            .await
    });

    let operation_id = started_rx.await.expect("runner started");
    let join_error = handle
        .await
        .expect_err("runner task must propagate the panic");
    assert!(join_error.is_panic());
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let operation = kernel.inspect_operation(operation_id).await.unwrap();
            if coordinator.active_turn_count().await == 0
                && operation.state == fabric::OperationState::Failed
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("panic fallback must settle within the bounded guard task");
}
