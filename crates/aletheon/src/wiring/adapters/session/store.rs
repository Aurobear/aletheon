//! Retired session store — one-way compatibility re-export.
//!
//! The store implementation moved to `adapters_sqlite::session::store`
//! (persistence owner). This module re-exports the public surface so existing
//! Aletheon-internal callers and compatibility tests keep compiling while the
//! cutover completes. No new implementation is added here.

pub use adapters_sqlite::session::store::{SessionRecord, SessionStore};
