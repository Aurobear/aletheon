//! Bounded perception observation — labels and summary only, no image bytes.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::DeviceId;
use crate::types::frame::FrameRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerceptionObservation {
    /// Device namespace this frame belongs to.
    pub device: DeviceId,
    /// Typed embodiment observation schema that carried the frame.
    pub schema: String,
    pub schema_version: u16,
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
        if self.device.0.trim().is_empty() {
            return Err("device must not be empty".into());
        }
        if self.schema.trim().is_empty() || self.schema_version == 0 {
            return Err("perception schema and nonzero version are required".into());
        }
        self.frame.validate()?;
        if self.labels.len() > 16 {
            return Err(format!("too many labels: {} > 16", self.labels.len()));
        }
        if self.summary.chars().count() > 256 {
            return Err("summary exceeds 256 characters".into());
        }
        if !self.confidence.is_finite() || self.confidence < 0.0 || self.confidence > 1.0 {
            return Err(format!("confidence {} out of [0,1]", self.confidence));
        }
        if self.received_ms < self.frame.source_time_ms {
            return Err("received timestamp precedes captured timestamp".into());
        }
        Ok(())
    }
}
