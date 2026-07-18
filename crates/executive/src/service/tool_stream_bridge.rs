//! Tool stream bridge — connects G2 ToolEventSink producers to
//! `TurnEventV1::ToolProgress` consumers without bypassing terminal governance.

use fabric::ipc::stream::{TurnEventSender, TurnEventV1};
use fabric::types::tool::ToolResult;
use fabric::types::tool_stream::{
    ToolEventSink, ToolExecutionError, ToolExecutionEvent, ToolProgress, tool_event_channel,
};
use tokio_util::sync::CancellationToken;

/// Maximum coalesced text payload emitted at once.
pub const TOOL_PROGRESS_TEXT_BYTES: usize = 4 * 1024;
/// Per-call protection against flooding the downstream turn stream.
pub const TOOL_PROGRESS_EVENT_LIMIT: usize = 64;

/// The tool-side handle given to streaming tool implementations.
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

/// Non-authoritative bridge result. The terminal value must still pass through
/// the existing Executive settle/audit path before it becomes a ToolResult.
#[derive(Debug)]
pub struct ToolStreamOutcome {
    pub terminal: Result<ToolResult, ToolExecutionError>,
    pub progress_emitted: usize,
    pub progress_dropped: usize,
}

struct ProgressAccumulator {
    text: String,
    emitted: usize,
    dropped: usize,
}

impl ProgressAccumulator {
    fn new() -> Self {
        Self {
            text: String::new(),
            emitted: 0,
            dropped: 0,
        }
    }

    fn accept(
        &mut self,
        progress: ToolProgress,
        turn_events: &TurnEventSender,
        tool_name: &str,
        call_id: &str,
    ) {
        match progress {
            ToolProgress::Text(chunk) => {
                self.text.push_str(&chunk);
                while self.text.len() >= TOOL_PROGRESS_TEXT_BYTES {
                    let split = floor_char_boundary(&self.text, TOOL_PROGRESS_TEXT_BYTES);
                    let remainder = self.text.split_off(split);
                    let payload = std::mem::replace(&mut self.text, remainder);
                    self.emit(
                        "text",
                        serde_json::Value::String(payload),
                        turn_events,
                        tool_name,
                        call_id,
                    );
                }
            }
            other => {
                self.flush_text(turn_events, tool_name, call_id);
                self.emit(
                    other.kind(),
                    other.to_payload(),
                    turn_events,
                    tool_name,
                    call_id,
                );
            }
        }
    }

    fn flush_text(&mut self, turn_events: &TurnEventSender, tool_name: &str, call_id: &str) {
        if self.text.is_empty() {
            return;
        }
        let payload = serde_json::Value::String(std::mem::take(&mut self.text));
        self.emit("text", payload, turn_events, tool_name, call_id);
    }

    fn emit(
        &mut self,
        kind: &str,
        payload: serde_json::Value,
        turn_events: &TurnEventSender,
        tool_name: &str,
        call_id: &str,
    ) {
        if self.emitted >= TOOL_PROGRESS_EVENT_LIMIT {
            self.dropped += 1;
            return;
        }
        let event = TurnEventV1::ToolProgress {
            name: tool_name.to_owned(),
            call_id: call_id.to_owned(),
            kind: kind.to_owned(),
            payload,
        };
        match turn_events.send(&event) {
            Ok(()) => self.emitted += 1,
            Err(_) => self.dropped += 1,
        }
    }
}

fn floor_char_boundary(value: &str, requested: usize) -> usize {
    let mut split = requested.min(value.len());
    while !value.is_char_boundary(split) {
        split -= 1;
    }
    split
}

