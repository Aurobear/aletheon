use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info};

use fabric::{Clock, LlmProvider, Message, Role};
use mnemosyne::runtime::{AdvancedCompressor, ContextBudgetPlan};

/// Process-local working context for one canonical Session.
///
/// Durable history belongs to `SessionAppendStore`; this cache deliberately
/// owns no journal or event vocabulary. Bootstrap and compatibility services
/// hydrate it from the canonical Session/Turn/Item projection.
pub struct SessionManager {
    pub session_id: String,
    messages: Vec<Message>,
    compressor: AdvancedCompressor,
}

impl SessionManager {
    pub async fn new(
        _data_dir: &Path,
        session_id: String,
        max_tokens: usize,
        _clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        Ok(Self {
            session_id,
            messages: Vec::new(),
            compressor: AdvancedCompressor::new(
                (max_tokens as f64 * 0.25) as usize,
                4_000,
                max_tokens,
            ),
        })
    }

    /// Replace the process-local projection with canonical replay output.
    pub fn restore_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    pub async fn push_user(&mut self, content: &str) {
        self.messages.push(Message::user(content));
        debug!(
            len = content.len(),
            "Pushed user message into session projection"
        );
    }

    pub async fn push_assistant(&mut self, content: &str) {
        self.messages.push(Message::assistant(content));
        debug!(
            len = content.len(),
            "Pushed assistant message into session projection"
        );
    }

    pub fn push_system(&mut self, content: &str) {
        self.messages.push(Message::system(content));
        debug!(
            len = content.len(),
            "Pushed system message into session projection"
        );
    }

    pub async fn push_message(&mut self, message: Message) {
        debug!(role = ?message.role, blocks = message.content.len(), "Pushed message into session projection");
        self.messages.push(message);
    }

    pub fn history(&self) -> &[Message] {
        &self.messages
    }

    pub fn turn_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| matches!(message.role, Role::User))
            .count()
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Clear only the working projection. The compatibility service creates a
    /// fresh canonical Session rather than rewriting append-only history.
    pub async fn clear_history(&mut self) -> Result<()> {
        self.messages.clear();
        info!(session_id = %self.session_id, "Session working projection cleared");
        Ok(())
    }

    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(Message::estimate_tokens).sum()
    }

    pub fn projected_tokens(&self, pending_user: &str) -> usize {
        self.estimate_tokens()
            .saturating_add(Message::user(pending_user).estimate_tokens())
    }

    pub fn compaction_lineage(&self) -> &[mnemosyne::runtime::CompactionLineage] {
        self.compressor.lineage()
    }

    pub fn inherit_compaction_lineage(
        &mut self,
        lineage: &[mnemosyne::runtime::CompactionLineage],
    ) {
        self.compressor.inherit_lineage(lineage);
    }

    pub fn compaction_needed_for(&self, plan: &ContextBudgetPlan) -> bool {
        self.estimate_tokens() >= plan.soft_watermark && self.compressor.can_compact(&self.messages)
    }

    pub fn compaction_needed(&self) -> bool {
        self.compressor.should_compact(&self.messages)
    }

    /// Check if tokens exceed a custom threshold fraction (0.0-1.0).
    pub fn exceeds_threshold(&self, fraction: f64) -> bool {
        self.compressor.exceeds_threshold(&self.messages, fraction)
    }

    pub async fn compact_if_needed(&mut self, llm: &dyn LlmProvider) -> Result<bool> {
        self.run_compaction(llm, false).await
    }

    pub async fn force_compact(&mut self, llm: &dyn LlmProvider) -> Result<bool> {
        if self.messages.len() <= 2 {
            return Ok(false);
        }
        self.run_compaction(llm, true).await
    }

    /// Run the shared compaction engine against a per-turn target. The second
    /// attempt retains a smaller verbatim tail but uses the same validation and
    /// atomic commit path.
    pub async fn compact_to_budget(
        &mut self,
        llm: &dyn LlmProvider,
        plan: &ContextBudgetPlan,
        aggressive_retry: bool,
    ) -> Result<bool> {
        self.compressor
            .configure_budget(plan.history_budget, aggressive_retry);
        self.force_compact(llm).await
    }

    async fn run_compaction(&mut self, llm: &dyn LlmProvider, force: bool) -> Result<bool> {
        let before = self.messages.len();
        let compacted = if force {
            self.compressor.force_compact(&mut self.messages, llm).await
        } else {
            self.compressor.maybe_compact(&mut self.messages, llm).await
        }?;
        if compacted {
            info!(
                before,
                after = self.messages.len(),
                "Session projection compacted"
            );
        }
        Ok(compacted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fabric::message::is_tool_message;
    use fabric::{ContentBlock, LlmResponse, LlmStream, StopReason, ToolDefinition, Usage};
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    struct StubLlm;

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

    #[async_trait]
    impl LlmProvider for StubLlm {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
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
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("streaming not implemented")
        }

        fn name(&self) -> &str {
            "stub"
        }

        fn max_context_length(&self) -> usize {
            1_000
        }
    }

    struct FailingLlm;

    #[async_trait]
    impl LlmProvider for FailingLlm {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            anyhow::bail!("summary failed")
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            anyhow::bail!("streaming not implemented")
        }

        fn name(&self) -> &str {
            "failing"
        }

        fn max_context_length(&self) -> usize {
            1_000
        }
    }

    #[tokio::test]
    async fn canonical_replay_can_hydrate_working_projection() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path(), "session".into(), 1_000, test_clock())
            .await
            .unwrap();
        manager.restore_messages(vec![
            Message::user("restored"),
            Message::assistant("answer"),
        ]);
        assert_eq!(manager.turn_count(), 1);
        assert_eq!(manager.message_count(), 2);
    }

    #[tokio::test]
    async fn clear_drops_only_working_projection() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path(), "session".into(), 1_000, test_clock())
            .await
            .unwrap();
        manager.push_user("old context").await;
        manager.clear_history().await.unwrap();
        assert!(manager.history().is_empty());
    }

    #[tokio::test]
    async fn compaction_error_is_propagated() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path(), "fails".into(), 1_000, test_clock())
            .await
            .unwrap();
        for index in 0..8 {
            manager
                .push_user(&format!("request {index} {}", "中".repeat(200)))
                .await;
            manager
                .push_assistant(&format!("answer {index} {}", "🙂".repeat(200)))
                .await;
        }
        let error = manager.force_compact(&FailingLlm).await.unwrap_err();
        assert!(error.to_string().contains("summary failed"));
    }

    #[tokio::test]
    async fn compaction_tail_never_starts_with_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path(), "session".into(), 1_000, test_clock())
            .await
            .unwrap();
        for index in 0..12 {
            manager
                .push_message(Message {
                    role: Role::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: format!("assistant {index} {}", "x".repeat(400)),
                        },
                        ContentBlock::ToolUse {
                            id: format!("tool-{index}"),
                            name: "fixture".into(),
                            input: serde_json::Value::Null,
                        },
                    ],
                })
                .await;
            manager
                .push_message(Message::tool_result(
                    format!("tool-{index}"),
                    "y".repeat(400),
                    false,
                ))
                .await;
            manager
                .push_user(&format!("user {index} {}", "z".repeat(400)))
                .await;
        }
        assert!(manager.compact_if_needed(&StubLlm).await.unwrap());
        let first = manager
            .history()
            .iter()
            .find(|message| !matches!(message.role, Role::System));
        assert!(first.is_none_or(|message| !is_tool_message(message)));
    }
}
