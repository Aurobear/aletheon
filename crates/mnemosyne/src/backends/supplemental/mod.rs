//! Product-neutral supplemental-memory contracts and durable spool.

pub mod backend;
pub mod config;
pub mod migrations;
pub mod page;
pub mod reconcile;
pub mod spool;

pub use config::{validate_tools_list, RetryPolicy, SpoolPolicy, SupplementalBackendConfig};
pub use page::{SupplementalDocument, PAGE_SCHEMA_VERSION};
pub use reconcile::{
    ReconcileOperation, ReconcileOperationKind, ReconciliationDrainReport, RemoteMemoryReceipt,
    SupplementalReconciliation, SupplementalReconciliationService, RECONCILIATION_SCHEMA_VERSION,
};

pub use spool::{
    ClaimedPage, DeadLetter, EnqueueOutcome, MigrationReport, RetryOutcome, SpoolError,
    SpoolLimits, SupplementalSpool,
};

pub use backend::{
    SupplementalErrorCategory, SupplementalHit, SupplementalMemoryBackend, SupplementalMemoryError,
    SupplementalMemoryTransport, SupplementalRecall, SupplementalRecallHealth,
    SupplementalTransportError,
};
