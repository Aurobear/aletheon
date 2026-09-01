use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::*;
use ::contracts::message::{ContentBlock, Message, Role};
use cognit::config::ProviderTimeoutConfig;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    provider_identity: String,
    max_context: usize,
    max_tokens: u32,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let timeouts = ProviderTimeoutConfig::default();
        Self {
            client: Self::client(&timeouts),
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".to_string(),
            provider_identity: "anthropic".into(),
            max_context: 200_000,
            max_tokens: 4096,
            request_timeout: Duration::from_millis(timeouts.request_timeout_ms),
            stream_idle_timeout: Duration::from_millis(timeouts.stream_idle_timeout_ms),
        }
    }

    pub fn with_provider_identity(mut self, identity: impl Into<String>) -> Self {
        self.provider_identity = identity.into();
        self
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

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
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
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<ApiSystemBlock>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize)]
struct ApiSystemBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ApiContent>,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct ApiContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

fn anthropic_usage(usage: &ApiUsage) -> anyhow::Result<InferenceUsage> {
    let uncached = u64::from(usage.input_tokens);
    let read = u64::from(usage.cache_read_input_tokens.unwrap_or(0));
    let write = u64::from(usage.cache_creation_input_tokens.unwrap_or(0));
    let total = uncached
        .checked_add(read)
        .and_then(|value| value.checked_add(write))
        .ok_or_else(|| anyhow::anyhow!("anthropic input usage overflow"))?;
    Ok(InferenceUsage::reported(
        total,
        u64::from(usage.output_tokens),
        Some(uncached),
        usage.cache_read_input_tokens.map(u64::from),
        usage.cache_creation_input_tokens.map(u64::from),
    ))
}

/// SSE streaming event types for Anthropic API
#[derive(Deserialize)]
struct StreamMessageStart {
    message: StreamMessage,
}

