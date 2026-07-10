use cognit::r#impl::llm::LlmProvider;
use fabric::message::{ContentBlock, Message, Role};
use tracing::info;

/// Manages context compaction by summarizing old messages.
pub struct CompactionManager {
    /// Keep this many recent messages untouched
    keep_recent: usize,
    /// Compact when messages exceed this count
    summarize_threshold: usize,
    /// Target summary length in chars
    target_summary_chars: usize,
}

impl CompactionManager {
    pub fn new(
        keep_recent: usize,
        summarize_threshold: usize,
        target_summary_chars: usize,
    ) -> Self {
        Self {
            keep_recent,
            summarize_threshold,
            target_summary_chars,
        }
    }

    /// Check if compaction is needed and perform it.
    /// Returns true if compaction was performed.
    pub async fn maybe_compact(
        &self,
        messages: &mut Vec<Message>,
        llm: &dyn LlmProvider,
    ) -> anyhow::Result<bool> {
        if messages.len() <= self.summarize_threshold {
            return Ok(false);
        }

        let split_point = messages.len().saturating_sub(self.keep_recent);
        // Don't summarize system messages at the start
        let system_msgs: Vec<_> = messages
            .iter()
            .take_while(|m| m.role == Role::System)
            .cloned()
            .collect();
        let actual_split = std::cmp::max(split_point, system_msgs.len());

        let old_messages = &messages[system_msgs.len()..actual_split];
        if old_messages.is_empty() {
            return Ok(false);
        }

        let summary = self.summarize(old_messages, llm).await?;

        // Build compacted messages: system msgs + summary + recent msgs
        let mut compacted = system_msgs;
        compacted.push(Message::system(format!(
            "[Context Summary \u{2014} {} messages compacted]\n{}\n[End Summary]",
            old_messages.len(),
            summary
        )));
        compacted.extend_from_slice(&messages[actual_split..]);

        let before = messages.len();
        *messages = compacted;
        info!(before = before, after = messages.len(), "Context compacted");

        Ok(true)
    }

    async fn summarize(
        &self,
        messages: &[Message],
        llm: &dyn LlmProvider,
    ) -> anyhow::Result<String> {
        // Format messages into a single text block for summarization
        let conversation: String = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    Role::System => "System",
                };
                let content: String = m
                    .content
                    .iter()
                    .map(|c| match c {
                        ContentBlock::Text { text } => text.clone(),
                        ContentBlock::ToolUse { name, input, .. } => {
                            format!("[Tool Call: {}({})]", name, input)
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            format!("[Tool Result: {}]", content)
                        }
                        _ => String::new(),
                    })
                    .collect();
                format!("{}: {}", role, content)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = vec![
            Message::system(format!(
                "Summarize the following conversation concisely. \
                 Include: key topics discussed, decisions made, files modified, \
                 errors encountered, and current task state. \
                 Target length: ~{} characters. Be factual, not verbose.",
                self.target_summary_chars
            )),
            Message::user(conversation),
        ];

        let response = llm.complete(&prompt, &[]).await?;
        let summary = response
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
