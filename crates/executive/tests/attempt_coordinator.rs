use async_trait::async_trait;
use executive::kernel::chronos::TestClock;
use executive::r#impl::goal::{
    AttemptCoordinationOutcome, AttemptCoordinator, AttemptCoordinatorError, AttemptExecutor,
    AttemptRequest, ObjectiveStore, RetryDecision, RetryPolicy,
};
use fabric::{
    AttemptEvidence, AttemptUsage, CognitiveRole, FailureClass, GoalBudget, GoalId, GoalSpec,
    GoalState, GoalWaitReason, PrincipalId, RuntimeFailure, RuntimeId, RuntimeResult,
};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio_util::sync::CancellationToken;

struct FakeExecutor {
    available: HashSet<RuntimeId>,
    outcomes: Mutex<VecDeque<Result<RuntimeResult, RuntimeFailure>>>,
    tasks: Mutex<Vec<String>>,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
}

impl FakeExecutor {
    fn new(
        available: impl IntoIterator<Item = RuntimeId>,
        outcomes: Vec<Result<RuntimeResult, RuntimeFailure>>,
    ) -> Self {
        Self {
            available: available.into_iter().collect(),
            outcomes: Mutex::new(outcomes.into()),
            tasks: Mutex::new(vec![]),
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl AttemptExecutor for FakeExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        self.available.contains(runtime_id)
    }

    async fn run_once(
        &self,
        _runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.tasks.lock().unwrap().push(task.to_owned());
        self.active.fetch_add(1, Ordering::SeqCst);
        let _guard = ActiveGuard(self.active.clone());
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

struct Harness {
    _file: NamedTempFile,
    store: Arc<Mutex<ObjectiveStore>>,
    executor: Arc<FakeExecutor>,
    coordinator: AttemptCoordinator,
    goal_id: GoalId,
}

fn harness(budget: GoalBudget, outcomes: Vec<Result<RuntimeResult, RuntimeFailure>>) -> Harness {
    let file = NamedTempFile::new().unwrap();
    let store = Arc::new(Mutex::new(ObjectiveStore::open(file.path()).unwrap()));
    let goal = store
        .lock()
        .unwrap()
        .create_goal(
            &PrincipalId("owner".into()),
            "session",
            "project",
            &GoalSpec {
                original_intent: "implement a durable feature".into(),
                desired_state: vec!["tests pass".into()],
                constraints: vec!["one attempt per call".into()],
                acceptance_criteria: vec!["evidence persisted".into()],
                budget,
            },
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .transition_goal(
            goal.id,
            goal.version,
            GoalState::Running,
            None,
            &serde_json::json!({"test": "start"}),
        )
        .unwrap();
    let executor = Arc::new(FakeExecutor::new(
        [RuntimeId("worker".into()), RuntimeId("reviewer".into())],
        outcomes,
    ));
    let coordinator = AttemptCoordinator::new(
        store.clone(),
        executor.clone(),
        Arc::new(TestClock::new(10_000, 0)),
        RetryPolicy::default(),
    );
    Harness {
        _file: file,
        store,
        executor,
        coordinator,
        goal_id: goal.id,
    }
}

fn default_budget() -> GoalBudget {
    GoalBudget {
        max_input_tokens: 10_000,
        max_output_tokens: 10_000,
        max_cost_usd: Some(10.0),
        max_attempts: 10,
        deadline_ms: None,
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
        task: "run the bounded task".into(),
        estimated_usage: AttemptUsage {
            input_tokens: 10,
            output_tokens: 10,
            cost_usd: Some(0.01),
            elapsed_ms: 0,
        },
    }
}

fn success() -> RuntimeResult {
    RuntimeResult {
        output: "done".into(),
        usage: AttemptUsage {
            input_tokens: 4,
            output_tokens: 3,
            cost_usd: Some(0.002),
            elapsed_ms: 12,
        },
        evidence: vec![AttemptEvidence {
            kind: "test".into(),
            summary: "suite passed".into(),
            content: "12 passed".into(),
        }],
    }
}

fn failure(class: FailureClass, retryable: bool) -> RuntimeFailure {
    RuntimeFailure {
        class,
        message: format!("{class:?}"),
        retryable,
        usage: AttemptUsage {
            input_tokens: 5,
            output_tokens: 2,
            cost_usd: Some(0.003),
            elapsed_ms: 20,
        },
        evidence: vec![AttemptEvidence {
            kind: "diagnostic".into(),
            summary: "bounded failure".into(),
            content: "compiler/test excerpt".into(),
        }],
    }
}

fn resume_running(store: &Arc<Mutex<ObjectiveStore>>, goal_id: GoalId) {
    let store = store.lock().unwrap();
    let blocked = store.get_goal(goal_id).unwrap().unwrap();
    let ready = store
        .transition_goal(
            goal_id,
            blocked.version,
            GoalState::Ready,
            None,
            &serde_json::json!({"test": "backoff_elapsed"}),
        )
        .unwrap();
    store
        .transition_goal(
            goal_id,
            ready.version,
            GoalState::Running,
            None,
            &serde_json::json!({"test": "resume"}),
        )
        .unwrap();
}

#[tokio::test]
async fn success_persists_one_attempt_settles_usage_and_completes_goal() {
    let h = harness(default_budget(), vec![Ok(success())]);
    let outcome = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 1), CancellationToken::new())
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Succeeded { attempt, goal } = outcome else {
        panic!("expected success")
    };
    assert_eq!(attempt.output.unwrap().output, "done");
    assert_eq!(attempt.usage.input_tokens, 4);
    assert_eq!(goal.state, GoalState::Completed);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.executor.active.load(Ordering::SeqCst), 0);
    let prompt = &h.executor.tasks.lock().unwrap()[0];
    assert!(prompt.starts_with("<goal_frame version=\"m3\">"));
    assert!(prompt.contains("implement a durable feature"));
    assert!(prompt.contains("run the bounded task"));
    assert_eq!(
        h.store
            .lock()
            .unwrap()
            .attempts_for_goal(h.goal_id, 10)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn transient_failure_persists_evidence_and_wall_clock_backoff() {
    let h = harness(
        default_budget(),
        vec![Err(failure(FailureClass::ProviderTransient, true))],
    );
    let outcome = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 1), CancellationToken::new())
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Failed {
        attempt,
        decision,
        goal,
    } = outcome
    else {
        panic!("expected failure")
    };
    assert_eq!(attempt.evidence[0].kind, "diagnostic");
    assert!(matches!(
        decision,
        RetryDecision::RetrySame {
            after_ms: 1_000,
            ..
        }
    ));
    assert_eq!(goal.state, GoalState::Blocked);
    assert_eq!(
        goal.wait_reason,
        Some(GoalWaitReason::Backoff { until_ms: 11_000 })
    );
}

#[tokio::test]
async fn cancellation_is_terminal_and_drops_the_active_invocation() {
    let h = harness(default_budget(), vec![]);
    let cancel = CancellationToken::new();
    let future = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 1), cancel.clone());
    tokio::pin!(future);
    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => cancel.cancel(),
        _ = &mut future => panic!("pending runtime returned before cancellation"),
    }
    let outcome = future.await.unwrap();
    let AttemptCoordinationOutcome::Failed { decision, goal, .. } = outcome else {
        panic!("expected cancellation")
    };
    assert_eq!(decision, RetryDecision::Cancel);
    assert_eq!(goal.state, GoalState::Cancelled);
    assert_eq!(h.executor.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn timeout_is_a_structured_retryable_attempt_failure() {
    let h = harness(
        default_budget(),
        vec![Err(failure(FailureClass::Timeout, true))],
    );
    let outcome = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 1), CancellationToken::new())
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Failed {
        attempt, decision, ..
    } = outcome
    else {
        panic!("expected timeout failure")
    };
    assert_eq!(attempt.failure.unwrap().class, FailureClass::Timeout);
    assert!(matches!(decision, RetryDecision::RetrySame { .. }));
}

