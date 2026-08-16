//! Versioned contracts for evidence-backed task evaluation.

mod contract;
mod evidence;
mod receipt;
mod settings;

pub use evidence::{EvaluationEvidenceSnapshot, EvaluationSnapshotId};

pub use contract::{
    EvaluationContractId, EvaluationMode, EvaluationSubject, EvaluationThresholds, EvidenceRef,
    RequiredEvidence, RequiredGate, TaskEvaluationContract, TaskKind,
};
pub use receipt::{
    EvaluationDecision, EvaluationExecutionContext, EvaluationReceipt, EvaluationReceiptId,
    EvaluationReceiptRef,
};
pub use settings::EvaluationSettings;

pub const EVALUATION_SCHEMA_V1: u16 = 1;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EvaluationContractError {
    #[error("unsupported evaluation schema version {0}")]
    UnsupportedSchema(u16),
    #[error("evaluation field is empty: {0}")]
    EmptyField(&'static str),
    #[error("evaluation threshold is out of range: {0}")]
    InvalidThreshold(&'static str),
    #[error("evaluation contract has no required gates")]
    MissingRequiredGates,
    #[error("evaluation evidence contains duplicate id: {0}")]
    DuplicateEvidence(String),
    #[error("evaluation evidence digest mismatch: {0}")]
    EvidenceDigestMismatch(String),
    #[error("evaluation snapshot digest mismatch")]
    SnapshotDigestMismatch,
    #[error("evaluation identity mismatch: {0}")]
    IdentityMismatch(&'static str),
    #[error("evaluation decision is incompatible with contract mode")]
    DecisionModeMismatch,
    #[error("evaluation failed-gate summary does not match report")]
    FailedGateMismatch,
    #[error("evaluation serialization failed: {0}")]
    Serialization(String),
}
