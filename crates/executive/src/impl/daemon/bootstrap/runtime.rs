//! Runtime-provider construction for daemon bootstrap.

use std::collections::HashMap;
use std::sync::Arc;

use fabric::{AgentControlPort, Registry};
use tracing::info;

use crate::r#impl::agent_loader::AgentLoader;
use crate::service::inference_port::{InferencePort, PortLlmProvider};

/// Combine a profile-level and global iteration limit where `0` means
/// "unlimited". The old `.min(global).max(1)` collapsed `0` (unlimited)
/// into `1`; this preserves unlimited semantics on both sides.
fn combine_limits(profile: usize, global: usize) -> usize {
    match (profile, global) {
        (0, 0) => 0,
        (0, global) => global,
        (profile, 0) => profile,
        (profile, global) => profile.min(global),
    }
}

pub(super) fn load_agent_profiles(
    agents_dir: &std::path::Path,
    inference: Arc<dyn InferencePort>,
    default_llm: Arc<dyn LlmProvider>,
    definitions: &[fabric::ToolDefinition],
    config: &crate::core::config::ExecutiveConfig,
    profiles_config: &crate::core::config::AgentProfilesConfig,
) -> anyhow::Result<(
    Arc<crate::r#impl::runtime::AgentProfileRegistry>,
    HashMap<String, fabric::AgentProfile>,
)> {
    let mut loader = AgentLoader::new();
    if agents_dir.exists() {
        loader.load_from_dir(agents_dir)?;
        loader.validate_legacy_toml_grants(agents_dir)?;
    }
    for profile in profiles_config.overrides.keys() {
        anyhow::ensure!(
            loader.get(profile).is_some(),
            "agent_profiles override references unknown Markdown profile '{profile}'"
        );
    }
    if !profiles_config.default.trim().is_empty() {
        anyhow::ensure!(
            loader.get(&profiles_config.default).is_some(),
            "agent_profiles.default references unknown Markdown profile '{}'",
            profiles_config.default
        );
    }
    let catalog = definitions
        .iter()
        .map(|definition| (definition.name.clone(), definition.clone()))
        .collect::<HashMap<_, _>>();
    let registry = Arc::new(crate::r#impl::runtime::AgentProfileRegistry::default());
    let mut profiles = HashMap::new();
    for role in loader.list() {
        let llm: Arc<dyn LlmProvider> = match role.model.as_deref() {
            Some(model) => Arc::new(PortLlmProvider::new(inference.clone(), model)),
            None => default_llm.clone(),
        };
        let mut tools = Vec::with_capacity(role.tools.len());
        for name in &role.tools {
            let definition = catalog.get(name).cloned().with_context(|| {
                format!(
                    "Agent profile '{}' references unknown tool '{name}'",
                    role.name
                )
            })?;
            tools.push(definition);
        }

        // Apply per-profile overrides from AgentProfilesConfig.
        let overrides = profiles_config.overrides.get(&role.name);
        let profile_limit = overrides
            .and_then(|ov| ov.max_iterations)
            .unwrap_or(role.max_iterations);
        let max_iterations = combine_limits(profile_limit, config.max_iterations);
        let tool_timeout_ms = overrides
            .and_then(|ov| ov.tool_timeout_ms)
            .unwrap_or(30_000);
        let approval_policy = overrides
            .and_then(|ov| ov.approval_policy)
            .unwrap_or(fabric::AgentApprovalPolicy::AutoApprove);

        // Derive risk tier from tool permission levels — delegated to the
        // registry construction; here we use a simple heuristic.
        let risk_tier = derive_risk_tier(&role.tools, &catalog);

        let profile = fabric::AgentProfile {
            id: fabric::AgentProfileId(role.name.clone()),
            system_prompt: role.body.clone(),
            model: llm.name().to_string(),
            allowed_tools: role.tools.clone(),
            max_iterations,
            max_input_tokens: config.context_window_tokens as u64,
            max_output_tokens: 16_384,
            max_tool_calls: overrides.and_then(|ov| ov.max_tool_calls).unwrap_or(
                if config.agent_loop.max_tool_calls == 0 {
                    128
                } else {
                    config.agent_loop.max_tool_calls as u32
                },
            ),
            max_elapsed_ms: 10 * 60 * 1_000,
            profile_name: role.name.clone(),
            risk_tier,
            approval_policy,
            tool_timeout_ms,
            inheritable: true,
            parent_restriction: fabric::ParentRestriction::SameOrSafer,
        };
        registry.register(crate::r#impl::runtime::ResolvedAgentProfile {
            profile: profile.clone(),
            llm,
            tools,
        })?;
        profiles.insert(role.name.clone(), profile);
    }
    Ok((registry, profiles))
}

