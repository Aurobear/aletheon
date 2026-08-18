//! G4 workspace checkpoint capture and transactional rewind orchestration.

use runtime::event_projection::CanonicalEventBus;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::{
    CheckpointFileEntry, CheckpointFinalizeState, CheckpointId, FsDomainRef, RestoreOutcome,
    TurnCheckpoint, WorkspaceCheckpointStore, WorkspaceFilePort, WorkspaceIdentity,
    MAX_CHECKPOINT_FILES,
};
use ::contracts::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, MessageId, NamespaceId, SchemaId,
};
use ::contracts::{LeaseRequest, PrincipalId};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use runtime::{
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility,
    UnsequencedEvent,
};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const REWIND_LEASE_MS: u64 = 60_000;

#[async_trait]
pub trait WorkspaceLeasePort: Send + Sync {
    async fn acquire(
        &self,
        principal: &str,
        request: &LeaseRequest,
        now_ms: u64,
    ) -> std::result::Result<::contracts::ResourceLeaseId, String>;

    async fn release(&self, lease_id: ::contracts::ResourceLeaseId);
}

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

#[derive(Default)]
pub struct InMemoryCheckpointStore {
    records: Mutex<CheckpointRecords>,
}

type StoredCheckpoint = (TurnCheckpoint, Vec<CheckpointFileEntry>);
type CheckpointRecords = BTreeMap<(String, u64), StoredCheckpoint>;

#[async_trait]
impl WorkspaceCheckpointStore for InMemoryCheckpointStore {
    async fn begin(
        &self,
        checkpoint: TurnCheckpoint,
        files: Vec<CheckpointFileEntry>,
    ) -> Result<()> {
        anyhow::ensure!(
            checkpoint.verify_integrity(&files),
            "checkpoint integrity verification failed"
        );
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
        anyhow::ensure!(
            checkpoint.0.verify_integrity(&checkpoint.1),
            "checkpoint integrity verification failed"
        );
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
        let loaded = self
            .records
            .lock()
            .await
            .get(&(session.to_owned(), prompt_index))
            .cloned();
        if let Some((checkpoint, files)) = &loaded {
            anyhow::ensure!(
                checkpoint.verify_integrity(files),
                "checkpoint integrity verification failed"
            );
        }
        Ok(loaded)
    }

