use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentId, AttemptId, CodingJobId, GoalId, OperationId, TurnId};

use super::{EvaluationContractError, EVALUATION_SCHEMA_V1};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationContractId(pub Uuid);

impl EvaluationContractId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EvaluationContractId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvidenceRef(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Coding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationMode {
    Shadow,
    Enforce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluationSubject {
    Turn {
        turn_id: TurnId,
        operation_id: OperationId,
    },
    GoalAttempt {
        goal_id: GoalId,
        attempt_id: AttemptId,
    },
    AgentRun {
        agent_id: AgentId,
        operation_id: OperationId,
    },
    CodingJob {
        job_id: CodingJobId,
    },
}

impl EvaluationSubject {
    pub fn kind_and_id(&self) -> (&'static str, String) {
        match self {
            Self::Turn { turn_id, .. } => ("turn", turn_id.0.to_string()),
            Self::GoalAttempt {
                goal_id,
                attempt_id,
            } => ("goal_attempt", format!("{}:{}", goal_id.0, attempt_id.0)),
            Self::AgentRun {
                agent_id,
                operation_id,
            } => ("agent_run", format!("{}:{}", agent_id.0, operation_id.0)),
            Self::CodingJob { job_id } => ("coding_job", job_id.0.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationThresholds {
    pub min_score_millis: u32,
    pub min_evidence_coverage_millis: u16,
    pub min_confidence_millis: u16,
}

impl EvaluationThresholds {
    pub fn validate(&self) -> Result<(), EvaluationContractError> {
        if self.min_score_millis > 100_000 {
            return Err(EvaluationContractError::InvalidThreshold(
                "min_score_millis",
            ));
        }
        if self.min_evidence_coverage_millis > 1_000 {
            return Err(EvaluationContractError::InvalidThreshold(
                "min_evidence_coverage_millis",
            ));
        }
        if self.min_confidence_millis > 1_000 {
            return Err(EvaluationContractError::InvalidThreshold(
                "min_confidence_millis",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredEvidence {
    pub kind: crate::types::metacognition_evidence::EvidenceKind,
    pub minimum_count: u16,
    pub authoritative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredGate {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvaluationContract {
    pub schema_version: u16,
    pub contract_id: EvaluationContractId,
    pub task_kind: TaskKind,
    pub subject: EvaluationSubject,
    pub rubric: crate::types::metacognition_evaluation::RubricId,
    pub rubric_version: u32,
    pub mode: EvaluationMode,
    pub objective_ref: EvidenceRef,
    pub requirement_refs: Vec<EvidenceRef>,
    pub required_evidence: Vec<RequiredEvidence>,
    pub required_gates: Vec<RequiredGate>,
    pub thresholds: EvaluationThresholds,
    pub issued_by: String,
    pub issued_at_ms: i64,
}

impl TaskEvaluationContract {
    pub fn validate(&self) -> Result<(), EvaluationContractError> {
        if self.schema_version != EVALUATION_SCHEMA_V1 {
            return Err(EvaluationContractError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        if self.rubric.0.trim().is_empty() {
            return Err(EvaluationContractError::EmptyField("rubric"));
        }
        if self.rubric_version == 0 {
            return Err(EvaluationContractError::EmptyField("rubric_version"));
        }
        if self.objective_ref.0.trim().is_empty() {
            return Err(EvaluationContractError::EmptyField("objective_ref"));
        }
        if self.issued_by.trim().is_empty() {
            return Err(EvaluationContractError::EmptyField("issued_by"));
        }
        if self.required_gates.is_empty() {
            return Err(EvaluationContractError::MissingRequiredGates);
        }
        if self
            .required_gates
            .iter()
            .any(|gate| gate.name.trim().is_empty())
        {
            return Err(EvaluationContractError::EmptyField("required_gate"));
        }
        if self
            .required_evidence
            .iter()
            .any(|requirement| requirement.minimum_count == 0)
        {
            return Err(EvaluationContractError::InvalidThreshold(
                "required_evidence.minimum_count",
            ));
        }
        self.thresholds.validate()
    }

    pub fn validate_turn_identity(
        &self,
        turn_id: TurnId,
        operation_id: OperationId,
    ) -> Result<(), EvaluationContractError> {
        match &self.subject {
            EvaluationSubject::Turn {
                turn_id: contract_turn,
                operation_id: contract_operation,
            } if *contract_turn == turn_id && *contract_operation == operation_id => Ok(()),
            _ => Err(EvaluationContractError::IdentityMismatch("turn")),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn fixture(mode: EvaluationMode) -> TaskEvaluationContract {
        TaskEvaluationContract {
            schema_version: EVALUATION_SCHEMA_V1,
            contract_id: EvaluationContractId::new(),
            task_kind: TaskKind::Coding,
            subject: EvaluationSubject::Turn {
                turn_id: TurnId::new(),
                operation_id: OperationId::new(),
            },
            rubric: crate::types::metacognition_evaluation::RubricId("coding-v2".into()),
            rubric_version: 2,
            mode,
            objective_ref: EvidenceRef("session:s:turn:t:user_message".into()),
            requirement_refs: Vec::new(),
            required_evidence: vec![RequiredEvidence {
                kind: crate::types::metacognition_evidence::EvidenceKind::VerificationResult,
                minimum_count: 1,
                authoritative: true,
            }],
            required_gates: vec![RequiredGate {
                name: "required_verification_passed".into(),
            }],
            thresholds: EvaluationThresholds {
                min_score_millis: 70_000,
                min_evidence_coverage_millis: 600,
                min_confidence_millis: 700,
            },
            issued_by: "executive".into(),
            issued_at_ms: 1,
        }
    }

    #[test]
    fn coding_contract_round_trips_and_rejects_invalid_thresholds() {
        let contract = fixture(EvaluationMode::Shadow);
        let json = serde_json::to_string(&contract).unwrap();
        assert_eq!(
            serde_json::from_str::<TaskEvaluationContract>(&json).unwrap(),
            contract
        );
        let mut invalid = contract;
        invalid.thresholds.min_evidence_coverage_millis = 1_001;
        assert_eq!(
            invalid.validate(),
            Err(EvaluationContractError::InvalidThreshold(
                "min_evidence_coverage_millis"
            ))
        );
    }
}
