use super::{ReActLoop, TurnMetrics};

use crate::adapters::inference::provider::{LlmProvider, LlmResponse, LlmStream, StreamChunk};
use crate::harness::event_sink::{Event, EventSink};
use async_trait::async_trait;
use fabric::message::{ContentBlock, Message};
use fabric::ToolDefinition;
use std::future::Future;
use std::sync::Mutex;

/// Event adapter used by callers that need a terminal return value rather than
/// incremental UI events. It deliberately observes the same streaming engine as
/// interactive callers; it is not a second cognitive loop.
#[derive(Default)]
struct CollectingEventSink {
    terminal: Mutex<Option<Result<String, String>>>,
}

struct CompleteAsStream<'a, L>(&'a L);

#[async_trait]
impl<L: LlmProvider> LlmProvider for CompleteAsStream<'_, L> {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.0.complete(messages, tools).await
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let response = self.0.complete(messages, tools).await?;
        let mut chunks = Vec::new();
        for block in response.content {
            match block {
                ContentBlock::Text { text } => chunks.push(Ok(StreamChunk::TextDelta { text })),
                ContentBlock::Thinking { text, .. } => {
                    chunks.push(Ok(StreamChunk::ThinkingDelta { text }));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    chunks.push(Ok(StreamChunk::ToolUseStart {
                        id: id.clone(),
                        name,
                    }));
                    chunks.push(Ok(StreamChunk::ToolUseComplete { id, input }));
                }
                ContentBlock::ToolResult { .. }
                | ContentBlock::Image { .. }
                | ContentBlock::System { .. } => {}
            }
        }
        chunks.push(Ok(StreamChunk::Usage {
            usage: response.usage,
        }));
        chunks.push(Ok(StreamChunk::Done {
            stop_reason: response.stop_reason,
        }));
        Ok(Box::pin(futures::stream::iter(chunks)))
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn max_context_length(&self) -> usize {
        self.0.max_context_length()
    }
}

impl EventSink for CollectingEventSink {
    fn emit(&self, event: Event) {
        if let Event::TurnDone { result } = event {
            *self
                .terminal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        }
    }
}

impl CollectingEventSink {
    fn terminal(&self) -> Option<Result<String, String>> {
        self.terminal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ReActLoop {
    /// Non-streaming compatibility adapter over the authoritative event-driven
    /// loop. The provider may still stream internally; this caller simply
    /// collects the terminal event and returns the same outcome.
    pub async fn run<L, F, Fut>(
        &mut self,
        user_input: &str,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        let dasein_context = self
            .dasein_ctx_provider
            .as_ref()
            .and_then(|provider| provider());
        let user_message = if self.dasein_ctx_provider.is_some() {
            self.compose_user_message_with_dasein(user_input, dasein_context.as_deref())
        } else {
            self.compose_user_message(user_input)
        };
        self.pending_memory.clear();
        self.messages.push(Message::user(user_message));

        let sink = CollectingEventSink::default();
        let streaming_provider = CompleteAsStream(llm);
        let outcome = self
            .run_streaming(
                &streaming_provider,
                tool_defs,
                execute_tool,
                || async { Ok(Vec::new()) },
                &sink,
            )
            .await?;

        match sink.terminal() {
            Some(Ok(terminal)) if terminal == outcome.0 => Ok(outcome),
            Some(Ok(_)) => {
                anyhow::bail!("collecting adapter observed inconsistent terminal output")
            }
            Some(Err(error))
                if outcome.1.stop == fabric::TurnStop::Blocked && error == outcome.0 =>
            {
                Ok(outcome)
            }
            Some(Err(error)) => anyhow::bail!(error),
            None => anyhow::bail!("streaming cognitive loop ended without a terminal event"),
        }
    }
}
