use crate::adapters::inference::{LlmProvider, LlmResponse, LlmStream, ToolDefinition};
use anyhow::Result;
use fabric::{ContentBlock, Message, Role};
use std::sync::Arc;

/// Wraps LlmProvider for use by CognitCore.
pub struct LlmBridge {
    provider: Arc<dyn LlmProvider>,
}

impl LlmBridge {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Complete a conversation with the LLM.
    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        self.provider.complete(messages, tools).await
    }

    /// Stream a conversation with the LLM.
    pub async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmStream> {
        self.provider.complete_stream(messages, tools).await
    }

    /// Get provider name.
    pub fn name(&self) -> &str {
        self.provider.name()
    }

    /// Get max context length.
    pub fn max_context_length(&self) -> usize {
        self.provider.max_context_length()
    }

    /// Build a system message.
    pub fn system_message(text: &str) -> Message {
        Message {
            role: Role::System,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    /// Build a user message.
    pub fn user_message(text: &str) -> Message {
        Message::user(text)
    }

    /// Build an assistant message.
    pub fn assistant_message(text: &str) -> Message {
        Message::assistant(text)
    }
}
