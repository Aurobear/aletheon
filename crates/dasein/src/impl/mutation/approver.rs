//! Subscribes to EvolutionTriggeredEvent, validates mutation intents via SelfField.
//!
//! Uses simple heuristics to generate mutation intents from evolution context,
//! then validates each against boundary rules and identity continuity.

use crate::core::mutation::MutationLayer;
use anyhow::Result;
use fabric::evolution::EvolutionTriggeredPayload;
use fabric::self_field::{MutationIntent, Verdict};
use std::sync::Arc;

/// Validates evolution triggers and generates approved mutation intents.
pub struct MutationApprover {
    mutation_layer: Arc<MutationLayer>,
    max_magnitude: f64,
}

impl MutationApprover {
    pub fn new(mutation_layer: Arc<MutationLayer>) -> Self {
        Self {
            mutation_layer,
            max_magnitude: 0.3, // Conservative default
        }
    }

    pub fn with_max_magnitude(mut self, max_magnitude: f64) -> Self {
        self.max_magnitude = max_magnitude;
        self
    }

    /// Process an evolution trigger. Returns approved mutation intents.
    pub fn handle(&self, trigger: &EvolutionTriggeredPayload) -> Result<Vec<MutationIntent>> {
        // 1. Generate mutation intents via simple heuristic
        let intents = self.generate_intents(trigger);

        // 2. Validate each intent through MutationLayer
        let mut approved = Vec::new();
        for intent in intents {
            let verdict = self.mutation_layer.review(&intent);
            match verdict {
                Verdict::Allow => {
                    approved.push(intent);
                }
                _ => {
                    tracing::info!(
                        "MutationIntent rejected by SelfField: target={}, reason={}",
                        intent.target,
                        intent.reason
                    );
                }
            }
        }

        Ok(approved)
    }

    /// Generate mutation intents using simple heuristics.
    ///
    /// - If trigger_reason is "consecutive_failures", generate intent to increase safety_weight
    /// - If trigger_reason is "confidence_drop", generate intent to reduce mutation frequency
    fn generate_intents(&self, trigger: &EvolutionTriggeredPayload) -> Vec<MutationIntent> {
        let mut intents = Vec::new();

        match trigger.trigger_reason.as_str() {
            "consecutive_failures" => {
                // Increase safety weight to be more cautious
                intents.push(MutationIntent {
                    target: "care.priorities".to_string(),
                    change: serde_json::json!({
                        "field": "safety_weight",
                        "delta": self.max_magnitude
                    }),
                    reason: format!(
                        "Increasing safety_weight due to {} consecutive failures",
                        trigger.recent_reflections.len()
                    ),
                    reversible: true,
                });
            }
            "confidence_drop" => {
                // Reduce mutation frequency to allow more observation time
                intents.push(MutationIntent {
                    target: "mutation.config".to_string(),
                    change: serde_json::json!({
                        "field": "mutation_frequency",
                        "delta": -self.max_magnitude
                    }),
                    reason: "Reducing mutation frequency due to confidence drop".to_string(),
                    reversible: true,
                });
            }
            other => {
                tracing::debug!("No heuristic rule for trigger reason '{}', skipping", other);
            }
        }

        intents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use uuid::Uuid;

    fn make_trigger(reason: &str) -> EvolutionTriggeredPayload {
        EvolutionTriggeredPayload {
            trigger_reason: reason.to_string(),
            recent_reflections: vec![Uuid::new_v4(), Uuid::new_v4()],
            current_rules_snapshot: vec![],
        }
    }

    #[test]
    fn consecutive_failures_generates_safety_intent() {
        let layer = Arc::new(MutationLayer::new());
        let approver = MutationApprover::new(layer);
        let trigger = make_trigger("consecutive_failures");

        let approved = approver.handle(&trigger).unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].target, "care.priorities");
        assert!(approved[0].reversible);
    }

    #[test]
    fn confidence_drop_generates_frequency_intent() {
        let layer = Arc::new(MutationLayer::new());
        let approver = MutationApprover::new(layer);
        let trigger = make_trigger("confidence_drop");

        let approved = approver.handle(&trigger).unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].target, "mutation.config");
        assert!(approved[0].reversible);
    }

    #[test]
    fn unknown_trigger_generates_no_intents() {
        let layer = Arc::new(MutationLayer::new());
        let approver = MutationApprover::new(layer);
        let trigger = make_trigger("unknown_reason");

        let approved = approver.handle(&trigger).unwrap();
        assert!(approved.is_empty());
    }

    #[test]
    fn max_magnitude_applied() {
        let layer = Arc::new(MutationLayer::new());
        let approver = MutationApprover::new(layer).with_max_magnitude(0.5);
        let trigger = make_trigger("consecutive_failures");

        let approved = approver.handle(&trigger).unwrap();
        assert_eq!(approved.len(), 1);
        let delta = approved[0].change["delta"].as_f64().unwrap();
        assert!((delta - 0.5).abs() < f64::EPSILON);
    }
}
