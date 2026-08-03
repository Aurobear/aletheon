//! Production world-state adapter over the embodiment observation boundary.
//! Implements fabric::WorldStatePort using the existing EmbodimentExecutionPort.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, EmbodiedObservation, EmbodimentExecutionPort};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort, ANY_SCHEMA};
use fabric::{Clock, MonoDeadline, MonoTime};
use tokio::sync::Notify;

/// Per-device cached state entry.
///
/// A device can expose several observation schemas (e.g. `base_pose`,
/// `base_twist`, `ground_truth_pose`) whose sequences come from independent
/// counters. Each schema keeps its own monotonic slot — the `ingest` gate is
/// per schema, so a slower source can never evict a faster one.
struct DeviceState {
    /// Latest snapshot per observation schema.
    latest: HashMap<String, WorldSnapshot>,
    /// Device-level wakeup: any schema ingest wakes waiters.
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
            latest: HashMap::new(),
            notify: Arc::new(Notify::new()),
        });

        // Reject duplicate or lower sequence *within the same schema*. Schemas
        // carry independent counters, so a lower-sequenced schema must not be
        // blocked by (or evict) a higher-sequenced one.
        if let Some(existing) = entry.latest.get(&snapshot.schema) {
            if snapshot.sequence <= existing.sequence {
                return Err(format!(
                    "rejected sequence {} <= existing {} for device {:?} schema {}",
                    snapshot.sequence, existing.sequence, snapshot.device, snapshot.schema
                ));
            }
        }

        entry.latest.insert(snapshot.schema.clone(), snapshot);
        entry.notify.notify_waiters();
        Ok(())
    }
}

#[async_trait]
impl WorldStatePort for EmbodimentWorldState {
    async fn latest(&self, device: &DeviceId, schema: &str) -> Option<WorldSnapshot> {
        let devices = self.devices.read().ok()?;
        let entry = devices.get(device)?;
        if schema == ANY_SCHEMA {
            entry
                .latest
                .values()
                .max_by(|a, b| a.observed_at.cmp(&b.observed_at).then(a.sequence.cmp(&b.sequence)))
                .cloned()
        } else {
            entry.latest.get(schema).cloned()
        }
    }

    async fn observe_until(
        &self,
        device: &DeviceId,
        schema: &str,
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
                    let candidates: Vec<WorldSnapshot> = if schema == ANY_SCHEMA {
                        entry.latest.values().cloned().collect()
                    } else {
                        entry.latest.get(schema).cloned().into_iter().collect()
                    };
                    if let Some(snap) = candidates.into_iter().find(|s| s.sequence > after_sequence) {
                        return Some(snap);
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

    fn schema_snapshot(device: &str, schema: &str, seq: u64, payload: serde_json::Value) -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId(device.into()),
            schema: schema.into(),
            sequence: seq,
            payload,
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
        let snap = ws.latest(&dev, ANY_SCHEMA).await.unwrap();
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

    /// The bridge returns multiple schemas (base_pose/base_twist/ground_truth_pose)
    /// with independent sequence counters. Each must keep its own monotonic slot:
    /// a slower schema must neither be evicted by nor rejected because of a
    /// faster one's higher sequence.
    #[tokio::test]
    async fn distinct_schemas_keep_independent_monotonic_slots() {
        let ws = world_state(10);
        let dev = DeviceId("bot".into());

        // Three schemas arrive in one pump cycle; the odom counter is far ahead
        // of the ground-truth counter (independent counters).
        ws.ingest(dev.clone(), schema_snapshot("bot", "base_pose", 100, serde_json::json!({"position": 1.0})))
            .unwrap();
        ws.ingest(dev.clone(), schema_snapshot("bot", "base_twist", 100, serde_json::json!({"v": 0.0})))
            .unwrap();
        ws.ingest(dev.clone(), schema_snapshot("bot", "ground_truth_pose", 5, serde_json::json!({"position": 1.0})))
            .unwrap();

        // All three survive, despite the gt sequence being below the odom one.
        assert_eq!(ws.latest(&dev, "base_pose").await.unwrap().sequence, 100);
        assert_eq!(ws.latest(&dev, "base_twist").await.unwrap().sequence, 100);
        assert_eq!(ws.latest(&dev, "ground_truth_pose").await.unwrap().sequence, 5);

        // Independent gates: a lower-sequence re-ingest of gt is still rejected
        // within its own schema...
        assert!(ws
            .ingest(dev.clone(), schema_snapshot("bot", "ground_truth_pose", 4, serde_json::json!({})))
            .is_err());
        // ...but a new base_pose with seq 101 is accepted even though it is far
        // above gt's counter.
        ws.ingest(dev.clone(), schema_snapshot("bot", "base_pose", 101, serde_json::json!({"position": 2.0})))
            .unwrap();
        assert_eq!(ws.latest(&dev, "base_pose").await.unwrap().sequence, 101);

        // ANY_SCHEMA returns the freshest across schemas.
        assert_eq!(ws.latest(&dev, ANY_SCHEMA).await.unwrap().sequence, 101);
    }

    #[tokio::test]
    async fn missing_device_returns_none() {
        let ws = world_state(10);
        assert!(ws
            .latest(&DeviceId("nonexistent".into()), ANY_SCHEMA)
            .await
            .is_none());
        assert!(ws
            .latest(&DeviceId("nonexistent".into()), "base_pose")
            .await
            .is_none());
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
        assert_eq!(ws.latest(&device, ANY_SCHEMA).await.unwrap().sequence, 1);

        pump.poll_once(&device).await;
        let latest = ws.latest(&device, ANY_SCHEMA).await.unwrap();
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
        assert!(ws.latest(&device, ANY_SCHEMA).await.is_none());
    }
}
