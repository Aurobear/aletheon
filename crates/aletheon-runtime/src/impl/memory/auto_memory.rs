//! Automatic memory extraction from conversation turns.
//!
//! Uses a cheap LLM to analyze each conversation turn and extract
//! important facts about the user, storing them in CoreMemory without
//! requiring the main model to proactively call memory tools.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use aletheon_abi::message::{ContentBlock, Message, Role};
use aletheon_brain::r#impl::llm::LlmProvider;

use super::core_memory::CoreMemory;

/// Maximum number of recent facts to keep in the dedup buffer.
const DEDUP_BUFFER_SIZE: usize = 100;

/// Minimum user message length to trigger extraction.
const MIN_MESSAGE_LENGTH: usize = 20;

/// Maximum tokens for the extraction LLM call.
#[allow(dead_code)]
const MAX_EXTRACTION_TOKENS: u32 = 500;

/// Extracted fact from a conversation turn.
#[derive(Debug, Clone)]
pub struct ExtractedFact {
    pub label: String,
    pub content: String,
}

/// Automatic memory extraction system.
///
/// Analyzes each conversation turn using a cheap LLM and stores
/// important facts in CoreMemory. Runs after the main response is sent
/// to avoid blocking the user-facing turn.
pub struct AutoMemory {
    /// Cheap LLM model for fact extraction (e.g., deepseek_flash, mimo-v2.5-flash).
    llm: Arc<dyn LlmProvider>,
    /// Core memory to store extracted facts into.
    core_memory: Arc<Mutex<CoreMemory>>,
    /// Buffer of recently stored facts for deduplication.
    recent_facts: Vec<String>,
}

impl AutoMemory {
    /// Create a new AutoMemory extractor.
    ///
    /// - `llm`: A cheap, fast LLM provider for extraction (not the main model).
    /// - `core_memory`: Shared CoreMemory instance to store facts into.
    pub fn new(llm: Arc<dyn LlmProvider>, core_memory: Arc<Mutex<CoreMemory>>) -> Self {
        Self {
            llm,
            core_memory,
            recent_facts: Vec::new(),
        }
    }

    /// Analyze a conversation turn and store important facts in CoreMemory.
    ///
    /// Returns the list of facts that were actually stored (after dedup).
    /// Errors are logged and swallowed — this must never fail the main turn.
    pub async fn analyze_and_store(
        &mut self,
        user_message: &str,
        assistant_response: &str,
    ) -> Result<Vec<String>> {
        // Skip short messages — too little signal to extract from.
        if user_message.len() < MIN_MESSAGE_LENGTH {
            debug!(
                len = user_message.len(),
                "AutoMemory: skipping short message"
            );
            return Ok(Vec::new());
        }

        // Build the extraction prompt.
        let prompt = build_extraction_prompt(user_message, assistant_response);

        // Call the cheap LLM for extraction.
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = match self.llm.complete(&messages, &[]).await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(error = %e, "AutoMemory: LLM extraction failed, skipping");
                return Ok(Vec::new());
            }
        };

        // Extract text from response.
        let response_text: String = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Parse the JSON array of facts.
        let facts = match parse_facts(&response_text) {
            Ok(f) => f,
            Err(e) => {
                debug!(error = %e, response = %response_text, "AutoMemory: failed to parse extraction response");
                return Ok(Vec::new());
            }
        };

        if facts.is_empty() {
            debug!("AutoMemory: no facts extracted");
            return Ok(Vec::new());
        }

        // Deduplicate against recent facts.
        let new_facts: Vec<ExtractedFact> = facts
            .into_iter()
            .filter(|f| !self.is_duplicate(&f.content))
            .collect();

        if new_facts.is_empty() {
            debug!("AutoMemory: all extracted facts are duplicates");
            return Ok(Vec::new());
        }

        // Store new facts in CoreMemory.
        let mut stored = Vec::new();
        {
            let mut core = self.core_memory.lock().await;
            for fact in &new_facts {
                let block_label = normalize_label(&fact.label);
                let snippet = format!("- {}", fact.content);
                match core.append(&block_label, &snippet) {
                    Ok(_) => {
                        info!(
                            block = %block_label,
                            content = %fact.content,
                            "AutoMemory: stored fact"
                        );
                        self.recent_facts.push(fact.content.clone());
                        stored.push(fact.content.clone());
                    }
                    Err(e) => {
                        debug!(
                            block = %block_label,
                            error = %e,
                            "AutoMemory: failed to append fact"
                        );
                    }
                }
            }
        }

        // Trim dedup buffer if too large.
        if self.recent_facts.len() > DEDUP_BUFFER_SIZE {
            let drain_count = self.recent_facts.len() - DEDUP_BUFFER_SIZE;
            self.recent_facts.drain(..drain_count);
        }

        Ok(stored)
    }

    /// Check if a fact is a duplicate of a recently stored one.
    fn is_duplicate(&self, content: &str) -> bool {
        let normalized = content.trim().to_lowercase();
        self.recent_facts
            .iter()
            .any(|existing| existing.trim().to_lowercase() == normalized)
    }
}

