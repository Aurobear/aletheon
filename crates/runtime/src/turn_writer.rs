//! RA-04 PR-C Runtime Turn writer (Agent Kernel V2).
//!
//! The deployable Turn writer: the Runtime mints the canonical `TurnId`, runs
//! every cancel/timeout/disconnect/late-receipt/crash through the single
//! `TurnReducerSeam` terminal fence, and emits typed `TurnStarted`/`TurnSettled`
//! events.  The legacy `TurnCoordinator` stays authoritative until the PR-C
//! switch.  This is the PR-C code; the maintenance/drain + installed acceptance
//! is the deployment slice.

use crate::command::{CommandReceipt, RuntimeCommand, StartTurnCommand};
use crate::error::RuntimeError;
use crate::event::{RuntimeEvent, TurnTerminal};
use crate::ids::{SessionId, TurnId};
use crate::ports::RuntimeCommandPort;
use crate::turn_reducer::{TransitionOutcome, TurnReducerSeam, TurnTransition};
use async_trait::async_trait;

/// Runtime Turn writer over the canonical Turn reducer + a typed event sink.
pub struct RuntimeTurnWriter {
    reducer: TurnReducerSeam,
    /// Current terminal per turn (single terminal source).
    terminals: std::sync::Mutex<std::collections::HashMap<TurnId, TurnTerminal>>,
    /// Typed events emitted (in a real composition this is the RuntimeEventPort
    /// journal; here a test-observable vector).
    emitted: std::sync::Mutex<Vec<RuntimeEvent>>,
}

impl Default for RuntimeTurnWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeTurnWriter {
    pub fn new() -> Self {
        Self {
            reducer: TurnReducerSeam,
            terminals: std::sync::Mutex::new(std::collections::HashMap::new()),
            emitted: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Start a turn: mint the canonical TurnId, apply Start through the
    /// reducer, and emit TurnStarted.  The Runtime assigns the TurnId.
    pub async fn start_turn(&self, session: &SessionId) -> Result<TurnId, RuntimeError> {
        let turn = TurnId(format!("turn-{}", session.0));
        let transition = TurnTransition::Start {
            session: session.clone(),
            turn: turn.clone(),
        };
        match self.reducer.apply(None, &transition)? {
            TransitionOutcome::Pending { .. } => {
                self.emitted
                    .lock()
                    .unwrap()
                    .push(RuntimeEvent::TurnStarted {
                        session: session.clone(),
                        turn: turn.clone(),
                    });
                Ok(turn)
            }
            _ => Err(RuntimeError::Internal),
        }
    }

    /// Settle a turn to a typed terminal through the reducer.  A duplicate
    /// settle is rejected (AlreadyTerminal); a late receipt is observation
    /// only, never a terminal reversal.
    pub async fn settle(
        &self,
        session: &SessionId,
        turn: &TurnId,
        terminal: TurnTerminal,
    ) -> Result<(), RuntimeError> {
        let current = self.terminals.lock().unwrap().get(turn).cloned();
        let transition = TurnTransition::Settle {
            session: session.clone(),
            turn: turn.clone(),
            terminal: terminal.clone(),
        };
        match self.reducer.apply(current.as_ref(), &transition)? {
            TransitionOutcome::Terminal { terminal: t, .. } => {
                self.terminals
                    .lock()
                    .unwrap()
                    .insert(turn.clone(), t.clone());
                self.emitted
                    .lock()
                    .unwrap()
                    .push(RuntimeEvent::TurnSettled {
                        session: session.clone(),
                        turn: turn.clone(),
                        terminal: t,
                    });
                Ok(())
            }
            TransitionOutcome::Pending { .. } => Err(RuntimeError::Internal),
        }
    }

    /// Typed events emitted so far (test/observability).
    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.emitted.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimeCommandPort for RuntimeTurnWriter {
    async fn dispatch(&self, command: RuntimeCommand) -> Result<CommandReceipt, RuntimeError> {
        match command {
            RuntimeCommand::StartTurn(StartTurnCommand { session, content }) => {
                let _ = content;
                let turn = self.start_turn(&session).await?;
                Ok(CommandReceipt {
                    session: Some(session),
                    turn: Some(turn),
                    agent_run: None,
                    generation: None,
                })
            }
            // RA-04 writer owns turn start; session create is RA-03.
            _ => Err(RuntimeError::UnknownSchema),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writer_mints_turn_and_settles_through_the_single_fence() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        assert!(turn.0.starts_with("turn-s1"));

        writer
            .settle(&session, &turn, TurnTerminal::Completed)
            .await
            .unwrap();
        let events = writer.events();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(&events[1], RuntimeEvent::TurnSettled { .. }));
    }

    #[tokio::test]
    async fn duplicate_settle_is_rejected_no_terminal_reversal() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        writer
            .settle(&session, &turn, TurnTerminal::Completed)
            .await
            .unwrap();
        // A late Failed settle is rejected — terminal is not reversed.
        assert_eq!(
            writer
                .settle(
                    &session,
                    &turn,
                    TurnTerminal::Failed {
                        message: "late".into()
                    }
                )
                .await,
            Err(RuntimeError::AlreadyTerminal)
        );
        // Still exactly one TurnSettled.
        let settles = writer
            .events()
            .into_iter()
            .filter(|e| matches!(e, RuntimeEvent::TurnSettled { .. }))
            .count();
        assert_eq!(settles, 1);
    }
}
