//! Optional GBrain supplemental-memory contracts.

pub mod config;
pub mod page;

pub use config::{validate_tools_list, GbrainBackendConfig, RetryPolicy, SpoolPolicy};
pub use page::{GbrainPage, PAGE_SCHEMA_VERSION};
