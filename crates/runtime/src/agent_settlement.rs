//! Runtime-owned Agent recovery-to-settlement policy (RA-05).
//!
//! Host settlement engines may release leases, flush memory, and append
//! projections, but the mapping from a Runtime recovery decision to the
//! required resource disposition is Runtime semantics.

use ::contracts::AgentRecoveryDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResourceDisposition {
    RetainForResume,
    ReplaySettlement,
    TerminateAndReclaim,
}

pub fn recovery_disposition(decision: AgentRecoveryDecision) -> RecoveryResourceDisposition {
    match decision {
        AgentRecoveryDecision::Resume => RecoveryResourceDisposition::RetainForResume,
        AgentRecoveryDecision::Finalize => RecoveryResourceDisposition::ReplaySettlement,
        AgentRecoveryDecision::Interrupt | AgentRecoveryDecision::Reclaim => {
            RecoveryResourceDisposition::TerminateAndReclaim
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{recovery_disposition, RecoveryResourceDisposition};
    use ::contracts::AgentRecoveryDecision;

    #[test]
    fn recovery_policy_preserves_checkpointed_work_only() {
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Resume),
            RecoveryResourceDisposition::RetainForResume
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Finalize),
            RecoveryResourceDisposition::ReplaySettlement
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Interrupt),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Reclaim),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
    }
}
