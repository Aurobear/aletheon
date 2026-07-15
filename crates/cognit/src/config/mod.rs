//! Configuration types shared between brain-core and runtime.
//!
//! These types were originally in the core crate, then moved to aletheon-runtime.
//! Duplicated here to break the cyclic dependency (brain-core <-> runtime).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Dynamic model routing — maps task types to model specs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRoutingConfig {
    /// Default model for general chat (e.g., "mimo/mimo-v2.5-pro").
    pub default: Option<String>,
    /// Model for multimodal inputs (images, audio).
    pub multimodal: Option<String>,
    /// Cheap model for simple tasks, code reading, extraction.
    pub cheap: Option<String>,
    /// Model for complex reasoning tasks.
    pub reasoning: Option<String>,
    /// Model for AutoMemory fact extraction.
    pub auto_memory: Option<String>,
}

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    #[serde(default)]
    pub model_routing: ModelRoutingConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub perception: PerceptionConfig,
    #[serde(default)]
    pub evolution: EvolutionSettings,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub goal_runtime: Option<GoalRuntimeConfig>,
    #[serde(default)]
    pub pi_runtime: PiRuntimeConfig,
    #[serde(default)]
    pub deployment: DeploymentConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    Development,
    #[default]
    User,
    Production,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentPathsConfig {
    pub state_root: PathBuf,
    pub config_root: PathBuf,
    pub runtime_root: PathBuf,
    pub cache_root: PathBuf,
    pub state: PathBuf,
    pub goals: PathBuf,
    pub sessions: PathBuf,
    pub mnemosyne: PathBuf,
    pub artifacts: PathBuf,
    pub worktrees: PathBuf,
    pub audit: PathBuf,
    pub secret_root: PathBuf,
}

impl Default for DeploymentPathsConfig {
    fn default() -> Self {
        Self {
            state_root: "~/.aletheon".into(),
            config_root: "~/.aletheon".into(),
            runtime_root: "/run/aletheon".into(),
            cache_root: "~/.cache/aletheon".into(),
            state: "~/.aletheon/state".into(),
            goals: "~/.aletheon/goals".into(),
            sessions: "~/.aletheon/sessions".into(),
            mnemosyne: "~/.aletheon/memory".into(),
            artifacts: "~/.aletheon/artifacts".into(),
            worktrees: "~/.aletheon/worktrees".into(),
            audit: "~/.aletheon/audit".into(),
            secret_root: "~/.config/aletheon".into(),
        }
    }
}

