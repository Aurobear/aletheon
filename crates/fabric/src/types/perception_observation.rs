//! Bounded perception observation — labels and summary only, no image bytes.

use serde::{Deserialize, Serialize};

use crate::types::frame::FrameRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerceptionObservation {
    /// Reference to the visual frame (no image bytes).
    pub frame: FrameRef,
    /// Compact semantic labels (max 16).
    pub labels: Vec<String>,
    /// One-line natural-language summary (max 256 chars).
    pub summary: String,
    /// Confidence score [0.0, 1.0].
    pub confidence: f32,
    /// Wall-clock receipt timestamp.
    pub received_ms: i64,
}

impl PerceptionObservation {
    pub fn validate(&self) -> Result<(), String> {
        self.frame.validate()?;
        if self.labels.len() > 16 {
            return Err(format!("too many labels: {} > 16", self.labels.len()));
        }
        if self.summary.len() > 256 {
            return Err("summary exceeds 256 characters".into());
        }
        if self.confidence < 0.0 || self.confidence > 1.0 {
            return Err(format!("confidence {} out of [0,1]", self.confidence));
        }
        Ok(())
    }
}
