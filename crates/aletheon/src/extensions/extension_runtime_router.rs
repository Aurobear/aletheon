//! Generic routing boundary for extension-provided Agent runtimes.
//!
//! The router owns provider selection and handle affinity. Callers only use
//! stable Fabric contracts and never depend on a concrete subprocess or
//! external runtime implementation.

use ::contracts::{AgentHandle, AgentSpawnRequest, RuntimeId};
use anyhow::{Context, Result};
use parking_lot::RwLock;
use runtime::AgentRuntimeProvider;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ExtensionRuntimeRouter {
    state: RwLock<RouterState>,
}

#[derive(Default)]
struct RouterState {
    providers: HashMap<RuntimeId, Arc<dyn AgentRuntimeProvider>>,
    package_owners: HashMap<RuntimeId, String>,
}

impl ExtensionRuntimeRouter {
    pub fn register(
        &self,
        runtime_id: RuntimeId,
        provider: Arc<dyn AgentRuntimeProvider>,
    ) -> Result<()> {
        anyhow::ensure!(
            !runtime_id.0.trim().is_empty(),
            "extension runtime ID must not be empty"
        );
        let mut state = self.state.write();
        anyhow::ensure!(
            !state.providers.contains_key(&runtime_id),
            "extension runtime is already registered: {}",
            runtime_id.0
        );
        state.providers.insert(runtime_id, provider);
        Ok(())
    }

    pub fn unregister(&self, runtime_id: &RuntimeId) -> bool {
        let mut state = self.state.write();
        state.package_owners.remove(runtime_id);
        state.providers.remove(runtime_id).is_some()
    }

    pub fn validate_package_providers(
        &self,
        replaced_owners: &[String],
        replacements: &[(String, RuntimeId, Arc<dyn AgentRuntimeProvider>)],
    ) -> Result<()> {
        let replaced = replaced_owners
            .iter()
            .collect::<std::collections::HashSet<_>>();
        let state = self.state.read();
        let mut ids = std::collections::HashSet::new();
        for (owner, id, _) in replacements {
            anyhow::ensure!(
                replaced.contains(owner),
                "runtime owner is outside replacement set"
            );
            anyhow::ensure!(
                !id.0.trim().is_empty(),
                "extension runtime ID must not be empty"
            );
            anyhow::ensure!(
                ids.insert(id.clone()),
                "duplicate extension runtime ID: {}",
                id.0
            );
            if state.providers.contains_key(id)
                && state
                    .package_owners
                    .get(id)
                    .is_none_or(|existing| !replaced.contains(existing))
            {
                anyhow::bail!("extension runtime is already registered: {}", id.0);
            }
        }
        Ok(())
    }

