//! Runtime-provider construction for daemon bootstrap.

use std::collections::HashMap;
use std::sync::Arc;

use fabric::{AgentControlPort, Registry};
use tracing::info;

use crate::r#impl::agent_loader::AgentLoader;

/// Register the legacy AgentTool against the canonical runtime and capability path.
pub(super) async fn register_agent_tool(
    agents_dir: &std::path::Path,
    tools: crate::core::corpus_group::ToolRegistryHandle,
    agent_control: Arc<dyn AgentControlPort>,
) {
    let mut rt_agent_loader = AgentLoader::new();
    if agents_dir.exists() {
        let _ = rt_agent_loader.load_from_dir(agents_dir);
    }
    let mut agent_defs: HashMap<String, corpus::tools::tools::agent_tool::AgentDefinition> =
        HashMap::new();
    for role in rt_agent_loader.list() {
        agent_defs.insert(
            role.name.clone(),
            corpus::tools::tools::agent_tool::AgentDefinition {
                name: role.name.clone(),
                description: role.description.clone(),
                tools: role.tools.clone(),
                model: role.model.clone(),
                max_iterations: 20,
                system_prompt: role.body.clone(),
            },
        );
    }
    if !agent_defs.is_empty() {
        let execute_fn: corpus::tools::tools::agent_tool::ExecuteSubAgentFn =
            Arc::new(move |system_prompt, user_prompt, allowed_tools| {
                let agent_control = agent_control.clone();
                Box::pin(async move {
                    let root = fabric::AgentId::new();
                    let handle = agent_control
                        .spawn(fabric::AgentSpawnRequest {
                            root_agent_id: root,
                            parent_agent_id: None,
                            parent_process_id: None,
                            profile_id: fabric::AgentProfileId("agent-tool".into()),
                            runtime_id: fabric::RuntimeId("default".into()),
                            task: user_prompt,
                            context: fabric::AgentContextFork::SelectedProjection {
                                items: vec![system_prompt],
                            },
                            allowed_tools,
                            budget: fabric::AgentBudget {
                                max_input_tokens: 128_000,
                                max_output_tokens: 16_384,
                                max_tool_calls: 128,
                                max_elapsed_ms: 10 * 60 * 1_000,
                                max_cost_usd: None,
                                max_depth: 4,
                            },
                        })
                        .await
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    let snapshot = agent_control
                        .wait(fabric::AgentWaitRequest {
                            caller_root_agent_id: root,
                            agent_id: handle.agent_id,
                            timeout_ms: 10 * 60 * 1_000,
                        })
                        .await
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    snapshot.result.map(|result| result.output).ok_or_else(|| {
                        anyhow::anyhow!("Agent terminated without a result: {:?}", snapshot.status)
                    })
                })
            });
        let agent_tool =
            corpus::tools::tools::agent_tool::AgentTool::new(agent_defs.clone(), execute_fn);
        if let Err(e) = tools.lock().await.register(Arc::new(agent_tool)) {
            tracing::warn!(error = %e, "Failed to register AgentTool");
        } else {
            info!(
                agents = agent_defs.len(),
                "Registered AgentTool with sub-agents"
            );
        }
    }
}

#[cfg(test)]
use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::tools::tools::ToolRegistry;
use fabric::{Clock, LlmProvider};
use tokio::sync::Mutex;
#[cfg(test)]
use tokio_util::sync::CancellationToken;

use crate::core::sub_agent::SubAgentSpawner;
use crate::r#impl::runtime::ProviderWorkerRuntime;
use crate::service::CapabilityService;

pub(crate) fn register_goal_runtimes(
    spawner: &mut SubAgentSpawner,
    config: &cognit::config::GoalRuntimeConfig,
    providers: &ProviderRegistry,
    tools: Arc<Mutex<ToolRegistry>>,
    capability: Arc<dyn CapabilityService>,
    clock: Arc<dyn Clock>,
) -> anyhow::Result<Vec<fabric::RuntimeId>> {
    if !config.enabled {
        return Ok(Vec::new());
    }
    let worker = config
        .worker
        .as_ref()
        .context("goal runtime is enabled but worker routing is missing")?;
    let reviewer = config
        .reviewer
        .as_ref()
        .context("goal runtime is enabled but reviewer routing is missing")?;
    if worker.runtime_id == reviewer.runtime_id {
        anyhow::bail!("worker and reviewer runtime IDs must be distinct");
    }

    let routes = [
        (worker, fabric::CognitiveRole::Worker),
        (reviewer, fabric::CognitiveRole::Reviewer),
    ];
    let mut prepared = Vec::with_capacity(routes.len());
    for (route, role) in routes {
        if route.runtime_id.trim().is_empty() {
            anyhow::bail!("goal runtime ID must not be empty");
        }
        let (provider_config, model) =
            providers
                .resolve_role_alias(&route.model_alias)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "resolving runtime '{}': {}: {error}",
                        route.runtime_id,
                        route.model_alias
                    )
                })?;
        let provider: Arc<dyn LlmProvider> =
            Arc::from(providers.create_provider(&provider_config, &model));
        let runtime_id = fabric::RuntimeId(route.runtime_id.clone());
        let runtime = Arc::new(ProviderWorkerRuntime::new(
            runtime_id.clone(),
            role,
            provider,
            tools.clone(),
            capability.clone(),
            clock.clone(),
            route.max_steps,
            route.max_persisted_bytes,
            route.allowed_tools.clone(),
        ));
        prepared.push((runtime_id, runtime));
    }

    let mut registered = Vec::with_capacity(prepared.len());
    for (runtime_id, runtime) in prepared {
        spawner
            .runtime_registry_mut()
            .register(runtime_id.clone(), runtime)?;
        registered.push(runtime_id);
    }
    Ok(registered)
}

