//! Product-neutral supplemental-memory deployment configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::memory_policy::MemoryPolicyConfig;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
    #[serde(default, alias = "gbrain")]
    pub supplemental: SupplementalMemoryConfig,
    #[serde(default)]
    pub recall: MemoryRecallConfig,
    #[serde(default)]
    pub extraction: MemoryExtractionConfig,
    #[serde(default)]
    pub promotion: MemoryPromotionConfig,
    #[serde(default)]
    pub embedding: MemoryEmbeddingConfig,
    #[serde(default)]
    pub policy: MemoryPolicyConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
            data_dir: default_memory_data_dir(),
            supplemental: Default::default(),
            recall: Default::default(),
            extraction: Default::default(),
            promotion: Default::default(),
            embedding: Default::default(),
            policy: Default::default(),
        }
    }
}

impl MemoryConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.policy.validate()?;
        self.recall.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryEmbeddingConfig {
    pub enabled: bool,
    /// `open_ai` or `ollama`.
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub dimensions: usize,
    pub credential_env: String,
    #[serde(default = "default_embedding_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub rotation_generation: u32,
}

fn default_embedding_timeout_ms() -> u64 {
    400
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SupplementalMemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_server_name")]
    pub server_name: String,
    #[serde(default = "default_read_sources")]
    pub read_sources: Vec<String>,
    #[serde(default = "default_source", alias = "source")]
    pub write_source: String,
    #[serde(default = "default_timeout_ms", alias = "timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_batch_size")]
    pub delivery_batch_size: usize,
    #[serde(default = "default_max_results", alias = "max_results")]
    pub recall_limit: usize,
    #[serde(default = "default_max_chars", alias = "max_chars")]
    pub max_content_bytes: usize,
    #[serde(default, alias = "capture_enabled")]
    pub projection_enabled: bool,
    #[serde(default = "default_spool_path")]
    pub spool_path: String,
    #[serde(default = "default_spool_items")]
    pub spool_max_items: usize,
    #[serde(default = "default_spool_bytes")]
    pub spool_max_bytes: u64,
    #[serde(default = "default_retry_initial_ms")]
    pub retry_initial_ms: u64,
    #[serde(default = "default_retry_max_ms")]
    pub retry_max_ms: u64,
    #[serde(default = "default_retry_attempts")]
    pub retry_max_attempts: u32,
    #[serde(default = "default_retry_age_secs")]
    pub retry_max_age_secs: u64,
    #[serde(default = "default_schema_fixture")]
    pub schema_fixture: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "default_outbox_dir", alias = "outbox_dir")]
    pub legacy_outbox_dir: String,
    /// Non-secret proofs binding opaque MCP destinations to external sources.
    /// OAuth client credentials remain exclusively in the MCP secret config.
    #[serde(default)]
    pub destination_attestations: Vec<SupplementalDestinationAttestationConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SupplementalDestinationAttestationConfig {
    pub destination_handle: String,
    pub source_id: String,
    pub marker_slug: String,
    pub marker_sha256: String,
    #[serde(default = "default_attestation_revalidate_secs")]
    pub revalidate_after_secs: u64,
}

fn default_attestation_revalidate_secs() -> u64 {
    300
}

impl Default for SupplementalMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: default_server_name(),
            read_sources: default_read_sources(),
            write_source: default_source(),
            request_timeout_ms: default_timeout_ms(),
            delivery_batch_size: default_batch_size(),
            recall_limit: default_max_results(),
            max_content_bytes: default_max_chars(),
            projection_enabled: false,
            spool_path: default_spool_path(),
            spool_max_items: default_spool_items(),
            spool_max_bytes: default_spool_bytes(),
            retry_initial_ms: default_retry_initial_ms(),
            retry_max_ms: default_retry_max_ms(),
            retry_max_attempts: default_retry_attempts(),
            retry_max_age_secs: default_retry_age_secs(),
            schema_fixture: default_schema_fixture(),
            schema_version: default_schema_version(),
            legacy_outbox_dir: default_outbox_dir(),
            destination_attestations: Vec::new(),
        }
    }
}

fn default_memory_backend() -> String {
    "sqlite".into()
}
fn default_memory_data_dir() -> String {
    "~/.aletheon/memory".into()
}
fn default_server_name() -> String {
    "supplemental".into()
}
fn default_source() -> String {
    "aletheon".into()
}
fn default_read_sources() -> Vec<String> {
    vec!["aletheon".into(), "general".into()]
}
fn default_timeout_ms() -> u64 {
    1200
}
fn default_batch_size() -> usize {
    20
}
fn default_max_results() -> usize {
    4
}
fn default_max_chars() -> usize {
    6000
}
fn default_spool_path() -> String {
    "~/.aletheon/memory/memory-spool.db".into()
}
fn default_spool_items() -> usize {
    10_000
}
fn default_spool_bytes() -> u64 {
    256 * 1024 * 1024
}
fn default_retry_initial_ms() -> u64 {
    1_000
}
fn default_retry_max_ms() -> u64 {
    60_000
}
fn default_retry_attempts() -> u32 {
    12
}
fn default_retry_age_secs() -> u64 {
    86_400
}
fn default_schema_fixture() -> String {
    "".into()
}
fn default_schema_version() -> String {
    "v0.42.59.0".into()
}
fn default_outbox_dir() -> String {
    "~/.aletheon/memory-outbox".into()
}

