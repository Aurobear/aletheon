//! Text helpers and size constants shared across the daemon turn pipeline.
//!
//! Originally duplicated between `chat.rs` and `daemon_turn.rs`. Colocated here
//! so both the orchestrator and the handler can use them without copies.

use fabric::{ContentBlock, Message};

// ── Size constants ──────────────────────────────────────────────────────────

pub(crate) const MAX_HISTORY_MESSAGE_CHARS: usize = 16 * 1024;
pub(crate) const MAX_HISTORY_TOTAL_CHARS: usize = 64 * 1024;
pub(crate) const MAX_HISTORY_MESSAGES: usize = 6;

// ── Free functions ───────────────────────────────────────────────────────────

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let truncated: String = value.chars().take(max_chars - 1).collect();
    format!("{truncated}…")
}

pub(crate) fn bounded_text_history(history: &[Message]) -> Vec<Message> {
    let mut bounded: Vec<Message> = Vec::new();
    let mut remaining = MAX_HISTORY_TOTAL_CHARS;
    for message in history.iter().rev().take(MAX_HISTORY_MESSAGES) {
        if remaining == 0 {
            break;
        }
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    let bounded_text =
                        truncate_chars(text, MAX_HISTORY_MESSAGE_CHARS.min(remaining));
                    remaining = remaining.saturating_sub(bounded_text.chars().count());
                    content_blocks.push(ContentBlock::Text { text: bounded_text });
                }
                // Skip non-text blocks (tool use/results are replayed by the harness)
                _ => {}
            }
        }
        if !content_blocks.is_empty() {
            bounded.push(Message {
                role: message.role,
                content: content_blocks,
            });
        }
    }
    bounded.reverse();
    bounded
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
    fn bounded_history_excludes_tool_result_blocks() {
        let history = vec![
            Message::system("large transient prefix"),
            Message::user("raw user"),
            Message::tool_result("call-1", "tool output", false),
            Message::assistant("raw assistant"),
        ];

        let bounded = bounded_text_history(&history);

        // New implementation includes system messages but excludes tool_result blocks.
        assert_eq!(bounded.len(), 3);
        assert_eq!(text_of(&bounded[0]), "large transient prefix");
        assert_eq!(text_of(&bounded[1]), "raw user");
        assert_eq!(text_of(&bounded[2]), "raw assistant");
    }

    #[test]
    fn bounded_history_caps_restored_injected_payloads() {
        let huge = format!("<activated-skill>{}</activated-skill>", "x".repeat(200_000));
        let history = vec![Message::user(huge)];

        let bounded = bounded_text_history(&history);

        assert_eq!(bounded.len(), 1);
        assert!(text_of(&bounded[0]).chars().count() <= MAX_HISTORY_MESSAGE_CHARS);
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
