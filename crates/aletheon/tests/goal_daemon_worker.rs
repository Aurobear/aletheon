use ::contracts::{
    GoalBudget, GoalSpec, GoalState, PrincipalId, RuntimeFailure, RuntimeId, RuntimeResult,
};
use async_trait::async_trait;
use aletheon::wiring::application::goal::{AttemptExecutor, GoalWorker, ObjectiveStore};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct ReportingRuntime(mpsc::UnboundedSender<&'static str>, &'static str);

#[async_trait]
impl AttemptExecutor for ReportingRuntime {
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
    let worker = GoalWorker::new_with_executor(
        store.clone(),
        Arc::new(ReportingRuntime(calls_tx.clone(), "worker")),
        RuntimeId("worker".into()),
        RuntimeId("reviewer".into()),
        progress_tx,
    );

    assert!(worker.tick_once(CancellationToken::new()).await.unwrap());
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

    assert!(worker.tick_once(CancellationToken::new()).await.unwrap());
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
