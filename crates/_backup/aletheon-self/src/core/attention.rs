//! AttentionLayer — focus tracking with priority and decay.
//!
//! Tracks what the agent is currently focused on. Focus topics have
//! a priority that decays over time, so stale focus naturally fades.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::core::store::SelfFieldStore;

/// A focus topic the agent is attending to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusTopic {
    pub topic: String,
    pub priority: f64,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

/// AttentionLayer — manages focus topics with time-based decay.
pub struct AttentionLayer {
    focus: RwLock<Vec<FocusTopic>>,
    /// Priority decays by this amount per second of inactivity.
    decay_rate: f64,
}

impl AttentionLayer {
    pub fn new(decay_rate: f64) -> Self {
        Self {
            focus: RwLock::new(Vec::new()),
            decay_rate,
        }
    }

    /// Attend to a topic (adds or refreshes).
    pub fn attend(&self, topic: &str, priority: f64) {
        let mut focus = self.focus.write();
        if let Some(existing) = focus.iter_mut().find(|f| f.topic == topic) {
            existing.priority = priority;
            existing.last_updated = Utc::now();
        } else {
            focus.push(FocusTopic {
                topic: topic.to_string(),
                priority,
                started_at: Utc::now(),
                last_updated: Utc::now(),
            });
        }
    }

    /// Apply decay to all focus topics. Removes topics with priority <= 0.
    pub fn decay(&self) {
        let now = Utc::now();
        let mut focus = self.focus.write();
        for topic in focus.iter_mut() {
            let elapsed = (now - topic.last_updated).num_seconds() as f64;
            topic.priority -= self.decay_rate * elapsed;
        }
        focus.retain(|t| t.priority > 0.0);
    }

    /// Get the current highest-priority focus topic, after applying decay.
    pub fn current_focus(&self) -> Option<FocusTopic> {
        self.decay();
        self.focus.read().iter().max_by(|a, b| {
            a.priority.partial_cmp(&b.priority).unwrap_or(std::cmp::Ordering::Equal)
        }).cloned()
    }

    /// Get all active focus topics (after decay).
    pub fn all_topics(&self) -> Vec<FocusTopic> {
        self.decay();
        self.focus.read().clone()
    }

    /// Set a topic as the current auto-focus target.
    ///
    /// This sets the topic with a high default priority (0.9).
    /// The existing time-based decay still applies, so the focus
    /// will fade naturally if not refreshed.
    pub fn auto_focus(&self, topic: &str) {
        self.attend(topic, 0.9);
    }

    /// Persist all current focus topics to the store (replaces existing rows).
    pub fn save_to_store(&self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        conn.execute("DELETE FROM attention_topics", [])
            .context("Failed to clear attention_topics")?;

        let topics = self.focus.read();
        let mut stmt = conn
            .prepare(
                "INSERT INTO attention_topics (topic, priority, started_at, last_updated)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .context("Failed to prepare attention insert")?;
        for t in topics.iter() {
            stmt.execute(rusqlite::params![
                t.topic,
                t.priority,
                t.started_at.to_rfc3339(),
                t.last_updated.to_rfc3339(),
            ])
            .with_context(|| format!("Failed to insert topic '{}'", t.topic))?;
        }
        Ok(())
    }

    /// Load focus topics from the store, replacing the current in-memory state.
    pub fn load_from_store(&mut self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT topic, priority, started_at, last_updated
                 FROM attention_topics ORDER BY priority DESC",
            )
            .context("Failed to prepare attention select")?;

        let rows = stmt
            .query_map([], |row| {
                let started_str: String = row.get(2)?;
                let updated_str: String = row.get(3)?;
                Ok(FocusTopic {
                    topic: row.get(0)?,
                    priority: row.get(1)?,
                    started_at: DateTime::parse_from_rfc3339(&started_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    last_updated: DateTime::parse_from_rfc3339(&updated_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .context("Failed to query attention_topics")?;

        let mut topics = Vec::new();
        for row in rows {
            topics.push(row.context("Failed to read attention row")?);
        }

        let mut focus = self.focus.write();
        *focus = topics;
        Ok(())
    }

    /// Remove a specific focus topic.
    pub fn dismiss(&self, topic: &str) -> bool {
        let mut focus = self.focus.write();
        let len_before = focus.len();
        focus.retain(|f| f.topic != topic);
        focus.len() < len_before
    }
}

impl Default for AttentionLayer {
    fn default() -> Self {
        // Default decay: 0.001 priority per second (~1.0 per 1000 seconds)
        Self::new(0.001)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attend_and_focus() {
        let layer = AttentionLayer::new(0.0); // no decay for testing
        layer.attend("task_a", 0.8);
        layer.attend("task_b", 0.5);

        let focus = layer.current_focus();
        assert!(focus.is_some());
        assert_eq!(focus.unwrap().topic, "task_a");
    }

    #[test]
    fn refresh_updates_priority() {
        let layer = AttentionLayer::new(0.0);
        layer.attend("task_a", 0.3);
        layer.attend("task_a", 0.9);

        let topics = layer.all_topics();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].priority, 0.9);
    }

    #[test]
    fn dismiss_removes_topic() {
        let layer = AttentionLayer::new(0.0);
        layer.attend("task_a", 0.5);
        assert!(layer.dismiss("task_a"));
        assert!(layer.current_focus().is_none());
        assert!(!layer.dismiss("nonexistent"));
    }

    #[test]
    fn auto_focus_sets_high_priority() {
        let layer = AttentionLayer::new(0.0);
        layer.auto_focus("debug_session");
        let focus = layer.current_focus();
        assert!(focus.is_some());
        let f = focus.unwrap();
        assert_eq!(f.topic, "debug_session");
        assert!((f.priority - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn auto_focus_overrides_existing() {
        let layer = AttentionLayer::new(0.0);
        layer.attend("task_a", 0.3);
        layer.auto_focus("task_a");
        let topics = layer.all_topics();
        assert_eq!(topics.len(), 1);
        assert!((topics[0].priority - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn save_and_load_roundtrip() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = AttentionLayer::new(0.0);
        layer.attend("topic_a", 0.8);
        layer.attend("topic_b", 0.5);

        layer.save_to_store(&store).unwrap();

        let mut loaded = AttentionLayer::new(0.0);
        loaded.load_from_store(&store).unwrap();

        let topics = loaded.all_topics();
        assert_eq!(topics.len(), 2);
        let highest = loaded.current_focus().unwrap();
        assert_eq!(highest.topic, "topic_a");
        assert!((highest.priority - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn save_clears_previous() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = AttentionLayer::new(0.0);
        layer.attend("old_topic", 0.7);
        layer.save_to_store(&store).unwrap();

        let layer2 = AttentionLayer::new(0.0);
        layer2.attend("new_topic", 0.9);
        layer2.save_to_store(&store).unwrap();

        let mut loaded = AttentionLayer::new(0.0);
        loaded.load_from_store(&store).unwrap();
        let topics = loaded.all_topics();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].topic, "new_topic");
    }
}
