//! Product-neutral supplemental-memory contracts and durable spool.

pub mod backend;
pub mod config;
pub mod migrations;
pub mod page;
pub mod reconcile;
pub mod spool;

pub use config::{validate_tools_list, SupplementalBackendConfig, RetryPolicy, SpoolPolicy};
pub use page::{SupplementalDocument, PAGE_SCHEMA_VERSION};
pub use reconcile::{
    SupplementalReconciliation, SupplementalReconciliationService, ReconcileOperation, ReconcileOperationKind,
    ReconciliationDrainReport, RemoteMemoryReceipt, RECONCILIATION_SCHEMA_VERSION,
};

pub use spool::{
    ClaimedPage, DeadLetter, EnqueueOutcome, SupplementalSpool, MigrationReport, RetryOutcome,
    SpoolError, SpoolLimits,
};

pub use backend::{
    SupplementalMemoryBackend, SupplementalMemoryError, SupplementalErrorCategory, SupplementalHit,
    SupplementalMemoryTransport, SupplementalRecall, SupplementalRecallHealth,
    SupplementalTransportError,
};
