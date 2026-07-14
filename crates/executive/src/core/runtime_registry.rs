//! Runtime selection registry for sub-agent spawns.

use crate::core::sub_agent::SubAgentRuntime;
use anyhow::{bail, Context};
use fabric::RuntimeId;
use std::collections::HashMap;
use std::sync::Arc;

/// Configured sub-agent runtimes indexed by stable runtime ID.
#[derive(Default)]
pub struct RuntimeRegistry {
    runtimes: HashMap<RuntimeId, Arc<dyn SubAgentRuntime>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a runtime, rejecting ambiguous duplicate IDs.
    pub fn register(
        &mut self,
        id: RuntimeId,
        runtime: Arc<dyn SubAgentRuntime>,
    ) -> anyhow::Result<()> {
        if id.0.trim().is_empty() {
            bail!("runtime id must not be empty");
        }
        if self.runtimes.contains_key(&id) {
            bail!("runtime already registered: {}", id.0);
        }
        self.runtimes.insert(id, runtime);
        Ok(())
    }

    /// Resolve one configured runtime.
    pub fn resolve(&self, id: &RuntimeId) -> anyhow::Result<Arc<dyn SubAgentRuntime>> {
        self.runtimes
            .get(id)
            .cloned()
            .with_context(|| format!("runtime not registered: {}", id.0))
    }

    /// Set the compatibility default runtime, replacing an earlier default.
    pub(crate) fn set_default(&mut self, runtime: Arc<dyn SubAgentRuntime>) {
        self.runtimes.insert(Self::default_id(), runtime);
    }

    pub fn default_id() -> RuntimeId {
        RuntimeId("default".into())
    }

    pub fn contains(&self, id: &RuntimeId) -> bool {
        self.runtimes.contains_key(id)
    }
}

impl std::fmt::Debug for RuntimeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&RuntimeId> = self.runtimes.keys().collect();
        f.debug_struct("RuntimeRegistry")
            .field("runtime_ids", &ids)
            .finish()
    }
}
