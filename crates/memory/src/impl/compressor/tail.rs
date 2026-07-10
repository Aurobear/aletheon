use base::message::Message;

#[derive(Debug, Clone)]
pub struct TailProtectionConfig {
    pub tail_token_budget: usize,
    pub min_tail_messages: usize,
    pub soft_ceiling_multiplier: f64,
}

impl Default for TailProtectionConfig {
    fn default() -> Self {
        Self {
            tail_token_budget: 20_000,
            min_tail_messages: 1,
            soft_ceiling_multiplier: 1.5,
        }
    }
}

pub fn find_tail_cut(messages: &[Message], config: &TailProtectionConfig) -> usize {
    if messages.is_empty() {
        return 0;
    }

    let mut token_count = 0usize;
    let mut cut = messages.len();
    let soft_limit = (config.tail_token_budget as f64 * config.soft_ceiling_multiplier) as usize;

    for (i, msg) in messages.iter().enumerate().rev() {
        let msg_tokens = msg.estimate_tokens();
        if token_count + msg_tokens > soft_limit {
            break;
        }
        token_count += msg_tokens;
        cut = i;
    }

    let min_cut = messages.len().saturating_sub(config.min_tail_messages);
    cut = cut.min(min_cut);
    cut = align_boundary_backward(messages, cut);
    cut = ensure_last_user_message_in_tail(messages, cut);

    cut
}

fn align_boundary_backward(messages: &[Message], cut: usize) -> usize {
    if cut == 0 || cut >= messages.len() {
        return cut;
    }
    let mut aligned = cut;

    // Skip past tool messages to avoid starting the tail with an orphan
    // tool_result (which requires a preceding tool_use).
    while aligned > 0 && base::message::is_tool_message(&messages[aligned]) {
        aligned -= 1;
    }

    if aligned == 0 {
        // All messages from 1..=cut are tool messages.  Walk backward from
        // the original cut to find the last tool_use.  Starting the tail at a
        // tool_use is safe because its tool_results follow in the tail.
        for i in (1..=cut).rev() {
            if messages[i].role == base::message::Role::Assistant
                && messages[i]
                    .content
                    .iter()
                    .any(|c| matches!(c, base::message::ContentBlock::ToolUse { .. }))
            {
                return i;
            }
        }
        // No tool_use found — fall back to the original cut (degraded but
        // allows compaction to proceed).
        return cut;
    }

    // Check for tool_use/tool_result pair splitting: if the last message in
    // the old section has ToolUse blocks, its tool_results are in the tail.
    // Include the tool_use in the tail to keep the pair together.
    let prev = &messages[aligned - 1];
    if prev.role == base::message::Role::Assistant
        && prev
            .content
            .iter()
            .any(|c| matches!(c, base::message::ContentBlock::ToolUse { .. }))
    {
        aligned -= 1;
        // Also skip any preceding tool messages to maintain the chain.
        while aligned > 0 && base::message::is_tool_message(&messages[aligned]) {
            aligned -= 1;
        }
    }

    aligned
}

fn ensure_last_user_message_in_tail(messages: &[Message], cut: usize) -> usize {
    // Find the last user message before the cut.
    let last_user_before_cut = messages[..cut]
        .iter()
        .rposition(|m| m.role == base::message::Role::User);

    match last_user_before_cut {
        Some(pos) => {
            let distance = cut - pos;
            // Only pull back if the user message is immediately before the cut
            // (within 1 message) to keep user+response together.
            if distance <= 1 && pos > 0 {
                pos
            } else {
                cut
            }
        }
        None => cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_conversation_no_split() {
        let messages = vec![Message::user("hello"), Message::assistant("hi there")];
        let config = TailProtectionConfig {
            tail_token_budget: 10_000,
            min_tail_messages: 3,
            soft_ceiling_multiplier: 1.5,
        };
        let cut = find_tail_cut(&messages, &config);
        assert_eq!(cut, 0);
    }

    #[test]
    fn test_long_conversation_splits() {
        let messages: Vec<Message> = (0..100)
            .map(|i| Message::user(format!("message {}", i)))
            .collect();
        let config = TailProtectionConfig {
            tail_token_budget: 100,
            min_tail_messages: 3,
            soft_ceiling_multiplier: 1.5,
        };
        let cut = find_tail_cut(&messages, &config);
        assert!(cut > 0);
        assert!(cut < messages.len());
        assert!(messages.len() - cut >= 3);
    }
}
