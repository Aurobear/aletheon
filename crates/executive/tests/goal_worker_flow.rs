use async_trait::async_trait;
use executive::goal::{
    goal_progress_from_outcome, AttemptCoordinationOutcome, AttemptCoordinator,
    AttemptCoordinatorError, AttemptExecutor, AttemptRequest, GoalCoordinator, ObjectiveStore,
    RetryDecision, RetryPolicy,
};
use executive::kernel::chronos::TestClock;
use fabric::channel::{ConversationId, OutboundMessage};
use fabric::{
    AttemptEvidence, AttemptUsage, CognitiveRole, FailureClass, GoalBudget, GoalId, GoalSpec,
    GoalState, PrincipalId, RuntimeFailure, RuntimeId, RuntimeResult,
};
use gateway::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use gateway::ChannelStore;
use rusqlite::Connection;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{NamedTempFile, TempDir};
use tokio_util::sync::CancellationToken;

struct QueueExecutor {
    outcomes: Mutex<VecDeque<Result<RuntimeResult, RuntimeFailure>>>,
    calls: AtomicUsize,
}

#[async_trait]
impl AttemptExecutor for QueueExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        matches!(runtime_id.0.as_str(), "worker" | "reviewer")
    }

    async fn run_once(
        &self,
        _runtime_id: &RuntimeId,
        _task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcomes.lock().unwrap().pop_front();
        match outcome {
            Some(outcome) => outcome,
            None => {
                cancel.cancelled().await;
                Err(failure(FailureClass::Cancelled, false))
            }
        }
    }
}

struct NoopTurn;

#[async_trait]
impl ChannelTurnExecutor for NoopTurn {
    async fn execute(
        &self,
        _principal: &str,
        _message: &str,
        _correlation_id: &str,
    ) -> anyhow::Result<String> {
        unreachable!("Goal progress does not execute a chat turn")
    }
}

struct InspectingTransport {
    channel_path: PathBuf,
    goal_path: PathBuf,
    sent: Mutex<Vec<OutboundMessage>>,
}

#[async_trait]
impl ChannelTransport for InspectingTransport {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn receive(&self, _cursor: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(vec![])
    }

    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String> {
        // Both durable effects must already be visible before network I/O.
        let channel = Connection::open(&self.channel_path)?;
        let outbox: i64 = channel.query_row(
            "SELECT COUNT(*) FROM channel_outbox WHERE correlation_id = ?1",
            rusqlite::params![message.correlation_id],
            |row| row.get(0),
        )?;
        assert_eq!(outbox, 1, "outbox must commit before Telegram send");
        let goals = Connection::open(&self.goal_path)?;
        let events: i64 = goals.query_row(
            "SELECT COUNT(*) FROM goal_events WHERE objective_id = ?1",
            rusqlite::params![goal_id_from_text(message)],
            |row| row.get(0),
        )?;
        assert!(events >= 4, "terminal progress event must precede send");
        self.sent.lock().unwrap().push(message.clone());
        Ok("telegram-message-1".into())
    }
}

fn goal_id_from_text(message: &OutboundMessage) -> i64 {
    let fabric::MessageContent::Text { text } = &message.content else {
        panic!("expected text")
    };
    text.split_whitespace().nth(1).unwrap().parse().unwrap()
}

struct Harness {
    _goal_file: NamedTempFile,
    _channel_dir: TempDir,
    store: Arc<Mutex<ObjectiveStore>>,
    goal: GoalCoordinator,
    attempts: AttemptCoordinator,
    executor: Arc<QueueExecutor>,
    router: ChannelDispatcher,
    transport: InspectingTransport,
    goal_id: GoalId,
}

