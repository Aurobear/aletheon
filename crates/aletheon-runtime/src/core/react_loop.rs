use crate::core::config::RuntimeConfig;
use crate::r#impl::memory::compressor::AdvancedCompressor;
use aletheon_abi::tool::ConcurrencyClass;

/// Marker injected into user messages when plan mode is active.
/// Shared between `ReActLoop` and `Controller` to keep them in sync.
pub const PLAN_MODE_MARKER: &str = "[PLAN MODE ACTIVE]";
use aletheon_abi::body::Action;
use aletheon_abi::message::{ContentBlock, Message, Role};
use aletheon_abi::self_field::{Intent, IntentSource};
use aletheon_abi::ToolDefinition;

/// Metrics collected during a single ReAct turn.
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
}

/// Maximum number of tools to execute in a single parallel batch.
const MAX_PARALLEL_TOOLS: usize = 8;

/// A batch of tool calls classified for parallel or serial execution.
pub enum ToolBatch {
    /// Read-only tools safe to execute concurrently.
    Parallel(Vec<(String, String, serde_json::Value)>),
    /// Side-effect tools that must be executed sequentially.
    Serial(Vec<(String, String, serde_json::Value)>),
}

/// Classify a tool by name into its concurrency class.
fn classify_tool(tool_name: &str) -> ConcurrencyClass {
    match tool_name {
        "read_file" | "glob" | "grep" | "file_read" | "system_status"
        | "process_list" | "memory_search" | "ls" | "web_fetch" | "web_search" => {
            ConcurrencyClass::ReadOnly
        }
        _ => ConcurrencyClass::SideEffect,
    }
}

/// Partition a list of tool calls into parallel and serial batches.
///
/// Contiguous read-only tools are grouped into a single `Parallel` batch
/// (up to `MAX_PARALLEL_TOOLS`). Each side-effect tool gets its own `Serial`
/// batch. A switch from read-only to side-effect (or vice versa) flushes the
/// current batch.
pub fn partition_tool_calls(
    calls: &[(String, String, serde_json::Value)],
) -> Vec<ToolBatch> {
    if calls.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut current_readonly: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut current_serial: Vec<(String, String, serde_json::Value)> = Vec::new();

    let flush_readonly = |batches: &mut Vec<ToolBatch>,
                          buf: &mut Vec<(String, String, serde_json::Value)>| {
        if !buf.is_empty() {
            batches.push(ToolBatch::Parallel(std::mem::take(buf)));
        }
    };

    let flush_serial = |batches: &mut Vec<ToolBatch>,
                        buf: &mut Vec<(String, String, serde_json::Value)>| {
        if !buf.is_empty() {
            batches.push(ToolBatch::Serial(std::mem::take(buf)));
        }
    };

    for call in calls {
        let class = classify_tool(&call.1);
        match class {
            ConcurrencyClass::ReadOnly => {
                flush_serial(&mut batches, &mut current_serial);
                current_readonly.push(call.clone());
                if current_readonly.len() >= MAX_PARALLEL_TOOLS {
                    flush_readonly(&mut batches, &mut current_readonly);
                }
            }
            _ => {
                flush_readonly(&mut batches, &mut current_readonly);
                current_serial.push(call.clone());
                // Each side-effect tool gets its own serial batch.
                flush_serial(&mut batches, &mut current_serial);
            }
        }
    }

    flush_readonly(&mut batches, &mut current_readonly);
    flush_serial(&mut batches, &mut current_serial);

    batches
}
use aletheon_brain::r#impl::llm::provider::{LlmProvider, StopReason, StreamChunk};
use std::future::Future;
use tracing::{debug, warn};

use crate::core::event_sink::{Event, EventSink, ToolResultEvent};

/// The ReAct (Reason + Act) iteration loop
/// This is the core cognitive cycle extracted from Engine::run_turn()
pub struct ReActLoop {
    config: RuntimeConfig,
    iteration: usize,
    messages: Vec<Message>,
    compressor: AdvancedCompressor,
    /// Immutable system prompt — never changes after construction.
    system_prompt: String,
    /// Plan mode flag — injected into user message, NOT system prompt.
    plan_mode: bool,
    /// Pending memory updates — drained into user message each turn.
    pending_memory: Vec<String>,
}

