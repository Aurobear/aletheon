//! Product-neutral supplemental-memory deployment configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
    #[serde(default, alias = "gbrain")]
    pub supplemental: SupplementalMemoryConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { backend: default_memory_backend(), data_dir: default_memory_data_dir(), supplemental: Default::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SupplementalMemoryConfig {
    #[serde(default)] pub enabled: bool,
    #[serde(default = "default_server_name")] pub server_name: String,
    #[serde(default = "default_read_sources")] pub read_sources: Vec<String>,
    #[serde(default = "default_source", alias = "source")] pub write_source: String,
    #[serde(default = "default_timeout_ms", alias = "timeout_ms")] pub request_timeout_ms: u64,
    #[serde(default = "default_batch_size")] pub delivery_batch_size: usize,
    #[serde(default = "default_max_results", alias = "max_results")] pub recall_limit: usize,
    #[serde(default = "default_max_chars", alias = "max_chars")] pub max_content_bytes: usize,
    #[serde(default, alias = "capture_enabled")] pub projection_enabled: bool,
    #[serde(default = "default_spool_path")] pub spool_path: String,
    #[serde(default = "default_spool_items")] pub spool_max_items: usize,
    #[serde(default = "default_spool_bytes")] pub spool_max_bytes: u64,
    #[serde(default = "default_retry_initial_ms")] pub retry_initial_ms: u64,
    #[serde(default = "default_retry_max_ms")] pub retry_max_ms: u64,
    #[serde(default = "default_retry_attempts")] pub retry_max_attempts: u32,
    #[serde(default = "default_retry_age_secs")] pub retry_max_age_secs: u64,
    #[serde(default = "default_schema_fixture")] pub schema_fixture: String,
    #[serde(default = "default_schema_version")] pub schema_version: String,
    #[serde(default = "default_outbox_dir", alias = "outbox_dir")] pub legacy_outbox_dir: String,
}

impl Default for SupplementalMemoryConfig {
    fn default() -> Self {
        Self { enabled: false, server_name: default_server_name(), read_sources: default_read_sources(),
            write_source: default_source(), request_timeout_ms: default_timeout_ms(),
            delivery_batch_size: default_batch_size(), recall_limit: default_max_results(),
            max_content_bytes: default_max_chars(), projection_enabled: false,
            spool_path: default_spool_path(), spool_max_items: default_spool_items(),
            spool_max_bytes: default_spool_bytes(), retry_initial_ms: default_retry_initial_ms(),
            retry_max_ms: default_retry_max_ms(), retry_max_attempts: default_retry_attempts(),
            retry_max_age_secs: default_retry_age_secs(), schema_fixture: default_schema_fixture(),
            schema_version: default_schema_version(), legacy_outbox_dir: default_outbox_dir() }
    }
}

fn default_memory_backend() -> String { "sqlite".into() }
fn default_memory_data_dir() -> String { "~/.aletheon/memory".into() }
fn default_server_name() -> String { "gbrain".into() }
fn default_source() -> String { "aletheon".into() }
fn default_read_sources() -> Vec<String> { vec!["aletheon".into(), "general".into()] }
fn default_timeout_ms() -> u64 { 1200 }
fn default_batch_size() -> usize { 20 }
fn default_max_results() -> usize { 4 }
fn default_max_chars() -> usize { 6000 }
fn default_spool_path() -> String { "~/.aletheon/memory/gbrain-spool.db".into() }
fn default_spool_items() -> usize { 10_000 }
fn default_spool_bytes() -> u64 { 256 * 1024 * 1024 }
fn default_retry_initial_ms() -> u64 { 1_000 }
fn default_retry_max_ms() -> u64 { 60_000 }
fn default_retry_attempts() -> u32 { 12 }
fn default_retry_age_secs() -> u64 { 86_400 }
fn default_schema_fixture() -> String { "config/gbrain/tools-schema.json".into() }
fn default_schema_version() -> String { "v0.42.59.0".into() }
fn default_outbox_dir() -> String { "~/.aletheon/gbrain-outbox".into() }

#[deprecated(note = "use SupplementalMemoryConfig")]
pub type McpMemoryConfig = SupplementalMemoryConfig;
#[deprecated(note = "use SupplementalMemoryConfig")]
pub type GbrainMemoryConfig = SupplementalMemoryConfig;
