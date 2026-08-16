//! Retired event-sourced session store — one-way compatibility re-export.
//!
//! The store implementation moved to `adapters_sqlite::session::event_sourced_store`
//! (persistence owner). This module re-exports the public surface so existing
//! Aletheon-internal callers and compatibility tests keep compiling while the
//! cutover completes. No new implementation is added here.

pub use adapters_sqlite::session::event_sourced_store::{
    reconcile_committed_session_events, session_append_metrics, EventSourcedSessionStore,
    SessionAppendMetrics, SessionEventReconcileReport,
};
