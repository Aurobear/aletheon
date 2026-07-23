pub mod budget;
pub mod tail;
pub mod template;

use anyhow::Result;
use tracing::info;

use fabric::message::{ContentBlock, Message};
use fabric::CompactorTrait;
use fabric::LlmProvider;
use fabric::{
    is_degenerate_summary, safe_tail_cut, CompactionFailure, CompactionOutcome, CompactionStrategy,
    MIN_SUMMARY_SEED_CHARS,
};
use tail::{find_tail_cut, TailProtectionConfig};
use template::{SummaryTemplate, SUMMARY_PREFIX};

/// Immutable record of one compaction run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactionLineage {
    /// Monotonic run identifier.
    pub run_id: u64,
    /// Strategy used for this compaction.
    pub strategy: CompactionStrategy,
    /// Token count before compaction.
    pub tokens_before: usize,
    /// Token count after compaction.
    pub tokens_after: usize,
    /// Whether the compaction was forced (true) or automatic (false).
    pub forced: bool,
    /// Whether the compaction succeeded (true) or failed (false).
    pub applied: bool,
    /// Failure reason if not applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// Advanced context compressor with token-budget tail protection
/// and iterative summary updates.
pub struct AdvancedCompressor {
    pub tail_config: TailProtectionConfig,
    pub target_summary_chars: usize,
    context_window_tokens: usize,
    previous_summary: Option<String>,
    template: SummaryTemplate,
    run_counter: u64,
    lineage: Vec<CompactionLineage>,
}

impl AdvancedCompressor {
    pub fn new(
        tail_token_budget: usize,
        target_summary_chars: usize,
        context_window_tokens: usize,
    ) -> Self {
        Self {
            tail_config: TailProtectionConfig {
                tail_token_budget,
                ..Default::default()
            },
            target_summary_chars,
            context_window_tokens,
            previous_summary: None,
            template: SummaryTemplate,
            run_counter: 0,
            lineage: Vec::new(),
        }
    }

    /// Check if compaction is needed and perform it. Returns true if performed.
    pub async fn maybe_compact<L: LlmProvider + ?Sized>(
        &mut self,
        messages: &mut Vec<Message>,
        llm: &L,
    ) -> Result<bool> {
        self.compact_impl(messages, llm, false).await
    }

    /// Compact regardless of the token threshold (still tool-boundary-safe).
    pub async fn force_compact<L: LlmProvider + ?Sized>(
        &mut self,
        messages: &mut Vec<Message>,
        llm: &L,
    ) -> Result<bool> {
        self.compact_impl(messages, llm, true).await
    }

    /// The most recent summary produced by a compaction, if any.
    pub fn last_summary(&self) -> Option<&str> {
        self.previous_summary.as_deref()
    }

    /// All compaction lineage records, oldest first.
    pub fn lineage(&self) -> &[CompactionLineage] {
        &self.lineage
    }

    /// Carry validated receipts when a compacted working projection is
    /// materialized as a new immutable canonical session.
    pub fn inherit_lineage(&mut self, lineage: &[CompactionLineage]) {
        self.lineage = lineage.to_vec();
        self.run_counter = self.lineage.last().map_or(0, |entry| entry.run_id);
    }

    /// Read-only prediction used by lifecycle observers. It applies the same
    /// threshold and boundary guard as `maybe_compact` without mutating state.
    pub fn should_compact(&self, messages: &[Message]) -> bool {
        let total_tokens: usize = messages.iter().map(Message::estimate_tokens).sum();
        let threshold = (self.context_window_tokens as f64 * 0.8) as usize;
        if total_tokens < threshold {
            return false;
        }
        let cut = safe_tail_cut(messages, find_tail_cut(messages, &self.tail_config));
        cut > 0 && cut < messages.len()
    }

    pub fn can_compact(&self, messages: &[Message]) -> bool {
        let cut = safe_tail_cut(messages, find_tail_cut(messages, &self.tail_config));
        cut > 0 && cut < messages.len()
    }

    /// Adapt the protected tail to the actual history budget for this turn.
    pub fn configure_budget(&mut self, history_budget: usize, aggressive: bool) {
        let divisor = if aggressive { 5 } else { 3 };
        self.tail_config.tail_token_budget = (history_budget / divisor).max(256);
        self.context_window_tokens = history_budget;
    }