    async fn load_by_id(
        &self,
        id: CheckpointId,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>> {
        let loaded = self
            .records
            .lock()
            .await
            .values()
            .find(|(checkpoint, _)| checkpoint.checkpoint_id == id)
            .cloned();
        if let Some((checkpoint, files)) = &loaded {
            anyhow::ensure!(
                checkpoint.verify_integrity(files),
                "checkpoint integrity verification failed"
            );
        }
        Ok(loaded)
    }

    async fn list(&self, session: &str, limit: usize) -> Result<Vec<TurnCheckpoint>> {
        let records = self.records.lock().await;
        let mut checkpoints = records
            .iter()
            .filter(|((record_session, _), _)| record_session == session)
            .rev()
            .take(limit)
            .map(|(_, (checkpoint, files))| {
                anyhow::ensure!(
                    checkpoint.verify_integrity(files),
                    "checkpoint integrity verification failed"
                );
                Ok(checkpoint.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        checkpoints.reverse();
        Ok(checkpoints)
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

    async fn stored_bytes(&self) -> Result<u64> {
        Ok(self
            .records
            .lock()
            .await
            .values()
            .flat_map(|(_, files)| files)
            .filter_map(|entry| entry.content.as_ref())
            .map(|content| content.len() as u64)
            .sum())
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceCheckpointMetrics {
    pub files_captured: u64,
    pub checkpoint_disk_bytes: u64,
    pub rewind_partial_total: u64,
    pub rewind_identity_mismatch_total: u64,
    pub checkpoint_quota_rejected_total: u64,
    pub checkpoint_startup_aborted_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceCheckpointMetricSample {
    pub name: &'static str,
    pub value: u64,
}

/// Narrow consumer port for workspace-checkpoint listing used by the daemon
/// transport (session checkpoint RPC).
#[async_trait]
pub trait WorkspaceCheckpointPort: Send + Sync {
    async fn list_session_checkpoints(
        &self,
        session: &str,
        limit: usize,
    ) -> Result<Vec<TurnCheckpoint>>;
}

#[async_trait]
impl WorkspaceCheckpointPort for WorkspaceCheckpointService {
    async fn list_session_checkpoints(
        &self,
        session: &str,
        limit: usize,
    ) -> Result<Vec<TurnCheckpoint>> {
        WorkspaceCheckpointService::list_session_checkpoints(self, session, limit).await
    }
}

pub struct WorkspaceCheckpointService {
    store: Arc<dyn WorkspaceCheckpointStore>,
    leases: Arc<dyn WorkspaceLeasePort>,
    io: Arc<dyn WorkspaceFilePort>,
    safety_guard: Arc<dyn RewindSafetyGuard>,
    event_bus: Option<Arc<CanonicalEventBus>>,
    event_spine: Option<Arc<dyn EventSpine>>,
    feature_enabled: bool,
    max_disk_bytes: u64,
    quota_lock: Mutex<()>,
    files_captured: AtomicU64,
    checkpoint_bytes: AtomicU64,
    rewind_partial: AtomicU64,
    identity_mismatch: AtomicU64,
    quota_rejected: AtomicU64,
    startup_aborted: AtomicU64,
}

impl WorkspaceCheckpointService {
    pub fn new(
        store: Arc<dyn WorkspaceCheckpointStore>,
        leases: Arc<dyn WorkspaceLeasePort>,
        io: Arc<dyn WorkspaceFilePort>,
        feature_enabled: bool,
    ) -> Self {
        let startup_aborted = store.startup_reconciled_open();
        let startup_stored_bytes = store.startup_stored_bytes();
        Self {
            store,
            leases,
            io,
            safety_guard: Arc::new(AllowSingleAgentRewind),
            event_bus: None,
            event_spine: None,
            feature_enabled,
            max_disk_bytes: u64::MAX,
            quota_lock: Mutex::new(()),
            files_captured: AtomicU64::new(0),
            checkpoint_bytes: AtomicU64::new(startup_stored_bytes),
            rewind_partial: AtomicU64::new(0),
            identity_mismatch: AtomicU64::new(0),
            quota_rejected: AtomicU64::new(0),
            startup_aborted: AtomicU64::new(startup_aborted),
        }
    }

    pub fn with_disk_quota(mut self, max_disk_bytes: u64) -> Self {
        self.max_disk_bytes = max_disk_bytes;
        self
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
        let _quota_guard = self.quota_lock.lock().await;
        let mut capture = self
            .io
            .capture(
                &context.workspace,
                &context.writable_roots,
                MAX_CHECKPOINT_FILES,
            )
            .await?;
        let captured_bytes = capture
            .files
            .iter()
            .filter_map(|entry| entry.content.as_ref())
            .map(|content| content.len() as u64)
            .sum::<u64>();
        let stored_bytes = self.store.stored_bytes().await?;
        let quota_exceeded = stored_bytes.saturating_add(captured_bytes) > self.max_disk_bytes;
        if quota_exceeded {
            self.quota_rejected.fetch_add(1, Ordering::Relaxed);
            warn!(
                session = %context.session_id,
                stored_bytes,
                captured_bytes,
                max_disk_bytes = self.max_disk_bytes,
                "workspace checkpoint disk quota exceeded; recording aborted checkpoint"
            );
            capture.files.clear();
        }
        let checkpoint_id = CheckpointId::new();
        let finalize_state = if capture.truncated || quota_exceeded {
            warn!(
                session = %context.session_id,
                prompt_index = context.prompt_index,
                limit = MAX_CHECKPOINT_FILES,
                quota_exceeded,
                capture_truncated = capture.truncated,
                "workspace checkpoint capture was aborted by configured bounds"
            );
            CheckpointFinalizeState::Aborted
        } else {
            CheckpointFinalizeState::Open
        };
        let mut checkpoint = TurnCheckpoint {
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
            integrity_digest: String::new(),
            finalize_state,
        };
        checkpoint.seal_integrity(&capture.files);
        self.files_captured
            .fetch_add(capture.files.len() as u64, Ordering::Relaxed);
        self.store.begin(checkpoint.clone(), capture.files).await?;
        self.checkpoint_bytes
            .store(self.store.stored_bytes().await?, Ordering::Relaxed);
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

    pub async fn list_session_checkpoints(
        &self,
        session: &str,
        limit: usize,
    ) -> Result<Vec<TurnCheckpoint>> {
        anyhow::ensure!(
            (1..=256).contains(&limit),
            "checkpoint list limit must be 1..=256"
        );
        self.store.list(session, limit).await
    }

    pub async fn rewind_to(
        &self,
        principal: &PrincipalId,
        session: &str,
        prompt_index: u64,
        current_workspace: &WorkspaceIdentity,
        now_mono_ms: u64,
    ) -> RestoreOutcome {
        tracing::info!(
            event = "workspace.rewind.stage",
            stage = "begin",
            outcome = "started",
            session,
            prompt_index,
            "workspace rewind started"
        );
        if !self.feature_enabled {
            tracing::warn!(
                event = "workspace.rewind.stage",
                stage = "feature_gate",
                outcome = "disabled",
                session,
                prompt_index,
                "workspace rewind rejected"
            );
            return RestoreOutcome::FsRestoreFailed {
                detail: "workspace checkpoint feature is disabled".into(),
            };
        }
        if !self.safety_guard.permits_single_agent_rewind().await {
            tracing::warn!(
                event = "workspace.rewind.stage",
                stage = "agent_safety",
                outcome = "rejected",
                session,
                prompt_index,
                "workspace rewind rejected while child agent is active"
            );
            return RestoreOutcome::FsRestoreFailed {
                detail: "workspace rewind rejected while a child agent is active".into(),
            };
        }
        let loaded = match self.store.load(session, prompt_index).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                tracing::warn!(
                    event = "workspace.rewind.stage",
                    stage = "load",
                    outcome = "not_found",
                    session,
                    prompt_index,
                    "workspace checkpoint load failed"
                );
                return RestoreOutcome::FsRestoreFailed {
                    detail: "checkpoint not found".into(),
                };
            }
            Err(error) => {
                tracing::warn!(event = "workspace.rewind.stage", stage = "load", outcome = "failed", session, prompt_index, %error, "workspace checkpoint load failed");
                return RestoreOutcome::FsRestoreFailed {
                    detail: error.to_string(),
                };
            }
        };
        let (checkpoint, files) = loaded;
        if !checkpoint.verify_integrity(&files) {
            tracing::warn!(
                event = "workspace.rewind.stage",
                stage = "integrity",
                outcome = "failed",
                session,
                prompt_index,
                "workspace checkpoint integrity rejected"
            );
            return RestoreOutcome::FsRestoreFailed {
                detail: "checkpoint integrity verification failed".into(),
            };
        }
        if checkpoint.finalize_state != CheckpointFinalizeState::Finalized {
            tracing::warn!(event = "workspace.rewind.stage", stage = "finalize_state", outcome = "rejected", session, prompt_index, state = ?checkpoint.finalize_state, "workspace checkpoint is not restorable");
            return RestoreOutcome::FsRestoreFailed {
                detail: "checkpoint is not finalized".into(),
            };
        }
        if !checkpoint.workspace.matches(current_workspace) {
            self.identity_mismatch.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                event = "workspace.rewind.stage",
                stage = "identity",
                outcome = "mismatch",
                session,
                prompt_index,
                "workspace identity mismatch"
            );
            return RestoreOutcome::IdentityMismatch;
        }
        tracing::info!(
            event = "workspace.rewind.stage",
            stage = "identity",
            outcome = "passed",
            session,
            prompt_index,
            "workspace identity verified"
        );

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
                tracing::warn!(event = "workspace.rewind.stage", stage = "lease", outcome = "failed", session, prompt_index, %error, "workspace rewind lease unavailable");
                return RestoreOutcome::FsRestoreFailed {
                    detail: format!("workspace rewind lease unavailable: {error}"),
                };
            }
        };

        let outcome = match self.io.protect_current(current_workspace).await {
            Err(error) => {
                tracing::warn!(event = "workspace.rewind.stage", stage = "protect_current", outcome = "failed", session, prompt_index, %error, "current workspace could not be protected");
                RestoreOutcome::UnprotectedChangesAbort
            }
            Ok(rollback) => {
                match self.io.restore(current_workspace, &files, &rollback).await {
                    Err(error) => {
                        tracing::warn!(event = "workspace.rewind.stage", stage = "fs_restore", outcome = "failed_checkpoints_retained", session, prompt_index, %error, "workspace restore failed; checkpoints retained");
                        RestoreOutcome::FsRestoreFailed {
                            detail: error.to_string(),
                        }
                    }
                    Ok(()) => {
                        match self.store.truncate_after(session, prompt_index).await {
                            Ok(()) => {
                                if let Ok(bytes) = self.store.stored_bytes().await {
                                    self.checkpoint_bytes.store(bytes, Ordering::Relaxed);
                                }
                                tracing::info!(event = "workspace.rewind.stage", stage = "truncate", outcome = "completed", session, prompt_index, "future workspace checkpoints truncated after successful restore");
                                RestoreOutcome::Completed
                            }
                            Err(error) => {
                                self.rewind_partial.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(event = "workspace.rewind.stage", stage = "truncate", outcome = "partial", session, prompt_index, %error, "workspace restored but checkpoint truncation failed");
                                RestoreOutcome::Partial {
                                    detail: format!(
                                "workspace restored but checkpoint truncation failed: {error}"
                            ),
                                }
                            }
                        }
                    }
                }
            }
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
        tracing::info!(
            event = "workspace.rewind.stage",
            stage = "complete",
            outcome = ?outcome,
            session,
            prompt_index,
            "workspace rewind finished"
        );
        outcome
    }

    pub fn metrics(&self) -> WorkspaceCheckpointMetrics {
        WorkspaceCheckpointMetrics {
            files_captured: self.files_captured.load(Ordering::Relaxed),
            checkpoint_disk_bytes: self.checkpoint_bytes.load(Ordering::Relaxed),
            rewind_partial_total: self.rewind_partial.load(Ordering::Relaxed),
            rewind_identity_mismatch_total: self.identity_mismatch.load(Ordering::Relaxed),
            checkpoint_quota_rejected_total: self.quota_rejected.load(Ordering::Relaxed),
            checkpoint_startup_aborted_total: self.startup_aborted.load(Ordering::Relaxed),
        }
    }

    /// Fixed-cardinality aggregate export. No principal/session/thread/workspace
    /// labels are admitted, preventing unbounded metric series.
    pub fn metric_samples(&self) -> [WorkspaceCheckpointMetricSample; 6] {
        let metrics = self.metrics();
        [
            WorkspaceCheckpointMetricSample {
                name: "checkpoint_files_captured_total",
                value: metrics.files_captured,
            },
            WorkspaceCheckpointMetricSample {
                name: "checkpoint_disk_bytes",
                value: metrics.checkpoint_disk_bytes,
            },
            WorkspaceCheckpointMetricSample {
                name: "rewind_partial_total",
                value: metrics.rewind_partial_total,
            },
            WorkspaceCheckpointMetricSample {
                name: "rewind_identity_mismatch_total",
                value: metrics.rewind_identity_mismatch_total,
            },
            WorkspaceCheckpointMetricSample {
                name: "checkpoint_quota_rejected_total",
                value: metrics.checkpoint_quota_rejected_total,
            },
            WorkspaceCheckpointMetricSample {
                name: "checkpoint_startup_aborted_total",
                value: metrics.checkpoint_startup_aborted_total,
            },
        ]
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
