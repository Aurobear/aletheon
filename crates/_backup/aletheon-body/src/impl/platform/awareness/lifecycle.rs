//! Agent lifecycle state machine.
//!
//! Manages agent state transitions: Starting -> Running -> Paused/Degraded -> Stopping -> Stopped/Crashed.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Status of an agent in the lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentStatus {
    /// Agent is initializing.
    Starting,
    /// Agent is fully operational.
    Running,
    /// Agent is temporarily paused (e.g., resource contention).
    Paused,
    /// Agent is running with reduced functionality.
    Degraded,
    /// Agent is shutting down gracefully.
    Stopping,
    /// Agent has stopped normally.
    Stopped,
    /// Agent has crashed unexpectedly.
    Crashed,
}

impl AgentStatus {
    /// Returns true if the agent is in a state where it can accept work.
    pub fn is_operational(&self) -> bool {
        matches!(self, AgentStatus::Running | AgentStatus::Degraded)
    }

    /// Returns true if the agent is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentStatus::Stopped | AgentStatus::Crashed)
    }

    /// Returns true if the agent is transitioning (not stable).
    pub fn is_transitioning(&self) -> bool {
        matches!(self, AgentStatus::Starting | AgentStatus::Stopping)
    }
}

/// A state transition event in the agent lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition {
    /// The state we're transitioning from.
    pub from: AgentStatus,
    /// The state we're transitioning to.
    pub to: AgentStatus,
    /// Optional reason for the transition.
    pub reason: Option<String>,
}

/// Manages the lifecycle state machine for a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLifecycle {
    /// Current status.
    current: AgentStatus,
    /// History of state transitions.
    history: Vec<StateTransition>,
}

impl AgentLifecycle {
    /// Create a new lifecycle in the Starting state.
    pub fn new() -> Self {
        Self {
            current: AgentStatus::Starting,
            history: Vec::new(),
        }
    }

    /// Get the current status.
    pub fn status(&self) -> AgentStatus {
        self.current
    }

    /// Get the transition history.
    pub fn history(&self) -> &[StateTransition] {
        &self.history
    }

    /// Attempt a state transition.
    ///
    /// Returns Ok(()) if the transition was valid and applied.
    /// Returns Err with a description if the transition is invalid.
    pub fn transition(&mut self, to: AgentStatus) -> Result<(), String> {
        let from = self.current;

        if !is_valid_transition(from, to) {
            let msg = format!("Invalid transition: {:?} -> {:?}", from, to);
            warn!("{}", msg);
            return Err(msg);
        }

        let transition = StateTransition {
            from,
            to,
            reason: None,
        };

        debug!("Agent lifecycle: {:?} -> {:?}", from, to);
        self.current = to;
        self.history.push(transition);
        Ok(())
    }

    /// Attempt a state transition with a reason.
    pub fn transition_with_reason(
        &mut self,
        to: AgentStatus,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        let from = self.current;

        if !is_valid_transition(from, to) {
            let msg = format!("Invalid transition: {:?} -> {:?}", from, to);
            warn!("{}", msg);
            return Err(msg);
        }

        let reason_str = reason.into();
        info!("Agent lifecycle: {:?} -> {:?} ({})", from, to, reason_str);

        let transition = StateTransition {
            from,
            to,
            reason: Some(reason_str),
        };

        self.current = to;
        self.history.push(transition);
        Ok(())
    }

    /// Convenience: mark as running (from Starting).
    pub fn start(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Running)
    }

    /// Convenience: pause the agent (from Running).
    pub fn pause(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Paused)
    }

    /// Convenience: resume from pause (to Running).
    pub fn resume(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Running)
    }

    /// Convenience: begin graceful shutdown.
    pub fn stop(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Stopping)
    }

    /// Convenience: mark as stopped (from Stopping).
    pub fn finish_stop(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Stopped)
    }

    /// Convenience: mark as crashed (from any non-terminal state).
    pub fn crash(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Crashed)
    }

    /// Convenience: mark as degraded (from Running).
    pub fn degrade(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Degraded)
    }

    /// Convenience: recover from degraded to running.
    pub fn recover(&mut self) -> Result<(), String> {
        self.transition(AgentStatus::Running)
    }
}

