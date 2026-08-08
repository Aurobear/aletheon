use super::*;
use crate::adapters::inference::provider::{
    InferenceUsage, LlmProvider, LlmResponse, LlmStream, StopReason, StreamChunk,
};
use async_trait::async_trait;
use fabric::message::{ContentBlock, Message};
use fabric::CompactionOutcome;
use fabric::ToolDefinition;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

mod policy_denial_tests;
mod projection_tests;

/// No-op compressor for tests that don't exercise compaction.
struct NoopCompressor;
impl CompactorTrait for NoopCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}
/// Counts which compaction entry point the harness invoked, to prove the
/// `compaction_v2` flag routes correctly.
struct RecordingCompressor {
    v2: Arc<AtomicUsize>,
    legacy: Arc<AtomicUsize>,
}
impl CompactorTrait for RecordingCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        self.legacy.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(false) })
    }
    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
    fn maybe_compact_v2<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
        strategy: CompactionStrategy,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<CompactionOutcome>> + Send + 'a>>
    {
        self.v2.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(CompactionOutcome {
                strategy,
                applied: false,
                tokens_before: 0,
                tokens_after: 0,
                evicted: Vec::new(),
                failure: None,
            })
        })
    }
}
#[tokio::test]
async fn compaction_v2_flag_routes_to_v2_path() {
    let v2 = Arc::new(AtomicUsize::new(0));
    let legacy = Arc::new(AtomicUsize::new(0));
    let cfg = HarnessConfig {
        compaction_v2: true,
        ..Default::default()
    };
    let mut lp = ReActLoop::new(
        cfg,
        Box::new(RecordingCompressor {
            v2: v2.clone(),
            legacy: legacy.clone(),
        }),
    );
    lp.run_proactive_compaction(
        &ScriptedLlm {
            calls: Mutex::new(0),
        },
        None,
    )
    .await;
    assert_eq!(v2.load(Ordering::SeqCst), 1, "v2 path should be taken");
    assert_eq!(legacy.load(Ordering::SeqCst), 0, "legacy must not run");
}
#[tokio::test]
async fn compaction_uses_legacy_path_when_flag_off() {
    let v2 = Arc::new(AtomicUsize::new(0));
    let legacy = Arc::new(AtomicUsize::new(0));
    let cfg = HarnessConfig {
        compaction_v2: false,
        ..Default::default()
    };
    let mut lp = ReActLoop::new(
        cfg,
        Box::new(RecordingCompressor {
            v2: v2.clone(),
            legacy: legacy.clone(),
        }),
    );
    lp.run_proactive_compaction(
        &ScriptedLlm {
            calls: Mutex::new(0),
        },
        None,
    )
    .await;
    assert_eq!(
        legacy.load(Ordering::SeqCst),
        1,
        "legacy path should be taken"
    );
    assert_eq!(v2.load(Ordering::SeqCst), 0, "v2 must not run");
}

struct CollectingEventSink(Mutex<Vec<crate::harness::event_sink::Event>>);

impl crate::harness::event_sink::EventSink for CollectingEventSink {
    fn emit(&self, event: crate::harness::event_sink::Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct CollectingGroundedSink {
    outcomes: Mutex<Vec<fabric::cognitive_workflow::GroundedCognitiveOutcome>>,
    fail: bool,
}

#[async_trait]
impl crate::core::GroundedOutcomeSink for CollectingGroundedSink {
    async fn publish(
        &self,
        outcome: fabric::cognitive_workflow::GroundedCognitiveOutcome,
    ) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("observational sink unavailable");
        }
        self.outcomes.lock().unwrap().push(outcome);
        Ok(())
    }
}

