//! Text helpers and size constants shared across the daemon turn pipeline.
//!
//! Originally duplicated between `chat.rs` and `daemon_turn.rs`. Colocated here
//! so both the orchestrator and the handler can use them without copies.

use fabric::{ContentBlock, Message};

// ── Size constants ──────────────────────────────────────────────────────────

pub(crate) const MAX_ACTIVATED_SKILL_CHARS: usize = 12 * 1024;
pub(crate) const MAX_ACTIVATED_SKILLS_TOTAL_CHARS: usize = 24 * 1024;
pub(crate) const MAX_RECALLED_FACT_CHARS: usize = 2 * 1024;
pub(crate) const MAX_RECALL_TOTAL_CHARS: usize = 8 * 1024;
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

pub(crate) fn append_bounded_text(
    target: &mut String,
    value: &str,
    per_item: usize,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    let bounded = truncate_chars(value, per_item.min(*remaining));
    *remaining = (*remaining).saturating_sub(bounded.chars().count());
    target.push_str(&bounded);
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
