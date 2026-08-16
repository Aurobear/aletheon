//! Typed, model-history-independent state for practical cognitive work.

use ::contracts::OperationId;
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

    /// Replace version-bound obligations for one change transaction with the
    /// latest applied workspace version.
    ///
    /// A transaction may contain several scoped writes. Once a later write is
    /// applied, review or validation of an earlier workspace version is both
    /// stale and impossible: the transaction registry only exposes its current
    /// version. Keeping every intermediate version as an outstanding action
    /// would therefore make deterministic completion unattainable and pressure
    /// the model into invalid `git_diff` calls for historical versions.
    pub fn require_current_change_version(
        &mut self,
        transaction_id: &str,
        workspace_version: &str,
        requires_validation: bool,
    ) {
        let belongs_to_transaction = |action: &RequiredAction| match action {
            RequiredAction::ReviewChange {
                transaction_id: id, ..
            }
            | RequiredAction::ValidateChange {
                transaction_id: id, ..
            }
            | RequiredAction::AcceptChange {
                transaction_id: id, ..
            } => id == transaction_id,
            _ => false,
        };
        self.contract
            .required_actions
            .retain(|action| !belongs_to_transaction(action));
        self.outstanding.retain(|obligation| {
            !matches!(
                obligation,
                Obligation::RequiredAction(action) if belongs_to_transaction(action)
            )
        });

        self.require_action(RequiredAction::ReviewChange {
            transaction_id: transaction_id.to_owned(),
            workspace_version: workspace_version.to_owned(),
        });
        if requires_validation {
            self.require_action(RequiredAction::ValidateChange {
                transaction_id: transaction_id.to_owned(),
                workspace_version: workspace_version.to_owned(),
            });
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

    #[test]
    fn newer_change_version_supersedes_stale_transaction_obligations() {
        let mut state = CognitiveTurnState::from_contract(CognitiveTaskContract {
            objective: "change two files".into(),
            task_kind: CognitiveTaskKind::CodeChange,
            required_actions: Vec::new(),
            deliverables: Vec::new(),
            validation_requirements: Vec::new(),
        });
        state.require_current_change_version("tx", "v1", true);
        state.require_current_change_version("other", "o1", false);
        state.require_current_change_version("tx", "v2", true);

        assert!(!state.outstanding.iter().any(|obligation| matches!(
            obligation,
            Obligation::RequiredAction(
                RequiredAction::ReviewChange { transaction_id, workspace_version }
                | RequiredAction::ValidateChange { transaction_id, workspace_version }
            ) if transaction_id == "tx" && workspace_version == "v1"
        )));
        assert!(state.outstanding.contains(&Obligation::RequiredAction(
            RequiredAction::ReviewChange {
                transaction_id: "tx".into(),
                workspace_version: "v2".into(),
            }
        )));
        assert!(state.outstanding.contains(&Obligation::RequiredAction(
            RequiredAction::ValidateChange {
                transaction_id: "tx".into(),
                workspace_version: "v2".into(),
            }
        )));
        assert!(state.outstanding.contains(&Obligation::RequiredAction(
            RequiredAction::ReviewChange {
                transaction_id: "other".into(),
                workspace_version: "o1".into(),
            }
        )));
    }
}
