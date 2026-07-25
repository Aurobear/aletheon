use super::outcome::OutcomeRecord;
use super::rule::LearnRule;
use fabric::Clock;
use std::sync::Arc;
use tracing::info;

/// Extracts patterns from outcomes to generate learning rules.
pub struct PatternExtractor {
    min_occurrences: usize,
    success_threshold: f64,
    clock: Arc<dyn Clock>,
}

impl PatternExtractor {
    pub fn new(min_occurrences: usize, success_threshold: f64, clock: Arc<dyn Clock>) -> Self {
        Self {
            min_occurrences,
            success_threshold,
            clock,
        }
    }

    /// Analyze outcomes and extract patterns.
    pub fn extract(&self, outcomes: &[OutcomeRecord]) -> Vec<LearnRule> {
        let mut rules = Vec::new();

        // Group by tool name
        let mut by_tool: std::collections::HashMap<&str, Vec<&OutcomeRecord>> =
            std::collections::HashMap::new();
        for outcome in outcomes {
            by_tool.entry(&outcome.tool_name).or_default().push(outcome);
        }

        for (tool_name, tool_outcomes) in &by_tool {
            // Count successes and failures
            let successes = tool_outcomes.iter().filter(|o| !o.is_error).count();
            let failures = tool_outcomes.iter().filter(|o| o.is_error).count();
            let total = successes + failures;

            if total < self.min_occurrences {
                continue;
            }

            let success_rate = successes as f64 / total as f64;

            // If failure rate is high, create a warning rule
            if success_rate < self.success_threshold && failures >= self.min_occurrences {
                let common_errors: Vec<String> = tool_outcomes
                    .iter()
                    .filter(|o| o.is_error)
                    .map(|o| o.result_summary.clone())
                    .collect();

                let rule = LearnRule {
                    id: uuid::Uuid::new_v4().to_string(),
                    rule_type: "warning".to_string(),
                    tool_pattern: tool_name.to_string(),
                    condition: format!("success_rate < {}", self.success_threshold),
                    action: "warn_before_execute".to_string(),
                    examples: common_errors.into_iter().take(5).collect(),
                    confidence: 1.0 - success_rate,
                    created_at: fabric::wall_to_datetime(self.clock.wall_now()),
                };

                info!(tool = tool_name, rule = %rule.id, "Extracted warning rule");
                rules.push(rule);
            }

            // Check for common failure patterns
            let error_patterns = self.find_error_patterns(tool_outcomes);
            for pattern in error_patterns {
                let rule = LearnRule {
                    id: uuid::Uuid::new_v4().to_string(),
                    rule_type: "avoid".to_string(),
                    tool_pattern: tool_name.to_string(),
                    condition: pattern.condition,
                    action: "suggest_alternative".to_string(),
                    examples: pattern.examples,
                    confidence: pattern.confidence,
                    created_at: fabric::wall_to_datetime(self.clock.wall_now()),
                };
                rules.push(rule);
            }
        }

        rules
    }

    fn find_error_patterns(&self, outcomes: &[&OutcomeRecord]) -> Vec<ErrorPattern> {
        let mut patterns = Vec::new();

        // Group errors by similar messages
        let errors: Vec<&OutcomeRecord> = outcomes.iter().filter(|o| o.is_error).copied().collect();
        if errors.len() < self.min_occurrences {
            return patterns;
        }

        // Simple pattern: same error message prefix
        let mut by_prefix: std::collections::HashMap<String, Vec<&OutcomeRecord>> =
            std::collections::HashMap::new();
        for error in &errors {
            let prefix: String = error.result_summary.chars().take(50).collect();
            by_prefix.entry(prefix).or_default().push(error);
        }

        for (prefix, group) in by_prefix {
            if group.len() >= self.min_occurrences {
                patterns.push(ErrorPattern {
                    condition: format!("error contains '{prefix}'"),
                    examples: group.iter().map(|o| o.result_summary.clone()).collect(),
                    confidence: group.len() as f64 / errors.len() as f64,
                });
            }
        }

        patterns
    }
}

struct ErrorPattern {
    condition: String,
    examples: Vec<String>,
    confidence: f64,
}