#[derive(Deserialize)]
struct StreamMessage {
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct StreamContentBlockStart {
    index: usize,
    content_block: StreamContentBlock,
}

#[derive(Deserialize)]
struct StreamContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct StreamContentBlockDelta {
    index: usize,
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamContentBlockStop {
    index: usize,
}

#[derive(Deserialize)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct StreamMessageDelta {
    delta: StreamMessageDeltaInner,
    #[serde(default)]
    usage: Option<StreamUsage>,
}

#[derive(Deserialize)]
struct StreamMessageDeltaInner {
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamUsage {
    #[serde(default)]
    output_tokens: Option<u32>,
}

fn messages_to_api(messages: &[Message]) -> (Vec<ApiSystemBlock>, Vec<ApiMessage>) {
    let mut system = Vec::new();
    let mut dynamic = Vec::new();
    for message in messages {
        if message.role == Role::System {
            for block in &message.content {
                match block {
                    ContentBlock::Text { text } | ContentBlock::System { text, .. } => {
                        system.push(ApiSystemBlock {
                            block_type: "text",
                            text: text.clone(),
                            cache_control: None,
                        });
                    }
                    _ => tracing::warn!("ignoring non-text block in Anthropic system message"),
                }
            }
            continue;
        }
        {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => unreachable!("system messages were partitioned"),
            };
            let content = if message.content.len() == 1 {
                match &message.content[0] {
                    ContentBlock::Text { text } => serde_json::json!(text),
                    _ => serde_json::to_value(&message.content).unwrap_or_default(),
                }
            } else {
                serde_json::to_value(&message.content).unwrap_or_default()
            };
            dynamic.push(ApiMessage {
                role: role.to_string(),
                content,
            });
        }
    }
    if let Some(last) = system.last_mut() {
        last.cache_control = Some(serde_json::json!({"type": "ephemeral"}));
    }
    (system, dynamic)
}

fn tools_to_api(tools: &[ToolDefinition]) -> Vec<ApiTool> {
    let len = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| ApiTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
            cache_control: if i == len - 1 {
                Some(serde_json::json!({"type": "ephemeral"}))
            } else {
                None
            },
        })
        .collect()
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let tools = canonicalize_tool_definitions(tools)?;
        let (system, messages) = messages_to_api(messages);
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            // ... (line 236, complete())
            system,
            messages,
            tools: tools_to_api(&tools),
            stream: None,
        };

        // Debug: log the full request to diagnose tool_use/tool_result ordering issues
        if std::env::var("ALETHEON_DEBUG_API").is_ok() {
            if let Ok(json) = serde_json::to_string_pretty(&request) {
                eprintln!(
                    "[API-DEBUG] Request to {}:/v1/messages\n{}",
                    self.base_url, json
                );
            }
        }

        let api_resp: ApiResponse = tokio::time::timeout(self.request_timeout, async {
            let response = self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(provider_request_error)?;

            if !response.status().is_success() {
                return Err(InferenceFailure::from_http_response(
                    response,
                    &self.provider_identity,
                )
                .await);
            }
            response.json().await.map_err(provider_request_error)
        })
        .await
        .map_err(|_| provider_timeout())??;

        let content: Vec<ContentBlock> = api_resp
            .content
            .into_iter()
            .filter_map(|c| match c.content_type.as_str() {
                "text" => Some(ContentBlock::Text {
                    text: c.text.unwrap_or_default(),
                }),
                "tool_use" => Some(ContentBlock::ToolUse {
                    id: c.id.unwrap_or_default(),
                    name: c.name.unwrap_or_default(),
                    input: c.input.unwrap_or(serde_json::Value::Null),
                }),
                "thinking" => {
                    // Skip thinking blocks (extended thinking)
                    tracing::debug!("Skipping thinking block");
                    None
                }
                _ => {
                    tracing::warn!(content_type = %c.content_type, "Unknown content type");
                    Some(ContentBlock::Text {
                        text: format!("[unknown content type: {}]", c.content_type),
                    })
                }
            })
            .collect();

        let stop_reason = match api_resp.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        Ok(LlmResponse {
            content,
            stop_reason,
            usage: anthropic_usage(&api_resp.usage)?,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let tools = canonicalize_tool_definitions(tools)?;
        let (system, messages) = messages_to_api(messages);
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            // ... (line 236, complete())
            system,
            messages,
            tools: tools_to_api(&tools),
            stream: Some(true),
        };

        // Debug: log the full request to diagnose tool_use/tool_result ordering issues
        if std::env::var("ALETHEON_DEBUG_API").is_ok() {
            if let Ok(json) = serde_json::to_string_pretty(&request) {
                eprintln!(
                    "[API-DEBUG-STREAM] Request to {}:/v1/messages\n{}",
                    self.base_url, json
                );
            }
        }

        let response = tokio::time::timeout(
            self.request_timeout,
            self.client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send(),
        )
        .await
        .map_err(|_| provider_timeout())?
        .map_err(provider_request_error)?;

        if !response.status().is_success() {
            return Err(
                InferenceFailure::from_http_response(response, &self.provider_identity).await,
            );
        }

        let byte_stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));

        let stream = futures::stream::unfold(
            AnthropicStreamState {
                byte_stream: Box::pin(byte_stream),
                buffer: crate::utf8_stream::Utf8StreamBuffer::default(),
                tool_state: AnthropicToolState::default(),
                usage: InferenceUsage::default(),
                stop_reason: StopReason::EndTurn,
                stream_idle_timeout: self.stream_idle_timeout,
            },
            |mut state| async move {
                loop {
                    // Try to extract a complete SSE event from the buffer
                    // Anthropic SSE format: "event: <type>\n" followed by "data: <json>\n\n"
                    let block = match state.buffer.take_event() {
                        Ok(block) => block,
                        Err(error) => {
                            return Some((
                                Err(anyhow::anyhow!(
                                    "provider stream contained invalid UTF-8: {error}"
                                )),
                                state,
                            ));
                        }
                    };
                    if let Some(block) = block {
                        let mut event_type = String::new();
                        let mut data = String::new();

                        for line in block.lines() {
                            if let Some(et) = line.strip_prefix("event: ") {
                                event_type = et.trim().to_string();
                            } else if let Some(d) = line.strip_prefix("data: ") {
                                data = d.trim().to_string();
                            }
                        }

                        if event_type.is_empty() || data.is_empty() {
                            continue;
                        }

                        match event_type.as_str() {
                            "message_start" => {
                                match serde_json::from_str::<StreamMessageStart>(&data) {
                                    Ok(msg_start) => {
                                        match anthropic_usage(&msg_start.message.usage) {
                                            Ok(usage) => state.usage = usage,
                                            Err(error) => {
                                                tracing::warn!(%error, "invalid Anthropic usage")
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse message_start");
                                    }
                                }
                            }
                            "content_block_start" => {
                                match serde_json::from_str::<StreamContentBlockStart>(&data) {
                                    Ok(block_start) => {
                                        match block_start.content_block.block_type.as_str() {
                                            "text" => {
                                                // Text block started, will receive deltas
                                            }
                                            "tool_use" => {
                                                let id = block_start
                                                    .content_block
                                                    .id
                                                    .unwrap_or_default();
                                                let name = block_start
                                                    .content_block
                                                    .name
                                                    .unwrap_or_default();
                                                state.tool_state.start_block(
                                                    block_start.index,
                                                    id.clone(),
                                                    name.clone(),
                                                );
                                                return Some((
                                                    Ok(StreamChunk::ToolUseStart { id, name }),
                                                    state,
                                                ));
                                            }
                                            _ => {
                                                tracing::debug!(
                                                    block_type = %block_start.content_block.block_type,
                                                    "Skipping content block type"
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse content_block_start");
                                    }
                                }
                            }
                            "content_block_delta" => {
                                match serde_json::from_str::<StreamContentBlockDelta>(&data) {
                                    Ok(delta) => match delta.delta.delta_type.as_str() {
                                        "text_delta" => {
                                            if let Some(text) = delta.delta.text {
                                                if !text.is_empty() {
                                                    return Some((
                                                        Ok(StreamChunk::TextDelta { text }),
                                                        state,
                                                    ));
                                                }
                                            }
                                        }
                                        "input_json_delta" => {
                                            if let Some(json_str) = delta.delta.partial_json {
                                                state
                                                    .tool_state
                                                    .append_json(delta.index, json_str.clone());
                                                if let Some(block) =
                                                    state.tool_state.get_block(delta.index)
                                                {
                                                    return Some((
                                                        Ok(StreamChunk::ToolUseDelta {
                                                            id: block.id.clone(),
                                                            delta: json_str,
                                                        }),
                                                        state,
                                                    ));
                                                }
                                            }
                                        }
                                        _ => {
                                            tracing::debug!(
                                                delta_type = %delta.delta.delta_type,
                                                "Skipping content block delta type"
                                            );
                                        }
                                    },
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse content_block_delta");
                                    }
                                }
                            }
                            "content_block_stop" => {
                                match serde_json::from_str::<StreamContentBlockStop>(&data) {
                                    Ok(block_stop) => {
                                        if let Some(settled) =
                                            state.tool_state.settle_block(block_stop.index)
                                        {
                                            return Some((settled, state));
                                        }
                                    }
                                    Err(error) => tracing::warn!(
                                        %error,
                                        "Failed to parse content_block_stop"
                                    ),
                                }
                            }
                            "message_delta" => {
                                match serde_json::from_str::<StreamMessageDelta>(&data) {
                                    Ok(msg_delta) => {
                                        if let Some(sr) = msg_delta.delta.stop_reason {
                                            state.stop_reason = match sr.as_str() {
                                                "end_turn" => StopReason::EndTurn,
                                                "tool_use" => StopReason::ToolUse,
                                                "max_tokens" => StopReason::MaxTokens,
                                                _ => StopReason::EndTurn,
                                            };
                                        }
                                        if let Some(u) = msg_delta.usage {
                                            if let Some(ot) = u.output_tokens {
                                                state.usage.output_tokens = Some(u64::from(ot));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Failed to parse message_delta");
                                    }
                                }
                            }
                            "message_stop" => {
                                return Some((
                                    Ok(StreamChunk::Usage {
                                        usage: state.usage.clone(),
                                    }),
                                    state,
                                ));
                            }
                            "ping" => {
                                // Ignore ping events
                            }
                            _ => {
                                tracing::debug!(event = %event_type, "Unknown SSE event type");
                            }
                        }
                    } else {
                        // Need more data from the stream
                        let stream_idle_timeout = state.stream_idle_timeout;
                        match tokio::time::timeout(stream_idle_timeout, state.byte_stream.next())
                            .await
                        {
                            Err(_) => return Some((Err(provider_timeout()), state)),
                            Ok(Some(Ok(bytes))) => {
                                state.buffer.push(&bytes);
                            }
                            Ok(Some(Err(e))) => {
                                return Some((Err(provider_request_error(e)), state));
                            }
                            Ok(None) => {
                                // Stream ended
                                if !state.buffer.is_empty() {
                                    tracing::warn!(
                                        remaining_bytes = state.buffer.len(),
                                        "Stream ended with unprocessed data"
                                    );
                                }
                                if let Some(settled) = state.tool_state.take_settled() {
                                    return Some((settled, state));
                                }
                                return Some((
                                    Ok(StreamChunk::Done {
                                        stop_reason: state.stop_reason.clone(),
                                    }),
                                    state,
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

/// State for the Anthropic SSE stream parser.
struct AnthropicStreamState {
    byte_stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>,
    buffer: crate::utf8_stream::Utf8StreamBuffer,
    tool_state: AnthropicToolState,
    usage: InferenceUsage,
    stop_reason: StopReason,
    stream_idle_timeout: Duration,
}

/// Tracks in-flight tool use blocks during Anthropic streaming.
#[derive(Default)]
struct AnthropicToolState {
    blocks: std::collections::HashMap<usize, ActiveToolBlock>,
}

struct ActiveToolBlock {
    id: String,
    name: String,
    json_buffer: String,
}

impl AnthropicToolState {
    fn start_block(&mut self, index: usize, id: String, name: String) {
        self.blocks.insert(
            index,
            ActiveToolBlock {
                id,
                name,
                json_buffer: String::new(),
            },
        );
    }

    fn append_json(&mut self, index: usize, delta: String) {
        if let Some(block) = self.blocks.get_mut(&index) {
            block.json_buffer.push_str(&delta);
        }
    }

    fn get_block(&self, index: usize) -> Option<&ActiveToolBlock> {
        self.blocks.get(&index)
    }

    fn settle_block(&mut self, index: usize) -> Option<anyhow::Result<StreamChunk>> {
        let block = self.blocks.remove(&index)?;
        Some(settle_anthropic_block(block))
    }

    fn take_settled(&mut self) -> Option<anyhow::Result<StreamChunk>> {
        let index = self.blocks.keys().copied().min()?;
        self.settle_block(index)
    }
}

fn settle_anthropic_block(block: ActiveToolBlock) -> anyhow::Result<StreamChunk> {
    serde_json::from_str::<serde_json::Value>(&block.json_buffer)
        .map(|input| StreamChunk::ToolUseComplete {
            id: block.id.clone(),
            input,
        })
        .map_err(|error| {
            anyhow::Error::new(::contracts::MalformedToolArgumentsError::from_json_error(
                &block.id,
                &block.name,
                block.json_buffer.len(),
                &error,
            ))
        })
}

#[cfg(test)]
mod cache_contract_tests {
    use super::*;

    #[test]
    fn content_block_stop_settles_valid_split_json() {
        let mut state = AnthropicToolState::default();
        state.start_block(3, "tool-3".into(), "search".into());
        state.append_json(3, "{".into());
        state.append_json(3, "\"query\":\"term\"}".into());

        match state.settle_block(3).unwrap().unwrap() {
            StreamChunk::ToolUseComplete { id, input } => {
                assert_eq!(id, "tool-3");
                assert_eq!(input, serde_json::json!({"query": "term"}));
            }
            chunk => panic!("unexpected chunk: {chunk:?}"),
        }
    }

    #[test]
    fn content_block_stop_emits_typed_error_for_invalid_json() {
        let mut state = AnthropicToolState::default();
        state.start_block(7, "tool-7".into(), "search".into());
        state.append_json(7, "{invalid".into());

        let error = state.settle_block(7).unwrap().unwrap_err();
        let typed = error
            .downcast_ref::<::contracts::MalformedToolArgumentsError>()
            .unwrap();
        assert_eq!(typed.tool_id, "tool-7");
        assert_eq!(typed.tool_name, "search");
        assert_eq!(typed.parse_kind, "json_syntax");
        assert_eq!(typed.argument_bytes, 8);
        assert!(state.take_settled().is_none());
    }

    #[test]
    fn system_and_tool_breakpoints_are_stable_and_dynamic_messages_are_unmarked() {
        let messages = vec![
            Message::system("stable system"),
            Message::user("dynamic user"),
        ];
        let (system, dynamic) = messages_to_api(&messages);
        let tools = ::contracts::canonicalize_tool_definitions(&[
            ToolDefinition {
                name: "zeta".into(),
                description: "z".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "alpha".into(),
                description: "a".into(),
                input_schema: serde_json::json!({}),
            },
        ])
        .unwrap();
        let body = serde_json::to_value(ApiRequest {
            model: "model".into(),
            max_tokens: 1,
            system,
            messages: dynamic,
            tools: tools_to_api(&tools),
            stream: None,
        })
        .unwrap();

        assert_eq!(body["system"][0]["text"], "stable system");
        assert!(body["system"][0]["cache_control"].is_object());
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body["messages"][0]["content"]
            .get("cache_control")
            .is_none());
        assert_eq!(body["tools"][0]["name"], "alpha");
        assert_eq!(body["tools"][1]["name"], "zeta");
        assert!(body["tools"][1]["cache_control"].is_object());
    }

    #[test]
    fn anthropic_usage_counts_native_cache_dimensions() {
        let usage = anthropic_usage(&ApiUsage {
            input_tokens: 7,
            output_tokens: 3,
            cache_creation_input_tokens: Some(5),
            cache_read_input_tokens: Some(11),
        })
        .unwrap();
        assert_eq!(usage.total_input_tokens, Some(23));
        assert_eq!(usage.uncached_input_tokens, Some(7));
        assert_eq!(usage.cache_read_tokens, Some(11));
        assert_eq!(usage.cache_write_tokens, Some(5));
    }
}
