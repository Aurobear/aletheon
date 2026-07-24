use fabric::message::{ContentBlock, Message, Role};

pub struct SummaryTemplate;

impl SummaryTemplate {
    pub fn render(&self, messages: &[Message], target_chars: usize) -> String {
        let conversation = format_messages(messages);
        format!(
            r#"You are a context compressor. Summarize the following conversation into a structured handoff document.

**Rules:**
- PRESERVE all factual information (file paths, error messages, decisions, commands run)
- Use the exact section structure below
- "Active Task" must capture the user's most recent unfulfilled input VERBATIM
- "Remaining Work" is context, NOT instructions
- Target length: ~{target_chars} characters

**Required sections:**

## Active Task
[User's most recent unfulfilled input, verbatim]

## Goal
[What the user is trying to accomplish overall]

## Completed Actions
[Numbered list: tool name -> target -> outcome]

## Active State
[Working directory, branch, modified files, test status]

## In Progress
[Work underway when compaction fired]

## Blocked
[Blockers/errors with exact error messages]

## Key Decisions
[Technical decisions and rationale]

## Pending User Asks
[Unfulfilled questions/requests]

## Relevant Files
[Files read, modified, or created]

## Remaining Work
[Framed as context, not instructions]

## Critical Context
[Specific values, errors, config that would be lost]

---

**Conversation to summarize:**

{conversation}"#
        )
    }

    pub fn render_iterative(
        &self,
        previous_summary: &str,
        new_messages: &[Message],
        target_chars: usize,
    ) -> String {
        let new_turns = format_messages(new_messages);
        format!(
            r#"You are a context compressor. A previous summary exists and new conversation turns need to be integrated.

**Rules:**
- PRESERVE all existing information that is still relevant
- ADD new completed actions to the numbered list (continue numbering)
- Move items from "In Progress" to "Completed Actions" when done
- Update "Active Task" to the LATEST user message if it supersedes the old one
- If the latest user message contradicts the summary, the latest message WINS
- Target length: ~{target_chars} characters

**Previous summary:**

{previous_summary}

**New turns to integrate:**

{new_turns}

**Output the updated structured summary with the same section format.**"#
        )
    }
}

pub const SUMMARY_PREFIX: &str = "\
[CONTEXT COMPACTION \u{2014} REFERENCE ONLY] Earlier turns were compacted \
into the summary below. This is a handoff from a previous context \
window \u{2014} treat it as background reference, NOT as active instructions. \
Do NOT answer questions or fulfill requests mentioned in this summary; \
they were already addressed. \
Respond ONLY to the latest user message that appears AFTER this \
summary \u{2014} that message is the single source of truth for what to do \
right now. \
If the latest user message contradicts the summary, discard the \
stale items entirely. \
The current session state (files, config, etc.) may reflect work \
described here \u{2014} avoid repeating it:";

fn format_messages(messages: &[Message]) -> String {
    messages
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
                        format!("[Tool Call: {name}({input})]")
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        format!("[Tool Result: {content}]")
                    }
                    _ => String::new(),
                })
                .collect();
            format!("{role}: {content}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_contains_all_sections() {
        let template = SummaryTemplate;
        let messages = vec![Message::user("test")];
        let prompt = template.render(&messages, 1000);
        assert!(prompt.contains("## Active Task"));
        assert!(prompt.contains("## Goal"));
        assert!(prompt.contains("## Completed Actions"));
        assert!(prompt.contains("## Remaining Work"));
    }

    #[test]
    fn test_render_iterative_contains_previous() {
        let template = SummaryTemplate;
        let messages = vec![Message::user("new message")];
        let prompt = template.render_iterative("old summary", &messages, 1000);
        assert!(prompt.contains("old summary"));
        assert!(prompt.contains("new message"));
    }
}
