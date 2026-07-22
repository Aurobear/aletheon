//! Visual frame aggregator — deduplication, rate limiting, stale eviction.
//! No image bytes enter the turn context.

use std::collections::{HashMap, VecDeque};

use fabric::types::frame::FrameRef;
use fabric::types::perception_observation::PerceptionObservation;

/// Configuration for visual frame aggregation.
#[derive(Debug, Clone)]
pub struct VisualAggregatorConfig {
    /// Maximum Hz for frame ingestion per camera.
    pub max_hz: usize,
    /// Maximum age before a frame is considered stale (ms).
    pub max_age_ms: i64,
    /// Maximum number of frames to retain per camera.
    pub max_frames_per_camera: usize,
    /// Maximum total perception events retained.
    pub max_total_events: usize,
}

impl Default for VisualAggregatorConfig {
    fn default() -> Self {
        Self {
            max_hz: 5,
            max_age_ms: 5000,
            max_frames_per_camera: 8,
            max_total_events: 64,
        }
    }
}

/// Aggregates visual frames with dedup, rate limiting, and staleness checks.
pub struct VisualAggregator {
    config: VisualAggregatorConfig,
    /// Per-camera dedup: camera_id → hash → timestamp
    seen_hashes: HashMap<String, HashMap<String, i64>>,
    /// Per-camera rate limiting timestamps.
    camera_timestamps: HashMap<String, VecDeque<i64>>,
    /// Latest frame per camera.
    camera_latest: HashMap<String, PerceptionObservation>,
    /// Total events retained (bounded).
    total_events: usize,
}

impl VisualAggregator {
    pub fn new(config: VisualAggregatorConfig) -> Self {
        Self {
            config,
            seen_hashes: HashMap::new(),
            camera_timestamps: HashMap::new(),
            camera_latest: HashMap::new(),
            total_events: 0,
        }
    }

    /// Ingest a frame. Returns Some(PerceptionObservation) if the frame
    /// passes all filters (not duplicate, within rate limit, not stale).
    /// Returns None if the frame is filtered out.
    pub fn ingest(
        &mut self,
        frame: FrameRef,
        labels: Vec<String>,
        summary: String,
        confidence: f32,
        now_ms: i64,
    ) -> Option<PerceptionObservation> {
        // 1. Staleness check
        if now_ms - frame.source_time_ms > self.config.max_age_ms {
            return None;
        }

        // 2. Dedup by camera + sha256 hash
        let camera_hashes = self.seen_hashes.entry(frame.camera_id.clone()).or_default();
        if camera_hashes.contains_key(&frame.sha256) {
            return None;
        }
        camera_hashes.insert(frame.sha256.clone(), now_ms);

        // 3. Rate limiting per camera (max Hz)
        let timestamps = self
            .camera_timestamps
            .entry(frame.camera_id.clone())
            .or_default();
        while !timestamps.is_empty() && timestamps.front().is_none_or(|t| now_ms - t > 1000) {
            timestamps.pop_front();
        }
        if timestamps.len() >= self.config.max_hz {
            return None;
        }
        timestamps.push_back(now_ms);

        // 4. Bounded total events
        if self.total_events >= self.config.max_total_events {
            // Evict oldest per-camera entry to make room
            let mut oldest_cam = None;
            let mut oldest_time = i64::MAX;
            for (cam, obs) in &self.camera_latest {
                if obs.received_ms < oldest_time {
                    oldest_time = obs.received_ms;
                    oldest_cam = Some(cam.clone());
                }
            }
            if let Some(cam) = oldest_cam {
                self.camera_latest.remove(&cam);
                self.total_events = self.total_events.saturating_sub(1);
            } else {
                return None;
            }
        }

        // 5. Clamp confidence
        let clamped_confidence = confidence.max(0.0).min(1.0);

        let obs = PerceptionObservation {
            frame,
            labels,
            summary,
            confidence: clamped_confidence,
            received_ms: now_ms,
        };

        // 6. Update latest per camera
        self.camera_latest
            .insert(obs.frame.camera_id.clone(), obs.clone());
        self.total_events += 1;

        Some(obs)
    }

    /// Get the latest observation for a specific camera.
    pub fn latest(&self, camera_id: &str) -> Option<&PerceptionObservation> {
        self.camera_latest.get(camera_id)
    }

    /// Get all retained observations.
    pub fn all_observations(&self) -> Vec<&PerceptionObservation> {
        self.camera_latest.values().collect()
    }

