pub mod tail;
pub mod template;

use anyhow::Result;
use tracing::info;

use aletheon_abi::message::{ContentBlock, Message};
use aletheon_brain::r#impl::llm::LlmProvider;
use tail::{find_tail_cut, TailProtectionConfig};
use template::{SummaryTemplate, SUMMARY_PREFIX};

/// Advanced context compressor with token-budget tail protection
/// and iterative summary updates.
pub struct AdvancedCompressor {
    pub tail_config: TailProtectionConfig,
    pub target_summary_chars: usize,
    previous_summary: Option<String>,
    template: SummaryTemplate,
}

impl AdvancedCompressor {
    pub fn new(tail_token_budget: usize, target_summary_chars: usize) -> Self {
        Self {
            tail_config: TailProtectionConfig {
                tail_token_budget,
                ..Default::default()
            },
            target_summary_chars,
            previous_summary: None,
            template: SummaryTemplate,
        }
    }

    /// Check if compaction is needed and perform it.
    /// Returns true if compaction was performed.
    pub async fn maybe_compact(
        &mut self,
        messages: &mut Vec<Message>,
        llm: &dyn LlmProvider,
    ) -> Result<bool> {
        let total_tokens: usize = messages.iter().map(|m| m.estimate_tokens()).sum();

        if total_tokens < self.tail_config.tail_token_budget * 2 {
            return Ok(false);
        }

        let cut = find_tail_cut(messages, &self.tail_config);
        if cut == 0 || cut >= messages.len() {
            return Ok(false);
        }

        let system_msgs: Vec<Message> = messages
            .iter()
            .take_while(|m| m.role == aletheon_abi::message::Role::System)
            .cloned()
            .collect();
        let actual_cut = std::cmp::max(cut, system_msgs.len());

        let old_messages = &messages[system_msgs.len()..actual_cut];
        let tail_messages = &messages[actual_cut..];

        if old_messages.is_empty() {
            return Ok(false);
        }

        // Prune tool outputs before summarization
        let mut pruned_messages = old_messages.to_vec();
        aletheon_body::r#impl::tools::output::pruner::prune_tool_outputs(&mut pruned_messages, 0);

        let summary = self.generate_summary(&pruned_messages, llm).await?;

        let mut compacted = system_msgs;
        compacted.push(Message::system(format!(
            "{}\n{}\n[End Summary]",
            SUMMARY_PREFIX, summary
        )));
        compacted.extend_from_slice(tail_messages);

        self.previous_summary = Some(summary);

        let before = messages.len();
        *messages = compacted;
        info!(
            before = before,
            after = messages.len(),
            cut = actual_cut,
            "Context compacted with token-budget tail protection"
        );

        Ok(true)
    }

    async fn generate_summary(
        &self,
        messages: &[Message],
        llm: &dyn LlmProvider,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_compressor() {
        let compressor = AdvancedCompressor::new(20_000, 4_000);
        assert_eq!(compressor.tail_config.tail_token_budget, 20_000);
        assert_eq!(compressor.target_summary_chars, 4_000);
        assert!(compressor.previous_summary.is_none());
    }
}
