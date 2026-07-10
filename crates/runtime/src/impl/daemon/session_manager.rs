use std::path::Path;

use anyhow::Result;
use tracing::{debug, info, warn};

use base::{ContentBlock, Message, Role};
use cognit::r#impl::llm::LlmProvider;

use crate::r#impl::session::journal::{EventJournal, SessionEvent};
use memory::AdvancedCompressor;

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
            compressor: AdvancedCompressor::new(
                (max_tokens as f64 * 0.25) as usize, // tail token budget
                4_000,                               // target summary chars
                max_tokens,                          // context window
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

    /// Push an arbitrary message into the conversation history and journal.
    /// Tool use/result blocks are journaled so session recovery can reconstruct
    /// multi-block messages correctly.
    pub async fn push_message(&mut self, message: Message) {
        debug!(role = ?message.role, blocks = message.content.len(), "Pushed message");

        // Journal tool blocks so session recovery can reconstruct multi-block messages
        for block in &message.content {
            match block {
                ContentBlock::ToolUse { id, name, input } => {
                    let _ = self
                        .journal
                        .append(SessionEvent::ToolUseBlock {
                            tool_use_id: id.clone(),
                            tool_name: name.clone(),
                            input: input.clone(),
                        })
                        .await;
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let _ = self
                        .journal
                        .append(SessionEvent::ToolResultBlock {
                            tool_use_id: tool_use_id.clone(),
                            content: content.clone(),
                            is_error: *is_error,
                        })
                        .await;
                }
                _ => {}
            }
        }

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

    /// Return total message count (all roles: user, assistant, system).
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Get a reference to the event journal for query access.
    pub fn journal(&self) -> &EventJournal {
        &self.journal
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
            self.compressor.force_compact(&mut self.messages, llm).await
        } else {
            self.compressor.maybe_compact(&mut self.messages, llm).await
        }
        .unwrap_or(false);
        if !did {
            return false;
        }
        let after_count = self.messages.len();
        let summary = self.compressor.last_summary().unwrap_or("").to_string();
        self.persist_compaction(before_count, after_count, summary)
            .await;
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
                .append(SessionEvent::Summary { text: summary })
                .await;
        }
        // Re-journal the surviving tail (text + tool blocks) after the checkpoint
        // so a reopen reconstructs [summary] ++ tail.  System messages are skipped.
        let tail: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .cloned()
            .collect();
        for m in &tail {
            // Emit tool_use / tool_result blocks as dedicated events so recovery
            // can reconstruct multi-block messages correctly.
            for block in &m.content {
                match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        let _ = self
                            .journal
                            .append(SessionEvent::ToolUseBlock {
                                tool_use_id: id.clone(),
                                tool_name: name.clone(),
                                input: input.clone(),
                            })
                            .await;
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let _ = self
                            .journal
                            .append(SessionEvent::ToolResultBlock {
                                tool_use_id: tool_use_id.clone(),
                                content: content.clone(),
                                is_error: *is_error,
                            })
                            .await;
                    }
                    _ => {}
                }
            }
            // Also emit a text-content event for the message role so recovery
            // can reconstruct the text parts without losing message boundaries.
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
        let _ = self.journal.flush().await;
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

        // Detect corrupted sessions: orphan tool_use blocks, API error messages, etc.
        if is_session_corrupted(&state.events_after_checkpoint) {
            warn!(
                session_id,
                "Recovered session data is corrupted (orphan tool calls or API errors); starting fresh"
            );
            return None;
        }

        let mut messages = Vec::new();
        let mut pending_tool_uses: Vec<ContentBlock> = Vec::new();

        for event in &state.events_after_checkpoint {
            match event {
                SessionEvent::Summary { text } => {
                    flush_tool_uses(&mut messages, &mut pending_tool_uses);
                    messages.push(Message::system(format!("[Conversation summary]\n{}", text)));
                }
                SessionEvent::ToolUseBlock {
                    tool_use_id,
                    tool_name,
                    input,
                } => {
                    pending_tool_uses.push(ContentBlock::ToolUse {
                        id: tool_use_id.clone(),
                        name: tool_name.clone(),
                        input: input.clone(),
                    });
                }
                SessionEvent::ToolResultBlock {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    // Flush any pending tool_uses from a prior assistant message
                    // before emitting the tool_result.
                    flush_tool_uses(&mut messages, &mut pending_tool_uses);
                    messages.push(Message::tool_result(tool_use_id, content, *is_error));
                }
                SessionEvent::UserMessage { content } => {
                    flush_tool_uses(&mut messages, &mut pending_tool_uses);
                    messages.push(Message::user(content));
                }
                SessionEvent::AssistantMessage { content } => {
                    if pending_tool_uses.is_empty() {
                        messages.push(Message::assistant(content));
                    } else {
                        // Assistant message follows accumulated tool_use blocks —
                        // combine them into a single multi-block assistant message.
                        let mut blocks = std::mem::take(&mut pending_tool_uses);
                        if !content.is_empty() {
                            blocks.push(ContentBlock::Text {
                                text: content.clone(),
                            });
                        }
                        messages.push(Message {
                            role: Role::Assistant,
                            content: blocks,
                        });
                    }
                }
                SessionEvent::Compacted { .. } => {
                    // Superseded by the checkpoint written right after compaction.
                    // The summary and tail are in subsequent events after the checkpoint.
                }
                _ => {}
            }
        }

        // Flush any remaining pending tool_uses at end of stream.
        flush_tool_uses(&mut messages, &mut pending_tool_uses);

        Some(messages)
    }
}

