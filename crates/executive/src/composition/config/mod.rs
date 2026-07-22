//! Executive-owned application configuration.
//!
//! Domain crates define typed sub-configuration values. Only Executive discovers,
//! merges, validates, and reports application layers.

mod agent;
pub mod backpressure;
mod channel;
mod coding;
pub mod diagnostics;
mod genome;
mod grok_hardening;
mod infra;
mod integrations;
mod provenance;
mod provider;
pub mod schema;
mod supplemental_memory;

pub use agent::{
    AgentConfig, AgentLoopConfig, CircuitBreakerConfig, EvolutionSettings, ExecutiveConfig,
    HooksConfig, PerceptionConfig,
};
pub use backpressure::BackpressureConfig;
pub use channel::TelegramChannelConfig;
pub use coding::CodingRuntimeConfig;
pub use cognit::config::{
    AgentAdmissionConfig, BackupMode, CognitConfig, DeploymentBackupConfig, DeploymentConfig,
    DeploymentHealthConfig, DeploymentIntegrationsConfig, DeploymentMode, DeploymentPathsConfig,
    DeploymentQuotaConfig, DeploymentSecretFilesConfig, GoalRuntimeConfig, RoleRuntimeConfig,
};
pub use diagnostics::{EffectiveConfigView, LayerInfo, LayersView};
pub use genome::GenomeConfig;
pub use grok_hardening::GrokHardeningConfig;
pub use infra::{DaemonConfig, McpServerConfig, PluginsConfig, SandboxConfig};
pub use integrations::{
    CredentialResolver, EmbodimentProviderConfig, EnvironmentCredentialResolver,
    IntegrationsConfig, OAuthClientType, ProductionEmbodimentConfig, ResolvedGoogleIntegration,
    ResolvedIntegrations, ResolvedSearchIntegration, RuntimeBootstrapConfig, SecretRef,
    SecretValue,
};
pub use provenance::{ConfigProvenance, ConfigSource, ConfigSourceKind, Provenanced};
pub use provider::{ModelRoutingConfig, ProviderConfig, Transport};
pub use supplemental_memory::{MemoryConfig, SupplementalMemoryConfig};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent profile configuration with optional per-profile overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfilesConfig {
    /// Default profile name for new sessions (e.g. "code-agent").
    pub default: String,
    /// Per-profile overrides applied during profile construction.
    pub overrides: HashMap<String, ProfileOverride>,
}

/// Per-profile override values. All fields are optional; only the provided
/// values replace the profile's declared defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileOverride {
    pub max_iterations: Option<usize>,
    pub max_tool_calls: Option<u32>,
    pub tool_timeout_ms: Option<u64>,
    pub approval_policy: Option<fabric::AgentApprovalPolicy>,
}

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
    pub telegram: TelegramChannelConfig,
    pub goal_runtime: Option<GoalRuntimeConfig>,
    pub pi_runtime: CodingRuntimeConfig,
    pub deployment: DeploymentConfig,
    pub grok_hardening: GrokHardeningConfig,
    /// D2-M5-T2: overload/backpressure limits (default unlimited).
    #[serde(default)]
    pub backpressure: BackpressureConfig,
    /// S1 sandbox profiles (from trusted daemon config, never from repo).
    #[serde(default)]
    pub sandbox_profiles: fabric::SandboxProfiles,
    /// Host-owned outbound network authority for built-in tools.
    #[serde(default)]
    pub network_policy: fabric::network_policy::NetworkPolicy,
    #[serde(default)]
    pub agent_profiles: AgentProfilesConfig,
    /// Host/bootstrap values that are passed into the daemon as typed inputs.
    #[serde(default)]
    pub bootstrap: RuntimeBootstrapConfig,
    /// Optional external integrations and their credential references.
    #[serde(default)]
    pub integrations: IntegrationsConfig,
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

    pub fn preflight_integrations(
        &self,
        resolver: &dyn CredentialResolver,
    ) -> Result<ResolvedIntegrations> {
        self.integrations
            .preflight(self.deployment.integrations.google, resolver)
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

    /// Resolve optional integrations before startup work begins. Error context
    /// reports only configuration source kinds and typed paths/reference names.
    pub fn preflight_integrations(
        &self,
        resolver: &dyn CredentialResolver,
    ) -> Result<ResolvedIntegrations> {
        self.value
            .preflight_integrations(resolver)
            .map_err(|error| {
                let google_source = self
                    .source("deployment.integrations.google")
                    .map(|source| format!("{:?}", source.kind).to_ascii_lowercase())
                    .unwrap_or_else(|| "default".into());
                let search_source = self
                    .source("integrations.search.enabled")
                    .map(|source| format!("{:?}", source.kind).to_ascii_lowercase())
                    .unwrap_or_else(|| "default".into());
                anyhow::anyhow!(
                    "integration configuration sources: google={google_source}, search={search_source}; {error}"
                )
            })
    }
}

