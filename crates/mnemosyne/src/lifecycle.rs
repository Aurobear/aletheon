//! Pure operation lifecycle for local-first memory orchestration.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOperationState {
    Ready,
    LocalWrite,
    Projection,
    SupplementalWrite,
    LocalRecall,
    SupplementalRecall,
    Merging,
    MergingDegraded,
    Reconciliation,
    Retention,
    Completed,
    Degraded,
    Failed,
}

impl MemoryOperationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Degraded | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOperationEvent {
    BeginWrite,
    LocalWriteFinished,
    ProjectionFinished,
    SupplementalWritten,
    SupplementalSkipped,
    BeginRecall,
    LocalRecallFinished,
    SupplementalRecalled,
    SupplementalRecallSkipped,
    SupplementalRecallDegraded,
    MergeFinished,
    BeginReconciliation,
    ReconciliationFinished,
    BeginRetention,
    RetentionFinished,
    Degrade,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOperationEffect {
    WriteLocal,
    ProjectLocal,
    WriteSupplemental,
    RecallLocal,
    RecallSupplemental,
    MergeRecall,
    ReconcileSupplemental,
    ApplyRetention,
    ReportCompleted,
    ReportDegraded,
    ReportFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryOperationTransition {
    pub previous: MemoryOperationState,
    pub next_state: MemoryOperationState,
    pub effects: Vec<MemoryOperationEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidMemoryOperationTransition {
    pub previous: MemoryOperationState,
    pub event: MemoryOperationEvent,
}

impl fmt::Display for InvalidMemoryOperationTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid memory transition {:?} + {:?}",
            self.previous, self.event
        )
    }
}
impl std::error::Error for InvalidMemoryOperationTransition {}

pub fn reduce_memory_operation(
    previous: MemoryOperationState,
    event: MemoryOperationEvent,
) -> Result<MemoryOperationTransition, InvalidMemoryOperationTransition> {
    use MemoryOperationEffect::*;
    use MemoryOperationEvent::*;
    use MemoryOperationState::*;
    let (next_state, effects) = match (previous, event) {
        (Ready, BeginWrite) => (LocalWrite, vec![WriteLocal]),
        (LocalWrite, LocalWriteFinished) => (Projection, vec![ProjectLocal]),
        (Projection, ProjectionFinished) => (SupplementalWrite, vec![WriteSupplemental]),
        (SupplementalWrite, SupplementalWritten | SupplementalSkipped) => {
            (Completed, vec![ReportCompleted])
        }
        (Ready, BeginRecall) => (LocalRecall, vec![RecallLocal]),
        (LocalRecall, LocalRecallFinished) => (SupplementalRecall, vec![RecallSupplemental]),
        (SupplementalRecall, SupplementalRecalled) => (Merging, vec![MergeRecall]),
        (SupplementalRecall, SupplementalRecallSkipped) => (Completed, vec![ReportCompleted]),
        (SupplementalRecall, SupplementalRecallDegraded) => (MergingDegraded, vec![MergeRecall]),
        (Merging, MergeFinished) => (Completed, vec![ReportCompleted]),
        (MergingDegraded, MergeFinished) => (Degraded, vec![ReportDegraded]),
        (Ready, BeginReconciliation) => (Reconciliation, vec![ReconcileSupplemental]),
        (Reconciliation, ReconciliationFinished) => (Completed, vec![ReportCompleted]),
        (Ready, BeginRetention) => (Retention, vec![ApplyRetention]),
        (Retention, RetentionFinished) => (Completed, vec![ReportCompleted]),
        (state, Degrade) if !state.is_terminal() => (Degraded, vec![ReportDegraded]),
        (state, Fail) if !state.is_terminal() => (Failed, vec![ReportFailed]),
        _ => return Err(InvalidMemoryOperationTransition { previous, event }),
    };
    Ok(MemoryOperationTransition {
        previous,
        next_state,
        effects,
    })
}

#[derive(Debug, Default)]
pub struct MemoryOperationLifecycle {
    state: MemoryOperationState,
}

impl Default for MemoryOperationState {
    fn default() -> Self {
        Self::Ready
    }
}

impl MemoryOperationLifecycle {
    pub fn state(&self) -> MemoryOperationState {
        self.state
    }
    pub fn apply(
        &mut self,
        event: MemoryOperationEvent,
    ) -> Result<MemoryOperationTransition, InvalidMemoryOperationTransition> {
        let transition = reduce_memory_operation(self.state, event)?;
        self.state = transition.next_state;
        Ok(transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[MemoryOperationEvent]) -> MemoryOperationState {
        let mut lifecycle = MemoryOperationLifecycle::default();
        for event in events {
            lifecycle.apply(*event).unwrap();
        }
        lifecycle.state()
    }

    #[test]
    fn characterizes_write_recall_reconciliation_and_retention_paths() {
        use MemoryOperationEvent::*;
        assert_eq!(
            run(&[
                BeginWrite,
                LocalWriteFinished,
                ProjectionFinished,
                SupplementalWritten
            ]),
            MemoryOperationState::Completed
        );
        assert_eq!(
            run(&[
                BeginWrite,
                LocalWriteFinished,
                ProjectionFinished,
                SupplementalSkipped
            ]),
            MemoryOperationState::Completed
        );
        assert_eq!(
            run(&[
                BeginRecall,
                LocalRecallFinished,
                SupplementalRecalled,
                MergeFinished
            ]),
            MemoryOperationState::Completed
        );
        assert_eq!(
            run(&[BeginRecall, LocalRecallFinished, SupplementalRecallSkipped]),
            MemoryOperationState::Completed
        );
        assert_eq!(
            run(&[
                BeginRecall,
                LocalRecallFinished,
                SupplementalRecallDegraded,
                MergeFinished
            ]),
            MemoryOperationState::Degraded
        );
        assert_eq!(
            run(&[BeginReconciliation, ReconciliationFinished]),
            MemoryOperationState::Completed
        );
        assert_eq!(
            run(&[BeginRetention, RetentionFinished]),
            MemoryOperationState::Completed
        );
    }

    #[test]
    fn failure_and_degradation_are_terminal_and_repeated_events_fail_closed() {
        for state in [
            MemoryOperationState::Ready,
            MemoryOperationState::LocalWrite,
            MemoryOperationState::Projection,
            MemoryOperationState::SupplementalWrite,
            MemoryOperationState::LocalRecall,
            MemoryOperationState::SupplementalRecall,
            MemoryOperationState::Merging,
            MemoryOperationState::MergingDegraded,
            MemoryOperationState::Reconciliation,
            MemoryOperationState::Retention,
        ] {
            assert_eq!(
                reduce_memory_operation(state, MemoryOperationEvent::Fail)
                    .unwrap()
                    .next_state,
                MemoryOperationState::Failed
            );
            assert_eq!(
                reduce_memory_operation(state, MemoryOperationEvent::Degrade)
                    .unwrap()
                    .next_state,
                MemoryOperationState::Degraded
            );
        }
        for terminal in [
            MemoryOperationState::Completed,
            MemoryOperationState::Degraded,
            MemoryOperationState::Failed,
        ] {
            assert!(reduce_memory_operation(terminal, MemoryOperationEvent::Fail).is_err());
            assert!(reduce_memory_operation(terminal, MemoryOperationEvent::BeginWrite).is_err());
        }
        assert!(reduce_memory_operation(
            MemoryOperationState::Ready,
            MemoryOperationEvent::MergeFinished
        )
        .is_err());
    }
}
