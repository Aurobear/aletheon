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

/// Sentinel schema selecting the freshest snapshot across a device's schemas.
/// A device may expose several observation schemas (e.g. `base_pose`,
/// `base_twist`, `ground_truth_pose`) with independent sequence counters, so
/// ports are consulted per schema; `ANY_SCHEMA` asks for the freshest one.
pub const ANY_SCHEMA: &str = "*";

/// Read-only port for embodied world state observation.
#[async_trait]
pub trait WorldStatePort: Send + Sync {
    /// Get the latest snapshot for a device and observation schema.
    /// Pass [`ANY_SCHEMA`] to get the freshest snapshot across schemas.
    async fn latest(&self, device: &DeviceId, schema: &str) -> Option<WorldSnapshot>;

    /// Wait for a snapshot of `schema` with sequence greater than the given
    /// value, up to the deadline. Pass [`ANY_SCHEMA`] to wait on any schema.
    /// Returns None on timeout.
    async fn observe_until(
        &self,
        device: &DeviceId,
        schema: &str,
        after_sequence: u64,
        deadline: crate::MonoDeadline,
    ) -> Option<WorldSnapshot>;
}
