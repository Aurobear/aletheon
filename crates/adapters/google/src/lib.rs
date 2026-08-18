//! Google/Gmail sync store, Gmail channel ingress, and bounded classification
//! adapters. Owns the Google sync cursor/projection/outbox SQLite schema via the
//! shared migration runner; host glue (worker admin ports, external identity
//! lifecycle, google use-case facades) stays in the `aletheon` composition.

pub mod gmail;
pub mod gmail_classification;
pub mod sync;

pub use gmail::{
    load_gmail_ingress_policies, GmailGoalDraft, GmailGoalDraftCoordinator, GmailGoalEventIngress,
    GmailIngressPolicy,
};
pub use sync::event_dispatcher::{
    DispatchOutcome, DurableGoogleNotificationSink, GoogleCurrentTaskProjection,
    GoogleEventCapabilityHandler, GoogleEventDispatcher, GoogleEventRouter, GoogleEventSink,
    GoogleMemoryProposalSink, GoogleNotificationSink,
};
pub use sync::store::{
    CommitEventOutcome, GoogleOutboxClaim, GoogleSubscription, GoogleSubscriptionQuery,
    GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit, SyncCommitOutcome,
    SyncStoreError, SyncStream,
};
pub use sync::sync_manager::{
    CalendarDeltaPoller, DriveChangesPoller, GmailHistoryPoller, GooglePollBatch,
    GooglePollFailure, GoogleSyncHandle, GoogleSyncManager, GoogleSyncManagerConfig,
    GoogleSyncPoller, GoogleSyncRegistration,
};