#[cfg(test)]
mod goal_runtime_tests {
    use super::*;
    use cognit::config::{
        AppConfig, GoalRuntimeConfig, ProviderConfig, RoleRuntimeConfig, Transport,
    };

    struct NoopCapability;

    #[async_trait::async_trait]
    impl CapabilityService for NoopCapability {
        async fn invoke(
            &self,
            _context: Option<crate::service::CapabilityExecutionContext>,
            call: fabric::CapabilityCall,
            _cancel: CancellationToken,
        ) -> fabric::CapabilityResult {
            fabric::CapabilityResult {
                call_id: call.call_id,
                output: "unused".into(),
                is_error: true,
                usage: fabric::UsageReport::default(),
                audit_id: None,
            }
        }
    }

    fn provider(name: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: String::new(),
            transport: Transport::Openai,
            models: vec!["model".into()],
            max_context_length: None,
            pricing: None,
        }
    }

    fn route(runtime_id: &str, model_alias: &str) -> RoleRuntimeConfig {
        RoleRuntimeConfig {
            runtime_id: runtime_id.into(),
            model_alias: model_alias.into(),
            max_steps: 2,
            max_persisted_bytes: 1024,
            allowed_tools: vec![],
        }
    }

    fn register(
        config: GoalRuntimeConfig,
        app: AppConfig,
    ) -> anyhow::Result<(SubAgentSpawner, Vec<fabric::RuntimeId>)> {
        let providers = ProviderRegistry::from_config(&app)?;
        let mut spawner = SubAgentSpawner::new();
        let ids = super::register_goal_runtimes(
            &mut spawner,
            &config,
            &providers,
            Arc::new(Mutex::new(ToolRegistry::new())),
            Arc::new(NoopCapability),
            Arc::new(SystemClock::new()),
        )?;
        Ok((spawner, ids))
    }

    #[test]
    fn disabled_goal_runtime_registers_nothing() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let (spawner, ids) = register(GoalRuntimeConfig::default(), app).unwrap();
        assert!(ids.is_empty());
        assert!(!spawner
            .runtime_registry()
            .contains(&fabric::RuntimeId("deepseek-worker".into())));
    }

    #[test]
    fn enabled_goal_runtime_rejects_missing_route_and_unknown_alias() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let missing = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "p/model")),
            reviewer: None,
        };
        assert!(register(missing, app.clone())
            .unwrap_err()
            .to_string()
            .contains("reviewer routing is missing"));

        let unknown = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "unknown-alias")),
            reviewer: Some(route("escalation-reviewer", "p/model")),
        };
        assert!(register(unknown, app)
            .unwrap_err()
            .to_string()
            .contains("model alias 'unknown-alias' not found"));
    }

    #[test]
    fn same_provider_can_back_distinct_runtime_ids() {
        let mut app = AppConfig::default();
        app.providers.push(provider("shared"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "shared/worker-model")),
            reviewer: Some(route("escalation-reviewer", "shared/reviewer-model")),
        };
        let (spawner, ids) = register(config, app).unwrap();
        assert_eq!(ids.len(), 2);
        for id in ids {
            assert!(spawner.runtime_registry().contains(&id));
        }
    }

    #[test]
    fn distinct_providers_register_worker_and_reviewer() {
        let mut app = AppConfig::default();
        app.providers.push(provider("worker-provider"));
        app.providers.push(provider("review-provider"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "worker-provider/model")),
            reviewer: Some(route("escalation-reviewer", "review-provider/model")),
        };
        let (spawner, ids) = register(config, app).unwrap();
        assert_eq!(
            ids,
            vec![
                fabric::RuntimeId("deepseek-worker".into()),
                fabric::RuntimeId("escalation-reviewer".into())
            ]
        );
        assert!(spawner.runtime_registry().contains(&ids[0]));
        assert!(spawner.runtime_registry().contains(&ids[1]));
    }
}
