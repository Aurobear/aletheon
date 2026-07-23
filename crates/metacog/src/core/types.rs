//! Morphogenesis and evaluation types — candidate proposals, patches, and evaluation results.
//!
//! Genome data model types have moved to `crate::genome::model`.

use serde::{Deserialize, Serialize};

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
