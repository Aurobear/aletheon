pub(crate) mod candidate;
pub(crate) mod candidate_bridge;
pub(crate) mod candidate_evaluator;
pub mod experiment;
pub mod experiment_store;
pub(crate) mod lineage;
pub(crate) mod migration;
pub mod model;
pub(crate) mod pipeline;
pub mod rollback;
pub(crate) mod sandbox_runner;

pub use candidate_bridge::CandidateBridge;
pub use lineage::LineageLink;
pub use model::{EvaluationResult, EvaluatorMetric, EvaluatorSpec};
pub use pipeline::{MorphogenesisPipeline, PipelineResult};
pub use rollback::RollbackManager;

// Compatibility re-export — mutation_intent moves to improvement/ in Task 4.
pub use crate::improvement::promotion::MutationIntentGenerator;
