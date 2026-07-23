//! Generic routing boundary for extension-provided Agent runtimes.
//!
//! The router owns provider selection and handle affinity. Callers only use
//! stable Fabric contracts and never depend on a concrete subprocess or
//! external runtime implementation.

use anyhow::{Context, Result};
use fabric::{AgentHandle, AgentRuntimeProvider, AgentSpawnRequest, RuntimeId};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::agent_control::{
    AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
};

#[derive(Default)]
pub struct ExtensionRuntimeRouter {
    providers: RwLock<HashMap<RuntimeId, Arc<dyn AgentRuntimeProvider>>>,
}

/// Adapts the stable Provider contract into Executive's governed Agent
/// lifecycle. Executive retains admission, event, cancellation, and settlement
/// authority; the provider only performs runtime work.
pub struct ExtensionProviderLauncher {
    router: Arc<ExtensionRuntimeRouter>,
}

impl ExtensionProviderLauncher {
    pub fn new(router: Arc<ExtensionRuntimeRouter>) -> Self {
        Self { router }
    }
}

fn runtime_error(error: impl std::fmt::Display) -> fabric::AgentControlError {
    fabric::AgentControlError {
        kind: fabric::AgentControlErrorKind::Runtime,
        message: error.to_string(),
    }
}

#[async_trait::async_trait]
impl AgentRuntimeLauncher for ExtensionProviderLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<fabric::AgentResult, fabric::AgentControlError> {
        events
            .emit(AgentRuntimeEvent::Started {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
            })
            .await;
        let runtime_handle = self
            .router
            .start(input.request)
            .await
            .map_err(runtime_error)?;
        let value = tokio::select! {
            result = self.router.wait(&runtime_handle) => result.map_err(runtime_error)?,
            _ = input.cancellation.cancelled() => {
                self.router
                    .cancel(&runtime_handle, "executive cancellation")
                    .await
                    .map_err(runtime_error)?;
                events.emit(AgentRuntimeEvent::Terminal {
                    agent_id: input.handle.agent_id,
                    process_id: input.handle.process_id,
                    operation_id: input.handle.operation_id,
                    status: fabric::AgentRunStatus::Cancelled,
                    result: None,
                }).await;
                return Err(fabric::AgentControlError {
                    kind: fabric::AgentControlErrorKind::Terminal,
                    message: "extension runtime was cancelled".into(),
                });
            }
        };
        let result: fabric::AgentResult =
            serde_json::from_value(value).map_err(runtime_error)?;
        result.validate()?;
        events
            .emit(AgentRuntimeEvent::Terminal {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
                status: fabric::AgentRunStatus::Succeeded,
                result: Some(result.clone()),
            })
            .await;
        Ok(result)
    }
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
        let mut providers = self.providers.write();
        anyhow::ensure!(
            !providers.contains_key(&runtime_id),
            "extension runtime is already registered: {}",
            runtime_id.0
        );
        providers.insert(runtime_id, provider);
        Ok(())
    }

    pub fn unregister(&self, runtime_id: &RuntimeId) -> bool {
        self.providers.write().remove(runtime_id).is_some()
    }

    pub fn registered(&self) -> Vec<RuntimeId> {
        let mut ids: Vec<_> = self.providers.read().keys().cloned().collect();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    fn resolve(&self, runtime_id: &RuntimeId) -> Result<Arc<dyn AgentRuntimeProvider>> {
        self.providers
            .read()
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
            .providers
            .read()
            .iter()
            .map(|(id, provider)| (id.clone(), provider.clone()))
            .collect();
        let mut health = HashMap::new();
        for (id, provider) in providers {
            health.insert(id, provider.health().await.map_err(|error| error.to_string()));
        }
        health
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fabric::{
        AgentContextFork, AgentId, AgentProfileId, OperationId, ProcessId, RuntimeId,
    };
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
            task: "test".into(),
            context: AgentContextFork::default(),
            broadcast_refs: Vec::new(),
            allowed_tools: Vec::new(),
            budget: fabric::AgentBudget {
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
        assert_eq!(
            router.observe(&handle).await.unwrap()["status"],
            "running"
        );
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