    /// Check if token count exceeds a given fraction of the context window.
    pub fn exceeds_threshold(&self, messages: &[Message], fraction: f64) -> bool {
        let total_tokens: usize = messages.iter().map(Message::estimate_tokens).sum();
        let threshold = (self.context_window_tokens as f64 * fraction) as usize;
        total_tokens > threshold
    }

    async fn compact_impl<L: LlmProvider + ?Sized>(
        &mut self,
        messages: &mut Vec<Message>,
        llm: &L,
        force: bool,
    ) -> Result<bool> {
        let total_tokens: usize = messages.iter().map(|m| m.estimate_tokens()).sum();

        if !force {
            let threshold = (self.context_window_tokens as f64 * 0.8) as usize;
            if total_tokens < threshold {
                return Ok(false);
            }
        }

        // Keep the legacy production compaction path on the same authoritative
        // C1 boundary guard as the rich v2 path.
        let cut = safe_tail_cut(messages, find_tail_cut(messages, &self.tail_config));
        if cut == 0 || cut >= messages.len() {
            return Ok(false);
        }

        // Split: everything before the cut is "old" (to be summarized),
        // everything from the cut onward is "tail" (preserved verbatim).
        let old_messages = &messages[..cut];
        let tail_messages = &messages[cut..];

        if old_messages.is_empty() {
            return Ok(false);
        }

        // Prune tool outputs before summarization
        let mut pruned_messages = old_messages.to_vec();
        fabric::prune_tool_outputs(&mut pruned_messages, 0);
        prune_long_text_for_summary(&mut pruned_messages);

        let summary = match self.generate_summary(&pruned_messages, llm).await {
            Ok(summary) => summary,
            Err(error) => return self.reject_compaction(total_tokens, force, error.to_string()),
        };

        // A request at index zero cannot be part of both sides of a contiguous
        // prefix/tail split. Preserve it explicitly after the summary so it
        // remains the verbatim active instruction instead of existing only in
        // generated summary text.
        let latest_text_user = messages.iter().rposition(|message| {
            message.role == fabric::Role::User
                && message
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::Text { .. }))
        });
        let protected_initial_user = (latest_text_user == Some(0)).then(|| messages[0].clone());

        let mut compacted = Vec::new();
        compacted.push(Message::system(format!(
            "{SUMMARY_PREFIX}\n{summary}\n[End Summary]"
        )));
        compacted.extend(protected_initial_user);
        compacted.extend_from_slice(tail_messages);

        let tokens_after: usize = compacted.iter().map(Message::estimate_tokens).sum();
        if let Err(error) = validate_structured_checkpoint(&summary) {
            return self.reject_compaction(total_tokens, force, error.to_string());
        }
        if tokens_after >= total_tokens {
            return self.reject_compaction(
                total_tokens,
                force,
                format!("no token benefit ({total_tokens} -> {tokens_after})"),
            );
        }
        if tokens_after > self.context_window_tokens {
            return self.reject_compaction(
                total_tokens,
                force,
                format!(
                    "result {tokens_after} exceeds history budget {}",
                    self.context_window_tokens
                ),
            );
        }

        self.previous_summary = Some(summary);
        let before = messages.len();
        *messages = compacted;
        info!(
            before = before,
            after = messages.len(),
            cut = cut,
            "Context compacted with token-budget tail protection"
        );

        self.run_counter += 1;
        self.lineage.push(CompactionLineage {
            run_id: self.run_counter,
            strategy: CompactionStrategy::TailKeep,
            tokens_before: total_tokens,
            tokens_after,
            forced: force,
            applied: true,
            failure: None,
        });

        Ok(true)
    }

    fn reject_compaction(
        &mut self,
        tokens_before: usize,
        forced: bool,
        reason: String,
    ) -> Result<bool> {
        self.run_counter += 1;
        self.lineage.push(CompactionLineage {
            run_id: self.run_counter,
            strategy: CompactionStrategy::TailKeep,
            tokens_before,
            tokens_after: tokens_before,
            forced,
            applied: false,
            failure: Some(reason.clone()),
        });
        anyhow::bail!("compaction validation failed: {reason}")
    }

    /// Guarded compaction returning a rich `CompactionOutcome` (C1).
    ///
    /// On any failure — session too short, sampler error, or a degenerate
    /// summary — the message buffer is left UNCHANGED (fail-safe: an over-long
    /// context is preferable to silently dropped content). TailKeep uses the
    /// proven find_tail_cut; FullReplace's aggressive cut is pulled back to a
    /// tool-pair boundary via `safe_tail_cut` so a pair is never split.
    async fn compact_v2_impl<L: LlmProvider + ?Sized>(
        &mut self,
        messages: &mut Vec<Message>,
        llm: &L,
        strategy: CompactionStrategy,
        force: bool,
    ) -> Result<CompactionOutcome> {
        let tokens_before: usize = messages.iter().map(|m| m.estimate_tokens()).sum();
        let unchanged = |failure: Option<CompactionFailure>| CompactionOutcome {
            strategy,
            applied: false,
            tokens_before,
            tokens_after: tokens_before,
            evicted: Vec::new(),
            failure,
        };

        if !force {
            let threshold = (self.context_window_tokens as f64 * 0.8) as usize;
            if tokens_before < threshold {
                return Ok(self.push_lineage(unchanged(None), force));
            }
        }

        // FullReplace summarizes everything but the final exchange, pulled back
        // to a tool-pair boundary via safe_tail_cut. TailKeep / PromoteToMemory
        // use find_tail_cut, which already guarantees the tail never begins on an
        // orphan tool_result (and does not over-retreat on pre-existing orphans).
        let cut = match strategy {
            CompactionStrategy::FullReplace => {
                safe_tail_cut(messages, messages.len().saturating_sub(2))
            }
            _ => find_tail_cut(messages, &self.tail_config),
        };
        if cut == 0 || cut >= messages.len() {
            return Ok(self.push_lineage(unchanged(None), force));
        }

        let old_messages = &messages[..cut];
        if old_messages.is_empty() {
            return Ok(self.push_lineage(unchanged(None), force));
        }

        let seed_chars: usize = old_messages
            .iter()
            .flat_map(|m| m.content.iter())
            .map(content_block_chars)
            .sum();
        if seed_chars < MIN_SUMMARY_SEED_CHARS {
            return Ok(self.push_lineage(
                unchanged(Some(CompactionFailure::TooShortToSummarize)),
                force,
            ));
        }

        let mut pruned = old_messages.to_vec();
        fabric::prune_tool_outputs(&mut pruned, 0);

        let summary = match self.generate_summary(&pruned, llm).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(self.push_lineage(
                    unchanged(Some(CompactionFailure::SamplerError {
                        detail: e.to_string(),
                    })),
                    force,
                ));
            }
        };

        if is_degenerate_summary(&summary) {
            return Ok(self.push_lineage(
                unchanged(Some(CompactionFailure::DegenerateSummary {
                    reason: format!(
                        "summary rejected ({} chars)",
                        summary.trim().chars().count()
                    ),
                })),
                force,
            ));
        }

        // Segments that will leave the main buffer — surfaced for promotion.
        // Every removed message is reported to the harness. The harness may
        // promote it through its bounded callback; the strategy controls the
        // compaction shape, not whether removal remains observable.
        let evicted = old_messages.to_vec();
        let tail_messages = &messages[cut..];

        let latest_text_user = messages.iter().rposition(|message| {
            message.role == fabric::Role::User
                && message
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::Text { .. }))
        });
        let protected_initial_user = (latest_text_user == Some(0)).then(|| messages[0].clone());

        let mut compacted = Vec::new();
        compacted.push(Message::system(format!(
            "{SUMMARY_PREFIX}\n{summary}\n[End Summary]"
        )));
        compacted.extend(protected_initial_user);
        compacted.extend_from_slice(tail_messages);

        self.previous_summary = Some(summary);
        *messages = compacted;
        let tokens_after: usize = messages.iter().map(|m| m.estimate_tokens()).sum();

        Ok(self.push_lineage(
            CompactionOutcome {
                strategy,
                applied: true,
                tokens_before,
                tokens_after,
                evicted,
                failure: None,
            },
            force,
        ))
    }

    /// Record a compaction lineage entry and return the outcome unchanged.
    fn push_lineage(&mut self, outcome: CompactionOutcome, forced: bool) -> CompactionOutcome {
        self.run_counter += 1;
        self.lineage.push(CompactionLineage {
            run_id: self.run_counter,
            strategy: outcome.strategy,
            tokens_before: outcome.tokens_before,
            tokens_after: outcome.tokens_after,
            forced,
            applied: outcome.applied,
            failure: outcome.failure.as_ref().map(|f| format!("{f:?}")),
        });
        outcome
    }

    async fn generate_summary<L: LlmProvider + ?Sized>(
        &self,
        messages: &[Message],
        llm: &L,
    ) -> Result<String> {
        let prompt_text = match &self.previous_summary {
            Some(prev) => self
                .template
                .render_iterative(prev, messages, self.target_summary_chars),
            None => self.template.render(messages, self.target_summary_chars),
        };

        let prompt = vec![Message::user(prompt_text)];
        let summary = match llm.complete(&prompt, &[]).await {
            Ok(response) => response
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
            Err(complete_error) => {
                // Some production gateways expose streaming inference only.
                // Compaction still uses the same provider boundary, but
                // aggregates text deltas into the validated checkpoint.
                let mut stream = llm.complete_stream(&prompt, &[]).await.map_err(|stream_error| {
                    anyhow::anyhow!(
                        "summary inference failed (complete: {complete_error:#}; stream: {stream_error:#})"
                    )
                })?;
                let mut text = String::new();
                while let Some(chunk) =
                    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
                {
                    if let fabric::StreamChunk::TextDelta { text: delta } = chunk? {
                        text.push_str(&delta);
                    }
                }
                text
            }
        };

        Ok(summary)
    }
}

