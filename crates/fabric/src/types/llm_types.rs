//! LLM-related shared types.
//!
//! LLM-related shared types.

use async_trait::async_trait;
use futures::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::pin::Pin;

use crate::message::Message;

/// Tool definition sent to the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolDefinitionCanonicalizationError {
    #[error("tool name is empty or contains control characters")]
    InvalidName,
    #[error("duplicate tool name: {0}")]
    DuplicateName(String),
    #[error("canonical tool schema serialization failed: {0}")]
    Serialization(String),
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => BTreeMap::from_iter(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_json(value))),
        )
        .into_iter()
        .collect(),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        scalar => scalar.clone(),
    }
}

pub fn canonicalize_tool_definitions(
    definitions: &[ToolDefinition],
) -> Result<Vec<ToolDefinition>, ToolDefinitionCanonicalizationError> {
    let mut canonical = definitions.to_vec();
    for definition in &mut canonical {
        if definition.name.trim().is_empty() || definition.name.chars().any(char::is_control) {
            return Err(ToolDefinitionCanonicalizationError::InvalidName);
        }
        definition.input_schema = canonical_json(&definition.input_schema);
    }
    canonical.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in canonical.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(ToolDefinitionCanonicalizationError::DuplicateName(
                pair[0].name.clone(),
            ));
        }
    }
    Ok(canonical)
}

pub fn tool_schema_digest(
    definitions: &[ToolDefinition],
) -> Result<String, ToolDefinitionCanonicalizationError> {
    const DOMAIN: &[u8] = b"aletheon.tool-schema.v1\0";
    let canonical = canonicalize_tool_definitions(definitions)?;
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|error| ToolDefinitionCanonicalizationError::Serialization(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(encoded);
    Ok(format!("sha256:{:x}", hasher.finalize()))
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
    Usage { usage: InferenceUsage },
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

/// Host-observed facts about the effective inference route for one turn.
///
/// These values are runtime metadata, not claims made by the model itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRuntimeFacts {
    pub effective_model_id: String,
    pub display_name: String,
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

    /// Return authoritative host metadata for the effective route.
    /// Compatibility providers may expose only their display name; routed
    /// adapters should override this with the resolved model specification.
    fn runtime_facts(&self) -> ModelRuntimeFacts {
        ModelRuntimeFacts {
            effective_model_id: self.name().to_string(),
            display_name: self.name().to_string(),
            max_context_tokens: self.max_context_length(),
        }
    }

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
    pub usage: InferenceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheTelemetry {
    Reported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InferenceUsage {
    #[serde(default, alias = "input_tokens", alias = "tokens_in")]
    pub total_input_tokens: Option<u64>,
    #[serde(default, alias = "tokens_out")]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub uncached_input_tokens: Option<u64>,
    #[serde(default, alias = "cache_hit_tokens")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub cache_telemetry: CacheTelemetry,
}

impl InferenceUsage {
    pub fn reported(
        total_input_tokens: u64,
        output_tokens: u64,
        uncached_input_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
    ) -> Self {
        Self {
            total_input_tokens: Some(total_input_tokens),
            output_tokens: Some(output_tokens),
            uncached_input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            cache_telemetry: CacheTelemetry::Reported,
        }
    }

    pub fn unsupported(total_input_tokens: Option<u64>, output_tokens: Option<u64>) -> Self {
        Self {
            total_input_tokens,
            output_tokens,
            cache_telemetry: CacheTelemetry::Unsupported,
            ..Self::default()
        }
    }
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
            usage: InferenceUsage::reported(3, 2, Some(2), Some(1), None),
        };
        let response_json = serde_json::to_value(&response).unwrap();
        let decoded: LlmResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(decoded.stop_reason, StopReason::EndTurn);
        assert_eq!(decoded.usage, response.usage);
        assert_eq!(decoded.usage.cache_read_tokens, Some(1));

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
