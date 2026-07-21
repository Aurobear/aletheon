use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::provider::*;
use crate::config::ProviderTimeoutConfig;
use fabric::message::{ContentBlock, Message, Role};

/// Ollama provider using the native `/api/chat` endpoint.
///
/// Default base_url: `http://localhost:11434`
/// Supports streaming via NDJSON (newline-delimited JSON).
pub struct OllamaProvider {
    client: Client,
    model: String,
    base_url: String,
    max_context: usize,
    max_tokens: u32,
}

impl OllamaProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            model: model.into(),
            base_url: "http://localhost:11434".to_string(),
            max_context: 128_000,
            max_tokens: 100_000,
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

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_timeouts(mut self, timeouts: ProviderTimeoutConfig) -> anyhow::Result<Self> {
        self.client = Client::builder()
            .connect_timeout(Duration::from_millis(timeouts.connect_timeout_ms))
            .timeout(Duration::from_millis(timeouts.request_timeout_ms))
            .build()?;
        Ok(self)
    }
}

// ---------------------------------------------------------------------------
// Request / Response types for Ollama /api/chat
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatOptions {
    num_predict: u32,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(Serialize, Deserialize)]
struct OllamaFunction {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunctionDef,
}

#[derive(Serialize)]
struct OllamaFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

/// Non-streaming response from Ollama.
#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
    #[allow(dead_code)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

/// Streaming chunk from Ollama (NDJSON).
#[derive(Deserialize)]
struct OllamaStreamChunk {
    #[serde(default)]
    message: Option<StreamMessage>,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

// ---------------------------------------------------------------------------
// Message / tool conversion
// ---------------------------------------------------------------------------

fn messages_to_ollama(messages: &[Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };
            // Extract text content
            let text = m
                .content
                .iter()
                .filter_map(|c| match c {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            // Extract tool_use blocks from assistant messages
            let tool_calls: Vec<OllamaToolCall> = m
                .content
                .iter()
                .filter_map(|c| match c {
                    ContentBlock::ToolUse { name, input, .. } => Some(OllamaToolCall {
                        function: OllamaFunction {
                            name: name.clone(),
                            arguments: input.clone(),
                        },
                    }),
                    _ => None,
                })
                .collect();

            // Extract tool result from ToolResult blocks
            let tool_result_text = m.content.iter().find_map(|c| match c {
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    if *is_error {
                        Some(format!("[ERROR] {}", content))
                    } else {
                        Some(content.clone())
                    }
                }
                _ => None,
            });

            let final_text = if let Some(tr) = tool_result_text {
                tr
            } else {
                text
            };

            ChatMessage {
                role: role.to_string(),
                content: final_text,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
            }
        })
        .collect()
}

