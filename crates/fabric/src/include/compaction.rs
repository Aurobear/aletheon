//! Shared context-compaction interface and pruning helpers.
//!
//! `CompactorTrait` is the shared contract implemented by memory-compaction
//! strategies (e.g. `mnemosyne::runtime::AdvancedCompressor`) and consumed by
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

/// Truncate a string to a byte budget without splitting a UTF-8 code point.
pub fn truncate_utf8_bytes(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

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

    /// Rich compaction with guardrails (C1). The default bridges to
    /// `maybe_compact`, reporting a `TailKeep` outcome without eviction or
    /// failure classification — implementors override this to apply
    /// degenerate-summary detection, strategy selection and eviction. The
    /// `strategy` argument is advisory; the default ignores it.
    fn maybe_compact_v2<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
        strategy: CompactionStrategy,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<CompactionOutcome>> + Send + 'a>>
    {
        let _ = strategy;
        Box::pin(async move {
            let tokens_before: usize = messages.iter().map(|m| m.estimate_tokens()).sum();
            let applied = self.maybe_compact(messages, llm).await?;
            let tokens_after: usize = messages.iter().map(|m| m.estimate_tokens()).sum();
            Ok(CompactionOutcome {
                strategy: CompactionStrategy::TailKeep,
                applied,
                tokens_before,
                tokens_after,
                evicted: Vec::new(),
                failure: None,
            })
        })
    }
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
                        "[Tool result: {char_count} chars, {line_count} lines. Preview: {first_line}]"
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
                                truncate_utf8_bytes(s, 200);
                                s.push_str(&format!("... [truncated from {original_len} bytes]"));
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// C1: compaction guardrails (strategy, outcome, degenerate detection,
// tool-pair-safe tail cut). See docs/plans/grok/exec/C1-compaction.md.
// ---------------------------------------------------------------------------

/// Compaction strategy selector. TailKeep is the current AdvancedCompressor
/// behavior; FullReplace and PromoteToMemory are added surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompactionStrategy {
    /// Keep head + recent tail, drop the middle (current behavior).
    TailKeep,
    /// Summarize the whole session into a prefix + recent tail.
    FullReplace,
    /// Promote evictable segments to Mnemosyne, then remove them.
    PromoteToMemory,
}

/// Failure / degradation reason. When set, `messages` is left unchanged
/// (fail-safe: better an over-long context than silently dropped content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionFailure {
    /// LLM returned a degenerate summary (too short / empty / repetitive).
    DegenerateSummary { reason: String },
    /// Session too short to summarize meaningfully.
    TooShortToSummarize,
    /// Summarization LLM call failed.
    SamplerError { detail: String },
}

/// Rich result of a compaction attempt (replaces a bare bool).
#[derive(Debug, Clone)]
pub struct CompactionOutcome {
    pub strategy: CompactionStrategy,
    pub applied: bool,
    pub tokens_before: usize,
    pub tokens_after: usize,
    /// Messages evicted from the main buffer (promotion candidates).
    pub evicted: Vec<Message>,
    pub failure: Option<CompactionFailure>,
}

/// Minimum seed length below which a session is `TooShortToSummarize`.
pub const MIN_SUMMARY_SEED_CHARS: usize = 200;

/// Degenerate-summary detector (aligned with Grok `is_degenerate_summary`
/// semantics). Empty, too short, or mostly-repeated lines all count.
pub fn is_degenerate_summary(summary: &str) -> bool {
    let trimmed = summary.trim();
    trimmed.is_empty() || trimmed.chars().count() < 40 || is_mostly_repetition(trimmed)
}

fn is_mostly_repetition(s: &str) -> bool {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 3 {
        return false;
    }
    let unique: std::collections::HashSet<&str> = lines.iter().copied().collect();
    // Unique lines < 1/3 of total -> treat as repetition.
    unique.len() * 3 < lines.len()
}

/// Compute a tail-keep cut point that never splits a tool_use / tool_result
/// pair. Returns the largest index `cut <= keep_from` such that every known
/// `ToolUse`/`ToolResult` pair is wholly retained or wholly discarded.
/// Pre-existing orphan results do not force an unrelated cut to zero.
pub fn safe_tail_cut(messages: &[Message], keep_from: usize) -> usize {
    let mut cut = keep_from.min(messages.len());
    while cut > 0 && splits_tool_pair(messages, cut) {
        cut -= 1;
    }
    cut
}

/// True if cutting at `cut` leaves an orphan `ToolResult` in the tail whose
/// `ToolUse` is not present in the tail.
fn splits_tool_pair(messages: &[Message], cut: usize) -> bool {
    let tail = &messages[cut..];
    let all_tool_use_ids: std::collections::HashSet<&str> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut tool_use_ids = std::collections::HashSet::new();
    for msg in tail {
        for block in &msg.content {
            if let ContentBlock::ToolUse { id, .. } = block {
                tool_use_ids.insert(id.as_str());
            }
        }
    }
    for msg in tail {
        for block in &msg.content {
            if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                // A pre-existing orphan is malformed history, but the cut did
                // not create it. Only retreat when the matching call exists in
                // the discarded prefix and would therefore be split away.
                if all_tool_use_ids.contains(tool_use_id.as_str())
                    && !tool_use_ids.contains(tool_use_id.as_str())
                {
                    return true;
                }
            }
        }
    }
    false
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

    #[test]
    fn truncate_tool_call_args_is_utf8_safe() {
        let original = format!("{}{}", "中".repeat(67), "🙂".repeat(20));
        let mut messages = vec![Message {
            role: crate::Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "test".to_string(),
                name: "bash_exec".to_string(),
                input: serde_json::json!({ "command": original }),
            }],
        }];

        truncate_tool_call_args(&mut messages);

        let ContentBlock::ToolUse { input, .. } = &messages[0].content[0] else {
            panic!("expected tool use");
        };
        let command = input["command"].as_str().unwrap();
        let prefix = command.split("... [truncated").next().unwrap();
        assert!(prefix.len() <= 200);
        assert!(prefix.is_char_boundary(prefix.len()));
        assert!(command.contains("truncated from 281 bytes"));
    }

    #[test]
    fn truncate_utf8_bytes_preserves_under_budget_values() {
        let mut value = "中文🙂".to_string();
        let original = value.clone();
        let budget = value.len();
        truncate_utf8_bytes(&mut value, budget);
        assert_eq!(value, original);
    }
}

