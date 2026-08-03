//! Production world-state adapter over the embodiment observation boundary.
//! Implements fabric::WorldStatePort using the existing EmbodimentExecutionPort.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, EmbodiedObservation, EmbodimentExecutionPort};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{Clock, MonoDeadline, MonoTime};
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
    clock: Arc<dyn Clock>,
}

impl EmbodimentWorldState {
    /// Construct a world-state adapter from the runtime's shared clock.
    pub fn new(max_devices: usize, clock: Arc<dyn Clock>) -> Self {
        Self {
            devices: RwLock::new(HashMap::new()),
            max_devices,
            clock,
        }
    }

    /// Ingest a new observation into the world state. Called from the
    /// embodiment observation pipeline.
    pub fn ingest(&self, device: DeviceId, snapshot: WorldSnapshot) -> Result<(), String> {
        let mut devices = self.devices.write().map_err(|e| format!("lock: {e}"))?;
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

            // Check the operation deadline against the injected monotonic clock.
            let now = self.clock.mono_now();
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

/// Convert an embodied observation into a normalized world snapshot.
///
/// A snapshot is marked `stale` when it is past its validity window or carries
/// no meaningful confidence. Stale samples remain stored (for audit) but must
/// never count toward a verification stability window — the verifier reads the
/// `stale` flag and the `observed_at` freshness before matching.
pub fn observation_to_snapshot(
    device: &DeviceId,
    obs: &EmbodiedObservation,
    now: MonoTime,
) -> WorldSnapshot {
    let stale = obs
        .valid_until
        .map(|deadline| deadline.is_expired_at(now))
        .unwrap_or(false)
        || obs.confidence <= 0.0;
    WorldSnapshot {
        device: device.clone(),
        schema: obs.schema.clone(),
        sequence: obs.sequence,
        payload: obs.payload.clone(),
        observed_at: obs.source_time,
        stale,
    }
}

/// Background pump that polls the embodiment executor and feeds the world state.
///
/// Owns a shared `EmbodimentWorldState` and periodically issues `observe()`.
/// `ingest` enforces monotonic sequence and a device bound, so the pump never
/// overwrites a newer snapshot with a lower-sequenced or duplicate one.
pub struct WorldStatePump {
    world: Arc<EmbodimentWorldState>,
    executor: Arc<dyn EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    poll_interval: std::time::Duration,
}

impl WorldStatePump {
    pub fn new(
        world: Arc<EmbodimentWorldState>,
        executor: Arc<dyn EmbodimentExecutionPort>,
        clock: Arc<dyn Clock>,
        poll_interval: std::time::Duration,
    ) -> Self {
        Self {
            world,
            executor,
            clock,
            poll_interval,
        }
    }

    /// Ingest the device's current observations in one pass.
    pub async fn poll_once(&self, device: &DeviceId) {
        match self.executor.observe(device).await {
            Ok(observations) => {
                let now = self.clock.mono_now();
                for observation in observations {
                    let snapshot = observation_to_snapshot(device, &observation, now);
                    if let Err(reason) = self.world.ingest(device.clone(), snapshot) {
                        tracing::debug!(
                            device = %device.0,
                            %reason,
                            "world-state pump skipped observation"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    device = %device.0,
                    error = %error,
                    "world-state pump observe failed"
                );
            }
        }
    }

    /// Spawn a background task polling each device at the configured interval.
    pub fn spawn(self: Arc<Self>, devices: Vec<DeviceId>) {
        let poll_interval = self.poll_interval;
        tokio::spawn(async move {
            loop {
                for device in &devices {
                    self.poll_once(device).await;
                }
                tokio::time::sleep(poll_interval).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use fabric::MonoTime;
    use kernel::chronos::TestClock;

    fn world_state(max_devices: usize) -> EmbodimentWorldState {
        EmbodimentWorldState::new(max_devices, Arc::new(TestClock::default()))
    }

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
        let ws = world_state(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 1, 1.0)).unwrap();
        ws.ingest(dev.clone(), snapshot("bot", 2, 2.0)).unwrap();
        let snap = ws.latest(&dev).await.unwrap();
        assert_eq!(snap.sequence, 2);
        assert_eq!(snap.payload["x"].as_f64().unwrap(), 2.0);
    }

    #[test]
    fn lower_sequence_rejected() {
        let ws = world_state(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 10, 1.0)).unwrap();
        assert!(ws.ingest(dev.clone(), snapshot("bot", 5, 0.5)).is_err());
        assert!(ws.ingest(dev.clone(), snapshot("bot", 10, 9.0)).is_err());
    }

    #[test]
    fn duplicate_sequence_rejected() {
        let ws = world_state(10);
        let dev = DeviceId("bot".into());
        ws.ingest(dev.clone(), snapshot("bot", 1, 1.0)).unwrap();
        assert!(ws.ingest(dev.clone(), snapshot("bot", 1, 2.0)).is_err());
    }

    #[tokio::test]
    async fn missing_device_returns_none() {
        let ws = world_state(10);
        assert!(ws.latest(&DeviceId("nonexistent".into())).await.is_none());
    }

    #[test]
    fn bounded_device_count_enforced() {
        let ws = world_state(2);
        ws.ingest(DeviceId("a".into()), snapshot("a", 1, 0.0))
            .unwrap();
        ws.ingest(DeviceId("b".into()), snapshot("b", 1, 0.0))
            .unwrap();
        assert!(ws
            .ingest(DeviceId("c".into()), snapshot("c", 1, 0.0))
            .is_err());
    }

    fn observation(seq: u64, valid_until_ms: u64, confidence: f32) -> EmbodiedObservation {
        EmbodiedObservation {
            schema: "robot.state/v1".into(),
            schema_version: 1,
            source: "sim".into(),
            sequence: seq,
            source_time: MonoTime(seq),
            received_at: MonoTime(seq),
            valid_until: Some(MonoDeadline::after(MonoTime(0), valid_until_ms)),
            confidence,
            frame_ref: None,
            payload: serde_json::json!({"mode": "stance"}),
            evidence: vec![],
        }
    }

    #[test]
    fn observation_to_snapshot_marks_stale_on_expiry_or_low_confidence() {
        let device = DeviceId("bot".into());
        let now = MonoTime(2_000);
        let fresh = observation_to_snapshot(&device, &observation(1, 5_000, 1.0), now);
        assert!(!fresh.stale);
        assert_eq!(fresh.sequence, 1);
        assert_eq!(fresh.device, device);
        assert_eq!(fresh.schema, "robot.state/v1");
        assert_eq!(fresh.payload["mode"], serde_json::json!("stance"));
        // Deadline after(0, 1000) is 1000; now 2000 > 1000 => expired.
        let expired = observation_to_snapshot(&device, &observation(2, 1_000, 1.0), now);
        assert!(expired.stale);
        let low_confidence = observation_to_snapshot(&device, &observation(3, 5_000, 0.0), now);
        assert!(low_confidence.stale);
    }

    struct FakeExecutor {
        results: Mutex<VecDeque<Result<Vec<EmbodiedObservation>, fabric::types::embodiment::SkillDispatchError>>>,
    }

    #[async_trait::async_trait]
    impl EmbodimentExecutionPort for FakeExecutor {
        async fn observe(
            &self,
            _device: &DeviceId,
        ) -> Result<Vec<EmbodiedObservation>, fabric::types::embodiment::SkillDispatchError>
        {
            let mut queue = self.results.lock().unwrap();
            Ok(queue.pop_front().unwrap_or(Ok(vec![]))?)
        }
        async fn get_state(
            &self,
            _device: &DeviceId,
        ) -> Result<Option<EmbodiedObservation>, fabric::types::embodiment::SkillDispatchError>
        {
            Ok(None)
        }
        async fn list_skills(
            &self,
            _device: &DeviceId,
        ) -> Result<Vec<fabric::types::embodiment::SkillDescriptor>, fabric::types::embodiment::SkillDispatchError>
        {
            Ok(vec![])
        }
        async fn execute_skill(
            &self,
            _request: fabric::types::embodiment::SkillRequest,
        ) -> Result<fabric::types::embodiment::SkillResult, fabric::types::embodiment::SkillDispatchError>
        {
            Err(fabric::types::embodiment::SkillDispatchError::Rejected(
                "not used".into(),
            ))
        }
        async fn cancel(
            &self,
            _operation_id: &fabric::OperationId,
        ) -> Result<(), fabric::types::embodiment::SkillDispatchError> {
            Ok(())
        }
        async fn safe_stop(
            &self,
            _device: &DeviceId,
        ) -> Result<(), fabric::types::embodiment::SkillDispatchError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn pump_polls_and_ingests_monotonic_observations() {
        let ws = Arc::new(EmbodimentWorldState::new(10, Arc::new(TestClock::default())));
        let executor = Arc::new(FakeExecutor {
            results: Mutex::new(VecDeque::from(vec![
                Ok(vec![observation(1, 5_000, 1.0)]),
                // dup seq 1 must be rejected by ingest; seq 2 kept.
                Ok(vec![observation(2, 5_000, 1.0), observation(1, 5_000, 1.0)]),
            ])),
        });
        let pump = WorldStatePump::new(
            ws.clone(),
            executor,
            Arc::new(TestClock::default()),
            std::time::Duration::from_millis(10),
        );
        let device = DeviceId("bot".into());

        pump.poll_once(&device).await;
        assert_eq!(ws.latest(&device).await.unwrap().sequence, 1);

        pump.poll_once(&device).await;
        let latest = ws.latest(&device).await.unwrap();
        assert_eq!(latest.sequence, 2, "dup seq 1 rejected, seq 2 kept");
        assert!(!latest.stale);
    }

    #[tokio::test]
    async fn pump_survives_observe_error_without_poisoning() {
        let ws = Arc::new(EmbodimentWorldState::new(10, Arc::new(TestClock::default())));
        let executor = Arc::new(FakeExecutor {
            results: Mutex::new(VecDeque::from(vec![Err(
                fabric::types::embodiment::SkillDispatchError::Rejected("down".into()),
            )])),
        });
        let pump = WorldStatePump::new(
            ws.clone(),
            executor,
            Arc::new(TestClock::default()),
            std::time::Duration::from_millis(10),
        );
        let device = DeviceId("bot".into());
        pump.poll_once(&device).await;
        assert!(ws.latest(&device).await.is_none());
    }
}
