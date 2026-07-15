//! Bounded adapter over the retained Corpus MCP manager.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use corpus::tools::mcp::manager::McpManager;
use mnemosyne::backends::gbrain::page::MAX_PAGE_BYTES;
use mnemosyne::backends::gbrain::{validate_tools_list, GbrainPage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const MAX_TOOL_TEXT_BYTES: usize = 256 * 1024;
const MAX_SLUG_BYTES: usize = 512;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const MAX_RESULTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GbrainErrorCategory {
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

impl GbrainErrorCategory {
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Cancelled | Self::RateLimited | Self::Provider | Self::Transport
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("GBrain MCP {category:?}: {message}")]
pub struct GbrainAdapterError {
    pub category: GbrainErrorCategory,
    message: &'static str,
}

impl GbrainAdapterError {
    fn new(category: GbrainErrorCategory, message: &'static str) -> Self {
        Self { category, message }
    }
    pub fn sanitized_message(&self) -> &'static str {
        self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GbrainHealthState {
    Healthy,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GbrainSchemaStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GbrainHealth {
    pub state: GbrainHealthState,
    pub schema: GbrainSchemaStatus,
    pub last_error_category: Option<GbrainErrorCategory>,
    pub consecutive_failures: u64,
    pub last_success_unix_ms: Option<i64>,
    pub queue_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GbrainSearchHit {
    pub source_id: String,
    pub slug: String,
    pub content: String,
    pub score: f64,
}

pub struct GbrainMcpAdapter {
    manager: Arc<McpManager>,
    server_name: String,
    timeout: Duration,
    health: Mutex<GbrainHealth>,
}

impl GbrainMcpAdapter {
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
            health: Mutex::new(GbrainHealth {
                state: if schema_valid {
                    GbrainHealthState::Healthy
                } else {
                    GbrainHealthState::Degraded
                },
                schema: if schema_valid {
                    GbrainSchemaStatus::Valid
                } else {
                    GbrainSchemaStatus::Invalid
                },
                last_error_category: (!schema_valid).then_some(GbrainErrorCategory::Schema),
                consecutive_failures: u64::from(!schema_valid),
                last_success_unix_ms: None,
                queue_depth: 0,
            }),
        }
    }

    pub fn health(&self) -> GbrainHealth {
        self.health
            .lock()
            .expect("gbrain health mutex poisoned")
            .clone()
    }

    pub fn set_queue_depth(&self, queue_depth: usize) {
        self.health
            .lock()
            .expect("gbrain health mutex poisoned")
            .queue_depth = queue_depth;
    }

    pub async fn put_page(
        &self,
        page: &GbrainPage,
        cancel: &CancellationToken,
    ) -> Result<(), GbrainAdapterError> {
        if page.slug.len() > MAX_SLUG_BYTES || page.content.len() > MAX_PAGE_BYTES {
            return Err(self.fail(
                GbrainErrorCategory::InvalidPage,
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
    ) -> Result<Vec<GbrainSearchHit>, GbrainAdapterError> {
        self.validate_query(query, limit)?;
        if source_id.trim().is_empty() || source_id.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                GbrainErrorCategory::RejectedArguments,
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
    ) -> Result<Vec<GbrainSearchHit>, GbrainAdapterError> {
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
    ) -> Result<String, GbrainAdapterError> {
        if slug.trim().is_empty() || slug.len() > MAX_SLUG_BYTES {
            return Err(self.fail(
                GbrainErrorCategory::RejectedArguments,
                "page slug is invalid",
            ));
        }
        let value = self
            .invoke("get_page", json!({"slug": slug}), cancel)
            .await?;
        let text = extract_text(&value)?;
        let content = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("content")
                    .or_else(|| value.get("body"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or(text);
        if content.len() > MAX_TOOL_TEXT_BYTES {
            return Err(self.fail(
                GbrainErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        Ok(content)
    }

    fn validate_query(&self, query: &str, limit: usize) -> Result<(), GbrainAdapterError> {
        if query.trim().is_empty()
            || query.len() > MAX_QUERY_BYTES
            || !(1..=MAX_RESULTS).contains(&limit)
        {
            return Err(self.fail(
                GbrainErrorCategory::RejectedArguments,
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
    ) -> Result<Value, GbrainAdapterError> {
        if self.health().schema != GbrainSchemaStatus::Valid {
            return Err(self.fail(
                GbrainErrorCategory::Schema,
                "required MCP schema is unavailable",
            ));
        }
        let call = self.manager.call_tool(&self.server_name, tool, args);
        let result = tokio::select! {
            _ = cancel.cancelled() => return Err(self.fail(GbrainErrorCategory::Cancelled, "request cancelled")),
            result = tokio::time::timeout(self.timeout, call) => match result {
                Err(_) => return Err(self.fail(GbrainErrorCategory::Timeout, "request timed out")),
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
    ) -> Result<Vec<GbrainSearchHit>, GbrainAdapterError> {
        let text =
            extract_text(&value).map_err(|error| self.fail(error.category, error.message))?;
        if text.len() > MAX_TOOL_TEXT_BYTES {
            return Err(self.fail(
                GbrainErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        let values: Vec<Value> = serde_json::from_str(&text).map_err(|_| {
            self.fail(
                GbrainErrorCategory::MalformedResponse,
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
                        GbrainErrorCategory::MalformedResponse,
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
                    GbrainErrorCategory::OversizedResponse,
                    "tool response exceeds byte limit",
                ));
            }
            hits.push(GbrainSearchHit {
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
        let mut health = self.health.lock().expect("gbrain health mutex poisoned");
        health.state = GbrainHealthState::Healthy;
        health.last_error_category = None;
        health.consecutive_failures = 0;
        health.last_success_unix_ms = Some(chrono::Utc::now().timestamp_millis());
    }

    fn fail(&self, category: GbrainErrorCategory, message: &'static str) -> GbrainAdapterError {
        let mut health = self.health.lock().expect("gbrain health mutex poisoned");
        health.state = GbrainHealthState::Degraded;
        health.last_error_category = Some(category);
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        GbrainAdapterError::new(category, message)
    }
}

fn extract_text(value: &Value) -> Result<String, GbrainAdapterError> {
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            GbrainAdapterError::new(
                GbrainErrorCategory::MalformedResponse,
                "tool response is malformed",
            )
        })?;
    let mut text = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let value = block.get("text").and_then(Value::as_str).ok_or_else(|| {
            GbrainAdapterError::new(
                GbrainErrorCategory::MalformedResponse,
                "tool response is malformed",
            )
        })?;
        if text.len().saturating_add(value.len()) > MAX_TOOL_TEXT_BYTES {
            return Err(GbrainAdapterError::new(
                GbrainErrorCategory::OversizedResponse,
                "tool response exceeds byte limit",
            ));
        }
        text.push_str(value);
    }
    if text.is_empty() {
        return Err(GbrainAdapterError::new(
            GbrainErrorCategory::MalformedResponse,
            "tool response is malformed",
        ));
    }
    Ok(text)
}

fn classify_error(error: &anyhow::Error) -> GbrainErrorCategory {
    let message = format!("{error:#}").to_ascii_lowercase();
    if message.contains("401") || message.contains("403") || message.contains("authentication") {
        GbrainErrorCategory::Auth
    } else if message.contains("exceeds byte limit") {
        GbrainErrorCategory::OversizedResponse
    } else if message.contains("429") {
        GbrainErrorCategory::RateLimited
    } else if message.contains("500")
        || message.contains("502")
        || message.contains("503")
        || message.contains("504")
    {
        GbrainErrorCategory::Provider
    } else if message.contains("application error") {
        GbrainErrorCategory::RejectedArguments
    } else {
        GbrainErrorCategory::Transport
    }
}

fn category_message(category: GbrainErrorCategory) -> &'static str {
    match category {
        GbrainErrorCategory::Auth => "authentication failed",
        GbrainErrorCategory::RateLimited => "provider rate limited request",
        GbrainErrorCategory::Provider => "provider request failed",
        GbrainErrorCategory::RejectedArguments => "tool rejected arguments",
        GbrainErrorCategory::OversizedResponse => "tool response exceeds byte limit",
        _ => "transport request failed",
    }
}