impl From<fabric::paths::ProductionPaths> for DeploymentPathsConfig {
    fn from(value: fabric::paths::ProductionPaths) -> Self {
        Self {
            state_root: value.state_root,
            config_root: value.config_root,
            runtime_root: value.runtime_root,
            cache_root: value.cache_root,
            state: value.state,
            goals: value.goals,
            sessions: value.sessions,
            mnemosyne: value.mnemosyne,
            artifacts: value.artifacts,
            worktrees: value.worktrees,
            audit: value.audit,
            secret_root: value.secret_root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentQuotaConfig {
    pub total_data_bytes: u64,
    pub total_data_soft_bytes: u64,
    pub total_data_items: u64,
    pub minimum_free_bytes: u64,
    pub artifacts_bytes: u64,
    pub artifacts_soft_bytes: u64,
    pub artifacts_items: u64,
    pub worktrees_bytes: u64,
    pub worktrees_soft_bytes: u64,
    pub worktrees_items: u64,
    pub audit_bytes: u64,
    pub audit_soft_bytes: u64,
    pub audit_items: u64,
    pub sessions_bytes: u64,
    pub sessions_soft_bytes: u64,
    pub sessions_items: u64,
    pub google_bytes: u64,
    pub google_soft_bytes: u64,
    pub google_items: u64,
    pub gbrain_spool_bytes: u64,
    pub gbrain_spool_soft_bytes: u64,
    pub gbrain_spool_items: u64,
}

impl Default for DeploymentQuotaConfig {
    fn default() -> Self {
        Self {
            total_data_bytes: 100 * 1024 * 1024 * 1024,
            total_data_soft_bytes: 85 * 1024 * 1024 * 1024,
            total_data_items: 2_000_000,
            minimum_free_bytes: 5 * 1024 * 1024 * 1024,
            artifacts_bytes: 20 * 1024 * 1024 * 1024,
            artifacts_soft_bytes: 16 * 1024 * 1024 * 1024,
            artifacts_items: 100_000,
            worktrees_bytes: 40 * 1024 * 1024 * 1024,
            worktrees_soft_bytes: 32 * 1024 * 1024 * 1024,
            worktrees_items: 10_000,
            audit_bytes: 5 * 1024 * 1024 * 1024,
            audit_soft_bytes: 4 * 1024 * 1024 * 1024,
            audit_items: 400_000,
            sessions_bytes: 10 * 1024 * 1024 * 1024,
            sessions_soft_bytes: 8 * 1024 * 1024 * 1024,
            sessions_items: 500_000,
            google_bytes: 5 * 1024 * 1024 * 1024,
            google_soft_bytes: 4 * 1024 * 1024 * 1024,
            google_items: 500_000,
            gbrain_spool_bytes: 256 * 1024 * 1024,
            gbrain_spool_soft_bytes: 192 * 1024 * 1024,
            gbrain_spool_items: 10_000,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentIntegrationsConfig {
    pub telegram: bool,
    pub google: bool,
    pub gbrain: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentSecretFilesConfig {
    pub provider: Option<PathBuf>,
    pub telegram: Option<PathBuf>,
    pub google_vault_key: Option<PathBuf>,
    pub gbrain: Option<PathBuf>,
    pub backup_password: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupMode {
    #[default]
    Disabled,
    Local,
    EncryptedRemote,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentBackupConfig {
    pub mode: BackupMode,
    pub repository_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentHealthConfig {
    pub minimum_free_bytes: u64,
    pub maximum_backup_age_secs: u64,
    pub maximum_sync_lag_secs: u64,
}

impl Default for DeploymentHealthConfig {
    fn default() -> Self {
        Self {
            minimum_free_bytes: 5 * 1024 * 1024 * 1024,
            maximum_backup_age_secs: 36 * 60 * 60,
            maximum_sync_lag_secs: 60 * 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentConfig {
    pub mode: DeploymentMode,
    pub paths: DeploymentPathsConfig,
    pub quotas: DeploymentQuotaConfig,
    pub integrations: DeploymentIntegrationsConfig,
    pub secrets: DeploymentSecretFilesConfig,
    pub backup: DeploymentBackupConfig,
    pub health: DeploymentHealthConfig,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            mode: DeploymentMode::User,
            paths: DeploymentPathsConfig::default(),
            quotas: DeploymentQuotaConfig::default(),
            integrations: DeploymentIntegrationsConfig::default(),
            secrets: DeploymentSecretFilesConfig::default(),
            backup: DeploymentBackupConfig::default(),
            health: DeploymentHealthConfig::default(),
        }
    }
}

impl DeploymentConfig {
    pub fn production() -> Self {
        Self {
            mode: DeploymentMode::Production,
            paths: fabric::paths::ProductionPaths::default().into(),
            ..Self::default()
        }
    }

    pub fn validate(&self, require_existing: bool) -> Result<(), String> {
        if self.mode != DeploymentMode::Production {
            return Ok(());
        }
        let paths = fabric::paths::ProductionPaths {
            state_root: self.paths.state_root.clone(),
            config_root: self.paths.config_root.clone(),
            runtime_root: self.paths.runtime_root.clone(),
            cache_root: self.paths.cache_root.clone(),
            state: self.paths.state.clone(),
            goals: self.paths.goals.clone(),
            sessions: self.paths.sessions.clone(),
            mnemosyne: self.paths.mnemosyne.clone(),
            artifacts: self.paths.artifacts.clone(),
            worktrees: self.paths.worktrees.clone(),
            audit: self.paths.audit.clone(),
            secret_root: self.paths.secret_root.clone(),
        };
        paths
            .validate(require_existing)
            .map_err(|error| error.to_string())?;
        for path in [
            self.secrets.provider.as_ref(),
            self.secrets.telegram.as_ref(),
            self.secrets.google_vault_key.as_ref(),
            self.secrets.gbrain.as_ref(),
            self.secrets.backup_password.as_ref(),
            self.backup.repository_file.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !path.is_absolute()
                || path.to_string_lossy().contains('~')
                || !path.starts_with(&self.paths.config_root)
            {
                return Err("production secret/reference path is outside /etc/aletheon".into());
            }
        }
        if self.quotas.minimum_free_bytes > self.quotas.total_data_bytes
            || self.health.minimum_free_bytes > self.quotas.total_data_bytes
        {
            return Err("deployment free-space thresholds exceed total quota".into());
        }
        for (soft, hard, items) in [
            (
                self.quotas.total_data_soft_bytes,
                self.quotas.total_data_bytes,
                self.quotas.total_data_items,
            ),
            (
                self.quotas.artifacts_soft_bytes,
                self.quotas.artifacts_bytes,
                self.quotas.artifacts_items,
            ),
            (
                self.quotas.worktrees_soft_bytes,
                self.quotas.worktrees_bytes,
                self.quotas.worktrees_items,
            ),
            (
                self.quotas.audit_soft_bytes,
                self.quotas.audit_bytes,
                self.quotas.audit_items,
            ),
            (
                self.quotas.sessions_soft_bytes,
                self.quotas.sessions_bytes,
                self.quotas.sessions_items,
            ),
            (
                self.quotas.google_soft_bytes,
                self.quotas.google_bytes,
                self.quotas.google_items,
            ),
            (
                self.quotas.gbrain_spool_soft_bytes,
                self.quotas.gbrain_spool_bytes,
                self.quotas.gbrain_spool_items,
            ),
        ] {
            if soft > hard || hard == 0 || items == 0 {
                return Err("deployment storage quota is invalid".into());
            }
        }
        Ok(())
    }
}

/// Fail-closed configuration for the isolated Pi coding runtime.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiRuntimeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub executable: PathBuf,
    #[serde(default)]
    pub trusted_executable_dir: Option<PathBuf>,
    #[serde(default)]
    pub fixed_args: Vec<String>,
    #[serde(default)]
    pub worktree_base: PathBuf,
    #[serde(default = "default_pi_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_pi_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub allowed_paths: Vec<PathBuf>,
    #[serde(default)]
    pub forbidden_paths: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub require_namespace_isolation: bool,
    /// Pi has no network access by default and initial M4 rejects enabling it.
    #[serde(default)]
    pub network_enabled: bool,
}

impl Default for PiRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable: PathBuf::new(),
            trusted_executable_dir: None,
            fixed_args: Vec::new(),
            worktree_base: PathBuf::new(),
            timeout_ms: default_pi_timeout_ms(),
            max_output_bytes: default_pi_max_output_bytes(),
            allowed_paths: Vec::new(),
            forbidden_paths: Vec::new(),
            require_namespace_isolation: true,
            network_enabled: false,
        }
    }
}

impl fmt::Debug for PiRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PiRuntimeConfig")
            .field("enabled", &self.enabled)
            .field("executable", &self.executable)
            .field("trusted_executable_dir", &self.trusted_executable_dir)
            .field("fixed_arg_count", &self.fixed_args.len())
            .field("worktree_base", &self.worktree_base)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("allowed_paths", &self.allowed_paths)
            .field("forbidden_paths", &self.forbidden_paths)
            .field(
                "require_namespace_isolation",
                &self.require_namespace_isolation,
            )
            .field("network_enabled", &self.network_enabled)
            .finish()
    }
}

fn default_pi_timeout_ms() -> u64 {
    30 * 60 * 1_000
}

fn default_pi_max_output_bytes() -> usize {
    1024 * 1024
}

/// Provider/model routing for durable Goal worker attempts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalRuntimeConfig {
    /// Enables autonomous Goal attempts. Both worker and reviewer are required.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub worker: Option<RoleRuntimeConfig>,
    #[serde(default)]
    pub reviewer: Option<RoleRuntimeConfig>,
}

/// One cognitive role mapped to a stable runtime and strict model alias/spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleRuntimeConfig {
    pub runtime_id: String,
    /// A key from `[model_aliases]` or an explicit `provider/model` spec.
    pub model_alias: String,
    #[serde(default = "default_role_runtime_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_role_runtime_max_persisted_bytes")]
    pub max_persisted_bytes: usize,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

fn default_role_runtime_max_steps() -> usize {
    8
}

fn default_role_runtime_max_persisted_bytes() -> usize {
    16 * 1024
}

/// Agent-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_true")]
    pub compaction_enabled: bool,
    #[serde(default = "default_compaction_keep_recent")]
    pub compaction_keep_recent: usize,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: usize,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_provider: None,
            default_model: None,
            max_iterations: default_max_iterations(),
            max_tokens: default_max_tokens(),
            compaction_enabled: true,
            compaction_keep_recent: default_compaction_keep_recent(),
            compaction_threshold: default_compaction_threshold(),
            system_prompt: default_system_prompt(),
        }
    }
}

/// 0 means "no iteration cap" — termination then relies on the LLM stopping,
/// the circuit breaker, repeated-call detection, and the tool budget.
fn default_max_iterations() -> usize {
    0
}
fn default_max_tokens() -> usize {
    100_000
}
fn default_true() -> bool {
    true
}
fn default_compaction_keep_recent() -> usize {
    10
}
fn default_compaction_threshold() -> usize {
    30
}

fn default_system_prompt() -> String {
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user. \
     Before stating any conclusion about your own runtime state, logs, or configuration, \
     you MUST read the actual logs and the actually-effective config file first — never guess \
     or invent an explanation."
        .to_string()
}

/// Wire protocol between client and LLM server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Transport {
    Openai,
    Anthropic,
    #[default]
    Auto,
}

