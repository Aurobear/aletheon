//! Bridge module — connects impl to core traits.
//!
//! Provides adapters that bridge meta-runtime traits to their concrete implementations.

pub mod genome_bridge;
pub mod candidate_bridge;

pub use candidate_bridge::CandidateBridge;
pub use genome_bridge::GenomeBridge;

// Re-export commonly used impl types for convenience
pub use crate::r#impl::genome::loader::GenomeLoader;
pub use crate::r#impl::meta_runtime::evaluator::Evaluator;
pub use crate::r#impl::meta_runtime::lineage::LineageTracker;
pub use crate::r#impl::morphogenesis::candidate::CandidateGenerator;
pub use crate::r#impl::morphogenesis::pipeline::MorphogenesisPipeline;
