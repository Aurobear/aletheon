//! Learner — extracts learned rules from experience.
//!
//! Analyzes completed experiences (action + result + context) to produce
//! reusable LearnedRule structs. Maintains a history for dedup and
//! confidence tracking.

use aletheon_abi::brain::{Experience, LearnedRule};
use parking_lot::RwLock;
use std::collections::HashMap;

/// The learner component.
///
/// Extracts patterns from experience and produces learned rules.
/// Maintains an internal rule history for dedup and confidence refinement.
pub struct Learner {
    /// Map from pattern string to rule, for dedup.
    rules: RwLock<HashMap<String, LearnedRule>>,
    /// Maximum number of rules to retain.
    max_rules: usize,
}

impl Learner {
    pub fn new(max_rules: usize) -> Self {
        Self {
            rules: RwLock::new(HashMap::new()),
            max_rules,
        }
    }

    /// Learn from an experience — extract rules.
    pub fn learn(&self, experience: &Experience) -> Vec<LearnedRule> {
        let mut new_rules = Vec::new();

        let action_name = &experience.action.name;
        let success = experience.result.success;

        // Pattern 1: Failed action → create "avoid" or "retry" rule
        if !success {
            let error_summary = experience
                .result
                .error
                .as_deref()
                .unwrap_or("unknown error");

            let pattern = format!(
                "action:{} error:{}",
                action_name,
                self.error_prefix(error_summary)
            );
            let rule = LearnedRule {
                id: uuid::Uuid::new_v4().to_string(),
                pattern: pattern.clone(),
                action: format!(
                    "Consider alternative to '{}' — failed with: {}",
                    action_name, error_summary
                ),
                confidence: 0.6,
                examples: vec![format!("{}: {}", action_name, error_summary)],
            };

            self.upsert_rule(pattern, rule.clone());
            new_rules.push(rule);
        }

        // Pattern 2: Successful destructive action → create "prefer_with_backup" rule
        if success {
            let action_lower = action_name.to_lowercase();
            if action_lower.contains("delete") || action_lower.contains("rm") {
                let pattern = format!("action:{}", action_name);
                let rule = LearnedRule {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern: pattern.clone(),
                    action: format!("Always create backup before '{}'", action_name),
                    confidence: 0.8,
                    examples: vec![format!("{} succeeded — ensure backups exist", action_name)],
                };

                self.upsert_rule(pattern, rule.clone());
                new_rules.push(rule);
            }
        }

        // Pattern 3: Slow action → create "optimize" rule
        if experience.result.elapsed_ms > 10_000 {
            let pattern = format!("action:{} slow", action_name);
            let rule = LearnedRule {
                id: uuid::Uuid::new_v4().to_string(),
                pattern: pattern.clone(),
                action: format!(
                    "'{}' took {}ms — consider optimizing or caching",
                    action_name, experience.result.elapsed_ms
                ),
                confidence: 0.5,
                examples: vec![format!(
                    "{} took {}ms",
                    action_name, experience.result.elapsed_ms
                )],
            };

            self.upsert_rule(pattern, rule.clone());
            new_rules.push(rule);
        }

        new_rules
    }

    /// Get all learned rules.
    pub fn all_rules(&self) -> Vec<LearnedRule> {
        self.rules.read().values().cloned().collect()
    }

    /// Get a rule by pattern.
    pub fn get_by_pattern(&self, pattern: &str) -> Option<LearnedRule> {
        self.rules.read().get(pattern).cloned()
    }

    /// Number of rules currently stored.
    pub fn count(&self) -> usize {
        self.rules.read().len()
    }

    /// Get rules whose pattern matches a context string.
    ///
    /// Matching strategy:
    /// - Exact substring match (context in pattern, or pattern in context)
    /// - Word-level match: if any word from context (split on whitespace/colons/dots)
    ///   appears as a substring in the pattern
    ///
    /// Returns rules formatted as a bullet-point string suitable for LLM prompt injection.
    pub fn rules_for_context(&self, context: &str) -> String {
        let rules = self.rules.read();
        let context_lower = context.to_lowercase();
        let context_words: Vec<&str> = context_lower
            .split(|c: char| c.is_whitespace() || c == ':' || c == '.')
            .filter(|w| w.len() >= 2)
            .collect();
        let matched: Vec<&LearnedRule> = rules
            .values()
            .filter(|r| {
                let pattern_lower = r.pattern.to_lowercase();
                // Direct substring match
                if context_lower.contains(&pattern_lower)
                    || pattern_lower.contains(&context_lower)
                {
                    return true;
                }
                // Word-level match: any context word appears in pattern
                context_words
                    .iter()
                    .any(|w| pattern_lower.contains(w))
            })
            .collect();

        if matched.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Learned rules:".to_string()];
        for rule in &matched {
            lines.push(format!(
                "- [conf={:.2}] {} ({})",
                rule.confidence, rule.action, rule.pattern
            ));
        }
        lines.join("\n")
    }

