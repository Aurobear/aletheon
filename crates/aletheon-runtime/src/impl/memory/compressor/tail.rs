use aletheon_abi::message::Message;

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
    let original_cut = cut;
    let mut aligned = cut;
    while aligned > 0 && aletheon_abi::message::is_tool_message(&messages[aligned]) {
        aligned -= 1;
    }
    // If alignment collapsed to a non-useful position (all preceding messages
    // are tool messages), use the original cut to ensure compaction still happens.
    if aligned == 0 && original_cut > 1 {
        original_cut
    } else {
        aligned
    }
}

fn ensure_last_user_message_in_tail(messages: &[Message], cut: usize) -> usize {
    // Find the last user message before the cut.
    let last_user_before_cut = messages[..cut]
        .iter()
        .rposition(|m| m.role == aletheon_abi::message::Role::User);

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
