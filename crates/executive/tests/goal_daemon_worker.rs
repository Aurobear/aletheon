use async_trait::async_trait;
use executive::core::sub_agent::SubAgentRuntime;
use executive::core::RuntimeRegistry;
use executive::r#impl::goal::{GoalWorker, ObjectiveStore};
use fabric::{GoalBudget, GoalSpec, GoalState, PrincipalId, RuntimeId};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct ReportingRuntime(mpsc::UnboundedSender<&'static str>, &'static str);

#[async_trait]
impl SubAgentRuntime for ReportingRuntime {
    async fn run(&self, _task: &str, _cancel: CancellationToken) -> Result<String, String> {
        self.0.send(self.1).unwrap();
        Ok("done".into())
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
    let mut registry = RuntimeRegistry::new();
    for (id, label) in [("worker", "worker"), ("reviewer", "reviewer")] {
        registry
            .register(
                RuntimeId(id.into()),
                Arc::new(ReportingRuntime(calls_tx.clone(), label)),
            )
            .unwrap();
    }
    let (progress_tx, mut progress_rx) = mpsc::channel(4);
    let worker = GoalWorker::new(
        store.clone(),
        Arc::new(registry),
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
