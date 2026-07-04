use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::provider::*;
use base::message::{ContentBlock, Message, Role};

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_context: usize,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".to_string(),
            max_context: 200_000,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_max_context(mut self, max_context: usize) -> Self {
        self.max_context = max_context;
        self
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
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

fn messages_to_api(messages: &[Message]) -> Vec<ApiMessage> {
    let len = messages.len();
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "user", // Anthropic uses system param, but we fold into user
            };
            let content = if i == len - 1 {
                // Add cache_control to last content block of last message
                if m.content.len() == 1 {
                    match &m.content[0] {
                        ContentBlock::Text { text } => {
                            serde_json::json!([{
                                "type": "text",
                                "text": text,
                                "cache_control": {"type": "ephemeral"}
                            }])
                        }
                        _ => serde_json::to_value(&m.content).unwrap_or_default(),
                    }
                } else {
                    serde_json::to_value(&m.content).unwrap_or_default()
                }
            } else {
                if m.content.len() == 1 {
                    match &m.content[0] {
                        ContentBlock::Text { text } => serde_json::json!(text),
                        _ => serde_json::to_value(&m.content).unwrap_or_default(),
                    }
                } else {
                    serde_json::to_value(&m.content).unwrap_or_default()
                }
            };
            ApiMessage {
                role: role.to_string(),
                content,
            }
        })
        .collect()
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
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            messages: messages_to_api(messages),
            tools: tools_to_api(tools),
            stream: None,
        };

        // Debug: log the full request to diagnose tool_use/tool_result ordering issues
        if std::env::var("ALETHEON_DEBUG_API").is_ok() {
            if let Ok(json) = serde_json::to_string_pretty(&request) {
                eprintln!("[API-DEBUG] Request to {}:{}\n{}", self.base_url, "/v1/messages", json);
            }
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, body);
        }

        let api_resp: ApiResponse = response.json().await?;

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
            usage: Usage {
                input_tokens: api_resp.usage.input_tokens,
                output_tokens: api_resp.usage.output_tokens,
            },
            cache_hit_tokens: api_resp.usage.cache_read_input_tokens.unwrap_or(0),
            cache_miss_tokens: api_resp.usage.cache_creation_input_tokens.unwrap_or(0),
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            messages: messages_to_api(messages),
            tools: tools_to_api(tools),
            stream: Some(true),
        };

        // Debug: log the full request to diagnose tool_use/tool_result ordering issues
        if std::env::var("ALETHEON_DEBUG_API").is_ok() {
            if let Ok(json) = serde_json::to_string_pretty(&request) {
                eprintln!("[API-DEBUG-STREAM] Request to {}:{}\n{}", self.base_url, "/v1/messages", json);
            }
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, body);
        }

        let byte_stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));

        let stream = futures::stream::unfold(
            AnthropicStreamState {
                byte_stream: Box::pin(byte_stream),
                buffer: String::new(),
                tool_state: AnthropicToolState::default(),
                usage: Usage::default(),
                stop_reason: StopReason::EndTurn,
            },
            |mut state| async move {
                loop {
                    // Try to extract a complete SSE event from the buffer
                    // Anthropic SSE format: "event: <type>\n" followed by "data: <json>\n\n"
                    if let Some(double_newline) = state.buffer.find("\n\n") {
                        let block = state.buffer[..double_newline].to_string();
                        state.buffer = state.buffer[double_newline + 2..].to_string();

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
                                        state.usage.input_tokens =
                                            msg_start.message.usage.input_tokens;
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
                                // A content block has finished
                                // If it was a tool use block, emit ToolUseComplete
                                if let Some(completed) = state.tool_state.complete_block() {
                                    return Some((Ok(completed), state));
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
                                                state.usage.output_tokens = ot;
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
                                        input_tokens: state.usage.input_tokens,
                                        output_tokens: state.usage.output_tokens,
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
                        match state.byte_stream.next().await {
                            Some(Ok(bytes)) => {
                                let text = String::from_utf8_lossy(&bytes);
                                state.buffer.push_str(&text);
                            }
                            Some(Err(e)) => {
                                return Some((
                                    Err(anyhow::anyhow!("Stream read error: {}", e)),
                                    state,
                                ));
                            }
                            None => {
                                // Stream ended
                                if !state.buffer.trim().is_empty() {
                                    tracing::warn!(remaining = %state.buffer, "Stream ended with unprocessed data");
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
    buffer: String,
    tool_state: AnthropicToolState,
    usage: Usage,
    stop_reason: StopReason,
}

/// Tracks in-flight tool use blocks during Anthropic streaming.
#[derive(Default)]
struct AnthropicToolState {
    blocks: std::collections::HashMap<usize, ActiveToolBlock>,
}

struct ActiveToolBlock {
    id: String,
    #[allow(dead_code)]
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

    fn complete_block(&mut self) -> Option<StreamChunk> {
        // Find a block that has a complete JSON buffer and remove it
        let keys: Vec<usize> = self.blocks.keys().copied().collect();
        for key in keys {
            if let Some(block) = self.blocks.get(&key) {
                // Try to parse the JSON buffer to check if it's complete
                if let Ok(input) = serde_json::from_str::<serde_json::Value>(&block.json_buffer) {
                    let id = block.id.clone();
                    self.blocks.remove(&key);
                    return Some(StreamChunk::ToolUseComplete { id, input });
                }
            }
        }
        None
    }
}
