//! Rebuildable, process-local context cache for one canonical Session.

use crate::compaction::CompactionStrategy;
use ::contracts::{Clock, ContentBlock, LlmProvider, Message, Role};
use anyhow::Result;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, info};

/// Runtime-owned, implementation-neutral record of a context compaction.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextCompactionReceipt {
    pub run_id: u64,
    pub strategy: CompactionStrategy,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub forced: bool,
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// Narrow strategy port. Runtime owns the cache; Mnemosyne may implement how
/// an over-budget cache is summarized without becoming its owner.
pub trait ContextCompactor: Send + Sync {
    fn lineage(&self) -> &[ContextCompactionReceipt];
    fn inherit_lineage(&mut self, lineage: &[ContextCompactionReceipt]);
    fn can_compact(&self, messages: &[Message]) -> bool;
    fn should_compact(&self, messages: &[Message]) -> bool;
    fn exceeds_threshold(&self, messages: &[Message], fraction: f64) -> bool;
    fn configure_budget(&mut self, history_budget: usize, aggressive: bool);
    fn maybe_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>>;
    fn force_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>>;
}

pub trait ContextCompactorFactory: Send + Sync {
    fn create(&self, max_tokens: usize, threshold_percent: usize) -> Box<dyn ContextCompactor>;
}

/// Process-local working context for one canonical Session.
///
/// Durable history belongs to the Runtime journal/projection. This cache owns
/// neither a writer nor a Session constructor and can always be hydrated again.
pub struct ContextWorkingSet {
    pub session_id: String,
    messages: Vec<Message>,
    turn_count: usize,
    rewrite_version: u64,
    compactor: Box<dyn ContextCompactor>,
}

impl ContextWorkingSet {
    pub async fn new(
        _data_dir: &Path,
        session_id: String,
        _clock: Arc<dyn Clock>,
        compactor: Box<dyn ContextCompactor>,
    ) -> Result<Self> {
        Ok(Self {
            session_id,
            messages: Vec::new(),
            turn_count: 0,
            rewrite_version: 0,
            compactor,
        })
    }

    pub fn restore_messages(&mut self, messages: Vec<Message>) {
        self.turn_count = messages.iter().filter(|m| is_text_user_request(m)).count();
        self.messages = messages;
    }

    pub async fn push_user(&mut self, content: &str) {
        self.messages.push(Message::user(content));
        self.turn_count = self.turn_count.saturating_add(1);
        debug!(
            len = content.len(),
            "Pushed user message into context working set"
        );
    }

    pub async fn push_assistant(&mut self, content: &str) {
        self.messages.push(Message::assistant(content));
        debug!(
            len = content.len(),
            "Pushed assistant message into context working set"
        );
    }

    pub fn push_system(&mut self, content: &str) {
        self.messages.push(Message::system(content));
    }

    pub async fn push_message(&mut self, message: Message) {
        if is_text_user_request(&message) {
            self.turn_count = self.turn_count.saturating_add(1);
        }
        self.messages.push(message);
    }

