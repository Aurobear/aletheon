//! Runtime-provider construction for daemon bootstrap.

use std::collections::HashMap;
use std::sync::Arc;

use fabric::{AgentControlPort, Registry};
use tracing::info;

use crate::application::inference_port::{InferencePort, PortLlmProvider};
use crate::composition::agent_loader::AgentLoader;

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

/// A profile that failed validation, with diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct QuarantinedProfile {
    pub name: String,
    pub reason: String,
}

/// Result of loading agent profiles — separates valid from quarantined.
pub(super) struct ProfileLoadResult {
    pub registry: Arc<crate::adapters::runtime::AgentProfileRegistry>,
    pub profiles: HashMap<String, fabric::AgentProfile>,
    pub quarantined: Vec<QuarantinedProfile>,
}

pub(super) fn load_agent_profiles(
    agents_dir: &std::path::Path,
    inference: Arc<dyn InferencePort>,
    default_llm: Arc<dyn LlmProvider>,
    definitions: &[fabric::ToolDefinition],
    config: &crate::composition::config::ExecutiveConfig,
    profiles_config: &crate::composition::config::AgentProfilesConfig,
) -> anyhow::Result<ProfileLoadResult> {
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
    let registry = Arc::new(crate::adapters::runtime::AgentProfileRegistry::default());
    let mut profiles = HashMap::new();
    let mut quarantined = Vec::new();
    for role in loader.list() {
        let mut tools = Vec::with_capacity(role.tools.len());
        let mut failed = false;
        for name in &role.tools {
            match catalog.get(name).cloned() {
                Some(definition) => tools.push(definition),
                None => {
                    tracing::warn!(
                        profile = %role.name,
                        tool = %name,
                        "Agent profile references unknown tool — quarantining profile"
                    );
                    quarantined.push(QuarantinedProfile {
                        name: role.name.clone(),
                        reason: format!(
                            "Agent profile '{}' references unknown tool '{name}'",
                            role.name
                        ),
                    });
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }

        let llm: Arc<dyn LlmProvider> = match role.model.as_deref() {
            Some(model) => Arc::new(PortLlmProvider::new(inference.clone(), model)),
            None => default_llm.clone(),
        };

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
        let max_input_tokens = overrides
            .and_then(|ov| ov.max_input_tokens)
            .unwrap_or(config.context_window_tokens as u64)
            .min(config.context_window_tokens as u64);
        let max_output_tokens = overrides
            .and_then(|ov| ov.max_output_tokens)
            .unwrap_or(16_384);
        if max_input_tokens == 0 || max_output_tokens == 0 || max_output_tokens >= max_input_tokens
        {
            quarantined.push(QuarantinedProfile {
                name: role.name.clone(),
                reason: format!(
                    "invalid token budgets: input={max_input_tokens}, output={max_output_tokens}"
                ),
            });
            continue;
        }

        // Derive risk tier from tool permission levels — delegated to the
        // registry construction; here we use a simple heuristic.
        let risk_tier = derive_risk_tier(&role.tools, &catalog);

        let profile = fabric::AgentProfile {
            id: fabric::AgentProfileId(role.name.clone()),
            system_prompt: role.body.clone(),
            model: llm.name().to_string(),
            allowed_tools: role.tools.clone(),
            max_iterations,
            max_input_tokens,
            max_output_tokens,
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
        registry.register(crate::adapters::runtime::ResolvedAgentProfile {
            profile: profile.clone(),
            llm,
            tools,
        })?;
        profiles.insert(role.name.clone(), profile);
    }
    Ok(ProfileLoadResult {
        registry,
        profiles,
        quarantined,
    })
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
        let name = tool.name().to_owned();
        if let Err(error) = registry.register(tool) {
            tracing::warn!(%error, "Failed to register explicit Agent control tool");
        } else if let Err(error) = registry.set_proposal_confidence(&name, 0.5) {
            tracing::warn!(%error, tool = %name, "Failed to register Agent control proposal confidence");
        }
    }
    if !profiles.is_empty() {
        let count = profiles.len();
        let agent_tool = corpus::tools::tools::agent_tool::AgentTool::new(
            profiles,
            agent_control,
            crate::adapters::runtime::NativeCognitRuntime::runtime_id(),
        );
        if let Err(e) = registry.register(Arc::new(agent_tool)) {
            tracing::warn!(error = %e, "Failed to register AgentTool");
        } else if let Err(error) = registry.set_proposal_confidence("agent", 0.5) {
            tracing::warn!(%error, "Failed to register compatibility Agent proposal confidence");
        } else {
            info!(
                agents = count,
                "Registered compatibility AgentTool control client"
            );
        }
    }
}

use anyhow::Context;
use fabric::{Clock, LlmProvider};

use crate::adapters::runtime::ProviderWorkerRuntime;
use crate::application::CapabilityService;
use crate::core::runtime_registry::RuntimeRegistry;

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
mod runtime_tests;
