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

impl AgentComposition {
    /// List profiles that failed validation and were quarantined.
    pub fn quarantined_profiles(&self) -> &[super::runtime::QuarantinedProfile] {
        &self.quarantined_profiles
    }
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
    let mut result = super::runtime::load_agent_profiles(
        input.agents_dir,
        input.inference.clone(),
        input.default_llm.clone(),
        input.definitions,
        input.runtime_config,
        input.profiles_config,
    )?;
    let state_dir = input
        .agents_dir
        .parent()
        .unwrap_or(input.agents_dir)
        .join("state");
    persist_quarantine(&state_dir, &result.quarantined)?;

    let known_good = state_dir.join("agent-profile-known-good");
    if result.profiles.is_empty() && !result.quarantined.is_empty() && known_good.exists() {
        tracing::warn!(
            path = %known_good.display(),
            "All current profiles are invalid; loading previous-known-good snapshot"
        );
        let fallback = super::runtime::load_agent_profiles(
            &known_good,
            input.inference.clone(),
            input.default_llm.clone(),
            input.definitions,
            input.runtime_config,
            input.profiles_config,
        )?;
        if !fallback.profiles.is_empty() {
            result.registry = fallback.registry;
            result.profiles = fallback.profiles;
        }
    } else if !result.profiles.is_empty() {
        replace_profile_snapshot(input.agents_dir, &known_good)?;
    }

    for q in &result.quarantined {
        tracing::warn!(
            profile = %q.name,
            reason = %q.reason,
            "Agent profile quarantined — daemon will start without it"
        );
    }

    if !result.quarantined.is_empty() {
        tracing::warn!(
            count = result.quarantined.len(),
            names = ?result.quarantined.iter().map(|q| &q.name).collect::<Vec<_>>(),
            "Daemon starting with quarantined agent profiles"
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

    let active_profile_name = match select_active_profile(
        &input.profiles_config.default,
        result.profiles.keys().cloned().collect(),
    ) {
        Ok(name) => name,
        Err(_) if !result.profiles.is_empty() => {
            // Configured default was quarantined or missing; fall back to any valid profile.
            let fallback = if result.profiles.contains_key("code-agent") {
                "code-agent".to_string()
            } else {
                let mut names: Vec<_> = result.profiles.keys().cloned().collect();
                names.sort();
                names.into_iter().next().unwrap()
            };
            tracing::warn!(
                configured = %input.profiles_config.default,
                fallback = %fallback,
                quarantined_count = result.quarantined.len(),
                "Configured default profile unavailable; falling back to available profile"
            );
            fallback
        }
        Err(e) => return Err(e), // No profiles at all — genuine error
    };
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

fn persist_quarantine(
    state_dir: &Path,
    quarantined: &[super::runtime::QuarantinedProfile],
) -> anyhow::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let path = state_dir.join("agent-profile-quarantine.json");
    let temporary = state_dir.join(format!(
        ".agent-profile-quarantine.{}.tmp",
        std::process::id()
    ));
    let result = (|| -> anyhow::Result<()> {
        let file = std::fs::File::create(&temporary)?;
        serde_json::to_writer_pretty(&file, quarantined)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        std::fs::File::open(state_dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn replace_profile_snapshot(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let parent = destination.parent().context("snapshot has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".agent-profile-known-good.{}.tmp", std::process::id()));
    if temporary.exists() {
        std::fs::remove_dir_all(&temporary)?;
    }
    copy_directory(source, &temporary)?;
    if destination.exists() {
        let old = parent.join(format!(".agent-profile-known-good.{}.old", std::process::id()));
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(destination, &old)?;
        std::fs::rename(&temporary, destination)?;
        let _ = std::fs::remove_dir_all(old);
    } else {
        std::fs::rename(&temporary, destination)?;
    }
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(destination)?;
    if !source.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

    #[test]
    fn quarantine_and_known_good_snapshots_are_durable() {
        let temp = TempDir::new().unwrap();
        let agents = temp.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("code-agent.md"), "known good").unwrap();
        let state = temp.path().join("state");
        persist_quarantine(
            &state,
            &[super::super::runtime::QuarantinedProfile {
                name: "broken".into(),
                reason: "unknown tool".into(),
            }],
        )
        .unwrap();
        let quarantine: serde_json::Value = serde_json::from_slice(
            &std::fs::read(state.join("agent-profile-quarantine.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(quarantine[0]["name"], "broken");

        let snapshot = state.join("agent-profile-known-good");
        replace_profile_snapshot(&agents, &snapshot).unwrap();
        std::fs::write(agents.join("code-agent.md"), "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(snapshot.join("code-agent.md")).unwrap(),
            "known good"
        );
    }
}
