//! Typed event stream for agent lifecycle events.
//!
//! All frontends observe the same event stream. Each frontend
//! implements `EventSink` to receive events.

use base::tool::ToolResult;

/// Lifecycle events emitted by the agent.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new turn started.
    TurnStarted { iteration: usize },
    /// Streaming text from the LLM.
    Text { text: String },
    /// Streaming text delta (incremental token).
    TextDelta { delta: String },
    /// Reasoning/thinking text from the LLM.
    Reasoning { text: String },
    /// A tool call is about to be dispatched.
    ToolDispatch {
        name: String,
        args: serde_json::Value,
    },
    /// A tool call has started (name + call_id for streaming).
    ToolCallStart { name: String, call_id: String },
    /// A tool call's arguments are now complete (after streaming accumulation).
    ToolCallComplete {
        call_id: String,
        name: String,
        args: serde_json::Value,
    },
    /// A tool execution completed.
    ToolResult {
        name: String,
        call_id: String,
        result: ToolResultEvent,
    },
    /// Token usage update.
    Usage {
        tokens_in: u32,
        tokens_out: u32,
        cache_hit_tokens: u32,
        cache_miss_tokens: u32,
    },
    /// An approval is needed from the user.
    ApprovalRequest {
        id: String,
        tool: String,
        args: serde_json::Value,
        reason: String,
    },
    /// A question needs answering.
    AskRequest {
        id: String,
        question: String,
        options: Vec<String>,
    },
    /// Context compaction started.
    CompactionStarted,
    /// Context compaction completed.
    CompactionDone { summary_chars: usize },
    /// The turn completed.
    TurnDone { result: Result<String, String> },
    /// An error occurred.
    Error { message: String },
    /// Memory was updated (queued for next turn).
    MemoryUpdated { fact: String },
    /// Plan mode changed.
    PlanModeChanged { enabled: bool },
    /// Cache diagnostics.
    CacheDiagnostics {
        hit_tokens: u64,
        miss_tokens: u64,
        hit_rate: f64,
    },
    /// Awareness level changed during reasoning.
    AwarenessChanged { level: String, context: String },
    /// Mode was changed (default/plan/auto/sandbox).
    ModeChanged { mode: String },
    /// Sub-agent status update.
    SubAgentStatusChanged {
        agent_id: String,
        status: String,
        task: String,
    },
    /// Plan update (plan mode).
    PlanUpdate {
        version: u32,
        plan: String,
        critique: Option<String>,
        ready_for_approval: bool,
    },
    /// Streaming was interrupted.
    Interrupted { reason: String },
    /// Context window usage update.
    ContextUpdate { used_tokens: u32, max_tokens: u32 },
    /// Model was switched.
    ModelSwitch { model_name: String },
    /// Agent goal was set.
    GoalSet {
        goal: String,
        sub_goals: Vec<String>,
    },
    /// Reflection completed.
    Reflection {
        summary: String,
        recommendation: String,
    },
    /// Tool budget exceeded.
    BudgetExceeded { used: usize, max: usize },
    /// Circuit breaker tripped.
    CircuitBreakerTripped { reason: String },
    /// Compaction was triggered due to context usage.
    CompactionTriggered {
        used_tokens: usize,
        threshold: usize,
        reason: String,
    },
}

/// Simplified tool result for events.
#[derive(Debug, Clone)]
pub struct ToolResultEvent {
    pub content: String,
    pub is_error: bool,
    pub execution_time_ms: u64,
}

impl From<&ToolResult> for ToolResultEvent {
    fn from(tr: &ToolResult) -> Self {
        Self {
            content: tr.content.clone(),
            is_error: tr.is_error,
            execution_time_ms: tr.metadata.execution_time_ms,
        }
    }
}

/// Trait for receiving events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// mpsc-based sink for async frontends.
pub struct ChannelEventSink {
    tx: tokio::sync::mpsc::Sender<Event>,
}

impl ChannelEventSink {
    pub fn new(tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        Self { tx }
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, event: Event) {
        // Try send, drop if full (don't block the agent)
        let _ = self.tx.try_send(event);
    }
}

/// Broadcast sink for multiple subscribers.
pub struct BroadcastEventSink {
    tx: tokio::sync::broadcast::Sender<Event>,
}

impl BroadcastEventSink {
    pub fn new(tx: tokio::sync::broadcast::Sender<Event>) -> Self {
        Self { tx }
    }
}