/// Deterministic loader used by tests, embedders, and the host composition root.
pub fn merge_layers(layers: impl IntoIterator<Item = ConfigLayer>) -> Result<LoadedConfig> {
    let defaults = AppConfig::default();
    let mut merged = toml::Value::try_from(&defaults).context("serialize compiled defaults")?;
    let mut provenance = ConfigProvenance::default();
    provenance::record_leaves(&merged, "", &ConfigSource::defaults(), &mut provenance);

    for mut layer in layers {
        normalize_legacy_layer(&mut layer.value);
        // Project configuration is the only lower-trust profile source. It may
        // add names, but cannot hollow out a daemon/system/user profile by
        // redefining the same name. Environment and CLI remain trusted daemon
        // operator boundaries and retain their normal precedence.
        let project_profiles = if layer.source.kind == ConfigSourceKind::Project {
            layer
                .value
                .get("sandbox_profiles")
                .cloned()
                .map(|value| value.try_into::<fabric::SandboxProfiles>())
                .transpose()
                .context("validate project sandbox profiles")?
        } else {
            None
        };
        let lower_profiles = project_profiles.as_ref().map(|_| {
            merged
                .get("sandbox_profiles")
                .cloned()
                .unwrap_or_else(|| toml::Value::Table(Default::default()))
                .try_into::<fabric::SandboxProfiles>()
                .expect("effective lower sandbox profiles remain typed")
        });
        let mut provenance_value = layer.value.clone();
        if layer.source.kind == ConfigSourceKind::Project {
            // Network authority is host-owned. Repository configuration cannot
            // grant itself outbound access.
            if let Some(table) = layer.value.as_table_mut() {
                if table.remove("network_policy").is_some() {
                    tracing::warn!(
                        source = %layer.source.locator,
                        "project network_policy ignored; outbound authority is host-owned"
                    );
                }
            }
            if let Some(table) = provenance_value.as_table_mut() {
                table.remove("network_policy");
            }
        }
        if project_profiles.is_some() {
            // Ignored same-name definitions must never appear authoritative in
            // diagnostics. Profile authority is enforced by the typed merge;
            // this prevents ignored project leaves from impersonating the
            // trusted lower source in diagnostics.
            if let Some(table) = provenance_value.as_table_mut() {
                table.remove("sandbox_profiles");
            }
        }
        provenance::record_leaves(&provenance_value, "", &layer.source, &mut provenance);
        merge_value(&mut merged, layer.value);
        if let (Some(mut lower), Some(project)) = (lower_profiles, project_profiles) {
            for name in project
                .profiles
                .keys()
                .filter(|name| lower.profiles.contains_key(*name))
            {
                tracing::warn!(
                    profile = %name,
                    source = %layer.source.locator,
                    "project sandbox profile cannot override trusted same-name profile"
                );
            }
            lower.merge_project_additive(project);
            let encoded = toml::Value::try_from(lower)
                .context("serialize additively merged project sandbox profiles")?;
            merged
                .as_table_mut()
                .expect("AppConfig root is a TOML table")
                .insert("sandbox_profiles".into(), encoded);
        }
    }
    let value = merged
        .try_into::<AppConfig>()
        .context("validate effective application config")?;
    Ok(LoadedConfig { value, provenance })
}

fn normalize_alias(table: &mut toml::map::Map<String, toml::Value>, old: &str, canonical: &str) {
    if let Some(value) = table.remove(old) {
        table.entry(canonical.to_owned()).or_insert(value);
    }
}