fn tools_to_ollama(tools: &[ToolDefinition]) -> Vec<OllamaTool> {
    tools
        .iter()
        .map(|t| OllamaTool {
            tool_type: "function".to_string(),
            function: OllamaFunctionDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------
// LlmProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_ollama(messages),
            tools: tools_to_ollama(tools),
            stream: false,
            options: ChatOptions {
                num_predict: self.max_tokens,
            },
        };

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error {}: {}", status, body);
        }

        let api_resp: ChatResponse = response.json().await?;

        let mut content = Vec::new();

        // Text content
        if !api_resp.message.content.is_empty() {
            content.push(ContentBlock::Text {
                text: api_resp.message.content,
            });
        }

        // Tool calls
        if let Some(tool_calls) = api_resp.message.tool_calls {
            for tc in tool_calls {
                content.push(ContentBlock::ToolUse {
                    // Ollama doesn't provide tool call IDs; generate one
                    id: format!("ollama_tc_{}", uuid::Uuid::new_v4()),
                    name: tc.function.name,
                    input: tc.function.arguments,
                });
            }
        }

        let usage = Usage {
            input_tokens: api_resp.prompt_eval_count.unwrap_or(0),
            output_tokens: api_resp.eval_count.unwrap_or(0),
        };

        let has_tool_use = content
            .iter()
            .any(|c| matches!(c, ContentBlock::ToolUse { .. }));

        Ok(LlmResponse {
            content,
            stop_reason: if has_tool_use {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            },
            usage,
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages_to_ollama(messages),
            tools: tools_to_ollama(tools),
            stream: true,
            options: ChatOptions {
                num_predict: self.max_tokens,
            },
        };

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error {}: {}", status, body);
        }

        let byte_stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));

        // Ollama uses NDJSON — one JSON object per line
        let stream = futures::stream::unfold(
            OllamaStreamState {
                byte_stream: Box::pin(byte_stream),
                buffer: String::new(),
                tool_state: OllamaToolState::default(),
                usage: Usage::default(),
            },
            |mut state| async move {
                loop {
                    // Try to extract a complete NDJSON line from the buffer
                    if let Some(line_end) = state.buffer.find('\n') {
                        let line = state.buffer[..line_end].trim().to_string();
                        state.buffer = state.buffer[line_end + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<OllamaStreamChunk>(&line) {
                            Ok(chunk) => {
                                // Final chunk with usage stats
                                if chunk.done {
                                    state.usage.input_tokens =
                                        chunk.prompt_eval_count.unwrap_or(state.usage.input_tokens);
                                    state.usage.output_tokens =
                                        chunk.eval_count.unwrap_or(state.usage.output_tokens);

                                    // Emit any pending tool completions first
                                    if let Some(completed) = state.tool_state.take_completed() {
                                        // We need to emit Done after this, so stash the usage
                                        // and return the completed chunk. On the next poll
                                        // we'll emit Done. For simplicity, emit usage + done.
                                        return Some((Ok(completed), state));
                                    }

                                    return Some((
                                        Ok(super::provider::StreamChunk::Usage {
                                            input_tokens: state.usage.input_tokens,
                                            output_tokens: state.usage.output_tokens,
                                        }),
                                        state,
                                    ));
                                }

                                if let Some(msg) = chunk.message {
                                    // Handle text content
                                    if let Some(text) = msg.content {
                                        if !text.is_empty() {
                                            return Some((
                                                Ok(super::provider::StreamChunk::TextDelta {
                                                    text,
                                                }),
                                                state,
                                            ));
                                        }
                                    }

                                    // Handle tool calls — Ollama sends full arguments
                                    // in each chunk, so we process the first tool call
                                    // and return immediately.
                                    if let Some(tool_calls) = msg.tool_calls {
                                        if let Some(tc) = tool_calls.into_iter().next() {
                                            let idx = state.tool_state.next_index();
                                            let id = format!("ollama_tc_{}", uuid::Uuid::new_v4());
                                            state.tool_state.start_call(
                                                idx,
                                                id.clone(),
                                                tc.function.name.clone(),
                                            );
                                            state.tool_state.set_args(
                                                idx,
                                                serde_json::to_string(&tc.function.arguments)
                                                    .unwrap_or_default(),
                                            );

                                            return Some((
                                                Ok(super::provider::StreamChunk::ToolUseStart {
                                                    id,
                                                    name: tc.function.name,
                                                }),
                                                state,
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(data = %line, error = %e, "Failed to parse Ollama NDJSON chunk");
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
                                    tracing::warn!(
                                        remaining = %state.buffer,
                                        "Ollama stream ended with unprocessed data"
                                    );
                                }
                                return Some((
                                    Ok(super::provider::StreamChunk::Done {
                                        stop_reason: StopReason::EndTurn,
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

// ---------------------------------------------------------------------------
// Streaming state
// ---------------------------------------------------------------------------

struct OllamaStreamState {
    byte_stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>,
    buffer: String,
    tool_state: OllamaToolState,
    usage: Usage,
}

#[derive(Default)]
struct OllamaToolState {
    calls: std::collections::HashMap<usize, ActiveToolCall>,
    next_index: usize,
}

struct ActiveToolCall {
    id: String,
    /// Tool name — stored for diagnostics but not currently read during execution.
    #[allow(dead_code)]
    name: String,
    arguments: String,
}

impl OllamaToolState {
    fn next_index(&mut self) -> usize {
        let idx = self.next_index;
        self.next_index += 1;
        idx
    }

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

    fn set_args(&mut self, index: usize, args: String) {
        if let Some(call) = self.calls.get_mut(&index) {
            call.arguments = args;
        }
    }

    fn take_completed(&mut self) -> Option<super::provider::StreamChunk> {
        let keys: Vec<usize> = self.calls.keys().copied().collect();
        for key in keys {
            if let Some(call) = self.calls.get(&key) {
                if let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.arguments) {
                    let id = call.id.clone();
                    self.calls.remove(&key);
                    return Some(super::provider::StreamChunk::ToolUseComplete { id, input });
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_messages_to_ollama_system() {
        let messages = vec![Message::system("You are helpful.")];
        let ollama_msgs = messages_to_ollama(&messages);
        assert_eq!(ollama_msgs.len(), 1);
        assert_eq!(ollama_msgs[0].role, "system");
        assert_eq!(ollama_msgs[0].content, "You are helpful.");
    }

    #[test]
    fn test_messages_to_ollama_user() {
        let messages = vec![Message::user("hello")];
        let ollama_msgs = messages_to_ollama(&messages);
        assert_eq!(ollama_msgs.len(), 1);
        assert_eq!(ollama_msgs[0].role, "user");
        assert_eq!(ollama_msgs[0].content, "hello");
    }

    #[test]
    fn test_messages_to_ollama_tool_result() {
        let messages = vec![Message::tool_result("tc_123", "result data", false)];
        let ollama_msgs = messages_to_ollama(&messages);
        assert_eq!(ollama_msgs.len(), 1);
        assert_eq!(ollama_msgs[0].role, "user");
        assert_eq!(ollama_msgs[0].content, "result data");
    }

    #[test]
    fn test_messages_to_ollama_tool_result_error() {
        let messages = vec![Message::tool_result("tc_123", "something broke", true)];
        let ollama_msgs = messages_to_ollama(&messages);
        assert_eq!(ollama_msgs.len(), 1);
        assert_eq!(ollama_msgs[0].content, "[ERROR] something broke");
    }

    #[test]
    fn test_messages_to_ollama_assistant_with_tool_use() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Let me search.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "tc_1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"query": "test"}),
                },
            ],
        }];
        let ollama_msgs = messages_to_ollama(&messages);
        assert_eq!(ollama_msgs.len(), 1);
        assert_eq!(ollama_msgs[0].role, "assistant");
        assert!(ollama_msgs[0].tool_calls.is_some());
        let tc = ollama_msgs[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].function.name, "search");
    }

    #[test]
    fn test_tools_to_ollama_empty() {
        let tools: Vec<ToolDefinition> = vec![];
        let ollama_tools = tools_to_ollama(&tools);
        assert!(ollama_tools.is_empty());
    }

    #[test]
    fn test_tools_to_ollama_conversion() {
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        }];
        let ollama_tools = tools_to_ollama(&tools);
        assert_eq!(ollama_tools.len(), 1);
        assert_eq!(ollama_tools[0].tool_type, "function");
        assert_eq!(ollama_tools[0].function.name, "read_file");
    }

    #[test]
    fn test_ollama_tool_state_take_completed() {
        let mut state = OllamaToolState::default();
        let idx = state.next_index();
        state.start_call(idx, "id_1".to_string(), "test".to_string());
        state.set_args(idx, r#"{"key": "value"}"#.to_string());

        let result = state.take_completed();
        assert!(result.is_some());
        use super::StreamChunk;
        match result.unwrap() {
            StreamChunk::ToolUseComplete { id, input } => {
                assert_eq!(id, "id_1");
                assert_eq!(input, serde_json::json!({"key": "value"}));
            }
            _ => panic!("Expected ToolUseComplete"),
        }
    }

    #[test]
    fn test_ollama_tool_state_incomplete_json() {
        let mut state = OllamaToolState::default();
        let idx = state.next_index();
        state.start_call(idx, "id_1".to_string(), "test".to_string());
        state.set_args(idx, r#"{"key": "valu"#.to_string());

        let result = state.take_completed();
        assert!(result.is_none());
    }
}
