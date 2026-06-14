//! WorldModel — environment state tracking.
//!
//! Maintains a rolling log of observations about the environment.
//! Provides snapshot summaries for the Reasoner to use as context.

use aletheon_abi::brain::Observation;
use parking_lot::RwLock;
use std::collections::VecDeque;

/// The world model component.
///
/// Tracks observations about the environment state. Provides
/// a snapshot summary for reasoning context.
pub struct WorldModel {
    /// Rolling log of observations with configurable capacity.
    observations: RwLock<VecDeque<Observation>>,
    /// Maximum number of observations to retain.
    max_observations: usize,
}

impl WorldModel {
    pub fn new(max_observations: usize) -> Self {
        Self {
            observations: RwLock::new(VecDeque::with_capacity(max_observations)),
            max_observations,
        }
    }

    /// Update the world model with a new observation.
    pub fn update(&self, observation: Observation) {
        let mut obs = self.observations.write();
        if obs.len() >= self.max_observations {
            obs.pop_front();
        }
        obs.push_back(observation);
    }

    /// Get a snapshot summary of current world state.
    pub fn snapshot(&self) -> String {
        let obs = self.observations.read();
        if obs.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        for o in obs.iter().rev().take(10) {
            lines.push(format!("[{}] {}: {}", o.source, o.what, o.data));
        }
        lines.join("\n")
    }

    /// Get all observations (newest first).
    pub fn recent(&self, limit: usize) -> Vec<Observation> {
        let obs = self.observations.read();
        obs.iter().rev().take(limit).cloned().collect()
    }

    /// Get observations from a specific source.
    pub fn from_source(&self, source: &str) -> Vec<Observation> {
        let obs = self.observations.read();
        obs.iter()
            .filter(|o| o.source == source)
            .cloned()
            .collect()
    }

    /// Number of observations currently stored.
    pub fn count(&self) -> usize {
        self.observations.read().len()
    }

    /// Clear all observations.
    pub fn clear(&self) {
        self.observations.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_obs(what: &str, source: &str) -> Observation {
        Observation {
            what: what.to_string(),
            source: source.to_string(),
            data: json!({"detail": what}),
        }
    }

    #[test]
    fn update_and_count() {
        let wm = WorldModel::new(100);
        assert_eq!(wm.count(), 0);
        wm.update(make_obs("disk full", "system"));
        assert_eq!(wm.count(), 1);
    }

    #[test]
    fn capacity_eviction() {
        let wm = WorldModel::new(3);
        wm.update(make_obs("a", "src"));
        wm.update(make_obs("b", "src"));
        wm.update(make_obs("c", "src"));
        wm.update(make_obs("d", "src"));
        assert_eq!(wm.count(), 3);
        // "a" should be evicted
        let recent = wm.recent(10);
        assert!(!recent.iter().any(|o| o.what == "a"));
        assert!(recent.iter().any(|o| o.what == "d"));
    }

    #[test]
    fn snapshot_empty() {
        let wm = WorldModel::new(100);
        assert_eq!(wm.snapshot(), "");
    }

    #[test]
    fn snapshot_formats_observations() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("event1", "sensor"));
        wm.update(make_obs("event2", "sensor"));
        let snap = wm.snapshot();
        assert!(snap.contains("event1"));
        assert!(snap.contains("event2"));
        assert!(snap.contains("sensor"));
    }

    #[test]
    fn snapshot_limits_to_10() {
        let wm = WorldModel::new(100);
        for i in 0..15 {
            wm.update(make_obs(&format!("event_{}", i), "src"));
        }
        let snap = wm.snapshot();
        // Should contain newest 10
        assert!(snap.contains("event_14"));
        assert!(!snap.contains("event_0"));
    }

    #[test]
    fn from_source_filter() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("a", "sensor_a"));
        wm.update(make_obs("b", "sensor_b"));
        wm.update(make_obs("c", "sensor_a"));
        let from_a = wm.from_source("sensor_a");
        assert_eq!(from_a.len(), 2);
        assert!(from_a.iter().all(|o| o.source == "sensor_a"));
    }

    #[test]
    fn clear_empties() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("a", "src"));
        wm.clear();
        assert_eq!(wm.count(), 0);
    }

    #[test]
    fn recent_respects_limit() {
        let wm = WorldModel::new(100);
        for i in 0..5 {
            wm.update(make_obs(&format!("e{}", i), "src"));
        }
        let recent = wm.recent(3);
        assert_eq!(recent.len(), 3);
        // Should be newest first
        assert!(recent[0].what == "e4");
    }
}
