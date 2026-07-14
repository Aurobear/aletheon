//! Durable Google synchronization state and dispatch infrastructure.

pub mod event_dispatcher;
pub mod store;
pub mod sync_manager;

pub use event_dispatcher::{DispatchOutcome, GoogleEventDispatcher, GoogleEventSink};
pub use sync_manager::{
    CalendarDeltaPoller, DriveChangesPoller, GmailHistoryPoller, GooglePollBatch,
    GooglePollFailure, GoogleSyncHandle, GoogleSyncManager, GoogleSyncManagerConfig,
    GoogleSyncPoller, GoogleSyncRegistration,
};

pub use store::{
    CommitEventOutcome, GoogleOutboxClaim, GoogleSyncCursor, GoogleSyncStore, ProjectionWrite,
    SyncCommit, SyncCommitOutcome, SyncStoreError, SyncStream,
};
