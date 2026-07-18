//! G4 workspace checkpoint capture and transactional rewind orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use fabric::types::workspace_checkpoint::{
    CheckpointFileEntry, CheckpointFinalizeState, CheckpointId, FsDomainRef, RestoreOutcome,
    TurnCheckpoint, WorkspaceIdentity, MAX_CHECKPOINT_FILES,
};
use fabric::{
    CanonicalEventBus, EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity,
    EventPayload, EventSpine, EventTreeId, EventVisibility, MessageId, NamespaceId, SchemaId,
    UnsequencedEvent,
};
use fabric::{LeaseManager, LeaseRequest, PrincipalId};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const REWIND_LEASE_MS: u64 = 60_000;

#[derive(Debug, Clone)]
pub struct CheckpointTurnContext {
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub prompt_index: u64,
    pub principal_id: PrincipalId,
    pub workspace: WorkspaceIdentity,
    pub writable_roots: Vec<PathBuf>,
    pub created_at_ms: i64,
}

#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn begin(
        &self,
        checkpoint: TurnCheckpoint,
        files: Vec<CheckpointFileEntry>,
    ) -> Result<()>;
    async fn finalize(&self, id: CheckpointId, state: CheckpointFinalizeState) -> Result<()>;
    async fn load(
        &self,
        session: &str,
        prompt_index: u64,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>>;
    async fn load_by_id(
        &self,
        id: CheckpointId,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>>;
    async fn truncate_after(&self, session: &str, prompt_index: u64) -> Result<()>;
}

#[derive(Default)]
pub struct InMemoryCheckpointStore {
    records: Mutex<CheckpointRecords>,
}

type StoredCheckpoint = (TurnCheckpoint, Vec<CheckpointFileEntry>);
type CheckpointRecords = BTreeMap<(String, u64), StoredCheckpoint>;

#[async_trait]
impl CheckpointStore for InMemoryCheckpointStore {
    async fn begin(
        &self,
        checkpoint: TurnCheckpoint,
        files: Vec<CheckpointFileEntry>,
    ) -> Result<()> {
        let key = (checkpoint.session_id.clone(), checkpoint.prompt_index);
        let mut records = self.records.lock().await;
        anyhow::ensure!(!records.contains_key(&key), "checkpoint already exists");
        records.insert(key, (checkpoint, files));
        Ok(())
    }

    async fn finalize(&self, id: CheckpointId, state: CheckpointFinalizeState) -> Result<()> {
        let mut records = self.records.lock().await;
        let checkpoint = records
            .values_mut()
            .find(|(checkpoint, _)| checkpoint.checkpoint_id == id)
            .ok_or_else(|| anyhow!("checkpoint not found"))?;
        if checkpoint.0.finalize_state == CheckpointFinalizeState::Open {
            checkpoint.0.finalize_state = state;
        }
        Ok(())
    }

    async fn load(
        &self,
        session: &str,
        prompt_index: u64,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>> {
        Ok(self
            .records
            .lock()
            .await
            .get(&(session.to_owned(), prompt_index))
            .cloned())
    }

    async fn load_by_id(
        &self,
        id: CheckpointId,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>> {
        Ok(self
            .records
            .lock()
            .await
            .values()
            .find(|(checkpoint, _)| checkpoint.checkpoint_id == id)
            .cloned())
    }

    async fn truncate_after(&self, session: &str, prompt_index: u64) -> Result<()> {
        self.records
            .lock()
            .await
            .retain(|(record_session, index), _| {
                record_session != session || *index <= prompt_index
            });
        Ok(())
    }
}

#[async_trait]
pub trait WorkspaceSnapshotIo: Send + Sync {
    async fn capture(
        &self,
        workspace: &WorkspaceIdentity,
        writable_roots: &[PathBuf],
        limit: usize,
    ) -> Result<CaptureResult>;
    async fn protect_current(
        &self,
        workspace: &WorkspaceIdentity,
    ) -> Result<Vec<CheckpointFileEntry>>;
    async fn restore(
        &self,
        workspace: &WorkspaceIdentity,
        target: &[CheckpointFileEntry],
        rollback: &[CheckpointFileEntry],
    ) -> Result<()>;
}

#[async_trait]
pub trait RewindSafetyGuard: Send + Sync {
    async fn permits_single_agent_rewind(&self) -> bool;
}

#[derive(Default)]
struct AllowSingleAgentRewind;

#[async_trait]
impl RewindSafetyGuard for AllowSingleAgentRewind {
    async fn permits_single_agent_rewind(&self) -> bool {
        true
    }
}

#[async_trait]
impl RewindSafetyGuard for crate::service::agent_control::LiveAgentRuns {
    async fn permits_single_agent_rewind(&self) -> bool {
        self.all().await.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub files: Vec<CheckpointFileEntry>,
    pub truncated: bool,
}

#[derive(Default)]
pub struct LocalWorkspaceSnapshotIo;

#[async_trait]
impl WorkspaceSnapshotIo for LocalWorkspaceSnapshotIo {
    async fn capture(
        &self,
        workspace: &WorkspaceIdentity,
        writable_roots: &[PathBuf],
        limit: usize,
    ) -> Result<CaptureResult> {
        capture_files(workspace, writable_roots, limit)
    }

    async fn protect_current(
        &self,
        workspace: &WorkspaceIdentity,
    ) -> Result<Vec<CheckpointFileEntry>> {
        let captured = capture_files(
            workspace,
            std::slice::from_ref(&workspace.canonical_path),
            usize::MAX,
        )?;
        Ok(captured.files)
    }

    async fn restore(
        &self,
        workspace: &WorkspaceIdentity,
        target: &[CheckpointFileEntry],
        rollback: &[CheckpointFileEntry],
    ) -> Result<()> {
        if let Err(error) = apply_snapshot(&workspace.canonical_path, target) {
            let rollback_result = apply_snapshot(&workspace.canonical_path, rollback);
            return match rollback_result {
                Ok(()) => Err(error.context("restore failed; current workspace rolled back")),
                Err(rollback_error) => Err(anyhow!(
                    "restore failed ({error:#}); rollback also failed ({rollback_error:#})"
                )),
            };
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceCheckpointMetrics {
    pub files_captured: u64,
    pub checkpoint_disk_bytes: u64,
    pub rewind_partial_total: u64,
    pub rewind_identity_mismatch_total: u64,
}

pub struct WorkspaceCheckpointService {
    store: Arc<dyn CheckpointStore>,
    leases: Arc<dyn LeaseManager>,
    io: Arc<dyn WorkspaceSnapshotIo>,
    safety_guard: Arc<dyn RewindSafetyGuard>,
    event_bus: Option<Arc<CanonicalEventBus>>,
    event_spine: Option<Arc<dyn EventSpine>>,
    feature_enabled: bool,
    files_captured: AtomicU64,
    checkpoint_bytes: AtomicU64,
    rewind_partial: AtomicU64,
    identity_mismatch: AtomicU64,
}

impl WorkspaceCheckpointService {
    pub fn new(
        store: Arc<dyn CheckpointStore>,
        leases: Arc<dyn LeaseManager>,
        feature_enabled: bool,
    ) -> Self {
        Self::with_io(
            store,
            leases,
            Arc::new(LocalWorkspaceSnapshotIo),
            feature_enabled,
        )
    }

    pub fn with_io(
        store: Arc<dyn CheckpointStore>,
        leases: Arc<dyn LeaseManager>,
        io: Arc<dyn WorkspaceSnapshotIo>,
        feature_enabled: bool,
    ) -> Self {
        Self {
            store,
            leases,
            io,
            safety_guard: Arc::new(AllowSingleAgentRewind),
            event_bus: None,
            event_spine: None,
            feature_enabled,
            files_captured: AtomicU64::new(0),
            checkpoint_bytes: AtomicU64::new(0),
            rewind_partial: AtomicU64::new(0),
            identity_mismatch: AtomicU64::new(0),
        }
    }

    pub fn with_events(
        mut self,
        event_bus: Option<Arc<CanonicalEventBus>>,
        event_spine: Option<Arc<dyn EventSpine>>,
    ) -> Self {
        self.event_bus = event_bus;
        self.event_spine = event_spine;
        self
    }

    pub fn with_safety_guard(mut self, guard: Arc<dyn RewindSafetyGuard>) -> Self {
        self.safety_guard = guard;
        self
    }

    pub async fn begin_turn(&self, context: CheckpointTurnContext) -> Result<Option<CheckpointId>> {
        if !self.feature_enabled {
            return Ok(None);
        }
        let capture = self
            .io
            .capture(
                &context.workspace,
                &context.writable_roots,
                MAX_CHECKPOINT_FILES,
            )
            .await?;
        let checkpoint_id = CheckpointId::new();
        let finalize_state = if capture.truncated {
            warn!(
                session = %context.session_id,
                prompt_index = context.prompt_index,
                limit = MAX_CHECKPOINT_FILES,
                "workspace checkpoint exceeded file limit and was aborted"
            );
            CheckpointFinalizeState::Aborted
        } else {
            CheckpointFinalizeState::Open
        };
        let checkpoint = TurnCheckpoint {
            checkpoint_id,
            session_id: context.session_id,
            thread_id: context.thread_id,
            turn_id: context.turn_id,
            prompt_index: context.prompt_index,
            workspace: context.workspace,
            fs_domain: FsDomainRef {
                batch_id: Uuid::new_v4(),
                file_count: capture.files.len(),
            },
            vcs_domain_ref: None,
            patch_domain_ref: None,
            runtime_checkpoint_ref: None,
            created_at_ms: context.created_at_ms,
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            finalize_state,
        };
        let captured_bytes = capture
            .files
            .iter()
            .filter_map(|entry| entry.content.as_ref())
            .map(|content| content.len() as u64)
            .sum::<u64>();
        self.files_captured
            .fetch_add(capture.files.len() as u64, Ordering::Relaxed);
        self.checkpoint_bytes
            .fetch_add(captured_bytes, Ordering::Relaxed);
        self.store.begin(checkpoint.clone(), capture.files).await?;
        self.publish_event(
            SchemaId::EVENT_WORKSPACE_CHECKPOINT_BEGAN_V1,
            &checkpoint,
            serde_json::json!({
                "checkpoint_id": checkpoint_id.0,
                "file_count": checkpoint.fs_domain.file_count,
                "turn_id": checkpoint.turn_id,
                "state": checkpoint.finalize_state,
            }),
        )
        .await;
        Ok(Some(checkpoint_id))
    }

    pub async fn finalize_turn(&self, id: CheckpointId, succeeded: bool) -> Result<()> {
        if !self.feature_enabled {
            return Ok(());
        }
        let state = if succeeded {
            CheckpointFinalizeState::Finalized
        } else {
            CheckpointFinalizeState::Aborted
        };
        self.store.finalize(id, state).await?;
        if let Some((checkpoint, _)) = self.store.load_by_id(id).await? {
            self.publish_event(
                SchemaId::EVENT_WORKSPACE_CHECKPOINT_FINALIZED_V1,
                &checkpoint,
                serde_json::json!({
                    "checkpoint_id": id.0,
                    "turn_id": checkpoint.turn_id,
                    "state": checkpoint.finalize_state,
                }),
            )
            .await;
        }
        Ok(())
    }

    pub async fn rewind_to(
        &self,
        principal: &PrincipalId,
        session: &str,
        prompt_index: u64,
        current_workspace: &WorkspaceIdentity,
        now_mono_ms: u64,
    ) -> RestoreOutcome {
        if !self.feature_enabled {
            return RestoreOutcome::FsRestoreFailed {
                detail: "workspace checkpoint feature is disabled".into(),
            };
        }
        if !self.safety_guard.permits_single_agent_rewind().await {
            return RestoreOutcome::FsRestoreFailed {
                detail: "workspace rewind rejected while a child agent is active".into(),
            };
        }
        let loaded = match self.store.load(session, prompt_index).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                return RestoreOutcome::FsRestoreFailed {
                    detail: "checkpoint not found".into(),
                };
            }
            Err(error) => {
                return RestoreOutcome::FsRestoreFailed {
                    detail: error.to_string(),
                };
            }
        };
        let (checkpoint, files) = loaded;
        if checkpoint.finalize_state != CheckpointFinalizeState::Finalized {
            return RestoreOutcome::FsRestoreFailed {
                detail: "checkpoint is not finalized".into(),
            };
        }
        if !checkpoint.workspace.matches(current_workspace) {
            self.identity_mismatch.fetch_add(1, Ordering::Relaxed);
            return RestoreOutcome::IdentityMismatch;
        }

        let resource = format!(
            "workspace-rewind:{}",
            current_workspace.canonical_path.display()
        );
        let lease = match self
            .leases
            .acquire(
                &principal.0,
                &LeaseRequest {
                    resource,
                    duration_ms: REWIND_LEASE_MS,
                },
                now_mono_ms,
            )
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                return RestoreOutcome::FsRestoreFailed {
                    detail: format!("workspace rewind lease unavailable: {error}"),
                };
            }
        };

        let outcome = match self.io.protect_current(current_workspace).await {
            Err(_) => RestoreOutcome::UnprotectedChangesAbort,
            Ok(rollback) => match self.io.restore(current_workspace, &files, &rollback).await {
                Err(error) => RestoreOutcome::FsRestoreFailed {
                    detail: error.to_string(),
                },
                Ok(()) => match self.store.truncate_after(session, prompt_index).await {
                    Ok(()) => RestoreOutcome::Completed,
                    Err(error) => {
                        self.rewind_partial.fetch_add(1, Ordering::Relaxed);
                        RestoreOutcome::Partial {
                            detail: format!(
                                "workspace restored but checkpoint truncation failed: {error}"
                            ),
                        }
                    }
                },
            },
        };
        self.leases.release(lease).await;
        self.publish_event(
            SchemaId::EVENT_WORKSPACE_REWOUND_V1,
            &checkpoint,
            serde_json::json!({
                "checkpoint_id": checkpoint.checkpoint_id.0,
                "from_prompt_index": prompt_index,
                "outcome": outcome,
            }),
        )
        .await;
        outcome
    }

    pub fn metrics(&self) -> WorkspaceCheckpointMetrics {
        WorkspaceCheckpointMetrics {
            files_captured: self.files_captured.load(Ordering::Relaxed),
            checkpoint_disk_bytes: self.checkpoint_bytes.load(Ordering::Relaxed),
            rewind_partial_total: self.rewind_partial.load(Ordering::Relaxed),
            rewind_identity_mismatch_total: self.identity_mismatch.load(Ordering::Relaxed),
        }
    }

    async fn publish_event(
        &self,
        schema: &str,
        checkpoint: &TurnCheckpoint,
        payload: serde_json::Value,
    ) {
        if self.event_bus.is_none() && self.event_spine.is_none() {
            return;
        }
        let event_id = EventId::new();
        let mut envelope = EnvelopeV2::new(
            SchemaId::from(schema),
            EnvelopeV2Target("executive:workspace-checkpoint".into()),
            EnvelopeV2Target(format!("thread:{}", checkpoint.thread_id)),
            EnvelopeV2Delivery::FanOut,
            NamespaceId(format!("session:{}", checkpoint.session_id)),
            payload.clone(),
        );
        envelope.id = MessageId(event_id.0);
        if let Some(event_spine) = &self.event_spine {
            let _ = event_spine.append(UnsequencedEvent {
                tree_id: EventTreeId::for_root_session(&checkpoint.session_id),
                event_id,
                parent: None,
                identity: EventIdentity {
                    root_session_id: checkpoint.session_id.clone(),
                    session_id: checkpoint.session_id.clone(),
                    agent_id: None,
                },
                envelope: envelope.clone(),
                visibility: EventVisibility::Control,
                payload: EventPayload::Inline { value: payload },
            });
        }
        if let Some(event_bus) = &self.event_bus {
            let _ = event_bus.publish(envelope).await;
        }
    }
}

fn capture_files(
    workspace: &WorkspaceIdentity,
    writable_roots: &[PathBuf],
    limit: usize,
) -> Result<CaptureResult> {
    let root = canonical_directory(&workspace.canonical_path)?;
    let mut files = BTreeMap::new();
    let mut truncated = false;
    for writable_root in writable_roots {
        let writable_root = canonical_directory(writable_root)?;
        anyhow::ensure!(
            writable_root.starts_with(&root),
            "writable root escapes checkpoint workspace"
        );
        walk_files(&root, &writable_root, limit, &mut files, &mut truncated)?;
        if truncated {
            break;
        }
    }
    Ok(CaptureResult {
        files: files.into_values().collect(),
        truncated,
    })
}

fn walk_files(
    workspace: &Path,
    directory: &Path,
    limit: usize,
    files: &mut BTreeMap<PathBuf, CheckpointFileEntry>,
    truncated: &mut bool,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("read checkpoint directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk_files(workspace, &path, limit, files, truncated)?;
            if *truncated {
                return Ok(());
            }
        } else if file_type.is_file() {
            if files.len() >= limit {
                *truncated = true;
                return Ok(());
            }
            let relative = path
                .strip_prefix(workspace)
                .context("checkpoint file escaped workspace")?
                .to_path_buf();
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("read checkpoint file {}", path.display()))?;
            files.insert(
                relative.clone(),
                CheckpointFileEntry {
                    path: relative,
                    content: Some(content),
                },
            );
        }
    }
    Ok(())
}

fn apply_snapshot(workspace: &Path, target: &[CheckpointFileEntry]) -> Result<()> {
    let workspace = canonical_directory(workspace)?;
    let target_paths = target
        .iter()
        .map(|entry| validated_target(&workspace, &entry.path).map(|_| entry.path.clone()))
        .collect::<Result<BTreeSet<_>>>()?;
    let current = capture_files(
        &WorkspaceIdentity {
            canonical_path: workspace.clone(),
            repo_fingerprint: None,
        },
        std::slice::from_ref(&workspace),
        usize::MAX,
    )?;
    for entry in current.files {
        if !target_paths.contains(&entry.path) {
            std::fs::remove_file(validated_target(&workspace, &entry.path)?)?;
        }
    }
    for entry in target {
        let destination = validated_target(&workspace, &entry.path)?;
        match &entry.content {
            None => {
                if destination.exists() {
                    std::fs::remove_file(destination)?;
                }
            }
            Some(content) => {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let temporary =
                    destination.with_extension(format!("aletheon-rewind-{}.tmp", Uuid::new_v4()));
                std::fs::write(&temporary, content)?;
                std::fs::rename(temporary, destination)?;
            }
        }
    }
    Ok(())
}

fn validated_target(workspace: &Path, relative: &Path) -> Result<PathBuf> {
    anyhow::ensure!(relative.is_relative(), "checkpoint path must be relative");
    anyhow::ensure!(
        !relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)),
        "checkpoint path contains parent traversal"
    );
    Ok(workspace.join(relative))
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize checkpoint root {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "checkpoint root is not a directory");
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{AdmissionError, ResourceLeaseId};
    use std::sync::atomic::AtomicBool;
    use tokio::sync::Notify;

    #[derive(Default)]
    struct TestLeases {
        held: AtomicBool,
    }

    struct FailingIo {
        fail_protect: bool,
        fail_restore: bool,
    }

    #[async_trait]
    impl WorkspaceSnapshotIo for FailingIo {
        async fn capture(
            &self,
            workspace: &WorkspaceIdentity,
            writable_roots: &[PathBuf],
            limit: usize,
        ) -> Result<CaptureResult> {
            LocalWorkspaceSnapshotIo
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
            LocalWorkspaceSnapshotIo.protect_current(workspace).await
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
            LocalWorkspaceSnapshotIo
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
    impl WorkspaceSnapshotIo for BlockingIo {
        async fn capture(
            &self,
            workspace: &WorkspaceIdentity,
            writable_roots: &[PathBuf],
            limit: usize,
        ) -> Result<CaptureResult> {
            LocalWorkspaceSnapshotIo
                .capture(workspace, writable_roots, limit)
                .await
        }

        async fn protect_current(
            &self,
            workspace: &WorkspaceIdentity,
        ) -> Result<Vec<CheckpointFileEntry>> {
            LocalWorkspaceSnapshotIo.protect_current(workspace).await
        }

        async fn restore(
            &self,
            workspace: &WorkspaceIdentity,
            target: &[CheckpointFileEntry],
            rollback: &[CheckpointFileEntry],
        ) -> Result<()> {
            self.entered.notify_one();
            self.resume.notified().await;
            LocalWorkspaceSnapshotIo
                .restore(workspace, target, rollback)
                .await
        }
    }

    #[async_trait]
    impl LeaseManager for TestLeases {
        async fn acquire(
            &self,
            _principal: &str,
            _request: &LeaseRequest,
            _now_mono_ms: u64,
        ) -> std::result::Result<ResourceLeaseId, AdmissionError> {
            if self.held.swap(true, Ordering::SeqCst) {
                return Err(AdmissionError::LeaseUnavailable);
            }
            Ok(ResourceLeaseId::new())
        }

        async fn release(&self, _lease_id: ResourceLeaseId) {
            self.held.store(false, Ordering::SeqCst);
        }

        async fn is_leased(&self, _resource: &str, _now_mono_ms: u64) -> bool {
            self.held.load(Ordering::SeqCst)
        }

        async fn active_count(&self, _now_mono_ms: u64) -> usize {
            usize::from(self.held.load(Ordering::SeqCst))
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
            crate::r#impl::events::SqliteEventSpine::open(":memory:").expect("event spine"),
        );
        let service =
            WorkspaceCheckpointService::new(store.clone(), Arc::new(TestLeases::default()), true)
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
        let service = WorkspaceCheckpointService::new(
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
        let service = WorkspaceCheckpointService::new(
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
        let service =
            WorkspaceCheckpointService::new(store.clone(), Arc::new(TestLeases::default()), true);

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

    #[test]
    fn bounded_capture_is_deterministic_and_reports_truncation() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["c", "a", "b"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        let captured = capture_files(
            &identity(directory.path()),
            &[directory.path().to_path_buf()],
            2,
        )
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
    async fn protection_and_restore_failures_do_not_truncate_future_checkpoints() {
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
            let service = WorkspaceCheckpointService::with_io(
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
        let service = Arc::new(WorkspaceCheckpointService::with_io(
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
        let service = WorkspaceCheckpointService::new(
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
