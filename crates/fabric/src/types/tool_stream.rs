//! Streaming tool execution contract (G2).
//!
//! A tool call may emit zero-to-many `Progress`/`Notification` events, then
//! exactly one `Terminal`, after which no further events are valid. The
//! governed boundary is not bypassed by the stream: the terminal still passes
//! through the Executive settle/audit path; progress never represents success.
//!
//! This module holds the pure contract plus a bounded sink. Bridging progress
//! to the turn event spine and driving legacy tools live in the Executive.
//!
//! See `docs/plans/grok/exec/G2-streaming-tools.md`.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::types::tool::ToolResult;

/// A single event within one tool call. Invariant: 0..N Progress/Notification,
/// then exactly one Terminal, and nothing after the Terminal.
#[derive(Debug, Clone)]
pub enum ToolExecutionEvent {
    /// Progress: not entered into model context by default; may be summarized.
    Progress(ToolProgress),
    /// Notification: UI status / background handle / monitor. Never model context.
    Notification(ToolNotification),
    /// The single authoritative terminal. Ok = result, Err = execution error.
    Terminal(Result<ToolResult, ToolExecutionError>),
}

/// Progress payload. This phase supports text + structured + resource ref;
/// binary/image goes to a controlled artifact store in a later phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolProgress {
    /// Text fragment (stdout chunk, phase description), already coalesced tool-side.
    Text(String),
    /// Structured progress (download %, search phase, subtask counts).
    Structured(serde_json::Value),
    /// Host-minted reference to a landed file/artifact (no inline content).
    ResourceRef { uri: String, mime: Option<String> },
}

impl ToolProgress {
    /// Discriminant string for the `TurnEventV1::ToolProgress { kind }` field.
    pub fn kind(&self) -> &'static str {
        match self {
            ToolProgress::Text(_) => "text",
            ToolProgress::Structured(_) => "structured",
            ToolProgress::ResourceRef { .. } => "resource_ref",
        }
    }

    /// JSON payload for the turn event bridge.
    pub fn to_payload(&self) -> serde_json::Value {
        match self {
            ToolProgress::Text(s) => serde_json::Value::String(s.clone()),
            ToolProgress::Structured(v) => v.clone(),
            ToolProgress::ResourceRef { uri, mime } => serde_json::json!({
                "uri": uri,
                "mime": mime,
            }),
        }
    }
}

/// Non-model notification (does not enter context, does not affect terminal).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolNotification {
    pub kind: ToolNotificationKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolNotificationKind {
    BackgroundTaskStarted,
    MonitorEvent,
    UiStatus,
}

/// Tool execution error, distinct from `ToolResult { is_error: true }` (which is
/// "the tool returned an error result normally").
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, PartialEq, Eq)]
pub enum ToolExecutionError {
    #[error("tool cancelled: {0}")]
    Cancelled(String),
    #[error("tool panicked or stream ended without terminal")]
    NoTerminal,
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("execution failed: {0}")]
    Failed(String),
}

/// Per-call progress channel capacity. Independent of the turn stream's 64.
pub const TOOL_PROGRESS_CHANNEL_CAP: usize = 32;

/// Tool-side sender. The tool emits progress, then emits terminal exactly once.
/// If dropped without a terminal, the receiver observes the closed channel and
/// the runtime synthesizes a `NoTerminal` error.
pub struct ToolEventSink {
    tx: mpsc::Sender<ToolExecutionEvent>,
    call_id: Option<String>,
    terminal_sent: bool,
    terminal_result: Option<Result<ToolResult, ToolExecutionError>>,
}

/// Host-bound receiver for a governed call. Keeping the binding beside the
/// receiver lets the bridge detect accidental cross-call routing.
pub struct BoundToolEventReceiver {
    call_id: String,
    rx: mpsc::Receiver<ToolExecutionEvent>,
}

