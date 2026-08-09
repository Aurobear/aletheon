//! RA-05 AgentSupervisor + DelegateBackendRegistry owner seam (Agent Kernel V2).
//!
//! The single Agent spawn/send/wait/cancel/recovery entry contract.  This seam
//! defines the generic `DelegateBackendRegistry` (backend id → typed launcher)
//! and the AgentSupervisor shape.  Per runbook PR-A it is **not wired**: the
//! legacy AgentControlService/AgentRuntimeRegistry remain authoritative until
//! the RA-05 PR-C writer cutover.  Pi concrete files are **not** migrated
//! here (that is E6-K6d); this seam is generic only.

use crate::error::RuntimeError;
use crate::ids::{AgentRunId, Generation, SessionId, TurnId};
use std::collections::HashMap;
use std::sync::Arc;

/// Stable delegate backend id (e.g. "native", "pi-coder").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DelegateBackendId(pub String);

/// Typed delegate spawn request.  The Runtime assigns AgentRunId/Generation.
#[derive(Debug, Clone)]
pub struct DelegateSpawnRequest {
    pub parent_session: SessionId,
    pub parent_turn: TurnId,
    pub backend: DelegateBackendId,
    pub profile: Option<String>,
}

/// Typed delegate receipt returned after spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateReceipt {
    pub agent_run: AgentRunId,
    pub generation: Generation,
}

/// A backend launcher registered in the registry.  A running AgentRun is
/// pinned to its backend generation; a reload must not swap the binding.
#[async_trait::async_trait]
pub trait DelegateBackend: Send + Sync {
    async fn spawn(&self, request: &DelegateSpawnRequest) -> Result<DelegateReceipt, RuntimeError>;
    async fn cancel(&self, agent_run: &AgentRunId) -> Result<(), RuntimeError>;
    async fn wait(
        &self,
        agent_run: &AgentRunId,
    ) -> Result<crate::event::TurnTerminal, RuntimeError>;
}

/// Generic DelegateBackend registry.  Registration rejects duplicate ids.
/// A running binding is pinned to its backend generation.
#[derive(Default)]
pub struct DelegateBackendRegistry {
    backends: HashMap<DelegateBackendId, Arc<dyn DelegateBackend>>,
}

impl DelegateBackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
    ) -> Result<(), RuntimeError> {
        if id.0.trim().is_empty() {
            return Err(RuntimeError::Internal);
        }
        if self.backends.contains_key(&id) {
            return Err(RuntimeError::Internal); // duplicate backend
        }
        self.backends.insert(id, backend);
        Ok(())
    }

    pub fn resolve(&self, id: &DelegateBackendId) -> Option<Arc<dyn DelegateBackend>> {
        self.backends.get(id).cloned()
    }
}

/// AgentSupervisor shape: the single spawn/send/wait/cancel/recovery entry.
/// The production supervisor (PR-C) implements this; this seam defines the
/// contract and the generic registry.
pub struct AgentSupervisorSeam {
    registry: DelegateBackendRegistry,
}

impl AgentSupervisorSeam {
    pub fn new(registry: DelegateBackendRegistry) -> Self {
        Self { registry }
    }

    /// Spawn a delegate through the generic registry.  Returns typed receipt;
    /// the Runtime assigns AgentRunId/Generation.
    pub async fn spawn(
        &self,
        request: &DelegateSpawnRequest,
    ) -> Result<DelegateReceipt, RuntimeError> {
        let backend = self
            .registry
            .resolve(&request.backend)
            .ok_or(RuntimeError::AgentRunNotFound)?;
        backend.spawn(request).await
    }

    pub fn registry(&self) -> &DelegateBackendRegistry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubBackend;

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
        async fn wait(&self, _a: &AgentRunId) -> Result<crate::event::TurnTerminal, RuntimeError> {
            Ok(crate::event::TurnTerminal::Completed)
        }
    }

    #[test]
    fn registry_rejects_duplicate_backend() {
        let mut registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        assert!(registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend),)
            .is_err());
    }

    #[tokio::test]
    async fn supervisor_spawns_via_registry_with_runtime_assigned_ids() {
        let mut registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = AgentSupervisorSeam::new(registry);
        let receipt = supervisor
            .spawn(&DelegateSpawnRequest {
                parent_session: SessionId("s1".into()),
                parent_turn: TurnId("t1".into()),
                backend: DelegateBackendId("native".into()),
                profile: None,
            })
            .await
            .unwrap();
        assert_eq!(receipt.agent_run.0, "run-1");
    }
}
