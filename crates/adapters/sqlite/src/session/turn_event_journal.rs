//! Canonical Session journal adapter for streaming Turn events.

use contracts::ipc::TurnEventV1;

pub async fn journal_protocol_turn_event(
    sessions: &runtime::session_service::SessionService,
    session_id: &str,
    turn_id: ::contracts::TurnId,
    assistant_item_id: &str,
    event: &TurnEventV1,
) -> anyhow::Result<()> {
    use ::contracts::protocol::client::ItemPhase;
    let session_id = ::contracts::SessionId(session_id.to_owned());
    let entry = match event {
        TurnEventV1::TurnStarted { .. } => Some((
            assistant_item_id.to_owned(),
            ItemPhase::Started,
            None,
            Some(format!("{assistant_item_id}:assistant-started")),
        )),
        TurnEventV1::TextDelta { delta } => Some((
            assistant_item_id.to_owned(),
            ItemPhase::Streaming,
            Some(delta.clone()),
            None,
        )),
        TurnEventV1::ToolCallStart { call_id, .. } => {
            let id = format!("tool:{}:{call_id}", turn_id.0);
            Some((
                id.clone(),
                ItemPhase::Started,
                None,
                Some(format!("{id}:tool-started")),
            ))
        }
        TurnEventV1::ToolProgress {
            call_id, payload, ..
        } => Some((
            format!("tool:{}:{call_id}", turn_id.0),
            ItemPhase::Streaming,
            Some(payload.to_string()),
            None,
        )),
        _ => None,
    };
    if let Some((item_id, phase, delta, dedupe_key)) = entry {
        sessions
            .append_protocol_item_event(&session_id, item_id, phase, delta, None, None, dedupe_key)
            .await?;
    }
    Ok(())
}