/// Per-provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub models: Vec<String>,
    /// Override the default max context length for this provider's models.
    /// If not set, the provider uses its built-in default (128K for OpenAI, 200K for Anthropic).
    #[serde(default)]
    pub max_context_length: Option<usize>,
    /// Optional static pricing for per-provider cost accounting. `None` = unpriced.
    #[serde(default)]
    pub pricing: Option<ProviderPricing>,
}

/// Optional static per-provider pricing (USD per 1K tokens) for cost accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPricing {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}

// ---------------------------------------------------------------------------
// New config sub-structs
// ---------------------------------------------------------------------------

/// Sandbox execution preference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// "auto", "require", or "forbid"
    #[serde(default = "default_sandbox_preference")]
    pub preference: String,
    #[serde(default)]
    pub bubblewrap_path: Option<String>,
}

fn default_sandbox_preference() -> String {
    "auto".to_string()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            preference: default_sandbox_preference(),
            bubblewrap_path: None,
        }
    }
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// "stdio", "http", or "sse"
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// For stdio transport: command to run
    #[serde(default)]
    pub command: Option<String>,
    /// For http/sse transport: URL to connect to
    #[serde(default)]
    pub url: Option<String>,
    /// Environment variable containing the bearer token (never the token itself).
    #[serde(default)]
    pub bearer_token_env: Option<String>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: default_mcp_transport(),
            command: None,
            url: None,
            bearer_token_env: None,
        }
    }
}

/// Plugin directories.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsConfig {
    #[serde(default)]
    pub directories: Vec<String>,
}

/// Memory backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// "sqlite" or "in_memory"
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
    /// Optional gbrain shared memory integration (disabled by default).
    #[serde(default)]
    pub gbrain: GbrainMemoryConfig,
}

fn default_memory_backend() -> String {
    "sqlite".to_string()
}
fn default_memory_data_dir() -> String {
    "~/.aletheon/memory".to_string()
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
            data_dir: default_memory_data_dir(),
            gbrain: GbrainMemoryConfig::default(),
        }
    }
}
/// Optional GBrain supplemental memory over the configured HTTP MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GbrainMemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gbrain_server_name")]
    pub server_name: String,
    #[serde(default = "default_gbrain_read_sources")]
    pub read_sources: Vec<String>,
    #[serde(default = "default_gbrain_source", alias = "source")]
    pub write_source: String,
    #[serde(default = "default_gbrain_timeout_ms", alias = "timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_gbrain_batch_size")]
    pub delivery_batch_size: usize,
    #[serde(default = "default_gbrain_max_results", alias = "max_results")]
    pub recall_limit: usize,
    #[serde(default = "default_gbrain_max_chars", alias = "max_chars")]
    pub max_content_bytes: usize,
    #[serde(default, alias = "capture_enabled")]
    pub projection_enabled: bool,
    #[serde(default = "default_gbrain_spool_path")]
    pub spool_path: String,
    #[serde(default = "default_gbrain_spool_items")]
    pub spool_max_items: usize,
    #[serde(default = "default_gbrain_spool_bytes")]
    pub spool_max_bytes: u64,
    #[serde(default = "default_gbrain_retry_initial_ms")]
    pub retry_initial_ms: u64,
    #[serde(default = "default_gbrain_retry_max_ms")]
    pub retry_max_ms: u64,
    #[serde(default = "default_gbrain_retry_attempts")]
    pub retry_max_attempts: u32,
    #[serde(default = "default_gbrain_retry_age_secs")]
    pub retry_max_age_secs: u64,
    #[serde(default = "default_gbrain_schema_fixture")]
    pub schema_fixture: String,
    #[serde(default = "default_gbrain_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_gbrain_outbox_dir", alias = "outbox_dir")]
    pub legacy_outbox_dir: String,
}

