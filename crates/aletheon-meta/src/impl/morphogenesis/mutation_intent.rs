//! Mutation intent generation — how the agent decides what to change.
//!
//! Analyzes reflection entries and context to produce MutationIntents
//! that propose specific changes to the genome.

use aletheon_abi::MutationIntent;

/// Generate mutation intents from reflection and experience.
pub struct MutationIntentGenerator;

impl MutationIntentGenerator {
    pub fn new() -> Self { Self }

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
                reason: format!("Failures detected in recent context — increasing safety priority"),
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
                reason: "Successful patterns detected — reinforcing helpfulness priority".to_string(),
                reversible: true,
            });
        }

        intents
    }
}
