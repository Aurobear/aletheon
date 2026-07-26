//! Deterministic comparison of declared obligations with authoritative evidence.

use fabric::OperationId;
use serde::{Deserialize, Serialize};

use super::cognitive_task::{
    CognitiveTurnState, Obligation, RequiredAction, ValidationRequirement,
};
use super::evidence::{EvidenceLedger, EvidenceSubject};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressDecision {
    Continue {
        missing: Vec<Obligation>,
    },
    NeedsValidation {
        requirements: Vec<ValidationRequirement>,
    },
    WaitingForTerminalEvidence {
        operations: Vec<OperationId>,
    },
    Complete,
    Blocked {
        reason: String,
    },
}

impl ProgressDecision {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }

    pub fn recovery_message(&self) -> Option<String> {
        let lines = match self {
            Self::Complete => return None,
            Self::Continue { missing } => missing
                .iter()
                .map(|item| format!("- missing obligation: {item:?}"))
                .collect(),
            Self::NeedsValidation { requirements } => requirements
                .iter()
                .map(|item| format!("- validation `{}` has not completed", item.id))
                .collect(),
            Self::WaitingForTerminalEvidence { operations } => operations
                .iter()
                .map(|id| format!("- terminal evidence for operation `{:?}` is missing", id))
                .collect(),
            Self::Blocked { reason } => vec![format!("- blocked: {reason}")],
        };
        Some(format!(
            "[cognitive_completion_rejected]\n{}\nContinue the task. Do not claim completion until these obligations are met.",
            lines.join("\n")
        ))
    }
}

#[derive(Debug, Default)]
pub struct ProgressAuditor;

impl ProgressAuditor {
    pub fn audit(&self, state: &CognitiveTurnState, evidence: &EvidenceLedger) -> ProgressDecision {
        let unresolved = state
            .outstanding
            .iter()
            .filter(|obligation| !is_satisfied(obligation, evidence))
            .cloned()
            .collect::<Vec<_>>();

        if unresolved.is_empty() {
            return ProgressDecision::Complete;
        }

        let waiting = unresolved
            .iter()
            .filter_map(|obligation| match obligation {
                Obligation::RequiredAction(RequiredAction::ObserveTerminal { operation_id }) => {
                    Some(*operation_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if !waiting.is_empty() {
            return ProgressDecision::WaitingForTerminalEvidence {
                operations: waiting,
            };
        }

        let validations = unresolved
            .iter()
            .filter_map(|obligation| match obligation {
                Obligation::Validation(requirement) => Some(requirement.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !validations.is_empty() {
            return ProgressDecision::NeedsValidation {
                requirements: validations,
            };
        }

        ProgressDecision::Continue {
            missing: unresolved,
        }
    }
}

fn is_satisfied(obligation: &Obligation, evidence: &EvidenceLedger) -> bool {
    let subject = match obligation {
        Obligation::RequiredAction(RequiredAction::InvokeAgent { runtime }) => {
            EvidenceSubject::AgentInvocation {
                runtime: runtime.clone(),
            }
        }
        Obligation::RequiredAction(RequiredAction::InvokeTool { tool_name }) => {
            EvidenceSubject::ToolInvocation {
                tool_name: tool_name.clone(),
            }
        }
        Obligation::RequiredAction(RequiredAction::ObserveTerminal { operation_id }) => {
            EvidenceSubject::OperationTerminal {
                operation_id: *operation_id,
            }
        }
        Obligation::Deliverable(deliverable) => EvidenceSubject::Deliverable {
            deliverable_id: deliverable.id.clone(),
        },
        Obligation::Validation(requirement) => EvidenceSubject::Validation {
            requirement_id: requirement.id.clone(),
        },
        Obligation::UnresolvedRequirement { .. } => return false,
    };
    evidence.successful_for(&subject).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cognitive_task::{
        AgentRuntimeId, CognitiveTaskContract, CognitiveTaskKind, EvidenceId, RequiredAction,
    };
    use crate::core::evidence::{
        EvidenceLevel, EvidenceLocator, EvidenceRecord, EvidenceSource, TerminalStatus,
    };

    fn agent_state() -> CognitiveTurnState {
        CognitiveTurnState::from_contract(CognitiveTaskContract {
            objective: "delegate work".into(),
            task_kind: CognitiveTaskKind::RequiredAgentExecution,
            required_actions: vec![RequiredAction::InvokeAgent {
                runtime: AgentRuntimeId("selected-runtime".into()),
            }],
            deliverables: Vec::new(),
            validation_requirements: Vec::new(),
        })
    }

    fn agent_record(status: TerminalStatus) -> EvidenceRecord {
        let runtime = AgentRuntimeId("selected-runtime".into());
        EvidenceRecord {
            id: EvidenceId("agent-receipt".into()),
            subject: EvidenceSubject::AgentInvocation {
                runtime: runtime.clone(),
            },
            source: EvidenceSource::AgentRuntime { runtime },
            level: EvidenceLevel::RuntimeVerified,
            terminal_status: status,
            locator: EvidenceLocator::DurableReceipt {
                receipt_id: "agent-receipt".into(),
            },
            digest: None,
        }
    }

    #[test]
    fn agent_submission_does_not_complete_obligation() {
        let state = agent_state();
        let mut ledger = EvidenceLedger::default();
        ledger.record(agent_record(TerminalStatus::Pending));

        assert!(matches!(
            ProgressAuditor.audit(&state, &ledger),
            ProgressDecision::Continue { missing } if missing.len() == 1
        ));
    }

    #[test]
    fn terminal_agent_receipt_completes_obligation() {
        let state = agent_state();
        let mut ledger = EvidenceLedger::default();
        ledger.record(agent_record(TerminalStatus::Succeeded));

        assert_eq!(
            ProgressAuditor.audit(&state, &ledger),
            ProgressDecision::Complete
        );
    }
}
