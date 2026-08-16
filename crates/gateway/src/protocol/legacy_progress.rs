//! Legacy daemon progress event compatibility owned by Gateway protocol.

use serde::{Deserialize, Serialize};

/// Client-facing event produced by the daemon and consumed by the TUI/CLI.
///
/// This is the canonical wire-protocol type shared between daemon and client.
/// Every variant maps to an event notification sent over the Unix socket.
///
/// daemon:  executive::Event -> ClientEvent -> serde_json -> socket
/// client:  socket -> serde_json -> ClientEvent -> display handler
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    // ── Turn lifecycle ──
    TurnStarted {
        iteration: usize,
    },
    TurnDone,
    Error {
        message: String,
    },

    // ── Streaming text ──
    TextDelta {
        text: String,
    },
    /// Authoritative assistant text emitted after durable turn settlement.
    ///
    /// Unlike `TextDelta`, this replaces the in-progress assistant draft. It
    /// lets clients reconcile a bounded live stream if an intermediate delta
    /// was lost without treating a partial draft as the terminal answer.
    TextSnapshot {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },

    // ── Tool calls ──
    ToolCallStart {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    /// Emitted when streaming tool args are complete — carries the real args.
    ToolCallComplete {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolCallResult {
        call_id: String,
        tool: String,
        output: String,
        is_error: bool,
        elapsed_ms: u64,
        /// Structured filesystem delta from apply_patch (None for other tools).
        patch_delta: Option<::contracts::PatchDelta>,
    },
    ToolProgress {
        call_id: String,
        tool: String,
        kind: String,
        payload: serde_json::Value,
    },
    PatchProgress {
        status: String,
        path: Option<String>,
        operation: Option<String>,
        error: Option<String>,
        applied_count: Option<usize>,
        failed_count: Option<usize>,
    },

    // ── Bookkeeping ──
    Usage {
        #[serde(flatten)]
        usage: ::contracts::InferenceUsage,
    },
    ContextUpdate {
        max_tokens: u64,
        used_tokens: u64,
    },
    GoalSet {
        goal: String,
        sub_goals: Vec<String>,
    },
    ModelSwitch {
        model: String,
    },

    // ── Awareness / collaboration ──
    AwarenessChanged {
        level: String,
        context: String,
    },
    PlanUpdate {
        version: u32,
        plan: String,
        critique: Option<String>,
        ready_for_approval: bool,
    },
    SubAgentStatus {
        agent_id: String,
        task: String,
        status: String,
    },
    ModeChanged {
        new: String,
    },

    // ── Limits / interruptions ──
    Interrupted,
    BudgetExceeded {
        limit: u64,
    },
    CircuitBreakerTripped {
        reason: String,
    },
    CompactionTriggered,
    CompactionCompleted {
        strategy: String,
        tokens_before: u64,
        tokens_after: u64,
        evicted_messages: u64,
    },
    Reflection {
        summary: String,
    },
}

impl ClientEvent {
    /// Forward-compatible client decoding: unknown additive event variants are
    /// ignored rather than terminating the socket/TUI event loop.
    pub fn decode_if_known(value: serde_json::Value) -> Option<Self> {
        serde_json::from_value(value).ok()
    }
}

#[cfg(test)]
mod client_event_compatibility_tests {
    use super::ClientEvent;

    #[test]
    fn terminal_text_snapshot_round_trips() {
        let event = ClientEvent::TextSnapshot {
            text: "authoritative answer".into(),
        };
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(encoded["type"], "text_snapshot");
        assert!(matches!(
            ClientEvent::decode_if_known(encoded),
            Some(ClientEvent::TextSnapshot { text }) if text == "authoritative answer"
        ));
    }

    #[test]
    fn unknown_additive_event_is_ignored_without_panic() {
        assert!(ClientEvent::decode_if_known(serde_json::json!({
            "type": "future_progress_shape",
            "payload": {"pct": 50}
        }))
        .is_none());
    }
}
