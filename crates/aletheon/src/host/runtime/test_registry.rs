//! Integration-fixture launcher catalog.
//!
//! This is exported only through the explicit test adapter surface; installed
//! composition registers typed Runtime backends directly.

use std::collections::HashMap;
use std::sync::Arc;

use ::contracts::{AgentControlError, AgentControlErrorKind, RuntimeId};
use parking_lot::RwLock;

use crate::composition::agent_control::AgentRuntimeLauncher;

#[derive(Default)]
pub struct AgentExecutionRegistry {
    runtimes: RwLock<HashMap<RuntimeId, Arc<dyn AgentRuntimeLauncher>>>,
    manifests: RwLock<HashMap<RuntimeId, runtime::RuntimeManifest>>,
}

impl AgentExecutionRegistry {
    pub fn register(
        &self,
        id: RuntimeId,
        launcher: Arc<dyn AgentRuntimeLauncher>,
    ) -> Result<(), AgentControlError> {
        if id.0.trim().is_empty() {
            return Err(AgentControlError::invalid("runtime id must not be empty"));
        }
        let mut runtimes = self.runtimes.write();
        if runtimes.contains_key(&id) {
            return Err(AgentControlError {
                kind: AgentControlErrorKind::Conflict,
                message: format!("runtime already registered: {}", id.0),
            });
        }
        runtimes.insert(id, launcher);
        Ok(())
    }

    pub fn unregister(&self, id: &RuntimeId) -> bool {
        self.manifests.write().remove(id);
        self.runtimes.write().remove(id).is_some()
    }

    /// Register a selectable runtime contract alongside the Aletheon-owned
    /// launcher. The manifest describes capabilities; it does not gain
    /// lifecycle, admission, cancellation, or settlement authority.
    pub fn register_manifested(
        &self,
        id: RuntimeId,
        launcher: Arc<dyn AgentRuntimeLauncher>,
        manifest: runtime::RuntimeManifest,
    ) -> Result<(), AgentControlError> {
        if manifest.id != id.0 {
            return Err(AgentControlError::invalid(
                "runtime manifest id differs from registry id",
            ));
        }
        manifest.validate().map_err(AgentControlError::invalid)?;
        self.register(id.clone(), launcher)?;
        self.manifests.write().insert(id, manifest);
        Ok(())
    }

    pub fn catalog(&self) -> Vec<runtime::RuntimeManifest> {
        let mut manifests = self.manifests.read().values().cloned().collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.id.cmp(&right.id));
        manifests
    }

    /// All registered runtime IDs, including explicit fixture routes
    /// that do not publish a selectable manifest. The composition root uses
    /// this only to bind one Runtime DelegateBackend adapter per route.
    pub fn runtime_ids(&self) -> Vec<RuntimeId> {
        let mut ids = self.runtimes.read().keys().cloned().collect::<Vec<_>>();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    pub fn select(
        &self,
        request: &runtime::RuntimeSelectionRequest,
    ) -> Result<
        (
            RuntimeId,
            Arc<dyn AgentRuntimeLauncher>,
            runtime::RuntimeSelectionDecision,
        ),
        AgentControlError,
    > {
        let manifests = self.manifests.read();
        let decision = request
            .select(manifests.values())
            .map_err(|error| AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message: error.to_string(),
            })?;
        drop(manifests);
        let runtime_id = RuntimeId(decision.selected_runtime_id.clone());
        let launcher = self.resolve(&runtime_id)?;
        Ok((runtime_id, launcher, decision))
    }

    pub fn resolve_selector(
        &self,
        selector: &runtime::RuntimeSelector,
        required: &[runtime::RuntimeCapability],
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError> {
        let manifests = self.manifests.read();
        let id = selector
            .resolve_id(manifests.values(), required)
            .map_err(|message| AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message,
            })?;
        drop(manifests);
        self.resolve(&RuntimeId(id))
    }

    pub fn resolve(
        &self,
        id: &RuntimeId,
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError> {
        let runtimes = self.runtimes.read();
        runtimes.get(id).cloned().ok_or_else(|| {
            let mut available = runtimes
                .keys()
                .map(|runtime_id| runtime_id.0.as_str())
                .collect::<Vec<_>>();
            available.sort_unstable();
            AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message: format!(
                    "runtime is not registered: {}; available runtimes: {}",
                    id.0,
                    available.join(", ")
                ),
            }
        })
    }
}

impl crate::composition::agent_control::FixtureRuntimeCatalog for AgentExecutionRegistry {
    fn catalog(&self) -> Vec<runtime::RuntimeManifest> {
        Self::catalog(self)
    }

    fn select(
        &self,
        request: &runtime::RuntimeSelectionRequest,
    ) -> Result<
        (
            RuntimeId,
            Arc<dyn crate::composition::agent_control::AgentRuntimeLauncher>,
            runtime::RuntimeSelectionDecision,
        ),
        AgentControlError,
    > {
        Self::select(self, request)
    }

    fn resolve(
        &self,
        id: &RuntimeId,
    ) -> Result<Arc<dyn crate::composition::agent_control::AgentRuntimeLauncher>, AgentControlError>
    {
        Self::resolve(self, id)
    }
}
