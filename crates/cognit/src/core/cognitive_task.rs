//! Typed, model-history-independent state for practical cognitive work.

use fabric::OperationId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskContract {
    pub objective: String,
    pub task_kind: CognitiveTaskKind,
    pub required_actions: Vec<RequiredAction>,
    pub deliverables: Vec<Deliverable>,
    pub validation_requirements: Vec<ValidationRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CognitiveTaskKind {
    General,
    RepositoryAnalysis,
    CodeChange,
    RequiredAgentExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentRuntimeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequiredAction {
    RunRoleGraph {
        root_task_id: String,
        receipt_id: Option<String>,
    },
    InvokeAgent {
        runtime: AgentRuntimeId,
    },
    InvokeTool {
        tool_name: String,
    },
    ObserveTerminal {
        operation_id: OperationId,
    },
    ObserveCommandSession {
        session_id: String,
    },
    ReviewChange {
        transaction_id: String,
        workspace_version: String,
    },
    ValidateChange {
        transaction_id: String,
        workspace_version: String,
    },
    AcceptChange {
        transaction_id: String,
        workspace_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationRequirement {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CognitiveWorkPhase {
    Investigate,
    Plan,
    Execute,
    Verify,
    Synthesize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlan {
    pub steps: Vec<CognitivePlanStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlanStep {
    pub id: String,
    pub description: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EvidenceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationRecord {
    pub requirement_id: String,
    pub evidence: EvidenceId,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Obligation {
    RequiredAction(RequiredAction),
    Deliverable(Deliverable),
    Validation(ValidationRequirement),
    UnresolvedRequirement { description: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTurnState {
    pub contract: CognitiveTaskContract,
    pub phase: CognitiveWorkPhase,
    pub plan: Option<CognitivePlan>,
    pub observation_evidence: Vec<EvidenceId>,
    pub validations: Vec<ValidationRecord>,
    pub outstanding: Vec<Obligation>,
    pub completion_attempts: u32,
}

impl CognitiveTurnState {
    pub fn from_contract(contract: CognitiveTaskContract) -> Self {
        let mut outstanding = Vec::new();
        outstanding.extend(
            contract
                .required_actions
                .iter()
                .cloned()
                .map(Obligation::RequiredAction),
        );
        outstanding.extend(
            contract
                .deliverables
                .iter()
                .cloned()
                .map(Obligation::Deliverable),
        );
        outstanding.extend(
            contract
                .validation_requirements
                .iter()
                .cloned()
                .map(Obligation::Validation),
        );
        Self {
            contract,
            phase: CognitiveWorkPhase::Investigate,
            plan: None,
            observation_evidence: Vec::new(),
            validations: Vec::new(),
            outstanding,
            completion_attempts: 0,
        }
    }

    pub fn require_action(&mut self, action: RequiredAction) {
        if !self
            .outstanding
            .iter()
            .any(|obligation| obligation == &Obligation::RequiredAction(action.clone()))
        {
            self.contract.required_actions.push(action.clone());
            self.outstanding.push(Obligation::RequiredAction(action));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_state_materializes_every_declared_obligation() {
        let contract = CognitiveTaskContract {
            objective: "make a verified change".into(),
            task_kind: CognitiveTaskKind::CodeChange,
            required_actions: vec![RequiredAction::InvokeTool {
                tool_name: "file_read".into(),
            }],
            deliverables: vec![Deliverable {
                id: "patch".into(),
                description: "reviewable patch".into(),
            }],
            validation_requirements: vec![ValidationRequirement {
                id: "focused-test".into(),
                description: "focused deterministic test".into(),
            }],
        };

        let state = CognitiveTurnState::from_contract(contract);

        assert_eq!(state.phase, CognitiveWorkPhase::Investigate);
        assert_eq!(state.outstanding.len(), 3);
        assert_eq!(state.completion_attempts, 0);
    }

    #[test]
    fn contract_round_trips_without_model_messages() {
        let state = CognitiveTurnState::from_contract(CognitiveTaskContract {
            objective: "invoke selected runtime".into(),
            task_kind: CognitiveTaskKind::RequiredAgentExecution,
            required_actions: vec![RequiredAction::InvokeAgent {
                runtime: AgentRuntimeId("runtime-from-registry".into()),
            }],
            deliverables: Vec::new(),
            validation_requirements: Vec::new(),
        });

        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: CognitiveTurnState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, state);
    }
}