/// Derive the risk tier from the tool names and the tool catalog.
/// Uses permission levels from registered tools to compute the maximum risk.
fn derive_risk_tier(
    tool_names: &[String],
    catalog: &HashMap<String, fabric::ToolDefinition>,
) -> fabric::RiskTier {
    let mut max_level: i32 = 0;
    for name in tool_names {
        if let Some(_def) = catalog.get(name) {
            // PermissionLevel is not directly exposed on ToolDefinition;
            // use a name-based heuristic for built-in tools.
            let level = tool_permission_level(name);
            if level > max_level {
                max_level = level;
            }
        }
    }
    match max_level {
        0 => fabric::RiskTier::ReadOnly,
        1 => fabric::RiskTier::Sandboxed,
        2 => fabric::RiskTier::System,
        _ => fabric::RiskTier::Unrestricted,
    }
}

fn tool_permission_level(name: &str) -> i32 {
    match name {
        // L3 — Destructive
        "module_load" | "kernel_build" => 3,
        // L2 — System-level changes
        "ebpf_compile" | "module_build" => 2,
        // L1 — Sandboxed write
        "file_write" | "bash_exec" | "apply_patch" | "web_fetch" => 1,
        // L0 — Read-only (default)
        _ => 0,
    }
}

/// Register thin explicit controls plus the bounded compatibility `agent` tool.
pub(super) async fn register_agent_tools(
    tools: crate::core::corpus_group::ToolRegistryHandle,
    agent_control: Arc<dyn AgentControlPort>,
    profiles: HashMap<String, fabric::AgentProfile>,
) {
    let explicit =
        corpus::tools::tools::agent_control::AgentControlTools::new(agent_control.clone());
    let mut registry = tools.lock().await;
    for tool in explicit.tools() {
        if let Err(error) = registry.register(tool) {
            tracing::warn!(%error, "Failed to register explicit Agent control tool");
        }
    }
    if !profiles.is_empty() {
        let count = profiles.len();
        let agent_tool = corpus::tools::tools::agent_tool::AgentTool::new(
            profiles,
            agent_control,
            crate::r#impl::runtime::NativeCognitRuntime::runtime_id(),
        );
        if let Err(e) = registry.register(Arc::new(agent_tool)) {
            tracing::warn!(error = %e, "Failed to register AgentTool");
        } else {
            info!(
                agents = count,
                "Registered compatibility AgentTool control client"
            );
        }
    }
}

#[cfg(test)]
use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use fabric::{Clock, LlmProvider};
#[cfg(test)]
use tokio_util::sync::CancellationToken;

use crate::core::runtime_registry::RuntimeRegistry;
use crate::r#impl::runtime::ProviderWorkerRuntime;
use crate::service::CapabilityService;

pub(crate) fn register_goal_runtimes(
    registry: &mut RuntimeRegistry,
    config: &cognit::config::GoalRuntimeConfig,
    inference: Arc<dyn InferencePort>,
    model_aliases: &HashMap<String, String>,
    tools: Vec<fabric::ToolDefinition>,
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
        let model_spec = if route.model_alias.contains('/') {
            route.model_alias.clone()
        } else {
            model_aliases
                .get(&route.model_alias)
                .cloned()
                .with_context(|| format!("model alias '{}' not found", route.model_alias))?
        };
        let provider: Arc<dyn LlmProvider> =
            Arc::new(PortLlmProvider::new(inference.clone(), model_spec));
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
        registry.register(runtime_id.clone(), runtime)?;
        registered.push(runtime_id);
    }
    Ok(registered)
}

#[cfg(test)]
mod goal_runtime_tests {
    use super::*;
    use crate::core::config::AppConfig;
    use cognit::config::{GoalRuntimeConfig, ProviderConfig, RoleRuntimeConfig, Transport};

    struct NoopCapability;

    #[derive(Default)]
    struct NoopInference;

