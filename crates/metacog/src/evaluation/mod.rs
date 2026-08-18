pub mod coding_v2;
pub mod conscious_field_metrics;
pub mod engine;
pub mod hil_evidence;
pub mod hil_evidence_contract;
pub mod model;
pub mod outcome;
pub mod receipt_store;
pub mod rubric;
pub mod settlement_policy;

pub use engine::{DeterministicEvaluator, EvaluationError};
pub use model::{DimensionScore, DimensionValue, EvaluationReport, GateResult, RubricId};
pub use receipt_store::EvaluationReceiptStore;
pub use rubric::{coding_v2_rubric, Rubric, RubricDimension, RubricGate};
pub use settlement_policy::EvaluationSettlementPolicy;

pub use coding_v2::{score_coding_v2, CodingV2Score, CodingV2ScoreError};
pub use conscious_field_metrics::{
    quantize, FieldMetricHistory, FieldMetricIndicators, FieldMetricSnapshot,
    MAX_FIELD_METRIC_HISTORY, QUIET_CONVERGENCE_WINDOW,
};
