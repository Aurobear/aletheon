//! Pure Aletheon client-event to ACP session-update translation.

use fabric::protocol::client::{ApprovalEvent, ClientEvent, EventCursor, ItemEvent, ItemPhase};
use serde_json::{json, Value};

pub fn map_client_event_to_acp(event: &ClientEvent) -> Option<Value> {
    match event {
        ClientEvent::Item(item) => Some(map_item_event(item)),
        ClientEvent::Approval(approval) => Some(map_approval_to_permission(approval)),
        ClientEvent::Reconnected(cursor) => Some(map_reconnect(cursor)),
        ClientEvent::Snapshot(snapshot) => Some(json!({
            "sessionUpdate": "snapshot",
            "sessionId": snapshot.session_id.0,
            "cursor": cursor_json(&snapshot.cursor),
            "items": snapshot.items,
            "approvals": snapshot.approvals,
            "agents": snapshot.agents,
        })),
        ClientEvent::Failed { cursor, message } => Some(json!({
            "sessionUpdate": "error",
            "message": message,
            "cursor": cursor.as_ref().map(cursor_json),
        })),
        ClientEvent::TurnCompleted {
            status,
            stop,
            error,
            retryable,
            usage,
            ..
        } => {
            let status = status.unwrap_or_else(|| fabric::TurnTerminalStatus::from(stop.clone()));
            Some(json!({
                "sessionUpdate": "turn_end",
                "stopReason": format!("{status:?}").to_ascii_lowercase(),
                "error": error,
                "retryable": retryable,
                "usage": usage,
            }))
        }
        ClientEvent::TurnStopped { reason, .. } => {
            let status = fabric::TurnTerminalStatus::from(reason.clone());
            Some(json!({
                "sessionUpdate": "turn_end",
                "stopReason": format!("{status:?}").to_ascii_lowercase(),
            }))
        }
        ClientEvent::InitializeResponse(_)
        | ClientEvent::Agent(_)
        | ClientEvent::CommandCompleted { .. }
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
    use std::collections::BTreeMap;

    fn cursor(sequence: u64) -> EventCursor {
        EventCursor {
            sequence,
            event_id: Some(format!("event-{sequence}")),
        }
    }

    #[test]
    fn snapshot_maps_before_incremental_replay() {
        let update = map_client_event_to_acp(&ClientEvent::Snapshot(
            fabric::protocol::client::UiSnapshot {
                session_id: fabric::SessionId("s".into()),
                cursor: cursor(4),
                provider: None,
                model: None,
                items: Vec::new(),
                approvals: Vec::new(),
                agents: Vec::new(),
            },
        ))
        .unwrap();
        assert_eq!(update["sessionUpdate"], "snapshot");
        assert_eq!(update["cursor"]["sequence"], 4);
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
    fn approval_maps_to_permission_with_authoritative_call_correlation() {
        let subject = fabric::ApprovalSubject {
            category: fabric::ApprovalCategory::ApplyCode,
            goal_id: fabric::GoalId(42),
            attempt_id: None,
            job_id: None,
            attributes: BTreeMap::new(),
            allowed_scope: Vec::new(),
            apply_target: None,
        };
        let approval_id = fabric::ApprovalId(uuid::Uuid::from_u128(77));
        let event = ClientEvent::Approval(ApprovalEvent {
            cursor: cursor(8),
            approval: fabric::ApprovalSnapshot {
                id: approval_id,
                goal_id: subject.goal_id,
                attempt_id: None,
                job_id: None,
                owner_id: fabric::PrincipalId("owner".into()),
                category: subject.category,
                risk: fabric::ApprovalRisk::High,
                subject_hash: subject.subject_hash().unwrap(),
                subject,
                summary: "apply verified patch".into(),
                artifacts: Vec::new(),
                created_at_ms: 1,
                expires_at_ms: 100,
                status: fabric::ApprovalStatus::Pending,
                version: 1,
                resolution: None,
            },
        });

        let update = map_client_event_to_acp(&event).unwrap();
        assert_eq!(update["sessionUpdate"], "request_permission");
        assert_eq!(update["requestId"], approval_id.to_string());
        assert_eq!(update["callId"], approval_id.to_string());
        assert_eq!(update["goalId"], "42");
        assert_eq!(update["cursor"]["sequence"], 8);
    }

    #[test]
    fn reconnect_requires_snapshot_before_incremental_subscription() {
        let update = map_client_event_to_acp(&ClientEvent::Reconnected(cursor(9))).unwrap();
        assert_eq!(update["recovery"][0]["step"], "snapshot");
        assert_eq!(update["recovery"][1]["step"], "subscribe");
        assert_eq!(update["recovery"][1]["after"]["sequence"], 9);
    }

    #[test]
    fn acp_terminal_status_matches_canonical_tui_projection() {
        for stop in [
            fabric::TurnStop::Completed,
            fabric::TurnStop::Blocked,
            fabric::TurnStop::Cancelled,
            fabric::TurnStop::Failed,
        ] {
            let expected = format!("{:?}", fabric::TurnTerminalStatus::from(stop.clone()))
                .to_ascii_lowercase();
            let event = ClientEvent::TurnCompleted {
                thread_id: fabric::ThreadId("thread".into()),
                turn_id: fabric::TurnId::new(),
                operation_id: fabric::OperationId::new(),
                status: Some(fabric::TurnTerminalStatus::from(stop.clone())),
                stop,
                error: None,
                retryable: false,
                usage: Default::default(),
            };
            let mut tui = crate::tui::state::AppState::default();
            assert!(crate::tui::reducer::reduce_terminal(&mut tui, &event));
            let update = map_client_event_to_acp(&event).unwrap();
            assert_eq!(update["sessionUpdate"], "turn_end");
            assert_eq!(update["stopReason"], expected);
            assert_eq!(
                format!("{:?}", tui.last_terminal_status.unwrap()).to_ascii_lowercase(),
                update["stopReason"]
            );
            assert!(is_turn_terminal(&event));
        }
    }

    #[test]
    fn acp_terminal_projection_preserves_rich_failure_fields() {
        let event = ClientEvent::TurnCompleted {
            thread_id: fabric::ThreadId("thread".into()),
            turn_id: fabric::TurnId::new(),
            operation_id: fabric::OperationId::new(),
            stop: fabric::TurnStop::Failed,
            status: Some(fabric::TurnTerminalStatus::Failed),
            error: Some(fabric::protocol::client::TurnCompletionError {
                code: Some("overloaded".into()),
                message: "provider overloaded".into(),
            }),
            retryable: true,
            usage: fabric::protocol::client::TurnCompletionUsage {
                input_tokens: 7,
                output_tokens: 2,
                tool_calls: 1,
                elapsed_ms: 40,
            },
        };
        let update = map_client_event_to_acp(&event).unwrap();
        assert_eq!(update["stopReason"], "failed");
        assert_eq!(update["error"]["code"], "overloaded");
        assert_eq!(update["retryable"], true);
        assert_eq!(update["usage"]["input_tokens"], 7);
    }
}
