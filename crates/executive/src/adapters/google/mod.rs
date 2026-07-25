//! Durable Google synchronization state and dispatch infrastructure.

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::application::admin_service::BackgroundWorkerPort;

pub mod event_dispatcher;
pub mod store;
pub mod sync_manager;

pub use event_dispatcher::{
    DispatchOutcome, DurableGoogleNotificationSink, GoogleCurrentTaskProjection,
    GoogleEventDispatcher, GoogleEventRouter, GoogleEventSink, GoogleMemoryProposalSink,
    GoogleNotificationSink,
};
pub use sync_manager::{
    CalendarDeltaPoller, DriveChangesPoller, GmailHistoryPoller, GooglePollBatch,
    GooglePollFailure, GoogleSyncHandle, GoogleSyncManager, GoogleSyncManagerConfig,
    GoogleSyncPoller, GoogleSyncRegistration,
};

pub struct GoogleSyncWorkerPort {
    handle: Mutex<Option<GoogleSyncHandle>>,
}

impl GoogleSyncWorkerPort {
    pub fn new(handle: GoogleSyncHandle) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
        }
    }
}

#[async_trait]
impl BackgroundWorkerPort for GoogleSyncWorkerPort {
    async fn is_running(&self) -> bool {
        self.handle.lock().await.is_some()
    }

    async fn shutdown(&self) {
        if let Some(handle) = self.handle.lock().await.take() {
            handle.shutdown().await;
        }
    }
}

pub use store::{
    CommitEventOutcome, GoogleOutboxClaim, GoogleSubscription, GoogleSubscriptionQuery,
    GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit, SyncCommitOutcome,
    SyncStoreError, SyncStream,
};