    pub fn replace_package_providers(
        &self,
        replaced_owners: &[String],
        replacements: Vec<(String, RuntimeId, Arc<dyn AgentRuntimeProvider>)>,
    ) -> Result<()> {
        self.validate_package_providers(replaced_owners, &replacements)?;
        let replaced = replaced_owners
            .iter()
            .collect::<std::collections::HashSet<_>>();
        let mut state = self.state.write();
        let removed = state
            .package_owners
            .iter()
            .filter(|(_, owner)| replaced.contains(owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in removed {
            state.package_owners.remove(&id);
            state.providers.remove(&id);
        }
        for (owner, id, provider) in replacements {
            state.package_owners.insert(id.clone(), owner);
            state.providers.insert(id, provider);
        }
        Ok(())
    }

    pub fn registered(&self) -> Vec<RuntimeId> {
        let mut ids: Vec<_> = self.state.read().providers.keys().cloned().collect();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    fn resolve(&self, runtime_id: &RuntimeId) -> Result<Arc<dyn AgentRuntimeProvider>> {
        self.state
            .read()
            .providers
            .get(runtime_id)
            .cloned()
            .with_context(|| format!("extension runtime is not registered: {}", runtime_id.0))
    }

    pub async fn start(&self, request: AgentSpawnRequest) -> Result<AgentHandle> {
        let requested_runtime = request.runtime_id.clone();
        let handle = self.resolve(&requested_runtime)?.start(request).await?;
        anyhow::ensure!(
            handle.runtime_id == requested_runtime,
            "extension runtime returned a handle for a different runtime"
        );
        Ok(handle)
    }

    pub async fn observe(&self, handle: &AgentHandle) -> Result<Value> {
        self.resolve(&handle.runtime_id)?.observe(handle).await
    }

    pub async fn steer(&self, handle: &AgentHandle, input: Value) -> Result<()> {
        self.resolve(&handle.runtime_id)?.steer(handle, input).await
    }

    pub async fn follow_up(&self, handle: &AgentHandle, input: Value) -> Result<Value> {
        self.resolve(&handle.runtime_id)?
            .follow_up(handle, input)
            .await
    }

    pub async fn cancel(&self, handle: &AgentHandle, reason: &str) -> Result<()> {
        self.resolve(&handle.runtime_id)?
            .cancel(handle, reason)
            .await
    }

    pub async fn wait(&self, handle: &AgentHandle) -> Result<Value> {
        self.resolve(&handle.runtime_id)?.wait(handle).await
    }

    pub async fn health(&self) -> HashMap<RuntimeId, Result<(), String>> {
        let providers: Vec<_> = self
            .state
            .read()
            .providers
            .iter()
            .map(|(id, provider)| (id.clone(), provider.clone()))
            .collect();
        let mut health = HashMap::new();
        for (id, provider) in providers {
            health.insert(
                id,
                provider.health().await.map_err(|error| error.to_string()),
            );
        }
        health
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{
        AgentContextFork, AgentId, AgentProfileId, OperationId, ProcessId, RuntimeId,
    };
    use async_trait::async_trait;
    use uuid::Uuid;

    struct Provider {
        id: RuntimeId,
    }

    #[async_trait]
    impl AgentRuntimeProvider for Provider {
        async fn start(&self, request: AgentSpawnRequest) -> Result<AgentHandle> {
            Ok(AgentHandle {
                agent_id: AgentId(Uuid::new_v4()),
                root_agent_id: request.root_agent_id,
                parent_agent_id: request.parent_agent_id,
                process_id: ProcessId(Uuid::new_v4()),
                operation_id: OperationId(Uuid::new_v4()),
                runtime_id: self.id.clone(),
                profile_id: request.profile_id,
            })
        }

        async fn observe(&self, _: &AgentHandle) -> Result<Value> {
            Ok(serde_json::json!({"status": "running"}))
        }

        async fn steer(&self, _: &AgentHandle, _: Value) -> Result<()> {
            Ok(())
        }

        async fn follow_up(&self, _: &AgentHandle, input: Value) -> Result<Value> {
            Ok(input)
        }

        async fn cancel(&self, _: &AgentHandle, _: &str) -> Result<()> {
            Ok(())
        }

        async fn wait(&self, _: &AgentHandle) -> Result<Value> {
            Ok(serde_json::json!({"status": "completed"}))
        }

        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    fn request(runtime_id: RuntimeId) -> AgentSpawnRequest {
        AgentSpawnRequest {
            root_agent_id: AgentId(Uuid::new_v4()),
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: AgentProfileId("test".into()),
            runtime_id,
            trusted_workspace: None,
            delegator_authority: None,
            cognitive_binding: None,
            task: "test".into(),
            context: AgentContextFork::default(),
            broadcast_refs: Vec::new(),
            allowed_tools: Vec::new(),
            budget: ::contracts::AgentBudget {
                max_input_tokens: 1,
                max_output_tokens: 1,
                max_tool_calls: 1,
                max_elapsed_ms: 1,
                max_cost_usd: None,
                max_depth: 1,
            },
            background_decls: Vec::new(),
        }
    }

    #[tokio::test]
    async fn routes_every_operation_by_stable_runtime_id() {
        let router = ExtensionRuntimeRouter::default();
        let id = RuntimeId("generic-test-runtime".into());
        router
            .register(id.clone(), Arc::new(Provider { id: id.clone() }))
            .unwrap();
        let handle = router.start(request(id.clone())).await.unwrap();
        assert_eq!(router.registered(), vec![id.clone()]);
        assert_eq!(router.observe(&handle).await.unwrap()["status"], "running");
        assert_eq!(
            router
                .follow_up(&handle, serde_json::json!({"message": "next"}))
                .await
                .unwrap()["message"],
            "next"
        );
        router.steer(&handle, Value::Null).await.unwrap();
        router.cancel(&handle, "test").await.unwrap();
        assert_eq!(router.wait(&handle).await.unwrap()["status"], "completed");
        assert!(router.health().await[&id].is_ok());
    }

    #[tokio::test]
    async fn rejects_duplicate_and_cross_runtime_handles() {
        let router = ExtensionRuntimeRouter::default();
        let registered = RuntimeId("registered".into());
        router
            .register(
                registered.clone(),
                Arc::new(Provider {
                    id: RuntimeId("wrong".into()),
                }),
            )
            .unwrap();
        assert!(router
            .register(
                registered.clone(),
                Arc::new(Provider {
                    id: registered.clone()
                })
            )
            .is_err());
        assert!(router.start(request(registered)).await.is_err());
    }
}
