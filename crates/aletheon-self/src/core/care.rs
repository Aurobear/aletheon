//! CareLayer — weighted concerns that influence action scoring.
//!
//! Cares represent what the agent values. Each care has a weight (0.0–1.0)
//! and a set of keywords. `score_action()` computes a weighted relevance
//! score for a given action description.

use aletheon_abi::Care;
use parking_lot::RwLock;

/// A weighted concern with associated keywords.
#[derive(Debug, Clone)]
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
                    description: "Physical and digital safety of the agent and its environment".to_string(),
                },
                keywords: vec!["safety".to_string(), "danger".to_string(), "risk".to_string(), "harm".to_string(), "damage".to_string(), "destroy".to_string()],
            },
            CareEntry {
                care: Care {
                    topic: "user_intent".to_string(),
                    weight: 0.8,
                    description: "Fulfilling the user's actual intent accurately".to_string(),
                },
                keywords: vec!["user".to_string(), "request".to_string(), "intent".to_string(), "goal".to_string()],
            },
            CareEntry {
                care: Care {
                    topic: "efficiency".to_string(),
                    weight: 0.5,
                    description: "Completing tasks with minimal resource usage".to_string(),
                },
                keywords: vec!["efficiency".to_string(), "fast".to_string(), "optimize".to_string(), "resource".to_string()],
            },
            CareEntry {
                care: Care {
                    topic: "learning".to_string(),
                    weight: 0.3,
                    description: "Acquiring new knowledge and improving over time".to_string(),
                },
                keywords: vec!["learn".to_string(), "study".to_string(), "improve".to_string(), "adapt".to_string()],
            },
        ]
    }

    /// Get all current cares.
    pub fn all_cares(&self) -> Vec<Care> {
        self.cares.read().iter().map(|e| e.care.clone()).collect()
    }

    /// Add a new care. If a care with the same topic exists, it is replaced.
    pub fn add_care(&self, entry: CareEntry) {
        let mut cares = self.cares.write();
        if let Some(existing) = cares.iter_mut().find(|c| c.care.topic == entry.care.topic) {
            *existing = entry;
        } else {
            cares.push(entry);
        }
    }

    /// Remove a care by topic. Returns true if found and removed.
    pub fn remove_care(&self, topic: &str) -> bool {
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
            let matches = entry.keywords.iter().filter(|kw| desc_lower.contains(kw.as_str())).count();
            let ratio = matches as f64 / entry.keywords.len() as f64;
            score += entry.care.weight * ratio;
        }

        score
    }

    /// Get the weight of a specific care topic. Returns None if not found.
    pub fn weight_of(&self, topic: &str) -> Option<f64> {
        self.cares.read().iter().find(|c| c.care.topic == topic).map(|c| c.care.weight)
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
}
