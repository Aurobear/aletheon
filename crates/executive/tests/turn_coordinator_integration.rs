mod support {
    pub mod mock_llm_provider;
    pub mod mock_sandbox;
    pub mod test_aletheon_builder;
}

use std::sync::Arc;

use async_trait::async_trait;

use executive::application::turn_coordinator::TurnExecution;
use executive::application::turn_policy::TurnPolicy;
use executive::runtime::session::projection_store::SessionProjectionStore;
use fabric::{
    ItemPayload, OperationKind, OperationState, PrincipalId, SessionId, SessionReadStore,
    TurnMetrics, TurnRequest, TurnResult, TurnStop,
};
use support::test_aletheon_builder::TestAletheonBuilder;

struct TerminalFailingStore {
    inner: executive::runtime::session::canonical_store::CanonicalSessionStore,
}

struct AmbiguousTerminalAckStore {
    inner: executive::runtime::session::canonical_store::CanonicalSessionStore,
    terminal_attempts: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl SessionProjectionStore for TerminalFailingStore {
    async fn create(&self, session: fabric::SessionRecord) -> anyhow::Result<()> {
        self.inner.create(session).await
    }

    async fn next_sequence(&self, session: &SessionId) -> anyhow::Result<Option<u64>> {
        self.inner.next_sequence(session).await
    }

    async fn item_by_id(
        &self,
        session: &SessionId,
        id: &fabric::ItemId,
    ) -> anyhow::Result<Option<fabric::ItemRecord>> {
        self.inner.item_by_id(session, id).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: fabric::ItemRecord,
    ) -> anyhow::Result<fabric::AppendOutcome> {
        if matches!(
            &item.payload,
            ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. }
        ) {
            anyhow::bail!("injected terminal-persist crash")
        }
        self.inner.append(session, expected_sequence, item).await
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: fabric::SessionRecord,
    ) -> anyhow::Result<()> {
        self.inner.fork(parent, through_sequence, child).await
    }

    async fn bind_principal(
        &self,
        session: &SessionId,
        principal: &PrincipalId,
    ) -> anyhow::Result<()> {
        self.inner.bind_principal(session, principal).await
    }
}

#[async_trait]
impl SessionReadStore for TerminalFailingStore {
    async fn load_session(
        &self,
        session: &SessionId,
    ) -> anyhow::Result<Option<fabric::SessionRecord>> {
        self.inner.load_session(session).await
    }

    async fn load_items(
        &self,
        session: &SessionId,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<fabric::ItemRecord>> {
        self.inner.load_items(session, after).await
    }
}

#[async_trait]
impl SessionProjectionStore for AmbiguousTerminalAckStore {
    async fn create(&self, session: fabric::SessionRecord) -> anyhow::Result<()> {
        self.inner.create(session).await
    }

    async fn next_sequence(&self, session: &SessionId) -> anyhow::Result<Option<u64>> {
        self.inner.next_sequence(session).await
    }

    async fn item_by_id(
        &self,
        session: &SessionId,
        id: &fabric::ItemId,
    ) -> anyhow::Result<Option<fabric::ItemRecord>> {
        self.inner.item_by_id(session, id).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: fabric::ItemRecord,
    ) -> anyhow::Result<fabric::AppendOutcome> {
        let terminal = item.id.0 == item.turn_id.0;
        let outcome = self.inner.append(session, expected_sequence, item).await?;
        if terminal
            && self
                .terminal_attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
        {
            anyhow::bail!("injected lost terminal acknowledgement after durable append");
        }
        Ok(outcome)
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: fabric::SessionRecord,
    ) -> anyhow::Result<()> {
        self.inner.fork(parent, through_sequence, child).await
    }

    async fn bind_principal(
        &self,
        session: &SessionId,
        principal: &PrincipalId,
    ) -> anyhow::Result<()> {
        self.inner.bind_principal(session, principal).await
    }
}

#[async_trait]
impl SessionReadStore for AmbiguousTerminalAckStore {
    async fn load_session(
        &self,
        session: &SessionId,
    ) -> anyhow::Result<Option<fabric::SessionRecord>> {
        self.inner.load_session(session).await
    }

