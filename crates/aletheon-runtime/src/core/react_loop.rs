use crate::core::config::RuntimeConfig;
use crate::r#impl::memory::compressor::AdvancedCompressor;
use aletheon_abi::body::Action;
use aletheon_abi::message::{ContentBlock, Message, Role};
use aletheon_abi::self_field::{Intent, IntentSource};
use aletheon_abi::ToolDefinition;
use aletheon_brain::r#impl::llm::provider::{LlmProvider, StopReason};
use std::future::Future;
use tracing::{debug, warn};

/// The ReAct (Reason + Act) iteration loop
/// This is the core cognitive cycle extracted from Engine::run_turn()
pub struct ReActLoop {
    config: RuntimeConfig,
    iteration: usize,
    messages: Vec<Message>,
    compressor: AdvancedCompressor,
}

impl ReActLoop {
    pub fn new(config: RuntimeConfig) -> Self {
        let compressor = AdvancedCompressor::new(
            config.tail_token_budget,
            config.target_summary_chars,
        );
        Self {
            config,
            iteration: 0,
            messages: Vec::new(),
            compressor,
        }
    }

    /// Number of messages in the conversation buffer.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Current iteration number
    pub fn iteration(&self) -> usize {
        self.iteration
    }

    /// Reset iteration counter for a new turn
    pub fn reset(&mut self) {
        self.iteration = 0;
        self.messages.clear();
    }

    /// Seed the message buffer with pre-existing messages (e.g., from session restore).
    pub fn seed_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Check if we've hit the max iterations
    pub fn should_continue(&self) -> bool {
        self.iteration < self.config.max_iterations
    }

    /// Increment iteration counter
    pub fn advance(&mut self) {
        self.iteration += 1;
    }

    /// Build an Intent from user input
    pub fn build_intent(&self, input: &str) -> Intent {
        Intent {
            action: "user_request".to_string(),
            parameters: serde_json::json!({"input": input}),
            source: IntentSource::User,
            description: input.to_string(),
        }
    }

    /// Build an Action from a plan step
    pub fn step_to_action(&self, tool_name: &str, params: serde_json::Value) -> Action {
        Action {
            name: tool_name.to_string(),
            parameters: params,
            requires_sandbox: false,
            timeout: None,
        }
    }

    /// Max iterations
    pub fn max_iterations(&self) -> usize {
        self.config.max_iterations
    }

    /// Run the interleaved ReAct loop: call the LLM with tools, execute any
    /// requested tools via `execute_tool`, feed results back, and repeat until
    /// the LLM stops requesting tools or `max_iterations` is reached.
    pub async fn run<L, F, Fut>(
        &mut self,
        user_input: &str,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
    ) -> anyhow::Result<String>
    where
        L: LlmProvider + ?Sized,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        self.messages.push(Message::user(user_input));

        while self.should_continue() {
            self.advance();
            let response = match llm.complete(&self.messages, tool_defs).await {
                Ok(r) => r,
                Err(e) if is_context_overflow(&e) => {
                    // A3: reactive compaction on context overflow
                    warn!("Context overflow detected, forcing compaction: {e}");
                    self.compressor
                        .maybe_compact(&mut self.messages, llm)
                        .await?;
                    llm.complete(&self.messages, tool_defs).await?
                }
                Err(e) => return Err(e),
            };

            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            if tool_calls.is_empty() || matches!(response.stop_reason, StopReason::EndTurn) {
                let final_text = text_parts.join("\n");
                self.messages.push(Message::assistant(&final_text));
                return Ok(final_text);
            }

            // Record the assistant turn (text + tool_use blocks) verbatim.
            self.messages.push(Message {
                role: Role::Assistant,
                content: response.content.clone(),
            });

            // Execute each requested tool and feed results back.
            for (id, name, input) in &tool_calls {
                debug!(tool = name.as_str(), "ReActLoop executing tool");
                let (content, is_error) = execute_tool(id, name, input).await;
                if is_error {
                    warn!(tool = name.as_str(), "tool returned error");
                }
                self.messages
                    .push(Message::tool_result(id, &content, is_error));
            }

            // A2: proactive compaction after pushing tool results
            if self.config.compaction_enabled {
                let _ = self
                    .compressor
                    .maybe_compact(&mut self.messages, llm)
                    .await;
            }
        }

        warn!(
            max = self.config.max_iterations,
            "ReActLoop hit max_iterations"
        );
        Ok(self
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations)))
    }
}

