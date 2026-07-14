//! Durable Google synchronization state and dispatch infrastructure.

pub mod store;

pub use store::{
    CommitEventOutcome, GoogleSyncCursor, GoogleSyncStore, ProjectionWrite, SyncCommit,
    SyncCommitOutcome, SyncStoreError, SyncStream,
};
