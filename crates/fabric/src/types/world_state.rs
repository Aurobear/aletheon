//! World state observation port for embodied verification.
//! Normalized snapshot that Metacog and RobotHarness can read without coupling to Hardware.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::embodiment::DeviceId;
use crate::MonoTime;

/// A normalized snapshot of the world for a specific device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub device: DeviceId,
    pub schema: String,
    pub sequence: u64,
    pub payload: serde_json::Value,
    pub observed_at: MonoTime,
    pub stale: bool,
}

/// Read-only port for embodied world state observation.
#[async_trait]
pub trait WorldStatePort: Send + Sync {
    /// Get the latest snapshot for a device.
    async fn latest(&self, device: &DeviceId) -> Option<WorldSnapshot>;

    /// Wait for an observation with sequence greater than the given value,
    /// up to the deadline. Returns None on timeout.
    async fn observe_until(
        &self,
        device: &DeviceId,
        after_sequence: u64,
        deadline: crate::MonoDeadline,
    ) -> Option<WorldSnapshot>;
}
