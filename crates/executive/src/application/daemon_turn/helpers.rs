//! Text helpers and size constants shared across the daemon turn pipeline.
//!
//! Originally duplicated between `chat.rs` and `daemon_turn.rs`. Colocated here
//! so both the orchestrator and the handler can use them without copies.

use std::collections::HashSet;

use fabric::{ContentBlock, Message, Role};

// ── Size constants ──────────────────────────────────────────────────────────

// ── Free functions ───────────────────────────────────────────────────────────

pub(crate) fn select_text_history(history: &[Message], max_tokens: usize) -> Vec<Message> {
    if max_tokens == 0 {
        return Vec::new();
    }

    let projected: Vec<Message> = history.iter().filter_map(project_message).collect();
    let units = complete_units(&projected);
    let mut selected = Vec::new();
    let mut used = 0usize;

    for unit in units.iter().rev() {
        let unit_tokens: usize = unit.iter().map(Message::estimate_tokens).sum();
        if used.saturating_add(unit_tokens) <= max_tokens {
            selected.push(unit.clone());
            used += unit_tokens;
            continue;
        }
        if selected.is_empty() && unit.len() == 1 && unit[0].role == Role::User {
            if let Some(message) = truncate_message_to_tokens(&unit[0], max_tokens) {
                selected.push(vec![message]);
            }
        }
        break;
    }

    selected.reverse();
    selected.into_iter().flatten().collect()
}

fn project_message(message: &Message) -> Option<Message> {
    if message.role == Role::System {
        return None;
    }
    let content = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => {
                let text = strip_restored_payloads(text);
                (!text.trim().is_empty()).then_some(ContentBlock::Text { text })
            }
            ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. } => Some(block.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!content.is_empty()).then_some(Message {
        role: message.role,
        content,
    })
}

fn strip_restored_payloads(value: &str) -> String {
    let mut value = value.to_owned();
    for label in ["memory-context", "skills"] {
        let open = format!("<{label}>");
        let close = format!("</{label}>");
        while let Some(start) = value.find(&open) {
            let Some(relative_end) = value[start + open.len()..].find(&close) else {
                value.truncate(start);
                break;
            };
            let end = start + open.len() + relative_end + close.len();
            value.replace_range(start..end, "");
        }
    }
    value
}

fn complete_units(messages: &[Message]) -> Vec<Vec<Message>> {
    let mut units = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let tool_ids = tool_use_ids(&messages[index]);
        if tool_ids.is_empty() {
            if !has_tool_result(&messages[index]) {
                units.push(vec![messages[index].clone()]);
            }
            index += 1;
            continue;
        }

        let mut unit = vec![messages[index].clone()];
        let mut result_ids = HashSet::new();
        let mut next = index + 1;
        while next < messages.len() && has_tool_result(&messages[next]) {
            for block in &messages[next].content {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    result_ids.insert(tool_use_id.as_str());
                }
            }
            unit.push(messages[next].clone());
            next += 1;
        }
        if tool_ids.iter().all(|id| result_ids.contains(id.as_str())) {
            units.push(unit);
        }
        index = next;
    }
    units
}

fn tool_use_ids(message: &Message) -> Vec<String> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn has_tool_result(message: &Message) -> bool {
    message
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
}

fn truncate_message_to_tokens(message: &Message, max_tokens: usize) -> Option<Message> {
    let ContentBlock::Text { text } = message.content.first()? else {
        return None;
    };
    let max_bytes = max_tokens.saturating_sub(10).saturating_mul(4);
    if max_bytes == 0 {
        return None;
    }
    let mut end = 0;
    for (offset, character) in text.char_indices() {
        let candidate = offset + character.len_utf8();
        if candidate > max_bytes {
            break;
        }
        end = candidate;
    }
    (end > 0).then(|| Message::user(text[..end].to_owned()))
}

pub(crate) fn build_request_messages(
    system_prompt: String,
    history: &[Message],
    effective_user_message: String,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(Message::system(system_prompt));
    messages.extend_from_slice(history);
    messages.push(Message::user(effective_user_message));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(message: &Message) -> &str {
        match &message.content[0] {
            ContentBlock::Text { text } => text,
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn token_budget_selects_more_than_six_messages_in_order() {
        let history = (0_usize..20)
            .map(|index| {
                if index.is_multiple_of(2) {
                    Message::user(format!("user-{index}"))
                } else {
                    Message::assistant(format!("assistant-{index}"))
                }
            })
            .collect::<Vec<_>>();
        let selected = select_text_history(&history, 1_000);
        assert_eq!(selected.len(), 20);
        assert_eq!(text_of(&selected[0]), "user-0");
        assert_eq!(text_of(&selected[19]), "assistant-19");
    }

    #[test]
    fn selection_stops_by_tokens_and_preserves_newest() {
        let history = vec![
            Message::user("old"),
            Message::assistant("x".repeat(80)),
            Message::user("new"),
        ];
        let selected = select_text_history(&history, 21);
        assert_eq!(selected.len(), 1);
        assert_eq!(text_of(&selected[0]), "new");
    }

    #[test]
    fn tool_groups_are_atomic() {
        let tool_use = Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolUse {
                    id: "one".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "two".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
            ],
        };
        let history = vec![
            Message::user("before"),
            tool_use,
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_use_id: "one".into(),
                        content: "a".into(),
                        is_error: false,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "two".into(),
                        content: "b".into(),
                        is_error: false,
                    },
                ],
            },
        ];
        assert_eq!(select_text_history(&history, 1_000).len(), 3);
        assert!(select_text_history(&history, 20).is_empty());
    }

    #[test]
    fn oversized_utf8_user_is_truncated_safely() {
        let selected = select_text_history(&[Message::user("界".repeat(100))], 20);
        assert_eq!(selected.len(), 1);
        assert!(text_of(&selected[0]).len() <= 40);
        assert!(text_of(&selected[0]).is_char_boundary(text_of(&selected[0]).len()));
        assert!(selected[0].estimate_tokens() <= 20);
    }

    #[test]
    fn zero_budget_returns_no_history() {
        assert!(select_text_history(&[Message::user("anything")], 0).is_empty());
    }

    #[test]
    fn restored_payloads_are_not_replayed() {
        let history = vec![Message::user(
            "<memory-context>secret</memory-context>\nraw\n<skills>old</skills>",
        )];
        let selected = select_text_history(&history, 100);
        assert_eq!(text_of(&selected[0]).trim(), "raw");
        assert!(!text_of(&selected[0]).contains("secret"));
    }

    #[test]
    fn request_contains_system_prefix_and_user_message_with_full_history() {
        let history = vec![
            Message::system("old prefix that must not be replayed"),
            Message::user("raw prior user"),
            Message::assistant("raw prior assistant"),
        ];

        let messages = build_request_messages(
            "current prefix".into(),
            &history,
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user".into(),
        );

        // Current impl includes all history: 1 new system + 3 history + 1 user = 5 messages.
        assert_eq!(messages.len(), 5);
        assert_eq!(text_of(&messages[0]), "current prefix");
        assert_eq!(
            text_of(messages.last().unwrap()),
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user"
        );
        assert!(messages.iter().any(|m| text_of(m).contains("old prefix")));
    }
}