#[tokio::test]
async fn missing_runtime_creates_no_attempt_and_invokes_nothing() {
    let h = harness(default_budget(), vec![Ok(success())]);
    let mut req = request(&h.store, h.goal_id, 1);
    req.runtime_id = RuntimeId("missing".into());
    let error = h
        .coordinator
        .execute_one(req, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AttemptCoordinatorError::RuntimeUnavailable(_)
    ));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert!(h
        .store
        .lock()
        .unwrap()
        .attempts_for_goal(h.goal_id, 10)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn third_worker_failure_escalates_to_distinct_reviewer_runtime() {
    let repeated = || Err(failure(FailureClass::TestFailure, true));
    let h = harness(default_budget(), vec![repeated(), repeated(), repeated()]);
    for sequence in 1..=2 {
        let outcome = h
            .coordinator
            .execute_one(
                request(&h.store, h.goal_id, sequence),
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
        resume_running(&h.store, h.goal_id);
    }
    let outcome = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 3), CancellationToken::new())
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Failed { decision, goal, .. } = outcome else {
        panic!("expected failure")
    };
    assert!(matches!(
        decision,
        RetryDecision::Escalate { ref runtime_id, .. } if runtime_id.0 == "reviewer"
    ));
    assert_eq!(
        goal.wait_reason,
        Some(GoalWaitReason::ExternalEvent {
            key: "runtime:reviewer".into()
        })
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn exhausted_reviewer_attempts_await_human() {
    let repeated = || Err(failure(FailureClass::TestFailure, true));
    let h = harness(default_budget(), vec![repeated(), repeated()]);
    for sequence in 1..=2 {
        let mut req = request(&h.store, h.goal_id, sequence);
        req.runtime_id = RuntimeId("reviewer".into());
        req.role = CognitiveRole::Reviewer;
        req.escalation_runtime_id = None;
        let outcome = h
            .coordinator
            .execute_one(req, CancellationToken::new())
            .await
            .unwrap();
        if sequence == 1 {
            assert!(matches!(
                outcome,
                AttemptCoordinationOutcome::Failed {
                    decision: RetryDecision::RetrySame { .. },
                    ..
                }
            ));
            resume_running(&h.store, h.goal_id);
        } else {
            let AttemptCoordinationOutcome::Failed { decision, goal, .. } = outcome else {
                panic!("expected reviewer failure")
            };
            assert!(matches!(decision, RetryDecision::AwaitHuman { .. }));
            assert_eq!(goal.state, GoalState::AwaitingHuman);
        }
    }
}

#[tokio::test]
async fn settled_attempt_budget_blocks_a_second_runtime_call() {
    let h = harness(
        GoalBudget {
            max_attempts: 1,
            ..default_budget()
        },
        vec![
            Err(failure(FailureClass::ProviderTransient, true)),
            Ok(success()),
        ],
    );
    h.coordinator
        .execute_one(request(&h.store, h.goal_id, 1), CancellationToken::new())
        .await
        .unwrap();
    resume_running(&h.store, h.goal_id);
    let error = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 2), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, AttemptCoordinatorError::Budget(_)));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn begin_persistence_failure_revokes_budget_before_any_runtime_call() {
    let h = harness(
        GoalBudget {
            max_attempts: 1,
            ..default_budget()
        },
        vec![Ok(success())],
    );
    h.store
        .lock()
        .unwrap()
        .begin_attempt(
            h.goal_id,
            1,
            &RuntimeId("worker".into()),
            CognitiveRole::Worker,
            &serde_json::json!({"preexisting": true}),
        )
        .unwrap();
    let duplicate = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 1), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(duplicate, AttemptCoordinatorError::Persistence(_)));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);

    // The failed reservation was revoked, so a new sequence can still reserve.
    let outcome = h
        .coordinator
        .execute_one(request(&h.store, h.goal_id, 2), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AttemptCoordinationOutcome::Succeeded { .. }
    ));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}
