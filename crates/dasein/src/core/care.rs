//! CareLayer — weighted concerns that influence action scoring.
//!
//! Cares represent what the agent values. Each care has a weight (0.0–1.0)
//! and a set of keywords. `score_action()` computes a weighted relevance
//! score for a given action description.

#[cfg(test)]
use std::collections::HashMap;

use anyhow::Result;
use fabric::Care;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// A weighted concern with associated keywords.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareEntry {
    pub care: Care,
    /// Keywords that activate this care when found in an action description.
    pub keywords: Vec<String>,
}

/// CareLayer — manages weighted concerns and scores actions.
pub struct CareLayer {
    cares: RwLock<Vec<CareEntry>>,
}

impl CareLayer {
    pub fn new() -> Self {
        Self {
            cares: RwLock::new(Self::default_cares()),
        }
    }

    /// Default care set: safety(1.0), user_intent(0.8), efficiency(0.5), learning(0.3).
    fn default_cares() -> Vec<CareEntry> {
        vec![
            CareEntry {
                care: Care {
                    topic: "safety".to_string(),
                    weight: 1.0,
                    description: "Physical and digital safety of the agent and its environment"
                        .to_string(),
                },
                keywords: vec![
                    "safety".to_string(),
                    "danger".to_string(),
                    "risk".to_string(),
                    "harm".to_string(),
                    "damage".to_string(),
                    "destroy".to_string(),
                ],
            },
            CareEntry {
                care: Care {
                    topic: "user_intent".to_string(),
                    weight: 0.8,
                    description: "Fulfilling the user's actual intent accurately".to_string(),
                },
                keywords: vec![
                    "user".to_string(),
                    "request".to_string(),
                    "intent".to_string(),
                    "goal".to_string(),
                ],
            },
            CareEntry {
                care: Care {
                    topic: "efficiency".to_string(),
                    weight: 0.5,
                    description: "Completing tasks with minimal resource usage".to_string(),
                },
                keywords: vec![
                    "efficiency".to_string(),
                    "fast".to_string(),
                    "optimize".to_string(),
                    "resource".to_string(),
                ],
            },
            CareEntry {
                care: Care {
                    topic: "learning".to_string(),
                    weight: 0.3,
                    description: "Acquiring new knowledge and improving over time".to_string(),
                },
                keywords: vec![
                    "learn".to_string(),
                    "study".to_string(),
                    "improve".to_string(),
                    "adapt".to_string(),
                ],
            },
        ]
    }

    /// Get all current cares.
    pub fn all_cares(&self) -> Vec<Care> {
        self.cares.read().iter().map(|e| e.care.clone()).collect()
    }

    /// Add a new care. If a care with the same topic exists, it is replaced.
    #[cfg(test)]
    pub(crate) fn add_care(&self, entry: CareEntry) {
        let mut cares = self.cares.write();
        if let Some(existing) = cares.iter_mut().find(|c| c.care.topic == entry.care.topic) {
            *existing = entry;
        } else {
            cares.push(entry);
        }
    }

    /// Remove a care by topic. Returns true if found and removed.
    #[cfg(test)]
    pub(crate) fn remove_care(&self, topic: &str) -> bool {
        let mut cares = self.cares.write();
        let len_before = cares.len();
        cares.retain(|c| c.care.topic != topic);
        cares.len() < len_before
    }

    /// Score an action description against all cares.
    /// Returns a weighted sum: each matching care contributes `weight * keyword_match_ratio`.
    /// Result is in [0.0, sum_of_weights].
    pub fn score_action(&self, description: &str) -> f64 {
        let desc_lower = description.to_lowercase();
        let cares = self.cares.read();
        let mut score = 0.0;

        for entry in cares.iter() {
            if entry.keywords.is_empty() {
                continue;
            }
            let matches = entry
                .keywords
                .iter()
                .filter(|kw| desc_lower.contains(kw.as_str()))
                .count();
            let ratio = matches as f64 / entry.keywords.len() as f64;
            score += entry.care.weight * ratio;
        }

        score
    }

    /// Get the weight of a specific care topic. Returns None if not found.
    pub fn weight_of(&self, topic: &str) -> Option<f64> {
        self.cares
            .read()
            .iter()
            .find(|c| c.care.topic == topic)
            .map(|c| c.care.weight)
    }

    /// Adjust the weight of a care by a delta value.
    ///
    /// - Clamps the resulting weight to `[0.1, 1.0]`.
    /// - The `"safety"` care can never go below `0.8`.
    /// - Returns `Some((old_value, new_value))` if the care was found, `None` otherwise.
    #[cfg(test)]
    pub(crate) fn adjust_weight(&self, care_name: &str, delta: f64) -> Option<(f64, f64)> {
        let mut cares = self.cares.write();
        let entry = cares.iter_mut().find(|c| c.care.topic == care_name)?;
        let old = entry.care.weight;
        let mut new_val = old + delta;

        // Safety floor
        if care_name == "safety" {
            new_val = new_val.max(0.8);
        }

        // General clamp
        new_val = new_val.clamp(0.1, 1.0);

        entry.care.weight = new_val;
        Some((old, new_val))
    }

