//! Mutation intent generation — how the agent decides what to change.
//!
//! Analyzes reflection entries and context to produce MutationIntents
//! that propose specific changes to the genome.

use aletheon_abi::brain::{ReflectionEntry, ReflectionOutcome};
use aletheon_abi::MutationIntent;

/// Generate mutation intents from reflection and experience.
#[derive(Default)]
pub struct MutationIntentGenerator;

impl MutationIntentGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Generate mutation intents based on recent experience and reflection.
    ///
    /// Simple heuristic: scans the context string for keywords indicating
    /// problems or improvement areas, and produces targeted mutation intents.
    pub async fn generate(&self, context: &str) -> Vec<MutationIntent> {
        let mut intents = Vec::new();
        let lower = context.to_lowercase();

        // If failures detected, propose increasing safety weight
        if lower.contains("fail") || lower.contains("error") || lower.contains("fail") {
            intents.push(MutationIntent {
                target: "care.priorities".to_string(),
                change: serde_json::json!({
                    "topic": "safety",
                    "weight_delta": 0.05,
                    "action": "increase_weight"
                }),
                reason: "Failures detected in recent context — increasing safety priority".to_string(),
                reversible: true,
            });
        }

        // If slowness detected, propose reducing mutation frequency
        if lower.contains("slow") || lower.contains("timeout") || lower.contains("latency") {
            intents.push(MutationIntent {
                target: "mutation.config".to_string(),
                change: serde_json::json!({
                    "field": "summary_interval",
                    "action": "increase",
                    "delta": 5
                }),
                reason: "Performance issues detected — reducing mutation frequency".to_string(),
                reversible: true,
            });
        }

        // If success patterns detected, propose increasing helpfulness weight
        if lower.contains("success") || lower.contains("complete") || lower.contains("pass") {
            intents.push(MutationIntent {
                target: "care.priorities".to_string(),
                change: serde_json::json!({
                    "topic": "helpfulness",
                    "weight_delta": 0.02,
                    "action": "increase_weight"
                }),
                reason: "Successful patterns detected — reinforcing helpfulness priority"
                    .to_string(),
                reversible: true,
            });
        }

        intents
    }

    /// Generate mutation intents from structured reflection data.
    pub async fn from_reflections(&self, reflections: &[ReflectionEntry]) -> Vec<MutationIntent> {
        if reflections.is_empty() {
            return Vec::new();
        }
        let mut intents = Vec::new();
        let total = reflections.len() as f64;

        // High failure rate → increase safety weight
        let failures = reflections.iter()
            .filter(|r| matches!(r.outcome, ReflectionOutcome::Failure))
            .count() as f64;
        let failure_rate = failures / total;

        if failure_rate > 0.3 {
            intents.push(MutationIntent {
                target: "care.priorities".to_string(),
                change: serde_json::json!({
                    "action": "adjust_weight",
                    "topic": "safety",
                    "delta": (failure_rate * 0.1).min(0.2),
                }),
                reason: format!(
                    "Failure rate is {:.0}% across {} recent turns. Increasing safety care weight.",
                    failure_rate * 100.0, reflections.len()
                ),
                reversible: true,
            });
        }

        // Timeout/slow patterns → adjust mutation interval
        let has_timeout = reflections.iter()
            .filter(|r| matches!(r.outcome, ReflectionOutcome::Failure))
            .flat_map(|r| r.what_failed.clone())
            .any(|f| f.contains("timeout") || f.contains("slow") || f.contains("latency"));
        if has_timeout {
            intents.push(MutationIntent {
                target: "mutation.config".to_string(),
                change: serde_json::json!({
                    "action": "adjust_interval",
                    "delta": 5,
                }),
                reason: "Timeout/slow patterns detected. Increasing mutation interval.".to_string(),
                reversible: true,
            });
        }

        // High success rate → reinforce helpfulness
        let successes = reflections.iter()
            .filter(|r| matches!(r.outcome, ReflectionOutcome::Success))
            .count() as f64;
        if successes / total > 0.8 {
            intents.push(MutationIntent {
                target: "care.priorities".to_string(),
                change: serde_json::json!({
                    "action": "adjust_weight",
                    "topic": "helpfulness",
                    "delta": 0.02,
                }),
                reason: format!(
                    "Success rate is {:.0}%. Reinforcing helpfulness weight.",
                    (successes / total) * 100.0
                ),
                reversible: true,
            });
        }

        intents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::brain::{ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
    use chrono::Utc;

    fn make_entry(outcome: ReflectionOutcome, what_failed: Vec<String>) -> ReflectionEntry {
        ReflectionEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: "test task".to_string(),
            outcome,
            what_worked: vec![],
            what_failed,
            learned: vec![],
            behavior_changes: vec![],
            confidence: 0.5,
        }
    }

    #[tokio::test]
    async fn test_high_failure_triggers_safety() {
        let gen = MutationIntentGenerator::new();
        let reflections = vec![
            make_entry(ReflectionOutcome::Failure, vec!["error".to_string()]),
            make_entry(ReflectionOutcome::Failure, vec!["crash".to_string()]),
            make_entry(ReflectionOutcome::Success, vec![]),
        ];
        let intents = gen.from_reflections(&reflections).await;
        assert!(!intents.is_empty());
        assert!(intents.iter().any(|i| i.target == "care.priorities"));
    }

    #[tokio::test]
    async fn test_empty_reflections_no_intents() {
        let gen = MutationIntentGenerator::new();
        let intents = gen.from_reflections(&[]).await;
        assert!(intents.is_empty());
    }

    #[tokio::test]
    async fn test_high_success_reinforces_helpfulness() {
        let gen = MutationIntentGenerator::new();
        let reflections = (0..5).map(|_| make_entry(ReflectionOutcome::Success, vec![])).collect::<Vec<_>>();
        let intents = gen.from_reflections(&reflections).await;
        assert!(intents.iter().any(|i|
            i.change.get("topic").and_then(|v| v.as_str()) == Some("helpfulness")
        ));
    }
}
