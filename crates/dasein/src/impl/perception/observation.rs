//! Bounded visual observation produced by Dasein perception.

use ::contracts::types::embodiment::DeviceId;
use ::contracts::types::frame::FrameRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerceptionObservation {
    pub device: DeviceId,
    pub schema: String,
    pub schema_version: u16,
    pub frame: FrameRef,
    pub labels: Vec<String>,
    pub summary: String,
    pub confidence: f32,
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
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(format!("confidence {} out of [0,1]", self.confidence));
        }
        if self.received_ms < self.frame.source_time_ms {
            return Err("received timestamp precedes captured timestamp".into());
        }
        Ok(())
    }
}