/// Convert legacy TOML keys before they are merged with canonical compiled
/// defaults. Serde aliases alone cannot safely decode a merged table containing
/// both spellings because that is correctly rejected as a duplicate field.
fn normalize_legacy_layer(value: &mut toml::Value) {
    let Some(root) = value.as_table_mut() else {
        return;
    };
    if let Some(memory) = root.get_mut("memory").and_then(toml::Value::as_table_mut) {
        normalize_alias(memory, "gbrain", "supplemental");
        if let Some(config) = memory
            .get_mut("supplemental")
            .and_then(toml::Value::as_table_mut)
        {
            for (old, canonical) in [
                ("source", "write_source"),
                ("timeout_ms", "request_timeout_ms"),
                ("max_results", "recall_limit"),
                ("max_chars", "max_content_bytes"),
                ("capture_enabled", "projection_enabled"),
                ("outbox_dir", "legacy_outbox_dir"),
            ] {
                normalize_alias(config, old, canonical);
            }
        }
    }
    if let Some(deployment) = root
        .get_mut("deployment")
        .and_then(toml::Value::as_table_mut)
    {
        if let Some(integrations) = deployment
            .get_mut("integrations")
            .and_then(toml::Value::as_table_mut)
        {
            normalize_alias(integrations, "gbrain", "supplemental_memory");
        }
        if let Some(secrets) = deployment
            .get_mut("secrets")
            .and_then(toml::Value::as_table_mut)
        {
            normalize_alias(secrets, "gbrain", "supplemental_memory");
        }
        if let Some(quotas) = deployment
            .get_mut("quotas")
            .and_then(toml::Value::as_table_mut)
        {
            normalize_alias(quotas, "gbrain_spool_bytes", "supplemental_spool_bytes");
            normalize_alias(
                quotas,
                "gbrain_spool_soft_bytes",
                "supplemental_spool_soft_bytes",
            );
            normalize_alias(quotas, "gbrain_spool_items", "supplemental_spool_items");
        }
    }
}

/// Load defaults, system, user, project, environment, then CLI overrides.
pub fn load_layered(
    project_dir: Option<&Path>,
    environment: impl IntoIterator<Item = (String, String)>,
    cli: impl IntoIterator<Item = (String, String)>,
) -> Result<LoadedConfig> {
    let environment = normalize_legacy_environment(environment);
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
    let environment = std::env::vars().collect::<Vec<_>>();
    for (name, _) in environment
        .iter()
        .filter(|(name, _)| is_legacy_business_env(name))
    {
        tracing::warn!(variable = %name, "legacy business environment variable is deprecated; use ALETHEON__ typed configuration");
    }
    let mut loaded = load_layered(project_dir, environment, std::iter::empty())?;
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

/// Convert supported legacy business variables into the regular typed
/// environment layer. A native `ALETHEON__...` value always wins. Secret values
/// are never copied into configuration: only their environment-variable names
/// become `SecretRef`s.
fn normalize_legacy_environment(
    environment: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut values = environment.into_iter().collect::<Vec<_>>();
    let present = values
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<std::collections::HashSet<_>>();
    let legacy_drive_files = values
        .iter()
        .find(|(name, _)| name == "ALETHEON_GOOGLE_DRIVE_FILE_IDS")
        .map(|(_, value)| {
            toml::Value::Array(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| toml::Value::String(value.to_owned()))
                    .collect(),
            )
            .to_string()
        });
    let mut add = |legacy: &str, typed: &str, value: Option<String>| {
        if present.contains(legacy) && !present.contains(typed) {
            if let Some(value) = value.or_else(|| {
                values
                    .iter()
                    .find(|(name, _)| name == legacy)
                    .map(|(_, value)| value.clone())
            }) {
                values.push((typed.to_string(), value));
            }
        }
    };

    add(
        "AGENT_WORKING_DIR",
        "ALETHEON__BOOTSTRAP__WORKING_DIR",
        None,
    );
    add("AGENT_DATA_DIR", "ALETHEON__BOOTSTRAP__DATA_DIR", None);
    add(
        "AGENT_SYSTEM_PROMPT",
        "ALETHEON__AGENT__SYSTEM_PROMPT",
        None,
    );
    add(
        "AGENT_SANDBOX_PREFERENCE",
        "ALETHEON__BOOTSTRAP__SANDBOX_PREFERENCE",
        None,
    );
    add(
        "ALETHEON_CONSCIOUS_ARBITRATION_MODE",
        "ALETHEON__BOOTSTRAP__CONSCIOUS_ARBITRATION_MODE",
        None,
    );
    add(
        "ALETHEON_GOOGLE_CLIENT_ID",
        "ALETHEON__INTEGRATIONS__GOOGLE__CLIENT_ID",
        None,
    );
    add(
        "ALETHEON_GOOGLE_CLIENT_ID",
        "ALETHEON__DEPLOYMENT__INTEGRATIONS__GOOGLE",
        Some("true".into()),
    );
    add(
        "ALETHEON_GOOGLE_REDIRECT_URI",
        "ALETHEON__INTEGRATIONS__GOOGLE__REDIRECT_URI",
        None,
    );
    add(
        "ALETHEON_GOOGLE_CLIENT_SECRET",
        "ALETHEON__INTEGRATIONS__GOOGLE__CLIENT_SECRET__ENV",
        Some("ALETHEON_GOOGLE_CLIENT_SECRET".into()),
    );
    add(
        "ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED",
        "ALETHEON__INTEGRATIONS__GOOGLE__DRIVE_SYNC_ENABLED",
        None,
    );
    add(
        "ALETHEON_GOOGLE_DRIVE_FILE_IDS",
        "ALETHEON__INTEGRATIONS__GOOGLE__DRIVE_FILE_IDS",
        legacy_drive_files,
    );
    add(
        "ALETHEON_GMAIL_INGRESS_POLICY_FILE",
        "ALETHEON__INTEGRATIONS__GOOGLE__GMAIL_INGRESS_POLICY_FILE",
        None,
    );
    add(
        "SEARCH_API_URL",
        "ALETHEON__INTEGRATIONS__SEARCH__API_URL",
        None,
    );
    add(
        "SEARCH_API_URL",
        "ALETHEON__INTEGRATIONS__SEARCH__ENABLED",
        Some("true".into()),
    );
    add(
        "SEARCH_API_KEY",
        "ALETHEON__INTEGRATIONS__SEARCH__API_KEY__ENV",
        Some("SEARCH_API_KEY".into()),
    );
    values
}

