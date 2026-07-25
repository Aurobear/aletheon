//! Improvement proposal registry — tracks proposals through governance states.
//!
//! The registry enforces valid state transitions and rejects self-approval
//! of privileged proposals.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use thiserror::Error;

use super::model::{ImprovementProposal, ProposalId, ProposalState};

#[derive(Debug, Error)]
pub enum ProposalError {
    #[error("proposal with id {0} already exists")]
    AlreadyExists(ProposalId),
    #[error("proposal with id {0} not found")]
    NotFound(ProposalId),
    #[error("invalid state transition: from {from:?} to {to:?}")]
    InvalidTransition {
        from: ProposalState,
        to: ProposalState,
    },
    #[error(
        "self-approval rejected: principal {principal} cannot approve own privileged proposal {id}"
    )]
    SelfApproval { principal: String, id: ProposalId },
    #[error("proposal {0} has expired")]
    Expired(ProposalId),
}

/// Decision on a proposal.
#[derive(Debug, Clone)]
pub enum ProposalDecision {
    /// Advance to PendingApproval for governance review.
    Submit,
    /// Accept the proposal.
    Accept { principal: String, reason: String },
    /// Reject the proposal.
    Reject { principal: String, reason: String },
    /// Mark as promoted after conversion to MutationIntent.
    Promote,
}

/// Improvement proposal registry port.
///
/// Tracks proposals through governance states: Proposed -> PendingApproval ->
/// Accepted/Rejected/Expired -> Promoted.
#[async_trait]
pub trait ImprovementRegistry: Send + Sync {
    /// Register a new proposal in Proposed state.
    async fn propose(&self, proposal: ImprovementProposal) -> Result<(), ProposalError>;

    /// Apply a governance decision to a proposal.
    async fn decide(
        &self,
        id: &ProposalId,
        decision: ProposalDecision,
    ) -> Result<(), ProposalError>;

    /// Retrieve an accepted proposal by ID.
    async fn accepted(&self, id: &ProposalId) -> Result<ImprovementProposal, ProposalError>;
}

/// In-memory improvement proposal registry.
///
/// For testing and single-process use. Production should use a persisted store.
pub struct InMemoryImprovementRegistry {
    proposals: Mutex<HashMap<ProposalId, ImprovementProposal>>,
}