    pub fn history(&self) -> &[Message] {
        &self.messages
    }
    pub fn turn_count(&self) -> usize {
        self.turn_count
    }
    pub fn rewrite_version(&self) -> u64 {
        self.rewrite_version
    }
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub async fn clear_history(&mut self) -> Result<()> {
        self.messages.clear();
        self.turn_count = 0;
        self.rewrite_version = self.rewrite_version.saturating_add(1);
        info!(session_id = %self.session_id, "Context working set cleared");
        Ok(())
    }

    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(Message::estimate_tokens).sum()
    }

    pub fn projected_tokens(&self, pending_user: &str) -> usize {
        self.estimate_tokens()
            .saturating_add(Message::user(pending_user).estimate_tokens())
    }

    pub fn compaction_lineage(&self) -> &[ContextCompactionReceipt] {
        self.compactor.lineage()
    }
    pub fn inherit_compaction_lineage(&mut self, lineage: &[ContextCompactionReceipt]) {
        self.compactor.inherit_lineage(lineage);
    }
    pub fn compaction_needed_for(&self, soft_watermark: u64) -> bool {
        u64::try_from(self.estimate_tokens()).unwrap_or(u64::MAX) >= soft_watermark
            && self.compactor.can_compact(&self.messages)
    }
    pub fn compaction_needed(&self) -> bool {
        self.compactor.should_compact(&self.messages)
    }
    pub fn exceeds_threshold(&self, fraction: f64) -> bool {
        self.compactor.exceeds_threshold(&self.messages, fraction)
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
    pub async fn compact_to_budget(
        &mut self,
        llm: &dyn LlmProvider,
        history_budget: u64,
        aggressive_retry: bool,
    ) -> Result<bool> {
        let budget = usize::try_from(history_budget)
            .map_err(|_| anyhow::anyhow!("history budget exceeds this platform's usize"))?;
        self.compactor.configure_budget(budget, aggressive_retry);
        self.force_compact(llm).await
    }

    async fn run_compaction(&mut self, llm: &dyn LlmProvider, force: bool) -> Result<bool> {
        let before = self.messages.len();
        let compacted = if force {
            self.compactor.force_compact(&mut self.messages, llm).await
        } else {
            self.compactor.maybe_compact(&mut self.messages, llm).await
        }?;
        if compacted {
            self.rewrite_version = self.rewrite_version.saturating_add(1);
            info!(
                before,
                after = self.messages.len(),
                "Context working set compacted"
            );
        }
        Ok(compacted)
    }
}

fn is_text_user_request(message: &Message) -> bool {
    message.role == Role::User
        && message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubClock;
    impl Clock for StubClock {
        fn wall_now(&self) -> ::contracts::WallTime {
            ::contracts::WallTime(0)
        }
        fn mono_now(&self) -> ::contracts::MonoTime {
            ::contracts::MonoTime(0)
        }
    }

    #[derive(Default)]
    struct StubCompactor {
        lineage: Vec<ContextCompactionReceipt>,
    }

    impl ContextCompactor for StubCompactor {
        fn lineage(&self) -> &[ContextCompactionReceipt] {
            &self.lineage
        }
        fn inherit_lineage(&mut self, lineage: &[ContextCompactionReceipt]) {
            self.lineage = lineage.to_vec();
        }
        fn can_compact(&self, messages: &[Message]) -> bool {
            messages.len() > 2
        }
        fn should_compact(&self, messages: &[Message]) -> bool {
            messages.len() > 2
        }
        fn exceeds_threshold(&self, messages: &[Message], _fraction: f64) -> bool {
            messages.len() > 2
        }
        fn configure_budget(&mut self, _history_budget: usize, _aggressive: bool) {}
        fn maybe_compact<'a>(
            &'a mut self,
            messages: &'a mut Vec<Message>,
            _llm: &'a dyn LlmProvider,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            Box::pin(async move {
                messages.truncate(2);
                Ok(true)
            })
        }
        fn force_compact<'a>(
            &'a mut self,
            messages: &'a mut Vec<Message>,
            llm: &'a dyn LlmProvider,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            self.maybe_compact(messages, llm)
        }
    }

    async fn working_set() -> ContextWorkingSet {
        ContextWorkingSet::new(
            Path::new("/unused"),
            "session-1".into(),
            Arc::new(StubClock),
            Box::new(StubCompactor::default()),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn canonical_replay_rebuilds_process_local_projection() {
        let mut set = working_set().await;
        set.restore_messages(vec![Message::user("one"), Message::assistant("two")]);
        assert_eq!(set.turn_count(), 1);
        assert_eq!(set.message_count(), 2);
        assert_eq!(set.session_id, "session-1");
    }

    #[tokio::test]
    async fn clear_changes_only_the_cache_rewrite_identity() {
        let mut set = working_set().await;
        set.push_user("one").await;
        set.clear_history().await.unwrap();
        assert!(set.history().is_empty());
        assert_eq!(set.rewrite_version(), 1);
    }
}
