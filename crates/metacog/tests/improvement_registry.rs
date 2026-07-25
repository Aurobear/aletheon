//! Integration tests for the improvement proposal registry.
//!
//! Covers: Proposed->PendingApproval->Accepted flow, reject self-approved
//! privileged proposals, reject invalid state transitions.

use metacog::improvement::{
    ImprovementProposal, ImprovementRegistry, InMemoryImprovementRegistry, ProposalDecision,
    ProposalId, ProposalState,
};

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

    // Submit for approval
    registry
        .decide(&id, ProposalDecision::Submit)
        .await
        .unwrap();

    // Accept by a different principal
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

    // Metacog approving its own privileged proposal should be rejected
    let result = registry
        .decide(
            &id,
            ProposalDecision::Accept {
                principal: "metacog".to_string(),
                reason: "self-approval attempt".to_string(),
            },
        )
        .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("self-approval"));
}

#[tokio::test]
async fn reject_invalid_transition_proposed_directly_to_accepted() {
    let registry = InMemoryImprovementRegistry::new();
    let proposal = make_proposal("prop-1", "metacog", "tool.config");
    let id = proposal.id.clone();

    registry.propose(proposal).await.unwrap();

    // Cannot skip PendingApproval
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
async fn reject_transition_from_rejected_to_accepted() {
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
async fn promote_only_after_accepted() {
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

    // Promote from Accepted should succeed
    registry
        .decide(&id, ProposalDecision::Promote)
        .await
        .unwrap();
}

#[tokio::test]
async fn promote_from_pending_approval_is_rejected() {
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

#[tokio::test]
async fn non_privileged_proposal_can_be_self_approved() {
    let registry = InMemoryImprovementRegistry::new();
    // tool.config is not privileged
    let proposal = make_proposal("prop-1", "metacog", "tool.config");
    let id = proposal.id.clone();

    registry.propose(proposal).await.unwrap();
    registry
        .decide(&id, ProposalDecision::Submit)
        .await
        .unwrap();

    // Metacog self-approving a non-privileged proposal is allowed
    let result = registry
        .decide(
            &id,
            ProposalDecision::Accept {
                principal: "metacog".to_string(),
                reason: "trivial tool config change".to_string(),
            },
        )
        .await;
    assert!(result.is_ok());
}
