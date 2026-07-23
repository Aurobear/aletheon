pub mod bridge;
pub mod core;
pub mod evolution;
pub mod genome;
pub mod governance;
pub mod hil_evidence_verifier;
#[path = "impl/mod.rs"]
mod r#impl;
pub mod outcome_verifier;

pub use evolution::{EvaluationResult, EvaluatorMetric, EvaluatorSpec};
pub use genome::{
    CareExt, ChangeType, EvolutionConfig, GenomeChange, GenomeMeta, GenomeRule, IdentityExt,
    ReasoningConfig,
};
pub use governance::{
    ApplyMutation, DefaultMetaRuntime, DefaultMetacogService, GovernedMutationEvidence,
    MetacogError, MetacogService, MutationLifecycle, MutationOperation, MutationReceipt,
    MutationStatus, MetacogStatus, RetryDisposition, RollbackMutation, VerificationDecision,
    VerificationReceipt, VerifyMutation,
};
