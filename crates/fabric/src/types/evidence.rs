//! Evidence — a piece of evidence bearing on a hypothesis or decision.
//!
//! Defined here (rather than in fabric's primitives/) because the AgoraOps trait
//! contract in `include/agora.rs` references it.  Fabric's `primitives/cognitive.rs`
//! re-exports from here for backward compatibility.

use serde::{Deserialize, Serialize};

/// A piece of evidence bearing on a hypothesis or decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    /// Where the evidence came from (tool name, memory id, observation id).
    pub source: String,
    pub content: String,
    /// Relative weight in [0.0, 1.0].
    pub weight: f64,
}

impl Evidence {
    /// Build `Evidence` from a tool result — the canonical producer today.
    ///
    /// A successful result carries full weight (1.0); an error carries none
    /// (0.0) so downstream reasoning can discount it.
    pub fn from_tool_result(
        call_id: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            id: call_id.into(),
            source: source.into(),
            content: content.into(),
            weight: if is_error { 0.0 } else { 1.0 },
        }
    }
}
