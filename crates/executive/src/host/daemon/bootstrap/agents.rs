//! Typed construction unit for agent profiles.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use fabric::{LlmProvider, ToolDefinition};

use crate::application::inference_port::InferencePort;

pub(super) struct AgentCompositionInput<'a> {
    pub(super) agents_dir: &'a Path,
    pub(super) inference: Arc<dyn InferencePort>,
    pub(super) default_llm: Arc<dyn LlmProvider>,
    pub(super) definitions: &'a [ToolDefinition],
    pub(super) runtime_config: &'a crate::composition::config::ExecutiveConfig,
    pub(super) profiles_config: &'a crate::composition::config::AgentProfilesConfig,
}

pub(super) struct AgentComposition {
    pub(super) profiles: Arc<crate::adapters::runtime::AgentProfileRegistry>,
    pub(super) tool_profiles: HashMap<String, fabric::AgentProfile>,
    pub(super) active_profile_name: String,
    pub(super) quarantined_profiles: Vec<super::runtime::QuarantinedProfile>,
}

fn select_active_profile(configured: &str, mut names: Vec<String>) -> anyhow::Result<String> {
    if !configured.trim().is_empty() {
        anyhow::ensure!(
            names.iter().any(|name| name == configured),
            "configured default Agent profile '{configured}' is unavailable"
        );
        return Ok(configured.to_owned());
    }
    if names.iter().any(|name| name == "code-agent") {
        return Ok("code-agent".to_owned());
    }
    names.sort();
    names
        .into_iter()
        .next()
        .context("no Agent profile is available for the main turn")
}

pub(super) fn compose(input: AgentCompositionInput<'_>) -> anyhow::Result<AgentComposition> {
    let result = super::runtime::load_agent_profiles(
        input.agents_dir,
        input.inference,
        input.default_llm,
        input.definitions,
        input.runtime_config,
        input.profiles_config,
    )?;

    for q in &result.quarantined {
        tracing::warn!(
            profile = %q.name,
            reason = %q.reason,
            "Agent profile quarantined — daemon will start without it"
        );
    }

    if result.profiles.is_empty() && !result.quarantined.is_empty() {
        tracing::error!(
            count = result.quarantined.len(),
            "All agent profiles failed validation — daemon starting in degraded mode"
        );
        return Ok(AgentComposition {
            profiles: result.registry,
            tool_profiles: result.profiles,
            active_profile_name: String::new(),
            quarantined_profiles: result.quarantined,
        });
    }

    let active_profile_name =
        select_active_profile(&input.profiles_config.default, result.profiles.keys().cloned().collect())?;
    result.registry
        .resolve_by_name(&active_profile_name)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(AgentComposition {
        profiles: result.registry,
        tool_profiles: result.profiles,
        active_profile_name,
        quarantined_profiles: result.quarantined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_prefers_config_then_code_agent_then_sorted_fallback() {
        let names = vec!["reviewer".into(), "code-agent".into(), "alpha".into()];
        assert_eq!(
            select_active_profile("reviewer", names.clone()).unwrap(),
            "reviewer"
        );
        assert_eq!(
            select_active_profile("", names.clone()).unwrap(),
            "code-agent"
        );
        assert_eq!(
            select_active_profile("", vec!["reviewer".into(), "alpha".into()]).unwrap(),
            "alpha"
        );
    }

    #[test]
    fn selection_fails_closed_for_missing_configured_or_empty_profiles() {
        assert!(select_active_profile("missing", vec!["reviewer".into()]).is_err());
        assert!(select_active_profile("", Vec::new()).is_err());
    }
}