fn is_legacy_business_env(name: &str) -> bool {
    matches!(
        name,
        "AGENT_WORKING_DIR"
            | "AGENT_DATA_DIR"
            | "AGENT_SYSTEM_PROMPT"
            | "AGENT_SANDBOX_PREFERENCE"
            | "ALETHEON_CONSCIOUS_ARBITRATION_MODE"
            | "ALETHEON_GOOGLE_CLIENT_ID"
            | "ALETHEON_GOOGLE_CLIENT_SECRET"
            | "ALETHEON_GOOGLE_REDIRECT_URI"
            | "ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED"
            | "ALETHEON_GOOGLE_DRIVE_FILE_IDS"
            | "ALETHEON_GMAIL_INGRESS_POLICY_FILE"
            | "SEARCH_API_URL"
            | "SEARCH_API_KEY"
    )
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

#[cfg(test)]
mod legacy_environment_tests {
    use super::{merge_layers, normalize_legacy_environment, ConfigLayer};
    use crate::composition::config::{ConfigSource, ConfigSourceKind};
    use std::collections::HashMap;

    #[test]
    fn native_typed_environment_wins_over_legacy_alias() {
        let normalized = normalize_legacy_environment([
            ("AGENT_WORKING_DIR".into(), "/legacy".into()),
            ("ALETHEON__BOOTSTRAP__WORKING_DIR".into(), "/typed".into()),
        ]);
        let values = normalized.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(values["ALETHEON__BOOTSTRAP__WORKING_DIR"], "/typed");
    }

    #[test]
    fn legacy_secrets_become_references_not_config_values() {
        let normalized = normalize_legacy_environment([
            ("SEARCH_API_URL".into(), "https://search.example".into()),
            ("SEARCH_API_KEY".into(), "do-not-copy-me".into()),
        ]);
        let values = normalized.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            values["ALETHEON__INTEGRATIONS__SEARCH__API_KEY__ENV"],
            "SEARCH_API_KEY"
        );
        assert!(!values
            .iter()
            .filter(|(name, _)| name.starts_with("ALETHEON__"))
            .any(|(_, value)| value == "do-not-copy-me"));
    }

    #[test]
    fn legacy_drive_file_csv_becomes_typed_array() {
        let normalized = normalize_legacy_environment([(
            "ALETHEON_GOOGLE_DRIVE_FILE_IDS".into(),
            "first, second".into(),
        )]);
        let values = normalized.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            values["ALETHEON__INTEGRATIONS__GOOGLE__DRIVE_FILE_IDS"],
            "[\"first\", \"second\"]"
        );
    }

    #[test]
    fn legacy_file_aliases_merge_with_canonical_defaults() {
        let value = toml::from_str(
            r#"
                [memory.gbrain]
                source = "legacy-source"
                timeout_ms = 250
                [deployment.integrations]
                gbrain = true
                [deployment.secrets]
                gbrain = "/run/secrets/supplemental.env"
                [deployment.quotas]
                gbrain_spool_items = 42
            "#,
        )
        .unwrap();
        let loaded = merge_layers([ConfigLayer {
            source: ConfigSource::new(ConfigSourceKind::System, "legacy fixture"),
            value,
        }])
        .unwrap();

        let memory = &loaded.value.memory;
        assert_eq!(memory.supplemental.write_source, "legacy-source");
        assert_eq!(memory.supplemental.request_timeout_ms, 250);
        assert!(loaded.value.deployment.integrations.supplemental_memory);
        assert_eq!(loaded.value.deployment.quotas.supplemental_spool_items, 42);
    }
}
