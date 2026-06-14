//! Candidate runtime generation — building the next version of the agent.
//!
//! Applies mutation intents to a genome to produce RuntimeCandidates.

use anyhow::Result;
use aletheon_abi::{Genome, RuntimeCandidate, MutationIntent};

/// Generates candidate runtimes from genome mutations.
pub struct CandidateGenerator;

impl CandidateGenerator {
    pub fn new() -> Self { Self }

    /// Generate a candidate runtime from a genome and mutation intent.
    ///
    /// Clones the genome, applies the mutation described in the intent,
    /// and produces a RuntimeCandidate with the changes recorded.
    pub async fn generate(&self, genome: &Genome, intent: &MutationIntent) -> Result<RuntimeCandidate> {
        let mut candidate_genome = genome.clone();
        let mut changes = Vec::new();

        // Apply the mutation based on the target field
        match intent.target.as_str() {
            "care.priorities" => {
                if let Some(topic) = intent.change.get("topic").and_then(|v| v.as_str()) {
                    if let Some(delta) = intent.change.get("weight_delta").and_then(|v| v.as_f64()) {
                        // Find and adjust the care priority weight
                        if let Some(priority) = candidate_genome.care.priorities.iter_mut()
                            .find(|p| p.topic == topic)
                        {
                            let old_weight = priority.weight;
                            priority.weight = (old_weight + delta).clamp(0.0, 1.0);
                            changes.push(format!(
                                "care.{}.weight: {} -> {} (reason: {})",
                                topic, old_weight, priority.weight, intent.reason
                            ));
                        } else {
                            // Topic doesn't exist yet — add it
                            candidate_genome.care.priorities.push(
                                crate::core::types::CarePriority {
                                    topic: topic.to_string(),
                                    weight: delta.clamp(0.0, 1.0),
                                }
                            );
                            changes.push(format!(
                                "care.{}.weight: new = {} (reason: {})",
                                topic, delta, intent.reason
                            ));
                        }
                    }
                }
            }
            "boundary.rules" => {
                // For boundary rule changes, record the intent but don't auto-apply
                // (boundary rule changes need SelfField approval in real usage)
                changes.push(format!(
                    "boundary.rules: proposed change — {} (reason: {})",
                    serde_json::to_string(&intent.change).unwrap_or_default(),
                    intent.reason
                ));
            }
            "mutation.config" => {
                // Mutation config changes are recorded but not auto-applied
                changes.push(format!(
                    "mutation.config: proposed change — {} (reason: {})",
                    serde_json::to_string(&intent.change).unwrap_or_default(),
                    intent.reason
                ));
            }
            _ => {
                changes.push(format!(
                    "unknown target '{}': {} (reason: {})",
                    intent.target,
                    serde_json::to_string(&intent.change).unwrap_or_default(),
                    intent.reason
                ));
            }
        }

        Ok(RuntimeCandidate {
            id: uuid::Uuid::new_v4(),
            genome: candidate_genome,
            changes,
            generated_at: chrono::Utc::now(),
        })
    }
}
