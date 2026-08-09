//! K4 single admit/invoke/receipt/recovery path (Agent Kernel V2).
//!
//! Strings budget, lease, deadline, sandbox, dispatch fence, usage and receipt
//! into one state machine.  There is exactly one admit/invoke/receipt/recovery
//! path — no standalone permit issuer/settler, no public component getter.
//! Each operation generation is switched to a writer/executor once; shadow
//! never performs a real effect.  This seam is contract/state only; the
//! legacy `DefaultCapabilityInvoker` remains authoritative until the K4 PR-C.

use serde::{Deserialize, Serialize};

/// One admit/invoke/receipt/recovery state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvocationState {
    Prepared,
    Observed,
    Settled,
    Recovering,
    Terminal,
}

/// The single invocation record the state machine tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub capability: String,
    pub state: InvocationState,
    pub operation_epoch: u64,
    pub sandbox_required: bool,
}

/// A typed state transition.  Only this state machine may advance an
/// invocation; there is no standalone permit issuer/settler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationTransition {
    Admit,
    Observe,
    Settle,
    Recover,
    Terminal,
}

/// Outcome of applying a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionResult {
    Ok(InvocationState),
    /// A prepared-but-not-observed crash window requires explicit
    /// reconciliation (K4).
    NeedsReconciliation,
    /// The invocation is already terminal; a late transition is a typed
    /// error, never a silent replay of the effect.
    AlreadyTerminal,
}

/// The single admit/invoke/receipt/recovery state machine.  Pure: no I/O, no
/// executor call.  The production path wires budget/lease/deadline/sandbox/
/// dispatch/usage/receipt into this at PR-C.
pub struct InvocationStateMachine;

impl InvocationStateMachine {
    pub fn apply(
        &self,
        record: &InvocationRecord,
        transition: InvocationTransition,
    ) -> TransitionResult {
        match (record.state, transition) {
            (_, InvocationTransition::Terminal) => TransitionResult::Ok(InvocationState::Terminal),
            (InvocationState::Terminal, _) => TransitionResult::AlreadyTerminal,
            (InvocationState::Prepared, InvocationTransition::Observe) => {
                TransitionResult::Ok(InvocationState::Observed)
            }
            // Prepared → any other transition that skips Observe is a crash
            // window that needs explicit reconciliation.
            (
                InvocationState::Prepared,
                InvocationTransition::Settle | InvocationTransition::Recover,
            ) => TransitionResult::NeedsReconciliation,
            (InvocationState::Observed, InvocationTransition::Settle) => {
                TransitionResult::Ok(InvocationState::Settled)
            }
            (InvocationState::Observed, InvocationTransition::Recover) => {
                TransitionResult::Ok(InvocationState::Recovering)
            }
            (InvocationState::Recovering, InvocationTransition::Settle) => {
                TransitionResult::Ok(InvocationState::Settled)
            }
            (_, InvocationTransition::Admit) => TransitionResult::Ok(InvocationState::Prepared),
            _ => TransitionResult::NeedsReconciliation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> InvocationRecord {
        InvocationRecord {
            capability: "tool.x".into(),
            state: InvocationState::Prepared,
            operation_epoch: 1,
            sandbox_required: true,
        }
    }

    #[test]
    fn prepared_observe_settle_is_the_happy_path() {
        let sm = InvocationStateMachine;
        let r = record();
        assert_eq!(
            sm.apply(&r, InvocationTransition::Observe),
            TransitionResult::Ok(InvocationState::Observed)
        );
        let r2 = InvocationRecord {
            state: InvocationState::Observed,
            ..r
        };
        assert_eq!(
            sm.apply(&r2, InvocationTransition::Settle),
            TransitionResult::Ok(InvocationState::Settled)
        );
    }

    #[test]
    fn prepared_settle_skips_observe_and_needs_reconciliation() {
        let sm = InvocationStateMachine;
        let r = record();
        assert_eq!(
            sm.apply(&r, InvocationTransition::Settle),
            TransitionResult::NeedsReconciliation
        );
    }

    #[test]
    fn terminal_rejects_late_transitions() {
        let sm = InvocationStateMachine;
        let r = InvocationRecord {
            state: InvocationState::Terminal,
            ..record()
        };
        assert_eq!(
            sm.apply(&r, InvocationTransition::Settle),
            TransitionResult::AlreadyTerminal
        );
    }
}