impl ReActLoop {
    pub fn new(config: RuntimeConfig) -> Self {
        let compressor =
            AdvancedCompressor::new(config.tail_token_budget, config.target_summary_chars);
        Self {
            config,
            iteration: 0,
            messages: Vec::new(),
            compressor,
            system_prompt: String::new(),
            plan_mode: false,
            pending_memory: Vec::new(),
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

    /// Reset iteration counter for a new turn.
    /// Clears mutable state (messages, pending_memory) but preserves
    /// plan_mode and system_prompt (user choice / immutable).
    pub fn reset(&mut self) {
        self.iteration = 0;
        self.messages.clear();
        self.pending_memory.clear();
        // Note: plan_mode persists across resets (user choice)
        // Note: system_prompt never resets (immutable after construction)
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

    /// Set the system prompt (called once at construction or re-initialization).
    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    /// Get the immutable system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Enable/disable plan mode. Injected into user message, NOT system prompt.
    pub fn set_plan_mode(&mut self, enabled: bool) {
        self.plan_mode = enabled;
    }

    /// Queue a memory update for the next user message.
    pub fn queue_memory_update(&mut self, update: String) {
        self.pending_memory.push(update);
    }

    /// Compose user message with mid-session injections.
    /// Changes go here, NOT into system prompt, to preserve cache stability.
    pub fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        if self.plan_mode {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        if !self.pending_memory.is_empty() {
            let updates = self
                .pending_memory
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        parts.push(input.to_string());
        parts.join("\n\n")
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
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider + ?Sized,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        let start = std::time::Instant::now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;

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
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
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
                tool_calls_made += 1;
                if is_error {
                    tool_errors += 1;
                    warn!(tool = name.as_str(), "tool returned error");
                }
                self.messages
                    .push(Message::tool_result(id, &content, is_error));
            }

            // A2: proactive compaction after pushing tool results
            if self.config.compaction_enabled {
                let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
            }
        }

        warn!(
            max = self.config.max_iterations,
            "ReActLoop hit max_iterations"
        );
        let final_text = self
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations));
        let metrics = TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((final_text, metrics))
    }

    /// Streaming variant of `run()`. Uses `llm.complete_stream()` instead of
    /// `llm.complete()` and emits granular events through `event_sink`.
    pub async fn run_streaming<L, F, Fut>(
        &mut self,
        user_input: &str,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
        event_sink: &dyn EventSink,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider + ?Sized,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        use futures::StreamExt;

        let start = std::time::Instant::now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;

        self.messages.push(Message::user(user_input));
        event_sink.emit(Event::TurnStarted);

        while self.should_continue() {
            self.advance();

            // Use streaming instead of complete()
            let mut stream = match llm.complete_stream(&self.messages, tool_defs).await {
                Ok(s) => s,
                Err(e) if is_context_overflow(&e) => {
                    warn!("Context overflow detected, forcing compaction: {e}");
                    self.compressor.maybe_compact(&mut self.messages, llm).await?;
                    llm.complete_stream(&self.messages, tool_defs).await?
                }
                Err(e) => return Err(e),
            };

            let mut text_parts = Vec::new();
            let mut current_text = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;

            while let Some(chunk) = stream.next().await {
                match chunk? {
                    StreamChunk::TextDelta { text } => {
                        current_text.push_str(&text);
                        event_sink.emit(Event::TextDelta { delta: text });
                    }
                    StreamChunk::ToolUseStart { id, name } => {
                        // Flush any pending text
                        if !current_text.is_empty() {
                            text_parts.push(current_text.clone());
                            current_text.clear();
                        }
                        event_sink.emit(Event::ToolCallStart {
                            name: name.clone(),
                            call_id: id.clone(),
                        });
                        tool_calls.push((id, name, serde_json::Value::Null));
                    }
                    StreamChunk::ToolUseDelta { id: _, delta: _ } => {
                        // Accumulated in ToolUseComplete
                    }
                    StreamChunk::ToolUseComplete { id, input } => {
                        // Update tool_calls with correct input
                        if let Some(tc) = tool_calls.iter_mut().find(|(tid, _, _)| *tid == id) {
                            tc.2 = input;
                        }
                    }
                    StreamChunk::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        event_sink.emit(Event::Usage {
                            tokens_in: input_tokens,
                            tokens_out: output_tokens,
                            cache_hit_tokens: 0,
                            cache_miss_tokens: 0,
                        });
                    }
                    StreamChunk::Done { stop_reason: sr } => {
                        stop_reason = sr;
                        break;
                    }
                }
            }

            // Flush remaining text
            if !current_text.is_empty() {
                text_parts.push(current_text);
            }

            // No tool calls -> turn complete
            if tool_calls.is_empty() || matches!(stop_reason, StopReason::EndTurn) {
                let final_text = text_parts.join("\n");
                self.messages.push(Message::assistant(&final_text));
                event_sink.emit(Event::TurnDone {
                    result: Ok(final_text.clone()),
                });
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
            }

            // Has tool calls -> execute them
            let content_blocks: Vec<ContentBlock> = tool_calls
                .iter()
                .map(|(id, name, input)| ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                })
                .collect();

            self.messages.push(Message {
                role: Role::Assistant,
                content: content_blocks,
            });

            for (id, name, input) in &tool_calls {
                debug!(tool = name.as_str(), "ReActLoop streaming: executing tool");
                event_sink.emit(Event::ToolDispatch {
                    name: name.clone(),
                    args: input.clone(),
                });

                let (content, is_error) = execute_tool(id, name, input).await;

                event_sink.emit(Event::ToolResult {
                    name: name.clone(),
                    result: ToolResultEvent {
                        content: content.clone(),
                        is_error,
                        execution_time_ms: 0,
                    },
                });

                tool_calls_made += 1;
                if is_error {
                    tool_errors += 1;
                    warn!(tool = name.as_str(), "tool returned error");
                }
                self.messages
                    .push(Message::tool_result(id, &content, is_error));
            }

