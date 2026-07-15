//! Temporary legacy journal to canonical history projection (removed by S02).

use fabric::{ItemId, ItemPayload, ItemRecord, SessionId, TurnId, SESSION_SCHEMA_VERSION};

use super::journal::SessionEvent;

#[allow(dead_code)] // S02 wires this adapter while migrating SessionManager.
pub(crate) struct LegacyJournalProjector;

impl LegacyJournalProjector {
    #[allow(dead_code)] // S02 wires this adapter while migrating SessionManager.
    pub(crate) fn project(
        event: &SessionEvent,
        session_id: SessionId,
        turn_id: TurnId,
        sequence: u64,
        created_at_ms: u64,
    ) -> Option<ItemRecord> {
        let payload = match event {
            SessionEvent::UserMessage { content } => ItemPayload::UserMessage {
                content: content.clone(),
            },
            SessionEvent::AssistantMessage { content } => ItemPayload::AssistantMessage {
                content: content.clone(),
            },
            SessionEvent::ToolUseBlock {
                tool_use_id,
                tool_name,
                input,
            }
            | SessionEvent::ToolCallStarted {
                tool_call_id: tool_use_id,
                tool_name,
                input,
            } => ItemPayload::ToolCall {
                call_id: tool_use_id.clone(),
                name: tool_name.clone(),
                input: input.clone(),
            },
            SessionEvent::ToolResultBlock {
                tool_use_id,
                content,
                is_error,
            } => ItemPayload::ToolResult {
                call_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
                permit_id: None,
                audit_id: None,
            },
            SessionEvent::ToolCallCompleted {
                tool_call_id,
                is_error,
                content,
                ..
            } => ItemPayload::ToolResult {
                call_id: tool_call_id.clone(),
                content: content.clone(),
                is_error: *is_error,
                permit_id: None,
                audit_id: None,
            },
            _ => return None,
        };
        Some(ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id,
            turn_id,
            sequence,
            created_at_ms,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_and_assistant_events_project_only_to_history_items() {
        let turn = TurnId::new();
        let user = LegacyJournalProjector::project(
            &SessionEvent::UserMessage {
                content: "u".into(),
            },
            SessionId("s".into()),
            turn,
            1,
            1,
        )
        .unwrap();
        let assistant = LegacyJournalProjector::project(
            &SessionEvent::AssistantMessage {
                content: "a".into(),
            },
            SessionId("s".into()),
            turn,
            2,
            2,
        )
        .unwrap();
        assert!(matches!(user.payload, ItemPayload::UserMessage { .. }));
        assert!(matches!(
            assistant.payload,
            ItemPayload::AssistantMessage { .. }
        ));
    }
}
