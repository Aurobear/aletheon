//! Retired capability benchmark — one-way compatibility re-export.
//!
//! The `CapabilityRollupProjectionSink` implementation moved to
//! `application::capability_benchmark` (use-case owner). This module re-exports
//! the public surface so existing Aletheon-internal callers and compatibility
//! tests keep compiling while the cutover completes. No new implementation is
//! added here.

pub use adapters_sqlite::SqliteCapabilityRollupProjectionSink as CapabilityRollupProjectionSink;
pub use application::capability_benchmark::{CapabilityReceiptRollup, CapabilityRollupKey};
