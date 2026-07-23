//! Integration tests for proposal promotion — converting approved
//! ImprovementProposals into governed MutationIntents.
//!
//! Covers: reject unapproved, expired, irreversible-without-rollback, and
//! evidence-free proposals; accept approved proposal with problem IDs and rollback plan.

use metacog::improvement::{
    DeterministicProposalPromoter, ImprovementProposal, ProposalId, ProposalPromoter, ProposalState,
};

fn make_proposal(
    id: &str,
    state: ProposalState,
    reversible: bool,
    problem_ids: Vec<String>,
    rollback_plan: &str,
    expires_at_ms: i64,
) -> ImprovementProposal {
    ImprovementProposal {
        id: ProposalId(id.to_string()),
        proposer: "test".to_string(),
        target_capability: "tool.config".to_string(),
        problem_ids,
        proposed_change: "test change".to_string(),
        expected_benefit: "test benefit".to_string(),
        possible_regressions: vec![],
        validation_plan: "validate in sandbox".to_string(),
        rollback_plan: rollback_plan.to_string(),
        authority_requirements: vec!["governor".to_string()],
        reversible,
        expires_at_ms,
        state,
    }
}

#[test]
fn promote_accepted_proposal_with_problem_ids_and_rollback() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        true,
        vec!["p1".to_string(), "p2".to_string()],
        "revert to previous genome",
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_ok());

    let intent = result.unwrap();
    assert_eq!(intent.target, "tool.config");
    assert!(intent.reversible);

    let change = &intent.change;
    assert_eq!(change["proposal_id"], "prop-1");
    assert!(change["problem_ids"].as_array().unwrap().len() == 2);
    assert_eq!(change["rollback_plan"], "revert to previous genome");
    assert!(intent.reason.contains("approved proposal"));
}

#[test]
fn reject_unapproved_proposal() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Proposed,
        true,
        vec!["p1".to_string()],
        "revert",
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not in Accepted"));
}

#[test]
fn reject_pending_approval_proposal() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::PendingApproval,
        true,
        vec!["p1".to_string()],
        "revert",
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
}

#[test]
fn reject_rejected_proposal() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Rejected,
        true,
        vec!["p1".to_string()],
        "revert",
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
}

#[test]
fn reject_expired_proposal() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        true,
        vec!["p1".to_string()],
        "revert",
        100, // expires at 100ms
    );
    let result = promoter.promote(&proposal, 200); // now = 200ms > 100ms
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("expired"));
}

#[test]
fn reject_irreversible_without_rollback_plan() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        false, // irreversible
        vec!["p1".to_string()],
        "", // empty rollback plan
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("irreversible"));
}

#[test]
fn irreversible_proposal_with_rollback_plan_is_accepted() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        false, // irreversible
        vec!["p1".to_string()],
        "full system restore", // has rollback plan
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_ok());
}

#[test]
fn reject_evidence_free_proposal() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        true,
        vec![], // empty problem_ids = no evidence
        "revert",
        i64::MAX,
    );
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("evidence"));
}

#[test]
fn proposal_not_yet_expired_is_accepted() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Accepted,
        true,
        vec!["p1".to_string()],
        "revert",
        500, // expires at 500ms
    );
    // now = 200ms < 500ms, not expired yet
    let result = promoter.promote(&proposal, 200);
    assert!(result.is_ok());
}

#[test]
fn promoted_proposal_is_still_promoted_by_state() {
    let promoter = DeterministicProposalPromoter;
    let proposal = make_proposal(
        "prop-1",
        ProposalState::Promoted,
        true,
        vec!["p1".to_string()],
        "revert",
        i64::MAX,
    );
    // Already promoted should be rejected (not in Accepted state)
    let result = promoter.promote(&proposal, 0);
    assert!(result.is_err());
}
