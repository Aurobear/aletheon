//! Supervised, leased, restart-safe Google synchronization poll loops.

use super::{
    GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit, SyncStoreError, SyncStream,
};
use async_trait::async_trait;
use fabric::{Clock, ExternalEventEnvelope, ExternalIdentityId, PrincipalId};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub struct GmailHistoryPoller(pub corpus::tools::google::GmailHistorySynchronizer);
pub struct CalendarDeltaPoller(pub corpus::tools::google::CalendarSynchronizer);
pub struct DriveChangesPoller(pub corpus::tools::google::DriveSynchronizer);

#[async_trait]
impl GoogleSyncPoller for GmailHistoryPoller {
    async fn poll(
        &self,
        principal: &PrincipalId,
        cursor: &GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure> {
        let batch = self
            .0
            .synchronize(
                principal,
                cursor.account_id,
                cursor.token.as_deref(),
                &std::collections::HashSet::new(),
                cancel,
            )
            .await
            .map_err(map_api_error)?;
        Ok(normalized_batch(
            batch.successor_cursor,
            cursor.generation + u64::from(batch.reconciled),
            batch.events,
        ))
    }
}

#[async_trait]
impl GoogleSyncPoller for CalendarDeltaPoller {
    async fn poll(
        &self,
        principal: &PrincipalId,
        cursor: &GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure> {
        let batch = self
            .0
            .synchronize(
                principal,
                cursor.account_id,
                cursor.token.as_deref(),
                &std::collections::HashSet::new(),
                cancel,
            )
            .await
            .map_err(map_api_error)?;
        Ok(normalized_batch(
            batch.successor_cursor,
            cursor.generation + u64::from(batch.reconciled),
            batch.events,
        ))
    }
}

#[async_trait]
impl GoogleSyncPoller for DriveChangesPoller {
    async fn poll(
        &self,
        principal: &PrincipalId,
        cursor: &GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure> {
        let batch = self
            .0
            .synchronize(
                principal,
                cursor.account_id,
                cursor.token.as_deref(),
                &std::collections::HashSet::new(),
                cancel,
            )
            .await
            .map_err(map_api_error)?;
        Ok(normalized_batch(
            batch.successor_cursor,
            cursor.generation + u64::from(batch.reconciled),
            batch.events,
        ))
    }
}

fn normalized_batch(
    successor_cursor: String,
    cursor_generation: u64,
    events: Vec<ExternalEventEnvelope>,
) -> GooglePollBatch {
    let events = events
        .into_iter()
        .map(|event| {
            let tombstone = matches!(
                event.event,
                fabric::GoogleEvent::MailDeleted(_)
                    | fabric::GoogleEvent::CalendarEventDeleted(_)
                    | fabric::GoogleEvent::DriveFileDeleted(_)
            );
            let json = serde_json::to_value(&event.event).unwrap_or(serde_json::Value::Null);
            (event, ProjectionWrite { json, tombstone })
        })
        .collect();
    GooglePollBatch {
        successor_cursor,
        cursor_generation,
        events,
    }
}

fn map_api_error(error: corpus::tools::google::GoogleApiError) -> GooglePollFailure {
    use corpus::tools::google::GoogleApiError;
    match error {
        GoogleApiError::ReauthorizationRequired
        | GoogleApiError::CredentialUnavailable
        | GoogleApiError::ScopeDenied
        | GoogleApiError::UnauthorizedAccount => GooglePollFailure::AuthRequired,
        GoogleApiError::Cancelled => GooglePollFailure::Cancelled,
        _ => GooglePollFailure::Transient {
            retry_after_ms: None,
        },
    }
}

#[derive(Debug, Clone)]
pub struct GooglePollBatch {
    pub successor_cursor: String,
    pub cursor_generation: u64,
    pub events: Vec<(ExternalEventEnvelope, ProjectionWrite)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GooglePollFailure {
    Transient { retry_after_ms: Option<i64> },
    AuthRequired,
    Revoked,
    Cancelled,
}

#[async_trait]
pub trait GoogleSyncPoller: Send + Sync {
    async fn poll(
        &self,
        principal: &PrincipalId,
        cursor: &GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure>;
}

#[derive(Clone)]
pub struct GoogleSyncRegistration {
    pub principal: PrincipalId,
    pub account_id: ExternalIdentityId,
    pub stream: SyncStream,
    pub initial_cursor: Option<String>,
    pub cursor_generation: u64,
    pub poller: Arc<dyn GoogleSyncPoller>,
}

#[derive(Debug, Clone)]
pub struct GoogleSyncManagerConfig {
    pub lease_duration_ms: i64,
    pub idle_poll_ms: i64,
    pub base_backoff_ms: i64,
    pub max_backoff_ms: i64,
    pub circuit_failure_threshold: u32,
}

impl Default for GoogleSyncManagerConfig {
    fn default() -> Self {
        Self {
            lease_duration_ms: 30_000,
            idle_poll_ms: 30_000,
            base_backoff_ms: 1_000,
            max_backoff_ms: 300_000,
            circuit_failure_threshold: 5,
        }
    }
}

impl GoogleSyncManagerConfig {
    fn validate(&self) -> Result<(), SyncStoreError> {
        if !(1_000..=300_000).contains(&self.lease_duration_ms)
            || !(10..=3_600_000).contains(&self.idle_poll_ms)
            || !(10..=300_000).contains(&self.base_backoff_ms)
            || self.max_backoff_ms < self.base_backoff_ms
            || self.max_backoff_ms > 3_600_000
            || !(1..=100).contains(&self.circuit_failure_threshold)
        {
            Err(SyncStoreError::InvalidInput)
        } else {
            Ok(())
        }
    }
}

pub struct GoogleSyncManager {
    store: Arc<Mutex<GoogleSyncStore>>,
    owner: String,
    clock: Arc<dyn Clock>,
    config: GoogleSyncManagerConfig,
    registrations: Vec<GoogleSyncRegistration>,
}

impl GoogleSyncManager {
    pub fn new(
        store: Arc<Mutex<GoogleSyncStore>>,
        owner: String,
        clock: Arc<dyn Clock>,
        config: GoogleSyncManagerConfig,
    ) -> Result<Self, SyncStoreError> {
        config.validate()?;
        if owner.is_empty() || owner.len() > 256 {
            return Err(SyncStoreError::InvalidInput);
        }
        Ok(Self {
            store,
            owner,
            clock,
            config,
            registrations: Vec::new(),
        })
    }

