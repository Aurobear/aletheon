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

/// Advanced context compressor with token-budget tail protection
/// and iterative summary updates.
pub struct AdvancedCompressor {
    pub tail_config: TailProtectionConfig,
    pub target_summary_chars: usize,
    context_window_tokens: usize,
    previous_summary: Option<String>,
    template: SummaryTemplate,
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

        let cut = find_tail_cut(messages, &self.tail_config);
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

        let summary = self.generate_summary(&pruned_messages, llm).await?;

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
            "{}\n{}\n[End Summary]",
            SUMMARY_PREFIX, summary
        )));
        compacted.extend(protected_initial_user);
        compacted.extend_from_slice(tail_messages);

        self.previous_summary = Some(summary);

        let before = messages.len();
        *messages = compacted;
        info!(
            before = before,
            after = messages.len(),
            cut = cut,
            "Context compacted with token-budget tail protection"
        );

        Ok(true)
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
                return Ok(unchanged(None));
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
            return Ok(unchanged(None));
        }

        let old_messages = &messages[..cut];
        if old_messages.is_empty() {
            return Ok(unchanged(None));
        }

        let seed_chars: usize = old_messages
            .iter()
            .flat_map(|m| m.content.iter())
            .map(content_block_chars)
            .sum();
        if seed_chars < MIN_SUMMARY_SEED_CHARS {
            return Ok(unchanged(Some(CompactionFailure::TooShortToSummarize)));
        }

        let mut pruned = old_messages.to_vec();
        fabric::prune_tool_outputs(&mut pruned, 0);

        let summary = match self.generate_summary(&pruned, llm).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(unchanged(Some(CompactionFailure::SamplerError {
                    detail: e.to_string(),
                })));
            }
        };

        if is_degenerate_summary(&summary) {
            return Ok(unchanged(Some(CompactionFailure::DegenerateSummary {
                reason: format!(
                    "summary rejected ({} chars)",
                    summary.trim().chars().count()
                ),
            })));
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
            "{}\n{}\n[End Summary]",
            SUMMARY_PREFIX, summary
        )));
        compacted.extend(protected_initial_user);
        compacted.extend_from_slice(tail_messages);

        self.previous_summary = Some(summary);
        *messages = compacted;
        let tokens_after: usize = messages.iter().map(|m| m.estimate_tokens()).sum();

        Ok(CompactionOutcome {
            strategy,
            applied: true,
            tokens_before,
            tokens_after,
            evicted,
            failure: None,
        })
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
        let response = llm.complete(&prompt, &[]).await?;

        let summary: String = response
            .content
            .iter()
            .map(|c| match c {
                ContentBlock::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect();

        Ok(summary)
    }
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
        ContentBlock::Text { text } => text.len(),
        ContentBlock::ToolResult { content, .. } => content.len(),
        ContentBlock::ToolUse { input, .. } => input.to_string().len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut compressor = AdvancedCompressor::new(100, 200, 1_000);
        let llm = SimpleLlm;

        // Build many large messages to exceed threshold
        let mut messages = vec![Message::user("start")];
        for i in 0..10 {
            messages.push(Message::assistant(format!("response {}", "x".repeat(5000))));
            messages.push(Message::tool_result(
                format!("tool_{}", i),
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
        assert_eq!(c.last_summary(), Some("this is a summary"));
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
        let first = messages.clone();
        assert!(contains_text_user(&messages, "还是A吧"));
        assert_tail_is_tool_boundary_safe(&messages);

        assert!(c.force_compact(&mut messages, &llm).await.unwrap());
        assert_eq!(
            serde_json::to_value(&messages).unwrap(),
            serde_json::to_value(&first).unwrap()
        );
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
                format!("tool_{}", i),
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
        // SimpleLlm returns "this is a summary" (17 chars) -> degenerate.
        let mut c = AdvancedCompressor::new(100, 200, 1_000);
        let mut messages = oversized_messages();
        let snapshot = serde_json::to_value(&messages).unwrap();
        let outcome = c
            .maybe_compact_v2(&mut messages, &SimpleLlm, CompactionStrategy::TailKeep)
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
