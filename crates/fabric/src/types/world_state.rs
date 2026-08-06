//! World state observation port for embodied verification.
//! Normalized snapshot that Metacog and RobotHarness can read without coupling to Hardware.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::embodiment::DeviceId;
use crate::{MonoDeadline, MonoTime};

/// A normalized snapshot of the world for a specific device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub device: DeviceId,
    pub schema: String,
    /// Version of the observation schema advertised by the embodiment provider.
    /// Defaults to v1 when decoding legacy persisted snapshots.
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub sequence: u64,
    pub payload: serde_json::Value,
    pub observed_at: MonoTime,
    /// Provider/host monotonic freshness deadline. World-state adapters refresh
    /// `stale` at read time so a once-fresh cached sample cannot remain fresh
    /// forever merely because no newer observation arrived.
    #[serde(default)]
    pub valid_until: Option<MonoDeadline>,
    pub stale: bool,
}

const fn default_schema_version() -> u16 {
    1
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

    /// Get one read candidate per observation schema for planning.
    ///
    /// Implementations with a schema-indexed cache should override this. The
    /// default preserves compatibility for simple/test ports while avoiding a
    /// device-global sequence contract between independent sensor streams.
    async fn latest_all(&self, device: &DeviceId) -> Vec<WorldSnapshot> {
        self.latest(device, ANY_SCHEMA).await.into_iter().collect()
    }

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
