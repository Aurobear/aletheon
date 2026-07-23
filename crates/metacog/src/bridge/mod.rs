//! Bridge module — connects impl to core traits.
//!
//! Provides adapters that bridge meta-runtime traits to their concrete implementations.

pub mod candidate_bridge;
pub mod genome_bridge;

pub use candidate_bridge::CandidateBridge;
pub use genome_bridge::GenomeBridge;

// Re-export commonly used impl types for convenience
pub use crate::genome::loader::GenomeLoader;
pub use crate::evolution::candidate_evaluator::Evaluator;
pub use crate::evolution::candidate::CandidateGenerator;
pub use crate::evolution::lineage::LineageTracker;
pub use crate::evolution::pipeline::MorphogenesisPipeline;
