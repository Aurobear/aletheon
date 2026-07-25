//! Genome-derived behavior parameters.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lightweight genome config snapshot held by the runtime.
/// Extracted from GenomeMeta — does not hold the full genome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeConfig {
    /// Reasoning strategy name (e.g., "plan-then-execute", "react").
    pub reasoning_strategy: String,
    /// Confidence threshold below which the agent considers itself stuck.
    pub impasse_threshold: f64,
    /// What triggers reflection.
    pub reflection_trigger: String,
    /// Care weights by topic (e.g., "safety" -> 1.0).
    pub care_weights: HashMap<String, f64>,
    /// Current genome version string.
    pub genome_version: String,
}

impl Default for GenomeConfig {
    fn default() -> Self {
        Self {
            reasoning_strategy: "plan-then-execute".to_string(),
            impasse_threshold: 0.3,
            reflection_trigger: "task_complete".to_string(),
            care_weights: HashMap::new(),
            genome_version: "0.1.0".to_string(),
        }
    }
}

impl GenomeConfig {
    /// Extract from a GenomeMeta.
    pub fn from_genome_meta(meta: &metacog::GenomeMeta) -> Self {
        Self {
            reasoning_strategy: meta.reasoning.default_strategy.clone(),
            impasse_threshold: meta.reasoning.impasse_threshold,
            reflection_trigger: meta.reasoning.reflection_trigger.clone(),
            care_weights: meta.care_ext.weights.clone(),
            genome_version: meta.genome_version.clone(),
        }
    }

    /// Format care weights for injection into system prompt.
    pub fn care_weights_prompt(&self) -> String {
        if self.care_weights.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = self
            .care_weights
            .iter()
            .map(|(k, v)| format!("  {k}: {v:.2}"))
            .collect();
        parts.sort();
        format!("Current care priorities:\n{}", parts.join("\n"))
    }
}