    /// Evict frames older than max_age_ms.
    pub fn evict_stale(&mut self, now_ms: i64) -> usize {
        let stale_cameras: Vec<String> = self
            .camera_latest
            .iter()
            .filter(|(_, obs)| now_ms - obs.received_ms > self.config.max_age_ms)
            .map(|(cam, _)| cam.clone())
            .collect();
        let count = stale_cameras.len();
        for cam in &stale_cameras {
            self.camera_latest.remove(cam);
            self.total_events = self.total_events.saturating_sub(1);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_frame(camera: &str, sha256: &str, source_ms: i64) -> FrameRef {
        FrameRef {
            uri: format!("artifact://sha256:{sha256}"),
            sha256: sha256.into(),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            source_time_ms: source_ms,
            camera_id: camera.into(),
            frame_id: 0,
        }
    }

    #[test]
    fn dedup_by_camera_and_hash() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig::default());
        let f = test_frame("cam0", "aaa", 1000);
        assert!(agg
            .ingest(
                f.clone(),
                vec!["person".into()],
                "person detected".into(),
                0.9,
                1000
            )
            .is_some());
        // Same camera + hash → dedup
        assert!(agg
            .ingest(
                f,
                vec!["person".into()],
                "person detected".into(),
                0.9,
                1001
            )
            .is_none());
    }

    #[test]
    fn different_camera_same_hash_not_deduped() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig::default());
        let f1 = test_frame("cam0", "aaa", 1000);
        assert!(agg.ingest(f1, vec![], "".into(), 0.5, 1000).is_some());
        let f2 = test_frame("cam1", "aaa", 1000);
        assert!(agg.ingest(f2, vec![], "".into(), 0.5, 1001).is_some());
    }

    #[test]
    fn rate_limiting_at_max_hz() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig {
            max_hz: 2,
            ..Default::default()
        });
        assert!(agg
            .ingest(test_frame("c0", "a", 1000), vec![], "".into(), 0.5, 1000)
            .is_some());
        assert!(agg
            .ingest(test_frame("c0", "b", 1000), vec![], "".into(), 0.5, 1000)
            .is_some());
        // Third frame at same time → rate limited
        assert!(agg
            .ingest(test_frame("c0", "c", 1000), vec![], "".into(), 0.5, 1000)
            .is_none());
    }

    #[test]
    fn stale_frame_is_rejected() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig {
            max_age_ms: 500,
            ..Default::default()
        });
        let f = test_frame("c0", "aaa", 100);
        // now_ms = 1000, 900ms old > 500ms max_age → stale
        assert!(agg.ingest(f, vec![], "".into(), 0.5, 1000).is_none());
    }

    #[test]
    fn confidence_is_clamped() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig::default());
        let obs = agg
            .ingest(test_frame("c0", "aaa", 1000), vec![], "".into(), 1.5, 1000)
            .unwrap();
        assert_eq!(obs.confidence, 1.0);
        let obs2 = agg
            .ingest(test_frame("c0", "bbb", 1001), vec![], "".into(), -0.5, 1001)
            .unwrap();
        assert_eq!(obs2.confidence, 0.0);
    }

    #[test]
    fn eviction_removes_stale() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig {
            max_age_ms: 2000,
            ..Default::default()
        });
        agg.ingest(test_frame("c0", "aaa", 1000), vec![], "".into(), 0.5, 1000);
        assert_eq!(agg.evict_stale(4000), 1); // 3000ms old > 2000ms max_age
        assert_eq!(agg.all_observations().len(), 0);
    }

    #[test]
    fn latest_per_camera() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig::default());
        agg.ingest(
            test_frame("c0", "a", 1000),
            vec!["x".into()],
            "first".into(),
            0.8,
            1000,
        );
        agg.ingest(
            test_frame("c0", "b", 1001),
            vec!["y".into()],
            "second".into(),
            0.9,
            1001,
        );
        assert_eq!(agg.latest("c0").unwrap().summary, "second");
    }

    #[test]
    fn no_image_bytes_in_observation() {
        let mut agg = VisualAggregator::new(VisualAggregatorConfig::default());
        let obs = agg
            .ingest(
                test_frame("c0", "aaa", 1000),
                vec!["obj".into()],
                "seen".into(),
                0.9,
                1000,
            )
            .unwrap();
        // FrameRef carries URI+hash only — no image bytes
        assert!(!obs.frame.uri.contains("data:"));
        assert!(!obs.frame.uri.contains("base64"));
    }
}
