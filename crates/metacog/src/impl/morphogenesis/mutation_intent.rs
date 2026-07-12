//! Mutation intent generation — how the agent decides what to change.
//!
//! Analyzes reflection entries and context to produce MutationIntents
//! that propose specific changes to the genome.

use std::collections::HashMap;

use fabric::cognit::{ReflectionEntry, ReflectionOutcome};
use fabric::MutationIntent;

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
                reason: "Failures detected in recent context — increasing safety priority"
                    .to_string(),
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
    ///
    /// Analyzes failure rates, repeated tool failures, timeout patterns,
    /// success-rate trends, and `what_worked`/`what_failed`/`learned` fields.
    pub async fn from_reflections(&self, reflections: &[ReflectionEntry]) -> Vec<MutationIntent> {
        if reflections.is_empty() {
            return Vec::new();
        }
        let mut intents = Vec::new();
        let total = reflections.len() as f64;

        // --- 1. High failure rate → increase safety weight ---
        let failures = reflections
            .iter()
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
                    failure_rate * 100.0,
                    reflections.len()
                ),
                reversible: true,
            });
        }

        // --- 2. Repeated tool failures → targeted tool mutation ---
        // Count how many times each tool/pattern appears in what_failed.
        let mut tool_failures: HashMap<String, usize> = HashMap::new();
        for entry in reflections.iter() {
            for failure in &entry.what_failed {
                // Normalize: lowercase, trim whitespace
                let normalized = failure.trim().to_lowercase();
                if !normalized.is_empty() {
                    *tool_failures.entry(normalized).or_insert(0) += 1;
                }
            }
        }
        // A tool failing 2+ times is "repeated".
        let repeated: Vec<(&String, &usize)> = tool_failures
            .iter()
            .filter(|(_, &count)| count >= 2)
            .collect();
        if !repeated.is_empty() {
            let worst = repeated.iter().max_by_key(|(_, c)| *c).unwrap();
            intents.push(MutationIntent {
                target: "tool.config".to_string(),
                change: serde_json::json!({
                    "action": "targeted_mutation",
                    "failing_pattern": worst.0,
                    "failure_count": worst.1,
                }),
                reason: format!(
                    "Repeated failure pattern \"{}\" seen {} times. Proposing targeted mutation.",
                    worst.0, worst.1
                ),
                reversible: true,
            });
        }

        // --- 3. Timeout/slow patterns → efficiency mutation ---
        let timeout_keywords = [
            "timeout",
            "slow",
            "latency",
            "deadline exceeded",
            "timed out",
        ];
        let has_timeout = reflections
            .iter()
            .flat_map(|r| r.what_failed.iter().chain(r.learned.iter()))
            .any(|text| {
                let lower = text.to_lowercase();
                timeout_keywords.iter().any(|kw| lower.contains(kw))
            });
        if has_timeout {
            intents.push(MutationIntent {
                target: "mutation.config".to_string(),
                change: serde_json::json!({
                    "action": "adjust_interval",
                    "delta": 5,
                }),
                reason: "Timeout/slow patterns detected in what_failed/learned. Increasing mutation interval.".to_string(),
                reversible: true,
            });
            // Also propose an efficiency care bump
            intents.push(MutationIntent {
                target: "care.priorities".to_string(),
                change: serde_json::json!({
                    "action": "adjust_weight",
                    "topic": "efficiency",
                    "delta": 0.05,
                }),
                reason: "Repeated timeout patterns — increasing efficiency priority.".to_string(),
                reversible: true,
            });
        }

        // --- 4. Success-rate trend (improving vs declining) ---
        // Split into first-half and second-half; compare success rates.
        let mid = reflections.len() / 2;
        if reflections.len() >= 4 {
            let early = &reflections[..mid];
            let recent = &reflections[mid..];
            let early_rate = success_rate(early);
            let recent_rate = success_rate(recent);

            if early_rate > 0.5 && recent_rate < early_rate - 0.15 {
                // Declining: propose correction
                intents.push(MutationIntent {
                    target: "care.priorities".to_string(),
                    change: serde_json::json!({
                        "action": "adjust_weight",
                        "topic": "safety",
                        "delta": 0.03,
                    }),
                    reason: format!(
                        "Success rate declining ({:.0}% -> {:.0}%). Correcting safety weight.",
                        early_rate * 100.0,
                        recent_rate * 100.0
                    ),
                    reversible: true,
                });
            } else if recent_rate > early_rate + 0.1 && recent_rate > 0.7 {
                // Improving: reinforce current strategy via helpfulness
                intents.push(MutationIntent {
                    target: "care.priorities".to_string(),
                    change: serde_json::json!({
                        "action": "adjust_weight",
                        "topic": "helpfulness",
                        "delta": 0.02,
                    }),
                    reason: format!(
                        "Success rate improving ({:.0}% -> {:.0}%). Reinforcing helpfulness.",
                        early_rate * 100.0,
                        recent_rate * 100.0
                    ),
                    reversible: true,
                });
            }
        }

        // --- 5. High success rate → reinforce helpfulness (existing) ---
        let successes = reflections
            .iter()
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

        // --- 6. Use learned entries as mutation hints ---
        let learned_count: usize = reflections.iter().map(|r| r.learned.len()).sum();
        if learned_count >= 3 {
            let sample = reflections
                .iter()
                .flat_map(|r| r.learned.iter())
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            intents.push(MutationIntent {
                target: "genome.adaptation".to_string(),
                change: serde_json::json!({
                    "action": "incorporate_learnings",
                    "learning_count": learned_count,
                    "samples": sample,
                }),
                reason: format!(
                    "{} learning entries collected. Proposing adaptation.",
                    learned_count
                ),
                reversible: true,
            });
        }

        intents
    }
}

