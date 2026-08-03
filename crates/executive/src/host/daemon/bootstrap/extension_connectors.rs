//! Secret-safe projection and probing for package MCP Connector assets.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use corpus::tools::mcp::config::{McpConfig, McpServerConfig, McpTransportConfig, McpTrustLevel};
use corpus::tools::mcp::manager::McpManager;
use corpus::tools::tools::Tool;
use fabric::protocol::extension::McpConnectorTransportV1;
use tokio::sync::Mutex;

use crate::application::extension_snapshot::{ExtensionConnectorAsset, ExtensionRuntimeSnapshot};

#[derive(Default)]
pub struct ExtensionConnectorRuntime {
    state: Mutex<ConnectorState>,
}

#[derive(Default)]
struct ConnectorState {
    managers: HashMap<String, Arc<McpManager>>,
    current_digest: Option<String>,
    previous_digest: Option<String>,
}

impl ExtensionConnectorRuntime {
    pub async fn probe(&self, snapshot: &ExtensionRuntimeSnapshot) -> Result<()> {
        if snapshot.connectors.is_empty() {
            return Ok(());
        }
        if self
            .state
            .lock()
            .await
            .managers
            .contains_key(&snapshot.digest)
        {
            return Ok(());
        }

        let servers = snapshot
            .connectors
            .iter()
            .map(project_connector)
            .collect::<Result<Vec<_>>>()?;
        let unique_server_ids = servers
            .iter()
            .map(|server| server.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::ensure!(
            unique_server_ids.len() == servers.len(),
            "package MCP Connector ids collide after provider-safe normalization"
        );
        let expected = servers.len();
        let mut manager = McpManager::new(McpConfig {
            servers,
            ..McpConfig::default()
        });
        manager.connect_all().await?;
        anyhow::ensure!(
            manager.connected_count() == expected,
            "only {} of {expected} package MCP Connectors connected",
            manager.connected_count()
        );
        for connector in snapshot.connectors.iter() {
            let runtime_id = provider_safe_connector_id(&connector.manifest.id);
            let tools = manager.server_tools(&runtime_id).with_context(|| {
                format!(
                    "package MCP Connector '{}' did not enumerate tools",
                    connector.manifest.id
                )
            })?;
            for allowed in &connector.manifest.allowed_tools {
                anyhow::ensure!(
                    tools.iter().any(|tool| &tool.name == allowed),
                    "package MCP Connector '{}' did not enumerate allowed tool '{allowed}'",
                    connector.manifest.id
                );
            }
            if !connector.manifest.allowed_resources.is_empty() {
                let resources = manager.list_resources(&runtime_id).await?;
                for allowed in &connector.manifest.allowed_resources {
                    anyhow::ensure!(
                        resources
                            .iter()
                            .any(|resource| &resource.name == allowed || &resource.uri == allowed),
                        "package MCP Connector '{}' did not enumerate allowed resource '{allowed}'",
                        connector.manifest.id
                    );
                }
            }
        }
        self.state
            .lock()
            .await
            .managers
            .insert(snapshot.digest.clone(), Arc::new(manager));
        Ok(())
    }

    pub async fn tool_sets(
        &self,
        snapshot: &ExtensionRuntimeSnapshot,
    ) -> Result<Vec<(String, Vec<Arc<dyn Tool>>)>> {
        if snapshot.connectors.is_empty() {
            return Ok(Vec::new());
        }
        let manager = self
            .state
            .lock()
            .await
            .managers
            .get(&snapshot.digest)
            .cloned()
            .context("package MCP Connector candidate has not been probed")?;
        let mut grouped = BTreeMap::<String, Vec<Arc<dyn Tool>>>::new();
        for wrapper in manager
            .tool_wrappers()
            .into_iter()
            .chain(manager.resource_provider_wrappers())
        {
            let name = wrapper.name().to_owned();
            let connector = connector_for_wrapper(&name, snapshot)?;
            grouped
                .entry(connector_tool_owner(&connector.package_id))
                .or_default()
                .push(Arc::from(wrapper));
        }
        Ok(grouped.into_iter().collect())
    }

    pub async fn publish(&self, previous_digest: &str, candidate_digest: &str) {
        let mut state = self.state.lock().await;
        state.previous_digest = Some(previous_digest.to_owned());
        state.current_digest = Some(candidate_digest.to_owned());
        let retained = [state.current_digest.clone(), state.previous_digest.clone()];
        let removed: Vec<_> = state
            .managers
            .keys()
            .filter(|digest| {
                !retained
                    .iter()
                    .flatten()
                    .any(|retained_digest| retained_digest == *digest)
            })
            .cloned()
            .collect();
        let removed: Vec<_> = removed
            .into_iter()
            .filter_map(|digest| state.managers.remove(&digest))
            .collect();
        drop(state);
        for manager in removed {
            let _ = manager.shutdown(std::time::Duration::from_secs(2)).await;
        }
    }
}

pub fn connector_tool_owner(package_id: &str) -> String {
    format!("extension-mcp:{package_id}")
}

fn provider_safe_connector_id(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn project_connector(asset: &ExtensionConnectorAsset) -> Result<McpServerConfig> {
    if let Some(secret_name) = &asset.manifest.bearer_token_env {
        anyhow::ensure!(
            std::env::var_os(secret_name).is_some(),
            "package MCP Connector '{}' requires missing host secret '{secret_name}'",
            asset.manifest.id
        );
    }
    let (transport, trust) = match &asset.manifest.transport {
        McpConnectorTransportV1::Stdio { command, args } => {
            let command = contained_command(&asset.package_root, command)?;
            (
                McpTransportConfig::Stdio {
                    command: command.to_string_lossy().into_owned(),
                    args: args.clone(),
                },
                McpTrustLevel::LocalTrusted,
            )
        }
        McpConnectorTransportV1::StreamableHttp { url } => (
            McpTransportConfig::StreamableHttp { url: url.clone() },
            endpoint_trust(url),
        ),
        McpConnectorTransportV1::Sse { url } => (
            McpTransportConfig::Sse { url: url.clone() },
            endpoint_trust(url),
        ),
    };
    Ok(McpServerConfig {
        name: provider_safe_connector_id(&asset.manifest.id),
        transport,
        trust,
        enabled: true,
        bearer_token_env: asset.manifest.bearer_token_env.clone(),
        oauth: None,
        request_timeout_ms: Some(asset.manifest.request_timeout_ms),
        health_check_interval_sec: 0,
        allowlist: asset.manifest.allowed_tools.clone(),
        denylist: Vec::new(),
        resource_allowlist: asset.manifest.allowed_resources.clone(),
        permission_overrides: std::collections::HashMap::new(),
    })
}

fn contained_command(package_root: &Path, declared: &str) -> Result<std::path::PathBuf> {
    let command = package_root
        .join(declared)
        .canonicalize()
        .with_context(|| format!("package MCP command does not exist: {declared}"))?;
    anyhow::ensure!(
        command.starts_with(package_root),
        "package MCP command escapes its package root"
    );
    anyhow::ensure!(command.is_file(), "package MCP command is not a file");
    Ok(command)
}

fn endpoint_trust(url: &str) -> McpTrustLevel {
    if url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]")
    {
        McpTrustLevel::LocalTrusted
    } else {
        McpTrustLevel::Untrusted
    }
}

fn connector_for_wrapper<'a>(
    wrapper_name: &str,
    snapshot: &'a ExtensionRuntimeSnapshot,
) -> Result<&'a ExtensionConnectorAsset> {
    snapshot
        .connectors
        .iter()
        .filter(|connector| {
            let runtime_id = provider_safe_connector_id(&connector.manifest.id);
            wrapper_name.starts_with(&format!("{runtime_id}__"))
                || wrapper_name.starts_with(&format!("mcp.{runtime_id}."))
        })
        .max_by_key(|connector| connector.manifest.id.len())
        .with_context(|| format!("MCP wrapper '{wrapper_name}' has no package Connector owner"))
}

#[cfg(test)]
mod tests {
    use super::provider_safe_connector_id;

    #[test]
    fn connector_runtime_id_is_provider_safe_and_stable() {
        assert_eq!(provider_safe_connector_id("aurb.gbrain"), "aurb_gbrain");
        assert_eq!(provider_safe_connector_id("already-safe"), "already-safe");
    }
}
