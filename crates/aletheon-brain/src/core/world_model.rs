//! WorldModel — environment state tracking.
//!
//! Maintains a rolling log of observations about the environment,
//! and tracks entity state derived from those observations.
//! Provides snapshot summaries for the Reasoner to use as context.

use aletheon_abi::brain::Observation;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};

/// Tracked entity state derived from observations.
#[derive(Debug, Clone)]
pub struct EntityState {
    pub id: String,
    pub properties: serde_json::Value,
    pub last_updated: DateTime<Utc>,
    pub confidence: f64, // 0.0-1.0, decays over time
    pub observation_count: usize,
}

/// The world model component.
///
/// Tracks observations about the environment state. Provides
/// a snapshot summary for reasoning context. Also maintains
/// entity state derived from observations for change tracking.
pub struct WorldModel {
    /// Rolling log of observations with configurable capacity.
    observations: RwLock<VecDeque<Observation>>,
    /// Maximum number of observations to retain.
    max_observations: usize,
    /// Entity states keyed by entity ID (derived from observation source).
    entities: RwLock<HashMap<String, EntityState>>,
    /// Hash of the last entity snapshot for quick change detection.
    last_snapshot_hash: RwLock<u64>,
}

impl WorldModel {
    pub fn new(max_observations: usize) -> Self {
        Self {
            observations: RwLock::new(VecDeque::with_capacity(max_observations)),
            max_observations,
            entities: RwLock::new(HashMap::new()),
            last_snapshot_hash: RwLock::new(0),
        }
    }

    /// Update the world model with a new observation.
    ///
    /// Also updates the entity tracking for the observation's source.
    pub fn update(&self, observation: Observation) {
        // Update entity tracking first (uses observation fields before move)
        self.update_entity(&observation);

        let mut obs = self.observations.write();
        if obs.len() >= self.max_observations {
            obs.pop_front();
        }
        obs.push_back(observation);
    }

    /// Update or create an entity from an observation.
    ///
    /// The entity ID is derived from `observation.source`.
    /// The observation's `data` is merged into the entity's properties,
    /// and the `what` field is recorded as the latest event.
    pub fn update_entity(&self, observation: &Observation) {
        let now = Utc::now();
        let mut entities = self.entities.write();

        let entity = entities
            .entry(observation.source.clone())
            .or_insert_with(|| EntityState {
                id: observation.source.clone(),
                properties: serde_json::json!({}),
                last_updated: now,
                confidence: 1.0,
                observation_count: 0,
            });

        // Merge observation data into entity properties
        if let (Some(existing), Some(new_data)) = (
            entity.properties.as_object_mut(),
            observation.data.as_object(),
        ) {
            for (key, value) in new_data {
                existing.insert(key.clone(), value.clone());
            }
        }

        entity.last_updated = now;
        entity.confidence = 1.0;
        entity.observation_count += 1;

        // Update snapshot hash for change detection
        drop(entities);
        self.update_snapshot_hash();
    }

    /// Get current state of a tracked entity.
    pub fn get_entity(&self, id: &str) -> Option<EntityState> {
        self.entities.read().get(id).cloned()
    }

    /// Get all tracked entity states.
    pub fn entity_states(&self) -> Vec<EntityState> {
        self.entities.read().values().cloned().collect()
    }

    /// Quick change detection — returns true if entities changed since the given hash.
    pub fn changed_since(&self, last_hash: u64) -> bool {
        *self.last_snapshot_hash.read() != last_hash
    }

    /// Get the current snapshot hash for later change detection.
    pub fn snapshot_hash(&self) -> u64 {
        *self.last_snapshot_hash.read()
    }

    /// Derive high-level state descriptions from tracked entities.
    ///
    /// Returns a list of human-readable summary strings describing
    /// the current world state based on entity tracking.
    pub fn infer_state(&self) -> Vec<String> {
        let entities = self.entities.read();
        let mut lines = Vec::new();

        if entities.is_empty() {
            lines.push("No entities tracked".to_string());
            return lines;
        }

        let total = entities.len();
        lines.push(format!("{} entities active", total));

        // Categorize by confidence
        let healthy = entities.values().filter(|e| e.confidence >= 0.7).count();
        let degraded = entities.values().filter(|e| e.confidence < 0.7 && e.confidence >= 0.3).count();
        let lost = entities.values().filter(|e| e.confidence < 0.3).count();

        if degraded > 0 || lost > 0 {
            lines.push(format!(
                "system health: {} healthy, {} degraded, {} lost",
                healthy, degraded, lost
            ));
        } else {
            lines.push("system health: nominal".to_string());
        }

        // Entity detail
        for entity in entities.values() {
            lines.push(format!(
                "  [{}] confidence={:.2}, observations={}",
                entity.id, entity.confidence, entity.observation_count
            ));
        }

        lines
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
        obs.iter().filter(|o| o.source == source).cloned().collect()
    }