impl InMemoryImprovementRegistry {
    pub fn new() -> Self {
        Self {
            proposals: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryImprovementRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ImprovementRegistry for InMemoryImprovementRegistry {
    async fn propose(&self, proposal: ImprovementProposal) -> Result<(), ProposalError> {
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalError::NotFound(ProposalId(format!("lock poisoned: {e}"))))?;
        if proposals.contains_key(&proposal.id) {
            return Err(ProposalError::AlreadyExists(proposal.id));
        }
        proposals.insert(proposal.id.clone(), proposal);
        Ok(())
    }

    async fn decide(
        &self,
        id: &ProposalId,
        decision: ProposalDecision,
    ) -> Result<(), ProposalError> {
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalError::NotFound(ProposalId(format!("lock poisoned: {e}"))))?;
        let proposal = proposals
            .get_mut(id)
            .ok_or_else(|| ProposalError::NotFound(id.clone()))?;

        match decision {
            ProposalDecision::Submit => {
                if proposal.state != ProposalState::Proposed {
                    return Err(ProposalError::InvalidTransition {
                        from: proposal.state,
                        to: ProposalState::PendingApproval,
                    });
                }
                proposal.state = ProposalState::PendingApproval;
            }
            ProposalDecision::Accept {
                principal,
                reason: _,
            } => {
                if proposal.state != ProposalState::PendingApproval {
                    return Err(ProposalError::InvalidTransition {
                        from: proposal.state,
                        to: ProposalState::Accepted,
                    });
                }
                // Reject self-approval of privileged proposals
                if proposal.is_privileged() && principal == proposal.proposer {
                    return Err(ProposalError::SelfApproval {
                        principal,
                        id: id.clone(),
                    });
                }
                proposal.state = ProposalState::Accepted;
            }
            ProposalDecision::Reject {
                principal: _,
                reason: _,
            } => {
                if proposal.state != ProposalState::PendingApproval {
                    return Err(ProposalError::InvalidTransition {
                        from: proposal.state,
                        to: ProposalState::Rejected,
                    });
                }
                proposal.state = ProposalState::Rejected;
            }
            ProposalDecision::Promote => {
                if proposal.state != ProposalState::Accepted {
                    return Err(ProposalError::InvalidTransition {
                        from: proposal.state,
                        to: ProposalState::Promoted,
                    });
                }
                proposal.state = ProposalState::Promoted;
            }
        }
        Ok(())
    }

    async fn accepted(&self, id: &ProposalId) -> Result<ImprovementProposal, ProposalError> {
        let proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalError::NotFound(ProposalId(format!("lock poisoned: {e}"))))?;
        let proposal = proposals
            .get(id)
            .ok_or_else(|| ProposalError::NotFound(id.clone()))?;
        if proposal.state != ProposalState::Accepted {
            return Err(ProposalError::InvalidTransition {
                from: proposal.state,
                to: ProposalState::Accepted,
            });
        }
        Ok(proposal.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_proposal(id: &str, proposer: &str, target: &str) -> ImprovementProposal {
        ImprovementProposal {
            id: ProposalId(id.to_string()),
            proposer: proposer.to_string(),
            target_capability: target.to_string(),
            problem_ids: vec!["p1".to_string()],
            proposed_change: "test change".to_string(),
            expected_benefit: "test benefit".to_string(),
            possible_regressions: vec![],
            validation_plan: "validate in sandbox".to_string(),
            rollback_plan: "revert to previous genome".to_string(),
            authority_requirements: vec!["governor".to_string()],
            reversible: true,
            expires_at_ms: i64::MAX,
            state: ProposalState::Proposed,
        }
    }

    #[tokio::test]
    async fn proposed_to_pending_approval_to_accepted_flow() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "tool.config");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();

        // Proposed -> PendingApproval
        registry
            .decide(&id, ProposalDecision::Submit)
            .await
            .unwrap();

        // PendingApproval -> Accepted (non-privileged)
        registry
            .decide(
                &id,
                ProposalDecision::Accept {
                    principal: "governor".to_string(),
                    reason: "reasonable change".to_string(),
                },
            )
            .await
            .unwrap();

        let accepted = registry.accepted(&id).await.unwrap();
        assert_eq!(accepted.state, ProposalState::Accepted);
    }

    #[tokio::test]
    async fn reject_self_approved_privileged_proposal() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "boundary.rules");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();
        registry
            .decide(&id, ProposalDecision::Submit)
            .await
            .unwrap();

        // Metacog trying to approve its own privileged proposal
        let result = registry
            .decide(
                &id,
                ProposalDecision::Accept {
                    principal: "metacog".to_string(),
                    reason: "i think this is fine".to_string(),
                },
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("self-approval"));
    }

    #[tokio::test]
    async fn reject_invalid_transition_proposed_to_accepted() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "tool.config");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();

        // Cannot accept directly from Proposed
        let result = registry
            .decide(
                &id,
                ProposalDecision::Accept {
                    principal: "governor".to_string(),
                    reason: "skip step".to_string(),
                },
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn reject_transition_rejected_to_accepted() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "tool.config");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();
        registry
            .decide(&id, ProposalDecision::Submit)
            .await
            .unwrap();
        registry
            .decide(
                &id,
                ProposalDecision::Reject {
                    principal: "governor".to_string(),
                    reason: "too risky".to_string(),
                },
            )
            .await
            .unwrap();

        // Cannot accept from Rejected
        let result = registry
            .decide(
                &id,
                ProposalDecision::Accept {
                    principal: "governor".to_string(),
                    reason: "changed my mind".to_string(),
                },
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn promote_only_from_accepted() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "tool.config");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();
        registry
            .decide(&id, ProposalDecision::Submit)
            .await
            .unwrap();
        registry
            .decide(
                &id,
                ProposalDecision::Accept {
                    principal: "governor".to_string(),
                    reason: "good".to_string(),
                },
            )
            .await
            .unwrap();

        // Promote from Accepted should work
        registry
            .decide(&id, ProposalDecision::Promote)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn promote_from_pending_approval_fails() {
        let registry = InMemoryImprovementRegistry::new();
        let proposal = make_proposal("prop-1", "metacog", "tool.config");
        let id = proposal.id.clone();

        registry.propose(proposal).await.unwrap();
        registry
            .decide(&id, ProposalDecision::Submit)
            .await
            .unwrap();

        // Promote from PendingApproval should fail
        let result = registry.decide(&id, ProposalDecision::Promote).await;
        assert!(result.is_err());
    }
}
