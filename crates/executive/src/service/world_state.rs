//! Production world-state adapter over the embodiment observation boundary.
//! Implements fabric::WorldStatePort using the existing EmbodimentExecutionPort.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fabric::types::embodiment::DeviceId;
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{MonoDeadline, MonoTime};
use tokio::sync::Notify;

/// Per-device cached state entry.
struct DeviceState {
    latest: Option<WorldSnapshot>,
    notify: Arc<Notify>,
}

/// Production world-state adapter using EmbodimentExecutionPort.
pub struct EmbodimentWorldState {
    devices: RwLock<HashMap<DeviceId, DeviceState>>,
    /// Maximum number of devices tracked (bounded).
    max_devices: usize,
}

impl EmbodimentWorldState {
    pub fn new(max_devices: usize) -> Self {
        Self { devices: RwLock::new(HashMap::new()), max_devices }
    }

    /// Ingest a new observation into the world state. Called from the
    /// embodiment observation pipeline.
    pub fn ingest(&self, device: DeviceId, snapshot: WorldSnapshot) -> Result<(), String> {
        let mut devices = self.devices.write().map_err(|e| format!("lock: {}", e))?;
        if devices.len() >= self.max_devices && !devices.contains_key(&device) {
            return Err(format!("device limit {} reached", self.max_devices));
        }
        let entry = devices.entry(device).or_insert_with(|| DeviceState {
            latest: None,
            notify: Arc::new(Notify::new()),
        });

        // Reject duplicate or lower sequence
        if let Some(ref existing) = entry.latest {
            if snapshot.sequence <= existing.sequence {
                return Err(format!(
                    "rejected sequence {} <= existing {} for device {:?}",
                    snapshot.sequence, existing.sequence, snapshot.device
                ));
            }
        }

        entry.latest = Some(snapshot);
        entry.notify.notify_waiters();
        Ok(())
    }
}

#[async_trait]
impl WorldStatePort for EmbodimentWorldState {
    async fn latest(&self, device: &DeviceId) -> Option<WorldSnapshot> {
        let devices = self.devices.read().ok()?;
        devices.get(device)?.latest.clone()
    }

    async fn observe_until(
        &self,
        device: &DeviceId,
        after_sequence: u64,
        deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        let notify = {
            let devices = self.devices.read().ok()?;
            devices.get(device)?.notify.clone()
        };

        loop {
            // Check current state
            {
                let devices = self.devices.read().ok()?;
                if let Some(entry) = devices.get(device) {
                    if let Some(ref snap) = entry.latest {
                        if snap.sequence > after_sequence {
                            return Some(snap.clone());
                        }
                    }
                }
            }

            // Check deadline
            // TODO: inject a production monotonic clock instead of hardcoded MonoTime(0)
            let now = MonoTime(0);
            if deadline.is_expired_at(now) {
                return None;
            }

            // Wait for notification or timeout
            tokio::select! {
                _ = notify.notified() => continue,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(device: &str, seq: u64, x: f64) -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId(device.into()),
            schema: "test".into(),
            sequence: seq,
            payload: serde_json::json!({"x": x}),
            observed_at: MonoTime(seq),
            stale: false,
        }
    }

    #[tokio::test]
    async fn latest_returns_most_recent_ingested() {
        let ws = EmbodimentWorldState::new(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 1, 1.0)).unwrap();
        ws.ingest(dev.clone(), snapshot("bot", 2, 2.0)).unwrap();
        let snap = ws.latest(&dev).await.unwrap();
        assert_eq!(snap.sequence, 2);
        assert_eq!(snap.payload["x"].as_f64().unwrap(), 2.0);
    }

    #[test]
    fn lower_sequence_rejected() {
        let ws = EmbodimentWorldState::new(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 10, 1.0)).unwrap();
        assert!(ws.ingest(dev.clone(), snapshot("bot", 5, 0.5)).is_err());
        assert!(ws.ingest(dev.clone(), snapshot("bot", 10, 9.0)).is_err());
    }

    #[test]
    fn duplicate_sequence_rejected() {
        let ws = EmbodimentWorldState::new(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 1, 1.0)).unwrap();
        assert!(ws.ingest(dev.clone(), snapshot("bot", 1, 2.0)).is_err());
    }

    #[tokio::test]
    async fn missing_device_returns_none() {
        let ws = EmbodimentWorldState::new(10);
        assert!(ws.latest(&DeviceId("nonexistent".into())).await.is_none());
    }

    #[test]
    fn bounded_device_count_enforced() {
        let ws = EmbodimentWorldState::new(2);
        ws.ingest(DeviceId("a".into()), snapshot("a", 1, 0.0)).unwrap();
        ws.ingest(DeviceId("b".into()), snapshot("b", 1, 0.0)).unwrap();
        assert!(ws.ingest(DeviceId("c".into()), snapshot("c", 1, 0.0)).is_err());
    }
}
