//! Morphogenesis types — candidate proposals and patches.
//!
//! Genome data model has moved to `crate::genome::model`.
//! Evaluation types have moved to `crate::evolution::model`.

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