/// Build the extraction prompt for the cheap LLM.
fn build_extraction_prompt(user_message: &str, assistant_response: &str) -> String {
    format!(
        r#"Analyze this conversation turn and extract important facts about the user.
Only extract facts that are worth remembering long-term:
- User's name, preferences, habits
- Project information, goals, constraints
- Technical preferences (languages, tools, styles)
- Important decisions or conclusions

Conversation:
User: {user}
Assistant: {assistant}

Output a JSON array of facts. Each fact: {{"label": "human|learned|system_state", "content": "one concise sentence"}}
If nothing worth remembering, output: []"#,
        user = user_message,
        assistant = assistant_response,
    )
}

/// Parse the JSON array of facts from the LLM response.
fn parse_facts(response: &str) -> Result<Vec<ExtractedFact>> {
    // Try to extract JSON array from response (handle markdown code blocks).
    let json_str = extract_json_array(response);

    let raw: Vec<serde_json::Value> = serde_json::from_str(&json_str)?;

    let mut facts = Vec::new();
    for item in raw {
        let label = item
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("learned")
            .to_string();
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => continue,
        };
        facts.push(ExtractedFact { label, content });
    }

    Ok(facts)
}

/// Extract a JSON array from a response that may contain markdown formatting.
fn extract_json_array(response: &str) -> &str {
    let trimmed = response.trim();

    // Try to find a JSON array in the response.
    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            if end > start {
                return &trimmed[start..=end];
            }
        }
    }

    trimmed
}