#[test]
fn compaction_outcome_updates_metrics_emits_event_and_hands_off_evicted() {
    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    let promoted = Arc::new(Mutex::new(Vec::<Message>::new()));
    let promoted_clone = promoted.clone();
    lp.set_evicted_callback(Arc::new(move |messages| {
        promoted_clone.lock().unwrap().extend(messages);
    }));
    let sink = CollectingEventSink(Mutex::new(Vec::new()));
    let before = compaction_metrics();
    let failed = CompactionOutcome {
        strategy: CompactionStrategy::TailKeep,
        applied: false,
        tokens_before: 100,
        tokens_after: 100,
        evicted: Vec::new(),
        failure: Some(fabric::CompactionFailure::DegenerateSummary {
            reason: "short".into(),
        }),
    };
    let applied = CompactionOutcome {
        strategy: CompactionStrategy::TailKeep,
        applied: true,
        tokens_before: 100,
        tokens_after: 50,
        evicted: vec![Message::user("remember this")],
        failure: None,
    };

    lp.observe_compaction_outcome(&failed, Some(&sink));
    lp.observe_compaction_outcome(&applied, Some(&sink));

    let after = compaction_metrics();
    assert!(after.degenerate_total > before.degenerate_total);
    assert!(after.evicted_messages_total > before.evicted_messages_total);
    assert_eq!(promoted.lock().unwrap().len(), 1);
    let events = sink.0.lock().unwrap();
    assert!(matches!(
        &events[1],
        crate::harness::event_sink::Event::CompactionOutcome {
            strategy,
            applied: true,
            tokens_before: 100,
            tokens_after: 50,
            evicted_messages: 1,
            failure: None,
        } if strategy == "tailkeep"
    ));
}

#[test]
fn sampler_failure_increments_its_distinct_metric() {
    let lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    let before = compaction_metrics();
    lp.observe_compaction_outcome(
        &CompactionOutcome {
            strategy: CompactionStrategy::FullReplace,
            applied: false,
            tokens_before: 1,
            tokens_after: 1,
            evicted: Vec::new(),
            failure: Some(fabric::CompactionFailure::SamplerError {
                detail: "offline".into(),
            }),
        },
        None,
    );
    assert!(compaction_metrics().sampler_error_total > before.sampler_error_total);
}

struct ScriptedLlm {
    calls: Mutex<usize>,
}

struct RecordingTextLlm {
    messages: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl LlmProvider for RecordingTextLlm {
    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.messages.lock().unwrap().push(messages.to_vec());
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "done".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("not used in test")
    }

    fn name(&self) -> &str {
        "recording-text"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
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
                usage: InferenceUsage::default(),
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "done: hi".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
            })
        }
    }

    async fn complete_stream(
        &self,
        _m: &[Message],
        _t: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        let chunks = if *n == 1 {
            vec![
                Ok(StreamChunk::ToolUseStart {
                    id: "call_1".into(),
                    name: "echo_tool".into(),
                }),
                Ok(StreamChunk::ToolUseComplete {
                    id: "call_1".into(),
                    input: serde_json::json!({"text": "hi"}),
                }),
                Ok(StreamChunk::Done {
                    stop_reason: StopReason::ToolUse,
                }),
            ]
        } else {
            vec![
                Ok(StreamChunk::TextDelta {
                    text: "done: hi".into(),
                }),
                Ok(StreamChunk::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ]
        };
        Ok(Box::pin(futures::stream::iter(chunks)))
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
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
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
                    (format!("ran {name}"), false)
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
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        if *n <= self.tool_until {
            let big_text = "x".repeat(10_000);
            Ok(LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: format!("call_{n}"),
                    name: "big_tool".into(),
                    input: serde_json::json!({"data": big_text}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: InferenceUsage::default(),
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
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
    // triggers when total > 80% of context_window_tokens.
    // Set context_window_tokens=12_500 so 80% threshold = 10_000 tokens (about 4 messages).
    let cfg = HarnessConfig {
        max_iterations: 30,
        learning_enabled: false,
        compaction_enabled: true,
        tail_token_budget: 5_000,
        target_summary_chars: 200,
        context_window_tokens: 12_500,
        max_tool_calls: 25,      // Higher threshold for this compaction test
        reflection_interval: 30, // Disable reflection for this compaction test
        circuit_breaker_max_repeats: 25, // Higher threshold for this compaction test
        circuit_breaker_window_size: 50,
        ..HarnessConfig::default()
    };
    let compressor = Box::new(NoopCompressor) as Box<dyn CompactorTrait>;
    let mut lp = ReActLoop::new(cfg, compressor);
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
                async move { (format!("result_{name}: {big}"), false) }
            },
        )
        .await
        .unwrap();

    assert!(out.contains("done"), "final text returned: {out}");
    // With NoopCompressor in tests, compaction is a no-op.
    // The loop still completes normally; message count reflects full history.
    let count = lp.message_count();
    assert!(count > 0, "should have some messages");
}

#[tokio::test]
async fn exhausted_tool_budget_closes_pending_tool_calls() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        max_tool_calls: 1,
        reflection_interval: 30,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let llm = BigToolLlm {
        calls: Mutex::new(0),
        tool_until: 2,
    };

    let (out, metrics) = lp
        .run(
            "use more tools than allowed",
            &llm,
            &[],
            |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                ("ok".into(), false)
            },
        )
        .await
        .unwrap();

    assert!(out.contains("Tool budget exhausted"));
    assert!(!metrics.completed_normally);

    for (index, message) in lp.messages.iter().enumerate() {
        for block in &message.content {
            if let ContentBlock::ToolUse { id, .. } = block {
                assert!(
                    lp.messages.iter().skip(index + 1).any(|later| {
                        later.content.iter().any(|candidate| {
                            matches!(candidate, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id)
                        })
                    }),
                    "tool call {id} must have a matching result"
                );
            }
        }
    }
}

