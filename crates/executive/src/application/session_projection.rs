//! Pure projection from canonical session items to model messages.

use anyhow::{bail, Result};
use fabric::{ContentBlock, ItemPayload, ItemRecord, Message, Role};

const MAX_PROJECTED_TOOL_RESULT_BYTES: usize = 8_000;

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
            } => Some(Message::tool_result(
                call_id,
                &bounded_tool_result(content),
                *is_error,
            )),
            ItemPayload::ContextProjection { .. } => None,
        };
        if let Some(message) = message {
            messages.push(message);
        }
    }
    Ok(messages)
}

pub(crate) fn bounded_tool_result(content: &str) -> String {
    if content.len() <= MAX_PROJECTED_TOOL_RESULT_BYTES {
        return content.to_owned();
    }
    let marker = format!(
        "\n... [canonical tool result truncated from {} bytes] ...\n",
        content.len()
    );
    let payload_budget = MAX_PROJECTED_TOOL_RESULT_BYTES.saturating_sub(marker.len());
    let mut head_end = payload_budget / 2;
    while head_end > 0 && !content.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = content.len().saturating_sub(payload_budget - head_end);
    while tail_start < content.len() && !content.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!(
        "{}{}{}",
        &content[..head_end],
        marker,
        &content[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_tool_results_are_bounded_for_model_projection() {
        let content = format!("开头{}结尾", "工具输出".repeat(10_000));
        let bounded = bounded_tool_result(&content);

        assert!(bounded.len() <= MAX_PROJECTED_TOOL_RESULT_BYTES);
        assert!(bounded.starts_with("开头"));
        assert!(bounded.ends_with("结尾"));
        assert!(bounded.contains("canonical tool result truncated"));
    }
}
