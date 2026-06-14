use async_trait::async_trait;
use futures::Stream;

use std::pin::Pin;

use aletheon_abi::message::Message;

/// A chunk of a streamed LLM response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content delta
    TextDelta { text: String },
    /// Tool use start (name + id)
    ToolUseStart { id: String, name: String },
    /// Tool use input delta (partial JSON)
    ToolUseDelta { id: String, delta: String },
    /// Tool use complete
    ToolUseComplete { id: String, input: serde_json::Value },
    /// Usage update
    Usage { input_tokens: u32, output_tokens: u32 },
    /// Stream complete
    Done { stop_reason: StopReason },
}

/// A pinned, boxed stream of `StreamChunk` results.
pub type LlmStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>;

/// Canonical LlmProvider trait. See shared/traits.md.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send messages and get a response with optional tool calls.
    async fn complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> anyhow::Result<LlmResponse>;

    /// Stream a response. Default implementation falls back to `complete()`.
    async fn complete_stream(&self, messages: &[Message], tools: &[ToolDefinition]) -> anyhow::Result<LlmStream>;

    /// Provider name (e.g., "anthropic", "llama-cpp").
    fn name(&self) -> &str;

    /// Maximum context length in tokens.
    fn max_context_length(&self) -> usize;
}

/// Tool definition sent to the LLM.
pub use aletheon_abi::ToolDefinition;

/// Response from the LLM.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<aletheon_abi::message::ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