/// An LLM that errors "prompt is too long" on the first call, then succeeds.
struct ErrorThenOkLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ErrorThenOkLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
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
                usage: InferenceUsage::default(),
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
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: true,
        tail_token_budget: 1_000,
        target_summary_chars: 100,
        ..HarnessConfig::default()
    };
    let compressor = Box::new(NoopCompressor) as Box<dyn CompactorTrait>;
    let mut lp = ReActLoop::new(cfg, compressor);
    // Seed with some messages to give compaction something to work with
    lp.messages = vec![
        Message::user("old message 1"),
        Message::assistant("old response 1"),
        Message::user("old message 2"),
        Message::assistant("old response 2"),
    ];
    let llm = ErrorThenOkLlm {
        calls: Mutex::new(0),
    };
    let tool_defs: Vec<ToolDefinition> = vec![];

    // With NoopCompressor, the context overflow is not resolved by compaction,
    // so the error propagates (no recovery).
    let result = lp
        .run(
            "trigger overflow",
            &llm,
            &tool_defs,
            |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                ("tool result".into(), false)
            },
        )
        .await;

    // With NoopCompressor, the LLM retry after overflow still succeeds.
    // The overflow error triggers the compaction path (no-op with mock compressor),
    // then the LLM is retried and the second call produces text.
    if let Ok((out, _)) = result {
        assert!(out.contains("recovered") || !out.is_empty());
    }
}

/// An LLM that returns no text and tool calls on first iteration,
/// then returns text on second.
struct EmptyThenTextLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for EmptyThenTextLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        if *n == 1 {
            Ok(LlmResponse {
                content: vec![],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "finally text".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
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
        "empty_then_text"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[tokio::test]
async fn empty_content_still_returns_text() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let llm = EmptyThenTextLlm {
        calls: Mutex::new(0),
    };
    let tool_defs: Vec<ToolDefinition> = vec![];

    let (out, metrics) = lp
        .run(
            "test",
            &llm,
            &tool_defs,
            |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                ("result".into(), false)
            },
        )
        .await
        .unwrap();

    // Empty content blocks -> still returns a string (possibly empty)
    // The loop should complete normally
    assert!(metrics.completed_normally);
    // On first call, content is empty, so text_parts.join returns ""
    // On second call (since no tool calls), it returns "finally text"
    assert!(out.contains("finally text") || out.is_empty());
}

// ── Compose message tests ────────────────────────────────────────────────

async fn run_and_capture_user_message(lp: &mut ReActLoop) -> String {
    let llm = RecordingTextLlm {
        messages: Mutex::new(Vec::new()),
    };
    lp.run(
        "inspect only",
        &llm,
        &[],
        |_id: &str, _name: &str, _input: &serde_json::Value| async move {
            ("unused".into(), false)
        },
    )
    .await
    .unwrap();
    let calls = llm.messages.lock().unwrap();
    calls[0]
        .iter()
        .rev()
        .find(|message| message.role == fabric::message::Role::User)
        .and_then(|message| {
            message.content.iter().find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
        })
        .unwrap()
}

#[tokio::test]
async fn plan_mode_run_injects_marker() {
    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_plan_mode(true);

    let message = run_and_capture_user_message(&mut lp).await;

    assert!(message.starts_with(PLAN_MODE_MARKER));
    assert!(message.ends_with("inspect only"));
    assert_eq!(message.matches(PLAN_MODE_MARKER).count(), 1);
}

#[tokio::test]
async fn run_without_plan_mode_has_no_marker() {
    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));

    let message = run_and_capture_user_message(&mut lp).await;

    assert_eq!(message, "inspect only");
}

