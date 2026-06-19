use crate::core::config::RuntimeConfig;
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
}

impl ReActLoop {
    pub fn new(config: RuntimeConfig) -> Self {
        Self {
            config,
            iteration: 0,
            messages: Vec::new(),
        }
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
            let response = llm.complete(&self.messages, tool_defs).await?;

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
}
