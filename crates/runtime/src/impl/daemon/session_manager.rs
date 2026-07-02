use std::path::Path;

use anyhow::Result;
use tracing::{debug, info};

use base::{ContentBlock, Message, Role};
use cognit::r#impl::llm::LlmProvider;

use crate::r#impl::memory::compressor::AdvancedCompressor;
use crate::r#impl::session::journal::{EventJournal, SessionEvent};

/// SessionManager: persists conversation history, journals events, and
/// performs context compaction when the token budget is exceeded.
///
/// Note: `SessionStore` and `RecallMemory` are NOT held here because
/// `rusqlite::Connection` is not `Send`.  The caller is responsible for
/// journaling to store/memory outside the manager when needed.
pub struct SessionManager {
    pub session_id: String,
    messages: Vec<Message>,
    journal: EventJournal,
    max_tokens: usize,
    compaction_threshold: f64,
    compressor: AdvancedCompressor,
}

impl SessionManager {
    /// Create a new SessionManager.  If a journal already exists for
    /// `session_id` the history is recovered automatically.
    pub async fn new(data_dir: &Path, session_id: String, max_tokens: usize) -> Result<Self> {
        // Try to recover existing messages from journal
        let messages = match Self::recover(data_dir, &session_id).await {
            Some(msgs) if !msgs.is_empty() => {
                info!(
                    session_id = %session_id,
                    recovered = msgs.len(),
                    "Recovered session history from journal"
                );
                msgs
            }
            _ => Vec::new(),
        };

        let journal = EventJournal::create(&session_id, data_dir).await?;

        Ok(Self {
            session_id,
            messages,
            journal,
            max_tokens,
            compaction_threshold: 0.8,
            compressor: AdvancedCompressor::new(
                (max_tokens as f64 * 0.25) as usize, // tail token budget
                4_000,                                // target summary chars
                max_tokens,                           // context window
            ),
        })
    }

    /// Push a user message into the conversation history and journal.
    pub async fn push_user(&mut self, content: &str) {
        self.messages.push(Message::user(content));
        let _ = self
            .journal
            .append(SessionEvent::UserMessage {
                content: content.to_string(),
            })
            .await;
        debug!(len = content.len(), "Pushed user message");
    }

    /// Push an assistant message into the conversation history and journal.
    pub async fn push_assistant(&mut self, content: &str) {
        self.messages.push(Message::assistant(content));
        let _ = self
            .journal
            .append(SessionEvent::AssistantMessage {
                content: content.to_string(),
            })
            .await;
        debug!(len = content.len(), "Pushed assistant message");
    }

    /// Push a system message into the conversation history (no journal).
    pub fn push_system(&mut self, content: &str) {
        self.messages.push(Message::system(content));
        debug!(len = content.len(), "Pushed system message");
    }

    /// Push an arbitrary message into the conversation history (no journal).
    /// Used for tool call/result messages that need to be preserved in session history.
    pub fn push_message(&mut self, message: Message) {
        debug!(role = ?message.role, blocks = message.content.len(), "Pushed message");
        self.messages.push(message);
    }

    /// Return a reference to the full message history.
    pub fn history(&self) -> &[Message] {
        &self.messages
    }