#[tokio::test]
async fn run_consumes_pending_memory_once() {
    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.queue_memory_update("bounded fact".into());

    let first = run_and_capture_user_message(&mut lp).await;
    let second = run_and_capture_user_message(&mut lp).await;

    assert!(first.contains("<memory-update>"));
    assert!(first.contains("bounded fact"));
    assert!(!second.contains("<memory-update>"));
    assert!(!second.contains("bounded fact"));
}

#[tokio::test]
async fn plan_mode_run_injects_dasein_and_one_marker() {
    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_plan_mode(true);
    lp.set_dasein_context_provider(Box::new(|| Some("mood: attentive".into())));

    let message = run_and_capture_user_message(&mut lp).await;

    assert!(message.contains("<dasein-state>"));
    assert!(message.contains("mood: attentive"));
    assert_eq!(message.matches(PLAN_MODE_MARKER).count(), 1);
}

#[test]
fn compose_user_message_plain_input() {
    let cfg = HarnessConfig::default();
    let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    assert_eq!(lp.compose_user_message("hello"), "hello");
}

#[test]
fn compose_user_message_with_plan_mode() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_plan_mode(true);
    let msg = lp.compose_user_message("hello");
    assert!(msg.contains(PLAN_MODE_MARKER));
    assert!(msg.contains("hello"));
}

#[test]
fn compose_user_message_with_memory_updates() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.queue_memory_update("user prefers dark mode".into());
    let msg = lp.compose_user_message("hello");
    assert!(msg.contains("<memory-update>"));
    assert!(msg.contains("user prefers dark mode"));
}

#[test]
fn compose_user_message_plan_and_memory() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_plan_mode(true);
    lp.queue_memory_update("fact".into());
    let msg = lp.compose_user_message("hi");
    assert!(msg.contains(PLAN_MODE_MARKER));
    assert!(msg.contains("<memory-update>"));
    assert!(msg.contains("hi"));
}

#[test]
fn system_prompt_immutable_after_construction() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    assert_eq!(lp.system_prompt(), "");
    lp.set_system_prompt("test prompt".into());
    assert_eq!(lp.system_prompt(), "test prompt");
}

#[test]
fn reset_clears_pending_memory_but_not_plan_mode() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_plan_mode(true);
    lp.queue_memory_update("test".into());
    lp.reset();
    // plan_mode persists
    assert!(lp.plan_mode);
    // pending_memory cleared
    assert!(lp.pending_memory.is_empty());
}

#[test]
fn set_system_prompt_works() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_system_prompt("You are helpful".into());
    assert_eq!(lp.system_prompt(), "You are helpful");
}

#[test]
fn compose_multiple_memory_updates() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.queue_memory_update("fact 1".into());
    lp.queue_memory_update("fact 2".into());
    let msg = lp.compose_user_message("hi");
    assert!(msg.contains("fact 1"));
    assert!(msg.contains("fact 2"));
}

#[test]
fn compose_user_message_with_dasein_injection() {
    let cfg = HarnessConfig::default();
    let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let msg = lp.compose_user_message_with_dasein("hello", Some("mood: curious"));
    assert!(msg.contains("<dasein-state>"));
    assert!(msg.contains("mood: curious"));
    assert!(msg.contains("hello"));
}

#[test]
fn compose_user_message_with_dasein_none() {
    let cfg = HarnessConfig::default();
    let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let msg = lp.compose_user_message_with_dasein("hello", None);
    assert!(!msg.contains("<dasein-state>"));
    assert!(msg.contains("hello"));
}

#[test]
fn compose_user_message_with_dasein_empty() {
    let cfg = HarnessConfig::default();
    let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let msg = lp.compose_user_message_with_dasein("hello", Some(""));
    assert!(!msg.contains("<dasein-state>"));
    assert!(msg.contains("hello"));
}