impl Default for AgentLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a state transition is valid.
fn is_valid_transition(from: AgentStatus, to: AgentStatus) -> bool {
    use AgentStatus::*;
    matches!(
        (from, to),
        // From Starting
        (Starting, Running)
            | (Starting, Crashed)
            // From Running
            | (Running, Paused)
            | (Running, Degraded)
            | (Running, Stopping)
            | (Running, Crashed)
            // From Paused
            | (Paused, Running)
            | (Paused, Stopping)
            | (Paused, Crashed)
            // From Degraded
            | (Degraded, Running)
            | (Degraded, Stopping)
            | (Degraded, Crashed)
            // From Stopping
            | (Stopping, Stopped)
            | (Stopping, Crashed)
            // Stopped and Crashed are terminal — no transitions out
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let lifecycle = AgentLifecycle::new();
        assert_eq!(lifecycle.status(), AgentStatus::Starting);
        assert!(lifecycle.history().is_empty());
    }

    #[test]
    fn test_full_lifecycle() {
        let mut lc = AgentLifecycle::new();

        lc.start().unwrap();
        assert_eq!(lc.status(), AgentStatus::Running);
        assert!(lc.status().is_operational());

        lc.pause().unwrap();
        assert_eq!(lc.status(), AgentStatus::Paused);
        assert!(!lc.status().is_operational());

        lc.resume().unwrap();
        assert_eq!(lc.status(), AgentStatus::Running);

        lc.stop().unwrap();
        assert_eq!(lc.status(), AgentStatus::Stopping);
        assert!(lc.status().is_transitioning());

        lc.finish_stop().unwrap();
        assert_eq!(lc.status(), AgentStatus::Stopped);
        assert!(lc.status().is_terminal());

        assert_eq!(lc.history().len(), 5);
    }

    #[test]
    fn test_degraded_path() {
        let mut lc = AgentLifecycle::new();

        lc.start().unwrap();
        lc.degrade().unwrap();
        assert_eq!(lc.status(), AgentStatus::Degraded);
        assert!(lc.status().is_operational()); // Degraded is still operational

        lc.recover().unwrap();
        assert_eq!(lc.status(), AgentStatus::Running);
    }

    #[test]
    fn test_crash_from_running() {
        let mut lc = AgentLifecycle::new();
        lc.start().unwrap();
        lc.crash().unwrap();
        assert_eq!(lc.status(), AgentStatus::Crashed);
        assert!(lc.status().is_terminal());
    }

    #[test]
    fn test_invalid_transition() {
        let mut lc = AgentLifecycle::new();

        // Cannot go from Starting directly to Stopping
        let result = lc.transition(AgentStatus::Stopping);
        assert!(result.is_err());
        assert_eq!(lc.status(), AgentStatus::Starting); // unchanged
    }

    #[test]
    fn test_terminal_state_no_transitions() {
        let mut lc = AgentLifecycle::new();
        lc.start().unwrap();
        lc.stop().unwrap();
        lc.finish_stop().unwrap();

        // Stopped is terminal — no transitions allowed
        assert!(lc.transition(AgentStatus::Running).is_err());
        assert!(lc.transition(AgentStatus::Starting).is_err());
        assert!(lc.transition(AgentStatus::Crashed).is_err());
    }

    #[test]
    fn test_transition_with_reason() {
        let mut lc = AgentLifecycle::new();
        lc.start().unwrap();
        lc.transition_with_reason(AgentStatus::Degraded, "high memory usage")
            .unwrap();

        assert_eq!(lc.status(), AgentStatus::Degraded);
        let last = lc.history().last().unwrap();
        assert_eq!(last.reason.as_deref(), Some("high memory usage"));
    }

    #[test]
    fn test_status_properties() {
        // Starting is transitioning, not operational, not terminal
        assert!(AgentStatus::Starting.is_transitioning());
        assert!(!AgentStatus::Starting.is_operational());
        assert!(!AgentStatus::Starting.is_terminal());

        // Running is operational, not transitioning, not terminal
        assert!(AgentStatus::Running.is_operational());
        assert!(!AgentStatus::Running.is_transitioning());
        assert!(!AgentStatus::Running.is_terminal());

        // Stopped is terminal
        assert!(AgentStatus::Stopped.is_terminal());
        assert!(!AgentStatus::Stopped.is_operational());
    }

    #[test]
    fn test_crash_from_starting() {
        let mut lc = AgentLifecycle::new();
        lc.crash().unwrap();
        assert_eq!(lc.status(), AgentStatus::Crashed);
    }
}
