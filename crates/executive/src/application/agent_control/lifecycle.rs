//! Pure Agent Control lifecycle transitions.

use fabric::AgentRunStatus;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLifecycleEvent {
    Start,
    Wait,
    Resume,
    Succeed,
    Fail,
    Cancel,
    Interrupt,
}

impl AgentLifecycleEvent {
    pub fn target(self) -> AgentRunStatus {
        match self {
            Self::Start | Self::Resume => AgentRunStatus::Running,
            Self::Wait => AgentRunStatus::Waiting,
            Self::Succeed => AgentRunStatus::Succeeded,
            Self::Fail => AgentRunStatus::Failed,
            Self::Cancel => AgentRunStatus::Cancelled,
            Self::Interrupt => AgentRunStatus::Interrupted,
        }
    }
    fn for_target(target: AgentRunStatus) -> Self {
        match target {
            AgentRunStatus::Running => Self::Start,
            AgentRunStatus::Waiting => Self::Wait,
            AgentRunStatus::Succeeded => Self::Succeed,
            AgentRunStatus::Failed => Self::Fail,
            AgentRunStatus::Cancelled => Self::Cancel,
            AgentRunStatus::Interrupted => Self::Interrupt,
            AgentRunStatus::Queued => Self::Start,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLifecycleEffect {
    PersistStatus,
    MarkStarted,
    MarkTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLifecycleTransition {
    pub previous: AgentRunStatus,
    pub next_state: AgentRunStatus,
    pub effects: Vec<AgentLifecycleEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAgentLifecycleTransition {
    pub previous: AgentRunStatus,
    pub event: AgentLifecycleEvent,
}

impl fmt::Display for InvalidAgentLifecycleTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal Agent transition {:?} -> {:?}",
            self.previous,
            self.event.target()
        )
    }
}
impl std::error::Error for InvalidAgentLifecycleTransition {}

pub fn reduce_agent_lifecycle(
    previous: AgentRunStatus,
    event: AgentLifecycleEvent,
) -> Result<AgentLifecycleTransition, InvalidAgentLifecycleTransition> {
    use AgentLifecycleEvent::{Cancel, Fail, Interrupt, Resume, Start, Succeed, Wait};
    use AgentRunStatus::{Queued, Running, Waiting};
    let valid = matches!(
        (previous, event),
        (Queued, Start)
            | (Queued, Cancel | Fail | Interrupt)
            | (Running, Wait | Succeed | Fail | Cancel | Interrupt)
            | (Waiting, Resume | Succeed | Fail | Cancel | Interrupt)
    );
    if !valid {
        return Err(InvalidAgentLifecycleTransition { previous, event });
    }
    let next_state = event.target();
    let mut effects = vec![AgentLifecycleEffect::PersistStatus];
    if previous == Queued && next_state == Running {
        effects.push(AgentLifecycleEffect::MarkStarted);
    }
    if next_state.is_terminal() {
        effects.push(AgentLifecycleEffect::MarkTerminal);
    }
    Ok(AgentLifecycleTransition {
        previous,
        next_state,
        effects,
    })
}

pub fn reduce_agent_status_transition(
    previous: AgentRunStatus,
    next: AgentRunStatus,
) -> Result<AgentLifecycleTransition, InvalidAgentLifecycleTransition> {
    let event = if previous == AgentRunStatus::Waiting && next == AgentRunStatus::Running {
        AgentLifecycleEvent::Resume
    } else {
        AgentLifecycleEvent::for_target(next)
    };
    let transition = reduce_agent_lifecycle(previous, event)?;
    if transition.next_state != next {
        return Err(InvalidAgentLifecycleTransition { previous, event });
    }
    Ok(transition)
}

#[cfg(test)]
mod tests {
    use super::*;
    const STATES: [AgentRunStatus; 7] = [
        AgentRunStatus::Queued,
        AgentRunStatus::Running,
        AgentRunStatus::Waiting,
        AgentRunStatus::Succeeded,
        AgentRunStatus::Failed,
        AgentRunStatus::Cancelled,
        AgentRunStatus::Interrupted,
    ];
    const VALID: [(AgentRunStatus, AgentRunStatus); 14] = [
        (AgentRunStatus::Queued, AgentRunStatus::Running),
        (AgentRunStatus::Queued, AgentRunStatus::Cancelled),
        (AgentRunStatus::Queued, AgentRunStatus::Failed),
        (AgentRunStatus::Queued, AgentRunStatus::Interrupted),
        (AgentRunStatus::Running, AgentRunStatus::Waiting),
        (AgentRunStatus::Waiting, AgentRunStatus::Running),
        (AgentRunStatus::Running, AgentRunStatus::Succeeded),
        (AgentRunStatus::Running, AgentRunStatus::Failed),
        (AgentRunStatus::Running, AgentRunStatus::Cancelled),
        (AgentRunStatus::Running, AgentRunStatus::Interrupted),
        (AgentRunStatus::Waiting, AgentRunStatus::Succeeded),
        (AgentRunStatus::Waiting, AgentRunStatus::Failed),
        (AgentRunStatus::Waiting, AgentRunStatus::Cancelled),
        (AgentRunStatus::Waiting, AgentRunStatus::Interrupted),
    ];
    #[test]
    fn characterizes_all_valid_and_invalid_transitions() {
        for previous in STATES {
            for next in STATES {
                assert_eq!(
                    reduce_agent_status_transition(previous, next).is_ok(),
                    VALID.contains(&(previous, next)),
                    "{previous:?} -> {next:?}"
                );
            }
        }
    }
    #[test]
    fn emits_boundary_timestamp_effects() {
        let start =
            reduce_agent_lifecycle(AgentRunStatus::Queued, AgentLifecycleEvent::Start).unwrap();
        assert!(start.effects.contains(&AgentLifecycleEffect::MarkStarted));
        assert!(!start.effects.contains(&AgentLifecycleEffect::MarkTerminal));
        let end =
            reduce_agent_lifecycle(AgentRunStatus::Running, AgentLifecycleEvent::Succeed).unwrap();
        assert!(end.effects.contains(&AgentLifecycleEffect::MarkTerminal));
    }
}
