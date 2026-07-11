use cognit::r#impl::provider_registry::ProviderRegistry;
use fabric::LlmProvider;
use std::sync::Arc;

use crate::core::config::ModelRoutingConfig;

/// Task type for model routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// General chat — use default model.
    General,
    /// Input contains images or audio — needs multimodal model.
    Multimodal,
    /// Simple task (short message, file reading) — use cheap model.
    Simple,
    /// Complex reasoning task — use reasoning model.
    Reasoning,
    /// AutoMemory fact extraction — use cheap model.
    AutoMemory,
}

/// Dynamic model router that selects the best model for each task type.
pub struct ModelRouter {
    routing: ModelRoutingConfig,
    registry: Arc<ProviderRegistry>,
}

impl ModelRouter {
    pub fn new(routing: ModelRoutingConfig, registry: Arc<ProviderRegistry>) -> Self {
        Self { routing, registry }
    }

    /// Resolve the model spec for a given task type.
    /// Returns the spec string, or None if no routing configured for that type.
    fn resolve_spec(&self, task: TaskType) -> Option<&str> {
        match task {
            TaskType::General => self.routing.default.as_deref(),
            TaskType::Multimodal => self.routing.multimodal.as_deref(),
            TaskType::Simple => self.routing.cheap.as_deref(),
            TaskType::Reasoning => self.routing.reasoning.as_deref(),
            TaskType::AutoMemory => self.routing.auto_memory.as_deref(),
        }
    }

    /// Create an LLM provider for the given task type.
    /// Falls back through the routing chain: task-specific → default → registry default.
    pub fn create_provider(&self, task: TaskType) -> anyhow::Result<Box<dyn LlmProvider>> {
        // Try task-specific model
        if let Some(spec) = self.resolve_spec(task) {
            if let Ok(provider) = self.registry.resolve_and_create(spec) {
                return Ok(provider);
            }
        }

        // Fall back to default routing
        if task != TaskType::General {
            if let Some(spec) = self.resolve_spec(TaskType::General) {
                if let Ok(provider) = self.registry.resolve_and_create(spec) {
                    return Ok(provider);
                }
            }
        }

        // Fall back to registry default
        self.registry.resolve_and_create("")
    }

    /// Classify a message into a task type based on content analysis.
    pub fn classify_message(&self, message: &str) -> TaskType {
        // Check for image/audio indicators
        if Self::has_multimodal_content(message) {
            return TaskType::Multimodal;
        }

        // Check for reasoning indicators
        if Self::is_reasoning_task(message) {
            return TaskType::Reasoning;
        }

        // Check for simple tasks
        if Self::is_simple_task(message) {
            return TaskType::Simple;
        }

        TaskType::General
    }

    /// Detect multimodal content indicators.
    fn has_multimodal_content(message: &str) -> bool {
        let lower = message.to_lowercase();
        // Image indicators
        lower.contains("[image")
            || lower.contains("![")
            || lower.contains("data:image/")
            || lower.contains(".png")
            || lower.contains(".jpg")
            || lower.contains(".jpeg")
            || lower.contains(".gif")
            || lower.contains(".webp")
            || lower.contains("图片")
            || lower.contains("照片")
            || lower.contains("screenshot")
            // Audio indicators
            || lower.contains("[audio")
            || lower.contains("data:audio/")
            || lower.contains(".mp3")
            || lower.contains(".wav")
            || lower.contains(".ogg")
            || lower.contains("音频")
            || lower.contains("语音")
    }

    /// Detect complex reasoning tasks.
    fn is_reasoning_task(message: &str) -> bool {
        let lower = message.to_lowercase();
        let word_count = message.split_whitespace().count();

        // Long messages with reasoning keywords
        if word_count > 50 {
            let reasoning_keywords = [
                "analyze",
                "分析",
                "reason",
                "推理",
                "explain why",
                "解释为什么",
                "compare",
                "对比",
                "evaluate",
                "评估",
                "design",
                "设计",
                "architecture",
                "架构",
                "trade-off",
                "权衡",
                "consider",
                "think step by step",
                "逐步思考",
                "chain of thought",
            ];
            let matches = reasoning_keywords
                .iter()
                .filter(|kw| lower.contains(*kw))
                .count();
            if matches >= 2 {
                return true;
            }
        }

        false
    }

    /// Detect simple tasks that don't need an expensive model.
    ///
    /// IMPORTANT: Only classify messages as Simple if they DON'T require
    /// tool execution. File operations (read/cat/ls/list) all need bash_exec
    /// or file_read tools, so they must go through the General model which
    /// reliably supports tool calling.
    fn is_simple_task(message: &str) -> bool {
        let word_count = message.split_whitespace().count();

        // Very short greetings / acknowledgments that need only text response
        if word_count <= 5 {
            let lower = message.to_lowercase();
            let simple_patterns = [
                "hello",
                "hi",
                "hey",
                "thanks",
                "thank you",
                "ok",
                "okay",
                "yes",
                "no",
                "sure",
                "好的",
                "谢谢",
                "你好",
                "嗨",
                "再见",
                "bye",
                "goodbye",
            ];
            if simple_patterns
                .iter()
                .any(|p| lower.trim() == *p || lower.trim().starts_with(p))
            {
                return true;
            }
        }

        false
    }

    /// Get the model name for a task type (for logging/debugging).
    pub fn model_name_for(&self, task: TaskType) -> String {
        self.resolve_spec(task)
            .unwrap_or("(registry default)")
            .to_string()
    }
}

// Static version for tests (no self needed)
#[cfg(test)]
impl ModelRouter {
    fn classify_message_static(message: &str) -> TaskType {
        if Self::has_multimodal_content(message) {
            return TaskType::Multimodal;
        }
        if Self::is_simple_task(message) {
            return TaskType::Simple;
        }
        TaskType::General
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_multimodal() {
        assert_eq!(
            ModelRouter::classify_message_static("看这张图片 ![photo](test.png)"),
            TaskType::Multimodal
        );
        assert_eq!(
            ModelRouter::classify_message_static("帮我识别这张照片"),
            TaskType::Multimodal
        );
    }

    #[test]
    fn classify_simple() {
        // Greetings/acknowledgments that don't need tools
        assert_eq!(
            ModelRouter::classify_message_static("hello"),
            TaskType::Simple
        );
        assert_eq!(
            ModelRouter::classify_message_static("thanks"),
            TaskType::Simple
        );
        // File operations need tools → General, not Simple
        assert_eq!(
            ModelRouter::classify_message_static("read README.md"),
            TaskType::General
        );
        assert_eq!(
            ModelRouter::classify_message_static("ls"),
            TaskType::General
        );
    }

    #[test]
    fn classify_general() {
        assert_eq!(
            ModelRouter::classify_message_static("帮我写一个 Rust 函数"),
            TaskType::General
        );
    }
}
