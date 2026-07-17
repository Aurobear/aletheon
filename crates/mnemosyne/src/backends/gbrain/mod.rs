//! Optional GBrain supplemental-memory contracts.

pub mod backend;
pub mod config;
pub mod migrations;
pub mod page;
pub mod reconcile;
pub mod spool;

pub use config::{validate_tools_list, GbrainBackendConfig, RetryPolicy, SpoolPolicy};
pub use page::{GbrainPage, PAGE_SCHEMA_VERSION};
pub use reconcile::{
    GbrainReconciliation, GbrainReconciliationService, ReconcileOperation, ReconcileOperationKind,
    ReconciliationDrainReport, RemoteMemoryReceipt, RECONCILIATION_SCHEMA_VERSION,
};

pub use spool::{
    ClaimedPage, DeadLetter, EnqueueOutcome, GbrainSpool, MigrationReport, RetryOutcome,
    SpoolError, SpoolLimits,
};

pub use backend::{
    GbrainBackend, GbrainBackendError, SupplementalErrorCategory, SupplementalHit,
    SupplementalMemoryTransport, SupplementalRecall, SupplementalRecallHealth,
    SupplementalTransportError,
};