const SUMMARY_TEXT_HEAD_CHARS: usize = 3_000;
const SUMMARY_TEXT_TAIL_CHARS: usize = 1_000;

/// Bound pathological logs or pasted payloads before the summarizer call while
/// retaining both the identifying prefix and the most recent evidence suffix.
/// This only changes the summarizer input; canonical history remains untouched.
fn prune_long_text_for_summary(messages: &mut [Message]) {
    for message in messages {
        for block in &mut message.content {
            let ContentBlock::Text { text } = block else {
                continue;
            };
            let char_count = text.chars().count();
            let keep = SUMMARY_TEXT_HEAD_CHARS + SUMMARY_TEXT_TAIL_CHARS;
            if char_count <= keep {
                continue;
            }
            let head: String = text.chars().take(SUMMARY_TEXT_HEAD_CHARS).collect();
            let tail: String = text
                .chars()
                .skip(char_count - SUMMARY_TEXT_TAIL_CHARS)
                .collect();
            *text = format!(
                "{head}\n\n[... deterministically elided {} characters for compaction ...]\n\n{tail}",
                char_count - keep
            );
        }
    }
}

fn validate_structured_checkpoint(summary: &str) -> Result<()> {
    const REQUIRED: [&str; 10] = [
        "## Active Task",
        "## Goal",
        "## Completed Actions",
        "## Active State",
        "## In Progress",
        "## Blocked",
        "## Key Decisions",
        "## Pending User Asks",
        "## Relevant Files",
        "## Remaining Work",
    ];
    let missing: Vec<&str> = REQUIRED
        .into_iter()
        .filter(|heading| !summary.contains(heading))
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "compaction checkpoint missing required sections: {}",
            missing.join(", ")
        );
    }
    if !summary.contains("## Critical Context") {
        anyhow::bail!("compaction checkpoint missing required section: ## Critical Context");
    }
    Ok(())
}

