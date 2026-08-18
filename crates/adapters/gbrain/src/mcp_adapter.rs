//! Bounded adapter over the retained Corpus MCP manager.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use corpus::tools::mcp::manager::McpManager;
use mnemosyne::supplemental::page::{MAX_PAGE_BYTES, PAGE_SCHEMA_VERSION};
use mnemosyne::supplemental::{validate_tools_list, SupplementalDocument};
use mnemosyne::{
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity,
    RecallItem, RecallSet, SupplementalCapabilityGrant, TemporalState, WorkspaceMemoryBinding,
    WorkspaceMemoryBindingState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use mnemosyne::supplemental_memory::SupplementalDestinationAttestationConfig;

const MAX_TOOL_TEXT_BYTES: usize = 256 * 1024;
const MAX_SLUG_BYTES: usize = 512;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const MAX_RESULTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplementalAdapterErrorCategory {
    Auth,
    Schema,
    InvalidPage,
    RejectedArguments,
    Timeout,
    Cancelled,
    RateLimited,
    Provider,
    Transport,
    MalformedResponse,
    OversizedResponse,
}

impl SupplementalAdapterErrorCategory {
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Cancelled | Self::RateLimited | Self::Provider | Self::Transport
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Supplemental MCP {category:?}: {message}")]
pub struct SupplementalAdapterError {
    pub category: SupplementalAdapterErrorCategory,
    message: &'static str,
}

impl SupplementalAdapterError {
    fn new(category: SupplementalAdapterErrorCategory, message: &'static str) -> Self {
        Self { category, message }
    }
    pub fn sanitized_message(&self) -> &'static str {
        self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplementalHealthState {
    Healthy,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplementalSchemaStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplementalHealth {
    pub state: SupplementalHealthState,
    pub schema: SupplementalSchemaStatus,
    pub last_error_category: Option<SupplementalAdapterErrorCategory>,
    pub consecutive_failures: u64,
    pub last_success_unix_ms: Option<i64>,
    pub queue_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupplementalSearchHit {
    pub source_id: String,
    pub slug: String,
    pub content: String,
    pub score: f64,
}

pub struct SupplementalMcpAdapter {
    manager: Arc<McpManager>,
    server_name: String,
    timeout: Duration,
    health: Mutex<SupplementalHealth>,
    destination_attestations: Arc<BTreeMap<String, SupplementalDestinationAttestationConfig>>,
    destination_verified_at: Arc<Mutex<BTreeMap<String, Instant>>>,
}

pub struct McpSupplementalBindingNegotiator {
    manager: Arc<McpManager>,
    timeout: Duration,
    destination_attestations: Arc<BTreeMap<String, SupplementalDestinationAttestationConfig>>,
    destination_verified_at: Mutex<BTreeMap<String, Instant>>,
    read_only_destinations: BTreeSet<String>,
}

impl McpSupplementalBindingNegotiator {
    pub fn new(
        manager: Arc<McpManager>,
        timeout: Duration,
        destination_attestations: &[SupplementalDestinationAttestationConfig],
    ) -> anyhow::Result<Self> {
        Ok(Self {
            manager,
            timeout,
            destination_attestations: Arc::new(attestation_map(destination_attestations)?),
            destination_verified_at: Mutex::new(BTreeMap::new()),
            read_only_destinations: BTreeSet::new(),
        })
    }

    pub fn with_read_only_destinations(
        mut self,
        destinations: impl IntoIterator<Item = String>,
    ) -> Self {
        self.read_only_destinations = destinations.into_iter().collect();
        self
    }
}

#[async_trait::async_trait]
impl mnemosyne::SupplementalBindingNegotiator for McpSupplementalBindingNegotiator {
    async fn negotiate(
        &self,
        destination_handle: &str,
        backend_id: &str,
        expected_source: &str,
    ) -> anyhow::Result<SupplementalCapabilityGrant> {
        let policy = self
            .destination_attestations
            .get(destination_handle)
            .ok_or_else(|| anyhow::anyhow!("supplemental destination has no attestation policy"))?;
        anyhow::ensure!(
            policy.source_id == expected_source,
            "supplemental destination source policy does not match binding"
        );
        if recently_verified(
            &self.destination_verified_at,
            destination_handle,
            policy.revalidate_after_secs,
        ) {
            return Ok(
                if self.read_only_destinations.contains(destination_handle) {
                    read_only_attested_grant(backend_id, expected_source)
                } else {
                    attested_grant(backend_id, expected_source)
                },
            );
        }
        let adapter = SupplementalMcpAdapter::new(
            self.manager.clone(),
            destination_handle.to_owned(),
            self.timeout,
        );
        let grant = if self.read_only_destinations.contains(destination_handle) {
            adapter
                .negotiate_read_attested(backend_id, policy, &CancellationToken::new())
                .await
        } else {
            adapter
                .negotiate_attested(backend_id, policy, &CancellationToken::new())
                .await
        };
        grant.map_err(anyhow::Error::from).inspect(|_| {
            mark_verified(&self.destination_verified_at, destination_handle);
        })
    }
}

#[async_trait::async_trait]
impl mnemosyne::SupplementalBindingRecallPort for McpSupplementalBindingNegotiator {
    async fn recall(
        &self,
        binding: &WorkspaceMemoryBinding,
        request: &mnemosyne::RecallRequest,
    ) -> anyhow::Result<RecallSet> {
        anyhow::ensure!(
            binding.state == WorkspaceMemoryBindingState::Active
                && binding.verified_capability_digest.is_some(),
            "supplemental workspace binding is not active"
        );
        anyhow::ensure!(
            binding.read_destination_handles.len() == binding.expected_read_sources.len(),
            "supplemental read destinations are not paired with expected sources"
        );
        let cancel = CancellationToken::new();
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut degraded = false;
        for (handle, expected_source) in binding
            .read_destination_handles
            .iter()
            .zip(&binding.expected_read_sources)
        {
            let read_only_destination = self.read_only_destinations.contains(handle);
            let grant = match mnemosyne::SupplementalBindingNegotiator::negotiate(
                self,
                handle,
                &binding.backend_id,
                expected_source,
            )
            .await
            {
                Ok(grant) => grant,
                Err(_) => {
                    degraded = true;
                    continue;
                }
            };
            let adapter =
                SupplementalMcpAdapter::new(self.manager.clone(), handle.clone(), self.timeout);
            for source in grant.read_sources.iter() {
                let hits = match adapter
                    .query(
                        &request.query,
                        source,
                        request.max_items.min(MAX_RESULTS),
                        &cancel,
                    )
                    .await
                {
                    Ok(hits) => hits,
                    Err(_) => {
                        degraded = true;
                        continue;
                    }
                };
                for hit in hits {
                    if hit.source_id != *source
                        || !seen.insert((hit.source_id.clone(), hit.slug.clone()))
                    {
                        continue;
                    }
                    let content = if hit.content.starts_with("---\n") {
                        hit.content.clone()
                    } else {
                        match adapter.get_page(&hit.slug, &cancel).await {
                            Ok(content) => content,
                            Err(_) => {
                                degraded = true;
                                continue;
                            }
                        }
                    };
                    let page = SupplementalDocument {
                        slug: hit.slug.clone(),
                        content,
                    };
                    let mut item = if read_only_destination {
                        // A legacy read-only source is always foreign authority,
                        // even when a page claims Aletheon's schema. Never let
                        // external frontmatter upgrade the item's trust level.
                        match read_only_external_item(&page.content, hit.score) {
                            Some(item) => item,
                            None => continue,
                        }
                    } else {
                        match is_supplemental_memory_document(&page.content) {
                            Ok(true) => match page.to_recall_item(request.current_at) {
                                Ok(item) => item,
                                Err(_) => {
                                    degraded = true;
                                    continue;
                                }
                            },
                            Ok(false) => continue,
                            Err(()) => {
                                degraded = true;
                                continue;
                            }
                        }
                    };
                    if matches!(
                        item.metadata.sensitivity,
                        MemorySensitivity::Confidential | MemorySensitivity::Restricted
                    ) || (!request.include_historical
                        && matches!(
                            item.temporal_state,
                            TemporalState::Superseded | TemporalState::Expired
                        ))
                    {
                        continue;
                    }
                    let mut hasher = Sha256::new();
                    for value in [
                        binding.backend_id.as_bytes(),
                        hit.source_id.as_bytes(),
                        hit.slug.as_bytes(),
                        item.content.as_bytes(),
                    ] {
                        hasher.update((value.len() as u64).to_le_bytes());
                        hasher.update(value);
                    }
                    let external_id = format!("supplemental:sha256:{:x}", hasher.finalize());
                    item.metadata.record_id = external_id;
                    item.metadata.provenance.source = "supplemental".into();
                    item.metadata.provenance.source_id = format!("{}:{}", hit.source_id, hit.slug);
                    item.metadata.provenance.principal = None;
                    item.scope = MemoryScope::Workspace(binding.workspace_key.as_str().to_owned());
                    item.authority = MemoryAuthority::ExternalReference;
                    item.score = hit.score.clamp(0.0, 1.0) as f32;
                    items.push(item);
                    if items.len() >= request.max_items {
                        break;
                    }
                }
                if items.len() >= request.max_items {
                    break;
                }
            }
            if items.len() >= request.max_items {
                break;
            }
        }
        Ok(RecallSet {
            items,
            degraded_sources: degraded
                .then(|| "supplemental".to_owned())
                .into_iter()
                .collect(),
        })
    }
}

impl SupplementalMcpAdapter {
    pub fn new(
        manager: Arc<McpManager>,
        server_name: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let server_name = server_name.into();
        let schema_valid = manager.server_tools(&server_name).is_some_and(|tools| {
            let tools = tools.into_iter().map(|tool| json!({
                "name": tool.name, "description": tool.description, "inputSchema": tool.input_schema,
            })).collect::<Vec<_>>();
            validate_tools_list(&json!({"result": {"tools": tools}})).is_ok()
        });
        Self {
            manager,
            server_name,
            timeout,
            health: Mutex::new(SupplementalHealth {
                state: if schema_valid {
                    SupplementalHealthState::Healthy
                } else {
                    SupplementalHealthState::Degraded
                },
                schema: if schema_valid {
                    SupplementalSchemaStatus::Valid
                } else {
                    SupplementalSchemaStatus::Invalid
                },
                last_error_category: (!schema_valid)
                    .then_some(SupplementalAdapterErrorCategory::Schema),
                consecutive_failures: u64::from(!schema_valid),
                last_success_unix_ms: None,
                queue_depth: 0,
            }),
            destination_attestations: Arc::new(BTreeMap::new()),
            destination_verified_at: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn with_destination_attestations(
        mut self,
        values: &[SupplementalDestinationAttestationConfig],
    ) -> anyhow::Result<Self> {
        self.destination_attestations = Arc::new(attestation_map(values)?);
        Ok(self)
    }

    pub fn health(&self) -> SupplementalHealth {
        self.health
            .lock()
            .expect("supplemental health mutex poisoned")
            .clone()
    }

    pub fn set_queue_depth(&self, queue_depth: usize) {
        self.health
            .lock()
            .expect("supplemental health mutex poisoned")
            .queue_depth = queue_depth;
    }

    /// Verify a standard OAuth identity and a marker that is readable only
    /// through the source-bound destination credential. A configured source
    /// name alone is never treated as evidence of remote authority.
    pub async fn negotiate_attested(
        &self,
        backend_id: &str,
        attestation: &SupplementalDestinationAttestationConfig,
        cancel: &CancellationToken,
    ) -> Result<SupplementalCapabilityGrant, SupplementalAdapterError> {
        if backend_id.trim().is_empty() || backend_id.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "backend identity is invalid",
            ));
        }
        let value = self.invoke("whoami", json!({}), cancel).await?;
        let text =
            extract_text(&value).map_err(|error| self.fail(error.category, error.message))?;
        let identity: WhoAmI = serde_json::from_str(&text).map_err(|error| {
            tracing::warn!(%error, whoami = %text, "whoami identity failed to parse");
            self.fail(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "capability response is malformed",
            )
        })?;
        if identity.transport != "oauth" {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Auth,
                "capability identity is not OAuth",
            ));
        }
        let scopes: std::collections::BTreeSet<_> =
            identity.scopes.iter().map(String::as_str).collect();
        if scopes != std::collections::BTreeSet::from(["read", "write"]) {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Auth,
                "capability identity does not have least-privilege memory scopes",
            ));
        }
        let marker = self
            .get_attestation_payload(&attestation.marker_slug, cancel)
            .await?;
        let marker_sha256 = format!("{:x}", Sha256::digest(marker.as_bytes()));
        if marker_sha256 != attestation.marker_sha256 {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Auth,
                "destination source attestation failed",
            ));
        }
        Ok(attested_grant(backend_id, &attestation.source_id))
    }

    /// Attest an explicitly configured read-only destination. Legacy bearer
    /// credentials are accepted only on this path; the returned grant cannot
    /// authorize projection writes.
    pub async fn negotiate_read_attested(
        &self,
        backend_id: &str,
        attestation: &SupplementalDestinationAttestationConfig,
        cancel: &CancellationToken,
    ) -> Result<SupplementalCapabilityGrant, SupplementalAdapterError> {
        if backend_id.trim().is_empty() || backend_id.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "backend identity is invalid",
            ));
        }
        let value = self.invoke("whoami", json!({}), cancel).await?;
        let text = extract_text(&value)?;
        let identity: WhoAmI = serde_json::from_str(&text).map_err(|_| {
            self.fail(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "capability response is malformed",
            )
        })?;
        if identity.transport != "legacy" || !identity.scopes.iter().any(|scope| scope == "read") {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Auth,
                "read-only capability identity is invalid",
            ));
        }
        let marker = self
            .get_attestation_payload(&attestation.marker_slug, cancel)
            .await?;
        if format!("{:x}", Sha256::digest(marker.as_bytes())) != attestation.marker_sha256 {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Auth,
                "destination source attestation failed",
            ));
        }
        Ok(read_only_attested_grant(backend_id, &attestation.source_id))
    }

    pub async fn put_page(
        &self,
        page: &SupplementalDocument,
        cancel: &CancellationToken,
    ) -> Result<(), SupplementalAdapterError> {
        if page.slug.len() > MAX_SLUG_BYTES || page.content.len() > MAX_PAGE_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::InvalidPage,
                "page rejected by local bounds",
            ));
        }
        self.invoke(
            "put_page",
            json!({"slug": page.slug, "content": page.content}),
            cancel,
        )
        .await?;
        Ok(())
    }

    pub async fn query(
        &self,
        query: &str,
        source_id: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalSearchHit>, SupplementalAdapterError> {
        self.validate_query(query, limit)?;
        if source_id.trim().is_empty() || source_id.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "source scope is invalid",
            ));
        }
        let value = self
            .invoke(
                "query",
                json!({"query": query, "source_id": source_id, "limit": limit}),
                cancel,
            )
            .await?;
        self.parse_hits(value, limit)
    }

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalSearchHit>, SupplementalAdapterError> {
        self.validate_query(query, limit)?;
        let value = self
            .invoke("search", json!({"query": query, "limit": limit}), cancel)
            .await?;
        self.parse_hits(value, limit)
    }

    pub async fn get_page(
        &self,
        slug: &str,
        cancel: &CancellationToken,
    ) -> Result<String, SupplementalAdapterError> {
        if slug.trim().is_empty() || slug.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "page slug is invalid",
            ));
        }
        let value = self
            .invoke("get_page", json!({"slug": slug}), cancel)
            .await?;
        let text = extract_text(&value)?;
        let content = parse_page_content(&text).map_err(|_| {
            self.fail(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "page response is malformed",
            )
        })?;
        if content.len() > MAX_TOOL_TEXT_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        Ok(content)
    }

    async fn get_attestation_payload(
        &self,
        slug: &str,
        cancel: &CancellationToken,
    ) -> Result<String, SupplementalAdapterError> {
        if slug.trim().is_empty() || slug.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "page slug is invalid",
            ));
        }
        let value = self
            .invoke("get_page", json!({"slug": slug}), cancel)
            .await?;
        let text = extract_text(&value)?;
        let payload = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("compiled_truth")
                    .or_else(|| value.get("content"))
                    .or_else(|| value.get("body"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or(text);
        if payload.len() > MAX_TOOL_TEXT_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        Ok(payload)
    }

    fn validate_query(&self, query: &str, limit: usize) -> Result<(), SupplementalAdapterError> {
        if query.trim().is_empty()
            || query.len() > MAX_QUERY_BYTES
            || !(1..=MAX_RESULTS).contains(&limit)
        {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::RejectedArguments,
                "query arguments are invalid",
            ));
        }
        Ok(())
    }

    async fn invoke(
        &self,
        tool: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, SupplementalAdapterError> {
        if self.health().schema != SupplementalSchemaStatus::Valid {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::Schema,
                "required MCP schema is unavailable",
            ));
        }
        let call = self.manager.call_tool(&self.server_name, tool, args);
        let result = tokio::select! {
            _ = cancel.cancelled() => return Err(self.fail(SupplementalAdapterErrorCategory::Cancelled, "request cancelled")),
            result = tokio::time::timeout(self.timeout, call) => match result {
                Err(_) => return Err(self.fail(SupplementalAdapterErrorCategory::Timeout, "request timed out")),
                Ok(result) => result,
            }
        };
        match result {
            Ok(value) => {
                self.succeed();
                Ok(value)
            }
            Err(error) => {
                let category = classify_error(&error);
                Err(self.fail(category, category_message(category)))
            }
        }
    }

    fn parse_hits(
        &self,
        value: Value,
        limit: usize,
    ) -> Result<Vec<SupplementalSearchHit>, SupplementalAdapterError> {
        let text =
            extract_text(&value).map_err(|error| self.fail(error.category, error.message))?;
        if text.len() > MAX_TOOL_TEXT_BYTES {
            return Err(self.fail(
                SupplementalAdapterErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        let values: Vec<Value> = serde_json::from_str(&text).map_err(|_| {
            self.fail(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "tool response is malformed",
            )
        })?;
        let mut hits = Vec::new();
        for value in values.into_iter().take(limit) {
            let slug = value
                .get("slug")
                .and_then(Value::as_str)
                .filter(|slug| !slug.is_empty() && slug.len() <= MAX_SLUG_BYTES)
                .ok_or_else(|| {
                    self.fail(
                        SupplementalAdapterErrorCategory::MalformedResponse,
                        "tool response is malformed",
                    )
                })?;
            let source_id = value
                .get("source_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = value
                .get("chunk_text")
                .or_else(|| value.get("content"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if source_id.len() > MAX_SLUG_BYTES || content.len() > MAX_TOOL_TEXT_BYTES {
                return Err(self.fail(
                    SupplementalAdapterErrorCategory::OversizedResponse,
                    "tool response exceeds byte limit",
                ));
            }
            hits.push(SupplementalSearchHit {
                source_id: source_id.to_owned(),
                slug: slug.to_owned(),
                content: content.to_owned(),
                score: value
                    .get("score")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0),
            });
        }
        Ok(hits)
    }

    fn succeed(&self) {
        let mut health = self
            .health
            .lock()
            .expect("supplemental health mutex poisoned");
        health.state = SupplementalHealthState::Healthy;
        health.last_error_category = None;
        health.consecutive_failures = 0;
        health.last_success_unix_ms = Some(chrono::Utc::now().timestamp_millis());
    }

    fn fail(
        &self,
        category: SupplementalAdapterErrorCategory,
        message: &'static str,
    ) -> SupplementalAdapterError {
        let mut health = self
            .health
            .lock()
            .expect("supplemental health mutex poisoned");
        health.state = SupplementalHealthState::Degraded;
        health.last_error_category = Some(category);
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        SupplementalAdapterError::new(category, message)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WhoAmI {
    transport: String,
    scopes: Vec<String>,
    #[serde(default, rename = "client_id")]
    _client_id: Option<String>,
    #[serde(default, rename = "client_name")]
    _client_name: Option<String>,
    #[serde(default, rename = "token_name")]
    _token_name: Option<String>,
    #[serde(default, rename = "expires_at")]
    _expires_at: Option<Value>,
    /// Newer gbrain identities carry federation metadata; tolerate them so an
    /// otherwise valid OAuth identity is not rejected as malformed.
    #[serde(default, rename = "source_id")]
    _source_id: Option<String>,
    #[serde(default, rename = "federated_read")]
    _federated_read: Option<Vec<String>>,
}

fn attestation_map(
    values: &[SupplementalDestinationAttestationConfig],
) -> anyhow::Result<BTreeMap<String, SupplementalDestinationAttestationConfig>> {
    let mut result = BTreeMap::new();
    for value in values {
        anyhow::ensure!(
            !value.destination_handle.trim().is_empty()
                && value.destination_handle.len() <= MAX_SLUG_BYTES
                && !value.destination_handle.chars().any(char::is_whitespace),
            "supplemental attestation destination handle is invalid"
        );
        anyhow::ensure!(
            !value.source_id.trim().is_empty()
                && value.source_id.len() <= MAX_SLUG_BYTES
                && !value.source_id.chars().any(char::is_whitespace),
            "supplemental attestation source is invalid"
        );
        anyhow::ensure!(
            !value.marker_slug.trim().is_empty() && value.marker_slug.len() <= MAX_SLUG_BYTES,
            "supplemental attestation marker slug is invalid"
        );
        anyhow::ensure!(
            value.marker_sha256.len() == 64
                && value
                    .marker_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "supplemental attestation marker digest is invalid"
        );
        anyhow::ensure!(
            (1..=86_400).contains(&value.revalidate_after_secs),
            "supplemental attestation revalidation interval is invalid"
        );
        anyhow::ensure!(
            result
                .insert(value.destination_handle.clone(), value.clone())
                .is_none(),
            "supplemental attestation destination handle is duplicated"
        );
    }
    Ok(result)
}

fn attested_grant(backend_id: &str, source_id: &str) -> SupplementalCapabilityGrant {
    SupplementalCapabilityGrant {
        backend_id: backend_id.to_owned(),
        write_source: Some(source_id.to_owned()),
        read_sources: vec![source_id.to_owned()],
        can_read: true,
        can_write: true,
    }
}

fn read_only_attested_grant(backend_id: &str, source_id: &str) -> SupplementalCapabilityGrant {
    SupplementalCapabilityGrant {
        backend_id: backend_id.to_owned(),
        write_source: None,
        read_sources: vec![source_id.to_owned()],
        can_read: true,
        can_write: false,
    }
}

fn recently_verified(
    cache: &Mutex<BTreeMap<String, Instant>>,
    destination_handle: &str,
    revalidate_after_secs: u64,
) -> bool {
    cache
        .lock()
        .expect("supplemental attestation cache mutex poisoned")
        .get(destination_handle)
        .is_some_and(|verified| verified.elapsed() < Duration::from_secs(revalidate_after_secs))
}

fn mark_verified(cache: &Mutex<BTreeMap<String, Instant>>, destination_handle: &str) {
    cache
        .lock()
        .expect("supplemental attestation cache mutex poisoned")
        .insert(destination_handle.to_owned(), Instant::now());
}

fn extract_text(value: &Value) -> Result<String, SupplementalAdapterError> {
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SupplementalAdapterError::new(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "tool response is malformed",
            )
        })?;
    let mut text = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let value = block.get("text").and_then(Value::as_str).ok_or_else(|| {
            SupplementalAdapterError::new(
                SupplementalAdapterErrorCategory::MalformedResponse,
                "tool response is malformed",
            )
        })?;
        if text.len().saturating_add(value.len()) > MAX_TOOL_TEXT_BYTES {
            return Err(SupplementalAdapterError::new(
                SupplementalAdapterErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        text.push_str(value);
    }
    if text.is_empty() {
        return Err(SupplementalAdapterError::new(
            SupplementalAdapterErrorCategory::MalformedResponse,
            "tool response is malformed",
        ));
    }
    Ok(text)
}

fn parse_page_content(text: &str) -> anyhow::Result<String> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(text.to_owned());
    };
    if let Some(content) = value
        .get("content")
        .or_else(|| value.get("body"))
        .and_then(Value::as_str)
    {
        return Ok(content.to_owned());
    }
    let compiled_truth = value
        .get("compiled_truth")
        .and_then(Value::as_str)
        .context("page response has no content")?;
    let frontmatter = value
        .get("frontmatter")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let yaml = serde_yaml::to_string(&frontmatter).context("serializing page frontmatter")?;
    let mut content = format!(
        "---\n{}---\n\n{}",
        yaml.trim_start_matches("---\n"),
        compiled_truth
    );
    if let Some(timeline) = value.get("timeline").and_then(Value::as_str) {
        if !timeline.is_empty() {
            content.push_str("\n\n<!-- timeline -->\n\n");
            content.push_str(timeline);
        }
    }
    content.push('\n');
    Ok(content)
}

/// Distinguish Aletheon-owned memory projections from ordinary pages sharing
/// the same source. A foreign schema is expected in a general-purpose GBrain
/// source (for example the source-attestation marker) and is therefore skipped
/// rather than reported as a backend outage. A page that claims the Aletheon
/// memory schema but has malformed frontmatter remains degraded/fail-closed.
fn is_supplemental_memory_document(content: &str) -> Result<bool, ()> {
    let Some(remainder) = content.strip_prefix("---\n") else {
        return Ok(false);
    };
    let Some((yaml, _)) = remainder.split_once("\n---\n") else {
        return Err(());
    };
    let frontmatter = serde_yaml::from_str::<serde_yaml::Value>(yaml).map_err(|_| ())?;
    let schema = frontmatter
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String("schema".into())))
        .and_then(serde_yaml::Value::as_str);
    Ok(schema == Some(PAGE_SCHEMA_VERSION))
}

fn read_only_external_item(content: &str, score: f64) -> Option<RecallItem> {
    let content = content.trim();
    if content.is_empty() || content.len() > MAX_PAGE_BYTES {
        return None;
    }
    let normalized = content.to_ascii_lowercase();
    if [
        "<dasein_mutation",
        "<identity_instruction",
        "<tool_execution",
        "<policy_change",
        "\"dasein_mutation\":",
        "\"identity_instruction\":",
        "\"tool_execution\":",
        "\"tool_call\":",
        "\"policy_change\":",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return None;
    }
    let observed_time = chrono::Utc::now();
    Some(RecallItem {
        content: content.to_owned(),
        kind: MemoryKind::ExternalReference,
        metadata: MemoryMetadata {
            record_id: "pending-external-id".into(),
            provenance: MemoryProvenance {
                source: "supplemental".into(),
                source_id: "pending-external-source".into(),
                principal: None,
                source_commit: None,
            },
            source_time: None,
            observed_time,
            valid_from: None,
            valid_until: None,
            supersedes: None,
            superseded_by: None,
            confidence: score.clamp(0.0, 1.0),
            sensitivity: MemorySensitivity::Internal,
        },
        temporal_state: TemporalState::Unknown,
        authority: MemoryAuthority::ExternalReference,
        scope: MemoryScope::Global,
        score: score.clamp(0.0, 1.0) as f32,
        evidence: None,
    })
}

fn classify_error(error: &anyhow::Error) -> SupplementalAdapterErrorCategory {
    let message = format!("{error:#}").to_ascii_lowercase();
    let has_status = |status: &str| {
        message
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| token == status)
    };
    if has_status("401") || has_status("403") || message.contains("authentication") {
        SupplementalAdapterErrorCategory::Auth
    } else if message.contains("exceeds byte limit") {
        SupplementalAdapterErrorCategory::OversizedResponse
    } else if has_status("429") {
        SupplementalAdapterErrorCategory::RateLimited
    } else if has_status("500") || has_status("502") || has_status("503") || has_status("504") {
        SupplementalAdapterErrorCategory::Provider
    } else if message.contains("application error") {
        SupplementalAdapterErrorCategory::RejectedArguments
    } else {
        SupplementalAdapterErrorCategory::Transport
    }
}

fn category_message(category: SupplementalAdapterErrorCategory) -> &'static str {
    match category {
        SupplementalAdapterErrorCategory::Auth => "authentication failed",
        SupplementalAdapterErrorCategory::RateLimited => "provider rate limited request",
        SupplementalAdapterErrorCategory::Provider => "provider request failed",
        SupplementalAdapterErrorCategory::RejectedArguments => "tool rejected arguments",
        SupplementalAdapterErrorCategory::OversizedResponse => "tool response exceeds byte limit",
        _ => "transport request failed",
    }
}