impl BoundToolEventReceiver {
    pub fn into_parts(self) -> (String, mpsc::Receiver<ToolExecutionEvent>) {
        (self.call_id, self.rx)
    }
}

impl ToolEventSink {
    /// Emit progress. Returns `false` if dropped (channel full or terminal
    /// already sent) — never blocks tool completion.
    pub fn progress(&self, p: ToolProgress) -> bool {
        if self.terminal_sent {
            tracing::warn!(
                call_id = self.call_id.as_deref().unwrap_or("unknown"),
                "tool progress emitted after terminal; event rejected"
            );
            return false;
        }
        self.tx.try_send(ToolExecutionEvent::Progress(p)).is_ok()
    }

    /// Emit a notification. Same drop-on-full semantics as progress.
    pub fn notify(&self, n: ToolNotification) -> bool {
        if self.terminal_sent {
            tracing::warn!(
                call_id = self.call_id.as_deref().unwrap_or("unknown"),
                "tool notification emitted after terminal; event rejected"
            );
            return false;
        }
        self.tx
            .try_send(ToolExecutionEvent::Notification(n))
            .is_ok()
    }

    /// Emit the single terminal. A second call is a protocol violation and is
    /// ignored (debug-asserted). Uses a bounded await send so the terminal is
    /// never dropped.
    pub async fn terminal(&mut self, result: Result<ToolResult, ToolExecutionError>) {
        if self.terminal_sent {
            tracing::warn!(
                call_id = self.call_id.as_deref().unwrap_or("unknown"),
                "second tool terminal ignored as a protocol violation"
            );
        }
        debug_assert!(
            !self.terminal_sent,
            "second terminal is a protocol violation"
        );
        if self.terminal_sent {
            return;
        }
        self.terminal_sent = true;
        self.terminal_result = Some(result.clone());
        let _ = self.tx.send(ToolExecutionEvent::Terminal(result)).await;
    }

    /// Whether the terminal has been sent.
    pub fn terminal_sent(&self) -> bool {
        self.terminal_sent
    }

    /// Borrow the terminal emitted by a tool so the governed executor can
    /// validate and settle the same result without re-running side effects.
    pub fn terminal_result(&self) -> Option<&Result<ToolResult, ToolExecutionError>> {
        self.terminal_result.as_ref()
    }
}

/// Create a bounded tool event channel (sink + receiver).
pub fn tool_event_channel() -> (ToolEventSink, mpsc::Receiver<ToolExecutionEvent>) {
    let (tx, rx) = mpsc::channel(TOOL_PROGRESS_CHANNEL_CAP);
    (
        ToolEventSink {
            tx,
            call_id: None,
            terminal_sent: false,
            terminal_result: None,
        },
        rx,
    )
}