impl CompactorTrait for AdvancedCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>
    {
        Box::pin(self.maybe_compact(messages, llm))
    }

    fn force_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>
    {
        Box::pin(self.force_compact(messages, llm))
    }

    fn maybe_compact_v2<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
        strategy: CompactionStrategy,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<CompactionOutcome>> + Send + 'a>,
    > {
        Box::pin(self.compact_v2_impl(messages, llm, strategy, false))
    }
}

/// Approximate character weight of a content block, used to decide whether a
/// prefix is substantial enough to be worth summarizing.
fn content_block_chars(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text { text } => text.chars().count(),
        ContentBlock::ToolResult { content, .. } => content.chars().count(),
        ContentBlock::ToolUse { input, .. } => input.to_string().chars().count(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CHECKPOINT: &str = "\
## Active Task\ncontinue\n\
## Goal\ncomplete the task\n\
## Completed Actions\n- inspected state\n\
## Active State\nworkspace is active\n\
## In Progress\nimplementation\n\
## Blocked\nnone\n\
## Key Decisions\npreserve evidence\n\
## Pending User Asks\nfinish\n\
## Relevant Files\nsrc/lib.rs\n\
## Remaining Work\nvalidate\n\
## Critical Context\nconstraints remain";
    use async_trait::async_trait;
    use fabric::ToolDefinition;
    use fabric::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};

    #[test]
    fn test_new_compressor() {
        let compressor = AdvancedCompressor::new(20_000, 4_000, 128_000);
        assert_eq!(compressor.tail_config.tail_token_budget, 20_000);
        assert_eq!(compressor.target_summary_chars, 4_000);
        assert_eq!(compressor.context_window_tokens, 128_000);
        assert!(compressor.previous_summary.is_none());
    }

    #[test]
    fn aggressive_budget_retry_reduces_the_protected_tail() {
        let mut compressor = AdvancedCompressor::new(8_000, 2_000, 100_000);
        compressor.configure_budget(9_000, false);
        assert_eq!(compressor.tail_config.tail_token_budget, 3_000);
        compressor.configure_budget(9_000, true);
        assert_eq!(compressor.tail_config.tail_token_budget, 1_800);
    }

    #[test]
    fn deterministic_summary_pruning_preserves_head_tail_and_utf8() {
        let marker = "EXACT_REQUIREMENT";
        let suffix = "LATEST_ERROR";
        let mut messages = vec![Message::user(format!(
            "{marker}{}{}",
            "界".repeat(6_000),
            suffix
        ))];

        prune_long_text_for_summary(&mut messages);

        let ContentBlock::Text { text } = &messages[0].content[0] else {
            panic!("expected text block");
        };
        assert!(text.starts_with(marker));
        assert!(text.ends_with(suffix));
        assert!(text.contains("deterministically elided"));
        assert!(text.chars().count() < 4_200);
    }

    struct SimpleLlm;

    #[async_trait]
    impl LlmProvider for SimpleLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: VALID_CHECKPOINT.into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("mock(SimpleLlm): streaming not implemented")
        }
        fn name(&self) -> &str {
            "simple"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn test_compressor_actually_compacts() {
        let mut compressor = AdvancedCompressor::new(100, 200, 4_000);
        let llm = SimpleLlm;

        // Build many large messages to exceed threshold
        let mut messages = vec![Message::user("start")];
        for i in 0..10 {
            messages.push(Message::assistant(format!("response {}", "x".repeat(5000))));
            messages.push(Message::tool_result(
                format!("tool_{i}"),
                "y".repeat(5000),
                false,
            ));
        }

        let before = messages.len();
        let result = compressor.maybe_compact(&mut messages, &llm).await;
        assert!(result.is_ok(), "maybe_compact failed: {:?}", result.err());
        assert!(result.unwrap(), "compaction should have been performed");
        assert!(
            messages.len() < before,
            "messages should be fewer after compaction: {} -> {}",
            before,
            messages.len()
        );
        assert!(contains_text_user(&messages, "start"));
    }

    #[tokio::test]
    async fn force_compact_ignores_threshold_and_exposes_summary() {
        // context window huge so the normal threshold is NOT exceeded
        let mut c = AdvancedCompressor::new(50, 200, 10_000_000);
        let llm = SimpleLlm;
        let mut messages = vec![Message::user("start")];
        for i in 0..8 {
            messages.push(Message::assistant(format!("a{i} {}", "x".repeat(400))));
            messages.push(Message::user(format!("u{i} {}", "y".repeat(400))));
        }
        // maybe_compact would be a no-op (under threshold)
        assert!(!c.maybe_compact(&mut messages.clone(), &llm).await.unwrap());
        // force_compact compacts anyway and records the summary
        let did = c.force_compact(&mut messages, &llm).await.unwrap();
        assert!(did, "force_compact should compact regardless of threshold");
        assert_eq!(c.last_summary(), Some(VALID_CHECKPOINT));
    }

    #[tokio::test]
    async fn repeated_multibyte_compaction_preserves_latest_user_and_tool_boundary() {
        let mut c = AdvancedCompressor::new(20, 200, 1_000);
        let llm = SimpleLlm;
        let mut messages = vec![Message::user("旧任务")];
        for i in 0..8 {
            messages.push(Message {
                role: fabric::Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: format!("old-{i}"),
                    name: "bash_exec".into(),
                    input: serde_json::json!({"command": "读取机器人运控文档🧠".repeat(30)}),
                }],
            });
            messages.push(Message::tool_result(
                format!("old-{i}"),
                "多字节工具结果🧠".repeat(80),
                false,
            ));
            messages.push(Message::assistant("旧结论".repeat(40)));
        }
        messages.push(Message::user("还是A吧"));
        messages.push(Message {
            role: fabric::Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "latest".into(),
                name: "file_read".into(),
                input: serde_json::json!({"path": "机器人/运控/设计🧠.md"}),
            }],
        });
        messages.push(Message::tool_result("latest", "最新工具结果🧠", false));

        assert!(c.force_compact(&mut messages, &llm).await.unwrap());
        assert!(contains_text_user(&messages, "还是A吧"));
        assert_tail_is_tool_boundary_safe(&messages);

        let second_snapshot = serde_json::to_value(&messages).unwrap();
        let second = c.force_compact(&mut messages, &llm).await;
        if second.is_err() {
            assert_eq!(serde_json::to_value(&messages).unwrap(), second_snapshot);
        }
        assert!(contains_text_user(&messages, "还是A吧"));
        assert_tail_is_tool_boundary_safe(&messages);
    }

    fn contains_text_user(messages: &[Message], expected: &str) -> bool {
        messages.iter().any(|message| {
            message.role == fabric::Role::User
                && message
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::Text { text } if text == expected))
        })
    }

    fn assert_tail_is_tool_boundary_safe(messages: &[Message]) {
        let first_non_system = messages
            .iter()
            .find(|message| message.role != fabric::Role::System)
            .expect("compacted tail");
        assert!(
            !matches!(
                first_non_system.content.first(),
                Some(ContentBlock::ToolResult { .. })
            ),
            "compacted tail must not begin with an orphan tool result"
        );
    }

    /// Returns a real, non-degenerate summary.
    struct GoodLlm;
    #[async_trait]
    impl LlmProvider for GoodLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "The user refactored the auth module: extracted the token validator, \
                           added regression tests, and fixed a refresh-path race condition."
                        .into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("mock(GoodLlm): streaming not implemented")
        }
        fn name(&self) -> &str {
            "good"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    struct DegenerateLlm;
    #[async_trait]
    impl LlmProvider for DegenerateLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "this is a summary".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("mock(DegenerateLlm): streaming not implemented")
        }
        fn name(&self) -> &str {
            "degenerate"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    /// Fails every completion (simulates a sampler outage).
    struct ErrLlm;
    #[async_trait]
    impl LlmProvider for ErrLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            anyhow::bail!("upstream sampler unavailable")
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("mock(ErrLlm): streaming not implemented")
        }
        fn name(&self) -> &str {
            "err"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    fn oversized_messages() -> Vec<Message> {
        let mut messages = vec![Message::user("start")];
        for i in 0..10 {
            messages.push(Message::assistant(format!("response {}", "x".repeat(5000))));
            messages.push(Message::tool_result(
                format!("tool_{i}"),
                "y".repeat(5000),
                false,
            ));
        }
        messages
    }

    #[tokio::test]
    async fn v2_applies_with_good_summary_and_reports_outcome() {
        let mut c = AdvancedCompressor::new(100, 200, 1_000);
        let mut messages = oversized_messages();
        let before = messages.len();
        let outcome = c
            .maybe_compact_v2(&mut messages, &GoodLlm, CompactionStrategy::TailKeep)
            .await
            .unwrap();
        assert!(outcome.applied);
        assert!(outcome.failure.is_none());
        assert_eq!(outcome.strategy, CompactionStrategy::TailKeep);
        assert!(outcome.tokens_after <= outcome.tokens_before);
        assert!(messages.len() < before);
        assert!(c.last_summary().is_some());
    }

    #[tokio::test]
    async fn v2_rejects_degenerate_summary_and_keeps_messages() {
        // DegenerateLlm returns a very short free-form summary.
        let mut c = AdvancedCompressor::new(100, 200, 1_000);
        let mut messages = oversized_messages();
        let snapshot = serde_json::to_value(&messages).unwrap();
        let outcome = c
            .maybe_compact_v2(&mut messages, &DegenerateLlm, CompactionStrategy::TailKeep)
            .await
            .unwrap();
        assert!(!outcome.applied);
        assert!(matches!(
            outcome.failure,
            Some(CompactionFailure::DegenerateSummary { .. })
        ));
        // Fail-safe: buffer untouched.
        assert_eq!(serde_json::to_value(&messages).unwrap(), snapshot);
    }

    #[tokio::test]
    async fn v2_reports_sampler_error_and_keeps_messages() {
        let mut c = AdvancedCompressor::new(100, 200, 1_000);
        let mut messages = oversized_messages();
        let snapshot = serde_json::to_value(&messages).unwrap();
        let outcome = c
            .maybe_compact_v2(&mut messages, &ErrLlm, CompactionStrategy::TailKeep)
            .await
            .unwrap();
        assert!(!outcome.applied);
        assert!(matches!(
            outcome.failure,
            Some(CompactionFailure::SamplerError { .. })
        ));
        assert_eq!(serde_json::to_value(&messages).unwrap(), snapshot);
    }

    #[tokio::test]
    async fn v2_reports_too_short_and_keeps_messages() {
        // Force past the threshold with a tiny prefix so the summarization seed
        // is below MIN_SUMMARY_SEED_CHARS.
        let mut c = AdvancedCompressor::new(1_000_000, 200, 1_000);
        let mut messages = vec![Message::user("a"), Message::user("b"), Message::user("c")];
        let snapshot = serde_json::to_value(&messages).unwrap();
        let outcome = c
            .compact_v2_impl(
                &mut messages,
                &GoodLlm,
                CompactionStrategy::FullReplace,
                true,
            )
            .await
            .unwrap();
        assert!(!outcome.applied);
        assert!(matches!(
            outcome.failure,
            Some(CompactionFailure::TooShortToSummarize)
        ));
        assert_eq!(serde_json::to_value(&messages).unwrap(), snapshot);
    }

    #[tokio::test]
    async fn v2_summary_seed_limit_counts_unicode_characters_not_utf8_bytes() {
        let mut c = AdvancedCompressor::new(1_000_000, 200, 1_000);
        let mut messages = vec![
            Message::user("界".repeat(100)),
            Message::assistant("tail-a"),
            Message::user("tail-b"),
        ];
        let snapshot = serde_json::to_value(&messages).unwrap();
        let outcome = c
            .compact_v2_impl(
                &mut messages,
                &GoodLlm,
                CompactionStrategy::FullReplace,
                true,
            )
            .await
            .unwrap();
        assert!(!outcome.applied);
        assert!(matches!(
            outcome.failure,
            Some(CompactionFailure::TooShortToSummarize)
        ));
        assert_eq!(serde_json::to_value(&messages).unwrap(), snapshot);
    }

    #[tokio::test]
    async fn full_replace_applies_good_summary_and_preserves_recent_tail() {
        let mut compressor = AdvancedCompressor::new(100, 200, 1_000);
        let mut messages = (0..8)
            .map(|index| {
                if index % 2 == 0 {
                    Message::user(format!("request {index} {}", "x".repeat(2_000)))
                } else {
                    Message::assistant(format!("response {index} {}", "y".repeat(2_000)))
                }
            })
            .collect::<Vec<_>>();
        let latest = serde_json::to_value(messages.last().unwrap()).unwrap();
        let outcome = compressor
            .compact_v2_impl(
                &mut messages,
                &GoodLlm,
                CompactionStrategy::FullReplace,
                true,
            )
            .await
            .unwrap();

        assert!(outcome.applied);
        assert_eq!(outcome.strategy, CompactionStrategy::FullReplace);
        assert!(outcome.failure.is_none());
        assert!(!outcome.evicted.is_empty());
        assert_eq!(
            serde_json::to_value(messages.last().unwrap()).unwrap(),
            latest
        );
        assert!(matches!(messages[0].role, fabric::Role::System));
        assert!(outcome.tokens_after < outcome.tokens_before);
    }
}
