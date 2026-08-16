mod adapters;
pub mod coding_evidence_adapter;
pub mod coding_rubric;
pub mod evaluation;
pub mod evidence;
pub mod evolution;
pub mod experience;
pub mod genome;
pub mod governance;
pub mod improvement;
pub mod problem;
pub mod reflection;

pub use adapters::{
    EvolutionAction, EvolutionDecision, MetaCognition, MetaCognitionThresholds, SystemState,
};
pub use evolution::{CandidateBridge, EvaluationResult, EvaluatorMetric, EvaluatorSpec};
pub use genome::{
    CareExt, ChangeType, EvolutionConfig, GenomeBridge, GenomeChange, GenomeMeta, GenomeRule,
    IdentityExt, ReasoningConfig,
};
pub use governance::{
    ApplyMutation, DefaultMetaRuntime, DefaultMetacogService, GovernedMutationEvidence,
    MetacogError, MetacogService, MetacogStatus, MutationLifecycle, MutationOperation,
    MutationReceipt, MutationStatus, RetryDisposition, RollbackMutation, VerificationDecision,
    VerificationReceipt, VerifyMutation,
};
pub use improvement::{GenomePatch, MorphogenesisCandidate, PatchOperation};