fn default_gbrain_server_name() -> String {
    "gbrain".into()
}
fn default_gbrain_source() -> String {
    "aletheon".into()
}
fn default_gbrain_read_sources() -> Vec<String> {
    vec!["aletheon".into(), "general".into()]
}
fn default_gbrain_timeout_ms() -> u64 {
    1200
}
fn default_gbrain_batch_size() -> usize {
    20
}
fn default_gbrain_max_results() -> usize {
    4
}
fn default_gbrain_max_chars() -> usize {
    6000
}
fn default_gbrain_spool_path() -> String {
    "~/.aletheon/memory/gbrain-spool.db".into()
}
fn default_gbrain_spool_items() -> usize {
    10_000
}
fn default_gbrain_spool_bytes() -> u64 {
    256 * 1024 * 1024
}
fn default_gbrain_retry_initial_ms() -> u64 {
    1_000
}
fn default_gbrain_retry_max_ms() -> u64 {
    60_000
}
fn default_gbrain_retry_attempts() -> u32 {
    12
}
fn default_gbrain_retry_age_secs() -> u64 {
    86_400
}
fn default_gbrain_schema_fixture() -> String {
    "config/gbrain/tools-schema.json".into()
}
fn default_gbrain_schema_version() -> String {
    "v0.42.59.0".into()
}
fn default_gbrain_outbox_dir() -> String {
    "~/.aletheon/gbrain-outbox".into()
}

impl Default for GbrainMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: default_gbrain_server_name(),
            read_sources: default_gbrain_read_sources(),
            write_source: default_gbrain_source(),
            request_timeout_ms: default_gbrain_timeout_ms(),
            delivery_batch_size: default_gbrain_batch_size(),
            recall_limit: default_gbrain_max_results(),
            max_content_bytes: default_gbrain_max_chars(),
            projection_enabled: false,
            spool_path: default_gbrain_spool_path(),
            spool_max_items: default_gbrain_spool_items(),
            spool_max_bytes: default_gbrain_spool_bytes(),
            retry_initial_ms: default_gbrain_retry_initial_ms(),
            retry_max_ms: default_gbrain_retry_max_ms(),
            retry_max_attempts: default_gbrain_retry_attempts(),
            retry_max_age_secs: default_gbrain_retry_age_secs(),
            schema_fixture: default_gbrain_schema_fixture(),
            schema_version: default_gbrain_schema_version(),
            legacy_outbox_dir: default_gbrain_outbox_dir(),
        }
    }
}

/// Daemon runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_socket_path")]
    pub socket_path: String,
    #[serde(default = "default_daemon_log_level")]
    pub log_level: String,
}

fn default_daemon_socket_path() -> String {
    "/run/aletheond/aletheond.sock".to_string()
}
fn default_daemon_log_level() -> String {
    "info".to_string()
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: default_daemon_socket_path(),
            log_level: default_daemon_log_level(),
        }
    }
}

/// Self-evolution loop settings. Default OFF (HIGH-risk autonomy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSettings {
    /// Master switch for the self-evolution loop.
    /// When false (default), the loop is inert regardless of other settings.
    #[serde(default)] // bool default = false
    pub enabled: bool,
    /// Trigger evolution every N turns.
    #[serde(default = "default_evolution_trigger_every_n_turns")]
    pub trigger_every_n_turns: usize,
}

fn default_evolution_trigger_every_n_turns() -> usize {
    10
}

impl Default for EvolutionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger_every_n_turns: default_evolution_trigger_every_n_turns(),
        }
    }
}

/// Perception subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionConfig {
    /// Master switch. Off by default: the perception→behavior loop is not yet
    /// wired (see roadmap §T3). When false, no watchers are spawned.
    #[serde(default)]
    pub enabled: bool,
    /// Filesystem paths to watch with inotify.
    #[serde(default = "default_perception_watch_paths")]
    pub watch_paths: Vec<String>,
    /// Whether to enable journald log monitoring.
    #[serde(default = "default_true")]
    pub enable_journald: bool,
}

fn default_perception_watch_paths() -> Vec<String> {
    vec!["/etc".to_string(), "/var/log".to_string()]
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            watch_paths: default_perception_watch_paths(),
            enable_journald: true,
        }
    }
}

/// Telegram bot configuration for owner-only control channel.
///
/// The config stores the environment-variable NAME, never the token value itself.
/// The runtime reads the token from that env var at startup.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TelegramConfig {
    /// Master switch. When false (default), the Telegram bot is not started.
    #[serde(default)]
    pub enabled: bool,
    /// Environment variable name that holds the bot token.
    /// Example value: `"ALETHEON_TELEGRAM_BOT_TOKEN"`.
    pub bot_token_env: Option<String>,
    /// Owner's Telegram user ID. Only messages from this user are accepted.
    pub owner_user_id: Option<i64>,
    /// Polling timeout in seconds (clamped to 1–50).
    #[serde(default = "default_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}

