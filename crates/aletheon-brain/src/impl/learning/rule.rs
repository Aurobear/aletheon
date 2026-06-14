use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A learned rule from the self-learning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnRule {
    pub id: String,
    pub rule_type: String,  // "warning", "avoid", "prefer"
    pub tool_pattern: String,
    pub condition: String,
    pub action: String,
    pub examples: Vec<String>,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

/// Stores learned rules.
pub struct RuleStore {
    rules: Vec<LearnRule>,
    max_rules: usize,
}

impl RuleStore {
    pub fn new(max_rules: usize) -> Self {
        Self {
            rules: Vec::new(),
            max_rules,
        }
    }

    /// Add a rule, evicting lowest-confidence if at capacity.
    pub fn add(&mut self, rule: LearnRule) {
        if self.rules.len() >= self.max_rules {
            // Evict lowest confidence
            if let Some(min_idx) = self.rules.iter()
                .enumerate()
                .min_by(|a, b| a.1.confidence.partial_cmp(&b.1.confidence).unwrap())
                .map(|(i, _)| i)
            {
                if self.rules[min_idx].confidence < rule.confidence {
                    self.rules.remove(min_idx);
                } else {
                    return; // New rule has lower confidence, skip
                }
            }
        }
        self.rules.push(rule);
    }

    /// Get rules matching a tool.
    pub fn get_for_tool(&self, tool_name: &str) -> Vec<&LearnRule> {
        self.rules.iter()
            .filter(|r| {
                if r.tool_pattern.ends_with('*') {
                    tool_name.starts_with(&r.tool_pattern[..r.tool_pattern.len() - 1])
                } else {
                    r.tool_pattern == tool_name
                }
            })
            .collect()
    }

    /// Format rules for context injection.
    pub fn format_for_context(&self) -> String {
        if self.rules.is_empty() {
            return String::new();
        }

        let mut result = String::from("## Learned Rules\n");
        for rule in &self.rules {
            result.push_str(&format!(
                "- [{}] {} (confidence: {:.0}%): {}\n",
                rule.rule_type, rule.tool_pattern, rule.confidence * 100.0, rule.action
            ));
        }
        result
    }

    pub fn count(&self) -> usize {
        self.rules.len()
    }
}
