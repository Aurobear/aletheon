//! RA-04 canonical Turn reducer owner seam (Agent Kernel V2).
//!
//! The single Turn state machine/terminal writer contract.  This seam defines
//! the typed transitions (start, cancel, timeout, disconnect, late receipt,
//! crash) and the terminal fence — a valid late effect may only append
//! observation/accounting, never reverse terminal.  The production
//! `RuntimeTurnWriter` consumes this reducer; the legacy coordinator remains
//! an execution facade while the cutover is completed.

use crate::error::RuntimeError;
use crate::event::TurnTerminal;
use crate::ids::{SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// Typed Turn transition.  Every cancel/timeout/disconnect/late-receipt/crash
/// path goes through one of these — no second terminal state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnTransition {
    Start {
        session: SessionId,
        turn: TurnId,
    },
    Cancel {
        session: SessionId,
        turn: TurnId,
    },
    Timeout {
        session: SessionId,
        turn: TurnId,
    },
    Disconnect {
        session: SessionId,
        turn: TurnId,
    },
    LateReceipt {
        session: SessionId,
        turn: TurnId,
    },
    Crash {
        session: SessionId,
        turn: TurnId,
    },
    Settle {
        session: SessionId,
        turn: TurnId,
        terminal: TurnTerminal,
    },
}

/// Outcome of a Turn transition: either the terminal state or a pending
/// observation.  A valid late effect may only append observation/accounting;
/// it must never reverse an already-terminal turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionOutcome {
    Terminal {
        turn: TurnId,
        terminal: TurnTerminal,
    },
    Pending {
        turn: TurnId,
        /// Late effect observation/accounting (never terminal reversal).
        observation: Option<String>,
    },
}

/// Canonical Turn reducer seam. The writer invokes this pure transition table
/// so all terminal and late-observation decisions share one implementation.
pub struct TurnReducerSeam;

impl TurnReducerSeam {
    /// Pure transition classification: given the current terminal state and
    /// an incoming transition, decide Terminal vs Pending.  No I/O, no writer.
    pub fn apply(
        &self,
        current_terminal: Option<&TurnTerminal>,
        transition: &TurnTransition,
    ) -> Result<TransitionOutcome, RuntimeError> {
        let turn = match transition {
            TurnTransition::Start { turn, .. } => turn.clone(),
            TurnTransition::Cancel { turn, .. } => turn.clone(),
            TurnTransition::Timeout { turn, .. } => turn.clone(),
            TurnTransition::Disconnect { turn, .. } => turn.clone(),
            TurnTransition::LateReceipt { turn, .. } => turn.clone(),
            TurnTransition::Crash { turn, .. } => turn.clone(),
            TurnTransition::Settle { turn, .. } => turn.clone(),
        };

        match transition {
            TurnTransition::Start { .. } => Ok(TransitionOutcome::Pending {
                turn,
                observation: None,
            }),
            TurnTransition::Settle {
                terminal, session, ..
            } => {
                // A turn that is already terminal must not be reversed; a
                // duplicate settle is rejected (already-terminal is typed).
                if current_terminal.is_some() {
                    return Err(RuntimeError::AlreadyTerminal);
                }
                let _ = session;
                Ok(TransitionOutcome::Terminal {
                    turn,
                    terminal: terminal.clone(),
                })
            }
            TurnTransition::Cancel { .. }
            | TurnTransition::Timeout { .. }
            | TurnTransition::Disconnect { .. }
            | TurnTransition::Crash { .. } => {
                // If already terminal, a cancel/timeout/disconnect/crash is a
                // late observation only — never a terminal reversal.
                if let Some(terminal) = current_terminal {
                    Ok(TransitionOutcome::Terminal {
                        turn,
                        terminal: terminal.clone(),
                    })
                } else {
                    Ok(TransitionOutcome::Pending {
                        turn,
                        observation: Some("interrupted-before-terminal".into()),
                    })
                }
            }
            TurnTransition::LateReceipt { .. } => {
                // Late receipt after terminal → accounting only.
                if current_terminal.is_some() {
                    Ok(TransitionOutcome::Terminal {
                        turn,
                        terminal: current_terminal.cloned().expect("checked above"),
                    })
                } else {
                    Ok(TransitionOutcome::Pending {
                        turn,
                        observation: Some("late-receipt-pending".into()),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(sid: &str, tid: &str) -> (SessionId, TurnId) {
        (SessionId(sid.into()), TurnId(tid.into()))
    }

    #[test]
    fn settle_after_terminal_is_rejected_no_reversal() {
        let seam = TurnReducerSeam;
        let (s, t) = t("s1", "t1");
        // First settle → terminal Completed.
        let out = seam
            .apply(
                None,
                &TurnTransition::Settle {
                    session: s.clone(),
                    turn: t.clone(),
                    terminal: TurnTerminal::Completed,
                },
            )
            .unwrap();
        assert!(matches!(out, TransitionOutcome::Terminal { .. }));
        // Second settle (duplicate) → typed AlreadyTerminal, never reversed.
        assert_eq!(
            seam.apply(
                Some(&TurnTerminal::Completed),
                &TurnTransition::Settle {
                    session: s,
                    turn: t,
                    terminal: TurnTerminal::Failed {
                        message: "late".into(),
                    },
                },
            ),
            Err(RuntimeError::AlreadyTerminal)
        );
    }

    #[test]
    fn cancel_after_terminal_is_observation_only() {
        let seam = TurnReducerSeam;
        let (s, t) = t("s1", "t1");
        let out = seam
            .apply(
                Some(&TurnTerminal::Interrupted),
                &TurnTransition::Cancel {
                    session: s,
                    turn: t,
                },
            )
            .unwrap();
        // Terminal preserved; cancel does not reverse it.
        assert_eq!(
            out,
            TransitionOutcome::Terminal {
                turn: TurnId("t1".into()),
                terminal: TurnTerminal::Interrupted,
            }
        );
    }
}