/// Normalize fact labels to valid CoreMemory block names.
fn normalize_label(label: &str) -> String {
    match label.to_lowercase().as_str() {
        "human" | "user" | "user_info" | "user_pref" => "human".to_string(),
        "learned" | "knowledge" | "learned_knowledge" => "learned".to_string(),
        "system_state" | "system" | "system_observation" => "system_state".to_string(),
        other => {
            // Default to "learned" for unknown labels.
            debug!(label = other, "AutoMemory: unknown label, defaulting to 'learned'");
            "learned".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::message::ContentBlock;
    use aletheon_brain::r#impl::llm::{LlmResponse, StopReason, Usage};

    /// Stub LLM that returns a fixed response.
    struct StubLlm {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for StubLlm {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[aletheon_abi::ToolDefinition],
        ) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: self.response.clone(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 50,
                },
                cache_hit_tokens: 0,
                cache_miss_tokens: 100,
            })
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[aletheon_abi::ToolDefinition],
        ) -> Result<aletheon_brain::r#impl::llm::LlmStream> {
            unimplemented!("StubLlm does not support streaming")
        }

        fn name(&self) -> &str {
            "stub"
        }

        fn max_context_length(&self) -> usize {
            4096
        }
    }

    /// Stub LLM that always errors.
    struct ErrorLlm;

    #[async_trait::async_trait]
    impl LlmProvider for ErrorLlm {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[aletheon_abi::ToolDefinition],
        ) -> Result<LlmResponse> {
            anyhow::bail!("simulated LLM error")
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[aletheon_abi::ToolDefinition],
        ) -> Result<aletheon_brain::r#impl::llm::LlmStream> {
            unimplemented!()
        }

        fn name(&self) -> &str {
            "error_stub"
        }

        fn max_context_length(&self) -> usize {
            4096
        }
    }

    fn make_auto_memory(llm_response: &str) -> AutoMemory {
        let llm: Arc<dyn LlmProvider> = Arc::new(StubLlm {
            response: llm_response.to_string(),
        });
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        AutoMemory::new(llm, core_memory)
    }

    fn make_auto_memory_with_core(llm_response: &str, core: Arc<Mutex<CoreMemory>>) -> AutoMemory {
        let llm: Arc<dyn LlmProvider> = Arc::new(StubLlm {
            response: llm_response.to_string(),
        });
        AutoMemory::new(llm, core)
    }

    #[tokio::test]
    async fn test_auto_memory_skips_short() {
        let mut am = make_auto_memory("[]");
        let facts = am.analyze_and_store("hi", "hello").await.unwrap();
        assert!(facts.is_empty(), "Short messages should be skipped");
    }

    #[tokio::test]
    async fn test_auto_memory_extracts_name() {
        let response = r#"[{"label": "human", "content": "User's name is Aurobear"}]"#;
        let mut am = make_auto_memory(response);
        let facts = am
            .analyze_and_store(
                "你好，我叫Aurobear，是一个机器人工程师",
                "你好Aurobear！很高兴认识你。",
            )
            .await
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("Aurobear"));
    }

    #[tokio::test]
    async fn test_auto_memory_extracts_preference() {
        let response =
            r#"[{"label": "human", "content": "User prefers Rust for systems programming"}]"#;
        let mut am = make_auto_memory(response);
        let facts = am
            .analyze_and_store(
                "我比较喜欢用Rust来写系统级的程序，性能很好",
                "Rust确实很适合系统编程，内存安全且高性能。",
            )
            .await
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("Rust"));

        // Verify it was stored in CoreMemory.
        let core = am.core_memory.lock().await;
        let human_block = core.get("human").unwrap();
        assert!(human_block.contains("Rust"));
    }

    #[tokio::test]
    async fn test_auto_memory_dedup() {
        let response =
            r#"[{"label": "human", "content": "User prefers Rust for systems programming"}]"#;
        let core = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let mut am = make_auto_memory_with_core(response, core);

        // First call — should store.
        let facts1 = am
            .analyze_and_store(
                "我比较喜欢用Rust来写系统级的程序，性能很好",
                "Rust确实很适合系统编程。",
            )
            .await
            .unwrap();
        assert_eq!(facts1.len(), 1);

        // Second call with same fact — should be deduped.
        let facts2 = am
            .analyze_and_store(
                "我还是喜欢用Rust来写系统程序，因为它性能好",
                "是的，Rust的性能确实出色。",
            )
            .await
            .unwrap();
        assert!(facts2.is_empty(), "Duplicate fact should not be stored again");
    }

    #[tokio::test]
    async fn test_auto_memory_empty_response() {
        let mut am = make_auto_memory("[]");
        let facts = am
            .analyze_and_store(
                "今天天气怎么样？天气预报说会下雨。",
                "根据天气预报，今天可能会有小雨，建议带伞。",
            )
            .await
            .unwrap();
        assert!(facts.is_empty(), "Empty extraction should return no facts");
    }

    #[tokio::test]
    async fn test_auto_memory_llm_error_graceful() {
        let llm: Arc<dyn LlmProvider> = Arc::new(ErrorLlm);
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let mut am = AutoMemory::new(llm, core_memory);

        // Should not panic or return error — just gracefully skip.
        let facts = am
            .analyze_and_store(
                "我叫Aurobear，我喜欢用Rust编程",
                "你好Aurobear！Rust是一门很棒的语言。",
            )
            .await
            .unwrap();
        assert!(facts.is_empty(), "LLM error should result in empty facts");
    }

    #[test]
    fn test_parse_facts_valid_json() {
        let json = r#"[{"label": "human", "content": "User likes Rust"}, {"label": "learned", "content": "Rust is good for systems"}]"#;
        let facts = parse_facts(json).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].label, "human");
        assert_eq!(facts[1].label, "learned");
    }

    #[test]
    fn test_parse_facts_empty_array() {
        let facts = parse_facts("[]").unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn test_parse_facts_with_markdown() {
        let json = r#"```json
[{"label": "human", "content": "User name is Aurobear"}]
```"#;
        let facts = parse_facts(json).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].content.contains("Aurobear"));
    }

    #[test]
    fn test_normalize_label() {
        assert_eq!(normalize_label("human"), "human");
        assert_eq!(normalize_label("Human"), "human");
        assert_eq!(normalize_label("user"), "human");
        assert_eq!(normalize_label("learned"), "learned");
        assert_eq!(normalize_label("knowledge"), "learned");
        assert_eq!(normalize_label("system_state"), "system_state");
        assert_eq!(normalize_label("system"), "system_state");
        assert_eq!(normalize_label("random_label"), "learned"); // default
    }
}
