//! Retired SessionService entry point — one-way compatibility re-export.
//!
//! The session command/query service moved to `runtime::session_service`
//! (Session semantic authority owner). This module re-exports the public
//! surface so existing Aletheon-internal callers and compatibility tests keep
//! compiling while the cutover completes. No new implementation is added here.

pub use runtime::session_service::{
    InterruptOutcome, ResumeResult, SessionService, session_visible_to,
};
