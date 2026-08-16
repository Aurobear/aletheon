//! Context — like Linux kernel's task_struct.
//!
//! A Context flows through the entire request lifecycle. It carries
//! request identity, session state, permissions, and trace information
//! from Intent Gateway through SelfField, CognitCore, and BodyRuntime.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::types::capability::CapabilitySet;

/// Request-scoped context — flows through the entire lifecycle.
///
/// Like `task_struct` in Linux, this carries all state associated
/// with a single request/user intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Unique request identifier.
    pub request_id: Uuid,

    /// Session identifier (persists across requests).
    pub session_id: String,

    /// Capability set — what this request is allowed to do.
    pub permissions: CapabilitySet,

    /// Current working directory.
    pub working_dir: PathBuf,
}

impl Context {
    /// Create a new context for a fresh request.
    pub fn new(session_id: impl Into<String>, working_dir: PathBuf) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            session_id: session_id.into(),
            permissions: CapabilitySet::new(),
            working_dir,
        }
    }
}
