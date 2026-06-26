use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;

use crate::r#impl::llm::{
    LlmProvider, LlmResponse, LlmStream, StopReason, StreamChunk, ToolDefinition, Usage,
};
use aletheon_abi::message::{ContentBlock, Message};

/// Mock LLM provider with canned responses.
///
/// Responses are returned in FIFO order. If the queue is empty,
/// `complete()` returns an error.
pub struct MockLlmProvider {
    name: String,
    max_context: usize,
    responses: Mutex<VecDeque<LlmResponse>>,
    /// Log of all messages received by `complete()`.
    pub call_log: Mutex<Vec<Vec<Message>>>,
}

impl MockLlmProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            max_context: 128_000,
            responses: Mutex::new(VecDeque::new()),
            call_log: Mutex::new(Vec::new()),
        }
    }

    /// Enqueue a canned text response.
    pub fn push_text_response(&self, text: impl Into<String>, stop: StopReason) {
        let mut q = self.responses.lock().unwrap();
        q.push_back(LlmResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: stop,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
            },
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        });
    }

    /// Enqueue a raw `LlmResponse`.
    pub fn push_response(&self, response: LlmResponse) {
        let mut q = self.responses.lock().unwrap();
        q.push_back(response);
    }

    /// Set the maximum context length for testing.
    pub fn with_max_context(mut self, max: usize) -> Self {
        self.max_context = max;
        self
    }

    /// Number of canned responses remaining.
    pub fn remaining(&self) -> usize {
        self.responses.lock().unwrap().len()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.call_log.lock().unwrap().push(messages.to_vec());

        let mut q = self.responses.lock().unwrap();
        q.pop_front()
            .ok_or_else(|| anyhow::anyhow!("MockLlmProvider: no more canned responses"))
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        self.call_log.lock().unwrap().push(messages.to_vec());

        let mut q = self.responses.lock().unwrap();
        let response = q
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("MockLlmProvider: no more canned responses"))?;

        // Build a simple stream from the response text
        let mut chunks: Vec<anyhow::Result<StreamChunk>> = Vec::new();
        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    chunks.push(Ok(StreamChunk::TextDelta { text: text.clone() }));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    chunks.push(Ok(StreamChunk::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    }));
                    chunks.push(Ok(StreamChunk::ToolUseComplete {
                        id: id.clone(),
                        input: input.clone(),
                    }));
                }
                _ => {}
            }
        }
        chunks.push(Ok(StreamChunk::Usage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        }));
        chunks.push(Ok(StreamChunk::Done {
            stop_reason: response.stop_reason.clone(),
        }));

        Ok(Box::pin(stream::iter(chunks)))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn max_context_length(&self) -> usize {
        self.max_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_llm_text_response() {
        let mock = MockLlmProvider::new("test");
        mock.push_text_response("Hello!", StopReason::EndTurn);

        let response = mock.complete(&[Message::user("Hi")], &[]).await.unwrap();
        match &response.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("Expected Text block"),
        }
        assert!(matches!(response.stop_reason, StopReason::EndTurn));
    }

    #[tokio::test]
    async fn test_mock_llm_fifo_order() {
        let mock = MockLlmProvider::new("test");
        mock.push_text_response("first", StopReason::EndTurn);
        mock.push_text_response("second", StopReason::EndTurn);

        let r1 = mock.complete(&[], &[]).await.unwrap();
        let r2 = mock.complete(&[], &[]).await.unwrap();

        match &r1.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "first"),
            _ => panic!(),
        }
        match &r2.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "second"),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn test_mock_llm_exhausted() {
        let mock = MockLlmProvider::new("test");
        let result = mock.complete(&[], &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_llm_call_log() {
        let mock = MockLlmProvider::new("test");
        mock.push_text_response("ok", StopReason::EndTurn);

        let msg = Message::user("question");
        mock.complete(&[msg.clone()], &[]).await.unwrap();

        let log = mock.call_log.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].len(), 1);
    }

    #[tokio::test]
    async fn test_mock_llm_stream() {
        let mock = MockLlmProvider::new("test");
        mock.push_text_response("streamed", StopReason::EndTurn);

        let mut stream = mock
            .complete_stream(&[Message::user("go")], &[])
            .await
            .unwrap();
        use futures::StreamExt;
        let first = stream.next().await.unwrap().unwrap();
        match first {
            StreamChunk::TextDelta { text } => assert_eq!(text, "streamed"),
            other => panic!("Expected TextDelta, got {:?}", other),
        }
    }
}