fn supplemental_category(
    category: SupplementalAdapterErrorCategory,
) -> mnemosyne::supplemental::SupplementalErrorCategory {
    use mnemosyne::supplemental::SupplementalErrorCategory as Target;
    match category {
        SupplementalAdapterErrorCategory::Auth => Target::Auth,
        SupplementalAdapterErrorCategory::Schema => Target::Schema,
        SupplementalAdapterErrorCategory::InvalidPage => Target::InvalidPage,
        SupplementalAdapterErrorCategory::RejectedArguments => Target::RejectedArguments,
        SupplementalAdapterErrorCategory::Timeout => Target::Timeout,
        SupplementalAdapterErrorCategory::Cancelled => Target::Cancelled,
        SupplementalAdapterErrorCategory::RateLimited => Target::RateLimited,
        SupplementalAdapterErrorCategory::Provider => Target::Provider,
        SupplementalAdapterErrorCategory::Transport => Target::Transport,
        SupplementalAdapterErrorCategory::MalformedResponse => Target::MalformedResponse,
        SupplementalAdapterErrorCategory::OversizedResponse => Target::OversizedResponse,
    }
}

fn supplemental_error(
    error: SupplementalAdapterError,
) -> mnemosyne::supplemental::SupplementalTransportError {
    mnemosyne::supplemental::SupplementalTransportError::new(
        supplemental_category(error.category),
        error.sanitized_message(),
    )
}