    /// Upsert a rule — if pattern exists, merge (increase confidence, add examples).
    /// If at capacity and new rule has higher confidence than the lowest, replace.
    fn upsert_rule(&self, pattern: String, rule: LearnedRule) {
        let mut rules = self.rules.write();

        // If pattern already exists, merge
        if let Some(existing) = rules.get_mut(&pattern) {
            existing.confidence = (existing.confidence + rule.confidence) / 2.0;
            for example in &rule.examples {
                if !existing.examples.contains(example) {
                    existing.examples.push(example.clone());
                }
            }
            return;
        }

        // If at capacity, check if we should evict
        if rules.len() >= self.max_rules {
            if let Some((min_pattern, min_confidence)) = rules
                .iter()
                .map(|(p, r)| (p.clone(), r.confidence))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            {
                if min_confidence < rule.confidence {
                    rules.remove(&min_pattern);
                } else {
                    return; // New rule has lower confidence, skip
                }
            }
        }

        rules.insert(pattern, rule);
    }

    /// Extract prefix of an error message for pattern matching.
    fn error_prefix(&self, error: &str) -> String {
        error.chars().take(60).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::body::{Action, ActionResult};
    use aletheon_abi::brain::Experience;
    use aletheon_abi::context::Context;
    use serde_json::json;
    use std::path::PathBuf;

    fn make_experience(
        action_name: &str,
        success: bool,
        error: Option<&str>,
        elapsed_ms: u64,
    ) -> Experience {
        Experience {
            action: Action {
                name: action_name.to_string(),
                parameters: json!({}),
                requires_sandbox: false,
                timeout: None,
            },
            result: ActionResult {
                success,
                output: "output".to_string(),
                error: error.map(|s| s.to_string()),
                elapsed_ms,
                truncated: false,
                side_effects: vec![],
            },
            context: Context::new("test", PathBuf::from("/tmp")),
        }
    }

    #[test]
    fn failed_action_creates_rule() {
        let learner = Learner::new(100);
        let exp = make_experience("shell.execute", false, Some("permission denied"), 100);
        let rules = learner.learn(&exp);
        assert!(!rules.is_empty());
        assert!(rules[0].pattern.contains("shell.execute"));
        assert!(rules[0].action.contains("permission denied"));
    }

    #[test]
    fn successful_destructive_creates_backup_rule() {
        let learner = Learner::new(100);
        let exp = make_experience("file.delete", true, None, 100);
        let rules = learner.learn(&exp);
        assert!(rules.iter().any(|r| r.action.contains("backup")));
    }

    #[test]
    fn slow_action_creates_optimize_rule() {
        let learner = Learner::new(100);
        let exp = make_experience("shell.execute", true, None, 15_000);
        let rules = learner.learn(&exp);
        assert!(rules.iter().any(|r| r.action.contains("optimizing")));
    }

    #[test]
    fn successful_fast_action_no_rules() {
        let learner = Learner::new(100);
        let exp = make_experience("file.read", true, None, 100);
        let rules = learner.learn(&exp);
        assert!(rules.is_empty());
    }

    #[test]
    fn rule_dedup_merges_confidence() {
        let learner = Learner::new(100);
        let exp = make_experience("shell.execute", false, Some("timeout"), 100);
        learner.learn(&exp);
        learner.learn(&exp);
        assert_eq!(learner.count(), 1);
        let rule = learner
            .get_by_pattern(&format!("action:{} error:timeout", "shell.execute"))
            .unwrap();
        // Confidence should be averaged
        assert!(rule.confidence > 0.5);
    }

    #[test]
    fn eviction_respects_max_rules() {
        let learner = Learner::new(2);
        learner.learn(&make_experience("tool_a", false, Some("err_a"), 100));
        learner.learn(&make_experience("tool_b", false, Some("err_b"), 100));
        assert_eq!(learner.count(), 2);

        // Add a third — should evict lowest confidence
        learner.learn(&make_experience("tool_c", false, Some("err_c"), 100));
        assert!(learner.count() <= 2);
    }

    #[test]
    fn all_rules_returns_cloned() {
        let learner = Learner::new(100);
        learner.learn(&make_experience("tool_a", false, Some("err"), 100));
        let rules = learner.all_rules();
        assert!(!rules.is_empty());
    }
}
