//! Durable Google synchronization state and dispatch infrastructure.

pub mod event_dispatcher;
pub mod store;
pub mod sync_manager;

pub use event_dispatcher::{
    DispatchOutcome, GoogleCurrentTaskProjection, GoogleEventDispatcher, GoogleEventRouter,
    GoogleEventSink, GoogleMemoryProposalSink,
};
pub use sync_manager::{
    CalendarDeltaPoller, DriveChangesPoller, GmailHistoryPoller, GooglePollBatch,
    GooglePollFailure, GoogleSyncHandle, GoogleSyncManager, GoogleSyncManagerConfig,
    GoogleSyncPoller, GoogleSyncRegistration,
};

pub use store::{
    CommitEventOutcome, GoogleOutboxClaim, GoogleSubscription, GoogleSubscriptionQuery,
    GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit, SyncCommitOutcome,
    SyncStoreError, SyncStream,
};
