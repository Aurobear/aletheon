//! Extended Genome types for meta-runtime.
//!
//! Re-exports ABI Genome types and adds evaluator/morphogenesis-specific types
//! that the ABI doesn't define yet.

use serde::{Deserialize, Serialize};

// Re-export ABI Genome types for convenience
pub use aletheon_abi::genome::{
    Genome, Topology, SubsystemSpec, SubsystemType,
    IdentitySpec, BoundarySpec, BoundaryRuleSpec,
    CareSpec, CarePriority, MemorySpec, MutationSpec, LifecycleSpec,
};

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
