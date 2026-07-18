//! ACP (Agent Client Protocol) edge mapping (G8).
//!
//! Pure translation from Aletheon's streaming `ClientEvent` to ACP
//! `session/update` notification payloads. This is an edge adapter concern:
//! ACP types never leak past interact, and the domain keeps using Aletheon's
//! own event stream. Only the mapping lives here; transport, correlation, and
//! request dispatch are separate.
//!
//! Represented as `serde_json::Value` (ACP session-update shape) so no ACP
//! crate dependency is required for this slice.
//!
//! See `docs/plans/grok/exec/G8-acp-adapter.md`.

use fabric::ClientEvent;
use serde_json::{json, Value};

/// Map one Aletheon `ClientEvent` to an ACP `session/update` payload, or `None`
/// for events with no client-facing ACP representation. The `sessionUpdate`
/// discriminator mirrors ACP's notification tag.
pub fn map_client_event_to_acp(event: &ClientEvent) -> Option<Value> {
    match event {
        ClientEvent::TextDelta { text } => Some(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text },
        })),
        ClientEvent::ThinkingDelta { text } => Some(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": text },
        })),
        ClientEvent::ToolCallStart {
            call_id,
            tool,
            args,
        } => Some(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": call_id,
            "title": tool,
            "status": "pending",
            "rawInput": args,
        })),
        ClientEvent::ToolCallComplete {
            call_id,
            tool,
            args,
        } => Some(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": call_id,
            "title": tool,
            "status": "in_progress",
            "rawInput": args,
        })),
        ClientEvent::ToolCallResult {
            call_id,
            tool,
            output,
            is_error,
            ..
        } => Some(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": call_id,
            "title": tool,
            "status": if *is_error { "failed" } else { "completed" },
            "content": [{ "type": "content", "content": { "type": "text", "text": output } }],
        })),
        ClientEvent::ToolProgress {
            call_id,
            tool,
            kind,
            payload,
        } => Some(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": call_id,
            "title": tool,
            "status": "in_progress",
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": payload },
            }],
            "_meta": { "aletheonProgressKind": kind },
        })),
        ClientEvent::PlanUpdate { plan, .. } => Some(json!({
            "sessionUpdate": "plan",
            "plan": plan,
        })),
        // Terminal / control events: represented as agent-facing stops. The ACP
        // gateway turns these into the prompt-turn result, not session updates,
        // so they are surfaced here as an explicit stop marker for the caller.
        ClientEvent::TurnDone => {
            Some(json!({ "sessionUpdate": "_end_turn", "stopReason": "end_turn" }))
        }
        ClientEvent::Error { message } => {
            Some(json!({ "sessionUpdate": "_error", "message": message }))
        }
        ClientEvent::Interrupted => {
            Some(json!({ "sessionUpdate": "_end_turn", "stopReason": "cancelled" }))
        }
        // No ACP session-update surface (bookkeeping / internal signals).
        ClientEvent::TurnStarted { .. }
        | ClientEvent::Usage { .. }
        | ClientEvent::ContextUpdate { .. }
        | ClientEvent::GoalSet { .. }
        | ClientEvent::ModelSwitch { .. }
        | ClientEvent::AwarenessChanged { .. }
        | ClientEvent::SubAgentStatus { .. }
        | ClientEvent::ModeChanged { .. }
        | ClientEvent::BudgetExceeded { .. }
        | ClientEvent::CircuitBreakerTripped { .. }
        | ClientEvent::CompactionTriggered
        | ClientEvent::Reflection { .. } => None,
    }
}

/// Whether an event is a turn-terminal signal (the ACP gateway uses this to
/// close the prompt turn rather than stream a session update).
pub fn is_turn_terminal(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::TurnDone | ClientEvent::Error { .. } | ClientEvent::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_maps_to_agent_message_chunk() {
        let ev = ClientEvent::TextDelta {
            text: "hello".into(),
        };
        let v = map_client_event_to_acp(&ev).unwrap();
        assert_eq!(v["sessionUpdate"], "agent_message_chunk");
        assert_eq!(v["content"]["text"], "hello");
    }

    #[test]
    fn tool_call_lifecycle_maps() {
        let start = ClientEvent::ToolCallStart {
            call_id: "c1".into(),
            tool: "bash".into(),
            args: json!({"cmd": "ls"}),
        };
        let v = map_client_event_to_acp(&start).unwrap();
        assert_eq!(v["sessionUpdate"], "tool_call");
        assert_eq!(v["toolCallId"], "c1");
        assert_eq!(v["status"], "pending");

        let ok = ClientEvent::ToolCallResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            output: "files".into(),
            is_error: false,
            elapsed_ms: 5,
        };
        let v = map_client_event_to_acp(&ok).unwrap();
        assert_eq!(v["sessionUpdate"], "tool_call_update");
        assert_eq!(v["status"], "completed");

        let err = ClientEvent::ToolCallResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            output: "boom".into(),
            is_error: true,
            elapsed_ms: 5,
        };
        let v = map_client_event_to_acp(&err).unwrap();
        assert_eq!(v["status"], "failed");
    }

    #[test]
    fn terminal_events_are_marked() {
        assert!(is_turn_terminal(&ClientEvent::TurnDone));
        assert!(is_turn_terminal(&ClientEvent::Interrupted));
        assert!(is_turn_terminal(&ClientEvent::Error {
            message: "x".into()
        }));
        assert!(!is_turn_terminal(&ClientEvent::TextDelta {
            text: "x".into()
        }));

        let done = map_client_event_to_acp(&ClientEvent::TurnDone).unwrap();
        assert_eq!(done["stopReason"], "end_turn");
        let cancelled = map_client_event_to_acp(&ClientEvent::Interrupted).unwrap();
        assert_eq!(cancelled["stopReason"], "cancelled");
    }

    #[test]
    fn bookkeeping_events_have_no_acp_surface() {
        for ev in [
            ClientEvent::TurnStarted { iteration: 1 },
            ClientEvent::Usage {
                tokens_in: 1,
                tokens_out: 2,
            },
            ClientEvent::CompactionTriggered,
            ClientEvent::ModeChanged { new: "auto".into() },
        ] {
            assert!(
                map_client_event_to_acp(&ev).is_none(),
                "{ev:?} should have no ACP session update"
            );
        }
    }
}
