//! Deterministic mock LLM provider for integration tests.
//!
//! Implements `fabric::LlmProvider` with pre-configured response sequences.
//! Records all messages and tool definitions sent to it for post-turn assertion.
//!
//! When a response sequence is exhausted, the provider panics — the test under-specified its
//! expected turn count.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use fabric::{
    ContentBlock, LlmProvider, LlmResponse, LlmStream, Message, StopReason, StreamChunk, ToolDefinition, Usage,
};
use futures::stream;

/// One turn from the mock model's perspective — a list of responses delivered in order.
/// When a response list is exhausted the provider moves to the next sequence.
pub struct MockTurnSequence {
    pub responses: Vec<MockTurnResponse>,
}

/// A single model response the mock provider can deliver.
pub enum MockTurnResponse {
    /// A single complete() response.
    Complete {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        usage: Usage,
    },
    /// A streamed response: chunks delivered sequentially.
    Stream {
        chunks: Vec<StreamChunk>,
    },
    /// Simulate a provider error (rate limit, overload, auth failure).
    Error {
        message: String,
    },
    /// Simulate a timeout — the provider never resolves.
    /// Tests should use `tokio::time::timeout` to bound this.
    Timeout,
}

/// Deterministic mock LLM provider for integration tests.
///
/// Pre-configured with sequences of responses. Each call to `complete()` or `complete_stream()`
/// advances an internal cursor through the current sequence. If no responses remain, the
/// provider panics to signal a test that did not specify enough turns.
pub struct MockLlmProvider {
    sequences: Mutex<Vec<MockTurnSequence>>,
    /// Index into `sequences` for the current active sequence.
    sequence_index: AtomicUsize,
    /// Index into the current sequence's responses.
    response_index: AtomicUsize,
    /// Record of the most recent complete() call arguments.
    last_messages: Mutex<Vec<Message>>,
    last_tools: Mutex<Vec<ToolDefinition>>,
    /// Full history of all complete() calls.
    message_history: Mutex<Vec<Vec<Message>>>,
    /// Number of times complete() was called.
    call_count: AtomicUsize,
}

impl MockLlmProvider {
    /// Create a provider from pre-configured turn sequences.
    pub fn new(sequences: Vec<MockTurnSequence>) -> Self {
        Self {
            sequences: Mutex::new(sequences),
            sequence_index: AtomicUsize::new(0),
            response_index: AtomicUsize::new(0),
            last_messages: Mutex::new(Vec::new()),
            last_tools: Mutex::new(Vec::new()),
            message_history: Mutex::new(Vec::new()),
            call_count: AtomicUsize::new(0),
        }
    }

    /// Single-turn text-only response. Convenience for simple tests.
    pub fn single_text_response(text: &str) -> Self {
        Self::new(vec![MockTurnSequence {
            responses: vec![MockTurnResponse::Complete {
                content: vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            }],
        }])
    }

    /// Provider that always errors on the first turn.
    pub fn always_error(message: &str) -> Self {
        Self::new(vec![MockTurnSequence {
            responses: vec![MockTurnResponse::Error {
                message: message.to_string(),
            }],
        }])
    }

    /// Return the messages sent in the most recent complete() call.
    pub fn last_messages(&self) -> Vec<Message> {
        self.last_messages.lock().unwrap().clone()
    }

    /// Return all message batches sent across all complete() calls.
    pub fn message_history(&self) -> Vec<Vec<Message>> {
        self.message_history.lock().unwrap().clone()
    }

    /// Return the tool definitions from the most recent call.
    pub fn last_tools(&self) -> Vec<ToolDefinition> {
        self.last_tools.lock().unwrap().clone()
    }