    #[async_trait::async_trait]
    impl InferencePort for NoopInference {
        async fn complete(
            &self,
            _request: crate::service::inference_port::CoreInferenceRequest,
        ) -> Result<fabric::LlmResponse, crate::service::inference_port::InferenceError> {
            Err(anyhow::anyhow!("unused test inference").into())
        }

        async fn stream(
            &self,
            _request: crate::service::inference_port::CoreInferenceRequest,
        ) -> Result<fabric::LlmStream, crate::service::inference_port::InferenceError> {
            Err(anyhow::anyhow!("unused test inference").into())
        }
    }

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
                patch_delta: None,
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
    ) -> anyhow::Result<(RuntimeRegistry, Vec<fabric::RuntimeId>)> {
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let mut registry = RuntimeRegistry::new();
        let ids = super::register_goal_runtimes(
            &mut registry,
            &config,
            inference,
            &app.model_aliases,
            Vec::new(),
            Arc::new(NoopCapability),
            Arc::new(SystemClock::new()),
        )?;
        Ok((registry, ids))
    }

    #[test]
    fn disabled_goal_runtime_registers_nothing() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let (registry, ids) = register(GoalRuntimeConfig::default(), app).unwrap();
        assert!(ids.is_empty());
        assert!(!registry.contains(&fabric::RuntimeId("deepseek-worker".into())));
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
        let (registry, ids) = register(config, app).unwrap();
        assert_eq!(ids.len(), 2);
        for id in ids {
            assert!(registry.contains(&id));
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
        let (registry, ids) = register(config, app).unwrap();
        assert_eq!(
            ids,
            vec![
                fabric::RuntimeId("deepseek-worker".into()),
                fabric::RuntimeId("escalation-reviewer".into())
            ]
        );
        assert!(registry.contains(&ids[0]));
        assert!(registry.contains(&ids[1]));
    }

    #[test]
    fn agent_profiles_resolve_model_tools_and_frontmatter_limits() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: review\ntools: [file_read, grep]\nmax_iterations: 3\n---\nReview evidence only.",
        )
        .unwrap();
        let mut app = AppConfig::default();
        app.providers.push(provider("shared"));
        app.agent.default_provider = Some("shared".into());
        app.agent.default_model = Some("model".into());
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> =
            Arc::new(PortLlmProvider::new(inference.clone(), "shared/model"));
        let definitions = ["file_read", "grep"]
            .into_iter()
            .map(|name| fabric::ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .collect::<Vec<_>>();
        let (registry, profiles) = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &crate::core::config::ExecutiveConfig::default(),
            &crate::core::config::AgentProfilesConfig::default(),
        )
        .unwrap();
        let profile = profiles.get("reviewer").unwrap();
        assert_eq!(profile.allowed_tools, vec!["file_read", "grep"]);
        assert_eq!(profile.max_iterations, 3);
        assert_eq!(profile.max_tool_calls, 128);
        assert!(registry.resolve(&profile.id).is_ok());
    }

    #[test]
    fn agent_profile_config_rejects_unknown_default_and_override() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: review\ntools: [file_read]\n---\nReview.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> =
            Arc::new(PortLlmProvider::new(inference.clone(), "shared/model"));
        let definitions = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];

        let mut unknown_default = crate::core::config::AgentProfilesConfig {
            default: "missing".into(),
            ..Default::default()
        };
        assert!(super::load_agent_profiles(
            directory.path(),
            inference.clone(),
            llm.clone(),
            &definitions,
            &crate::core::config::ExecutiveConfig::default(),
            &unknown_default,
        )
        .is_err());

        unknown_default.default.clear();
        unknown_default.overrides.insert(
            "missing".into(),
            crate::core::config::ProfileOverride::default(),
        );
        assert!(super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &crate::core::config::ExecutiveConfig::default(),
            &unknown_default,
        )
        .is_err());
    }
}

#[cfg(test)]
mod combine_limits_tests {
    use super::combine_limits;

    #[test]
    fn zero_profile_zero_global_is_unlimited() {
        assert_eq!(combine_limits(0, 0), 0);
    }

    #[test]
    fn zero_profile_uses_global_cap() {
        assert_eq!(combine_limits(0, 50), 50);
    }

    #[test]
    fn zero_global_keeps_profile() {
        assert_eq!(combine_limits(20, 0), 20);
    }

    #[test]
    fn both_nonzero_takes_min() {
        assert_eq!(combine_limits(20, 50), 20);
        assert_eq!(combine_limits(80, 50), 50);
    }
}
