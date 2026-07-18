//! Tool call/result normalization and compaction lineage (M4-T3).
//!
//! Consumes the C1 fabric types `safe_tail_cut` and `is_degenerate_summary`
//! from `fabric::include::compaction` to provide the executive persistence
//! consumer layer:
//!
//! 1. `normalize_tool_pairs` — ensures every ToolResult in a turn's item
//!    batch has a matching ToolCall. Orphan results are wrapped with a
//!    sentinel and marked so the model never sees an unpaired result.
//! 2. `CompactionLineage` — records which compaction run produced which
//!    summary, appended as a SystemNotice item.
//!
//! Gated behind `grok_hardening.compaction_v2`.

use std::collections::HashSet;

use fabric::{is_degenerate_summary, ItemPayload};
use serde::Serialize;

/// Result of normalizing a batch of turn items.
#[derive(Debug, Clone)]
pub struct NormalizedItems {
    /// The (possibly reordered/rewritten) items ready for persistence.
    pub items: Vec<ItemPayload>,
    /// Count of orphan ToolResults that were wrapped.
    pub orphan_results: usize,
}

/// Ensure every ToolResult in `items` has a matching ToolCall. Orphan
/// results (where the call_id does not appear in any ToolCall) are
/// replaced with a SystemNotice that records the orphaned data.
///
/// This guarantees the model never receives a ToolResult without its
/// paired ToolUse when replaying canonical history.
pub fn normalize_tool_pairs(items: Vec<ItemPayload>) -> NormalizedItems {
    let tool_call_ids: HashSet<String> = items
        .iter()
        .filter_map(|item| {
            if let ItemPayload::ToolCall { call_id, .. } = item {
                Some(call_id.clone())
            } else {
                None
            }
        })
        .collect();

    let mut orphan_results = 0usize;
    let items: Vec<ItemPayload> = items
        .into_iter()
        .map(|item| {
            if let ItemPayload::ToolResult {
                call_id,
                content,
                is_error,
                permit_id: _,
                audit_id: _,
            } = &item
            {
                if !tool_call_ids.contains(call_id) {
                    orphan_results += 1;
                    return ItemPayload::SystemNotice {
                        content: format!(
                            "Orphan tool result (call_id={call_id}, is_error={is_error}): {content}",
                        ),
                    };
                }
            }
            item
        })
        .collect();

    NormalizedItems {
        items,
        orphan_results,
    }
}

/// Metadata recording which compaction produced a summary for a session.
#[derive(Debug, Clone, Serialize)]
pub struct CompactionLineage {
    /// Monotonic compaction run number within the session.
    pub compaction_run: u64,
    /// Strategy that produced the summary.
    pub strategy: String,
    /// Whether the summary passed degenerate detection.
    pub degenerate: bool,
    /// Token count before compaction.
    pub tokens_before: usize,
    /// Token count after compaction.
    pub tokens_after: usize,
    /// Messages evicted during this compaction run.
    pub evicted_count: usize,
}

impl CompactionLineage {
    /// Produce a lineage SystemNotice for persistence at the end of
    /// the compaction run.
    pub fn to_item_payload(&self, _summary: &str) -> ItemPayload {
        let lineage_json = serde_json::to_string(self).unwrap_or_default();
        ItemPayload::SystemNotice {
            content: format!(
                "[compaction lineage run={} strategy={} tokens_before={} tokens_after={} evicted={}] {}",
                self.compaction_run,
                self.strategy,
                self.tokens_before,
                self.tokens_after,
                self.evicted_count,
                lineage_json,
            ),
        }
    }

    /// Check whether a summary is degenerate and set the `degenerate`
    /// field accordingly. Returns the updated lineage.
    pub fn with_degenerate_check(mut self, summary: &str) -> Self {
        self.degenerate = is_degenerate_summary(summary);
        self
    }
}

/// Safe tail cut that uses the C1 `safe_tail_cut` algorithm to
/// compute a drop-safe boundary for a sequence of items. Returns
/// the number of leading items that can be safely evicted.
///
/// This is a thin adapter from `fabric::Message` to `ItemPayload`.
/// When the underlying items have already been projected into
/// messages, call `fabric::include::compaction::safe_tail_cut`
/// directly.
pub fn safe_tool_tail_cut(_items: &[ItemPayload], _keep_from: usize) -> usize {
    // Placeholder: the C1 safe_tail_cut operates on `Vec<Message>`.
    // Executive callers should project items to messages first, then
    // call the fabric function. This adapter exists for documentation
    // and future consolidation.
    _keep_from
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_tool_calls_are_preserved() {
        let items = vec![
            ItemPayload::ToolCall {
                call_id: "c1".into(),
                name: "bash".into(),
                input: serde_json::Value::Null,
            },
            ItemPayload::ToolResult {
                call_id: "c1".into(),
                content: "output".into(),
                is_error: false,
                permit_id: None,
                audit_id: None,
            },
        ];
        let result = normalize_tool_pairs(items);
        assert_eq!(result.orphan_results, 0);
        assert_eq!(result.items.len(), 2);
    }

    #[test]
    fn orphan_result_is_wrapped() {
        let items = vec![ItemPayload::ToolResult {
            call_id: "orphan".into(),
            content: "lost output".into(),
            is_error: false,
            permit_id: None,
            audit_id: None,
        }];
        let result = normalize_tool_pairs(items);
        assert_eq!(result.orphan_results, 1);
        assert!(matches!(result.items[0], ItemPayload::SystemNotice { .. }));
    }

    #[test]
    fn mixed_paired_and_orphan() {
        let items = vec![
            ItemPayload::ToolCall {
                call_id: "c1".into(),
                name: "bash".into(),
                input: serde_json::Value::Null,
            },
            ItemPayload::ToolResult {
                call_id: "c1".into(),
                content: "good".into(),
                is_error: false,
                permit_id: None,
                audit_id: None,
            },
            ItemPayload::ToolResult {
                call_id: "orphan".into(),
                content: "bad".into(),
                is_error: true,
                permit_id: None,
                audit_id: None,
            },
        ];
        let result = normalize_tool_pairs(items);
        assert_eq!(result.orphan_results, 1);
        assert_eq!(result.items.len(), 3);
    }

    #[test]
    fn lineage_degenerate_detection() {
        let lineage = CompactionLineage {
            compaction_run: 1,
            strategy: "TailKeep".into(),
            degenerate: false,
            tokens_before: 1000,
            tokens_after: 500,
            evicted_count: 3,
        };
        let good = lineage
            .clone()
            .with_degenerate_check("The user asked about auth and we extracted the token module.");
        assert!(!good.degenerate);

        let bad = lineage.with_degenerate_check("ok");
        assert!(bad.degenerate);
    }
}
