use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::*;
use crate::config::{CacheReportingMode, ProviderTimeoutConfig};
use fabric::llm_types::{InferenceUsageError, ModelRuntimeFacts};
use fabric::message::{ContentBlock, ImageSource, Message, Role};

/// OpenAI-compatible provider (chat/completions).
/// Works with OpenAI, DeepSeek, Ollama, LM Studio, vLLM, Xiaomi MiMo, etc.
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_context: usize,
    max_tokens: u32,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    /// How usage telemetry cache figures are interpreted (from provider config).
    cache_reporting: CacheReportingMode,
}

impl OpenAiProvider {
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let timeouts = ProviderTimeoutConfig::default();
        Self {
            client: Self::client(&timeouts),
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_context: 128_000,
            max_tokens: 4096,
            request_timeout: Duration::from_millis(timeouts.request_timeout_ms),
            stream_idle_timeout: Duration::from_millis(timeouts.stream_idle_timeout_ms),
            cache_reporting: CacheReportingMode::Auto,
        }
    }

    fn client(timeouts: &ProviderTimeoutConfig) -> Client {
        Client::builder()
            .connect_timeout(Duration::from_millis(timeouts.connect_timeout_ms))
            .build()
            .expect("reqwest client configuration is valid")
    }

    pub fn with_timeouts(mut self, timeouts: ProviderTimeoutConfig) -> Self {
        timeouts
            .validate()
            .expect("provider timeout configuration must be validated");
        self.client = Self::client(&timeouts);
        self.request_timeout = Duration::from_millis(timeouts.request_timeout_ms);
        self.stream_idle_timeout = Duration::from_millis(timeouts.stream_idle_timeout_ms);
        self
    }

    pub fn with_max_context(mut self, max_context: usize) -> Self {
        self.max_context = max_context;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Declare how this provider reports cache usage in its usage telemetry.
    /// Defaults to `Auto` (inspect the wire fields); only config-derived hints
    /// set this, never model-name guesses.
    pub fn with_cache_reporting(mut self, mode: CacheReportingMode) -> Self {
        self.cache_reporting = mode;
        self
    }
}

fn provider_timeout() -> anyhow::Error {
    InferenceFailure::transient("provider_timeout")
}

fn provider_request_error(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        provider_timeout()
    } else {
        InferenceFailure::transient("provider_request_failed")
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: FunctionCall,
}

#[derive(Serialize, Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: FunctionDef,
}

#[derive(Serialize)]
struct FunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
    /// Some reasoning models (GLM-5.2, DeepSeek Reasoning) put their output here
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    /// DeepSeek-style prompt cache accounting. Hit + miss should equal
    /// `prompt_tokens`; either may be absent on proxies that strip them.
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u64>,
}