impl EventSink for BroadcastEventSink {
    fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

/// No-op sink for testing.
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _event: Event) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_does_nothing() {
        let sink = NullEventSink;
        sink.emit(Event::TurnStarted { iteration: 0 }); // should not panic
    }

    #[test]
    fn tool_result_from_conversion() {
        let tr = ToolResult {
            content: "ok".into(),
            is_error: false,
            metadata: base::tool::ToolResultMeta {
                execution_time_ms: 50,
                truncated: false,
            },
        };
        let event = ToolResultEvent::from(&tr);
        assert_eq!(event.content, "ok");
        assert_eq!(event.execution_time_ms, 50);
    }

    #[test]
    fn tool_result_from_error() {
        let tr = ToolResult {
            content: "error output".into(),
            is_error: true,
            metadata: base::tool::ToolResultMeta {
                execution_time_ms: 10,
                truncated: false,
            },
        };
        let event = ToolResultEvent::from(&tr);
        assert_eq!(event.content, "error output");
        assert!(event.is_error);
    }

    #[test]
    fn event_clone_works() {
        let event = Event::Text {
            text: "hello".into(),
        };
        let cloned = event.clone();
        assert!(matches!(cloned, Event::Text { text } if text == "hello"));
    }

    #[test]
    fn event_debug_works() {
        let event = Event::TurnStarted { iteration: 0 };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("TurnStarted"));
    }

    #[test]
    fn channel_sink_try_send() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let sink = ChannelEventSink::new(tx);

        sink.emit(Event::TurnStarted { iteration: 0 });
        sink.emit(Event::Text {
            text: "hello".into(),
        });

        // Use blocking recv in a sync test context via tokio runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let e1 = rx.recv().await.unwrap();
            assert!(matches!(e1, Event::TurnStarted { iteration: 0 }));

            let e2 = rx.recv().await.unwrap();
            assert!(matches!(e2, Event::Text { text } if text == "hello"));
        });
    }

    #[test]
    fn broadcast_sink_sends_to_subscribers() {
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let sink = BroadcastEventSink::new(tx.clone());

        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();

        sink.emit(Event::TurnStarted { iteration: 0 });

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let e1 = rx1.recv().await.unwrap();
            let e2 = rx2.recv().await.unwrap();
            assert!(matches!(e1, Event::TurnStarted { iteration: 0 }));
            assert!(matches!(e2, Event::TurnStarted { iteration: 0 }));
        });
    }

    #[test]
    fn event_variants_constructible() {
        // Verify all variants can be constructed
        let _ = Event::TurnStarted { iteration: 0 };
        let _ = Event::Text { text: "t".into() };
        let _ = Event::Reasoning { text: "r".into() };
        let _ = Event::ToolDispatch {
            name: "bash".into(),
            args: serde_json::json!({}),
        };
        let _ = Event::ToolResult {
            name: "bash".into(),
            call_id: "call-1".into(),
            result: ToolResultEvent {
                content: "out".into(),
                is_error: false,
                execution_time_ms: 100,
            },
        };
        let _ = Event::Usage {
            tokens_in: 10,
            tokens_out: 20,
            cache_hit_tokens: 5,
            cache_miss_tokens: 5,
        };
        let _ = Event::ApprovalRequest {
            id: "1".into(),
            tool: "bash".into(),
            args: serde_json::json!({}),
            reason: "destructive".into(),
        };
        let _ = Event::AskRequest {
            id: "1".into(),
            question: "why?".into(),
            options: vec!["a".into()],
        };
        let _ = Event::CompactionStarted;
        let _ = Event::CompactionDone { summary_chars: 100 };
        let _ = Event::TurnDone {
            result: Ok("done".into()),
        };
        let _ = Event::Error {
            message: "err".into(),
        };
        let _ = Event::MemoryUpdated { fact: "f".into() };
        let _ = Event::PlanModeChanged { enabled: true };
        let _ = Event::CacheDiagnostics {
            hit_tokens: 100,
            miss_tokens: 50,
            hit_rate: 0.67,
        };
        let _ = Event::TextDelta {
            delta: "tok".into(),
        };
        let _ = Event::ToolCallStart {
            name: "bash".into(),
            call_id: "c1".into(),
        };
        let _ = Event::ToolCallComplete {
            call_id: "c1".into(),
            name: "bash".into(),
            args: serde_json::json!({"command": "ls"}),
        };
        let _ = Event::AwarenessChanged {
            level: "hesitant".into(),
            context: "uncertain about approach".into(),
        };
        let _ = Event::ModeChanged {
            mode: "plan".into(),
        };
        let _ = Event::SubAgentStatusChanged {
            agent_id: "sub1".into(),
            status: "running".into(),
            task: "research".into(),
        };
        let _ = Event::PlanUpdate {
            version: 1,
            plan: "do something".into(),
            critique: Some("needs work".into()),
            ready_for_approval: false,
        };
        let _ = Event::Interrupted {
            reason: "user_cancelled".into(),
        };
        let _ = Event::ContextUpdate {
            used_tokens: 50000,
            max_tokens: 128000,
        };
        let _ = Event::ModelSwitch {
            model_name: "claude-sonnet-4".into(),
        };
    }

    #[test]
    fn text_delta_event_debug() {
        let event = Event::TextDelta {
            delta: "hello".into(),
        };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("TextDelta"));
        assert!(debug_str.contains("hello"));
    }

    #[test]
    fn tool_call_start_event_clone() {
        let event = Event::ToolCallStart {
            name: "edit".into(),
            call_id: "abc".into(),
        };
        let cloned = event.clone();
        assert!(matches!(
            cloned,
            Event::ToolCallStart { name, call_id }
            if name == "edit" && call_id == "abc"
        ));
    }

    #[test]
    fn notify_tx_emits_tool_result_json() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

        // Simulate what handler.rs does: build a JSON notification and send it.
        let tool_name = "bash";
        let result_content = "output text";
        let event = serde_json::json!({
            "type": "tool_result",
            "name": tool_name,
            "result": result_content.chars().take(200).collect::<String>(),
        });
        let _ = tx.try_send(event.to_string());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let msg = rx.recv().await.unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
            assert_eq!(parsed["type"], "tool_result");
            assert_eq!(parsed["name"], "bash");
            assert_eq!(parsed["result"], "output text");
        });
    }
}