#[cfg(test)]
mod guardrail_tests {
    use super::*;
    use crate::message::{ContentBlock, Message, Role};

    fn tool_use(id: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: id.to_string(),
                name: "bash".to_string(),
                input: serde_json::Value::Null,
            }],
        }
    }

    fn tool_result(tool_use_id: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: "out".to_string(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn degenerate_summary_detects_empty_and_short() {
        assert!(is_degenerate_summary(""));
        assert!(is_degenerate_summary("   \n  "));
        assert!(is_degenerate_summary("too short"));
    }

    #[test]
    fn degenerate_summary_detects_repetition() {
        let repeated = "same line\nsame line\nsame line\nsame line\nsame line";
        assert!(is_degenerate_summary(repeated));
    }

    #[test]
    fn degenerate_summary_accepts_real_summary() {
        let good = "The user asked to refactor the auth module. We extracted the \
                    token validator, added tests, and fixed a race in the refresh path.";
        assert!(!is_degenerate_summary(good));
    }

    #[test]
    fn safe_tail_cut_retreats_to_keep_tool_pair() {
        // [0]=user, [1]=ToolUse(A), [2]=ToolResult(A), [3]=assistant
        let msgs = vec![
            Message::user("do it"),
            tool_use("A"),
            tool_result("A"),
            Message::assistant("done"),
        ];
        // keep_from=2 would orphan ToolResult(A) (its ToolUse is at index 1).
        // Must retreat to 1 to include the ToolUse.
        assert_eq!(safe_tail_cut(&msgs, 2), 1);
    }

    #[test]
    fn safe_tail_cut_keeps_when_no_split() {
        let msgs = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
        ];
        assert_eq!(safe_tail_cut(&msgs, 2), 2);
        assert_eq!(safe_tail_cut(&msgs, 0), 0);
    }

    #[test]
    fn safe_tail_cut_pair_fully_in_tail_is_stable() {
        // ToolUse and ToolResult both at/after keep_from -> no retreat.
        let msgs = vec![
            Message::user("x"),
            Message::assistant("y"),
            tool_use("B"),
            tool_result("B"),
        ];
        assert_eq!(safe_tail_cut(&msgs, 2), 2);
    }

    #[test]
    fn safe_tail_cut_does_not_over_retreat_for_preexisting_orphan_result() {
        let msgs = vec![
            Message::user("old"),
            Message::assistant("older"),
            tool_result("missing-call"),
            Message::assistant("tail"),
        ];
        assert_eq!(safe_tail_cut(&msgs, 2), 2);
    }

    #[test]
    fn safe_tail_cut_clamps_out_of_range() {
        let msgs = vec![Message::user("only")];
        assert_eq!(safe_tail_cut(&msgs, 99), 1);
    }

    #[test]
    fn safe_tail_cut_preserves_pair_invariant_for_arbitrary_bounded_sequences() {
        // Exhaust all length <= 6 sequences over text/use/result and every cut.
        // This is deterministic property coverage without a new test dependency.
        for len in 0..=6 {
            for mut shape in 0usize..3usize.pow(len) {
                let mut messages = Vec::with_capacity(len as usize);
                for index in 0..len {
                    let message = match shape % 3 {
                        0 => Message::user(format!("text-{index}")),
                        1 => tool_use(if index % 2 == 0 { "A" } else { "B" }),
                        _ => tool_result(if index % 2 == 0 { "A" } else { "B" }),
                    };
                    messages.push(message);
                    shape /= 3;
                }
                for keep_from in 0..=messages.len() + 1 {
                    let cut = safe_tail_cut(&messages, keep_from);
                    assert!(cut <= keep_from.min(messages.len()));
                    let all_uses: std::collections::HashSet<&str> = messages
                        .iter()
                        .flat_map(|message| message.content.iter())
                        .filter_map(|block| match block {
                            ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                            _ => None,
                        })
                        .collect();
                    let tail_uses: std::collections::HashSet<&str> = messages[cut..]
                        .iter()
                        .flat_map(|message| message.content.iter())
                        .filter_map(|block| match block {
                            ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                            _ => None,
                        })
                        .collect();
                    for result_id in messages[cut..]
                        .iter()
                        .flat_map(|message| message.content.iter())
                        .filter_map(|block| match block {
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                Some(tool_use_id.as_str())
                            }
                            _ => None,
                        })
                    {
                        assert!(
                            !all_uses.contains(result_id) || tail_uses.contains(result_id),
                            "pair split at cut={cut} keep_from={keep_from} messages={messages:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn min_summary_seed_chars_is_reasonable() {
        const { assert!(MIN_SUMMARY_SEED_CHARS >= 100) };
    }
}
