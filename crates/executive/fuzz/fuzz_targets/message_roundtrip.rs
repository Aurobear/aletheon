#![no_main]

use arbitrary::Arbitrary;
use fabric::{ContentBlock, ImageSource, Message, MessagePriority, Role};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum BlockInput {
    Text(String),
    Thinking(String, Option<String>),
    ToolUse(String, String, String),
    ToolResult(String, String, bool),
    Base64Image(String, String),
    UrlImage(String),
    System(String, u8),
}

#[derive(Arbitrary, Debug)]
struct MessageInput {
    role: u8,
    blocks: Vec<BlockInput>,
}

fn block(input: BlockInput) -> ContentBlock {
    match input {
        BlockInput::Text(text) => ContentBlock::Text { text },
        BlockInput::Thinking(text, signature) => ContentBlock::Thinking { text, signature },
        BlockInput::ToolUse(id, name, input) => ContentBlock::ToolUse {
            id,
            name,
            input: serde_json::from_str(&input).unwrap_or(serde_json::Value::String(input)),
        },
        BlockInput::ToolResult(tool_use_id, content, is_error) => ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        },
        BlockInput::Base64Image(media_type, data) => ContentBlock::Image {
            source: ImageSource::Base64 { media_type, data },
        },
        BlockInput::UrlImage(url) => ContentBlock::Image {
            source: ImageSource::Url { url },
        },
        BlockInput::System(text, priority) => ContentBlock::System {
            text,
            priority: match priority % 4 {
                0 => MessagePriority::Low,
                1 => MessagePriority::Normal,
                2 => MessagePriority::High,
                _ => MessagePriority::Critical,
            },
        },
    }
}

fuzz_target!(|input: MessageInput| {
    let message = Message {
        role: match input.role % 3 {
            0 => Role::User,
            1 => Role::Assistant,
            _ => Role::System,
        },
        content: input.blocks.into_iter().map(block).collect(),
    };

    let encoded = serde_json::to_vec(&message).expect("Message must serialize");
    let decoded: Message =
        serde_json::from_slice(&encoded).expect("serialized Message must deserialize");
    assert_eq!(
        serde_json::to_value(decoded).expect("decoded Message must serialize"),
        serde_json::to_value(message).expect("original Message must serialize")
    );
});