/// Compute the fraction of entries with `Success` outcome.
fn success_rate(entries: &[ReflectionEntry]) -> f64 {
    if entries.is_empty() {
        return 0.0;
    }
    let successes = entries
        .iter()
        .filter(|r| matches!(r.outcome, ReflectionOutcome::Success))
        .count();
    successes as f64 / entries.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::cognit::{ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
    use fabric::wall_to_datetime;
    use fabric::Clock;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> TestClock {
        TestClock::default()
    }

    fn make_entry(outcome: ReflectionOutcome, what_failed: Vec<String>) -> ReflectionEntry {
        make_entry_full(outcome, what_failed, vec![], vec![])
    }

    fn make_entry_full(
        outcome: ReflectionOutcome,
        what_failed: Vec<String>,
        what_worked: Vec<String>,
        learned: Vec<String>,
    ) -> ReflectionEntry {
        ReflectionEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: wall_to_datetime(test_clock().wall_now()),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: "test task".to_string(),
            outcome,
            what_worked,
            what_failed,
            learned,
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
        let reflections = (0..5)
            .map(|_| make_entry(ReflectionOutcome::Success, vec![]))
            .collect::<Vec<_>>();
        let intents = gen.from_reflections(&reflections).await;
        assert!(intents
            .iter()
            .any(|i| i.change.get("topic").and_then(|v| v.as_str()) == Some("helpfulness")));
    }

    #[tokio::test]
    async fn test_repeated_tool_failure_generates_targeted_mutation() {
        let gen = MutationIntentGenerator::new();
        // Same failure pattern "tool x crashed" appears 3 times
        let reflections = vec![
            make_entry(
                ReflectionOutcome::Failure,
                vec!["tool x crashed".to_string()],
            ),
            make_entry(
                ReflectionOutcome::Failure,
                vec!["tool x crashed".to_string()],
            ),
            make_entry(
                ReflectionOutcome::Failure,
                vec!["tool x crashed".to_string()],
            ),
            make_entry(ReflectionOutcome::Success, vec![]),
        ];
        let intents = gen.from_reflections(&reflections).await;
        let targeted = intents.iter().find(|i| i.target == "tool.config");
        assert!(targeted.is_some(), "expected tool.config mutation intent");
        let change = &targeted.unwrap().change;
        assert_eq!(change["action"], "targeted_mutation");
        assert_eq!(change["failure_count"], 3);
    }

    #[tokio::test]
    async fn test_timeout_pattern_generates_efficiency_mutation() {
        let gen = MutationIntentGenerator::new();
        let reflections = vec![
            make_entry(
                ReflectionOutcome::Failure,
                vec!["timeout waiting for response".to_string()],
            ),
            make_entry(ReflectionOutcome::Success, vec![]),
        ];
        let intents = gen.from_reflections(&reflections).await;
        // Should have mutation.config (interval) AND care.priorities (efficiency)
        assert!(intents.iter().any(|i| i.target == "mutation.config"));
        assert!(intents.iter().any(|i| {
            i.target == "care.priorities"
                && i.change.get("topic").and_then(|v| v.as_str()) == Some("efficiency")
        }));
    }

    #[tokio::test]
    async fn test_declining_success_rate_triggers_correction() {
        let gen = MutationIntentGenerator::new();
        // 8 entries: early 4 are all success, recent 4 are mostly failure
        let reflections: Vec<ReflectionEntry> = (0..4)
            .map(|_| make_entry(ReflectionOutcome::Success, vec![]))
            .chain((0..4).map(|_| make_entry(ReflectionOutcome::Failure, vec!["err".to_string()])))
            .collect();
        let intents = gen.from_reflections(&reflections).await;
        // Should correct safety weight
        let correction = intents.iter().find(|i| {
            i.target == "care.priorities"
                && i.change.get("topic").and_then(|v| v.as_str()) == Some("safety")
                && i.reason.contains("declining")
        });
        assert!(correction.is_some(), "expected declining correction intent");
    }

    #[tokio::test]
    async fn test_improving_success_rate_reinforces() {
        let gen = MutationIntentGenerator::new();
        // 8 entries: early 4 are mostly failure, recent 4 are all success
        let reflections: Vec<ReflectionEntry> = (0..4)
            .map(|_| make_entry(ReflectionOutcome::Failure, vec!["err".to_string()]))
            .chain((0..4).map(|_| make_entry(ReflectionOutcome::Success, vec![])))
            .collect();
        let intents = gen.from_reflections(&reflections).await;
        let reinforce = intents.iter().find(|i| {
            i.target == "care.priorities"
                && i.change.get("topic").and_then(|v| v.as_str()) == Some("helpfulness")
                && i.reason.contains("improving")
        });
        assert!(
            reinforce.is_some(),
            "expected improving reinforcement intent"
        );
    }

    #[tokio::test]
    async fn test_learned_entries_trigger_adaptation() {
        let gen = MutationIntentGenerator::new();
        let reflections = vec![
            make_entry_full(
                ReflectionOutcome::Success,
                vec![],
                vec!["fast tool".to_string()],
                vec!["prefer tool A over tool B".to_string()],
            ),
            make_entry_full(
                ReflectionOutcome::Success,
                vec![],
                vec!["good response".to_string()],
                vec!["shorter prompts work better".to_string()],
            ),
            make_entry_full(
                ReflectionOutcome::Success,
                vec![],
                vec![],
                vec!["batch operations are faster".to_string()],
            ),
        ];
        let intents = gen.from_reflections(&reflections).await;
        let adaptation = intents.iter().find(|i| i.target == "genome.adaptation");
        assert!(adaptation.is_some(), "expected genome.adaptation intent");
        let change = &adaptation.unwrap().change;
        assert_eq!(change["action"], "incorporate_learnings");
        assert_eq!(change["learning_count"], 3);
    }

    #[test]
    fn test_success_rate_helper() {
        let entries = vec![
            make_entry(ReflectionOutcome::Success, vec![]),
            make_entry(ReflectionOutcome::Success, vec![]),
            make_entry(ReflectionOutcome::Failure, vec![]),
        ];
        let rate = success_rate(&entries);
        assert!((rate - 2.0 / 3.0).abs() < 1e-9);
    }
}
