//! Bounded perception adapter over typed embodiment observations.
//!
//! Only metadata and content-addressed frame references cross this boundary.
//! Image bytes and arbitrary observation payloads are never retained here.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::PerceptionObservation;
use super::RobotPerceptionPort;
use ::contracts::types::embodiment::{DeviceId, EmbodiedObservation};
use ::contracts::Clock;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct RobotPerceptionRuntimeConfig {
    pub max_devices: usize,
    pub max_cached_frames_per_device: usize,
    pub max_age: Duration,
    pub max_total_bytes: u64,
    pub allowed_uri_prefixes: Vec<String>,
}

pub struct EmbodimentPerceptionStore {
    devices: RwLock<HashMap<DeviceId, VecDeque<PerceptionObservation>>>,
    clock: Arc<dyn Clock>,
    max_devices: usize,
    max_cached_frames_per_device: usize,
    max_age_ms: i64,
    max_total_bytes: u64,
    allowed_uri_prefixes: Vec<String>,
}

impl EmbodimentPerceptionStore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: Arc<dyn Clock>,
        max_devices: usize,
        max_cached_frames_per_device: usize,
        max_age: Duration,
        max_total_bytes: u64,
        allowed_uri_prefixes: Vec<String>,
    ) -> Result<Self, String> {
        if max_devices == 0 || max_cached_frames_per_device == 0 || max_total_bytes == 0 {
            return Err("perception store bounds must be nonzero".into());
        }
        let max_age_ms = i64::try_from(max_age.as_millis())
            .map_err(|_| "perception max age does not fit milliseconds".to_string())?;
        if max_age_ms <= 0 || allowed_uri_prefixes.is_empty() {
            return Err("perception max age and URI policy must be non-empty".into());
        }
        Ok(Self {
            devices: RwLock::new(HashMap::new()),
            clock,
            max_devices,
            max_cached_frames_per_device,
            max_age_ms,
            max_total_bytes,
            allowed_uri_prefixes,
        })
    }

    pub fn from_config(
        clock: Arc<dyn Clock>,
        config: RobotPerceptionRuntimeConfig,
    ) -> Result<Self, String> {
        Self::new(
            clock,
            config.max_devices,
            config.max_cached_frames_per_device,
            config.max_age,
            config.max_total_bytes,
            config.allowed_uri_prefixes,
        )
    }

    /// Map and retain one visual observation. Non-visual observations are
    /// ignored without error so the world-state and perception pipelines can
    /// consume the same provider stream.
    pub fn ingest_observation(
        &self,
        device: &DeviceId,
        observation: &EmbodiedObservation,
    ) -> Result<bool, String> {
        let Some(perception) = observation_to_perception(device, observation)? else {
            return Ok(false);
        };
        perception.validate()?;
        if !self
            .allowed_uri_prefixes
            .iter()
            .any(|prefix| perception.frame.uri.starts_with(prefix))
        {
            return Err(format!(
                "frame URI is outside the configured perception authority: {}",
                perception.frame.uri
            ));
        }
        if perception.frame.byte_len > self.max_total_bytes {
            return Err(format!(
                "frame size {} exceeds perception byte budget {}",
                perception.frame.byte_len, self.max_total_bytes
            ));
        }

        let mut devices = self.devices.write().map_err(|e| format!("lock: {e}"))?;
        if devices.len() >= self.max_devices && !devices.contains_key(device) {
            return Err(format!(
                "perception device limit {} reached",
                self.max_devices
            ));
        }
        let frames = devices.entry(device.clone()).or_default();
        if let Some(existing) = frames.iter().rev().find(|existing| {
            existing.schema == perception.schema
                && existing.schema_version == perception.schema_version
                && existing.frame.camera_id == perception.frame.camera_id
        }) {
            if existing.frame.frame_id == perception.frame.frame_id
                && existing.frame == perception.frame
            {
                // Polling a provider faster than its capture cadence repeats
                // the same immutable FrameRef. Treat that exact replay as an
                // idempotent no-op; a conflicting/lower sequence still fails.
                return Ok(false);
            }
            if existing.frame.frame_id >= perception.frame.frame_id {
                return Err(format!(
                    "perception sequence {} is not newer for device={} schema={}/v{} source={}",
                    perception.frame.frame_id,
                    device.0,
                    perception.schema,
                    perception.schema_version,
                    perception.frame.camera_id
                ));
            }
        }
        frames.push_back(perception);
        while frames.len() > self.max_cached_frames_per_device {
            frames.pop_front();
        }
        Ok(true)
    }
}

#[async_trait]
impl RobotPerceptionPort for EmbodimentPerceptionStore {
    async fn latest(
        &self,
        device: &DeviceId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now_ms = self.clock.wall_now().0;
        let devices = self.devices.read().map_err(|e| format!("lock: {e}"))?;
        let Some(frames) = devices.get(device) else {
            return Ok(Vec::new());
        };
        let mut candidates = frames
            .iter()
            .filter(|observation| {
                !observation.frame.is_expired(now_ms, self.max_age_ms)
                    && after_sequence.is_none_or(|sequence| observation.frame.frame_id > sequence)
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .received_ms
                .cmp(&left.received_ms)
                .then(right.frame.frame_id.cmp(&left.frame.frame_id))
                .then(left.schema.cmp(&right.schema))
        });

        let mut selected = Vec::with_capacity(limit.min(candidates.len()));
        let mut selected_bytes = 0_u64;
        for candidate in candidates {
            if selected.len() == limit {
                break;
            }
            let next = selected_bytes.saturating_add(candidate.frame.byte_len);
            if next > self.max_total_bytes {
                continue;
            }
            selected_bytes = next;
            selected.push(candidate);
        }
        Ok(selected)
    }
}

