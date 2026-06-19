//! Typed event stream for agent lifecycle events.
//!
//! All frontends observe the same event stream. Each frontend
//! implements `EventSink` to receive events.

use aletheon_abi::tool::ToolResult;

/// Lifecycle events emitted by the agent.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new turn started.
    TurnStarted,
    /// Streaming text from the LLM.
    Text { text: String },
    /// Reasoning/thinking text from the LLM.
    Reasoning { text: String },
    /// A tool call is about to be dispatched.
    ToolDispatch {
        name: String,
        args: serde_json::Value,
    },
    /// A tool execution completed.
    ToolResult {
        name: String,
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
    TurnDone {
        result: Result<String, String>,
    },
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
        sink.emit(Event::TurnStarted); // should not panic
    }

    #[test]
    fn tool_result_from_conversion() {
        let tr = ToolResult {
            content: "ok".into(),
            is_error: false,
            metadata: aletheon_abi::tool::ToolResultMeta {
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
            metadata: aletheon_abi::tool::ToolResultMeta {
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
        let event = Event::TurnStarted;
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("TurnStarted"));
    }

    #[test]
    fn channel_sink_try_send() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let sink = ChannelEventSink::new(tx);

        sink.emit(Event::TurnStarted);
        sink.emit(Event::Text {
            text: "hello".into(),
        });

        // Use blocking recv in a sync test context via tokio runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let e1 = rx.recv().await.unwrap();
            assert!(matches!(e1, Event::TurnStarted));

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

        sink.emit(Event::TurnStarted);

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let e1 = rx1.recv().await.unwrap();
            let e2 = rx2.recv().await.unwrap();
            assert!(matches!(e1, Event::TurnStarted));
            assert!(matches!(e2, Event::TurnStarted));
        });
    }

    #[test]
    fn event_variants_constructible() {
        // Verify all variants can be constructed
        let _ = Event::TurnStarted;
        let _ = Event::Text {
            text: "t".into(),
        };
        let _ = Event::Reasoning {
            text: "r".into(),
        };
        let _ = Event::ToolDispatch {
            name: "bash".into(),
            args: serde_json::json!({}),
        };
        let _ = Event::ToolResult {
            name: "bash".into(),
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
        let _ = Event::CompactionDone {
            summary_chars: 100,
        };
        let _ = Event::TurnDone {
            result: Ok("done".into()),
        };
        let _ = Event::Error {
            message: "err".into(),
        };
        let _ = Event::MemoryUpdated {
            fact: "f".into(),
        };
        let _ = Event::PlanModeChanged { enabled: true };
        let _ = Event::CacheDiagnostics {
            hit_tokens: 100,
            miss_tokens: 50,
            hit_rate: 0.67,
        };
    }
}