    /// Return the number of user turns.
    pub fn turn_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .count()
    }

    /// Rough token estimate: 4 chars per token.
    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.estimate_tokens()).sum()
    }

    /// Compact the context window if we exceed the threshold, using the
    /// tool-boundary-safe compressor. Returns true if compaction happened.
    pub async fn compact_if_needed(&mut self, llm: &dyn LlmProvider) -> bool {
        self.run_compaction(llm, false).await
    }

    /// Force compaction regardless of token estimate.
    pub async fn force_compact(&mut self, llm: &dyn LlmProvider) -> bool {
        if self.messages.len() <= 2 {
            return false;
        }
        self.run_compaction(llm, true).await
    }

    async fn run_compaction(&mut self, llm: &dyn LlmProvider, force: bool) -> bool {
        let before_count = self.messages.len();
        let did = if force {
            self.compressor
                .force_compact(&mut self.messages, llm)
                .await
        } else {
            self.compressor
                .maybe_compact(&mut self.messages, llm)
                .await
        }
        .unwrap_or(false);
        if !did {
            return false;
        }
        let after_count = self.messages.len();
        let summary = self.compressor.last_summary().unwrap_or("").to_string();
        self.persist_compaction(before_count, after_count, summary).await;
        info!(
            before = before_count,
            after = after_count,
            "Context compaction complete"
        );
        true
    }

    async fn persist_compaction(
        &mut self,
        before_count: usize,
        after_count: usize,
        summary: String,
    ) {
        // Marker (keeps existing observability), then a fresh checkpoint so
        // recover starts from the compacted state, then the summary + surviving tail.
        let _ = self
            .journal
            .append(SessionEvent::Compacted {
                before_count,
                after_count,
            })
            .await;
        let iteration = self.turn_count();
        let _ = self
            .journal
            .append(SessionEvent::CheckpointBoundary { iteration })
            .await;
        if !summary.is_empty() {
            let _ = self
                .journal
                .append(SessionEvent::Summary {
                    text: summary,
                })
                .await;
        }
        // Re-journal the surviving tail (text content) after the checkpoint so a
        // reopen reconstructs [summary] ++ tail. System summary is emitted above.
        let tail: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .cloned()
            .collect();
        for m in &tail {
            let text: String = m
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            match m.role {
                Role::User => {
                    let _ = self
                        .journal
                        .append(SessionEvent::UserMessage { content: text })
                        .await;
                }
                Role::Assistant => {
                    let _ = self
                        .journal
                        .append(SessionEvent::AssistantMessage { content: text })
                        .await;
                }
                Role::System => {}
            }
        }
    }

    /// Write a checkpoint boundary to the journal.
    pub async fn save_checkpoint(&mut self) {
        let iteration = self.turn_count();
        let _ = self
            .journal
            .append(SessionEvent::CheckpointBoundary { iteration })
            .await;
        info!(iteration, "Checkpoint saved");
    }

    /// Recover message history from a journal on disk.
    pub async fn recover(data_dir: &Path, session_id: &str) -> Option<Vec<Message>> {
        let state = match EventJournal::recover(data_dir, session_id).await {
            Ok(s) => s,
            Err(e) => {
                debug!(error = %e, "Journal recovery failed (no existing session?)");
                return None;
            }
        };

        if state.events_after_checkpoint.is_empty() {
            return None;
        }

        let mut messages = Vec::new();
        for event in &state.events_after_checkpoint {
            match event {
                SessionEvent::Summary { text } => {
                    messages.push(Message::system(format!(
                        "[Conversation summary]\n{}",
                        text
                    )));
                }
                SessionEvent::UserMessage { content } => {
                    messages.push(Message::user(content));
                }
                SessionEvent::AssistantMessage { content } => {
                    messages.push(Message::assistant(content));
                }
                SessionEvent::Compacted { .. } => {
                    // Superseded by the checkpoint written right after compaction.
                    // The summary and tail are in subsequent events after the checkpoint.
                }
                _ => {}
            }
        }

        Some(messages)
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use base::message::is_tool_message;
    use base::ToolDefinition;
    use cognit::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};
    use async_trait::async_trait;

    struct StubLlm;

    #[async_trait]
    impl LlmProvider for StubLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "SUMMARY".into(),
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
            unimplemented!()
        }
        fn name(&self) -> &str {
            "stub"
        }
        fn max_context_length(&self) -> usize {
            1_000
        }
    }

    #[tokio::test]
    async fn compaction_tail_never_starts_with_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        // small max_tokens so the threshold trips easily
        let mut sm = SessionManager::new(dir.path(), "s1".into(), 1_000)
            .await
            .unwrap();
        // build a long history that interleaves tool_use/tool_result pairs
        for i in 0..12 {
            sm.push_assistant(&format!("assistant turn {i} {}", "x".repeat(400)))
                .await;
            sm.push_message(Message::tool_result(
                &format!("t{i}"),
                &"y".repeat(400),
                false,
            ));
            sm.push_user(&format!("user {i} {}", "z".repeat(400)))
                .await;
        }
        let did = sm.compact_if_needed(&StubLlm).await;
        assert!(did, "should compact");
        let hist = sm.history();
        // first non-system message after the summary must not be a bare tool_result
        let first_non_system = hist
            .iter()
            .find(|m| !matches!(m.role, Role::System));
        if let Some(m) = first_non_system {
            assert!(
                !is_tool_message(m),
                "tail must not start with an orphan tool message"
            );
        }
    }

    #[tokio::test]
    async fn compacted_history_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut sm = SessionManager::new(dir.path(), "s2".into(), 1_000)
                .await
                .unwrap();
            for i in 0..12 {
                sm.push_assistant(&format!("assistant {i} {}", "x".repeat(400)))
                    .await;
                sm.push_user(&format!("user {i} {}", "z".repeat(400)))
                    .await;
            }
            assert!(sm.compact_if_needed(&StubLlm).await);
        }
        // Reopen: recover must include the summary system message
        let sm2 = SessionManager::new(dir.path(), "s2".into(), 1_000)
            .await
            .unwrap();
        let hist = sm2.history();
        assert!(
            !hist.is_empty(),
            "recovered history must not be empty after compaction"
        );
        assert!(
            hist.iter().any(|m| matches!(m.role, Role::System)
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("SUMMARY")))),
            "recovered history must contain the persisted summary"
        );
    }
}
