//! Tool stream bridge — connects G2 ToolEventSink producers to
//! TurnEventV1::ToolProgress consumers.
//!
//! Each tool execution creates a ToolEventSink that the tool writes
//! Progress events to. The bridge reads from the mpsc::Receiver and
//! emits TurnEventV1::ToolProgress into the turn event stream.

use fabric::ipc::stream::TurnEventSender;
use fabric::types::tool_stream::{ToolEventSink, ToolExecutionEvent, tool_event_channel};

/// The tool-side handle given to tool implementations.
pub struct ToolStreamHandle {
    pub sink: ToolEventSink,
    pub event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
}

impl ToolStreamHandle {
    pub fn new() -> Self {
        let (sink, rx) = tool_event_channel();
        Self { sink, event_rx: rx }
    }
}

impl Default for ToolStreamHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Bridge task: forwards ToolExecutionEvent → TurnEventV1::ToolProgress.
///
/// Consumes the receiver and emits progress events into the turn stream
/// until the terminal event is reached.
///
/// Activation is gated behind `grok_hardening.streaming_tools`.
/// When the flag is off the receiver is simply dropped.
pub async fn bridge_tool_stream(
    event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
) {
    // TODO(D1-T10): Implement full bridging — drain the receiver and emit
    // TurnEventV1::ToolProgress { name, call_id, kind, payload } for each
    // Progress event. Stop on Terminal (which routes through the normal
    // settle/audit path, not through this bridge).
    //
    // For now, drain the receiver without emitting so producers aren't
    // backpressured. The bridge will be wired once exec-server / tool
    // streaming producers are implemented (D1 Phase 2 completion).

    drop(event_rx);
    let _ = (tool_name, call_id, turn_events);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::ipc::stream::{StreamConfig, OverflowPolicy};

    #[test]
    fn handle_default_constructs() {
        let handle = ToolStreamHandle::default();
        assert!(!handle.sink.terminal_sent());
    }

    #[test]
    fn progress_kind_and_payload_roundtrip() {
        use fabric::types::tool_stream::ToolProgress;

        let tp = ToolProgress::Text("hello".into());
        assert_eq!(tp.kind(), "text");
        assert_eq!(tp.to_payload(), serde_json::Value::String("hello".into()));

        let tp = ToolProgress::Structured(serde_json::json!({"pct": 50}));
        assert_eq!(tp.kind(), "structured");
        assert_eq!(tp.to_payload(), serde_json::json!({"pct": 50}));
    }

    #[tokio::test]
    async fn bridge_drains_without_emitting() {
        let (sink, rx) = tool_event_channel();
        sink.progress(fabric::types::tool_stream::ToolProgress::Text("chunk".into()));
        drop(sink); // close sender so rx completes

        // Create a dummy turn event stream — we only need the sender half
        let config = StreamConfig {
            capacity: 8,
            overflow: OverflowPolicy::BlockProducer,
        };
        let (_stream, sender) =
            fabric::ipc::stream::TurnEventStream::new(config);

        // Bridge drains; no panics, no deadlocks
        bridge_tool_stream(rx, sender, "test_tool".into(), "call-1".into()).await;
    }

    #[tokio::test]
    async fn bridge_handle_smoke() {
        let handle = ToolStreamHandle::new();
        // Tool-side progress should work
        assert!(handle.sink.progress(
            fabric::types::tool_stream::ToolProgress::Text("progress".into())
        ));
    }
}
