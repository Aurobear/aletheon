//! Optional GBrain supplemental-memory contracts.

pub mod config;
pub mod migrations;
pub mod page;
pub mod spool;

pub use config::{validate_tools_list, GbrainBackendConfig, RetryPolicy, SpoolPolicy};
pub use page::{GbrainPage, PAGE_SCHEMA_VERSION};

pub use spool::{
    ClaimedPage, DeadLetter, EnqueueOutcome, GbrainSpool, MigrationReport, RetryOutcome,
    SpoolError, SpoolLimits,
};
