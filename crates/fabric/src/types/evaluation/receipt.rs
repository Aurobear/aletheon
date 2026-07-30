use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::metacognition_evaluation::EvaluationReport;
use crate::OperationId;

use super::{
    EvaluationContractError, EvaluationContractId, EvaluationEvidenceSnapshot, EvaluationMode,
    EvaluationSubject, TaskEvaluationContract, EVALUATION_SCHEMA_V1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationReceiptId(pub Uuid);

impl EvaluationReceiptId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EvaluationReceiptId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationDecision {
    ObservedPass,
    ObservedFail,
    Accepted,
    Rejected,
    Indeterminate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvaluationExecutionContext {
    /// Host-owned runtime adapter identity; never inferred from model prose.
    pub runtime_id: String,
    /// Effective Agent profile resolved by the host for this turn.
    pub agent_profile: String,
    /// Effective provider/model route reported by typed host state.
    pub effective_model_id: String,
    /// User-facing model display name reported by typed host state.
    pub model_display_name: String,
    /// Digest of the authenticated workspace policy used for scope scoring.
    pub workspace_boundary_sha256: String,
    /// Digest of the ordered validation receipts selected for scoring.
    pub verification_selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationReceipt {
    pub schema_version: u16,
    pub receipt_id: EvaluationReceiptId,
    pub contract_id: EvaluationContractId,
    pub evaluation_operation_id: OperationId,
    pub subject: EvaluationSubject,
    pub evidence_snapshot_sha256: String,
    pub report: EvaluationReport,
    pub decision: EvaluationDecision,
    pub failed_gates: Vec<String>,
    pub evaluator: String,
    /// Durable host correlation needed to compare runtime capability without
    /// trusting model self-identification or recomputing workspace selection.
    #[serde(default)]
    pub execution: EvaluationExecutionContext,
    pub created_at_ms: i64,
}

impl EvaluationReceipt {
    pub fn validate(
        &self,
        contract: &TaskEvaluationContract,
        snapshot: &EvaluationEvidenceSnapshot,
    ) -> Result<(), EvaluationContractError> {
        if self.schema_version != EVALUATION_SCHEMA_V1 {
            return Err(EvaluationContractError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        if self.contract_id != contract.contract_id || snapshot.contract_id != contract.contract_id
        {
            return Err(EvaluationContractError::IdentityMismatch("contract"));
        }
        if self.subject != contract.subject {
            return Err(EvaluationContractError::IdentityMismatch("subject"));
        }
        if self.evidence_snapshot_sha256 != snapshot.sha256 {
            return Err(EvaluationContractError::IdentityMismatch(
                "evidence_snapshot",
            ));
        }
        let mode_matches = matches!(
            (contract.mode, self.decision),
            (
                EvaluationMode::Shadow,
                EvaluationDecision::ObservedPass
                    | EvaluationDecision::ObservedFail
                    | EvaluationDecision::Indeterminate
            ) | (
                EvaluationMode::Enforce,
                EvaluationDecision::Accepted
                    | EvaluationDecision::Rejected
                    | EvaluationDecision::Indeterminate
            )
        );
        if !mode_matches {
            return Err(EvaluationContractError::DecisionModeMismatch);
        }
        let mut report_failures = self
            .report
            .gates
            .iter()
            .filter(|gate| !gate.passed)
            .map(|gate| gate.name.clone())
            .collect::<Vec<_>>();
        report_failures.sort();
        let mut declared = self.failed_gates.clone();
        declared.sort();
        if report_failures != declared {
            return Err(EvaluationContractError::FailedGateMismatch);
        }
        if self.evaluator.trim().is_empty() {
            return Err(EvaluationContractError::EmptyField("evaluator"));
        }
        Ok(())
    }

    pub fn reference(&self) -> EvaluationReceiptRef {
        let (subject_kind, subject_id) = self.subject.kind_and_id();
        EvaluationReceiptRef {
            schema_version: self.schema_version,
            receipt_id: self.receipt_id,
            contract_id: self.contract_id,
            subject_kind: subject_kind.into(),
            subject_id,
            decision: self.decision,
            weighted_total_millis: self.report.weighted_total_millis,
            evidence_coverage_millis: self.report.evidence_coverage_millis,
            confidence_millis: self.report.confidence_millis,
            failed_gates: self.failed_gates.clone(),
            created_at_ms: self.created_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvaluationReceiptRef {
    pub schema_version: u16,
    #[schemars(with = "uuid::Uuid")]
    pub receipt_id: EvaluationReceiptId,
    #[schemars(with = "uuid::Uuid")]
    pub contract_id: EvaluationContractId,
    pub subject_kind: String,
    pub subject_id: String,
    pub decision: EvaluationDecision,
    pub weighted_total_millis: Option<u32>,
    pub evidence_coverage_millis: u16,
    pub confidence_millis: u16,
    pub failed_gates: Vec<String>,
    pub created_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::metacognition_evaluation::{GateResult, RubricId};

    #[test]
    fn shadow_contract_rejects_enforce_decision() {
        let mut contract = super::super::contract::tests::fixture(EvaluationMode::Shadow);
        let snapshot = EvaluationEvidenceSnapshot::new(contract.contract_id, vec![], 1).unwrap();
        let receipt = EvaluationReceipt {
            schema_version: EVALUATION_SCHEMA_V1,
            receipt_id: EvaluationReceiptId::new(),
            contract_id: contract.contract_id,
            evaluation_operation_id: OperationId::new(),
            subject: contract.subject.clone(),
            evidence_snapshot_sha256: snapshot.sha256.clone(),
            report: EvaluationReport {
                rubric: RubricId("coding-v2".into()),
                rubric_version: 2,
                dimensions: vec![],
                gates: vec![GateResult {
                    name: "required_verification_passed".into(),
                    passed: true,
                    evidence: vec![],
                }],
                weighted_total_millis: None,
                evidence_coverage_millis: 0,
                confidence_millis: 0,
                eligible: false,
            },
            decision: EvaluationDecision::Accepted,
            failed_gates: vec![],
            evaluator: "test".into(),
            execution: EvaluationExecutionContext::default(),
            created_at_ms: 1,
        };
        assert_eq!(
            receipt.validate(&contract, &snapshot),
            Err(EvaluationContractError::DecisionModeMismatch)
        );
        contract.mode = EvaluationMode::Enforce;
        assert!(receipt.validate(&contract, &snapshot).is_ok());
    }
}