            if self.config.compaction_enabled {
                let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
            }
        }

        warn!(
            max = self.config.max_iterations,
            "ReActLoop streaming hit max_iterations"
        );
        let fallback = self
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations));
        event_sink.emit(Event::TurnDone {
            result: Ok(fallback.clone()),
        });
        let metrics = TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((fallback, metrics))
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

        let (out, metrics) = lp
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
        assert_eq!(metrics.tool_calls_made, 1);
        assert_eq!(metrics.tool_errors, 0);
        assert!(metrics.completed_normally);
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

        let (out, _metrics) = lp
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

        let (out, _metrics) = lp
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
        assert_eq!(
            *llm.calls.lock().unwrap(),
            3,
            "LLM called 3 times: seed, error, success"
        );
    }

    // --- Task 1.1: compose_user_message tests ---

    #[test]
    fn compose_user_message_plain_input() {
        let cfg = RuntimeConfig::default();
        let lp = ReActLoop::new(cfg);
        let composed = lp.compose_user_message("hello");
        assert_eq!(composed, "hello");
    }

    #[test]
    fn compose_user_message_with_plan_mode() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        let composed = lp.compose_user_message("hello");
        assert!(composed.contains("[PLAN MODE ACTIVE]"));
        assert!(composed.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_memory_updates() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.queue_memory_update("user prefers dark mode".to_string());
        let composed = lp.compose_user_message("hello");
        assert!(composed.contains("<memory-update>"));
        assert!(composed.contains("user prefers dark mode"));
    }

    #[test]
    fn compose_user_message_plan_and_memory() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        lp.queue_memory_update("fact 1".to_string());
        let composed = lp.compose_user_message("do something");
        assert!(composed.contains("[PLAN MODE ACTIVE]"));
        assert!(composed.contains("<memory-update>"));
        assert!(composed.contains("do something"));
    }

    #[test]
    fn system_prompt_immutable_after_construction() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        let p1 = lp.system_prompt().to_string();
        lp.set_plan_mode(true);
        lp.queue_memory_update("new fact".to_string());
        let p2 = lp.system_prompt().to_string();
        assert_eq!(p1, p2, "system prompt must not change");
    }

    #[test]
    fn reset_clears_pending_memory_but_not_plan_mode() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        lp.queue_memory_update("fact".to_string());

        lp.reset();

        // plan_mode persists across resets
        let composed = lp.compose_user_message("hi");
        assert!(
            composed.contains("[PLAN MODE ACTIVE]"),
            "plan_mode should persist after reset"
        );
        // pending_memory was cleared
        assert!(
            !composed.contains("<memory-update>"),
            "pending_memory should be cleared after reset"
        );
    }

    #[test]
    fn set_system_prompt_works() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        assert_eq!(lp.system_prompt(), "");
        lp.set_system_prompt("You are helpful.".to_string());
        assert_eq!(lp.system_prompt(), "You are helpful.");
    }

    #[test]
    fn compose_multiple_memory_updates() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.queue_memory_update("fact a".to_string());
        lp.queue_memory_update("fact b".to_string());
        let composed = lp.compose_user_message("hello");
        assert!(composed.contains("- fact a"));
        assert!(composed.contains("- fact b"));
    }

    // --- Task 3: partition_tool_calls tests ---

    #[test]
    fn partition_read_only_batch() {
        let calls = vec![
            ("id1".into(), "read_file".into(), serde_json::json!({})),
            ("id2".into(), "glob".into(), serde_json::json!({})),
            ("id3".into(), "grep".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        assert_eq!(batches.len(), 1);
        match &batches[0] {
            ToolBatch::Parallel(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected Parallel batch"),
        }
    }

    #[test]
    fn partition_writer_serial() {
        let calls = vec![
            ("id1".into(), "write_file".into(), serde_json::json!({})),
            ("id2".into(), "bash".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        assert_eq!(batches.len(), 2);
        for batch in &batches {
            match batch {
                ToolBatch::Serial(v) => assert_eq!(v.len(), 1),
                _ => panic!("expected Serial batch"),
            }
        }
    }

    #[test]
    fn partition_mixed() {
        // read, read, write, read → Parallel(2), Serial(1), Parallel(1)
        let calls = vec![
            ("id1".into(), "read_file".into(), serde_json::json!({})),
            ("id2".into(), "grep".into(), serde_json::json!({})),
            ("id3".into(), "write_file".into(), serde_json::json!({})),
            ("id4".into(), "ls".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        assert_eq!(batches.len(), 3);

        match &batches[0] {
            ToolBatch::Parallel(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected Parallel batch with 2 items"),
        }
        match &batches[1] {
            ToolBatch::Serial(v) => assert_eq!(v.len(), 1),
            _ => panic!("expected Serial batch with 1 item"),
        }
        match &batches[2] {
            ToolBatch::Parallel(v) => assert_eq!(v.len(), 1),
            _ => panic!("expected Parallel batch with 1 item"),
        }
    }

    #[test]
    fn partition_empty() {
        let calls: Vec<(String, String, serde_json::Value)> = vec![];
        let batches = partition_tool_calls(&calls);
        assert!(batches.is_empty());
    }
}