    /// Record an outcome and adjust care weights based on experience.
    ///
    /// - If `success` is true and safety care was high, slightly reduce safety
    ///   weight (was over-cautious).
    /// - If `success` is false and safety care was low, increase safety weight.
    /// - If `success` is true and elapsed time is fast (< 2000ms), increase
    ///   efficiency weight.
    ///
    /// `care_scores` maps care topic names to the scores they had during the
    /// task (as returned by `score_action`-like logic). Only cares that
    /// *matched* (score > 0) are considered.
    #[cfg(test)]
    pub(crate) fn record_outcome(
        &self,
        success: bool,
        elapsed_ms: u64,
        care_scores: &HashMap<String, f64>,
    ) {
        self.adjust_from_outcome(success, elapsed_ms, care_scores);
    }

    /// Internal implementation of outcome-based weight adjustment.
    ///
    /// Adjustment rules (each 0.01-0.05 magnitude to avoid oscillation):
    ///
    /// 1. Success + high safety score (>= 0.5) → reduce safety by 0.02
    ///    (was over-cautious).
    /// 2. Failure + low safety score (< 0.2) → increase safety by 0.03
    ///    (under-weighted safety).
    /// 3. Success + fast (< 2000ms) → increase efficiency by 0.02.
    /// 4. Failure + slow (>= 10000ms) → increase efficiency by 0.03.
    #[cfg(test)]
    pub(crate) fn adjust_from_outcome(
        &self,
        success: bool,
        elapsed_ms: u64,
        care_scores: &HashMap<String, f64>,
    ) {
        let safety_score = care_scores.get("safety").copied().unwrap_or(0.0);

        if success && safety_score >= 0.5 {
            // Over-cautious: slightly relax safety weight
            self.adjust_weight("safety", -0.02);
        } else if !success && safety_score < 0.2 {
            // Under-cautious: bump safety weight
            self.adjust_weight("safety", 0.03);
        }

        if success && elapsed_ms < 2000 {
            // Fast success: reward efficiency
            self.adjust_weight("efficiency", 0.02);
        } else if !success && elapsed_ms >= 10_000 {
            // Slow failure: push for more efficiency
            self.adjust_weight("efficiency", 0.03);
        }
    }

    /// Persist all cares to the SQLite store.
    pub fn save_to_store(&self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let cares = self.cares.read();

        conn.execute("DELETE FROM care_entries", [])?;

        let mut stmt = conn.prepare(
            "INSERT INTO care_entries (topic, weight, description, keywords) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for entry in cares.iter() {
            let keywords_json = serde_json::to_string(&entry.keywords)?;
            stmt.execute(rusqlite::params![
                entry.care.topic,
                entry.care.weight,
                entry.care.description,
                keywords_json,
            ])?;
        }
        Ok(())
    }

