//! LLM-related shared types.
//!
//! LLM-related shared types.

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::message::Message;

/// Tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A chunk of a streamed LLM response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content delta
    TextDelta { text: String },
    /// Thinking/reasoning content delta (for models that support extended thinking)
    ThinkingDelta { text: String },
    /// Tool use start (name + id)
    ToolUseStart { id: String, name: String },
    /// Tool use input delta (partial JSON)
    ToolUseDelta { id: String, delta: String },
    /// Tool use complete
    ToolUseComplete {
        id: String,
        input: serde_json::Value,
    },
    /// Usage update
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Stream complete
    Done { stop_reason: StopReason },
}

/// A pinned, boxed stream of `StreamChunk` results.
pub type LlmStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>;

/// Model information for TUI status bar display.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Human-readable model name (e.g., "claude-sonnet-4-6").
    pub name: String,
    /// Maximum context length in tokens.
    pub max_context: usize,
}

/// Canonical LlmProvider trait. See shared/traits.md.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send messages and get a response with optional tool calls.
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse>;

    /// Stream a response. Default implementation falls back to `complete()`.
    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream>;

    /// Provider name (e.g., "anthropic", "llama-cpp").
    fn name(&self) -> &str;

    /// Maximum context length in tokens.
    fn max_context_length(&self) -> usize;

    /// Human-readable model info for status bar display.
    ///
    /// Default implementation uses `name()` and `max_context_length()`.
    /// Providers can override to return a more specific model name.
    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            name: self.name().to_string(),
            max_context: self.max_context_length(),
        }
    }
}

/// Response from the LLM.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<crate::message::ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// Tokens that hit the provider's cache (e.g. DeepSeek cached_tokens, Anthropic cache_read)
    pub cache_hit_tokens: u32,
    /// Tokens that missed the cache and were processed fresh
    pub cache_miss_tokens: u32,
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
