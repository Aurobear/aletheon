//! Binary composition adapters for the Application-owned workspace checkpoint service.

pub use application::workspace_checkpoint::{
    CheckpointTurnContext, InMemoryCheckpointStore, RewindSafetyGuard,
    WorkspaceCheckpointMetricSample, WorkspaceCheckpointMetrics, WorkspaceCheckpointService,
    WorkspaceLeasePort,
};

use async_trait::async_trait;
use std::sync::Arc;

pub struct KernelWorkspaceLeasePort {
    inner: Arc<dyn kernel::LeaseManager>,
}

impl KernelWorkspaceLeasePort {
    pub fn new(inner: Arc<dyn kernel::LeaseManager>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl WorkspaceLeasePort for KernelWorkspaceLeasePort {
    async fn acquire(
        &self,
        principal: &str,
        request: &contracts::LeaseRequest,
        now_ms: u64,
    ) -> Result<contracts::ResourceLeaseId, String> {
        self.inner
            .acquire(principal, request, now_ms)
            .await
            .map_err(|error| error.to_string())
    }

    async fn release(&self, lease_id: contracts::ResourceLeaseId) {
        self.inner.release(lease_id).await;
    }
}

pub struct LiveAgentRewindGuard(pub Arc<crate::composition::agent_control::LiveAgentRuns>);

#[async_trait]
impl RewindSafetyGuard for LiveAgentRewindGuard {
    async fn permits_single_agent_rewind(&self) -> bool {
        self.0.all().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{LeaseRequest, PrincipalId, ResourceLeaseId, SchemaId};
    use anyhow::Result;
    use application::workspace_checkpoint::{
        CaptureResult, CheckpointFileEntry, CheckpointFinalizeState, CheckpointId, RestoreOutcome,
        WorkspaceCheckpointStore, WorkspaceFilePort, WorkspaceIdentity,
    };
    use runtime::event_projection::CanonicalEventBus;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct TestLeases {
        held: AtomicBool,
    }

    fn checkpoint_service(
        store: Arc<dyn WorkspaceCheckpointStore>,
        leases: Arc<dyn WorkspaceLeasePort>,
        enabled: bool,
    ) -> WorkspaceCheckpointService {
        WorkspaceCheckpointService::new(
            store,
            leases,
            Arc::new(platform::workspace_checkpoint::LocalWorkspaceFilePort),
            enabled,
        )
    }

    struct FailingIo {
        fail_protect: bool,
        fail_restore: bool,
    }

    #[async_trait]
    impl WorkspaceFilePort for FailingIo {
        async fn capture(
            &self,
            workspace: &WorkspaceIdentity,
            writable_roots: &[PathBuf],
            limit: usize,
        ) -> Result<CaptureResult> {
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .capture(workspace, writable_roots, limit)
                .await
        }

        async fn protect_current(
            &self,
            workspace: &WorkspaceIdentity,
        ) -> Result<Vec<CheckpointFileEntry>> {
            if self.fail_protect {
                anyhow::bail!("protection failed");
            }
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .protect_current(workspace)
                .await
        }

        async fn restore(
            &self,
            workspace: &WorkspaceIdentity,
            target: &[CheckpointFileEntry],
            rollback: &[CheckpointFileEntry],
        ) -> Result<()> {
            if self.fail_restore {
                anyhow::bail!("restore failed");
            }
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .restore(workspace, target, rollback)
                .await
        }
    }

    struct BlockingIo {
        entered: Arc<Notify>,
        resume: Arc<Notify>,
    }

    struct DenyRewind;

    #[async_trait]
    impl RewindSafetyGuard for DenyRewind {
        async fn permits_single_agent_rewind(&self) -> bool {
            false
        }
    }

    #[async_trait]
    impl WorkspaceFilePort for BlockingIo {
        async fn capture(
            &self,
            workspace: &WorkspaceIdentity,
            writable_roots: &[PathBuf],
            limit: usize,
        ) -> Result<CaptureResult> {
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .capture(workspace, writable_roots, limit)
                .await
        }

        async fn protect_current(
            &self,
            workspace: &WorkspaceIdentity,
        ) -> Result<Vec<CheckpointFileEntry>> {
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .protect_current(workspace)
                .await
        }

        async fn restore(
            &self,
            workspace: &WorkspaceIdentity,
            target: &[CheckpointFileEntry],
            rollback: &[CheckpointFileEntry],
        ) -> Result<()> {
            self.entered.notify_one();
            self.resume.notified().await;
            platform::workspace_checkpoint::LocalWorkspaceFilePort
                .restore(workspace, target, rollback)
                .await
        }
    }

    #[async_trait]
    impl WorkspaceLeasePort for TestLeases {
        async fn acquire(
            &self,
            _principal: &str,
            _request: &LeaseRequest,
            _now_mono_ms: u64,
        ) -> std::result::Result<ResourceLeaseId, String> {
            if self.held.swap(true, Ordering::SeqCst) {
                return Err("lease unavailable".into());
            }
            Ok(ResourceLeaseId::new())
        }

        async fn release(&self, _lease_id: ResourceLeaseId) {
            self.held.store(false, Ordering::SeqCst);
        }
    }

    fn identity(path: &Path) -> WorkspaceIdentity {
        WorkspaceIdentity {
            canonical_path: std::fs::canonicalize(path).unwrap(),
            repo_fingerprint: None,
        }
    }

    fn context(path: &Path, prompt_index: u64) -> CheckpointTurnContext {
        CheckpointTurnContext {
            session_id: "session".into(),
            thread_id: "thread".into(),
            turn_id: format!("turn-{prompt_index}"),
            prompt_index,
            principal_id: PrincipalId("principal".into()),
            workspace: identity(path),
            writable_roots: vec![path.to_path_buf()],
            created_at_ms: 1,
        }
    }

    #[test]
    fn checkpoint_metric_export_has_fixed_aggregate_cardinality() {
        let service = checkpoint_service(
            Arc::new(InMemoryCheckpointStore::default()),
            Arc::new(TestLeases::default()),
            true,
        );
        let samples = service.metric_samples();
        assert_eq!(samples.len(), 6);
        assert_eq!(
            samples.map(|sample| sample.name),
            [
                "checkpoint_files_captured_total",
                "checkpoint_disk_bytes",
                "rewind_partial_total",
                "rewind_identity_mismatch_total",
                "checkpoint_quota_rejected_total",
                "checkpoint_startup_aborted_total",
            ]
        );
    }

    #[tokio::test]
    async fn disk_quota_records_terminal_aborted_checkpoint_without_blob_bytes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("large"), "123456").unwrap();
        let store = Arc::new(InMemoryCheckpointStore::default());
        let service = checkpoint_service(store.clone(), Arc::new(TestLeases::default()), true)
            .with_disk_quota(5);

        let checkpoint_id = service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .unwrap();
        let (checkpoint, files) = store.load_by_id(checkpoint_id).await.unwrap().unwrap();
        assert_eq!(checkpoint.finalize_state, CheckpointFinalizeState::Aborted);
        assert!(files.is_empty());
        assert_eq!(store.stored_bytes().await.unwrap(), 0);
        let metrics = service.metrics();
        assert_eq!(metrics.checkpoint_disk_bytes, 0);
        assert_eq!(metrics.checkpoint_quota_rejected_total, 1);
    }

    #[tokio::test]
    async fn capture_finalize_and_rewind_restores_add_modify_delete() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("modified"), "before").unwrap();
        std::fs::write(directory.path().join("deleted"), "restore-me").unwrap();
        let store = Arc::new(InMemoryCheckpointStore::default());
        let bus = Arc::new(CanonicalEventBus::new(8));
        let mut began = bus.subscribe_channel(SchemaId::from(
            SchemaId::EVENT_WORKSPACE_CHECKPOINT_BEGAN_V1,
        ));
        let mut finalized = bus.subscribe_channel(SchemaId::from(
            SchemaId::EVENT_WORKSPACE_CHECKPOINT_FINALIZED_V1,
        ));
        let mut rewound =
            bus.subscribe_channel(SchemaId::from(SchemaId::EVENT_WORKSPACE_REWOUND_V1));
        let spine = Arc::new(
            adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").expect("event spine"),
        );
        let service = checkpoint_service(store.clone(), Arc::new(TestLeases::default()), true)
            .with_events(Some(bus), Some(spine.clone()));
        let checkpoint_id = service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .unwrap();
        service.finalize_turn(checkpoint_id, true).await.unwrap();
        let (mut future, future_files) = store.load("session", 1).await.unwrap().unwrap();
        future.checkpoint_id = CheckpointId::new();
        future.prompt_index = 2;
        future.turn_id = "turn-2".into();
        future.seal_integrity(&future_files);
        store.begin(future, future_files).await.unwrap();