pub fn observation_to_perception(
    device: &DeviceId,
    observation: &EmbodiedObservation,
) -> Result<Option<PerceptionObservation>, String> {
    let Some(frame) = observation.frame.clone() else {
        return Ok(None);
    };
    let labels = match observation.payload.get("labels") {
        None => Vec::new(),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "perception labels must contain strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("perception labels must be an array".into()),
    };
    let summary = observation
        .payload
        .get("summary")
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "perception summary must be a string".to_string())
        })
        .transpose()?
        .unwrap_or_default();
    let perception = PerceptionObservation {
        device: device.clone(),
        schema: observation.schema.clone(),
        schema_version: observation.schema_version,
        frame,
        labels,
        summary,
        confidence: observation.confidence,
        received_ms: observation.received_unix_ms,
    };
    perception.validate()?;
    Ok(Some(perception))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::frame::FrameRef;
    use ::contracts::{MonoTime, WallTime};
    use std::sync::atomic::{AtomicI64, Ordering};

    struct WallClock(AtomicI64);
    impl WallClock {
        fn new(now: i64) -> Self {
            Self(AtomicI64::new(now))
        }
    }
    impl Clock for WallClock {
        fn wall_now(&self) -> WallTime {
            WallTime(self.0.load(Ordering::SeqCst))
        }
        fn mono_now(&self) -> MonoTime {
            MonoTime(0)
        }
    }

    fn visual(
        device: &str,
        schema: &str,
        sequence: u64,
        captured: i64,
        bytes: u64,
    ) -> EmbodiedObservation {
        let digest = format!("{sequence:064x}");
        EmbodiedObservation {
            schema: schema.into(),
            schema_version: 1,
            source: "front-camera".into(),
            sequence,
            source_time: MonoTime(sequence),
            received_at: MonoTime(sequence + 1),
            source_unix_ms: captured,
            received_unix_ms: captured + 1,
            valid_until: None,
            confidence: 0.9,
            reference_frame: None,
            frame: Some(FrameRef {
                uri: format!("artifact://sha256/{digest}"),
                sha256: digest,
                mime_type: "image/jpeg".into(),
                width: 640,
                height: 480,
                byte_len: bytes,
                source_time_ms: captured,
                camera_id: "front-camera".into(),
                frame_id: sequence,
            }),
            payload: serde_json::json!({"labels": [device], "summary": "bounded"}),
            evidence: vec![],
        }
    }

    fn store(clock: Arc<dyn Clock>, budget: u64) -> EmbodimentPerceptionStore {
        EmbodimentPerceptionStore::new(
            clock,
            4,
            8,
            Duration::from_millis(500),
            budget,
            vec!["artifact://sha256/".into()],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn filters_stale_device_sequence_and_schema_without_copying_payload() {
        let clock: Arc<dyn Clock> = Arc::new(WallClock::new(2_000));
        let store = store(clock, 10_000);
        let first = DeviceId("first".into());
        let second = DeviceId("second".into());
        store
            .ingest_observation(&first, &visual("first", "camera.rgb", 1, 1_000, 10))
            .unwrap();
        store
            .ingest_observation(&first, &visual("first", "camera.depth", 2, 1_900, 10))
            .unwrap();
        store
            .ingest_observation(&second, &visual("second", "camera.rgb", 3, 1_900, 10))
            .unwrap();

        let selected = store.latest(&first, Some(1), 4).await.unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].schema, "camera.depth");
        assert_eq!(selected[0].device, first);
        assert!(!serde_json::to_string(&selected).unwrap().contains("base64"));
    }

    #[tokio::test]
    async fn enforces_uri_and_total_byte_budgets() {
        let clock: Arc<dyn Clock> = Arc::new(WallClock::new(2_000));
        let store = store(clock, 15);
        let device = DeviceId("bot".into());
        store
            .ingest_observation(&device, &visual("bot", "camera.rgb", 1, 1_900, 10))
            .unwrap();
        store
            .ingest_observation(&device, &visual("bot", "camera.depth", 2, 1_901, 10))
            .unwrap();
        assert_eq!(store.latest(&device, None, 4).await.unwrap().len(), 1);

        let mut unauthorized = visual("bot", "camera.rgb", 3, 1_902, 10);
        let frame = unauthorized.frame.as_mut().unwrap();
        frame.uri = "workspace://camera/frame.jpg".into();
        assert!(store.ingest_observation(&device, &unauthorized).is_err());
    }

    #[tokio::test]
    async fn exact_provider_frame_replay_is_idempotent_but_conflicts_fail() {
        let clock: Arc<dyn Clock> = Arc::new(WallClock::new(2_000));
        let store = store(clock, 10_000);
        let device = DeviceId("bot".into());
        let frame = visual("bot", "camera.rgb", 7, 1_900, 10);
        assert!(store.ingest_observation(&device, &frame).unwrap());
        assert!(!store.ingest_observation(&device, &frame).unwrap());

        let mut conflict = frame;
        conflict.frame.as_mut().unwrap().byte_len = 11;
        assert!(store.ingest_observation(&device, &conflict).is_err());
        assert_eq!(store.latest(&device, None, 4).await.unwrap().len(), 1);
    }
}