#[async_trait::async_trait]
impl mnemosyne::supplemental::SupplementalMemoryTransport for SupplementalMcpAdapter {
    fn set_queue_depth(&self, queue_depth: usize) {
        SupplementalMcpAdapter::set_queue_depth(self, queue_depth);
    }

    async fn put_page(
        &self,
        page: &SupplementalDocument,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, mnemosyne::supplemental::SupplementalTransportError> {
        SupplementalMcpAdapter::put_page(self, page, cancel)
            .await
            .map(|()| None)
            .map_err(supplemental_error)
    }

    async fn put_page_to(
        &self,
        destination_handle: &str,
        page: &SupplementalDocument,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, mnemosyne::supplemental::SupplementalTransportError> {
        let policy = self
            .destination_attestations
            .get(destination_handle)
            .ok_or_else(|| {
                mnemosyne::supplemental::SupplementalTransportError::new(
                    mnemosyne::supplemental::SupplementalErrorCategory::Auth,
                    "destination write lacks a source attestation policy",
                )
            })?;
        let destination = SupplementalMcpAdapter::new(
            self.manager.clone(),
            destination_handle.to_owned(),
            self.timeout,
        );
        if !recently_verified(
            &self.destination_verified_at,
            destination_handle,
            policy.revalidate_after_secs,
        ) {
            destination
                .negotiate_attested("supplemental", policy, cancel)
                .await
                .map_err(supplemental_error)?;
            mark_verified(&self.destination_verified_at, destination_handle);
        }
        destination
            .put_page(page, cancel)
            .await
            .map(|()| None)
            .map_err(supplemental_error)
    }

    async fn query(
        &self,
        query: &str,
        source_id: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<
        Vec<mnemosyne::supplemental::SupplementalHit>,
        mnemosyne::supplemental::SupplementalTransportError,
    > {
        SupplementalMcpAdapter::query(self, query, source_id, limit, cancel)
            .await
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| mnemosyne::supplemental::SupplementalHit {
                        source_id: hit.source_id,
                        slug: hit.slug,
                        content: hit.content,
                        score: hit.score,
                    })
                    .collect()
            })
            .map_err(supplemental_error)
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<
        Vec<mnemosyne::supplemental::SupplementalHit>,
        mnemosyne::supplemental::SupplementalTransportError,
    > {
        SupplementalMcpAdapter::search(self, query, limit, cancel)
            .await
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| mnemosyne::supplemental::SupplementalHit {
                        source_id: hit.source_id,
                        slug: hit.slug,
                        content: hit.content,
                        score: hit.score,
                    })
                    .collect()
            })
            .map_err(supplemental_error)
    }

    async fn get_page(
        &self,
        slug: &str,
        cancel: &CancellationToken,
    ) -> Result<String, mnemosyne::supplemental::SupplementalTransportError> {
        SupplementalMcpAdapter::get_page(self, slug, cancel)
            .await
            .map_err(supplemental_error)
    }
}