        std::fs::write(directory.path().join("modified"), "after").unwrap();
        std::fs::remove_file(directory.path().join("deleted")).unwrap();
        std::fs::write(directory.path().join("added"), "remove-me").unwrap();
        let outcome = service
            .rewind_to(
                &PrincipalId("principal".into()),
                "session",
                1,
                &identity(directory.path()),
                0,
            )
            .await;

        assert_eq!(outcome, RestoreOutcome::Completed);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("modified")).unwrap(),
            "before"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("deleted")).unwrap(),
            "restore-me"
        );
        assert!(!directory.path().join("added").exists());
        assert!(store.load("session", 2).await.unwrap().is_none());
        for event in [
            began.recv().await.unwrap(),
            finalized.recv().await.unwrap(),
            rewound.recv().await.unwrap(),
        ] {
            assert_eq!(event.target.0, "thread:thread");
            assert_eq!(event.namespace.0, "session:session");
        }
        assert_eq!(spine.metrics().accepted, 3);
        assert_eq!(service.metrics().files_captured, 2);
        assert_eq!(service.metrics().checkpoint_disk_bytes, 16);
    }

    #[tokio::test]
    async fn identity_mismatch_and_unfinalized_checkpoint_make_zero_changes() {
        let directory = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file"), "before").unwrap();
        let service = checkpoint_service(
            Arc::new(InMemoryCheckpointStore::default()),
            Arc::new(TestLeases::default()),
            true,
        );
        let id = service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .unwrap();
        service.finalize_turn(id, true).await.unwrap();
        std::fs::write(directory.path().join("file"), "after").unwrap();

        assert_eq!(
            service
                .rewind_to(
                    &PrincipalId("principal".into()),
                    "session",
                    1,
                    &identity(other.path()),
                    0,
                )
                .await,
            RestoreOutcome::IdentityMismatch
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("file")).unwrap(),
            "after"
        );
    }

    #[tokio::test]
    async fn disabled_mode_is_a_strict_capture_bypass() {
        let directory = tempfile::tempdir().unwrap();
        let service = checkpoint_service(
            Arc::new(InMemoryCheckpointStore::default()),
            Arc::new(TestLeases::default()),
            false,
        );
        assert!(service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn every_turn_result_leaves_a_terminal_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(InMemoryCheckpointStore::default());
        let service = checkpoint_service(store.clone(), Arc::new(TestLeases::default()), true);

        for (prompt_index, succeeded, expected) in [
            (1, true, CheckpointFinalizeState::Finalized),
            (2, false, CheckpointFinalizeState::Aborted),
        ] {
            let id = service
                .begin_turn(context(directory.path(), prompt_index))
                .await
                .unwrap()
                .unwrap();
            service.finalize_turn(id, succeeded).await.unwrap();
            let (checkpoint, _) = store.load_by_id(id).await.unwrap().unwrap();
            assert_eq!(checkpoint.finalize_state, expected);
            assert_ne!(checkpoint.finalize_state, CheckpointFinalizeState::Open);
        }
    }

    #[tokio::test]
    async fn bounded_capture_is_deterministic_and_reports_truncation() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["c", "a", "b"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        let captured = platform::workspace_checkpoint::LocalWorkspaceFilePort
            .capture(
                &identity(directory.path()),
                &[directory.path().to_path_buf()],
                2,
            )
            .await
            .unwrap();
        assert!(captured.truncated);
        assert_eq!(
            captured
                .files
                .iter()
                .map(|entry| entry.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[tokio::test]
    async fn u_chk_006_partial_restore_failure_preserves_retry_evidence() {
        for (fail_protect, fail_restore, expected) in [
            (true, false, RestoreOutcome::UnprotectedChangesAbort),
            (
                false,
                true,
                RestoreOutcome::FsRestoreFailed {
                    detail: "restore failed".into(),
                },
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join("file"), "one").unwrap();
            let store = Arc::new(InMemoryCheckpointStore::default());
            let service = WorkspaceCheckpointService::new(
                store.clone(),
                Arc::new(TestLeases::default()),
                Arc::new(FailingIo {
                    fail_protect,
                    fail_restore,
                }),
                true,
            );
            for index in [1, 2] {
                let id = service
                    .begin_turn(context(directory.path(), index))
                    .await
                    .unwrap()
                    .unwrap();
                service.finalize_turn(id, true).await.unwrap();
            }

            let outcome = service
                .rewind_to(
                    &PrincipalId("principal".into()),
                    "session",
                    1,
                    &identity(directory.path()),
                    0,
                )
                .await;
            match (&outcome, &expected) {
                (
                    RestoreOutcome::FsRestoreFailed { detail },
                    RestoreOutcome::FsRestoreFailed { .. },
                ) => assert!(detail.contains("restore failed")),
                _ => assert_eq!(outcome, expected),
            }
            assert!(store.load("session", 2).await.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn exclusive_lease_rejects_a_second_rewind_while_restore_is_active() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file"), "one").unwrap();
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let service = Arc::new(WorkspaceCheckpointService::new(
            Arc::new(InMemoryCheckpointStore::default()),
            Arc::new(TestLeases::default()),
            Arc::new(BlockingIo {
                entered: entered.clone(),
                resume: resume.clone(),
            }),
            true,
        ));
        let id = service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .unwrap();
        service.finalize_turn(id, true).await.unwrap();
        let workspace = identity(directory.path());
        let first = {
            let service = service.clone();
            let workspace = workspace.clone();
            tokio::spawn(async move {
                service
                    .rewind_to(
                        &PrincipalId("principal".into()),
                        "session",
                        1,
                        &workspace,
                        0,
                    )
                    .await
            })
        };
        entered.notified().await;
        let second = service
            .rewind_to(
                &PrincipalId("principal".into()),
                "session",
                1,
                &workspace,
                0,
            )
            .await;
        assert!(matches!(
            second,
            RestoreOutcome::FsRestoreFailed { detail } if detail.contains("lease unavailable")
        ));
        resume.notify_one();
        assert_eq!(first.await.unwrap(), RestoreOutcome::Completed);
    }

    #[tokio::test]
    async fn active_child_guard_rejects_rewind_before_workspace_changes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file"), "before").unwrap();
        let service = checkpoint_service(
            Arc::new(InMemoryCheckpointStore::default()),
            Arc::new(TestLeases::default()),
            true,
        )
        .with_safety_guard(Arc::new(DenyRewind));
        let id = service
            .begin_turn(context(directory.path(), 1))
            .await
            .unwrap()
            .unwrap();
        service.finalize_turn(id, true).await.unwrap();
        std::fs::write(directory.path().join("file"), "after").unwrap();

        let outcome = service
            .rewind_to(
                &PrincipalId("principal".into()),
                "session",
                1,
                &identity(directory.path()),
                0,
            )
            .await;
        assert!(matches!(
            outcome,
            RestoreOutcome::FsRestoreFailed { detail } if detail.contains("child agent")
        ));
        assert_eq!(
            std::fs::read_to_string(directory.path().join("file")).unwrap(),
            "after"
        );
    }
}