/// Create a bounded tool event channel associated with a governed call.
///
/// Stream events deliberately do not carry a producer-supplied call id: the
/// host binds the channel to one call here, preventing call-id spoofing or
/// mismatch by construction. The id is retained for protocol-violation logs.
pub fn tool_event_channel_for_call(
    call_id: impl Into<String>,
) -> (ToolEventSink, BoundToolEventReceiver) {
    let call_id = call_id.into();
    let (tx, rx) = mpsc::channel(TOOL_PROGRESS_CHANNEL_CAP);
    (
        ToolEventSink {
            tx,
            call_id: Some(call_id.clone()),
            terminal_sent: false,
            terminal_result: None,
        },
        BoundToolEventReceiver { call_id, rx },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::tool::{ToolResult, ToolResultMeta};

    fn ok_result() -> ToolResult {
        ToolResult {
            content: "done".to_string(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }

    #[test]
    fn progress_kind_and_payload() {
        assert_eq!(ToolProgress::Text("x".into()).kind(), "text");
        assert_eq!(
            ToolProgress::Structured(serde_json::json!({"pct": 50})).kind(),
            "structured"
        );
        assert_eq!(
            ToolProgress::ResourceRef {
                uri: "file:///a".into(),
                mime: None
            }
            .kind(),
            "resource_ref"
        );
        assert_eq!(
            ToolProgress::Text("hi".into()).to_payload(),
            serde_json::Value::String("hi".into())
        );
    }

    #[tokio::test]
    async fn progress_then_single_terminal() {
        let (mut sink, mut rx) = tool_event_channel();
        assert!(sink.progress(ToolProgress::Text("chunk1".into())));
        assert!(sink.progress(ToolProgress::Text("chunk2".into())));
        sink.terminal(Ok(ok_result())).await;

        let mut progress_count = 0;
        let mut terminal_count = 0;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                ToolExecutionEvent::Progress(_) => progress_count += 1,
                ToolExecutionEvent::Terminal(_) => terminal_count += 1,
                ToolExecutionEvent::Notification(_) => {}
            }
        }
        assert_eq!(progress_count, 2);
        assert_eq!(terminal_count, 1);
    }

    #[tokio::test]
    async fn progress_after_terminal_is_rejected() {
        let (mut sink, _rx) = tool_event_channel();
        sink.terminal(Ok(ok_result())).await;
        assert!(!sink.progress(ToolProgress::Text("late".into())));
        assert!(!sink.notify(ToolNotification {
            kind: ToolNotificationKind::UiStatus,
            message: "late".into()
        }));
        assert!(sink.terminal_sent());
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    #[should_panic(expected = "second terminal is a protocol violation")]
    async fn second_terminal_is_a_debug_protocol_violation() {
        let (mut sink, _rx) = tool_event_channel();
        sink.terminal(Ok(ok_result())).await;
        sink.terminal(Ok(ok_result())).await;
    }

    #[tokio::test]
    async fn progress_dropped_when_channel_full() {
        let (sink, _rx) = tool_event_channel();
        // Fill beyond capacity without a consumer; excess must drop, not block.
        let mut accepted = 0;
        let mut dropped = 0;
        for i in 0..(TOOL_PROGRESS_CHANNEL_CAP + 16) {
            if sink.progress(ToolProgress::Text(format!("p{i}"))) {
                accepted += 1;
            } else {
                dropped += 1;
            }
        }
        assert_eq!(accepted, TOOL_PROGRESS_CHANNEL_CAP);
        assert!(dropped > 0, "excess progress must drop when full");
    }

    #[tokio::test]
    async fn terminal_delivers_even_when_progress_filled() {
        // Terminal uses awaiting send, so it is delivered once capacity frees up.
        let (mut sink, mut rx) = tool_event_channel();
        for i in 0..TOOL_PROGRESS_CHANNEL_CAP {
            let _ = sink.progress(ToolProgress::Text(format!("p{i}")));
        }
        // Spawn terminal send; a concurrent drain lets it complete.
        let term = tokio::spawn(async move {
            sink.terminal(Ok(ok_result())).await;
        });
        let mut saw_terminal = false;
        for _ in 0..(TOOL_PROGRESS_CHANNEL_CAP + 1) {
            if let Some(ev) = rx.recv().await {
                if matches!(ev, ToolExecutionEvent::Terminal(_)) {
                    saw_terminal = true;
                    break;
                }
            }
        }
        term.await.unwrap();
        assert!(saw_terminal, "terminal must be delivered, never dropped");
    }

    #[tokio::test]
    async fn drop_without_terminal_closes_channel() {
        let (sink, mut rx) = tool_event_channel();
        drop(sink);
        // Receiver observes closed channel -> runtime would synthesize NoTerminal.
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn execution_error_display() {
        assert_eq!(
            ToolExecutionError::NoTerminal.to_string(),
            "tool panicked or stream ended without terminal"
        );
        assert_eq!(
            ToolExecutionError::Cancelled("user".into()).to_string(),
            "tool cancelled: user"
        );
    }
}
