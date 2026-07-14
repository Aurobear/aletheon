//! Session Distiller — parses JSONL transcripts, extracts facts via a
//! pluggable extractor, and writes them to the FactStore.

use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Mutex;

use super::{Hook, HookEvent, HookResult};
use mnemosyne::FactStore;

const MAX_TRANSCRIPT_CHARS: usize = 12_000;
const MAX_FACTS_PER_SESSION: usize = 5;

// ── Extracted Fact ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ExtractedFact {
    pub content: String,
    pub category: String, // "user" | "feedback" | "project" | "reference"
    pub tags: Vec<String>,
    pub importance: String, // "high" | "medium" | "low"
}

// ── FactExtractor trait ──────────────────────────────────────────────────────

pub trait FactExtractor: Send + Sync {
    fn extract(&self, transcript: &str) -> Result<Vec<ExtractedFact>>;
}

// ── SessionDistiller ─────────────────────────────────────────────────────────

pub struct SessionDistiller {
    fact_store: Mutex<FactStore>,
    extractor: Option<Box<dyn FactExtractor>>,
}

impl SessionDistiller {
    pub fn new(fact_store: FactStore) -> Self {
        Self {
            fact_store: Mutex::new(fact_store),
            extractor: None,
        }
    }

    pub fn with_extractor(mut self, extractor: Box<dyn FactExtractor>) -> Self {
        self.extractor = Some(extractor);
        self
    }

    /// Parse a JSONL transcript file and extract the conversation text.
    /// Returns the concatenated user/assistant turns, truncated to MAX_TRANSCRIPT_CHARS.
    pub fn parse_transcript(transcript_text: &str) -> String {
        let mut turns: Vec<String> = Vec::new();

        for line in transcript_text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if role != "user" && role != "assistant" {
                continue;
            }

            let text = extract_text_content(&msg);
            if !text.is_empty() {
                turns.push(format!("[{}]: {}", role, text));
            }
        }

        let full = turns.join("\n");
        truncate_to(&full, MAX_TRANSCRIPT_CHARS)
    }

    /// Run the distillation pipeline: parse transcript, extract facts,
    /// write to FactStore.
    pub fn distill(&self, session_id: &str, transcript_text: &str) -> Result<Vec<ExtractedFact>> {
        let parsed = Self::parse_transcript(transcript_text);
        if parsed.is_empty() {
            return Ok(Vec::new());
        }

        let extractor = match &self.extractor {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        let facts = extractor.extract(&parsed)?;
        let store = self.fact_store.lock().unwrap_or_else(|e| e.into_inner());

        // Filter out low-importance and cap at MAX_FACTS_PER_SESSION
        let mut written = Vec::new();
        for fact in facts
            .iter()
            .filter(|f| f.importance != "low")
            .take(MAX_FACTS_PER_SESSION)
        {
            let tags_str = fact.tags.join(",");
            store.add_fact(
                &fact.content,
                &fact.category,
                &tags_str,
                &format!("session:{}", session_id),
                0.5,
                "episodic",
                14,
            )?;
            written.push(fact.clone());
        }

        Ok(written)
    }

    /// Get a reference to the fact store (for testing).
    #[cfg(test)]
    fn fact_store(&self) -> &Mutex<FactStore> {
        &self.fact_store
    }
}

impl Hook for SessionDistiller {
    fn name(&self) -> &str {
        "session-distiller"
    }

    fn handle(&self, event: &HookEvent) -> Result<HookResult> {
        match event {
            HookEvent::SessionStop {
                transcript_path,
                session_id,
            } => {
                let content = std::fs::read_to_string(transcript_path)
                    .with_context(|| format!("reading transcript {}", transcript_path.display()))?;
                let facts = self.distill(session_id, &content)?;
                if facts.is_empty() {
                    return Ok(HookResult::Noop);
                }
                Ok(HookResult::Inject {
                    context: format!("Distilled {} facts from session", facts.len()),
                })
            }
            _ => Ok(HookResult::Noop),
        }
    }
}

// ── Transcript Parsing Helpers ───────────────────────────────────────────────

