//! Improvement data model — proposals, patches, governance states, and promotion types.

use serde::{Deserialize, Serialize};

/// A stable proposal identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProposalId(pub String);

impl std::fmt::Display for ProposalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Governance state for an improvement proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// Initial state — proposal created but not yet submitted for approval.
    Proposed,
    /// Awaiting governance decision.
    PendingApproval,
    /// Approved by governance.
    Accepted,
    /// Rejected by governance.
    Rejected,
    /// Expired before a decision was made.
    Expired,
    /// Accepted proposal has been promoted to a MutationIntent.
    Promoted,
}

/// An improvement proposal targeting a specific capability or configuration surface.
///
/// Fields per spec §7: target capability, problem IDs, proposed change,
/// expected benefit, possible regressions, validation plan, rollback plan,
/// authority requirements, reversibility, expiration, and governance state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementProposal {
    /// Stable proposal identifier.
    pub id: ProposalId,
    /// The principal who created this proposal.
    pub proposer: String,
    /// Target capability or configuration surface (e.g., "tool.config", "care.priorities").
    pub target_capability: String,
    /// Problem IDs this proposal intends to address.
    pub problem_ids: Vec<String>,
    /// Description of the proposed change.
    pub proposed_change: String,
    /// Expected measurable benefit.
    pub expected_benefit: String,
    /// Possible regressions if the change is applied.
    pub possible_regressions: Vec<String>,
    /// Plan for validating the change (e.g., sandbox benchmarks).
    pub validation_plan: String,
    /// Plan for rolling back if the change fails.
    pub rollback_plan: String,
    /// Governance authority requirements (e.g., which principal may approve).
    pub authority_requirements: Vec<String>,
    /// Whether this change can be reverted after deployment.
    pub reversible: bool,
    /// Expiration timestamp in milliseconds since epoch.
    pub expires_at_ms: i64,
    /// Current governance state.
    pub state: ProposalState,
}

impl ImprovementProposal {
    /// Returns true if the proposal has expired (current time >= expiration).
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at_ms
    }

    /// Returns true if this is a privileged proposal (targets self-field or governance).
    pub fn is_privileged(&self) -> bool {
        self.target_capability.starts_with("self_field.")
            || self.target_capability.starts_with("governance.")
            || self.target_capability.starts_with("boundary.")
    }
}

/// Morphogenesis candidate — a proposed change to the genome.
#[derive(Debug, Clone)]
pub struct MorphogenesisCandidate {
    pub id: String,
    pub description: String,
    pub genome_patch: GenomePatch,
    pub reason: String,
}

/// A patch to apply to the genome.
#[derive(Debug, Clone)]
pub struct GenomePatch {
    /// Target path, e.g., "boundary.rules", "care.priorities"
    pub target: String,
    pub operation: PatchOperation,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum PatchOperation {
    Add,
    Remove,
    Replace,
    Modify,
}