/// Resolve OpenAI/DeepSeek cache-reporting wire fields into a single
/// `InferenceUsage`. Pure: never keys off the model name (proxies may rewrite
/// it), only off the configured `reporting` mode and which fields the provider
/// actually returned.
///
/// Priority (mirrors docs/plans/deepseek-cache-and-message-optimization-plan.md §3.2):
/// 0. `reporting == Unsupported` -> `CacheTelemetry::Unsupported`, no cache figures;
/// 1. DeepSeek hit + miss both present -> use both, validate conservation;
/// 2. only DeepSeek hit + total known -> miss = total - hit;
/// 3. OpenAI `cached_tokens` present -> read = cached, uncached = total - cached;
/// 4. both formats present and agree -> unified result;
/// 5. both formats present and conflict -> typed protocol error, never silently pick;
/// 7. no cache fields and capability unknown -> `CacheTelemetry::Unknown`.
fn openai_usage(usage: &ApiUsage, reporting: CacheReportingMode) -> anyhow::Result<InferenceUsage> {
    let total = usage.prompt_tokens;
    // A provider declared without cache reporting never carries cache figures,
    // even if it sends them: telemetry must stay Unsupported, not a fabricated
    // hit or miss.
    if reporting == CacheReportingMode::Unsupported {
        return Ok(InferenceUsage::unsupported(
            Some(total),
            Some(usage.completion_tokens),
        ));
    }
    let openai_cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    let deepseek_hit = usage.prompt_cache_hit_tokens;
    let deepseek_miss = usage.prompt_cache_miss_tokens;

    // Both formats present: they must agree, otherwise this is a protocol
    // error (never silently prefer one wire format over the other).
    if let (Some(hit), Some(openai)) = (deepseek_hit, openai_cached) {
        if hit != openai {
            return Err(InferenceUsageError::FormatConflict {
                deepseek_hit: hit,
                deepseek_miss: deepseek_miss.unwrap_or_else(|| total.saturating_sub(hit)),
                openai_cached: openai,
            }
            .into());
        }
    }

    let (read, uncached, telemetry) = match (deepseek_hit, deepseek_miss) {
        (Some(hit), Some(miss)) => (Some(hit), Some(miss), CacheTelemetry::Reported),
        (Some(hit), None) => (
            Some(hit),
            Some(total.saturating_sub(hit)),
            CacheTelemetry::Reported,
        ),
        (None, Some(_)) => {
            return Err(InferenceUsageError::Invalid(
                "prompt_cache_miss_tokens reported without prompt_cache_hit_tokens".into(),
            )
            .into());
        }
        (None, None) => match openai_cached {
            Some(cached) => (
                Some(cached),
                Some(total.saturating_sub(cached)),
                CacheTelemetry::Reported,
            ),
            None => (None, None, CacheTelemetry::Unknown),
        },
    };

    let resolved = InferenceUsage {
        total_input_tokens: Some(total),
        output_tokens: Some(usage.completion_tokens),
        uncached_input_tokens: uncached,
        cache_read_tokens: read,
        cache_write_tokens: None,
        cache_telemetry: telemetry,
    };
    resolved.validate()?;
    Ok(resolved)
}

/// SSE streaming response structures
#[derive(Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Deserialize)]
struct StreamToolCall {
    index: usize,
    id: Option<String>,
    function: StreamFunctionCall,
}

#[derive(Deserialize)]
struct StreamFunctionCall {
    name: Option<String>,
    arguments: Option<String>,
}

/// Convert content blocks to OpenAI vision-format content parts.
/// Skips `ToolResult`, `ToolUse`, and `System` blocks (not valid in user content arrays).
fn content_blocks_to_openai_parts(blocks: &[ContentBlock]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(serde_json::json!({
                "type": "text",
                "text": text,
            })),
            ContentBlock::Image { source } => {
                let url = match source {
                    ImageSource::Base64 { media_type, data } => {
                        format!("data:{media_type};base64,{data}")
                    }
                    ImageSource::Url { url } => url.clone(),
                };
                Some(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url },
                }))
            }
            _ => None,
        })
        .collect()
}

