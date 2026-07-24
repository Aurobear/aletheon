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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceCapabilities {
    pub streaming: bool,
    pub tool_calls: bool,
    pub max_context_tokens: usize,
}

/// Canonical LlmProvider trait. See shared/traits.md.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            max_context_tokens: self.max_context_length(),
        }
    }

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
    #[deprecated(note = "Use `name()` and `max_context_length()` directly instead")]
    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            name: self.name().to_string(),
            max_context: self.max_context_length(),
        }
    }
}

/// Response from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: Vec<crate::message::ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// Tokens that hit the provider's cache (e.g. DeepSeek cached_tokens, Anthropic cache_read)
    pub cache_hit_tokens: u32,
    /// Tokens that missed the cache and were processed fresh
    pub cache_miss_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ContentBlock;

    #[test]
    fn inference_response_and_stream_frames_round_trip() {
        let response = LlmResponse {
            content: vec![ContentBlock::Text { text: "ok".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
            cache_hit_tokens: 1,
            cache_miss_tokens: 2,
        };
        let response_json = serde_json::to_value(&response).unwrap();
        let decoded: LlmResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(decoded.stop_reason, StopReason::EndTurn);
        assert_eq!(decoded.usage, response.usage);
        assert_eq!(decoded.cache_hit_tokens, 1);
        assert_eq!(decoded.cache_miss_tokens, 2);

        for chunk in [
            StreamChunk::TextDelta { text: "a".into() },
            StreamChunk::Done {
                stop_reason: StopReason::EndTurn,
            },
        ] {
            let json = serde_json::to_value(&chunk).unwrap();
            let decoded: StreamChunk = serde_json::from_value(json).unwrap();
            assert_eq!(decoded, chunk);
        }
    }
}