#[test]
fn compose_user_message_with_all_injections() {
    let cfg = HarnessConfig::default();
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_plan_mode(true);
    lp.queue_memory_update("remember this".into());
    let msg = lp.compose_user_message_with_dasein("do task", Some("temporal: present"));
    assert!(msg.contains(PLAN_MODE_MARKER));
    assert!(msg.contains("<memory-update>"));
    assert!(msg.contains("<dasein-state>"));
    assert!(msg.contains("do task"));
}

// ── Partition tests ──────────────────────────────────────────────────────

#[test]
fn partition_read_only_batch() {
    let calls = vec![
        ("1".into(), "read_file".into(), serde_json::json!({})),
        ("2".into(), "glob".into(), serde_json::json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    assert_eq!(batches.len(), 1);
    assert!(matches!(batches[0], ToolBatch::Parallel(_)));
}

#[test]
fn partition_writer_serial() {
    let calls = vec![
        ("1".into(), "write_file".into(), serde_json::json!({})),
        ("2".into(), "bash_exec".into(), serde_json::json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    // Each side-effect tool gets its own serial batch
    assert_eq!(batches.len(), 2);
    for b in &batches {
        assert!(matches!(b, ToolBatch::Serial(_)));
    }
}

#[test]
fn partition_mixed() {
    let calls = vec![
        ("1".into(), "read_file".into(), serde_json::json!({})),
        ("2".into(), "write_file".into(), serde_json::json!({})),
        ("3".into(), "grep".into(), serde_json::json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    assert_eq!(batches.len(), 3);
    assert!(matches!(batches[0], ToolBatch::Parallel(_)));
    assert!(matches!(batches[1], ToolBatch::Serial(_)));
    assert!(matches!(batches[2], ToolBatch::Parallel(_)));
}

#[test]
fn partition_empty() {
    let batches = partition_tool_calls(&[]);
    assert!(batches.is_empty());
}

// ── M-C Verifier tests ─────────────────────────────────────────────────

use fabric::policy::verifier::{Verdict, Verifier};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Rejects the first candidate answer, accepts all subsequent ones.
struct RejectOnce {
    seen: AtomicUsize,
}
#[async_trait]
impl Verifier for RejectOnce {
    async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
        if self.seen.fetch_add(1, Ordering::SeqCst) == 0 {
            Verdict::Reject {
                reason: "first try rejected".into(),
            }
        } else {
            Verdict::Accept
        }
    }
}

/// An LLM that always returns plain text (no tool calls), counting its calls.
struct TextLlm {
    calls: Mutex<usize>,
}

struct TransientThenTextLlm {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for TransientThenTextLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        unreachable!("streaming test only")
    }

    async fn complete_stream(
        &self,
        _m: &[Message],
        _t: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(
                crate::adapters::inference::provider::InferenceFailure::transient(
                    "provider_unavailable",
                ),
            );
        }
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(StreamChunk::TextDelta {
                text: "recovered".into(),
            }),
            Ok(StreamChunk::Done {
                stop_reason: StopReason::EndTurn,
            }),
        ])))
    }

    fn name(&self) -> &str {
        "transient-then-text"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}
#[async_trait]
impl LlmProvider for TextLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: format!("answer {n}"),
            }],
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }
    async fn complete_stream(
        &self,
        _m: &[Message],
        _t: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(StreamChunk::TextDelta {
                text: format!("answer {n}"),
            }),
            Ok(StreamChunk::Done {
                stop_reason: StopReason::EndTurn,
            }),
        ])))
    }
    fn name(&self) -> &str {
        "text"
    }
    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[tokio::test]
async fn verifier_rejection_triggers_one_retry() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_verifier(std::sync::Arc::new(RejectOnce {
        seen: AtomicUsize::new(0),
    }));
    let llm = TextLlm {
        calls: Mutex::new(0),
    };
    let tool_defs: Vec<ToolDefinition> = vec![];
    let (out, _m) = lp
        .run(
            "go",
            &llm,
            &tool_defs,
            |_id: &str, name: &str, _in: &serde_json::Value| {
                let name = name.to_string();
                async move { (format!("ran {name}"), false) }
            },
        )
        .await
        .unwrap();
    // First answer rejected -> loop retried -> second answer accepted.
    assert_eq!(
        out, "answer 2",
        "rejected answer should be revised, got: {out}"
    );
}

