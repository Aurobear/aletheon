//! RA-04 PR-C Runtime Turn writer (Agent Kernel V2).
//!
//! The deployable Turn writer: the Runtime mints the canonical `TurnId`, runs
//! every cancel/timeout/disconnect/late-receipt/crash through the single
//! `TurnReducerSeam` terminal fence, and emits typed `TurnStarted`/`TurnSettled`
//! events. The host coordinator is retained as an execution facade, while
//! Runtime owns the canonical turn identity and terminal fence here.

use crate::command::{CommandReceipt, RuntimeCommand, StartTurnCommand};
use crate::error::RuntimeError;
use crate::event::{RuntimeEvent, TurnTerminal};
use crate::ids::{SessionId, TurnId};
use crate::ports::RuntimeCommandPort;
use crate::turn_reducer::{TransitionOutcome, TurnReducerSeam, TurnTransition};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

/// Runtime Turn writer over the canonical Turn reducer + a typed event sink.
pub struct RuntimeTurnWriter {
    reducer: TurnReducerSeam,
    sink: Option<std::sync::Arc<dyn crate::TurnEventSink>>,
    /// Current terminal per turn (single terminal source).
    terminals: std::sync::Mutex<std::collections::HashMap<TurnId, TurnTerminal>>,
    /// Serialize the append-and-fence critical section. Checking the in-memory
    /// terminal map before an async journal append is otherwise racy when a
    /// normal completion and a disconnect recovery settle concurrently.
    terminal_write_lock: tokio::sync::Mutex<()>,
    started: std::sync::Mutex<std::collections::HashMap<TurnId, SessionId>>,
    /// Typed events emitted (in a real composition this is the RuntimeEventPort
    /// journal; here a test-observable vector).
    emitted: std::sync::Mutex<Vec<RuntimeEvent>>,
    next_stream_sequence: AtomicU64,
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
            sink: None,
            terminals: std::sync::Mutex::new(std::collections::HashMap::new()),
            terminal_write_lock: tokio::sync::Mutex::new(()),
            started: std::sync::Mutex::new(std::collections::HashMap::new()),
            emitted: std::sync::Mutex::new(Vec::new()),
            next_stream_sequence: AtomicU64::new(1),
        }
    }

    pub fn with_event_sink(mut self, sink: std::sync::Arc<dyn crate::TurnEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    async fn append_event(&self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        let Some(sink) = &self.sink else {
            return Ok(());
        };
        let sequence = self.next_stream_sequence.fetch_add(1, Ordering::Relaxed);
        sink.append(crate::TurnStreamEvent::new(sequence, event))
            .await
    }

    /// Start a turn: mint the canonical TurnId, apply Start through the
    /// reducer, and emit TurnStarted.  The Runtime assigns the TurnId.
    pub async fn start_turn(&self, session: &SessionId) -> Result<TurnId, RuntimeError> {
        // Runtime owns the mint. A UUID prevents a second turn in one session
        // (or a daemon restart) from reusing an old turn identity. The UUID
        // representation remains compatible with fabric's durable TurnId.
        let turn = TurnId(format!("{}", uuid::Uuid::new_v4()));
        let transition = TurnTransition::Start {
            session: session.clone(),
            turn: turn.clone(),
        };
        match self.reducer.apply(None, &transition)? {
            TransitionOutcome::Pending { .. } => {
                self.append_event(RuntimeEvent::TurnStarted {
                    session: session.clone(),
                    turn: turn.clone(),
                })
                .await?;
                self.emitted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(RuntimeEvent::TurnStarted {
                        session: session.clone(),
                        turn: turn.clone(),
                    });
                self.started
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(turn.clone(), session.clone());
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
        let _write_guard = self.terminal_write_lock.lock().await;
        let started_session = self
            .started
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(turn)
            .cloned()
            .ok_or(RuntimeError::TurnNotFound)?;
        if started_session != *session {
            return Err(RuntimeError::WrongSession);
        }
        let current = self
            .terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(turn)
            .cloned();
        if current.as_ref() == Some(&terminal) {
            // The host execution facade may retry the same settlement after
            // the Runtime writer already fenced it. Idempotent same-terminal
            // acknowledgement is safe; a different terminal still fails
            // closed below.
            return Ok(());
        }
        let transition = TurnTransition::Settle {
            session: session.clone(),
            turn: turn.clone(),
            terminal: terminal.clone(),
        };
        match self.reducer.apply(current.as_ref(), &transition)? {
            TransitionOutcome::Terminal { terminal: t, .. } => {
                self.append_event(RuntimeEvent::TurnSettled {
                    session: session.clone(),
                    turn: turn.clone(),
                    terminal: t.clone(),
                })
                .await?;
                self.terminals
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(turn.clone(), t.clone());
                self.emitted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    /// Apply a non-settlement lifecycle transition through the same reducer
    /// and terminal fence. Cancel/timeout/disconnect/crash before terminal
    /// remain pending observations; late receipts after terminal never create
    /// a second terminal and are journaled as observations.
    pub async fn apply_transition(
        &self,
        session: &SessionId,
        turn: &TurnId,
        transition: TurnTransition,
    ) -> Result<TransitionOutcome, RuntimeError> {
        // Observations share the same append-and-fence critical section as a
        // terminal settlement.  Without this guard a disconnect/late receipt
        // could append concurrently with `settle`, making the durable event
        // order depend on whichever sink future happened to complete first.
        // The reducer remains the sole semantic fence; this lock only makes
        // its journal publication order deterministic.
        let _write_guard = self.terminal_write_lock.lock().await;
        let (transition_session, transition_turn) = match &transition {
            TurnTransition::Start { session, turn }
            | TurnTransition::Cancel { session, turn }
            | TurnTransition::Timeout { session, turn }
            | TurnTransition::Disconnect { session, turn }
            | TurnTransition::LateReceipt { session, turn }
            | TurnTransition::Crash { session, turn }
            | TurnTransition::Settle { session, turn, .. } => (session, turn),
        };
        if transition_turn != turn {
            return Err(RuntimeError::TurnNotFound);
        }
        if transition_session != session {
            return Err(RuntimeError::WrongSession);
        }
        let started_session = self
            .started
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(turn)
            .cloned()
            .ok_or(RuntimeError::TurnNotFound)?;
        if started_session != *session {
            return Err(RuntimeError::WrongSession);
        }
        let current = self
            .terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(turn)
            .cloned();
        let outcome = self.reducer.apply(current.as_ref(), &transition)?;
        let kind = match &transition {
            TurnTransition::Cancel { .. } => "cancel",
            TurnTransition::Timeout { .. } => "timeout",
            TurnTransition::Disconnect { .. } => "disconnect",
            TurnTransition::Crash { .. } => "crash",
            TurnTransition::LateReceipt { .. } => "late_receipt",
            TurnTransition::Start { .. } | TurnTransition::Settle { .. } => {
                return Err(RuntimeError::UnsupportedRequest)
            }
        };
        self.append_event(RuntimeEvent::TurnObserved {
            session: session.clone(),
            turn: turn.clone(),
            kind: kind.into(),
        })
        .await?;
        self.emitted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RuntimeEvent::TurnObserved {
                session: session.clone(),
                turn: turn.clone(),
                kind: kind.into(),
            });
        Ok(outcome)
    }

    pub async fn cancel(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        self.apply_transition(
            session,
            turn,
            TurnTransition::Cancel {
                session: session.clone(),
                turn: turn.clone(),
            },
        )
        .await
    }

    pub async fn timeout(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        self.apply_transition(
            session,
            turn,
            TurnTransition::Timeout {
                session: session.clone(),
                turn: turn.clone(),
            },
        )
        .await
    }

    pub async fn disconnect(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        self.apply_transition(
            session,
            turn,
            TurnTransition::Disconnect {
                session: session.clone(),
                turn: turn.clone(),
            },
        )
        .await
    }

    pub async fn late_receipt(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        self.apply_transition(
            session,
            turn,
            TurnTransition::LateReceipt {
                session: session.clone(),
                turn: turn.clone(),
            },
        )
        .await
    }

    /// Record a host crash observation through the same reducer used by
    /// cancellation, timeout, and disconnect. The observation is deliberately
    /// separate from terminal settlement so recovery can distinguish a crash
    /// from an ordinary interrupted request.
    pub async fn crash(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        self.apply_transition(
            session,
            turn,
            TurnTransition::Crash {
                session: session.clone(),
                turn: turn.clone(),
            },
        )
        .await
    }

    /// Typed events emitted so far (test/observability).
    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.emitted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Reconcile a replayed Runtime turn stream after restart. A started turn
    /// without a durable terminal is fenced as Interrupted; no transport EOF
    /// or missing in-memory entry is treated as successful completion.
    pub async fn recover_from_events(
        &self,
        events: impl IntoIterator<Item = RuntimeEvent>,
    ) -> Result<usize, RuntimeError> {
        let mut started = std::collections::HashMap::<TurnId, SessionId>::new();
        let mut settled = std::collections::HashMap::<TurnId, (SessionId, TurnTerminal)>::new();
        for event in events {
            match event {
                RuntimeEvent::TurnStarted { session, turn, .. } => {
                    if let Some(existing) = started.get(&turn) {
                        if existing != &session {
                            return Err(RuntimeError::UnknownSchema);
                        }
                    }
                    started.insert(turn, session);
                }
                RuntimeEvent::TurnSettled {
                    session,
                    turn,
                    terminal,
                } => {
                    if let Some(existing) = settled.get(&turn) {
                        if existing != &(session.clone(), terminal.clone()) {
                            return Err(RuntimeError::UnknownSchema);
                        }
                    }
                    settled.insert(turn, (session, terminal));
                }
                _ => {}
            }
        }
        if settled
            .iter()
            .any(|(turn, (session, _))| started.get(turn) != Some(session))
        {
            return Err(RuntimeError::UnknownSchema);
        }
        let mut recovered = 0;
        for (turn, session) in started {
            self.started
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(turn.clone(), session.clone());
            if let Some((_, terminal)) = settled.get(&turn).cloned() {
                self.terminals
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(turn, terminal);
                continue;
            }
            if self
                .terminals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(&turn)
            {
                continue;
            }
            self.settle(&session, &turn, TurnTerminal::Interrupted)
                .await?;
            recovered += 1;
        }
        Ok(recovered)
    }

    pub async fn recover_from_stream(
        &self,
        events: impl IntoIterator<Item = crate::TurnStreamEvent>,
    ) -> Result<usize, RuntimeError> {
        let mut replay = Vec::new();
        let mut max_sequence = 0;
        let mut previous_sequence = 0;
        for event in events {
            event.validate().map_err(|_| RuntimeError::UnknownSchema)?;
            if event.sequence <= previous_sequence {
                return Err(RuntimeError::UnknownSchema);
            }
            previous_sequence = event.sequence;
            max_sequence = max_sequence.max(event.sequence);
            replay.push(event.into_event());
        }
        if max_sequence > 0 {
            self.next_stream_sequence
                .fetch_max(max_sequence.saturating_add(1), Ordering::Relaxed);
        }
        self.recover_from_events(replay).await
    }
}

#[async_trait]
impl crate::TurnLifecycleWriter for RuntimeTurnWriter {
    async fn start_turn(&self, session: &SessionId) -> Result<TurnId, RuntimeError> {
        Self::start_turn(self, session).await
    }

    async fn settle(
        &self,
        session: &SessionId,
        turn: &TurnId,
        terminal: TurnTerminal,
    ) -> Result<(), RuntimeError> {
        Self::settle(self, session, turn, terminal).await
    }

    async fn cancel(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        Self::cancel(self, session, turn).await
    }

    async fn timeout(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        Self::timeout(self, session, turn).await
    }

    async fn disconnect(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        Self::disconnect(self, session, turn).await
    }

    async fn late_receipt(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        Self::late_receipt(self, session, turn).await
    }

    async fn crash(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<TransitionOutcome, RuntimeError> {
        Self::crash(self, session, turn).await
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
    use crate::TurnStreamEvent;

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<crate::TurnStreamEvent>>);

    #[async_trait::async_trait]
    impl crate::TurnEventSink for RecordingSink {
        async fn append(&self, event: crate::TurnStreamEvent) -> Result<(), RuntimeError> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event);
            Ok(())
        }
    }

    struct ConcurrencySink {
        active: std::sync::atomic::AtomicUsize,
        max_active: std::sync::atomic::AtomicUsize,
    }

    impl ConcurrencySink {
        fn new() -> Self {
            Self {
                active: std::sync::atomic::AtomicUsize::new(0),
                max_active: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn max_active(&self) -> usize {
            self.max_active.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl crate::TurnEventSink for ConcurrencySink {
        async fn append(&self, _event: crate::TurnStreamEvent) -> Result<(), RuntimeError> {
            let active = self
                .active
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.max_active
                .fetch_max(active, std::sync::atomic::Ordering::SeqCst);
            // Keep the append future open long enough for an unguarded
            // observation path to overlap a terminal publication.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            self.active
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn writer_mints_turn_and_settles_through_the_single_fence() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        assert!(uuid::Uuid::parse_str(&turn.0).is_ok());

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
    async fn writer_mints_distinct_ids_for_multiple_turns_in_one_session() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let first = writer.start_turn(&session).await.unwrap();
        let second = writer.start_turn(&session).await.unwrap();
        assert_ne!(first, second);
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

    #[tokio::test]
    async fn lifecycle_observations_are_journaled_without_reversing_terminal() {
        let sink = std::sync::Arc::new(RecordingSink::default());
        let writer = RuntimeTurnWriter::new().with_event_sink(sink.clone());
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        let pending = writer.cancel(&session, &turn).await.unwrap();
        assert!(matches!(pending, TransitionOutcome::Pending { .. }));
        writer
            .settle(&session, &turn, TurnTerminal::Completed)
            .await
            .unwrap();
        let late = writer.late_receipt(&session, &turn).await.unwrap();
        assert!(matches!(late, TransitionOutcome::Terminal { .. }));
        let events = sink
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|event| event.validate().is_ok()));
        assert!(matches!(
            events[1].event.as_ref(),
            RuntimeEvent::TurnObserved { kind, .. } if kind == "cancel"
        ));
        assert!(matches!(
            events[3].event.as_ref(),
            RuntimeEvent::TurnObserved { kind, .. } if kind == "late_receipt"
        ));
    }

    #[tokio::test]
    async fn observation_publication_is_serialized_with_terminal_append() {
        let sink = std::sync::Arc::new(ConcurrencySink::new());
        let writer = RuntimeTurnWriter::new().with_event_sink(sink.clone());
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();

        let (settled, observed) = tokio::join!(
            writer.settle(&session, &turn, TurnTerminal::Completed),
            writer.late_receipt(&session, &turn),
        );
        settled.unwrap();
        observed.unwrap();
        assert_eq!(sink.max_active(), 1);
        assert_eq!(
            writer
                .events()
                .into_iter()
                .filter(|event| matches!(event, RuntimeEvent::TurnSettled { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn crash_observation_precedes_failed_terminal_settlement() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        let observation = writer.crash(&session, &turn).await.unwrap();
        assert!(matches!(observation, TransitionOutcome::Pending { .. }));
        writer
            .settle(
                &session,
                &turn,
                TurnTerminal::Failed {
                    message: "host crashed".into(),
                },
            )
            .await
            .unwrap();
        let events = writer.events();
        assert!(matches!(
            events.get(1),
            Some(RuntimeEvent::TurnObserved { kind, .. }) if kind == "crash"
        ));
        assert!(matches!(
            events.get(2),
            Some(RuntimeEvent::TurnSettled {
                terminal: TurnTerminal::Failed { .. },
                ..
            })
        ));
    }

    #[tokio::test]
    async fn stream_recovery_advances_turn_sequence() {
        let sink = std::sync::Arc::new(RecordingSink::default());
        let writer = RuntimeTurnWriter::new().with_event_sink(sink.clone());
        let session = SessionId("s1".into());
        let turn = TurnId("orphan".into());
        let event = crate::TurnStreamEvent::new(
            17,
            RuntimeEvent::TurnStarted {
                session: session.clone(),
                turn: turn.clone(),
            },
        );
        assert_eq!(writer.recover_from_stream([event]).await.unwrap(), 1);
        let events = sink
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(events[0].sequence, 18);
        assert!(matches!(
            events[0].event.as_ref(),
            RuntimeEvent::TurnSettled {
                terminal: TurnTerminal::Interrupted,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn turn_stream_replay_rejects_non_monotonic_sequences() {
        let writer = RuntimeTurnWriter::new();
        let first = TurnStreamEvent::new(
            4,
            RuntimeEvent::TurnStarted {
                session: SessionId("s1".into()),
                turn: TurnId("turn-sequence-1".into()),
            },
        );
        let duplicate = TurnStreamEvent::new(
            4,
            RuntimeEvent::TurnStarted {
                session: SessionId("s1".into()),
                turn: TurnId("turn-sequence-2".into()),
            },
        );
        assert_eq!(
            writer.recover_from_stream([first, duplicate]).await,
            Err(RuntimeError::UnknownSchema)
        );
    }

    #[tokio::test]
    async fn turn_writes_are_fenced_to_the_started_session_and_turn() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        assert_eq!(
            writer
                .settle(&SessionId("s2".into()), &turn, TurnTerminal::Completed,)
                .await,
            Err(RuntimeError::WrongSession)
        );
        assert_eq!(
            writer
                .apply_transition(
                    &session,
                    &turn,
                    TurnTransition::Cancel {
                        session: SessionId("s2".into()),
                        turn: turn.clone(),
                    },
                )
                .await,
            Err(RuntimeError::WrongSession)
        );
        assert_eq!(
            writer
                .apply_transition(
                    &session,
                    &turn,
                    TurnTransition::Cancel {
                        session: session.clone(),
                        turn: TurnId("other-turn".into()),
                    },
                )
                .await,
            Err(RuntimeError::TurnNotFound)
        );
    }

    #[tokio::test]
    async fn restart_recovery_fences_orphaned_turn_as_interrupted() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = TurnId("orphan".into());
        let recovered = writer
            .recover_from_events([RuntimeEvent::TurnStarted {
                session: session.clone(),
                turn: turn.clone(),
            }])
            .await
            .unwrap();
        assert_eq!(recovered, 1);
        assert!(matches!(
            writer.events().last(),
            Some(RuntimeEvent::TurnSettled {
                terminal: TurnTerminal::Interrupted,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn replayed_terminal_remains_fenced_after_restart() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = TurnId("settled".into());
        writer
            .recover_from_events([
                RuntimeEvent::TurnStarted {
                    session: session.clone(),
                    turn: turn.clone(),
                },
                RuntimeEvent::TurnSettled {
                    session: session.clone(),
                    turn: turn.clone(),
                    terminal: TurnTerminal::Completed,
                },
            ])
            .await
            .unwrap();
        assert_eq!(
            writer
                .settle(&session, &turn, TurnTerminal::Interrupted)
                .await,
            Err(RuntimeError::AlreadyTerminal)
        );
    }

    #[tokio::test]
    async fn replay_rejects_terminal_without_matching_start() {
        let writer = RuntimeTurnWriter::new();
        let result = writer
            .recover_from_events([RuntimeEvent::TurnSettled {
                session: SessionId("s1".into()),
                turn: TurnId("orphan-terminal".into()),
                terminal: TurnTerminal::Completed,
            }])
            .await;
        assert_eq!(result, Err(RuntimeError::UnknownSchema));
    }

    #[tokio::test]
    async fn replay_rejects_conflicting_duplicate_terminal() {
        let writer = RuntimeTurnWriter::new();
        let session = SessionId("s1".into());
        let turn = TurnId("duplicate-terminal".into());
        let result = writer
            .recover_from_events([
                RuntimeEvent::TurnStarted {
                    session: session.clone(),
                    turn: turn.clone(),
                },
                RuntimeEvent::TurnSettled {
                    session: session.clone(),
                    turn: turn.clone(),
                    terminal: TurnTerminal::Completed,
                },
                RuntimeEvent::TurnSettled {
                    session,
                    turn,
                    terminal: TurnTerminal::Interrupted,
                },
            ])
            .await;
        assert_eq!(result, Err(RuntimeError::UnknownSchema));
    }

    #[tokio::test]
    async fn replay_rejects_terminal_bound_to_a_different_session() {
        let writer = RuntimeTurnWriter::new();
        let turn = TurnId("wrong-session".into());
        let result = writer
            .recover_from_events([
                RuntimeEvent::TurnStarted {
                    session: SessionId("s1".into()),
                    turn: turn.clone(),
                },
                RuntimeEvent::TurnSettled {
                    session: SessionId("s2".into()),
                    turn,
                    terminal: TurnTerminal::Completed,
                },
            ])
            .await;
        assert_eq!(result, Err(RuntimeError::UnknownSchema));
    }

    #[tokio::test]
    async fn poisoned_lock_recovers_and_writer_keeps_serving() {
        let writer = RuntimeTurnWriter::new();

        // Poison the terminals map by panicking while its guard is held. A
        // later lock on the same mutex returns a PoisonError; the writer must
        // recover the guard instead of unwrapping and crashing the daemon.
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = writer.terminals.lock().unwrap();
            panic!("intentionally poison the terminals lock");
        }));
        assert!(poisoned.is_err());

        let session = SessionId("s1".into());
        let turn = writer.start_turn(&session).await.unwrap();
        // settle reads and writes the poisoned terminals lock; it must recover
        // rather than panic.
        writer
            .settle(&session, &turn, TurnTerminal::Completed)
            .await
            .unwrap();
        assert!(matches!(
            writer.events().last(),
            Some(RuntimeEvent::TurnSettled { .. })
        ));
    }
}