fn harness(outcomes: Vec<Result<RuntimeResult, RuntimeFailure>>) -> Harness {
    let goal_file = NamedTempFile::new().unwrap();
    let store = Arc::new(Mutex::new(ObjectiveStore::open(goal_file.path()).unwrap()));
    let snapshot = store
        .lock()
        .unwrap()
        .create_goal(
            &PrincipalId("owner".into()),
            "session",
            "project",
            &GoalSpec {
                original_intent: "complete worker flow".into(),
                desired_state: vec![],
                constraints: vec![],
                acceptance_criteria: vec![],
                budget: GoalBudget {
                    max_input_tokens: 10_000,
                    max_output_tokens: 10_000,
                    max_cost_usd: None,
                    max_attempts: 10,
                    deadline_ms: None,
                },
            },
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .transition_goal(
            snapshot.id,
            snapshot.version,
            GoalState::Running,
            None,
            &serde_json::json!({"action": "worker_test_start"}),
        )
        .unwrap();
    let executor = Arc::new(QueueExecutor {
        outcomes: Mutex::new(outcomes.into()),
        calls: AtomicUsize::new(0),
    });
    let goal = GoalCoordinator::new(store.clone());
    let attempts = goal.attempt_coordinator(
        executor.clone(),
        Arc::new(TestClock::new(20_000, 0)),
        RetryPolicy::default(),
    );

    let channel_dir = tempfile::tempdir().unwrap();
    let channel_path = channel_dir.path().join("channels.db");
    let router = ChannelDispatcher::new(
        ChannelStore::open(&channel_path).unwrap(),
        Arc::new(NoopTurn),
    );
    let transport = InspectingTransport {
        channel_path,
        goal_path: goal_file.path().to_owned(),
        sent: Mutex::new(vec![]),
    };
    Harness {
        _goal_file: goal_file,
        _channel_dir: channel_dir,
        store,
        goal,
        attempts,
        executor,
        router,
        transport,
        goal_id: snapshot.id,
    }
}

fn request(store: &Arc<Mutex<ObjectiveStore>>, goal_id: GoalId, sequence: u32) -> AttemptRequest {
    let version = store
        .lock()
        .unwrap()
        .get_goal(goal_id)
        .unwrap()
        .unwrap()
        .version;
    AttemptRequest {
        goal_id,
        expected_version: version,
        sequence,
        runtime_id: RuntimeId("worker".into()),
        escalation_runtime_id: Some(RuntimeId("reviewer".into())),
        role: CognitiveRole::Worker,
        task: "perform one worker step".into(),
        estimated_usage: AttemptUsage::default(),
    }
}

fn success() -> RuntimeResult {
    RuntimeResult {
        output: "TOP_SECRET_RAW_PROVIDER_OUTPUT".into(),
        usage: AttemptUsage::default(),
        evidence: vec![],
    }
}

fn failure(class: FailureClass, retryable: bool) -> RuntimeFailure {
    RuntimeFailure {
        class,
        message: "TOP_SECRET_RAW_PROVIDER_ERROR".into(),
        retryable,
        usage: AttemptUsage::default(),
        evidence: vec![AttemptEvidence {
            kind: "test".into(),
            summary: "bounded".into(),
            content: "full raw tool output must stay durable-only".into(),
        }],
    }
}

async fn notify(h: &Harness, outcome: &AttemptCoordinationOutcome) {
    let progress = goal_progress_from_outcome(outcome);
    assert!(h
        .router
        .notify_goal_progress(&h.transport, ConversationId("owner-chat".into()), &progress,)
        .await
        .unwrap());
}

fn resume(store: &Arc<Mutex<ObjectiveStore>>, goal_id: GoalId) {
    let store = store.lock().unwrap();
    let blocked = store.get_goal(goal_id).unwrap().unwrap();
    let ready = store
        .transition_goal(
            goal_id,
            blocked.version,
            GoalState::Ready,
            None,
            &serde_json::json!({"action": "wait_complete"}),
        )
        .unwrap();
    store
        .transition_goal(
            goal_id,
            ready.version,
            GoalState::Running,
            None,
            &serde_json::json!({"action": "next_attempt"}),
        )
        .unwrap();
}

#[tokio::test]
async fn success_event_and_outbox_are_persisted_before_bounded_notification() {
    let h = harness(vec![Ok(success())]);
    let outcome = h
        .goal
        .tick_attempt(
            &h.attempts,
            request(&h.store, h.goal_id, 1),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    notify(&h, &outcome).await;
    let sent = h.transport.sent.lock().unwrap();
    let fabric::MessageContent::Text { text } = &sent[0].content else {
        panic!("expected text")
    };
    assert!(text.contains(&format!("Goal {} attempt", h.goal_id.0)));
    assert!(text.contains("completed successfully"));
    assert!(!text.contains("TOP_SECRET"));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_backoff_notification_contains_ids_without_raw_error() {
    let h = harness(vec![Err(failure(FailureClass::ProviderTransient, true))]);
    let outcome = h
        .goal
        .tick_attempt(
            &h.attempts,
            request(&h.store, h.goal_id, 1),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AttemptCoordinationOutcome::Failed {
            decision: RetryDecision::RetrySame { .. },
            ..
        }
    ));
    notify(&h, &outcome).await;
    let fabric::MessageContent::Text { text } = &h.transport.sent.lock().unwrap()[0].content else {
        panic!("expected text")
    };
    assert!(text.contains("bounded backoff"));
    assert!(!text.contains("TOP_SECRET"));
}

#[tokio::test]
async fn third_tick_escalates_and_notifies_once() {
    let repeated = || Err(failure(FailureClass::TestFailure, true));
    let h = harness(vec![repeated(), repeated(), repeated()]);
    let mut last = None;
    for sequence in 1..=3 {
        let outcome = h
            .goal
            .tick_attempt(
                &h.attempts,
                request(&h.store, h.goal_id, sequence),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        if sequence < 3 {
            resume(&h.store, h.goal_id);
        }
        last = Some(outcome);
    }
    let outcome = last.unwrap();
    assert!(matches!(
        outcome,
        AttemptCoordinationOutcome::Failed {
            decision: RetryDecision::Escalate { .. },
            ..
        }
    ));
    notify(&h, &outcome).await;
    let fabric::MessageContent::Text { text } = &h.transport.sent.lock().unwrap()[0].content else {
        panic!("expected text")
    };
    assert!(text.contains("escalated to reviewer"));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn permission_failure_awaits_human_and_notifies() {
    let h = harness(vec![Err(failure(FailureClass::PermissionDenied, false))]);
    let outcome = h
        .goal
        .tick_attempt(
            &h.attempts,
            request(&h.store, h.goal_id, 1),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Failed { ref goal, .. } = outcome else {
        panic!("expected failure")
    };
    assert_eq!(goal.state, GoalState::AwaitingHuman);
    notify(&h, &outcome).await;
    let fabric::MessageContent::Text { text } = &h.transport.sent.lock().unwrap()[0].content else {
        panic!("expected text")
    };
    assert!(text.contains("awaiting human input"));
}

#[tokio::test]
async fn cancellation_notifies_without_starting_another_attempt() {
    let h = harness(vec![]);
    let cancel = CancellationToken::new();
    let future = h
        .goal
        .tick_attempt(&h.attempts, request(&h.store, h.goal_id, 1), cancel.clone());
    tokio::pin!(future);
    while h.executor.calls.load(Ordering::SeqCst) == 0 {
        tokio::select! {
            outcome = &mut future => panic!("runtime returned early: {outcome:?}"),
            _ = tokio::task::yield_now() => {}
        }
    }
    cancel.cancel();
    let outcome = future.await.unwrap();
    notify(&h, &outcome).await;
    let fabric::MessageContent::Text { text } = &h.transport.sent.lock().unwrap()[0].content else {
        panic!("expected text")
    };
    assert!(text.contains("was cancelled"));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn duplicate_tick_hits_version_conflict_and_invokes_runtime_only_once() {
    let h = harness(vec![]);
    let req = request(&h.store, h.goal_id, 1);
    let cancel = CancellationToken::new();
    let running = h
        .goal
        .tick_attempt(&h.attempts, req.clone(), cancel.clone());
    tokio::pin!(running);
    while h.executor.calls.load(Ordering::SeqCst) == 0 {
        tokio::select! {
            outcome = &mut running => panic!("runtime returned early: {outcome:?}"),
            _ = tokio::task::yield_now() => {}
        }
    }
    let duplicate = h
        .goal
        .tick_attempt(&h.attempts, req, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        duplicate,
        AttemptCoordinatorError::VersionConflict { .. }
    ));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn daemon_restart_between_attempts_does_not_repeat_the_first_runtime_call() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_owned();
    let first_store = Arc::new(Mutex::new(ObjectiveStore::open(&path).unwrap()));
    let snapshot = first_store
        .lock()
        .unwrap()
        .create_goal(
            &PrincipalId("owner".into()),
            "session",
            "project",
            &GoalSpec {
                original_intent: "restart safely".into(),
                desired_state: vec![],
                constraints: vec![],
                acceptance_criteria: vec![],
                budget: GoalBudget::default(),
            },
        )
        .unwrap();
    first_store
        .lock()
        .unwrap()
        .transition_goal(
            snapshot.id,
            snapshot.version,
            GoalState::Running,
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    let first_executor = Arc::new(QueueExecutor {
        outcomes: Mutex::new(vec![Err(failure(FailureClass::ProviderTransient, true))].into()),
        calls: AtomicUsize::new(0),
    });
    let first_goal = GoalCoordinator::new(first_store.clone());
    let first_attempts = first_goal.attempt_coordinator(
        first_executor.clone(),
        Arc::new(TestClock::new(100, 0)),
        RetryPolicy::default(),
    );
    first_goal
        .tick_attempt(
            &first_attempts,
            request(&first_store, snapshot.id, 1),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(first_executor.calls.load(Ordering::SeqCst), 1);
    drop(first_attempts);
    drop(first_goal);
    drop(first_store);

    let reopened = Arc::new(Mutex::new(ObjectiveStore::open(&path).unwrap()));
    assert!(reopened
        .lock()
        .unwrap()
        .recover_stale_attempts()
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened
            .lock()
            .unwrap()
            .attempts_for_goal(snapshot.id, 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reopened
            .lock()
            .unwrap()
            .get_goal(snapshot.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Blocked
    );
}
