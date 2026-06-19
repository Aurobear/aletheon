use std::path::Path;

use anyhow::Result;
use tracing::{debug, info, warn};

use aletheon_abi::{ContentBlock, Message, Role};
use aletheon_brain::r#impl::llm::LlmProvider;

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

    /// Compact the context window if we exceed the threshold.
    ///
    /// Strategy: keep system messages, summarize everything except the
    /// last 6 non-system messages, and prepend the summary as a system message.
    /// Returns `true` if compaction was performed.
    pub async fn compact_if_needed(&mut self, llm: &dyn LlmProvider) -> bool {
        let estimated = self.estimate_tokens();
        let threshold = (self.max_tokens as f64 * self.compaction_threshold) as usize;

        if estimated <= threshold {
            return false;
        }

        info!(
            estimated = estimated,
            threshold = threshold,
            "Context window exceeded threshold, compacting"
        );

        let before_count = self.messages.len();

        // Split: system messages, middle (to summarize), tail (keep)
        let system_msgs: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| matches!(m.role, Role::System))
            .cloned()
            .collect();

        let non_system: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .cloned()
            .collect();

        let keep_count = 6.min(non_system.len());
        let split_idx = non_system.len().saturating_sub(keep_count);

        if split_idx == 0 {
            return false;
        }

        let to_summarize = &non_system[..split_idx];
        let tail = &non_system[split_idx..];

        // Build summarization prompt
        let mut summary_parts = Vec::new();
        for msg in to_summarize {
            let role_str = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let text: String = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            summary_parts.push(format!("{}: {}", role_str, text));
        }

        let summarize_messages = vec![
            Message::system(
                "Summarize the following conversation history concisely. \
                 Preserve key facts, decisions, and context needed for \
                 continuation. Output only the summary, no preamble.",
            ),
            Message::user(summary_parts.join("\n")),
        ];

        let summary_text = match llm.complete(&summarize_messages, &[]).await {
            Ok(resp) => resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
            Err(e) => {
                warn!(error = %e, "LLM summarization failed, using truncation fallback");
                summary_parts
                    .iter()
                    .map(|s| {
                        if s.len() > 200 {
                            let end = s.char_indices().nth(200).map(|(i, _)| i).unwrap_or(s.len());
                            format!("{}...", &s[..end])
                        } else {
                            s.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };

        // Rebuild: system messages + summary + tail
        let mut new_messages = system_msgs;
        new_messages.push(Message::system(format!(
            "[Conversation summary]\n{}",
            summary_text
        )));
        new_messages.extend(tail.iter().cloned());

        let after_count = new_messages.len();
        self.messages = new_messages;

        let _ = self
            .journal
            .append(SessionEvent::Compacted {
                before_count,
                after_count,
            })
            .await;

        info!(
            before = before_count,
            after = after_count,
            "Context compaction complete"
        );

        true
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

    /// Force compaction regardless of token estimate.
    pub async fn force_compact(&mut self, llm: &dyn LlmProvider) -> bool {
        if self.messages.len() <= 2 {
            return false;
        }

        let saved = self.compaction_threshold;
        self.compaction_threshold = 0.0;
        let result = self.compact_if_needed(llm).await;
        self.compaction_threshold = saved;
        result
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
                SessionEvent::UserMessage { content } => {
                    messages.push(Message::user(content));
                }
                SessionEvent::AssistantMessage { content } => {
                    messages.push(Message::assistant(content));
                }
                SessionEvent::Compacted { .. } => {
                    // After a compaction, the in-memory state is the source of truth.
                    messages.clear();
                }
                _ => {}
            }
        }

        Some(messages)
    }
}
