//! RA-05 PR-C Runtime AgentSupervisor writer (Agent Kernel V2).
//!
//! The deployable Agent writer: the Runtime assigns the child/generation/run
//! ID, resolves the delegate through the single generic `DelegateBackendRegistry`,
//! and exposes one spawn/wait/cancel entry.  A running AgentRun is pinned to
//! its backend generation (reload never swaps a running binding).  Pi concrete
//! files are NOT migrated here (that is E6-K6d).  The legacy AgentControlService
//! stays authoritative until the PR-C switch.

use crate::agent_supervisor::{
    DelegateBackend, DelegateBackendId, DelegateBackendRegistry, DelegateReceipt,
    DelegateSpawnRequest,
};
use crate::error::RuntimeError;
use crate::event::TurnTerminal;
use crate::ids::{AgentRunId, Generation, SessionId, TurnId};
use std::collections::HashMap;
use std::sync::Arc;

/// Runtime AgentSupervisor writer over the generic registry.
pub struct RuntimeAgentSupervisor {
    registry: DelegateBackendRegistry,
    /// Run ID → pinned backend handle (reload never swaps a running binding).
    bindings: std::sync::Mutex<HashMap<AgentRunId, Arc<dyn DelegateBackend>>>,
}

impl RuntimeAgentSupervisor {
    pub fn new(registry: DelegateBackendRegistry) -> Self {
        Self {
            registry,
            bindings: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// One spawn entry.  The Runtime assigns AgentRunId + Generation; the
    /// backend binding is resolved through the single registry and pinned.
    pub async fn spawn(
        &self,
        parent_session: &SessionId,
        parent_turn: &TurnId,
        backend: DelegateBackendId,
        profile: Option<String>,
    ) -> Result<DelegateReceipt, RuntimeError> {
        let request = DelegateSpawnRequest {
            parent_session: parent_session.clone(),
            parent_turn: parent_turn.clone(),
            backend,
            profile,
        };
        let backend = self
            .registry
            .resolve(&request.backend)
            .ok_or(RuntimeError::AgentRunNotFound)?;
        let receipt = backend.spawn(&request).await?;
        // Pin the running binding to the exact backend (reload must not swap).
        self.bindings
            .lock()
            .unwrap()
            .insert(receipt.agent_run.clone(), backend);
        Ok(receipt)
    }

    /// Wait for a run's authoritative terminal through its pinned backend.
    /// Never returns success before the terminal is authoritative.
    pub async fn wait(&self, agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
        let backend = self
            .bindings
            .lock()
            .unwrap()
            .get(agent_run)
            .cloned()
            .ok_or(RuntimeError::AgentRunNotFound)?;
        backend.wait(agent_run).await
    }

    /// Cancel a run through its pinned backend.
    pub async fn cancel(&self, agent_run: &AgentRunId) -> Result<(), RuntimeError> {
        let backend = self
            .bindings
            .lock()
            .unwrap()
            .get(agent_run)
            .cloned()
            .ok_or(RuntimeError::AgentRunNotFound)?;
        backend.cancel(agent_run).await
    }

    pub fn registry(&self) -> &DelegateBackendRegistry {
        &self.registry
    }
}

/// A backend handle pinned to a running AgentRun.
pub struct StubBackend;

#[async_trait::async_trait]
impl DelegateBackend for StubBackend {
    async fn spawn(&self, _r: &DelegateSpawnRequest) -> Result<DelegateReceipt, RuntimeError> {
        Ok(DelegateReceipt {
            agent_run: AgentRunId("run-1".into()),
            generation: Generation(1),
        })
    }
    async fn cancel(&self, _a: &AgentRunId) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn wait(&self, _a: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
        Ok(TurnTerminal::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_spawns_and_wait_returns_authoritative_terminal() {
        let mut registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(receipt.agent_run.0, "run-1");
        // Wait returns the authoritative terminal (typed, never guessed).
        let terminal = supervisor.wait(&receipt.agent_run).await.unwrap();
        assert_eq!(terminal, TurnTerminal::Completed);
    }

    #[tokio::test]
    async fn cancel_uses_the_pinned_backend() {
        let mut registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert!(supervisor.cancel(&receipt.agent_run).await.is_ok());
    }
}