fn messages_to_chat(messages: &[Message]) -> Vec<ChatMessage> {
    let mut result = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                // System message
                let text = msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                result.push(ChatMessage {
                    role: "system".to_string(),
                    content: serde_json::json!(text),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            Role::User => {
                // Check if ALL blocks are tool results — the ReAct loop batches
                // multiple ContentBlock::ToolResult entries into ONE user message
                // (required by Anthropic API). OpenAI requires each tool result
                // to be its own role="tool" message, so we emit one per block.
                let all_tool_results = msg
                    .content
                    .iter()
                    .all(|c| matches!(c, ContentBlock::ToolResult { .. }));
                if all_tool_results && !msg.content.is_empty() {
                    for block in &msg.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } = block
                        {
                            let text = if *is_error {
                                format!("[ERROR] {content}")
                            } else {
                                content.clone()
                            };
                            result.push(ChatMessage {
                                role: "tool".to_string(),
                                content: serde_json::json!(text),
                                tool_calls: None,
                                tool_call_id: Some(tool_use_id.clone()),
                            });
                        }
                    }
                } else {
                    // Check if we need multimodal content (images present or multiple blocks)
                    let has_images = msg
                        .content
                        .iter()
                        .any(|c| matches!(c, ContentBlock::Image { .. }));
                    let block_count = msg.content.len();

                    if !has_images && block_count == 1 {
                        // Single text block — use string content (backward compatible)
                        if let Some(ContentBlock::Text { text }) = msg.content.first() {
                            result.push(ChatMessage {
                                role: "user".to_string(),
                                content: serde_json::json!(text),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        } else {
                            // Single non-text block (e.g. a lone image) — still build array
                            let parts = content_blocks_to_openai_parts(&msg.content);
                            result.push(ChatMessage {
                                role: "user".to_string(),
                                content: serde_json::Value::Array(parts),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                    } else {
                        // Multiple blocks or images present — use array content
                        let parts = content_blocks_to_openai_parts(&msg.content);
                        result.push(ChatMessage {
                            role: "user".to_string(),
                            content: serde_json::Value::Array(parts),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
            }
            Role::Assistant => {
                // Check for tool_use blocks
                let tool_calls: Vec<ToolCall> = msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::ToolUse { id, name, input } => Some(ToolCall {
                            id: id.clone(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(input).unwrap_or_default(),
                            },
                        }),
                        _ => None,
                    })
                    .collect();

                let text = msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                result.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: if text.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!(text)
                    },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                });
            }
        }
    }

    result
}

fn tools_to_chat(tools: &[ToolDefinition]) -> Vec<ChatTool> {
    tools
        .iter()
        .map(|t| ChatTool {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect()
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn runtime_facts(&self) -> ModelRuntimeFacts {
        // Mirrors the trait default plus the config-declared cache reporting
        // mode. Built directly to keep the async_trait block free of
        // self-recursive fully-qualified calls.
        ModelRuntimeFacts {
            effective_model_id: self.name().to_string(),
            display_name: self.name().to_string(),
            max_context_tokens: self.max_context_length(),
            cache_reporting: Some(self.cache_reporting.as_str().to_string()),
        }
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let tools = canonicalize_tool_definitions(tools)?;
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_chat(messages),
            tools: tools_to_chat(&tools),
            max_tokens: Some(self.max_tokens),
            stream: None,
        };

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let api_resp: ChatResponse = tokio::time::timeout(self.request_timeout, async {
            let response = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(provider_request_error)?;

            if !response.status().is_success() {
                return Err(InferenceFailure::from_http_status(&response));
            }
            response.json().await.map_err(provider_request_error)
        })
        .await
        .map_err(|_| provider_timeout())??;

        let choice = api_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

        let mut content = Vec::new();

        // Reasoning content — preserve as Thinking block
        if let Some(thinking) = choice.message.reasoning_content.filter(|s| !s.is_empty()) {
            content.push(ContentBlock::Thinking {
                text: thinking,
                signature: None,
            });
        }

        // Text content
        if let Some(text) = choice.message.content.filter(|s| !s.is_empty()) {
            content.push(ContentBlock::Text { text });
        }

        // Tool calls
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                // Skip tool calls with empty names — some models emit malformed
                // tool-use blocks that would poison the conversation and trip
                // the circuit breaker.
                if tc.function.name.is_empty() {
                    tracing::warn!(
                        tool_id = %tc.id,
                        "Skipping tool call with empty name from model response"
                    );
                    continue;
                }
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
                content.push(ContentBlock::ToolUse {
                    id: tc.id,
                    name: tc.function.name,
                    input,
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = if let Some(u) = api_resp.usage {
            openai_usage(&u, self.cache_reporting)?
        } else {
            InferenceUsage::default()
        };

        Ok(LlmResponse {
            content,
            stop_reason,
            usage,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let tools = canonicalize_tool_definitions(tools)?;
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_chat(messages),
            tools: tools_to_chat(&tools),
            max_tokens: Some(self.max_tokens),
            stream: Some(true),
        };
        let request_body = serde_json::to_vec(&request).map_err(anyhow::Error::from)?;
        let request_bytes = request_body.len();

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let response = tokio::time::timeout(
            self.request_timeout,
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send(),
        )
        .await
        .map_err(|_| provider_timeout())?
        .map_err(provider_request_error)?;

        if !response.status().is_success() {
            let message_bytes = messages
                .iter()
                .map(|message| serde_json::to_vec(message).map_or(0, |value| value.len()))
                .collect::<Vec<_>>();
            let tool_bytes = tools
                .iter()
                .map(|tool| serde_json::to_vec(tool).map_or(0, |value| value.len()))
                .collect::<Vec<_>>();
            tracing::warn!(
                status = %response.status(),
                request_bytes,
                messages = messages.len(),
                tools = tools.len(),
                message_bytes = ?message_bytes,
                tool_bytes = ?tool_bytes,
                request_id = response
                    .headers()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or(""),
                retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or(""),
                "Streaming provider request rejected"
            );
            return Err(InferenceFailure::from_http_status(&response));
        }

        let byte_stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));
        let stream_idle_timeout = self.stream_idle_timeout;
        // Copied before the `move` closure so the returned `'static` stream does
        // not borrow `self` (CacheReportingMode is Copy).
        let cache_reporting = self.cache_reporting;

        let stream = futures::stream::unfold(
            (
                Box::pin(byte_stream),
                super::utf8_stream::Utf8StreamBuffer::default(),
                ToolCallState::default(),
            ),
            move |(mut byte_stream, mut buffer, mut tool_state)| async move {
                use futures::StreamExt;

                loop {
                    // Try to extract a complete SSE line from the buffer
                    let line = match buffer.take_line() {
                        Ok(line) => line,
                        Err(error) => {
                            return Some((
                                Err(anyhow::anyhow!(
                                    "provider stream contained invalid UTF-8: {error}"
                                )),
                                (byte_stream, buffer, tool_state),
                            ));
                        }
                    };
                    if let Some(line) = line {
                        let line = line.trim();

                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if data == "[DONE]" {
                                // Emit any completed tool calls before signaling done.
                                if let Some(completed) = tool_state.take_completed() {
                                    return Some((
                                        Ok(completed),
                                        (byte_stream, buffer, tool_state),
                                    ));
                                }
                                let stop_reason = tool_state.final_stop_reason();
                                return Some((
                                    Ok(StreamChunk::Done { stop_reason }),
                                    (byte_stream, buffer, tool_state),
                                ));
                            }

                            match serde_json::from_str::<StreamResponse>(data) {
                                Ok(resp) => {
                                    if let Some(usage) = &resp.usage {
                                        return Some((
                                            openai_usage(usage, cache_reporting)
                                                .map(|usage| StreamChunk::Usage { usage }),
                                            (byte_stream, buffer, tool_state),
                                        ));
                                    }

                                    if let Some(choice) = resp.choices.first() {
                                        if let Some(finish) = &choice.finish_reason {
                                            tool_state.finish_reason = Some(finish.clone());
                                        }

                                        // Handle text content
                                        if let Some(text) = &choice.delta.content {
                                            if !text.is_empty() {
                                                return Some((
                                                    Ok(StreamChunk::TextDelta {
                                                        text: text.clone(),
                                                    }),
                                                    (byte_stream, buffer, tool_state),
                                                ));
                                            }
                                        }

                                        // Handle reasoning content (DeepSeek reasoning models)
                                        if let Some(thinking) = &choice.delta.reasoning_content {
                                            if !thinking.is_empty() {
                                                return Some((
                                                    Ok(StreamChunk::ThinkingDelta {
                                                        text: thinking.clone(),
                                                    }),
                                                    (byte_stream, buffer, tool_state),
                                                ));
                                            }
                                        }

                                        // Handle tool calls
                                        if let Some(tool_calls) = &choice.delta.tool_calls {
                                            for tc in tool_calls {
                                                let idx = tc.index;

                                                // New tool call started — skip if name is empty (malformed)
                                                if let Some(id) = &tc.id {
                                                    if let Some(name) = &tc.function.name {
                                                        if name.is_empty() {
                                                            tracing::warn!(
                                                                tool_id = %id,
                                                                "Skipping streaming tool call with empty name"
                                                            );
                                                        } else {
                                                            tool_state.start_call(
                                                                idx,
                                                                id.clone(),
                                                                name.clone(),
                                                            );
                                                            return Some((
                                                                Ok(StreamChunk::ToolUseStart {
                                                                    id: id.clone(),
                                                                    name: name.clone(),
                                                                }),
                                                                (byte_stream, buffer, tool_state),
                                                            ));
                                                        }
                                                    }
                                                }

                                                // Tool call argument delta
                                                if let Some(args) = &tc.function.arguments {
                                                    tool_state.append_args(idx, args.clone());
                                                    if let Some(active) = tool_state.get_call(idx) {
                                                        return Some((
                                                            Ok(StreamChunk::ToolUseDelta {
                                                                id: active.id.clone(),
                                                                delta: args.clone(),
                                                            }),
                                                            (byte_stream, buffer, tool_state),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "Failed to parse SSE chunk");
                                }
                            }
                        }
                    } else {
                        // Need more data from the stream
                        match tokio::time::timeout(stream_idle_timeout, byte_stream.next()).await {
                            Err(_) => {
                                return Some((
                                    Err(provider_timeout()),
                                    (byte_stream, buffer, tool_state),
                                ));
                            }
                            Ok(Some(Ok(bytes))) => {
                                buffer.push(&bytes);
                            }
                            Ok(Some(Err(e))) => {
                                return Some((
                                    Err(provider_request_error(e)),
                                    (byte_stream, buffer, tool_state),
                                ));
                            }
                            Ok(None) => {
                                // Stream ended
                                if !buffer.is_empty() {
                                    tracing::warn!(
                                        remaining_bytes = buffer.len(),
                                        "Stream ended with unprocessed data"
                                    );
                                }
                                // Emit any completed tool calls
                                if let Some(completed) = tool_state.take_completed() {
                                    return Some((
                                        Ok(completed),
                                        (byte_stream, buffer, tool_state),
                                    ));
                                }
                                let stop_reason = tool_state.final_stop_reason();
                                return Some((
                                    Ok(StreamChunk::Done { stop_reason }),
                                    (byte_stream, buffer, tool_state),
                                ));
                            }
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        &self.model
    }

    fn max_context_length(&self) -> usize {
        self.max_context
    }
}

/// Tracks in-flight tool calls during streaming.
#[derive(Default)]
struct ToolCallState {
    calls: std::collections::HashMap<usize, ActiveToolCall>,
    finish_reason: Option<String>,
}

struct ActiveToolCall {
    id: String,
    #[allow(dead_code)]
    name: String,
    arguments: String,
}

impl ToolCallState {
    fn start_call(&mut self, index: usize, id: String, name: String) {
        self.calls.insert(
            index,
            ActiveToolCall {
                id,
                name,
                arguments: String::new(),
            },
        );
    }

    fn append_args(&mut self, index: usize, delta: String) {
        if let Some(call) = self.calls.get_mut(&index) {
            call.arguments.push_str(&delta);
        }
    }

    fn get_call(&self, index: usize) -> Option<&ActiveToolCall> {
        self.calls.get(&index)
    }

    fn take_completed(&mut self) -> Option<StreamChunk> {
        // Find the first call with complete JSON arguments and emit ToolUseComplete
        let keys: Vec<usize> = self.calls.keys().copied().collect();
        for key in keys {
            if let Some(call) = self.calls.get(&key) {
                // Try to parse the arguments as JSON to check if complete
                if let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.arguments) {
                    let id = call.id.clone();
                    self.calls.remove(&key);
                    return Some(StreamChunk::ToolUseComplete { id, input });
                }
            }
        }
        None
    }

    fn final_stop_reason(&self) -> StopReason {
        match self.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_messages_to_chat_system() {
        let messages = vec![Message::user("hello")];
        let chat = messages_to_chat(&messages);
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0].role, "user");
    }

    #[test]
    fn test_tools_to_chat_empty() {
        let tools: Vec<ToolDefinition> = vec![];
        let chat = tools_to_chat(&tools);
        assert!(chat.is_empty());
    }

    #[test]
    fn test_user_text_only_single_block_string_content() {
        let messages = vec![Message::user("hello")];
        let chat = messages_to_chat(&messages);
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0].role, "user");
        // Single text block should remain a plain string
        assert_eq!(chat[0].content, serde_json::json!("hello"));
    }

    #[test]
    fn test_user_image_base64_only() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Image {
                source: ImageSource::Base64 {
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                },
            }],
        }];
        let chat = messages_to_chat(&messages);
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0].role, "user");
        let arr = chat[0].content.as_array().expect("expected array content");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image_url");
        assert_eq!(
            arr[0]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo="
        );
    }

    #[test]
    fn test_user_image_url_only() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Image {
                source: ImageSource::Url {
                    url: "https://example.com/cat.jpg".to_string(),
                },
            }],
        }];
        let chat = messages_to_chat(&messages);
        let arr = chat[0].content.as_array().expect("expected array content");
        assert_eq!(arr[0]["image_url"]["url"], "https://example.com/cat.jpg");
    }

    #[test]
    fn test_user_mixed_text_and_image() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "What's in this image?".to_string(),
                },
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: "image/jpeg".to_string(),
                        data: "/9j/4AAQ".to_string(),
                    },
                },
            ],
        }];
        let chat = messages_to_chat(&messages);
        assert_eq!(chat.len(), 1);
        let arr = chat[0].content.as_array().expect("expected array content");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "What's in this image?");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(
            arr[1]["image_url"]["url"],
            "data:image/jpeg;base64,/9j/4AAQ"
        );
    }

    #[test]
    fn test_user_multiple_text_blocks_array() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "first".to_string(),
                },
                ContentBlock::Text {
                    text: "second".to_string(),
                },
            ],
        }];
        let chat = messages_to_chat(&messages);
        let arr = chat[0]
            .content
            .as_array()
            .expect("expected array for multi-text");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["text"], "first");
        assert_eq!(arr[1]["text"], "second");
    }

    mod usage_fixtures {
        use super::*;
        use fabric::llm_types::CacheTelemetry;

        const DEEPSEEK_HIT_MISS: &str =
            include_str!("../../../tests/fixtures/usage/deepseek_hit_miss.json");
        const DEEPSEEK_HIT_ONLY: &str =
            include_str!("../../../tests/fixtures/usage/deepseek_hit_only.json");
        const DEEPSEEK_EXPLICIT_ZERO_HIT: &str =
            include_str!("../../../tests/fixtures/usage/deepseek_explicit_zero_hit.json");
        const OPENAI_CACHED_TOKENS: &str =
            include_str!("../../../tests/fixtures/usage/openai_cached_tokens.json");
        const OPENAI_EXPLICIT_ZERO_CACHED: &str =
            include_str!("../../../tests/fixtures/usage/openai_explicit_zero_cached.json");
        const NEITHER: &str = include_str!("../../../tests/fixtures/usage/neither.json");
        const DEEPSEEK_HIT_GREATER_THAN_TOTAL: &str =
            include_str!("../../../tests/fixtures/usage/deepseek_hit_greater_than_total.json");
        const DEEPSEEK_MISS_NOT_CONSERVED: &str =
            include_str!("../../../tests/fixtures/usage/deepseek_miss_not_conserved.json");
        const FORMAT_CONFLICT: &str =
            include_str!("../../../tests/fixtures/usage/format_conflict.json");

        fn parse_result(json: &str) -> anyhow::Result<InferenceUsage> {
            let usage: ApiUsage = serde_json::from_str(json).unwrap();
            openai_usage(&usage, CacheReportingMode::Auto)
        }

        fn parse(json: &str) -> InferenceUsage {
            parse_result(json).unwrap()
        }

        // ---- Current behavior locked (green against today's parser) ----

        #[test]
        fn openai_cached_tokens_parse() {
            let usage = parse(OPENAI_CACHED_TOKENS);
            assert_eq!(usage.cache_read_tokens, Some(100));
            assert_eq!(usage.uncached_input_tokens, Some(156));
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
        }

        #[test]
        fn openai_explicit_zero_cached_is_distinct_from_none() {
            let usage = parse(OPENAI_EXPLICIT_ZERO_CACHED);
            assert_eq!(usage.cache_read_tokens, Some(0));
            assert_eq!(usage.uncached_input_tokens, Some(256));
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
        }

        #[test]
        fn no_cache_fields_reports_unknown() {
            // No cache-reporting fields at all: the parser cannot know whether
            // the provider supports caching, so telemetry must stay Unknown
            // rather than inventing a Reported zero hit.
            let usage = parse(NEITHER);
            assert_eq!(usage.cache_read_tokens, None);
            assert_eq!(usage.uncached_input_tokens, None);
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Unknown);
        }

        #[test]
        fn streaming_final_usage_chunk_openai_parses() {
            let envelope = format!(r#"{{"choices":[],"usage":{OPENAI_CACHED_TOKENS}}}"#);
            let response: StreamResponse = serde_json::from_str(&envelope).unwrap();
            let usage =
                openai_usage(response.usage.as_ref().unwrap(), CacheReportingMode::Auto).unwrap();
            assert_eq!(usage.cache_read_tokens, Some(100));
        }

        // ---- Gap evidence (red until C1 parses DeepSeek fields) ----

        #[test]
        fn deepseek_hit_miss_parse() {
            let usage = parse(DEEPSEEK_HIT_MISS);
            assert_eq!(usage.cache_read_tokens, Some(100));
            assert_eq!(usage.uncached_input_tokens, Some(156));
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
        }

        #[test]
        fn deepseek_hit_only_derives_miss_from_total() {
            let usage = parse(DEEPSEEK_HIT_ONLY);
            assert_eq!(usage.cache_read_tokens, Some(100));
            assert_eq!(usage.uncached_input_tokens, Some(156));
        }

        #[test]
        fn deepseek_explicit_zero_hit_is_distinct_from_none() {
            let usage = parse(DEEPSEEK_EXPLICIT_ZERO_HIT);
            assert_eq!(usage.cache_read_tokens, Some(0));
            assert_eq!(usage.uncached_input_tokens, Some(256));
        }

        #[test]
        fn deepseek_hit_greater_than_total_is_protocol_error() {
            assert!(parse_result(DEEPSEEK_HIT_GREATER_THAN_TOTAL).is_err());
        }

        #[test]
        fn deepseek_hit_plus_miss_not_conserved_is_protocol_error() {
            assert!(parse_result(DEEPSEEK_MISS_NOT_CONSERVED).is_err());
        }

        #[test]
        fn conflicting_cache_formats_is_protocol_error() {
            assert!(parse_result(FORMAT_CONFLICT).is_err());
        }

        #[test]
        fn streaming_final_usage_chunk_deepseek_parses() {
            let envelope = format!(r#"{{"choices":[],"usage":{DEEPSEEK_HIT_MISS}}}"#);
            let response: StreamResponse = serde_json::from_str(&envelope).unwrap();
            let usage =
                openai_usage(response.usage.as_ref().unwrap(), CacheReportingMode::Auto).unwrap();
            assert_eq!(usage.cache_read_tokens, Some(100));
        }

        // ---- C2: config-declared reporting modes gate telemetry interpretation ----

        fn parse_with(json: &str, mode: CacheReportingMode) -> InferenceUsage {
            let usage: ApiUsage = serde_json::from_str(json).unwrap();
            openai_usage(&usage, mode).unwrap()
        }

        #[test]
        fn unsupported_reporting_never_carries_cache_figures() {
            // A provider declared without cache reporting must not surface a
            // fabricated hit/miss even when the wire carries DeepSeek fields.
            let usage = parse_with(DEEPSEEK_HIT_MISS, CacheReportingMode::Unsupported);
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Unsupported);
            assert_eq!(usage.cache_read_tokens, None);
            assert_eq!(usage.uncached_input_tokens, None);
            assert_eq!(usage.cache_write_tokens, None);
        }

        #[test]
        fn deepseek_chat_mode_parses_hit_miss_like_auto() {
            let usage = parse_with(DEEPSEEK_HIT_MISS, CacheReportingMode::DeepSeekChat);
            assert_eq!(usage.cache_read_tokens, Some(100));
            assert_eq!(usage.uncached_input_tokens, Some(156));
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
        }

        #[test]
        fn openai_cached_mode_parses_cached_like_auto() {
            let usage = parse_with(OPENAI_CACHED_TOKENS, CacheReportingMode::OpenAiCachedTokens);
            assert_eq!(usage.cache_read_tokens, Some(100));
            assert_eq!(usage.uncached_input_tokens, Some(156));
            assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
        }
    }
}