    /// Number of observations currently stored.
    pub fn count(&self) -> usize {
        self.observations.read().len()
    }

    /// Clear all observations and entity tracking.
    pub fn clear(&self) {
        self.observations.write().clear();
        self.entities.write().clear();
        *self.last_snapshot_hash.write() = 0;
    }

    /// Compute a simple hash over all entity states for change detection.
    fn update_snapshot_hash(&self) {
        let entities = self.entities.read();
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (id, entity) in entities.iter() {
            id.hash(&mut hasher);
            entity.properties.to_string().hash(&mut hasher);
            entity.observation_count.hash(&mut hasher);
        }
        *self.last_snapshot_hash.write() = hasher.finish();
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

    // --- Existing tests (preserved) ---

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

    // --- Entity tracking tests ---

    #[test]
    fn update_creates_entity() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("disk full", "system"));

        let entity = wm.get_entity("system");
        assert!(entity.is_some());
        let entity = entity.unwrap();
        assert_eq!(entity.id, "system");
        assert_eq!(entity.observation_count, 1);
        assert_eq!(entity.confidence, 1.0);
        assert_eq!(entity.properties["detail"], "disk full");
    }

    #[test]
    fn multiple_observations_update_same_entity() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("event1", "sensor_a"));
        wm.update(make_obs("event2", "sensor_a"));
        wm.update(make_obs("event3", "sensor_a"));

        let entity = wm.get_entity("sensor_a").unwrap();
        assert_eq!(entity.observation_count, 3);
        // Last observation's data should be merged
        assert_eq!(entity.properties["detail"], "event3");
    }

    #[test]
    fn different_sources_create_different_entities() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("a", "src1"));
        wm.update(make_obs("b", "src2"));

        assert!(wm.get_entity("src1").is_some());
        assert!(wm.get_entity("src2").is_some());
        assert_eq!(wm.entity_states().len(), 2);
    }

    #[test]
    fn entity_properties_merge_data() {
        let wm = WorldModel::new(100);
        wm.update(Observation {
            what: "temp".to_string(),
            source: "sensor".to_string(),
            data: json!({"temperature": 25.0}),
        });
        wm.update(Observation {
            what: "humidity".to_string(),
            source: "sensor".to_string(),
            data: json!({"humidity": 60.0}),
        });

        let entity = wm.get_entity("sensor").unwrap();
        assert_eq!(entity.properties["temperature"], 25.0);
        assert_eq!(entity.properties["humidity"], 60.0);
        assert_eq!(entity.observation_count, 2);
    }

    #[test]
    fn changed_since_detects_changes() {
        let wm = WorldModel::new(100);
        let hash_before = wm.snapshot_hash();
        assert!(!wm.changed_since(hash_before));

        wm.update(make_obs("event", "src"));
        assert!(wm.changed_since(hash_before));
    }

    #[test]
    fn changed_since_no_change() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("event", "src"));
        let hash_after = wm.snapshot_hash();
        assert!(!wm.changed_since(hash_after));
    }

    #[test]
    fn infer_state_empty() {
        let wm = WorldModel::new(100);
        let state = wm.infer_state();
        assert_eq!(state.len(), 1);
        assert!(state[0].contains("No entities"));
    }

    #[test]
    fn infer_state_all_healthy() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("ok", "src_a"));
        wm.update(make_obs("ok", "src_b"));

        let state = wm.infer_state();
        assert!(state[0].contains("2 entities"));
        assert!(state[1].contains("nominal"));
    }

    #[test]
    fn infer_state_degraded() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("ok", "src_a"));
        wm.update(make_obs("ok", "src_b"));
        // Manually set low confidence on one entity
        {
            let mut entities = wm.entities.write();
            if let Some(e) = entities.get_mut("src_b") {
                e.confidence = 0.2;
            }
        }

        let state = wm.infer_state();
        assert!(state[1].contains("degraded"));
    }

    #[test]
    fn clear_resets_entities() {
        let wm = WorldModel::new(100);
        wm.update(make_obs("event", "src"));
        assert!(wm.get_entity("src").is_some());

        wm.clear();
        assert!(wm.get_entity("src").is_none());
        assert_eq!(wm.entity_states().len(), 0);
    }
}