// ── Memory recall / extraction / promotion config ───────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecallConfig {
    #[serde(default = "default_recall_enabled")]
    pub enabled: bool,
    #[serde(default = "default_inject")]
    pub inject_into_context: bool,
    #[serde(default = "default_recall_max_items")]
    pub max_items: usize,
    #[serde(default = "default_recall_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_recall_timeout_ms")]
    pub timeout_ms: u64,
    /// Independently disable reuse while retaining authoritative recall.
    #[serde(default = "default_recall_cache_enabled")]
    pub cache_enabled: bool,
    /// Recall-cache capacity (number of cache entries) and TTL in seconds.
    /// Bounded with safe defaults so the cache cannot grow without a configurable
    /// ceiling or serve stale recalls forever (C1-AUDIT-003).
    #[serde(default = "default_recall_cache_capacity")]
    #[schemars(range(min = 1, max = 4096))]
    pub cache_capacity: usize,
    #[serde(default = "default_recall_cache_ttl_seconds")]
    #[schemars(range(min = 1, max = 3600))]
    pub cache_ttl_seconds: u64,
}

impl Default for MemoryRecallConfig {
    fn default() -> Self {
        Self {
            enabled: default_recall_enabled(),
            inject_into_context: default_inject(),
            max_items: default_recall_max_items(),
            max_bytes: default_recall_max_bytes(),
            timeout_ms: default_recall_timeout_ms(),
            cache_enabled: default_recall_cache_enabled(),
            cache_capacity: default_recall_cache_capacity(),
            cache_ttl_seconds: default_recall_cache_ttl_seconds(),
        }
    }
}

impl MemoryRecallConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=4096).contains(&self.cache_capacity),
            "memory.recall.cache_capacity must be within 1..=4096"
        );
        anyhow::ensure!(
            (1..=3600).contains(&self.cache_ttl_seconds),
            "memory.recall.cache_ttl_seconds must be within 1..=3600"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryExtractionConfig {
    #[serde(default = "default_extraction_enabled")]
    pub enabled: bool,
    #[serde(default = "default_extraction_mode")]
    pub mode: String,
    #[serde(default = "default_max_facts")]
    pub max_facts_per_turn: usize,
}

impl Default for MemoryExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: default_extraction_enabled(),
            mode: default_extraction_mode(),
            max_facts_per_turn: default_max_facts(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryPromotionConfig {
    #[serde(default = "default_promotion_enabled")]
    pub enabled: bool,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    #[serde(default = "default_max_promoted")]
    pub max_promoted_facts: usize,
}

impl Default for MemoryPromotionConfig {
    fn default() -> Self {
        Self {
            enabled: default_promotion_enabled(),
            min_confidence: default_min_confidence(),
            max_promoted_facts: default_max_promoted(),
        }
    }
}

fn default_promotion_enabled() -> bool {
    true
}

fn default_recall_enabled() -> bool {
    true
}
fn default_inject() -> bool {
    true
}
fn default_recall_max_items() -> usize {
    4
}
fn default_recall_max_bytes() -> usize {
    65536
}
fn default_recall_timeout_ms() -> u64 {
    3_000
}
fn default_recall_cache_enabled() -> bool {
    true
}
fn default_recall_cache_capacity() -> usize {
    512
}
fn default_recall_cache_ttl_seconds() -> u64 {
    30
}
fn default_extraction_enabled() -> bool {
    true
}
fn default_extraction_mode() -> String {
    "local".into()
}
fn default_max_facts() -> usize {
    5
}
fn default_min_confidence() -> f64 {
    0.7
}
fn default_max_promoted() -> usize {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_cache_defaults_are_enabled_and_bounded() {
        let config = MemoryRecallConfig::default();
        assert!(config.cache_enabled);
        config.validate().unwrap();
    }

    #[test]
    fn recall_cache_rejects_capacity_and_ttl_outside_governed_limits() {
        for config in [
            MemoryRecallConfig {
                cache_capacity: 0,
                ..MemoryRecallConfig::default()
            },
            MemoryRecallConfig {
                cache_capacity: 4097,
                ..MemoryRecallConfig::default()
            },
            MemoryRecallConfig {
                cache_ttl_seconds: 0,
                ..MemoryRecallConfig::default()
            },
            MemoryRecallConfig {
                cache_ttl_seconds: 3601,
                ..MemoryRecallConfig::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
    }
}
