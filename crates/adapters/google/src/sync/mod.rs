//! Durable Google synchronization state, dispatch, and sync-manager adapters.

pub mod event_dispatcher;
pub mod store;
pub mod sync_manager;

pub use event_dispatcher::{
    DispatchOutcome, DurableGoogleNotificationSink, GoogleCurrentTaskProjection,
    GoogleEventCapabilityHandler, GoogleEventDispatcher, GoogleEventRouter, GoogleEventSink,
    GoogleMemoryProposalSink, GoogleNotificationSink,
};
pub use store::{
    CommitEventOutcome, GoogleOutboxClaim, GoogleSubscription, GoogleSubscriptionQuery,
    GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit, SyncCommitOutcome,
    SyncStoreError, SyncStream,
};
pub use sync_manager::{
    CalendarDeltaPoller, DriveChangesPoller, GmailHistoryPoller, GooglePollBatch,
    GooglePollFailure, GoogleSyncHandle, GoogleSyncManager, GoogleSyncManagerConfig,
    GoogleSyncPoller, GoogleSyncRegistration,
};
