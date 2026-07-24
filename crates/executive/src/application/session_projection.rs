//! Pure projection from canonical session items to model messages.

use anyhow::{bail, Result};
use fabric::{ContentBlock, ItemPayload, ItemRecord, Message, Role};

pub fn project_messages(items: &[ItemRecord]) -> Result<Vec<Message>> {
    let mut previous = 0;
    let mut messages = Vec::new();
    for item in items {
        if item.sequence <= previous {
            bail!(
                "items are duplicate or out of order at sequence {}",
                item.sequence
            );
        }
        previous = item.sequence;
    }
    // Canonical records remain immutable. Normalize only the model-facing
    // projection so an orphan result is never exposed during resume/replay.
    let normalized = crate::application::compaction_normalize::normalize_tool_pairs(
        items.iter().map(|item| item.payload.clone()).collect(),
    );
    for payload in &normalized.items {
        let message = match payload {
            ItemPayload::UserMessage { content } => Some(Message::user(content)),
            ItemPayload::AssistantMessage { content } => Some(Message::assistant(content)),
            ItemPayload::SystemNotice { content } => Some(Message::system(content)),
            ItemPayload::ToolCall {
                call_id,
                name,
                input,
            } => Some(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }],
            }),
            ItemPayload::ToolResult {
                call_id,
                content,
                is_error,
                ..
            } => Some(Message::tool_result(call_id, content, *is_error)),
            ItemPayload::ContextProjection { .. } => None,
        };
        if let Some(message) = message {
            messages.push(message);
        }
    }
    Ok(messages)
}