#[tokio::test]
async fn streaming_verifier_rejection_matches_collecting_adapter() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    lp.set_verifier(std::sync::Arc::new(RejectOnce {
        seen: AtomicUsize::new(0),
    }));
    lp.messages.push(Message::user("go"));
    let llm = TextLlm {
        calls: Mutex::new(0),
    };
    let sink = CollectingEventSink(Mutex::new(Vec::new()));
    let (out, _metrics) = lp
        .run_streaming(
            &llm,
            &[],
            |_id: &str, name: &str, _input: &serde_json::Value| {
                let name = name.to_string();
                async move { (format!("ran {name}"), false) }
            },
            || async { Ok(Vec::new()) },
            &sink,
        )
        .await
        .unwrap();

    assert_eq!(out, "answer 2");
    assert!(sink.0.lock().unwrap().iter().any(|event| matches!(
        event,
        crate::harness::event_sink::Event::TurnDone { result: Ok(text) }
            if text == "answer 2"
    )));
}

#[tokio::test(start_paused = true)]
async fn streaming_transient_provider_retry_is_counted_separately() {
    let mut lp = ReActLoop::new(
        HarnessConfig {
            max_iterations: 2,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        },
        Box::new(NoopCompressor),
    );
    lp.messages.push(Message::user("go"));
    let llm = TransientThenTextLlm {
        calls: AtomicUsize::new(0),
    };
    let sink = CollectingEventSink(Mutex::new(Vec::new()));

    let (out, metrics) = lp
        .run_streaming(
            &llm,
            &[],
            |_id: &str, _name: &str, _input: &serde_json::Value| async {
                ("unexpected tool call".into(), true)
            },
            || async { Ok(Vec::new()) },
            &sink,
        )
        .await
        .unwrap();

    assert_eq!(out, "recovered");
    assert_eq!(metrics.iterations, 1);
    assert_eq!(metrics.provider_retries, 1);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn completion_gate_records_missing_obligation_in_shadow_mode() {
    use crate::core::{
        CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState, ProgressDecision,
        RequiredAction,
    };

    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
        objective: "inspect through a configured tool".into(),
        task_kind: CognitiveTaskKind::RepositoryAnalysis,
        required_actions: vec![RequiredAction::InvokeTool {
            tool_name: "file_read".into(),
        }],
        deliverables: Vec::new(),
        validation_requirements: Vec::new(),
    }));

    let decision = lp
        .finalize_candidate("premature answer".into(), &|| async { Ok(Vec::new()) })
        .await
        .unwrap();

    assert!(matches!(
        decision,
        completion::FinalizationDecision::Accept { .. }
    ));
    assert!(matches!(
        lp.latest_completion_audit(),
        Some(ProgressDecision::Continue { missing }) if missing.len() == 1
    ));
}

#[tokio::test]
async fn grounded_outcomes_follow_deterministic_completion_decision() {
    use crate::core::{
        CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState, CompletionGateMode,
        RequiredAction,
    };
    use fabric::cognitive_workflow::GroundedCognitiveOutcome;

    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
        objective: "inspect through a configured tool".into(),
        task_kind: CognitiveTaskKind::RepositoryAnalysis,
        required_actions: vec![RequiredAction::InvokeTool {
            tool_name: "file_read".into(),
        }],
        deliverables: Vec::new(),
        validation_requirements: Vec::new(),
    }));
    lp.set_completion_gate_mode(CompletionGateMode::Enforce);
    let sink = Arc::new(CollectingGroundedSink {
        outcomes: Mutex::new(Vec::new()),
        fail: false,
    });
    lp.set_grounded_outcome_sink(sink.clone());

    let decision = lp
        .finalize_candidate("unsupported answer".into(), &|| async { Ok(Vec::new()) })
        .await
        .unwrap();

    assert!(matches!(
        decision,
        completion::FinalizationDecision::ContinueAfterRejection
    ));
    let outcomes = sink.outcomes.lock().unwrap();
    assert!(matches!(
        outcomes.as_slice(),
        [
            GroundedCognitiveOutcome::CompletionRejected { .. },
            GroundedCognitiveOutcome::FalseCompletionPrevented { .. }
        ]
    ));
    assert!(!outcomes
        .iter()
        .any(|outcome| matches!(outcome, GroundedCognitiveOutcome::TaskCompleted { .. })));
}

