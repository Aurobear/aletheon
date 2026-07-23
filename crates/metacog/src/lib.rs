pub mod adapters;
pub mod evaluation;
pub mod evolution;
pub mod genome;
pub mod governance;
pub mod improvement;

// Compatibility re-exports — remove after migration window (Phase 1 complete).
pub mod hil_evidence_verifier {
    pub use crate::evaluation::hil_evidence::HILEvidenceVerifier;
}
pub mod outcome_verifier {
    pub use crate::evaluation::outcome::*;
}

pub use adapters::*;
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