    /// Total number of complete() invocations.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    /// Assert that the Nth message batch contains a message matching the predicate.
    /// Panics if no match is found.
    pub fn assert_message<F>(&self, call_index: usize, predicate: F)
    where
        F: Fn(&Message) -> bool,
    {
        let history = self.message_history.lock().unwrap();
        let batch = &history[call_index];
        assert!(
            batch.iter().any(&predicate),
            "No message in call #{call_index} matched the predicate"
        );
    }

    fn next_response(&self) -> &MockTurnResponse {
        let seq_idx = self.sequence_index.load(Ordering::SeqCst);
        let resp_idx = self.response_index.fetch_add(1, Ordering::SeqCst);

        let sequences = self.sequences.lock().unwrap();
        let sequence = sequences.get(seq_idx).unwrap_or_else(|| {
            panic!(
                "MockLlmProvider: no sequence at index {seq_idx}. \
                 Test must configure enough turn sequences."
            )
        });

        sequence.responses.get(resp_idx).unwrap_or_else(|| {
            panic!(
                "MockLlmProvider: sequence {seq_idx} exhausted at response {resp_idx}. \
                 Expected more responses in this turn sequence."
            )
        })
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        // Record for assertion
        *self.last_messages.lock().unwrap() = messages.to_vec();
        *self.last_tools.lock().unwrap() = tools.to_vec();
        self.message_history.lock().unwrap().push(messages.to_vec());
        self.call_count.fetch_add(1, Ordering::SeqCst);

        match self.next_response() {
            MockTurnResponse::Complete {
                content,
                stop_reason,
                usage,
            } => Ok(LlmResponse {
                content: content.clone(),
                stop_reason: *stop_reason,
                usage: usage.clone(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            }),
            MockTurnResponse::Stream { chunks } => {
                // For complete(), concatenate text chunks and collect tool calls into content
                let mut content = Vec::new();
                let mut stop_reason = StopReason::EndTurn;
                let mut usage = Usage::default();
                let mut text_buf = String::new();
                let mut thinking_buf = String::new();

                for chunk in chunks {
                    match chunk {
                        StreamChunk::TextDelta { text } => text_buf.push_str(&text),
                        StreamChunk::ThinkingDelta { text } => thinking_buf.push_str(&text),
                        StreamChunk::Done { stop_reason: sr } => stop_reason = *sr,
                        StreamChunk::Usage {
                            input_tokens,
                            output_tokens,
                        } => {
                            usage = Usage {
                                input_tokens,
                                output_tokens,
                            };
                        }
                        StreamChunk::ToolUseStart { .. }
                        | StreamChunk::ToolUseDelta { .. }
                        | StreamChunk::ToolUseComplete { .. } => {
                            // Tool use chunks are collected in complete_stream path;
                            // for complete() we just note them
                        }
                    }
                }

                if !thinking_buf.is_empty() {
                    content.push(ContentBlock::Thinking {
                        text: thinking_buf,
                        signature: None,
                    });
                }
                if !text_buf.is_empty() {
                    content.push(ContentBlock::Text { text: text_buf });
                }

                Ok(LlmResponse {
                    content,
                    stop_reason,
                    usage,
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
            MockTurnResponse::Error { message } => {
                anyhow::bail!("mock provider error: {message}")
            }
            MockTurnResponse::Timeout => {
                // Simulate timeout by never resolving (test should use tokio::time::timeout)
                std::future::pending::<()>().await;
                unreachable!()
            }
        }
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        // Record for assertion
        *self.last_messages.lock().unwrap() = messages.to_vec();
        *self.last_tools.lock().unwrap() = tools.to_vec();
        self.message_history.lock().unwrap().push(messages.to_vec());
        self.call_count.fetch_add(1, Ordering::SeqCst);

        match self.next_response() {
            MockTurnResponse::Stream { chunks } => {
                let owned_chunks: Vec<anyhow::Result<StreamChunk>> =
                    chunks.into_iter().map(Ok).collect();
                Ok(Box::pin(stream::iter(owned_chunks)))
            }
            MockTurnResponse::Complete {
                content,
                stop_reason,
                usage,
            } => {
                let mut chunks: Vec<anyhow::Result<StreamChunk>> = Vec::new();
                for block in content {
                    match block {
                        ContentBlock::Text { text } => {
                            chunks.push(Ok(StreamChunk::TextDelta {
                                text: text.clone(),
                            }));
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            chunks.push(Ok(StreamChunk::ToolUseStart {
                                id: id.clone(),
                                name: name.clone(),
                            }));
                            chunks.push(Ok(StreamChunk::ToolUseDelta {
                                id: id.clone(),
                                delta: input.to_string(),
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
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                }));
                chunks.push(Ok(StreamChunk::Done {
                    stop_reason: stop_reason.clone(),
                }));
                Ok(Box::pin(stream::iter(chunks)))
            }
            MockTurnResponse::Error { message } => {
                anyhow::bail!("mock provider error: {message}")
            }
            MockTurnResponse::Timeout => {
                std::future::pending::<()>().await;
                unreachable!()
            }
        }
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn max_context_length(&self) -> usize {
        200_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_text_response_returns_preconfigured_text() {
        let provider = MockLlmProvider::single_text_response("hello world");
        let resp = provider.complete(&[Message::user("test")], &[]).await.unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert!(resp
            .content
            .iter()
            .any(|c| matches!(c, ContentBlock::Text { text } if text == "hello world")));
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn records_messages_and_tools_for_assertion() {
        let provider = MockLlmProvider::single_text_response("ok");
        let tools = vec![ToolDefinition {
            name: "bash".into(),
            description: "run command".into(),
            input_schema: serde_json::json!({}),
        }];
        provider
            .complete(&[Message::user("hi")], &tools)
            .await
            .unwrap();

        assert_eq!(provider.last_messages().len(), 1);
        assert_eq!(provider.last_tools().len(), 1);
        assert_eq!(provider.last_tools()[0].name, "bash");
    }

    #[tokio::test]
    #[should_panic(expected = "exhausted")]
    async fn panics_when_sequence_exhausted() {
        let provider = MockLlmProvider::single_text_response("only one");

        // First call succeeds.
        provider.complete(&[Message::user("1")], &[]).await.unwrap();

        // Second call panics because sequence has only 1 response.
        provider.complete(&[Message::user("2")], &[]).await.unwrap();
    }

    #[tokio::test]
    async fn error_response_returns_error() {
        let provider = MockLlmProvider::always_error("rate limited");
        let err = provider.complete(&[Message::user("hi")], &[]).await.unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[tokio::test]
    async fn complete_stream_from_stream_chunks() {
        let provider = MockLlmProvider::new(vec![MockTurnSequence {
            responses: vec![MockTurnResponse::Stream {
                chunks: vec![
                    StreamChunk::TextDelta {
                        text: "hello".into(),
                    },
                    StreamChunk::Done {
                        stop_reason: StopReason::EndTurn,
                    },
                ],
            }],
        }]);

        use futures::StreamExt;
        let mut stream = provider
            .complete_stream(&[Message::user("hi")], &[])
            .await
            .unwrap();
        let mut texts = Vec::new();
        while let Some(chunk) = stream.next().await {
            if let Ok(StreamChunk::TextDelta { text }) = chunk {
                texts.push(text);
            }
        }
        assert_eq!(texts, vec!["hello"]);
    }

    #[tokio::test]
    async fn complete_stream_from_complete_response() {
        let provider = MockLlmProvider::single_text_response("stream sim");
        use futures::StreamExt;
        let mut stream = provider
            .complete_stream(&[Message::user("hi")], &[])
            .await
            .unwrap();
        let mut texts = Vec::new();
        while let Some(chunk) = stream.next().await {
            if let Ok(StreamChunk::TextDelta { text }) = chunk {
                texts.push(text);
            }
        }
        assert_eq!(texts, vec!["stream sim"]);
    }
}