fn default_poll_timeout_secs() -> u64 {
    10
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token_env: None,
            owner_user_id: None,
            poll_timeout_secs: default_poll_timeout_secs(),
        }
    }
}

impl TelegramConfig {
    /// Validate the configuration, returning a list of errors.
    /// Also clamps `poll_timeout_secs` to the allowed 1–50 range.
    pub fn validate(&mut self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // Clamp poll timeout
        self.poll_timeout_secs = self.poll_timeout_secs.clamp(1, 50);

        if !self.enabled {
            // Disabled: no token or owner required
            return errors;
        }

        // Enabled: require bot_token_env
        match &self.bot_token_env {
            None => {
                errors.push("telegram.enabled=true but bot_token_env is not set".to_string());
            }
            Some(name) if name.trim().is_empty() => {
                errors.push("telegram.enabled=true but bot_token_env is empty".to_string());
            }
            Some(_) => {}
        }

        // Enabled: require owner_user_id > 0
        match self.owner_user_id {
            None => {
                errors.push("telegram.enabled=true but owner_user_id is not set".to_string());
            }
            Some(id) if id <= 0 => {
                errors.push(format!(
                    "telegram.enabled=true but owner_user_id={} is not positive",
                    id
                ));
            }
            Some(_) => {}
        }

        errors
    }
}

// ---------------------------------------------------------------------------
// AppConfig methods
// ---------------------------------------------------------------------------

