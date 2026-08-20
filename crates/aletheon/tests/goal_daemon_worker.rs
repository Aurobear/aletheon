#![cfg(feature = "test-support")]
use ::contracts::{
    GoalBudget, GoalSpec, GoalState, PrincipalId, RuntimeFailure, RuntimeId, RuntimeResult,
};
use adapters_sqlite::goal::ObjectiveStore;
use application::goal::AttemptCoordinator;
use application::goal_attempt::GoalAttemptPort;
use application::goal_retry::RetryPolicy;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct ReportingRuntime(mpsc::UnboundedSender<&'static str>, &'static str);

#[async_trait]
impl GoalAttemptPort for ReportingRuntime {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        runtime_id.0 == self.1
    }

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        _task: &str,
        _cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        if !self.is_available(runtime_id) {
            return Err(RuntimeFailure {
                class: ::contracts::FailureClass::MissingDependency,
                message: format!("runtime unavailable: {}", runtime_id.0),
                retryable: false,
                usage: ::contracts::AttemptUsage::default(),
                evidence: vec![],
            });
        }
        self.0.send(self.1).unwrap();
        Ok(RuntimeResult {
            output: "done".into(),
            ..RuntimeResult::default()
        })
    }
}

#[tokio::test]
async fn ready_goal_is_started_then_exactly_one_runtime_attempt_is_executed() {
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
                original_intent: "finish the durable goal".into(),
                desired_state: vec![],
                constraints: vec![],
                acceptance_criteria: vec![],
                budget: GoalBudget::default(),
            },
        )
        .unwrap();

    let (calls_tx, mut calls_rx) = mpsc::unbounded_channel();
    let (progress_tx, mut progress_rx) = mpsc::channel(4);
    let attempts = Arc::new(AttemptCoordinator::new(
        Arc::new(adapters_sqlite::goal::SqliteGoalAttemptPersistence::new(
            store.clone(),
        )),
        Arc::new(ReportingRuntime(calls_tx.clone(), "worker")),
        Arc::new(kernel::chronos::SystemClock::new()),
        RetryPolicy::default(),
    ));
    let coordinator = Arc::new(application::goal::GoalCoordinator::new(Arc::new(
        adapters_sqlite::goal::SqliteGoalCoordinatorRepository::new(store.clone()),
    )));
    let worker = application::goal::GoalAdvanceService::new(
        Arc::new(adapters_sqlite::goal::SqliteGoalAdvanceRepository::new(
            store.clone(),
        )),
        coordinator,
        attempts,
        RuntimeId("worker".into()),
        RuntimeId("reviewer".into()),
        Arc::new(aletheon::adapters::goal_progress::GatewayGoalProgressAdapter::new(progress_tx)),
    );

    assert!(worker
        .advance_once(0, CancellationToken::new())
        .await
        .unwrap());
    assert_eq!(
        store
            .lock()
            .unwrap()
            .get_goal(goal.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Running
    );
    assert!(calls_rx.try_recv().is_err());

    assert!(worker
        .advance_once(0, CancellationToken::new())
        .await
        .unwrap());
    assert_eq!(calls_rx.recv().await.unwrap(), "worker");
    let progress = progress_rx.recv().await.unwrap();
    assert_eq!(progress.goal_id, goal.id);
    assert_eq!(
        store
            .lock()
            .unwrap()
            .attempts_for_goal(goal.id, 10)
            .unwrap()
            .len(),
        1
    );
}
