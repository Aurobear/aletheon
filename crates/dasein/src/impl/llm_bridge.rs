//! Thin adapter for SelfField to call LLM without directly depending on aletheon-brain internals.
//!
//! Wraps an Arc<dyn LlmProvider> behind a purpose-based interface.

use base::message::Message;
use cognit::llm::provider::{LlmProvider, LlmResponse};
use anyhow::Result;
use std::sync::Arc;

pub struct LlmBridge {
    provider: Arc<dyn LlmProvider>,
}

impl LlmBridge {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn complete_for_purpose(
        &self,
        _purpose: &str,
        messages: &[Message],
    ) -> Result<LlmResponse> {
        self.provider.complete(messages, &[]).await
    }
}
