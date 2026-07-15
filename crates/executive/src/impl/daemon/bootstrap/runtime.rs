//! Runtime-provider construction for daemon bootstrap.

use std::sync::Arc;

#[cfg(test)]
use aletheon_kernel::chronos::SystemClock;
#[cfg(test)]
use tokio_util::sync::CancellationToken;

use anyhow::Context;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::tools::tools::ToolRegistry;
use fabric::{Clock, LlmProvider};
use tokio::sync::Mutex;

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