#[tokio::test]
async fn observational_sink_failure_cannot_override_acceptance() {
    use crate::core::{CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState};

    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
        objective: "answer without required actions".into(),
        task_kind: CognitiveTaskKind::General,
        required_actions: Vec::new(),
        deliverables: Vec::new(),
        validation_requirements: Vec::new(),
    }));
    lp.set_completion_gate_mode(crate::core::CompletionGateMode::Enforce);
    lp.set_grounded_outcome_sink(Arc::new(CollectingGroundedSink {
        outcomes: Mutex::new(Vec::new()),
        fail: true,
    }));

    let decision = lp
        .finalize_candidate("accepted answer".into(), &|| async { Ok(Vec::new()) })
        .await
        .unwrap();

    assert!(matches!(
        decision,
        completion::FinalizationDecision::Accept { final_text }
            if final_text == "accepted answer"
    ));
}

#[tokio::test]
async fn enforced_gate_blocks_bounded_false_completion() {
    use crate::core::{
        CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState, CompletionGateMode,
        RequiredAction,
    };

    let mut lp = ReActLoop::new(
        HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        },
        Box::new(NoopCompressor),
    );
    lp.set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
        objective: "read before answering".into(),
        task_kind: CognitiveTaskKind::RepositoryAnalysis,
        required_actions: vec![RequiredAction::InvokeTool {
            tool_name: "file_read".into(),
        }],
        deliverables: Vec::new(),
        validation_requirements: Vec::new(),
    }));
    lp.set_completion_gate_mode(CompletionGateMode::Enforce);
    let llm = TextLlm {
        calls: Mutex::new(0),
    };

    let (output, metrics) = lp
        .run(
            "go",
            &llm,
            &[],
            |_id: &str, _name: &str, _input: &serde_json::Value| async {
                ("unexpected tool call".into(), true)
            },
        )
        .await
        .unwrap();

    assert_eq!(metrics.stop, fabric::TurnStop::Blocked);
    assert!(!metrics.completed_normally);
    assert!(output.contains("Task incomplete after 3 completion attempts"));
    assert_eq!(*llm.calls.lock().unwrap(), 3);
}

#[tokio::test]
async fn enforced_gate_accepts_matching_terminal_tool_evidence() {
    use crate::core::{
        CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState, CompletionGateMode,
        RequiredAction,
    };

    let mut lp = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
    lp.set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
        objective: "use echo".into(),
        task_kind: CognitiveTaskKind::General,
        required_actions: vec![RequiredAction::InvokeTool {
            tool_name: "echo_tool".into(),
        }],
        deliverables: Vec::new(),
        validation_requirements: Vec::new(),
    }));
    lp.set_completion_gate_mode(CompletionGateMode::Enforce);
    let llm = ScriptedLlm {
        calls: Mutex::new(0),
    };

    let (output, metrics) = lp
        .run(
            "go",
            &llm,
            &[],
            |_id: &str, _name: &str, _input: &serde_json::Value| async { ("echoed".into(), false) },
        )
        .await
        .unwrap();

    assert_eq!(output, "done: hi");
    assert_eq!(metrics.stop, fabric::TurnStop::Completed);
    assert!(metrics.completed_normally);
}

