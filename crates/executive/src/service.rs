//! Deprecated compatibility facade for the canonical [`crate::application`] layer.
//!
//! New code must import application facades directly. This module remains only
//! until downstream callers complete the Phase 9 public API migration.

#[deprecated(note = "use executive::application")]
pub use crate::application::*;
