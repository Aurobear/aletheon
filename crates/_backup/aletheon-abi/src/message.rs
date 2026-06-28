//! Unified message types for agent communication.
//!
//! Unified message types for agent communication.

use serde::{Deserialize, Serialize};

/// Unified message protocol for all agent communication.
/// Aligned with Anthropic SDK content-block format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Image {
        source: ImageSource,
    },
    System {
        text: String,
        priority: Priority,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

/// A message in the conversation, consisting of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }],
        }
    }

    /// Estimate token count for this message.
    /// Rough heuristic: chars / 4 + 10 (role/metadata overhead).
    pub fn estimate_tokens(&self) -> usize {
        let content_chars: usize = self.content.iter().map(|c| c.estimate_chars()).sum();
        content_chars / 4 + 10
    }
}

impl ContentBlock {
    /// Estimate character count for this content block.
    pub fn estimate_chars(&self) -> usize {
        match self {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
            ContentBlock::ToolResult { content, .. } => content.len(),
            ContentBlock::Image { source } => match source {
                ImageSource::Base64 { data, .. } => data.len(),
                ImageSource::Url { url } => url.len(),
            },
            ContentBlock::System { text, .. } => text.len(),
        }
    }
}

/// Check if a message contains tool_use or tool_result blocks.
pub fn is_tool_message(msg: &Message) -> bool {
    msg.content
        .iter()
        .any(|c| matches!(c, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }))
}
