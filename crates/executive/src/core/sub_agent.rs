//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call.
//! Their status is tracked and emitted to the TUI via UiEvent, and their
//! control-plane lifecycle is enforced via `SubAgentState`.

use fabric::ui_event::{SubAgentHandle, SubAgentStatus};
use fabric::SubAgentState;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

/// Error returned when an illegal lifecycle transition is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No agent with the given id is tracked.
    Unknown(String),
    /// The transition `from -> to` is not legal.
    Illegal {
        from: SubAgentState,
        to: SubAgentState,
    },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::Unknown(id) => write!(f, "unknown sub-agent: {id}"),
            TransitionError::Illegal { from, to } => {
                write!(f, "illegal transition {from:?} -> {to:?}")
            }
        }
    }
}
impl std::error::Error for TransitionError {}

/// Internal per-agent record: the UI handle, the control-plane state, and a
/// cancellation token for in-flight work.
#[derive(Debug)]
struct SubAgentEntry {
    handle: SubAgentHandle,
    state: SubAgentState,
    cancel: CancellationToken,
}

/// Spawns and tracks sub-agents.
#[derive(Debug, Default)]
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentEntry>,
    next_id: usize,
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a new sub-agent and return its handle. Starts in `Created`.
    pub fn spawn(&mut self, task: String, parent_turn_id: String) -> SubAgentHandle {
        self.next_id += 1;
        let id = format!("agent-{}", self.next_id);
        let handle = SubAgentHandle {
            id: id.clone(),
            task,
            status: SubAgentStatus::Planning,
            parent_turn_id,
            spawned_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.agents.insert(
            id,
            SubAgentEntry {
                handle: handle.clone(),
                state: SubAgentState::Created,
                cancel: CancellationToken::new(),
            },
        );
        handle
    }

    /// Update an agent's UI status (unchanged UI-display behavior).
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(entry) = self.agents.get_mut(id) {
            entry.handle.status = status;
        }
    }

    /// Current control-plane state of an agent, if tracked.
    pub fn state(&self, id: &str) -> Option<SubAgentState> {
        self.agents.get(id).map(|e| e.state)
    }

    /// A clone of the agent's cancellation token (for wiring into spawned work).
    pub fn cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.agents.get(id).map(|e| e.cancel.clone())
    }

    /// Attempt a legal-only lifecycle transition.
    pub fn transition(&mut self, id: &str, next: SubAgentState) -> Result<(), TransitionError> {
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| TransitionError::Unknown(id.to_string()))?;
        if entry.state.can_transition_to(&next) {
            entry.state = next;
            Ok(())
        } else {
            Err(TransitionError::Illegal {
                from: entry.state,
                to: next,
            })
        }
    }

    /// Tear an agent down: cancel its in-flight work, drop its handle, free the
    /// map slot. Returns `false` if no such agent was tracked (idempotent).
    pub fn destroy(&mut self, id: &str) -> bool {
        match self.agents.remove(id) {
            Some(entry) => {
                entry.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Remove a completed/failed agent (delegates to `destroy` for teardown).
    pub fn remove(&mut self, id: &str) -> bool {
        self.destroy(id)
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().map(|e| &e.handle).collect()
    }

    /// Get a specific agent's handle.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id).map(|e| &e.handle)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use fabric::SubAgentState;

    #[test]
    fn spawn_starts_in_created_and_legal_transitions_advance() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
        assert!(s.transition(&h.id, SubAgentState::Running).is_ok());
        assert!(s.transition(&h.id, SubAgentState::Waiting).is_ok());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Waiting));
    }

    #[test]
    fn illegal_transition_is_rejected_and_state_unchanged() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        // Created -> Completed is illegal (must Run first).
        assert!(s.transition(&h.id, SubAgentState::Completed).is_err());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
    }

    #[tokio::test]
    async fn destroy_cancels_in_flight_work_and_frees_the_slot() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        let token = s
            .cancel_token(&h.id)
            .expect("token exists while agent is live");

        // Simulate in-flight work awaiting cancellation.
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            "cancelled"
        });

        assert!(s.destroy(&h.id), "destroy returns true for a live agent");
        assert_eq!(
            worker.await.unwrap(),
            "cancelled",
            "destroy must cancel the token"
        );
        assert!(s.get(&h.id).is_none(), "map slot is freed after destroy");
        assert_eq!(s.state(&h.id), None);
        assert!(!s.destroy(&h.id), "second destroy is a no-op");
    }

    #[test]
    fn remove_delegates_to_destroy() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        let token = s.cancel_token(&h.id).unwrap();
        assert!(!token.is_cancelled());

        assert!(s.remove(&h.id));
        assert!(token.is_cancelled(), "remove must cancel the token");
        assert!(s.get(&h.id).is_none());
    }

    #[test]
    fn list_and_get_preserved_after_internal_type_change() {
        let mut s = SubAgentSpawner::new();
        let h1 = s.spawn("task1".into(), "t1".into());
        let h2 = s.spawn("task2".into(), "t2".into());

        let list = s.list();
        assert_eq!(list.len(), 2);
        let ids: Vec<&str> = list.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&h1.id.as_str()));
        assert!(ids.contains(&h2.id.as_str()));

        let got = s.get(&h1.id).unwrap();
        assert_eq!(got.task, "task1");
    }

    #[test]
    fn update_status_still_works() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        s.update_status(
            &h.id,
            SubAgentStatus::Executing {
                current_step: "step-1".into(),
            },
        );
        let got = s.get(&h.id).unwrap();
        assert!(matches!(got.status, SubAgentStatus::Executing { .. }));
    }

    #[test]
    fn transition_error_display() {
        let err = TransitionError::Unknown("x".into());
        assert!(err.to_string().contains("x"));

        let err = TransitionError::Illegal {
            from: SubAgentState::Created,
            to: SubAgentState::Completed,
        };
        assert!(err.to_string().contains("Created"));
        assert!(err.to_string().contains("Completed"));
    }
}
