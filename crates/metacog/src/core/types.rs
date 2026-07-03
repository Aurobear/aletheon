//! Extended Genome types for meta-runtime.
//!
//! Re-exports ABI Genome types and adds evaluator/morphogenesis-specific types
//! that the ABI doesn't define yet.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export ABI Genome types for convenience
pub use base::genome::{
    BoundaryRuleSpec, BoundarySpec, CarePriority, CareSpec, Genome, IdentitySpec, LifecycleSpec,
    MemorySpec, MutationSpec, SubsystemSpec, SubsystemType, Topology,
};

/// Extended genome metadata — version tracking, lineage, and evolution parameters.
///
/// Wraps the ABI Genome with meta-level fields needed by the evolution pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeMeta {
    /// ABI-level genome.
    #[serde(flatten)]
    pub genome: Genome,
    /// Version of this genome (semantic version string).
    pub genome_version: String,
    /// Lineage ID — identifies the ancestry chain.
    pub lineage_id: String,
    /// Parent genome version (None for the initial genome).
    pub parent_version: Option<String>,
    /// Extended identity fields.
    pub identity_ext: IdentityExt,
    /// Care extension: numeric weights and boundary rules.
    pub care_ext: CareExt,
    /// Reasoning configuration.
    pub reasoning: ReasoningConfig,
    /// Evolution configuration.
    pub evolution: EvolutionConfig,
}

/// Extended identity — core values and purpose beyond the ABI IdentitySpec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityExt {
    /// Core values the agent holds.
    pub core_values: Vec<String>,
    /// The agent's fundamental purpose.
    pub fundamental_purpose: String,
}

/// Care extension — weight map and boundary rules for the evolution pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareExt {
    /// Numeric weights by topic (e.g., "safety" -> 1.0).
    pub weights: HashMap<String, f64>,
    /// Boundary rules with immutability flags.
    pub boundary_rules: Vec<GenomeRule>,
}

/// A genome rule — a boundary rule with pattern matching and immutability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeRule {
    /// Pattern this rule matches (glob or regex).
    pub pattern: String,
    /// Verdict: "allow", "deny", "sandbox_first", etc.
    pub verdict: String,
    /// Whether this rule can never be changed by evolution.
    pub immutable: bool,
    /// Where this rule came from (e.g., "initial", "learned", "user").
    pub origin: String,
}

/// Reasoning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    /// Default reasoning strategy (e.g., "plan-then-execute", "react").
    pub default_strategy: String,
    /// What triggers reflection (e.g., "task_complete", "impasse", "always").
    pub reflection_trigger: String,
    /// Confidence threshold below which the agent considers itself stuck.
    pub impasse_threshold: f64,
}

/// Evolution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionConfig {
    /// Whether the agent can auto-evolve without user approval.
    pub auto_evolve: bool,
    /// How often (in reflections) to run the summarizer.
    pub summary_interval: usize,
    /// Maximum weight adjustment per evolution step.
    pub max_adjustment: f64,
    /// Minimum safety weight that must always be maintained.
    pub safety_floor: f64,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            auto_evolve: false,
            summary_interval: 10,
            max_adjustment: 0.1,
            safety_floor: 0.8,
        }
    }
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            default_strategy: "plan-then-execute".to_string(),
            reflection_trigger: "task_complete".to_string(),
            impasse_threshold: 0.3,
        }
    }
}

/// A change between two genomes — produced by GenomeLoader::diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeChange {
    /// What changed (e.g., "care.weights.safety", "boundary.rules\[0\].verdict").
    pub path: String,
    /// Type of change.
    pub change_type: ChangeType,
    /// Old value (None if added).
    pub old_value: Option<serde_json::Value>,
    /// New value (None if removed).
    pub new_value: Option<serde_json::Value>,
}

/// Type of genome change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
}

/// Evaluator specification — not in ABI yet, defined here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorSpec {
    pub metrics: Vec<EvaluatorMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorMetric {
    pub name: String,
    pub weight: f64,
}

/// Morphogenesis candidate — a proposed change to the genome.
#[derive(Debug, Clone)]
pub struct MorphogenesisCandidate {
    pub id: String,
    pub description: String,
    pub genome_patch: GenomePatch,
    pub reason: String,
}

/// A patch to apply to the genome.
#[derive(Debug, Clone)]
pub struct GenomePatch {
    /// Target path, e.g., "boundary.rules", "care.priorities"
    pub target: String,
    pub operation: PatchOperation,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum PatchOperation {
    Add,
    Remove,
    Replace,
    Modify,
}

/// Result of evaluating a candidate genome.
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub passed: bool,
    pub reasons: Vec<String>,
}
