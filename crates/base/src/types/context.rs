//! Context — like Linux kernel's task_struct.
//!
//! A Context flows through the entire request lifecycle. It carries
//! request identity, session state, permissions, and trace information
//! from Intent Gateway through SelfField, BrainCore, and BodyRuntime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    /// Distributed trace state (for observability).
    pub trace: TraceState,

    /// Capability set — what this request is allowed to do.
    pub permissions: CapabilitySet,

    /// Current working directory.
    pub working_dir: PathBuf,

    /// Extensible metadata (subsystem-specific data).
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Distributed trace state for observability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceState {
    /// Trace ID (shared across a request tree).
    pub trace_id: String,
    /// Span ID (this specific operation).
    pub span_id: String,
    /// Parent span ID (who called us).
    pub parent_span_id: Option<String>,
}

impl Context {
    /// Create a new context for a fresh request.
    pub fn new(session_id: impl Into<String>, working_dir: PathBuf) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            session_id: session_id.into(),
            trace: TraceState::default(),
            permissions: CapabilitySet::new(),
            working_dir,
            metadata: HashMap::new(),
        }
    }

    /// Create a child context for a sub-operation.
    pub fn child(&self) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            session_id: self.session_id.clone(),
            trace: TraceState {
                trace_id: self.trace.trace_id.clone(),
                span_id: Uuid::new_v4().to_string(),
                parent_span_id: Some(self.trace.span_id.clone()),
            },
            permissions: self.permissions.clone(),
            working_dir: self.working_dir.clone(),
            metadata: self.metadata.clone(),
        }
    }

    /// Set a metadata key.
    pub fn set_meta(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.metadata.insert(key.into(), value);
    }

    /// Get a metadata key.
    pub fn get_meta(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.get(key)
    }
}