/// Check if an error indicates a context window overflow.
fn is_context_overflow(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("context")
        || msg.contains("too long")
        || msg.contains("maximum context")
        || msg.contains("prompt is too long")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::message::{ContentBlock, Message};
    use aletheon_abi::ToolDefinition;
    use aletheon_brain::r#impl::llm::provider::{
        LlmProvider, LlmResponse, LlmStream, StopReason, Usage,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct ScriptedLlm {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "echo_tool".into(),
                        input: serde_json::json!({"text": "hi"}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "done: hi".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "scripted"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn interleaved_loop_executes_tool_then_finishes() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: false,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = ScriptedLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let executed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executed2 = executed.clone();

        let out = lp
            .run(
                "make hi",
                &llm,
                &tool_defs,
                |_id: &str, name: &str, _input: &serde_json::Value| {
                    let executed = executed2.clone();
                    let name = name.to_string();
                    async move {
                        executed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        (format!("ran {}", name), false)
                    }
                },
            )
            .await
            .unwrap();

        assert_eq!(
            executed.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "tool ran exactly once"
        );
        assert!(out.contains("done"), "final text returned: {out}");
    }

    /// An LLM that always returns tool-use, then ends with text on the Nth call.
    struct BigToolLlm {
        calls: Mutex<usize>,
        tool_until: usize,
    }

    #[async_trait]
    impl LlmProvider for BigToolLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n <= self.tool_until {
                let big_text = "x".repeat(10_000);
                Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: format!("call_{}", n),
                        name: "big_tool".into(),
                        input: serde_json::json!({"data": big_text}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "big_tool"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn loop_compacts_when_over_budget() {
        // tail_token_budget=5000 so the tail can hold ~3 messages (~7500 tokens).
        // Messages from tool interactions are ~2500 tokens each, so compaction
        // triggers when total > 10000 tokens (about 4 messages).
        let cfg = RuntimeConfig {
            max_iterations: 30,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 5_000,
            target_summary_chars: 200,
            context_window_tokens: 128_000,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = BigToolLlm {
            calls: Mutex::new(0),
            tool_until: 20,
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let out = lp
            .run(
                "do many big things",
                &llm,
                &tool_defs,
                |_id: &str, name: &str, _input: &serde_json::Value| {
                    let name = name.to_string();
                    let big = "y".repeat(10_000);
                    async move { (format!("result_{}: {}", name, big), false) }
                },
            )
            .await
            .unwrap();

        assert!(out.contains("done"), "final text returned: {out}");
        // Without compaction we'd have 1 + 20*2 + 1 = 42 messages.
        // With compaction, the count should be significantly bounded.
        let count = lp.message_count();
        assert!(
            count < 20,
            "Expected message count bounded by compaction, got {count}"
        );
    }

    /// An LLM that errors "prompt is too long" on the first call, then succeeds.
    struct ErrorThenOkLlm {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for ErrorThenOkLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Err(anyhow::anyhow!("prompt is too long: 200000 tokens"))
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "recovered after compaction".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "error_then_ok"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn reactive_compaction_on_context_overflow() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 100,
            target_summary_chars: 200,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);

        // Seed the loop with enough messages so compaction has work to do.
        // ~8000 chars = ~2000 tokens, enough to exceed tail_token_budget * 2 = 200.
        let big_msg = Message::user("x".repeat(8_000));
        // We push directly onto the internal messages vec via reset + manual insert.
        // Since messages is private, we use the run path: the user_input + the big content
        // is already inside messages from previous iterations.
        // Actually, let's just pre-populate by using a preparatory run.
        // Simpler: we'll push messages via the public interface indirectly.
        // The easiest approach: make the first LLM produce a huge tool result that seeds the buffer.

        // Instead, let's use a two-phase LLM:
        // Phase 1: big tool result (seeds messages)
        // Phase 2: error "prompt is too long"
        // Phase 3: success

        struct SeedThenErrorThenOkLlm {
            calls: Mutex<usize>,
        }

        #[async_trait]
        impl LlmProvider for SeedThenErrorThenOkLlm {
            async fn complete(
                &self,
                _m: &[Message],
                _t: &[ToolDefinition],
            ) -> anyhow::Result<LlmResponse> {
                let mut n = self.calls.lock().unwrap();
                *n += 1;
                match *n {
                    1 => {
                        // Return huge tool use to seed the buffer
                        let big = "z".repeat(10_000);
                        Ok(LlmResponse {
                            content: vec![ContentBlock::ToolUse {
                                id: "seed_1".into(),
                                name: "seed_tool".into(),
                                input: serde_json::json!({"d": big}),
                            }],
                            stop_reason: StopReason::ToolUse,
                            usage: Usage::default(),
                            cache_hit_tokens: 0,
                            cache_miss_tokens: 0,
                        })
                    }
                    2 => {
                        // Error: context overflow
                        Err(anyhow::anyhow!("prompt is too long: 200000 tokens"))
                    }
                    _ => {
                        // Success after compaction
                        Ok(LlmResponse {
                            content: vec![ContentBlock::Text {
                                text: "recovered".into(),
                            }],
                            stop_reason: StopReason::EndTurn,
                            usage: Usage::default(),
                            cache_hit_tokens: 0,
                            cache_miss_tokens: 0,
                        })
                    }
                }
            }

            async fn complete_stream(
                &self,
                _m: &[Message],
                _t: &[ToolDefinition],
            ) -> anyhow::Result<LlmStream> {
                unimplemented!("not used in test")
            }

            fn name(&self) -> &str {
                "seed_error_ok"
            }

            fn max_context_length(&self) -> usize {
                100_000
            }
        }

        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 100,
            target_summary_chars: 200,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = SeedThenErrorThenOkLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let out = lp
            .run(
                "test overflow recovery",
                &llm,
                &tool_defs,
                |_id: &str, _name: &str, _input: &serde_json::Value| {
                    let big = "y".repeat(10_000);
                    async move { (format!("result: {}", big), false) }
                },
            )
            .await
            .unwrap();

        assert_eq!(out, "recovered");
        assert_eq!(*llm.calls.lock().unwrap(), 3, "LLM called 3 times: seed, error, success");
    }
}
