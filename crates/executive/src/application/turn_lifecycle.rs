//! Pure lifecycle reducer for the canonical turn pipeline.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPipelineState {
    Admission,
    PreTurn,
    CognitiveExecution,
    ToolLoop,
    PostTurn,
    Projection,
    Completed,
    Failed,
    Cancelled,
}

impl TurnPipelineState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPipelineEvent {
    Admit,
    ContextPrepared,
    ToolLoopStarted,
    ExecutionFinished,
    PostTurnSettled,
    ProjectionFinished,
    Fail,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPipelineEffect {
    RunPreTurn,
    RunCognitiveExecution,
    PumpToolEvents,
    RunPostTurn,
    PersistProjection,
    PublishCompletion,
    PublishFailure,
    PublishCancellation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnPipelineTransition {
    pub previous: TurnPipelineState,
    pub next_state: TurnPipelineState,
    pub effects: Vec<TurnPipelineEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTurnPipelineTransition {
    pub previous: TurnPipelineState,
    pub event: TurnPipelineEvent,
}

impl fmt::Display for InvalidTurnPipelineTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid turn transition {:?} + {:?}",
            self.previous, self.event
        )
    }
}
impl std::error::Error for InvalidTurnPipelineTransition {}

pub fn reduce_turn_pipeline(
    previous: TurnPipelineState,
    event: TurnPipelineEvent,
) -> Result<TurnPipelineTransition, InvalidTurnPipelineTransition> {
    use TurnPipelineEffect::*;
    use TurnPipelineEvent::*;
    use TurnPipelineState::*;
    let (next_state, effects) = match (previous, event) {
        (Admission, Admit) => (PreTurn, vec![RunPreTurn]),
        (PreTurn, ContextPrepared) => (CognitiveExecution, vec![RunCognitiveExecution]),
        (CognitiveExecution, ToolLoopStarted) => (ToolLoop, vec![PumpToolEvents]),
        (ToolLoop, ExecutionFinished) => (PostTurn, vec![RunPostTurn]),
        (PostTurn, PostTurnSettled) => (Projection, vec![PersistProjection]),
        (Projection, ProjectionFinished) => (Completed, vec![PublishCompletion]),
        (state, Fail) if !state.is_terminal() => (Failed, vec![PublishFailure]),
        (state, Cancel) if !state.is_terminal() => (Cancelled, vec![PublishCancellation]),
        _ => return Err(InvalidTurnPipelineTransition { previous, event }),
    };
    Ok(TurnPipelineTransition {
        previous,
        next_state,
        effects,
    })
}

/// The pipeline's single in-memory lifecycle mutation entry point.
#[derive(Debug)]
pub struct TurnPipelineLifecycle {
    state: TurnPipelineState,
}

impl Default for TurnPipelineLifecycle {
    fn default() -> Self {
        Self {
            state: TurnPipelineState::Admission,
        }
    }
}

impl TurnPipelineLifecycle {
    pub fn state(&self) -> TurnPipelineState {
        self.state
    }

    pub fn apply(
        &mut self,
        event: TurnPipelineEvent,
    ) -> Result<TurnPipelineTransition, InvalidTurnPipelineTransition> {
        let transition = reduce_turn_pipeline(self.state, event)?;
        self.state = transition.next_state;
        Ok(transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characterizes_the_complete_success_path() {
        let mut lifecycle = TurnPipelineLifecycle::default();
        for (event, expected) in [
            (TurnPipelineEvent::Admit, TurnPipelineState::PreTurn),
            (
                TurnPipelineEvent::ContextPrepared,
                TurnPipelineState::CognitiveExecution,
            ),
            (
                TurnPipelineEvent::ToolLoopStarted,
                TurnPipelineState::ToolLoop,
            ),
            (
                TurnPipelineEvent::ExecutionFinished,
                TurnPipelineState::PostTurn,
            ),
            (
                TurnPipelineEvent::PostTurnSettled,
                TurnPipelineState::Projection,
            ),
            (
                TurnPipelineEvent::ProjectionFinished,
                TurnPipelineState::Completed,
            ),
        ] {
            assert_eq!(lifecycle.apply(event).unwrap().next_state, expected);
        }
    }

    #[test]
    fn fail_and_cancel_are_terminal_from_every_active_state() {
        let active = [
            TurnPipelineState::Admission,
            TurnPipelineState::PreTurn,
            TurnPipelineState::CognitiveExecution,
            TurnPipelineState::ToolLoop,
            TurnPipelineState::PostTurn,
            TurnPipelineState::Projection,
        ];
        for state in active {
            assert_eq!(
                reduce_turn_pipeline(state, TurnPipelineEvent::Fail)
                    .unwrap()
                    .next_state,
                TurnPipelineState::Failed
            );
            assert_eq!(
                reduce_turn_pipeline(state, TurnPipelineEvent::Cancel)
                    .unwrap()
                    .next_state,
                TurnPipelineState::Cancelled
            );
        }
    }

    #[test]
    fn rejects_out_of_order_repeated_and_post_terminal_events() {
        assert!(reduce_turn_pipeline(
            TurnPipelineState::Admission,
            TurnPipelineEvent::ToolLoopStarted
        )
        .is_err());
        assert!(
            reduce_turn_pipeline(TurnPipelineState::PreTurn, TurnPipelineEvent::Admit).is_err()
        );
        for state in [
            TurnPipelineState::Completed,
            TurnPipelineState::Failed,
            TurnPipelineState::Cancelled,
        ] {
            for event in [
                TurnPipelineEvent::Admit,
                TurnPipelineEvent::ContextPrepared,
                TurnPipelineEvent::ToolLoopStarted,
                TurnPipelineEvent::ExecutionFinished,
                TurnPipelineEvent::PostTurnSettled,
                TurnPipelineEvent::ProjectionFinished,
                TurnPipelineEvent::Fail,
                TurnPipelineEvent::Cancel,
            ] {
                assert!(reduce_turn_pipeline(state, event).is_err());
            }
        }
    }
}