/// Flush accumulated tool_use blocks as an assistant message.
fn flush_tool_uses(messages: &mut Vec<Message>, pending: &mut Vec<ContentBlock>) {
    if !pending.is_empty() {
        messages.push(Message {
            role: Role::Assistant,
            content: std::mem::take(pending),
        });
    }
}

/// Check whether the recovered session events contain corruption that would
/// cause API errors on replay:
/// - Orphan tool_use blocks (assistant requests tool calls but tool results
///   are missing, creating an invalid `tool_calls`-without-response sequence).
/// - API error messages stored as assistant text responses.
fn is_session_corrupted(events: &[SessionEvent]) -> bool {
    let mut pending_ids: Vec<String> = Vec::new();
    for event in events {
        match event {
            SessionEvent::ToolUseBlock { tool_use_id, .. } => {
                pending_ids.push(tool_use_id.clone());
            }
            SessionEvent::ToolResultBlock { tool_use_id, .. } => {
                pending_ids.retain(|id| id != tool_use_id);
            }
            SessionEvent::AssistantMessage { content } => {
                // Detect API error messages that were stored as assistant content.
                if content.contains("tool_calls") && content.contains("must be followed") {
                    return true;
                }
            }
            SessionEvent::UserMessage { .. } if !pending_ids.is_empty() => {
                // A new user turn has started. Any unresolved tool_use blocks
                // from the previous turn are orphans.
                return true;
            }
            _ => {}
        }
    }
    // Unresolved tool_use blocks at the end of the event stream.
    !pending_ids.is_empty()
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use async_trait::async_trait;
    use base::message::is_tool_message;
    use base::ToolDefinition;
    use cognit::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};

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
            anyhow::bail!("mock(StubLlm): streaming not implemented")
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
                format!("t{i}"),
                "y".repeat(400),
                false,
            ))
            .await;
            sm.push_user(&format!("user {i} {}", "z".repeat(400))).await;
        }
        let did = sm.compact_if_needed(&StubLlm).await;
        assert!(did, "should compact");
        let hist = sm.history();
        // first non-system message after the summary must not be a bare tool_result
        let first_non_system = hist.iter().find(|m| !matches!(m.role, Role::System));
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
                sm.push_user(&format!("user {i} {}", "z".repeat(400))).await;
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

    #[tokio::test]
    async fn tool_messages_survive_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let session_id = "tool-test".to_string();

        // Create session, push tool messages
        {
            let mut sm = SessionManager::new(dir.path(), session_id.clone(), 100_000)
                .await
                .unwrap();
            sm.push_user("Read a file").await;
            sm.push_message(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "call_01".to_string(),
                    name: "file_read".to_string(),
                    input: serde_json::json!({"path": "/tmp/test.txt"}),
                }],
            })
            .await;
            sm.push_message(Message::tool_result("call_01", "file contents here", false))
                .await;
            sm.push_assistant("The file says: file contents here").await;
            // Flush so all events are persisted before recovery
            let _ = sm.journal().flush().await;
        }

        // Recover and verify tool messages are preserved
        let sm2 = SessionManager::new(dir.path(), session_id, 100_000)
            .await
            .unwrap();
        let hist = sm2.history();

        // Find tool_use message
        let has_tool_use = hist.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "call_01"))
        });
        assert!(has_tool_use, "ToolUse message should survive recovery");

        // Find tool_result message
        let has_tool_result = hist.iter().any(|m| {
            m.content.iter().any(|b| {
                matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "call_01")
            })
        });
        assert!(
            has_tool_result,
            "ToolResult message should survive recovery"
        );
    }
}