/// Extract text content from a JSONL message object.
/// Handles both `"content": "text"` and `"content": [{"type":"text","text":"..."}]`.
fn extract_text_content(msg: &Value) -> String {
    let content = match msg.get("content") {
        Some(c) => c,
        None => return String::new(),
    };

    if let Some(s) = content.as_str() {
        return s.to_string();
    }

    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for block in arr {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                parts.push(text.to_string());
            }
        }
        return parts.join(" ");
    }

    String::new()
}

/// Truncate a string to at most `max_chars` characters, preferring to cut at
/// a newline boundary.
fn truncate_to(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }

    // Try to find a newline near the cutoff point
    let cutoff = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());

    // Look for a newline before the cutoff
    if let Some(nl_pos) = s[..cutoff].rfind('\n') {
        s[..nl_pos].to_string()
    } else {
        s[..cutoff].to_string()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    struct MockExtractor {
        facts: Vec<ExtractedFact>,
    }

    impl FactExtractor for MockExtractor {
        fn extract(&self, _transcript: &str) -> Result<Vec<ExtractedFact>> {
            Ok(self.facts.clone())
        }
    }

    fn setup() -> (SessionDistiller, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = FactStore::open(tmp.path()).unwrap();
        let distiller = SessionDistiller::new(store);
        (distiller, tmp)
    }

    #[test]
    fn test_parse_transcript_basic() {
        let jsonl = r#"{"role":"user","content":"Hello"}
{"role":"assistant","content":"Hi there!"}
{"role":"user","content":"How are you?"}
"#;
        let parsed = SessionDistiller::parse_transcript(jsonl);
        assert!(parsed.contains("[user]: Hello"));
        assert!(parsed.contains("[assistant]: Hi there!"));
        assert!(parsed.contains("[user]: How are you?"));
    }

    #[test]
    fn test_parse_transcript_tool_use() {
        let jsonl = r#"{"role":"user","content":"Run the tests"}
{"role":"assistant","content":[{"type":"text","text":"Running tests..."},{"type":"tool_use","id":"1","name":"bash","input":{}}]}
"#;
        let parsed = SessionDistiller::parse_transcript(jsonl);
        assert!(parsed.contains("[user]: Run the tests"));
        assert!(parsed.contains("[assistant]: Running tests..."));
    }

    #[test]
    fn test_parse_transcript_truncate() {
        // Create a very long transcript
        let mut lines = Vec::new();
        for i in 0..2000 {
            lines.push(format!(
                r#"{{"role":"user","content":"This is line number {} with some extra padding text to make it longer"}}"#,
                i
            ));
        }
        let jsonl = lines.join("\n");
        let parsed = SessionDistiller::parse_transcript(&jsonl);
        assert!(parsed.len() <= MAX_TRANSCRIPT_CHARS);
    }

    #[test]
    fn test_distill_writes_to_store() {
        let (distiller, _tmp) = setup();

        let mock = MockExtractor {
            facts: vec![
                ExtractedFact {
                    content: "User prefers Rust".to_string(),
                    category: "user".to_string(),
                    tags: vec!["preference".to_string()],
                    importance: "medium".to_string(),
                },
                ExtractedFact {
                    content: "Deployed to production".to_string(),
                    category: "project".to_string(),
                    tags: vec!["deploy".to_string()],
                    importance: "high".to_string(),
                },
            ],
        };

        let distiller = distiller.with_extractor(Box::new(mock));
        let jsonl = r#"{"role":"user","content":"I prefer Rust for systems work"}
{"role":"assistant","content":"Great choice!"}
"#;

        let written = distiller.distill("sess-1", jsonl).unwrap();
        assert_eq!(written.len(), 2);

        // Verify facts were written to the store
        let store = distiller
            .fact_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let results = store.search_facts("Rust", None, 0.0, 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_distill_skips_low_importance() {
        let (distiller, _tmp) = setup();

        let mock = MockExtractor {
            facts: vec![
                ExtractedFact {
                    content: "Important fact".to_string(),
                    category: "project".to_string(),
                    tags: vec![],
                    importance: "high".to_string(),
                },
                ExtractedFact {
                    content: "Low importance fact".to_string(),
                    category: "reference".to_string(),
                    tags: vec![],
                    importance: "low".to_string(),
                },
            ],
        };

        let distiller = distiller.with_extractor(Box::new(mock));
        let jsonl = r#"{"role":"user","content":"Test content"}"#;
        let written = distiller.distill("sess-2", jsonl).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].content, "Important fact");
    }
}
