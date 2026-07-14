use async_trait::async_trait;
use executive::core::sub_agent::SubAgentRuntime;
use executive::core::{RuntimeRegistry, SubAgentSpawner};
use fabric::RuntimeId;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct ReportingRuntime {
    name: &'static str,
    reports: mpsc::UnboundedSender<&'static str>,
}

#[async_trait]
impl SubAgentRuntime for ReportingRuntime {
    async fn run(&self, _task: &str, _cancel: CancellationToken) -> Result<String, String> {
        self.reports.send(self.name).unwrap();
        Ok(self.name.into())
    }
}

fn runtime(
    name: &'static str,
    reports: &mpsc::UnboundedSender<&'static str>,
) -> Arc<dyn SubAgentRuntime> {
    Arc::new(ReportingRuntime {
        name,
        reports: reports.clone(),
    })
}

#[test]
fn registry_rejects_duplicate_and_missing_ids() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let id = RuntimeId("worker".into());
    let mut registry = RuntimeRegistry::new();
    registry.register(id.clone(), runtime("one", &tx)).unwrap();

    assert!(registry.register(id.clone(), runtime("two", &tx)).is_err());
    assert!(registry.resolve(&RuntimeId("missing".into())).is_err());
    assert!(registry
        .register(RuntimeId(" ".into()), runtime("empty", &tx))
        .is_err());
}

#[tokio::test]
async fn two_spawns_execute_distinct_selected_runtimes() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let worker_id = RuntimeId("worker".into());
    let reviewer_id = RuntimeId("reviewer".into());
    let mut spawner = SubAgentSpawner::new();
    spawner
        .runtime_registry_mut()
        .register(worker_id.clone(), runtime("worker", &tx))
        .unwrap();
    spawner
        .runtime_registry_mut()
        .register(reviewer_id.clone(), runtime("reviewer", &tx))
        .unwrap();

    let worker = spawner
        .spawn_with_runtime(
            "work".into(),
            "turn".into(),
            worker_id.clone(),
            aletheon_kernel::supervision::RestartPolicy::Never,
        )
        .await
        .unwrap();
    let reviewer = spawner
        .spawn_with_runtime(
            "review".into(),
            "turn".into(),
            reviewer_id.clone(),
            aletheon_kernel::supervision::RestartPolicy::Never,
        )
        .await
        .unwrap();

    let mut reports = vec![rx.recv().await.unwrap(), rx.recv().await.unwrap()];
    reports.sort_unstable();
    assert_eq!(reports, ["reviewer", "worker"]);
    assert_eq!(spawner.runtime_id(&worker.id), Some(&worker_id));
    assert_eq!(spawner.runtime_id(&reviewer.id), Some(&reviewer_id));
}

#[tokio::test]
async fn missing_runtime_fails_before_process_records_are_created() {
    let mut spawner = SubAgentSpawner::new();
    let error = spawner
        .spawn_with_runtime(
            "work".into(),
            "turn".into(),
            RuntimeId("missing".into()),
            aletheon_kernel::supervision::RestartPolicy::Never,
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("runtime not registered"));
    assert!(spawner.list().is_empty());
}

#[tokio::test]
async fn with_runtime_preserves_default_spawn_compatibility() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut spawner = SubAgentSpawner::new();
    spawner.with_runtime(runtime("default", &tx));

    let handle = spawner.spawn("work".into(), "turn".into()).await.unwrap();

    assert_eq!(rx.recv().await.unwrap(), "default");
    assert_eq!(
        spawner.runtime_id(&handle.id),
        Some(&RuntimeRegistry::default_id())
    );
}

#[tokio::test]
async fn supervisor_restart_reuses_the_original_runtime_id() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let runtime_id = RuntimeId("worker".into());
    let mut spawner = SubAgentSpawner::new();
    spawner
        .runtime_registry_mut()
        .register(runtime_id.clone(), runtime("worker", &tx))
        .unwrap();
    let original = spawner
        .spawn_with_runtime(
            "work".into(),
            "turn".into(),
            runtime_id.clone(),
            aletheon_kernel::supervision::RestartPolicy::RestartOnFailure { max_restarts: 1 },
        )
        .await
        .unwrap();
    assert_eq!(rx.recv().await.unwrap(), "worker");

    spawner
        .transition(&original.id, fabric::SubAgentState::Running)
        .await
        .unwrap();
    spawner
        .transition(&original.id, fabric::SubAgentState::Failed)
        .await
        .unwrap();
    assert_eq!(rx.recv().await.unwrap(), "worker");

    let replacement = spawner
        .list()
        .into_iter()
        .find(|entry| entry.id != original.id)
        .expect("restart creates replacement");
    assert_eq!(spawner.runtime_id(&replacement.id), Some(&runtime_id));
}