    /// Load cares from the SQLite store, replacing current state.
    pub fn load_from_store(&mut self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let mut stmt =
            conn.prepare("SELECT topic, weight, description, keywords FROM care_entries")?;

        let loaded: Vec<CareEntry> = stmt
            .query_map([], |row| {
                let keywords_json: String = row.get(3)?;
                let keywords: Vec<String> =
                    serde_json::from_str(&keywords_json).unwrap_or_default();
                Ok(CareEntry {
                    care: Care {
                        topic: row.get(0)?,
                        weight: row.get(1)?,
                        description: row.get(2)?,
                    },
                    keywords,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if !loaded.is_empty() {
            *self.cares.write() = loaded;
        }
        Ok(())
    }
}

impl Default for CareLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cares() {
        let layer = CareLayer::new();
        let cares = layer.all_cares();
        assert_eq!(cares.len(), 4);
        assert!(cares.iter().any(|c| c.topic == "safety"));
        assert!(cares.iter().any(|c| c.topic == "user_intent"));
        assert!(cares.iter().any(|c| c.topic == "efficiency"));
        assert!(cares.iter().any(|c| c.topic == "learning"));
    }

    #[test]
    fn add_remove() {
        let layer = CareLayer::new();
        layer.add_care(CareEntry {
            care: Care {
                topic: "privacy".to_string(),
                weight: 0.9,
                description: "data privacy".to_string(),
            },
            keywords: vec!["private".to_string(), "secret".to_string()],
        });
        assert_eq!(layer.all_cares().len(), 5);

        assert!(layer.remove_care("privacy"));
        assert_eq!(layer.all_cares().len(), 4);
        assert!(!layer.remove_care("nonexistent"));
    }

    #[test]
    fn score_safety_keyword() {
        let layer = CareLayer::new();
        let score = layer.score_action("this action involves safety and danger");
        // safety care: keywords = [safety, danger, risk, harm, damage, destroy]
        // matches 2/6 = 0.333... * weight 1.0 = 0.333...
        assert!(score > 0.3);
        assert!(score < 0.4);
    }

    #[test]
    fn score_no_match() {
        let layer = CareLayer::new();
        let score = layer.score_action("hello world foo bar");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn score_multiple_cares() {
        let layer = CareLayer::new();
        let score = layer.score_action("optimize safety for user request");
        // safety: 1 match (safety) / 6 keywords * 1.0 = 0.167
        // user_intent: 2 matches (user, request) / 4 keywords * 0.8 = 0.4
        // efficiency: 1 match (optimize) / 4 keywords * 0.5 = 0.125
        // learning: 0 matches
        // total = ~0.692
        assert!(score > 0.6);
    }

    #[test]
    fn weight_of() {
        let layer = CareLayer::new();
        assert_eq!(layer.weight_of("safety"), Some(1.0));
        assert_eq!(layer.weight_of("nonexistent"), None);
    }

    #[test]
    fn adjust_weight_basic() {
        let layer = CareLayer::new();
        // efficiency starts at 0.5, adjust by +0.2
        let result = layer.adjust_weight("efficiency", 0.2);
        assert_eq!(result, Some((0.5, 0.7)));
        assert_eq!(layer.weight_of("efficiency"), Some(0.7));
    }

    #[test]
    fn adjust_weight_clamp_upper() {
        let layer = CareLayer::new();
        // user_intent starts at 0.8, adjust by +0.5 -> clamped to 1.0
        let result = layer.adjust_weight("user_intent", 0.5);
        assert_eq!(result, Some((0.8, 1.0)));
    }

    #[test]
    fn adjust_weight_clamp_lower() {
        let layer = CareLayer::new();
        // learning starts at 0.3, adjust by -0.5 -> clamped to 0.1
        let result = layer.adjust_weight("learning", -0.5);
        assert_eq!(result, Some((0.3, 0.1)));
    }

    #[test]
    fn adjust_weight_safety_floor() {
        let layer = CareLayer::new();
        // safety starts at 1.0, adjust by -0.5 -> safety floor is 0.8
        let result = layer.adjust_weight("safety", -0.5);
        assert_eq!(result, Some((1.0, 0.8)));
    }

    #[test]
    fn adjust_weight_nonexistent() {
        let layer = CareLayer::new();
        assert!(layer.adjust_weight("nonexistent", 0.1).is_none());
    }

    #[test]
    fn record_outcome_success_high_safety_reduces_safety() {
        let layer = CareLayer::new();
        let mut scores = HashMap::new();
        scores.insert("safety".to_string(), 0.6);
        layer.record_outcome(true, 500, &scores);
        // Safety was 1.0, over-cautious success → should be reduced by 0.02
        let w = layer.weight_of("safety").unwrap();
        assert!((w - 0.98).abs() < 1e-9, "expected ~0.98, got {w}");
    }

    #[test]
    fn record_outcome_failure_low_safety_increases_safety() {
        let layer = CareLayer::new();
        // First lower safety so it's not already at 1.0
        layer.adjust_weight("safety", -0.25); // 1.0 → 0.75 (clamped to 0.8)

        let mut scores = HashMap::new();
        scores.insert("safety".to_string(), 0.1);
        layer.record_outcome(false, 5000, &scores);
        // Safety was 0.8, under-cautious failure → increase by 0.03
        let w = layer.weight_of("safety").unwrap();
        assert!((w - 0.83).abs() < 1e-9, "expected ~0.83, got {w}");
    }

    #[test]
    fn record_outcome_success_fast_increases_efficiency() {
        let layer = CareLayer::new();
        let scores = HashMap::new(); // no safety score → doesn't trigger safety rule
        layer.record_outcome(true, 100, &scores);
        // Efficiency was 0.5, fast success → +0.02
        let w = layer.weight_of("efficiency").unwrap();
        assert!((w - 0.52).abs() < 1e-9, "expected ~0.52, got {w}");
    }

    #[test]
    fn record_outcome_failure_slow_increases_efficiency() {
        let layer = CareLayer::new();
        let scores = HashMap::new();
        layer.record_outcome(false, 15_000, &scores);
        // Efficiency was 0.5, slow failure → +0.03
        let w = layer.weight_of("efficiency").unwrap();
        assert!((w - 0.53).abs() < 1e-9, "expected ~0.53, got {w}");
    }

    #[test]
    fn record_outcome_safety_floor_preserved() {
        let layer = CareLayer::new();
        // Bring safety to floor
        layer.adjust_weight("safety", -0.5); // clamped to 0.8
        assert_eq!(layer.weight_of("safety"), Some(0.8));

        let mut scores = HashMap::new();
        scores.insert("safety".to_string(), 0.8);
        // Success with high safety → try to reduce, but floor is 0.8
        layer.record_outcome(true, 3000, &scores);
        assert_eq!(layer.weight_of("safety"), Some(0.8));
    }

    #[test]
    fn adjust_from_outcome_neutral_timing_no_efficiency_change() {
        let layer = CareLayer::new();
        let scores = HashMap::new();
        // 5000ms: not fast (<2000) and not slow (>=10000) → no efficiency change
        layer.adjust_from_outcome(true, 5000, &scores);
        assert_eq!(layer.weight_of("efficiency"), Some(0.5));
    }
}