/// Drain one governed tool stream, forwarding only progress and returning the
/// unique terminal for settlement. Channel close synthesizes `NoTerminal`;
/// cancellation synthesizes `Cancelled` and prevents a later success terminal.
pub async fn bridge_tool_stream(
    mut event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
    cancel: CancellationToken,
) -> ToolStreamOutcome {
    let mut accumulator = ProgressAccumulator::new();
    let terminal = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                break Err(ToolExecutionError::Cancelled("turn cancelled".into()));
            }
            event = event_rx.recv() => match event {
                Some(ToolExecutionEvent::Progress(progress)) => {
                    accumulator.accept(progress, &turn_events, &tool_name, &call_id);
                }
                Some(ToolExecutionEvent::Notification(_)) => {
                    // Notifications remain UI-only and are not model or terminal data.
                }
                Some(ToolExecutionEvent::Terminal(result)) => break result,
                None => break Err(ToolExecutionError::NoTerminal),
            }
        }
    };
    accumulator.flush_text(&turn_events, &tool_name, &call_id);
    ToolStreamOutcome {
        terminal,
        progress_emitted: accumulator.emitted,
        progress_dropped: accumulator.dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::ipc::stream::{OverflowPolicy, StreamConfig, TurnEventStream};
    use fabric::types::tool::{ToolResult, ToolResultMeta};

    fn turn_stream(capacity: usize) -> (TurnEventStream, TurnEventSender) {
        TurnEventStream::new(StreamConfig {
            capacity,
            overflow: OverflowPolicy::BlockProducer,
        })
    }

    fn result() -> ToolResult {
        ToolResult {
            content: "done".into(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }

    #[tokio::test]
    async fn coalesces_progress_and_preserves_terminal() {
        let (mut sink, rx) = tool_event_channel();
        for _ in 0..10 {
            assert!(sink.progress(ToolProgress::Text("chunk".into())));
        }
        sink.terminal(Ok(result())).await;
        drop(sink);
        let (mut stream, sender) = turn_stream(8);

        let outcome = bridge_tool_stream(
            rx,
            sender,
            "bash".into(),
            "call-1".into(),
            CancellationToken::new(),
        )
        .await;

        assert!(outcome.terminal.is_ok());
        assert_eq!(outcome.progress_emitted, 1);
        assert!(matches!(
            stream.try_recv(),
            Some(Ok(TurnEventV1::ToolProgress { .. }))
        ));
    }

    #[tokio::test]
    async fn progress_flood_is_bounded_and_terminal_survives() {
        let (mut sink, rx) = tool_event_channel();
        let producer = tokio::spawn(async move {
            for index in 0..1_000 {
                let _ = sink.progress(ToolProgress::Structured(serde_json::json!({"i": index})));
                tokio::task::yield_now().await;
            }
            sink.terminal(Ok(result())).await;
        });
        let (_stream, sender) = turn_stream(128);
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "search".into(),
            "call-2".into(),
            CancellationToken::new(),
        )
        .await;
        producer.await.unwrap();

        assert!(outcome.terminal.is_ok());
        assert!(outcome.progress_emitted <= TOOL_PROGRESS_EVENT_LIMIT);
        assert!(outcome.progress_dropped > 0);
    }

    #[tokio::test]
    async fn producer_drop_synthesizes_no_terminal() {
        let (sink, rx) = tool_event_channel();
        drop(sink);
        let (_stream, sender) = turn_stream(1);
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "tool".into(),
            "call-3".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            outcome.terminal,
            Err(ToolExecutionError::NoTerminal)
        ));
    }

    #[tokio::test]
    async fn cancellation_wins_over_later_success_terminal() {
        let (mut sink, rx) = tool_event_channel();
        let (_stream, sender) = turn_stream(1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        sink.terminal(Ok(result())).await;
        let outcome = bridge_tool_stream(rx, sender, "tool".into(), "call-4".into(), cancel).await;
        assert!(matches!(
            outcome.terminal,
            Err(ToolExecutionError::Cancelled(_))
        ));
    }

    #[tokio::test]
    async fn utf8_text_chunks_split_only_on_character_boundaries() {
        let (mut sink, rx) = tool_event_channel();
        assert!(sink.progress(ToolProgress::Text("界".repeat(2_000))));
        sink.terminal(Ok(result())).await;
        let (_stream, sender) = turn_stream(8);
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "tool".into(),
            "call-5".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(outcome.terminal.is_ok());
        assert_eq!(outcome.progress_emitted, 2);
    }
}