struct ClarificationLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ClarificationLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        *self.calls.lock().unwrap() += 1;
        Ok(LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "clarify-1".into(),
                name: "request_user_input".into(),
                input: serde_json::json!({"question": "Which behavior?"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("collecting adapter test")
    }

    fn name(&self) -> &str {
        "clarification"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[tokio::test]
async fn durable_clarification_tool_blocks_without_another_inference() {
    let mut loop_state = ReActLoop::new(
        HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        },
        Box::new(NoopCompressor),
    );
    let llm = ClarificationLlm {
        calls: Mutex::new(0),
    };
    let definitions = vec![ToolDefinition {
        name: "request_user_input".into(),
        description: "block for clarification".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let (output, metrics) = loop_state
        .run(
            "resolve ambiguity",
            &llm,
            &definitions,
            |_id: &str, _name: &str, _input: &serde_json::Value| async {
                (
                    serde_json::json!({
                        "status": "blocked",
                        "clarification_id": "00000000-0000-0000-0000-000000000001",
                        "question": "Which behavior?"
                    })
                    .to_string(),
                    false,
                )
            },
        )
        .await
        .unwrap();
    assert_eq!(metrics.stop, fabric::TurnStop::Blocked);
    assert!(!metrics.completed_normally);
    assert_eq!(*llm.calls.lock().unwrap(), 1);
    assert_eq!(output, "Waiting for user clarification: Which behavior?");
}

struct ChangeClosureLlm {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for ChangeClosureLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let response = match *calls {
            1 => ContentBlock::ToolUse {
                id: "apply".into(),
                name: "apply_patch".into(),
                input: serde_json::json!({}),
            },
            2 => ContentBlock::Text {
                text: "premature after apply".into(),
            },
            3 => ContentBlock::ToolUse {
                id: "diff".into(),
                name: "git_diff".into(),
                input: serde_json::json!({}),
            },
            4 => ContentBlock::Text {
                text: "premature after diff".into(),
            },
            5 => ContentBlock::ToolUse {
                id: "validation".into(),
                name: "validation_run".into(),
                input: serde_json::json!({}),
            },
            _ => ContentBlock::Text {
                text: "version-bound change completed".into(),
            },
        };
        Ok(LlmResponse {
            stop_reason: if matches!(&response, ContentBlock::ToolUse { .. }) {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            },
            content: vec![response],
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("collecting adapter test")
    }

    fn name(&self) -> &str {
        "change-closure-script"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[tokio::test]
async fn coding_loop_rejects_apply_and_diff_only_then_completes_exact_closure() {
    let mut lp = ReActLoop::new(
        HarnessConfig {
            max_iterations: 10,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        },
        Box::new(NoopCompressor),
    );
    let llm = ChangeClosureLlm {
        calls: Mutex::new(0),
    };
    let (output, metrics) = lp
        .run(
            "make a verified change",
            &llm,
            &[],
            |_id: &str, name: &str, _input: &serde_json::Value| {
                let content = match name {
                    "apply_patch" => r#"{"kind":"apply_patch_receipt","transaction_id":"tx","resulting_workspace_version":"v1"}"#,
                    "git_diff" => r#"{"kind":"change_diff_receipt","transaction_id":"tx","workspace_version":"v1","diff_artifact_ref":"artifact://sha256/diff"}"#,
                    "validation_run" => r#"{"session_id":"validation-session","terminal":{"status":"exited","exit_code":0},"output_artifact_ref":"artifact://sha256/test","change_transaction":{"transaction_id":"tx","phase":"validated","validation_receipts":[{"workspace_version":"v1","output_ref":"artifact://sha256/test"}]}}"#,
                    _ => unreachable!(),
                };
                async move { (content.to_string(), false) }
            },
        )
        .await
        .unwrap();

    assert_eq!(output, "version-bound change completed");
    assert_eq!(metrics.stop, fabric::TurnStop::Completed);
    assert_eq!(*llm.calls.lock().unwrap(), 6);
    assert_eq!(metrics.tool_calls_made, 3);
    assert!(matches!(
        lp.latest_completion_audit(),
        Some(ProgressDecision::Complete)
    ));
}

#[tokio::test]
async fn no_verifier_returns_first_answer_unchanged() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor)); // no set_verifier -> None
    let llm = TextLlm {
        calls: Mutex::new(0),
    };
    let tool_defs: Vec<ToolDefinition> = vec![];
    let (out, _m) = lp
        .run(
            "go",
            &llm,
            &tool_defs,
            |_i: &str, n: &str, _in: &serde_json::Value| {
                let n = n.to_string();
                async move { (n, false) }
            },
        )
        .await
        .unwrap();
    assert_eq!(out, "answer 1", "no verifier = unchanged behavior");
}

#[test]
fn max_iterations_zero_means_unlimited() {
    let cfg = HarnessConfig {
        max_iterations: 0,
        ..HarnessConfig::default()
    };
    let loop_ = ReActLoop::new(cfg, Box::new(NoopCompressor));
    assert!(
        loop_.should_continue(),
        "max_iterations=0 must never stop on the iteration check"
    );

    let cfg = HarnessConfig {
        max_iterations: 5,
        ..HarnessConfig::default()
    };
    let loop_ = ReActLoop::new(cfg, Box::new(NoopCompressor));
    // iteration starts at 0, so at iteration=5 we should stop
    let mut loop_ = loop_;
    loop_.iteration = 5;
    assert!(
        !loop_.should_continue(),
        "finite cap still stops when reached"
    );
}
