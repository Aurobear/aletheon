use ::contracts::types::metacognition_evaluation::RubricId;
use ::contracts::types::metacognition_evidence::EvidenceKind;
use ::contracts::{
    EvaluationContractId, EvaluationSubject, EvidenceRef, RequiredEvidence, RequiredGate,
    TaskEvaluationContract, TaskKind, TurnRequest, EVALUATION_SCHEMA_V1,
};
use chrono::Utc;

use crate::config::EvaluationSettings;

use super::EvaluationApplicationError;

pub trait TaskEvaluationContractIssuer: Send + Sync {
    fn issue(
        &self,
        request: &TurnRequest,
    ) -> Result<Option<TaskEvaluationContract>, EvaluationApplicationError>;
}

#[derive(Debug, Clone)]
pub struct DefaultTaskEvaluationContractIssuer {
    settings: EvaluationSettings,
}

impl DefaultTaskEvaluationContractIssuer {
    pub fn new(settings: EvaluationSettings) -> Result<Self, EvaluationApplicationError> {
        settings.validate()?;
        Ok(Self { settings })
    }
}

impl TaskEvaluationContractIssuer for DefaultTaskEvaluationContractIssuer {
    fn issue(
        &self,
        request: &TurnRequest,
    ) -> Result<Option<TaskEvaluationContract>, EvaluationApplicationError> {
        if !self.settings.enabled || request.requested_task_kind.is_none() {
            return Ok(None);
        }
        if self.settings.coding_rubric != "coding-v2" {
            return Err(EvaluationApplicationError::UnsupportedRubric(
                self.settings.coding_rubric.clone(),
            ));
        }
        let task_kind = request.requested_task_kind.expect("checked above");
        if task_kind != TaskKind::Coding {
            return Ok(None);
        }
        let turn_id = request
            .context
            .turn_id
            .ok_or(EvaluationApplicationError::MissingTurnIdentity)?;
        let contract = TaskEvaluationContract {
            schema_version: EVALUATION_SCHEMA_V1,
            contract_id: EvaluationContractId::new(),
            task_kind,
            subject: EvaluationSubject::Turn {
                turn_id,
                operation_id: request.operation_id,
            },
            rubric: RubricId(self.settings.coding_rubric.clone()),
            rubric_version: 2,
            mode: self.settings.default_mode,
            objective_ref: EvidenceRef(format!(
                "thread:{}:turn:{}:user_message",
                request.context.thread_id.0, turn_id.0
            )),
            requirement_refs: Vec::new(),
            required_evidence: vec![RequiredEvidence {
                kind: EvidenceKind::VerificationResult,
                minimum_count: 1,
                authoritative: true,
            }],
            required_gates: vec![
                RequiredGate {
                    name: "required_verification_passed".into(),
                },
                RequiredGate {
                    name: "change_within_scope".into(),
                },
            ],
            thresholds: self.settings.thresholds(),
            issued_by: "executive.evaluation/coding-v2".into(),
            issued_at_ms: Utc::now().timestamp_millis(),
        };
        contract.validate()?;
        contract.validate_turn_identity(turn_id, request.operation_id)?;
        Ok(Some(contract))
    }
}