    pub fn register(&mut self, registration: GoogleSyncRegistration) -> Result<(), SyncStoreError> {
        if self.registrations.iter().any(|existing| {
            existing.account_id == registration.account_id && existing.stream == registration.stream
        }) {
            return Err(SyncStoreError::InvalidInput);
        }
        self.store.lock().unwrap().initialize_cursor(
            registration.account_id,
            registration.stream,
            registration.initial_cursor.as_deref(),
            registration.cursor_generation,
        )?;
        self.registrations.push(registration);
        Ok(())
    }

    pub fn start(self, parent_cancel: &CancellationToken) -> GoogleSyncHandle {
        let cancel = parent_cancel.child_token();
        let mut tasks = Vec::with_capacity(self.registrations.len());
        for registration in self.registrations {
            let worker = SyncWorker {
                store: self.store.clone(),
                owner: self.owner.clone(),
                clock: self.clock.clone(),
                config: self.config.clone(),
                registration,
            };
            let worker_cancel = cancel.child_token();
            tasks.push(tokio::spawn(async move { worker.run(worker_cancel).await }));
        }
        GoogleSyncHandle { cancel, tasks }
    }
}

pub struct GoogleSyncHandle {
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl GoogleSyncHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn spawn_supervised<F, Fut>(&mut self, worker: F)
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let cancel = self.cancel.child_token();
        self.tasks.push(tokio::spawn(worker(cancel)));
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

struct SyncWorker {
    store: Arc<Mutex<GoogleSyncStore>>,
    owner: String,
    clock: Arc<dyn Clock>,
    config: GoogleSyncManagerConfig,
    registration: GoogleSyncRegistration,
}

impl SyncWorker {
    async fn run(self, cancel: CancellationToken) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let cursor = match self
                .store
                .lock()
                .unwrap()
                .cursor(self.registration.account_id, self.registration.stream)
            {
                Ok(Some(cursor)) => cursor,
                _ => break,
            };
            if matches!(cursor.health_state.as_str(), "auth_required" | "revoked") {
                break;
            }
            let now = self.clock.wall_now().0.max(0);
            if let Some(due) = cursor.retry_after_ms.filter(|due| *due > now) {
                if sleep_or_cancel(due - now, &cancel).await {
                    break;
                }
                continue;
            }
            let acquired = self
                .store
                .lock()
                .unwrap()
                .acquire_lease(
                    self.registration.account_id,
                    self.registration.stream,
                    &self.owner,
                    now,
                    self.config.lease_duration_ms,
                )
                .unwrap_or(false);
            if !acquired {
                if sleep_or_cancel(self.config.base_backoff_ms, &cancel).await {
                    break;
                }
                continue;
            }
            let current = self
                .store
                .lock()
                .unwrap()
                .cursor(self.registration.account_id, self.registration.stream)
                .ok()
                .flatten();
            let Some(current) = current else {
                self.release();
                break;
            };
            let result = self
                .registration
                .poller
                .poll(&self.registration.principal, &current, &cancel)
                .await;
            let completed_at = self.clock.wall_now().0.max(0);
            match result {
                Ok(batch) => {
                    let committed = self
                        .store
                        .lock()
                        .unwrap()
                        .commit(SyncCommit {
                            account_id: self.registration.account_id,
                            stream: self.registration.stream,
                            expected_cursor_token: current.token,
                            expected_cursor_version: current.version,
                            successor_cursor_token: batch.successor_cursor,
                            cursor_generation: batch.cursor_generation,
                            events: batch.events,
                            committed_at_ms: completed_at,
                        })
                        .is_ok();
                    self.release();
                    let delay = if committed {
                        self.config.idle_poll_ms
                    } else {
                        self.config.base_backoff_ms
                    };
                    if sleep_or_cancel(delay, &cancel).await {
                        break;
                    }
                }
                Err(GooglePollFailure::Transient { retry_after_ms }) => {
                    let next_count = current.retry_count.saturating_add(1);
                    let delay = retry_after_ms
                        .filter(|delay| *delay >= 0)
                        .map(|delay| delay.min(self.config.max_backoff_ms))
                        .unwrap_or_else(|| self.backoff(next_count));
                    let state = if next_count >= self.config.circuit_failure_threshold {
                        "circuit_open"
                    } else {
                        "retrying"
                    };
                    let _ = self.store.lock().unwrap().record_sync_failure(
                        self.registration.account_id,
                        self.registration.stream,
                        completed_at,
                        Some(completed_at.saturating_add(delay)),
                        state,
                    );
                    self.release();
                }
                Err(GooglePollFailure::AuthRequired) => {
                    let _ = self.store.lock().unwrap().record_sync_failure(
                        self.registration.account_id,
                        self.registration.stream,
                        completed_at,
                        None,
                        "auth_required",
                    );
                    self.release();
                    break;
                }
                Err(GooglePollFailure::Revoked) => {
                    let _ = self.store.lock().unwrap().record_sync_failure(
                        self.registration.account_id,
                        self.registration.stream,
                        completed_at,
                        None,
                        "revoked",
                    );
                    self.release();
                    break;
                }
                Err(GooglePollFailure::Cancelled) => {
                    self.release();
                    break;
                }
            }
        }
        self.release();
    }

    fn release(&self) {
        let _ = self.store.lock().unwrap().release_lease(
            self.registration.account_id,
            self.registration.stream,
            &self.owner,
        );
    }

    fn backoff(&self, retry_count: u32) -> i64 {
        let exponent = retry_count.saturating_sub(1).min(20);
        let base = self
            .config
            .base_backoff_ms
            .saturating_mul(1_i64 << exponent)
            .min(self.config.max_backoff_ms);
        let material = format!(
            "{}:{}:{}",
            self.registration.account_id,
            self.registration.stream.as_str(),
            retry_count
        );
        let hash = material.bytes().fold(0_u64, |acc, byte| {
            acc.wrapping_mul(1099511628211)
                .wrapping_add(u64::from(byte))
        });
        let percent = i64::try_from(hash % 41).unwrap_or(20) - 20;
        base.saturating_add(base.saturating_mul(percent) / 100)
            .clamp(10, self.config.max_backoff_ms)
    }
}

async fn sleep_or_cancel(delay_ms: i64, cancel: &CancellationToken) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms.max(0) as u64)) => false,
    }
}
