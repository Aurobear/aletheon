//! Pure Aletheon client-event to ACP session-update translation.

use fabric::protocol::client::{
    ApprovalEvent, ClientEvent, EventCursor, ItemEvent, ItemPhase,
};
use serde_json::{json, Value};

pub fn map_client_event_to_acp(event: &ClientEvent) -> Option<Value> {
    match event {
        ClientEvent::Item(item) => Some(map_item_event(item)),
        ClientEvent::Approval(approval) => Some(map_approval_to_permission(approval)),
        ClientEvent::Reconnected(cursor) => Some(map_reconnect(cursor)),
        ClientEvent::Failed { cursor, message } => Some(json!({
            "sessionUpdate": "error",
            "message": message,
            "cursor": cursor.as_ref().map(cursor_json),
        })),
        ClientEvent::TurnCompleted { stop, .. } | ClientEvent::TurnStopped { reason: stop, .. } => {
            Some(json!({
                "sessionUpdate": "turn_end",
                "stopReason": format!("{stop:?}").to_ascii_lowercase(),
            }))
        }
        ClientEvent::InitializeResponse(_)
        | ClientEvent::Snapshot(_)
        | ClientEvent::Agent(_)
        | ClientEvent::TurnStarted { .. } => None,
    }
}

fn map_item_event(item: &ItemEvent) -> Value {
    let update = match item.phase {
        ItemPhase::Started => "item_started",
        ItemPhase::Streaming => "agent_message_chunk",
        ItemPhase::Completed => "item_completed",
        ItemPhase::Failed => "item_failed",
    };
    json!({
        "sessionUpdate": update,
        "itemId": item.item_id,
        "phase": format!("{:?}", item.phase).to_ascii_lowercase(),
        "content": item.delta.as_ref().map(|text| json!({"type": "text", "text": text})),
        "item": item.item,
        "error": item.error,
        "cursor": cursor_json(&item.cursor),
    })
}

fn map_approval_to_permission(event: &ApprovalEvent) -> Value {
    let approval = &event.approval;
    json!({
        "sessionUpdate": "request_permission",
        // The durable approval id is the request/call correlation id. Resolution
        // still goes through Executive's scoped ApprovalUseCases.
        "requestId": approval.id.to_string(),
        "callId": approval.id.to_string(),
        "goalId": approval.goal_id.to_string(),
        "attemptId": approval.attempt_id.as_ref().map(|id| id.0.to_string()),
        "jobId": approval.job_id.as_ref().map(|id| id.0.to_string()),
        "title": approval.summary,
        "category": format!("{:?}", approval.category).to_ascii_lowercase(),
        "risk": format!("{:?}", approval.risk).to_ascii_lowercase(),
        "cursor": cursor_json(&event.cursor),
    })
}

fn map_reconnect(cursor: &EventCursor) -> Value {
    json!({
        "sessionUpdate": "reconnected",
        // This ordering contract directs the gateway to request/emit the
        // authoritative snapshot before subscribing after this cursor.
        "recovery": [
            {"step": "snapshot"},
            {"step": "subscribe", "after": cursor_json(cursor)}
        ]
    })
}

fn cursor_json(cursor: &EventCursor) -> Value {
    json!({"sequence": cursor.sequence, "eventId": cursor.event_id})
}

pub fn is_turn_terminal(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::TurnCompleted { .. }
            | ClientEvent::TurnStopped { .. }
            | ClientEvent::Failed { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(sequence: u64) -> EventCursor {
        EventCursor {
            sequence,
            event_id: Some(format!("event-{sequence}")),
        }
    }

    #[test]
    fn streaming_item_maps_to_text_chunk() {
        let event = ClientEvent::Item(ItemEvent {
            cursor: cursor(4),
            item_id: "item-1".into(),
            phase: ItemPhase::Streaming,
            delta: Some("hello".into()),
            item: None,
            error: None,
        });
        let update = map_client_event_to_acp(&event).unwrap();
        assert_eq!(update["sessionUpdate"], "agent_message_chunk");
        assert_eq!(update["content"]["text"], "hello");
        assert_eq!(update["cursor"]["sequence"], 4);
    }

    #[test]
    fn failed_event_maps_error_and_is_terminal() {
        let event = ClientEvent::Failed {
            cursor: Some(cursor(7)),
            message: "boom".into(),
        };
        let update = map_client_event_to_acp(&event).unwrap();
        assert_eq!(update["sessionUpdate"], "error");
        assert_eq!(update["message"], "boom");
        assert!(is_turn_terminal(&event));
    }

    #[test]
    fn reconnect_requires_snapshot_before_incremental_subscription() {
        let update = map_client_event_to_acp(&ClientEvent::Reconnected(cursor(9))).unwrap();
        assert_eq!(update["recovery"][0]["step"], "snapshot");
        assert_eq!(update["recovery"][1]["step"], "subscribe");
        assert_eq!(update["recovery"][1]["after"]["sequence"], 9);
    }
}
