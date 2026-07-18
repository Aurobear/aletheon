//! Executive-owned application configuration.
//!
//! Domain crates define typed sub-configuration values. Only Executive discovers,
//! merges, validates, and reports application layers.

mod agent;
pub mod backpressure;
pub mod diagnostics;
mod genome;
mod grok_hardening;
mod infra;
mod provenance;
mod provider;
pub mod schema;

pub use agent::{
    AgentConfig, AgentLoopConfig, CircuitBreakerConfig, EvolutionSettings, ExecutiveConfig,
    HooksConfig, PerceptionConfig,
};
pub use backpressure::BackpressureConfig;
pub use diagnostics::{EffectiveConfigView, LayerInfo, LayersView};
pub use cognit::config::{
    AgentAdmissionConfig, BackupMode, CognitConfig, DeploymentBackupConfig, DeploymentConfig,
    DeploymentHealthConfig, DeploymentIntegrationsConfig, DeploymentMode, DeploymentPathsConfig,
    DeploymentQuotaConfig, DeploymentSecretFilesConfig, McpMemoryConfig, GoalRuntimeConfig,
    PiRuntimeConfig, RoleRuntimeConfig,
};
pub use genome::GenomeConfig;
pub use grok_hardening::GrokHardeningConfig;
pub use infra::{
    DaemonConfig, McpServerConfig, MemoryConfig, PluginsConfig, SandboxConfig, TelegramConfig,
};
pub use provenance::{ConfigProvenance, ConfigSource, ConfigSourceKind, Provenanced};
pub use provider::{ModelRoutingConfig, ProviderConfig, Transport};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The one application root schema. Its fields are typed domain inputs; it does
/// not grant any domain permission to discover files or environment variables.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub agent: AgentConfig,
    pub providers: Vec<ProviderConfig>,
    pub model_aliases: HashMap<String, String>,
    pub model_routing: ModelRoutingConfig,
    pub sandbox: SandboxConfig,
    pub mcp_servers: Vec<McpServerConfig>,
    pub plugins: PluginsConfig,
    pub memory: MemoryConfig,
    pub daemon: DaemonConfig,
    pub hooks: HooksConfig,
    pub perception: PerceptionConfig,
    pub evolution: EvolutionSettings,
    pub telegram: TelegramConfig,
    pub goal_runtime: Option<GoalRuntimeConfig>,
    pub pi_runtime: PiRuntimeConfig,
    pub deployment: DeploymentConfig,
    pub grok_hardening: GrokHardeningConfig,
    /// D2-M5-T2: overload/backpressure limits (default unlimited).
    #[serde(default)]
    pub backpressure: BackpressureConfig,
    /// S1 sandbox profiles (from trusted daemon config, never from repo).
    #[serde(default)]
    pub sandbox_profiles: fabric::SandboxProfiles,
}

impl AppConfig {
    pub fn cognit(&self) -> CognitConfig {
        CognitConfig {
            agent: self.agent.clone(),
            providers: self.providers.clone(),
            model_aliases: self.model_aliases.clone(),
            model_routing: self.model_routing.clone(),
        }
    }

    /// Parse one explicit file strictly. The caller chooses how it participates
    /// in application layering.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config layer {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("validate config layer {}", path.display()))
    }

    /// Compatibility merge used by composition tests. It has the same structural
    /// replacement semantics as application layers.
    pub fn merge(&mut self, other: AppConfig) {
        let mut base = toml::Value::try_from(self.clone()).expect("AppConfig serializes to TOML");
        let overlay = toml::Value::try_from(other).expect("AppConfig serializes to TOML");
        merge_value(&mut base, overlay);
        *self = base.try_into().expect("merged typed configs remain valid");
    }
}

#[derive(Debug, Clone)]
pub struct ConfigLayer {
    pub source: ConfigSource,
    pub value: toml::Value,
}

impl ConfigLayer {
    pub fn from_toml(source: ConfigSource, text: &str) -> Result<Self> {
        let value: toml::Value = toml::from_str(text)
            .map_err(|error| anyhow::anyhow!("parse config layer {}: {error}", source.locator))?;
        // Validate the layer against the typed root before it can affect lower layers.
        let _: AppConfig = value.clone().try_into().map_err(|error| {
            anyhow::anyhow!("validate config layer {}: {error}", source.locator)
        })?;
        Ok(Self { source, value })
    }

    pub fn from_path(kind: ConfigSourceKind, path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let source = ConfigSource::new(kind, path.display().to_string());
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config layer {}", path.display()))?;
        Self::from_toml(source, &text).map(Some)
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub value: AppConfig,
    pub provenance: ConfigProvenance,
}

impl LoadedConfig {
    pub fn source(&self, path: &str) -> Option<&ConfigSource> {
        self.provenance.source(path)
    }

