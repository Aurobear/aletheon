use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::provider::*;
use aletheon_abi::message::{ContentBlock, ImageSource, Message, Role};

/// OpenAI-compatible provider (chat/completions).
/// Works with OpenAI, DeepSeek, Ollama, LM Studio, vLLM, Xiaomi MiMo, etc.
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_context: usize,
}

impl OpenAiProvider {
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_context: 128_000,
        }
    }

    pub fn with_max_context(mut self, max_context: usize) -> Self {
        self.max_context = max_context;
        self
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
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
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
                        format!("data:{};base64,{}", media_type, data)
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
                // Check if this is a tool result
                if let Some(ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                }) = msg.content.first()
                {
                    let text = if *is_error {
                        format!("[ERROR] {}", content)
                    } else {
                        content.clone()
                    };
                    result.push(ChatMessage {
                        role: "tool".to_string(),
                        content: serde_json::json!(text),
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id.clone()),
                    });
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
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_chat(messages),
            tools: tools_to_chat(tools),
            max_tokens: Some(4096),
            stream: None,
        };

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {}: {}", status, body);
        }

        let api_resp: ChatResponse = response.json().await?;

        let choice = api_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

        let mut content = Vec::new();

        // Text content
        if let Some(text) = choice.message.content {
            if !text.is_empty() {
                content.push(ContentBlock::Text { text });
            }
        }

        // Tool calls
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
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

        let (cache_hit, cache_miss, usage) = if let Some(u) = api_resp.usage {
            let hit = u
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
                .unwrap_or(0);
            let miss = u.prompt_tokens.saturating_sub(hit);
            (
                hit,
                miss,
                Usage {
                    input_tokens: u.prompt_tokens,
                    output_tokens: u.completion_tokens,
                },
            )
        } else {
            (0, 0, Usage::default())
        };

        Ok(LlmResponse {
            content,
            stop_reason,
            usage,
            cache_hit_tokens: cache_hit,
            cache_miss_tokens: cache_miss,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_chat(messages),
            tools: tools_to_chat(tools),
            max_tokens: Some(4096),
            stream: Some(true),
        };

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {}: {}", status, body);
        }

        let byte_stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));

        let stream = futures::stream::unfold(
            (
                Box::pin(byte_stream),
                String::new(),
                ToolCallState::default(),
            ),
            |(mut byte_stream, mut buffer, mut tool_state)| async move {
                use futures::StreamExt;

                loop {
                    // Try to extract a complete SSE line from the buffer
                    if let Some(line_end) = buffer.find('\n') {
                        let line = buffer[..line_end].trim().to_string();
                        buffer = buffer[line_end + 1..].to_string();

                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if data == "[DONE]" {
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
                                            Ok(StreamChunk::Usage {
                                                input_tokens: usage.prompt_tokens,
                                                output_tokens: usage.completion_tokens,
                                            }),
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

                                        // Handle tool calls
                                        if let Some(tool_calls) = &choice.delta.tool_calls {
                                            for tc in tool_calls {
                                                let idx = tc.index;

                                                // New tool call started
                                                if let Some(id) = &tc.id {
                                                    if let Some(name) = &tc.function.name {
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
                                    tracing::warn!(data = %data, error = %e, "Failed to parse SSE chunk");
                                }
                            }
                        }
                    } else {
                        // Need more data from the stream
                        match byte_stream.next().await {
                            Some(Ok(bytes)) => {
                                let text = String::from_utf8_lossy(&bytes);
                                buffer.push_str(&text);
                            }
                            Some(Err(e)) => {
                                return Some((
                                    Err(anyhow::anyhow!("Stream read error: {}", e)),
                                    (byte_stream, buffer, tool_state),
                                ));
                            }
                            None => {
                                // Stream ended
                                if !buffer.trim().is_empty() {
                                    tracing::warn!(remaining = %buffer, "Stream ended with unprocessed data");
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
}
