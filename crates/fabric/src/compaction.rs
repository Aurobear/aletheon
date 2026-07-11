//! Shared context-compaction interface and pruning helpers.
//!
//! `CompactorTrait` is the shared contract implemented by memory-compaction
//! strategies (e.g. `mnemosyne::AdvancedCompressor`) and consumed by
//! cognitive harnesses (e.g. `cognit::ReActLoop`). Living in `fabric` lets
//! both sides depend on the interface without depending on each other,
//! avoiding a cyclic dependency between `cognit` and `mnemosyne`.
//!
//! `prune_tool_outputs` is a pure message transform (dedup / summarize /
//! truncate tool output) used ahead of summarization by compaction
//! strategies, and is likewise shared here so both `mnemosyne` and `corpus`
//! can use it without one depending on the other.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::pin::Pin;

use crate::message::{ContentBlock, Message};
use crate::LlmProvider;

/// Trait for context compaction into the message buffer.
/// Shared interface: lets cognitive harnesses depend on an abstract
/// compaction strategy without depending on a concrete memory crate.
pub trait CompactorTrait: Send {
    fn maybe_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>;

    fn force_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>;
}

/// Pre-summarization tool output pruning (Hermes 3-pass pattern).
///
/// 1. Deduplicate identical tool results (keep newest)
/// 2. Replace old tool results with one-line summaries
/// 3. Truncate large tool_call arguments
pub fn prune_tool_outputs(messages: &mut [Message], _tail_protected: usize) {
    deduplicate_tool_results(messages);
    summarize_old_tool_results(messages);
    truncate_tool_call_args(messages);
}

fn deduplicate_tool_results(messages: &mut [Message]) {
    let mut seen_hashes: HashMap<u64, usize> = HashMap::new();
    let mut to_clear: Vec<(usize, usize)> = Vec::new();

    for (msg_idx, msg) in messages.iter().enumerate() {
        for (block_idx, block) in msg.content.iter().enumerate() {
            if let ContentBlock::ToolResult { content, .. } = block {
                let mut hasher = DefaultHasher::new();
                content.hash(&mut hasher);
                let hash = hasher.finish();

                if let std::collections::hash_map::Entry::Vacant(e) = seen_hashes.entry(hash) {
                    e.insert(msg_idx);
                } else {
                    to_clear.push((msg_idx, block_idx));
                }
            }
        }
    }

    for (msg_idx, block_idx) in to_clear {
        if let Some(msg) = messages.get_mut(msg_idx) {
            if let Some(block) = msg.content.get_mut(block_idx) {
                *block = ContentBlock::Text {
                    text: "[Duplicate tool output \u{2014} same content as a more recent call]"
                        .to_string(),
                };
            }
        }
    }
}

fn summarize_old_tool_results(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        for block in msg.content.iter_mut() {
            if let ContentBlock::ToolResult { content, .. } = block {
                if content.len() > 200 {
                    let line_count = content.lines().count();
                    let char_count = content.len();
                    let first_line = content
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(80)
                        .collect::<String>();
                    *content = format!(
                        "[Tool result: {} chars, {} lines. Preview: {}]",
                        char_count, line_count, first_line
                    );
                }
            }
        }
    }
}

fn truncate_tool_call_args(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if msg.role != crate::Role::Assistant {
            continue;
        }
        for block in msg.content.iter_mut() {
            #[allow(clippy::collapsible_match)]
            if let ContentBlock::ToolUse { input, .. } = block {
                if let serde_json::Value::Object(map) = input {
                    for (_key, value) in map.iter_mut() {
                        if let serde_json::Value::String(s) = value {
                            if s.len() > 200 {
                                let original_len = s.len();
                                s.truncate(200);
                                s.push_str(&format!("... [truncated from {} chars]", original_len));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_result(content: &str) -> Message {
        Message {
            role: crate::Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "test".to_string(),
                content: content.to_string(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn test_summarize_large_results() {
        let large_content = "x".repeat(500);
        let mut messages = vec![make_tool_result(&large_content)];
        summarize_old_tool_results(&mut messages);
        if let ContentBlock::ToolResult { content, .. } = &messages[0].content[0] {
            assert!(content.contains("[Tool result:"));
            assert!(content.len() < large_content.len());
        }
    }

    #[test]
    fn test_truncate_tool_call_args() {
        let mut messages = vec![Message {
            role: crate::Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "test".to_string(),
                name: "bash_exec".to_string(),
                input: serde_json::json!({ "command": "x".repeat(500) }),
            }],
        }];
        truncate_tool_call_args(&mut messages);
        if let ContentBlock::ToolUse { input, .. } = &messages[0].content[0] {
            let cmd = input["command"].as_str().unwrap();
            assert!(cmd.len() < 500);
            assert!(cmd.contains("truncated"));
        }
    }
}