    async fn load_items(
        &self,
        session: &SessionId,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<fabric::ItemRecord>> {
        self.inner.load_items(session, after).await
    }
}

mod turn_request_support;

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

//
// Session creation
//

#[tokio::test]
async fn create_session_on_first_turn() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    test.coordinator
        .submit_with(
            request("new-session", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "ok".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();

    // Verify session was created in store.
    let session = test
        .store
        .load_session(&SessionId("new-session".into()))
        .await
        .unwrap()
        .expect("session should exist");
    assert_eq!(session.status, fabric::SessionStatus::Active);
}

#[tokio::test]
async fn terminal_writer_failure_prevents_false_success_and_retains_recovery_boundary() {
    let clock = Arc::new(kernel::chronos::TestClock::default());
    let kernel = Arc::new(::kernel::KernelRuntime::with_clock(
        clock as Arc<dyn fabric::Clock>,
    ));
    let projection_store: Arc<dyn SessionProjectionStore> = Arc::new(TerminalFailingStore {
        inner: executive::runtime::session::canonical_store::CanonicalSessionStore::open(
            ":memory:",
        )
        .unwrap(),
    });
    let spine = Arc::new(executive::runtime::events::SqliteEventSpine::open(":memory:").unwrap());
    let hardening = executive::composition::config::GrokHardeningConfig {
        compaction_v2: true,
        ..Default::default()
    };
    let coordinator = executive::testing::turn_coordinator::compose_with_event_spine(
        kernel.clone(),
        projection_store,
        spine,
        hardening,
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    let outcome = coordinator
        .submit_with(
            request("terminal-crash", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "must-not-succeed".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await;

    let error = outcome.expect_err("terminal persistence failure must reject success");
    assert!(error.to_string().contains("terminal durable write failed"));
    assert_eq!(coordinator.active_turn_count().await, 1);
    let items = store
        .load_items(&SessionId("terminal-crash".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
}

#[tokio::test]
async fn terminal_settlement_retry_is_idempotent_after_ambiguous_ack() {
    let clock = Arc::new(kernel::chronos::TestClock::default());
    let kernel = Arc::new(::kernel::KernelRuntime::with_clock(
        clock as Arc<dyn fabric::Clock>,
    ));
    let terminal_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let projection_store: Arc<dyn SessionProjectionStore> = Arc::new(AmbiguousTerminalAckStore {
        inner: executive::runtime::session::canonical_store::CanonicalSessionStore::open(
            ":memory:",
        )
        .unwrap(),
        terminal_attempts: terminal_attempts.clone(),
    });
    let spine = Arc::new(executive::runtime::events::SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = executive::testing::turn_coordinator::compose_with_event_spine(
        kernel.clone(),
        projection_store,
        spine,
        executive::composition::config::GrokHardeningConfig::default(),
    );
    let store = coordinator.store();
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    let result = coordinator
        .submit_with(
            request("terminal-ambiguous-ack", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "settled once".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .expect("the identical terminal retry should observe AlreadyPresent");

    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(
        terminal_attempts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    let items = store
        .load_items(&SessionId("terminal-ambiguous-ack".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2, "terminal retry must not duplicate items");
    assert_eq!(items[1].id.0, items[1].turn_id.0);
    assert!(matches!(
        &items[1].payload,
        ItemPayload::AssistantMessage { content } if content == "settled once"
    ));
}

//
// Item ordering
//

#[tokio::test]
async fn append_items_in_sequence_order() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    test.coordinator
        .submit_with(
            request("seq-test", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
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
                            call_id: "c1".into(),
                            name: "tool-a".into(),
                            input: serde_json::json!({}),
                        },
                        ItemPayload::ToolResult {
                            call_id: "c1".into(),
                            content: "ok".into(),
                            is_error: false,
                            permit_id: None,
                            audit_id: None,
                        },
                    ],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();

    let items = test
        .store
        .load_items(&SessionId("seq-test".into()), None)
        .await
        .unwrap();
    // UserMessage at seq 1, ToolCall at 2, ToolResult at 3, AssistantMessage at 4
    assert_eq!(items.len(), 4);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::ToolCall { .. }));
    assert!(matches!(items[2].payload, ItemPayload::ToolResult { .. }));
    assert!(matches!(
        items[3].payload,
        ItemPayload::AssistantMessage { .. }
    ));
    assert_eq!(items[0].sequence, 1);
    assert_eq!(items[1].sequence, 2);
    assert_eq!(items[2].sequence, 3);
    assert_eq!(items[3].sequence, 4);
}

//
// Operation settlement
//

#[tokio::test]
async fn settle_operation_on_success() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();

    let result = test
        .coordinator
        .submit_with(
            request("success-settle", process.id),
            &TurnPolicy::daemon(),
            move |request, _cancel| {
                let c = capture.clone();
                async move {
                    *c.lock().await = Some(request.operation_id);
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: "done".into(),
                            stop: TurnStop::Completed,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics {
                                completed_normally: true,
                                ..Default::default()
                            },
                        },
                        items: vec![],
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                }
            },
        )
        .await
        .unwrap();

    assert_eq!(result.stop, TurnStop::Completed);
    let op = test
        .kernel
        .inspect_operation(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(op.kind, OperationKind::Turn);
    assert_eq!(op.state, OperationState::Succeeded);

    let items = test
        .store
        .load_items(&SessionId("success-settle".into()), None)
        .await
        .unwrap();
    let last = items.last().unwrap();
    assert!(matches!(last.payload, ItemPayload::AssistantMessage { .. }));
}

#[tokio::test]
async fn settle_operation_on_failure() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();

    let err = test
        .coordinator
        .submit_with(
            request("fail-settle", process.id),
            &TurnPolicy::exec(),
            move |request, _cancel| {
                let c = capture.clone();
                async move {
                    *c.lock().await = Some(request.operation_id);
                    anyhow::bail!("model error")
                }
            },
        )
        .await
        .unwrap_err();

    assert!(err.to_string().contains("model error"));
    let op = test
        .kernel
        .inspect_operation(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(op.kind, OperationKind::Turn);
    assert_eq!(op.state, OperationState::Failed);

    let items = test
        .store
        .load_items(&SessionId("fail-settle".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::SystemNotice { .. }));
}

#[tokio::test]
async fn missing_principal_context_refusal_is_durable_and_auditable() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    let error = test
        .coordinator
        .submit_with(
            request("missing-principal-context", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
                Err(anyhow::Error::new(
                    executive::application::turn_engine::TurnEngineError::InvalidContext(
                        "authenticated principal context is missing".into(),
                    ),
                ))
            },
        )
        .await
        .expect_err("missing authority must fail before capability execution");
    assert!(error
        .to_string()
        .contains("authenticated principal context is missing"));

    let items = test
        .store
        .load_items(&SessionId("missing-principal-context".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(matches!(
        &items[1].payload,
        ItemPayload::SystemNotice { content }
            if content.contains("authenticated principal context is missing")
    ));
}

//
// Cancellation
//

#[tokio::test]
async fn cancel_mid_turn() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();

    let result = test
        .coordinator
        .submit_with(
            request("cancel-mid", process.id),
            &TurnPolicy::daemon(),
            move |request, cancel| {
                let c = capture.clone();
                async move {
                    *c.lock().await = Some(request.operation_id);
                    cancel.cancel();
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: String::new(),
                            stop: TurnStop::Cancelled,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics {
                                completed_normally: false,
                                ..Default::default()
                            },
                        },
                        items: vec![],
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                }
            },
        )
        .await
        .unwrap();

    assert_eq!(result.stop, TurnStop::Cancelled);
    let op = test
        .kernel
        .inspect_operation(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(op.state, OperationState::Cancelled);
}

//
// Concurrent turn isolation
//

#[tokio::test]
async fn concurrent_turns_different_sessions_dont_interfere() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    let coordinator = &test.coordinator;
    let policy_a = TurnPolicy::daemon();
    let policy_b = TurnPolicy::daemon();
    let (r1, r2) = tokio::join!(
        coordinator.submit_with(
            request("concurrent-a", process.id),
            &policy_a,
            |_request, _cancel| async {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "a".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        ),
        coordinator.submit_with(
            request("concurrent-b", process.id),
            &policy_b,
            |_request, _cancel| async {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "b".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        ),
    );

    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert_eq!(r1.unwrap().output, "a");
    assert_eq!(r2.unwrap().output, "b");

    // Both sessions exist independently.
    assert!(test
        .store
        .load_session(&SessionId("concurrent-a".into()))
        .await
        .unwrap()
        .is_some());
    assert!(test
        .store
        .load_session(&SessionId("concurrent-b".into()))
        .await
        .unwrap()
        .is_some());
}

//
// Event spine monotonicity
//

#[tokio::test]
async fn event_spine_sequence_monotonic_across_turns() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    // First turn
    test.coordinator
        .submit_with(
            request("mono-seq", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "t1".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();

    // Second turn
    test.coordinator
        .submit_with(
            request("mono-seq", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "t2".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();

    let items = test
        .store
        .load_items(&SessionId("mono-seq".into()), None)
        .await
        .unwrap();
    // Each turn: UserMessage + AssistantMessage. So 2 turns = 4 items.
    assert_eq!(items.len(), 4);

    let events = test
        .event_spine
        .read_tree(
            fabric::EventTreeId::for_root_session("mono-seq"),
            executive::runtime::events::EventReadFilter {
                limit: 20,
                ..Default::default()
            },
        )
        .unwrap();
    // SessionCreated (1) + 4 items = 5 events
    assert!(events.len() >= 4);

    // Verify event sequences are strictly increasing.
    let sequences: Vec<u64> = events.iter().map(|e| e.position.sequence.0).collect();
    for window in sequences.windows(2) {
        assert!(window[0] < window[1], "event sequence must be monotonic");
    }
}

//
// Context projection storage
//

#[tokio::test]
async fn context_projection_stored_as_item() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();

    test.coordinator
        .submit_with(
            request("ctx-proj", process.id),
            &TurnPolicy::daemon(),
            |_request, _cancel| async move {
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "ok".into(),
                        stop: TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: Some(fabric::ContextProjectionReceipt {
                        space: fabric::AgoraSpaceId("test-space".into()),
                        broadcast_epoch: Some(fabric::BroadcastEpoch(1)),
                        workspace_version: Some(2),
                        dasein_version: fabric::dasein::SelfVersion(3),
                        content_ids: vec![fabric::ContentId(uuid::Uuid::from_u128(1))],
                    }),
                    evaluation_artifacts: Default::default(),
                })
            },
        )
        .await
        .unwrap();

    let items = test
        .store
        .load_items(&SessionId("ctx-proj".into()), None)
        .await
        .unwrap();
    let ctx_item = items
        .iter()
        .find(|i| matches!(i.payload, ItemPayload::ContextProjection { .. }));
    assert!(
        ctx_item.is_some(),
        "ContextProjection item should be stored"
    );
    if let ItemPayload::ContextProjection {
        space,
        broadcast_epoch,
        workspace_version,
        dasein_version,
        ..
    } = &ctx_item.unwrap().payload
    {
        assert_eq!(space, "test-space");
        assert_eq!(*broadcast_epoch, Some(1));
        assert_eq!(*workspace_version, Some(2));
        assert_eq!(*dasein_version, 3);
    }
}