impl AppConfig {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::from_file(path).unwrap_or_default()
    }

    /// Merge `other` into `self`. Fields in `other` that are non-default
    /// override `self`. Lists are appended (providers merged by name).
    pub fn merge(&mut self, other: AppConfig) {
        // Agent: override non-default values
        if other.agent.default_provider.is_some() {
            self.agent.default_provider = other.agent.default_provider;
        }
        if other.agent.default_model.is_some() {
            self.agent.default_model = other.agent.default_model;
        }
        if other.agent.max_iterations != default_max_iterations() {
            self.agent.max_iterations = other.agent.max_iterations;
        }
        if other.agent.max_tokens != default_max_tokens() {
            self.agent.max_tokens = other.agent.max_tokens;
        }
        if other.agent.compaction_keep_recent != default_compaction_keep_recent() {
            self.agent.compaction_keep_recent = other.agent.compaction_keep_recent;
        }
        if other.agent.compaction_threshold != default_compaction_threshold() {
            self.agent.compaction_threshold = other.agent.compaction_threshold;
        }
        if other.agent.system_prompt != default_system_prompt() {
            self.agent.system_prompt = other.agent.system_prompt;
        }
        if !other.agent.compaction_enabled {
            self.agent.compaction_enabled = other.agent.compaction_enabled;
        }

        // Providers: merge by name, append new ones
        for other_provider in other.providers {
            if let Some(existing) = self
                .providers
                .iter_mut()
                .find(|p| p.name == other_provider.name)
            {
                *existing = other_provider;
            } else {
                self.providers.push(other_provider);
            }
        }

        // Model aliases: merge (other wins)
        for (key, value) in other.model_aliases {
            self.model_aliases.insert(key, value);
        }

        // Model routing: override non-default values
        if other.model_routing.default.is_some() {
            self.model_routing.default = other.model_routing.default;
        }
        if other.model_routing.multimodal.is_some() {
            self.model_routing.multimodal = other.model_routing.multimodal;
        }
        if other.model_routing.cheap.is_some() {
            self.model_routing.cheap = other.model_routing.cheap;
        }
        if other.model_routing.reasoning.is_some() {
            self.model_routing.reasoning = other.model_routing.reasoning;
        }
        if other.model_routing.auto_memory.is_some() {
            self.model_routing.auto_memory = other.model_routing.auto_memory;
        }

        // Goal runtimes are an explicit all-or-nothing routing block.
        if other.goal_runtime.is_some() {
            self.goal_runtime = other.goal_runtime;
        }
        if other.pi_runtime != PiRuntimeConfig::default() {
            self.pi_runtime = other.pi_runtime;
        }
        if other.deployment != DeploymentConfig::default() {
            self.deployment = other.deployment;
        }

        // Sandbox: override if non-default
        if other.sandbox.preference != default_sandbox_preference() {
            self.sandbox.preference = other.sandbox.preference;
        }
        if other.sandbox.bubblewrap_path.is_some() {
            self.sandbox.bubblewrap_path = other.sandbox.bubblewrap_path;
        }

        // MCP servers: append
        self.mcp_servers.extend(other.mcp_servers);

        // Plugins: append directories
        self.plugins.directories.extend(other.plugins.directories);

        // Memory: override if non-default
        if other.memory.backend != default_memory_backend() {
            self.memory.backend = other.memory.backend;
        }
        if other.memory.data_dir != default_memory_data_dir() {
            self.memory.data_dir = other.memory.data_dir;
        }
        // GBrain: merge non-default supplemental-memory settings.
        let defaults = GbrainMemoryConfig::default();
        let incoming = other.memory.gbrain;
        if incoming.enabled {
            self.memory.gbrain.enabled = true;
        }
        if incoming.server_name != defaults.server_name {
            self.memory.gbrain.server_name = incoming.server_name;
        }
        if incoming.read_sources != defaults.read_sources {
            self.memory.gbrain.read_sources = incoming.read_sources;
        }
        if incoming.write_source != defaults.write_source {
            self.memory.gbrain.write_source = incoming.write_source;
        }
        if incoming.request_timeout_ms != defaults.request_timeout_ms {
            self.memory.gbrain.request_timeout_ms = incoming.request_timeout_ms;
        }
        if incoming.delivery_batch_size != defaults.delivery_batch_size {
            self.memory.gbrain.delivery_batch_size = incoming.delivery_batch_size;
        }
        if incoming.recall_limit != defaults.recall_limit {
            self.memory.gbrain.recall_limit = incoming.recall_limit;
        }
        if incoming.max_content_bytes != defaults.max_content_bytes {
            self.memory.gbrain.max_content_bytes = incoming.max_content_bytes;
        }
        if incoming.projection_enabled {
            self.memory.gbrain.projection_enabled = true;
        }
        if incoming.spool_path != defaults.spool_path {
            self.memory.gbrain.spool_path = incoming.spool_path;
        }
        if incoming.spool_max_items != defaults.spool_max_items {
            self.memory.gbrain.spool_max_items = incoming.spool_max_items;
        }
        if incoming.spool_max_bytes != defaults.spool_max_bytes {
            self.memory.gbrain.spool_max_bytes = incoming.spool_max_bytes;
        }
        if incoming.retry_initial_ms != defaults.retry_initial_ms {
            self.memory.gbrain.retry_initial_ms = incoming.retry_initial_ms;
        }
        if incoming.retry_max_ms != defaults.retry_max_ms {
            self.memory.gbrain.retry_max_ms = incoming.retry_max_ms;
        }
        if incoming.retry_max_attempts != defaults.retry_max_attempts {
            self.memory.gbrain.retry_max_attempts = incoming.retry_max_attempts;
        }
        if incoming.retry_max_age_secs != defaults.retry_max_age_secs {
            self.memory.gbrain.retry_max_age_secs = incoming.retry_max_age_secs;
        }
        if incoming.schema_fixture != defaults.schema_fixture {
            self.memory.gbrain.schema_fixture = incoming.schema_fixture;
        }
        if incoming.schema_version != defaults.schema_version {
            self.memory.gbrain.schema_version = incoming.schema_version;
        }
        if incoming.legacy_outbox_dir != defaults.legacy_outbox_dir {
            self.memory.gbrain.legacy_outbox_dir = incoming.legacy_outbox_dir;
        }

        // Daemon: override if non-default
        if other.daemon.socket_path != default_daemon_socket_path() {
            self.daemon.socket_path = other.daemon.socket_path;
        }
        if other.daemon.log_level != default_daemon_log_level() {
            self.daemon.log_level = other.daemon.log_level;
        }

        // Perception: override if non-default
        if other.perception.enabled {
            self.perception.enabled = other.perception.enabled;
        }
        if other.perception.watch_paths != default_perception_watch_paths() {
            self.perception.watch_paths = other.perception.watch_paths;
        }
        if !other.perception.enable_journald {
            self.perception.enable_journald = other.perception.enable_journald;
        }

        // Evolution: override if enabled downstream
        if other.evolution.enabled {
            self.evolution.enabled = other.evolution.enabled;
        }

        // Telegram: override if non-default
        if other.telegram.enabled {
            self.telegram.enabled = other.telegram.enabled;
        }
        if other.telegram.bot_token_env.is_some() {
            self.telegram.bot_token_env = other.telegram.bot_token_env;
        }
        if other.telegram.owner_user_id.is_some() {
            self.telegram.owner_user_id = other.telegram.owner_user_id;
        }
        if other.telegram.poll_timeout_secs != default_poll_timeout_secs() {
            self.telegram.poll_timeout_secs = other.telegram.poll_timeout_secs;
        }
    }

    /// Load config with layer merging (low → high precedence):
    /// - Layer 0: compiled defaults
    /// - Layer 1: /etc/aletheon/config.toml   (system defaults)
    /// - Layer 2: ~/.aletheon/config.toml     (user; authoritative for daily edits)
    /// - Layer 3: `<project>/.aletheon/config.toml` (project-local)
    pub fn load_layered(project_dir: Option<&Path>) -> Self {
        let mut config = Self::default();

        // Layer 1: system
        let etc_path = Path::new("/etc/aletheon/config.toml");
        if etc_path.exists() {
            if let Ok(sys_config) = Self::from_file(etc_path) {
                config.merge(sys_config);
            }
        }

        // Layer 2: user global
        let global_path = dirs::home_dir()
            .map(|h| h.join(".aletheon/config.toml"))
            .filter(|p| p.exists());
        if let Some(path) = global_path {
            if let Ok(user_config) = Self::from_file(&path) {
                config.merge(user_config);
            }
        }

        // Layer 3: project local
        if let Some(dir) = project_dir {
            let project_path = dir.join(".aletheon/config.toml");
            if project_path.exists() {
                if let Ok(project_config) = Self::from_file(&project_path) {
                    config.merge(project_config);
                }
            }
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_runtime_defaults_disabled_without_claiming_providers() {
        let config = AppConfig::default();
        assert!(config.goal_runtime.is_none());
    }

    #[test]
    fn goal_runtime_role_routing_parses_and_merges() {
        let parsed: AppConfig = toml::from_str(
            r#"
            [goal_runtime]
            enabled = true

            [goal_runtime.worker]
            runtime_id = "deepseek-worker"
            model_alias = "deepseek"
            max_steps = 6
            allowed_tools = ["file_read", "grep"]

            [goal_runtime.reviewer]
            runtime_id = "escalation-reviewer"
            model_alias = "reviewer/model"
            "#,
        )
        .unwrap();
        let goal_runtime = parsed.goal_runtime.as_ref().unwrap();
        assert!(goal_runtime.enabled);
        assert_eq!(goal_runtime.worker.as_ref().unwrap().max_steps, 6);
        assert_eq!(
            goal_runtime.reviewer.as_ref().unwrap().max_persisted_bytes,
            16 * 1024
        );

        let mut base = AppConfig::default();
        base.merge(parsed.clone());
        assert_eq!(base.goal_runtime, parsed.goal_runtime);
    }

    #[test]
    fn pi_runtime_is_disabled_and_network_denied_by_default() {
        let config = AppConfig::default().pi_runtime;
        assert!(!config.enabled);
        assert!(!config.network_enabled);
        assert!(config.require_namespace_isolation);
    }

    #[test]
    fn pi_runtime_parses_merges_and_debug_omits_fixed_arguments() {
        let parsed: AppConfig = toml::from_str(
            r#"
            [pi_runtime]
            enabled = true
            executable = "/opt/aletheon/bin/pi"
            fixed_args = ["--api-key", "super-secret"]
            worktree_base = "/var/lib/aletheon/pi-worktrees"
            timeout_ms = 42000
            max_output_bytes = 8192
            allowed_paths = ["crates", "Cargo.toml"]
            forbidden_paths = [".git", ".env"]
            "#,
        )
        .unwrap();
        assert_eq!(parsed.pi_runtime.fixed_args.len(), 2);
        assert!(!parsed.pi_runtime.network_enabled);

        let debug = format!("{:?}", parsed.pi_runtime);
        assert!(!debug.contains("super-secret"));
        assert!(debug.contains("fixed_arg_count"));

        let mut base = AppConfig::default();
        base.merge(parsed.clone());
        assert_eq!(base.pi_runtime, parsed.pi_runtime);
    }

    #[test]
    fn provider_pricing_parses_and_defaults_to_none() {
        let with = r#"
            name = "anthropic"
            base_url = "https://api.anthropic.com"
            [pricing]
            input_per_1k = 3.0
            output_per_1k = 15.0
        "#;
        let p: ProviderConfig = toml::from_str(with).unwrap();
        let pr = p.pricing.expect("pricing present");
        assert_eq!(pr.input_per_1k, 3.0);
        assert_eq!(pr.output_per_1k, 15.0);

        let without = "name = \"local\"\nbase_url = \"http://localhost:11434\"\n";
        let p2: ProviderConfig = toml::from_str(without).unwrap();
        assert!(p2.pricing.is_none(), "pricing is optional");
    }

    #[test]
    fn merge_covers_perception_evolution_and_prompt() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.perception.enabled = true;
        other.agent.system_prompt = "OVERRIDDEN".into();
        other.agent.compaction_enabled = false;

        base.merge(other);

        assert!(base.perception.enabled, "perception must merge");
        assert_eq!(base.agent.system_prompt, "OVERRIDDEN");
        assert!(!base.agent.compaction_enabled);
    }

    #[test]
    fn merge_precedence_user_over_system() {
        // Unit-level proxy for layer precedence: later merge wins.
        let mut config = AppConfig::default();
        let mut system = AppConfig::default();
        system.agent.default_model = Some("system-model".into());
        let mut user = AppConfig::default();
        user.agent.default_model = Some("user-model".into());

        config.merge(system);
        config.merge(user);

        assert_eq!(config.agent.default_model.as_deref(), Some("user-model"));
    }

    // ── TelegramConfig validation ──────────────────────────────────────

    #[test]
    fn telegram_disabled_needs_no_token_or_owner() {
        let mut cfg = TelegramConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.bot_token_env.is_none());
        assert!(cfg.owner_user_id.is_none());
        let errors = cfg.validate();
        assert!(
            errors.is_empty(),
            "disabled config must have no errors, got: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_requires_token_env() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: None,
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject missing bot_token_env");
        assert!(
            errors.iter().any(|e| e.contains("bot_token_env")),
            "error must mention bot_token_env: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_rejects_empty_token_env() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("   ".to_string()),
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject empty bot_token_env");
        assert!(
            errors.iter().any(|e| e.contains("bot_token_env")),
            "error must mention bot_token_env: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_requires_owner_user_id() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
            owner_user_id: None,
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject missing owner_user_id");
        assert!(
            errors.iter().any(|e| e.contains("owner_user_id")),
            "error must mention owner_user_id: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_rejects_zero_or_negative_owner_id() {
        for bad_id in [0, -1] {
            let mut cfg = TelegramConfig {
                enabled: true,
                bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
                owner_user_id: Some(bad_id),
                poll_timeout_secs: 10,
            };
            let errors = cfg.validate();
            assert!(!errors.is_empty(), "must reject owner_user_id={bad_id}");
            assert!(
                errors.iter().any(|e| e.contains("not positive")),
                "error must say 'not positive' for id={bad_id}: {errors:?}"
            );
        }
    }

    #[test]
    fn telegram_valid_enabled_passes() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(
            errors.is_empty(),
            "valid config must have no errors, got: {errors:?}"
        );
    }

    #[test]
    fn telegram_clamps_poll_timeout() {
        // Below minimum
        let mut cfg = TelegramConfig {
            enabled: false,
            bot_token_env: None,
            owner_user_id: None,
            poll_timeout_secs: 0,
        };
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 1);

        // Above maximum
        cfg.poll_timeout_secs = 100;
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 50);

        // In range
        cfg.poll_timeout_secs = 30;
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 30);
    }

    #[test]
    fn telegram_parses_from_toml() {
        let toml = r#"
enabled = true
bot_token_env = "ALETHEON_TELEGRAM_BOT_TOKEN"
owner_user_id = 12345
poll_timeout_secs = 20
"#;
        let cfg: TelegramConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(
            cfg.bot_token_env.as_deref(),
            Some("ALETHEON_TELEGRAM_BOT_TOKEN")
        );
        assert_eq!(cfg.owner_user_id, Some(12345));
        assert_eq!(cfg.poll_timeout_secs, 20);
    }

    #[test]
    fn telegram_default_in_app_config() {
        let toml = r#"
[[providers]]
name = "test"
base_url = "http://localhost"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(!config.telegram.enabled, "telegram disabled by default");
        assert_eq!(config.telegram.poll_timeout_secs, 10);
    }

    #[test]
    fn telegram_merge_overrides() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.telegram.enabled = true;
        other.telegram.bot_token_env = Some("MY_TOKEN".into());
        other.telegram.owner_user_id = Some(42);

        base.merge(other);

        assert!(base.telegram.enabled);
        assert_eq!(base.telegram.bot_token_env.as_deref(), Some("MY_TOKEN"));
        assert_eq!(base.telegram.owner_user_id, Some(42));
    }

    #[test]
    fn gbrain_disabled_by_default() {
        let config = AppConfig::default();
        assert!(!config.memory.gbrain.enabled);
        assert_eq!(config.memory.gbrain.server_name, "gbrain");
        assert_eq!(config.memory.gbrain.write_source, "aletheon");
        assert_eq!(config.memory.gbrain.request_timeout_ms, 1200);
        assert_eq!(config.memory.gbrain.recall_limit, 4);
        assert_eq!(config.memory.gbrain.max_content_bytes, 6000);
        assert!(!config.memory.gbrain.projection_enabled);
    }

    #[test]
    fn gbrain_parses_from_toml() {
        let toml = r#"
enabled = true
server_name = "gbrain-primary"
read_sources = ["project", "general"]
write_source = "project"
request_timeout_ms = 2500
delivery_batch_size = 12
recall_limit = 6
max_content_bytes = 8192
projection_enabled = true
spool_path = "/tmp/gbrain.db"
spool_max_items = 500
spool_max_bytes = 1048576
retry_initial_ms = 250
retry_max_ms = 10000
retry_max_attempts = 8
retry_max_age_secs = 3600
schema_fixture = "config/gbrain/tools-schema.json"
schema_version = "v0.42.59.0"
legacy_outbox_dir = "/tmp/legacy"
"#;
        let cfg: GbrainMemoryConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.server_name, "gbrain-primary");
        assert_eq!(cfg.read_sources, ["project", "general"]);
        assert_eq!(cfg.write_source, "project");
        assert_eq!(cfg.request_timeout_ms, 2500);
        assert_eq!(cfg.delivery_batch_size, 12);
        assert_eq!(cfg.spool_max_bytes, 1_048_576);
        assert!(cfg.projection_enabled);
    }

    #[test]
    fn gbrain_nested_in_memory_config() {
        let toml = r#"
backend = "sqlite"
data_dir = "/tmp/mem"

[gbrain]
enabled = true
source = "aletheon"
timeout_ms = 2500
max_results = 6
"#;
        let mem: MemoryConfig = toml::from_str(toml).unwrap();
        assert!(mem.gbrain.enabled);
        assert_eq!(mem.gbrain.write_source, "aletheon");
        assert_eq!(mem.gbrain.request_timeout_ms, 2500);
        assert_eq!(mem.gbrain.server_name, "gbrain");
    }

    #[test]
    fn gbrain_merge_overrides() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.memory.gbrain.enabled = true;
        other.memory.gbrain.write_source = "custom".into();
        other.memory.gbrain.request_timeout_ms = 5000;

        base.merge(other);

        assert!(base.memory.gbrain.enabled);
        assert_eq!(base.memory.gbrain.write_source, "custom");
        assert_eq!(base.memory.gbrain.request_timeout_ms, 5000);
    }

    #[test]
    fn production_deployment_paths_parse_validate_and_merge() {
        let parsed: AppConfig = toml::from_str(
            r#"
            [deployment]
            mode = "production"
            [deployment.paths]
            state_root = "/var/lib/aletheon"
            config_root = "/etc/aletheon"
            runtime_root = "/run/aletheon"
            cache_root = "/var/cache/aletheon"
            state = "/var/lib/aletheon/state"
            goals = "/var/lib/aletheon/goals"
            sessions = "/var/lib/aletheon/sessions"
            mnemosyne = "/var/lib/aletheon/mnemosyne"
            artifacts = "/var/lib/aletheon/artifacts"
            worktrees = "/var/lib/aletheon/worktrees"
            audit = "/var/lib/aletheon/audit"
            secret_root = "/etc/aletheon/credentials"
            [deployment.secrets]
            provider = "/etc/aletheon/credentials/provider.env"
            "#,
        )
        .unwrap();
        assert!(parsed.deployment.validate(false).is_ok());
        let mut base = AppConfig::default();
        base.merge(parsed);
        assert_eq!(base.deployment.mode, DeploymentMode::Production);
    }

    #[test]
    fn production_deployment_rejects_tilde_outside_and_invalid_quota() {
        let mut deployment = DeploymentConfig::production();
        deployment.paths.goals = "~/.aletheon/goals".into();
        assert!(deployment.validate(false).is_err());

        let mut deployment = DeploymentConfig::production();
        deployment.secrets.provider = Some("/tmp/provider.env".into());
        assert!(deployment.validate(false).is_err());

        let mut deployment = DeploymentConfig::production();
        deployment.quotas.minimum_free_bytes = deployment.quotas.total_data_bytes + 1;
        assert!(deployment.validate(false).is_err());
    }

    #[test]
    fn development_and_user_modes_retain_compatible_paths() {
        let user = DeploymentConfig::default();
        assert_eq!(user.mode, DeploymentMode::User);
        assert!(user.paths.state_root.to_string_lossy().starts_with('~'));
        assert!(user.validate(false).is_ok());
        let mut development = user;
        development.mode = DeploymentMode::Development;
        development.paths.state_root = "./target/aletheon".into();
        assert!(development.validate(false).is_ok());
    }
}