    /// Render effective values for diagnostics while always masking secret-shaped leaves.
    pub fn redacted_effective_values(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(&self.value).expect("AppConfig serializes");
        provenance::redact_json(&mut value);
        value
    }
}

/// Deterministic loader used by tests, embedders, and the host composition root.
pub fn merge_layers(layers: impl IntoIterator<Item = ConfigLayer>) -> Result<LoadedConfig> {
    let defaults = AppConfig::default();
    let mut merged = toml::Value::try_from(&defaults).context("serialize compiled defaults")?;
    let mut provenance = ConfigProvenance::default();
    provenance::record_leaves(&merged, "", &ConfigSource::defaults(), &mut provenance);

    for layer in layers {
        provenance::record_leaves(&layer.value, "", &layer.source, &mut provenance);
        merge_value(&mut merged, layer.value);
    }
    let value = merged
        .try_into::<AppConfig>()
        .context("validate effective application config")?;
    Ok(LoadedConfig { value, provenance })
}

/// Load defaults, system, user, project, environment, then CLI overrides.
pub fn load_layered(
    project_dir: Option<&Path>,
    environment: impl IntoIterator<Item = (String, String)>,
    cli: impl IntoIterator<Item = (String, String)>,
) -> Result<LoadedConfig> {
    let mut layers = Vec::new();
    if let Some(layer) = ConfigLayer::from_path(
        ConfigSourceKind::System,
        Path::new("/etc/aletheon/config.toml"),
    )? {
        layers.push(layer);
    }
    if let Some(home) = dirs::home_dir() {
        if let Some(layer) =
            ConfigLayer::from_path(ConfigSourceKind::User, &home.join(".aletheon/config.toml"))?
        {
            layers.push(layer);
        }
    }
    if let Some(project) = project_dir {
        if let Some(layer) = ConfigLayer::from_path(
            ConfigSourceKind::Project,
            &project.join(".aletheon/config.toml"),
        )? {
            layers.push(layer);
        }
    }
    if let Some(layer) = override_layer(
        ConfigSourceKind::Environment,
        "environment:ALETHEON__",
        environment.into_iter().filter_map(|(key, value)| {
            key.strip_prefix("ALETHEON__")
                .map(|path| (path.to_ascii_lowercase().replace("__", "."), value))
        }),
    )? {
        layers.push(layer);
    }
    if let Some(layer) = override_layer(ConfigSourceKind::Cli, "cli", cli)? {
        layers.push(layer);
    }
    merge_layers(layers)
}

pub fn load_for_host(project_dir: Option<&Path>, explicit: Option<&Path>) -> Result<LoadedConfig> {
    let mut loaded = load_layered(project_dir, std::env::vars(), std::iter::empty())?;
    if let Some(path) = explicit {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read explicit config {}", path.display()))?;
        let layer = ConfigLayer::from_toml(
            ConfigSource::new(ConfigSourceKind::Cli, path.display().to_string()),
            &text,
        )?;
        // Preserve the regular layers and put the explicit file last.
        let base = ConfigLayer {
            source: ConfigSource::new(ConfigSourceKind::Project, "effective lower layers"),
            value: toml::Value::try_from(&loaded.value)?,
        };
        loaded = merge_layers([base, layer])?;
    }
    Ok(loaded)
}

fn override_layer(
    kind: ConfigSourceKind,
    locator: &str,
    values: impl IntoIterator<Item = (String, String)>,
) -> Result<Option<ConfigLayer>> {
    let mut root = toml::map::Map::new();
    let mut any = false;
    for (path, raw) in values {
        any = true;
        insert_path(&mut root, &path, parse_override(&raw)?)?;
    }
    if !any {
        return Ok(None);
    }
    let source = ConfigSource::new(kind, locator);
    let value = toml::Value::Table(root);
    let _: AppConfig = value
        .clone()
        .try_into()
        .with_context(|| format!("validate config layer {}", source.locator))?;
    Ok(Some(ConfigLayer { source, value }))
}

fn parse_override(raw: &str) -> Result<toml::Value> {
    let probe = format!("value = {raw}");
    if let Ok(toml::Value::Table(mut table)) = toml::from_str::<toml::Value>(&probe) {
        if let Some(value) = table.remove("value") {
            return Ok(value);
        }
    }
    Ok(toml::Value::String(raw.to_string()))
}

fn insert_path(
    root: &mut toml::map::Map<String, toml::Value>,
    path: &str,
    value: toml::Value,
) -> Result<()> {
    let components: Vec<_> = path.split('.').filter(|part| !part.is_empty()).collect();
    anyhow::ensure!(!components.is_empty(), "config override path is empty");
    let mut table = root;
    for component in &components[..components.len() - 1] {
        let entry = table
            .entry((*component).to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
        table = entry
            .as_table_mut()
            .with_context(|| format!("config override path '{path}' crosses a non-table value"))?;
    }
    table.insert(components.last().unwrap().to_string(), value);
    Ok(())
}

fn merge_value(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[allow(dead_code)]
fn _typed_path(_: PathBuf) {}
