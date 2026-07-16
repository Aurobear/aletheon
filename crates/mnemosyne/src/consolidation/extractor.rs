use super::{ExtractionCompletion, MemoryCandidate};
use crate::{MemoryKind, MemoryScope};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalMemoryEvent {
    pub event_id: String,
    pub kind: String,
    pub content: String,
}
#[derive(Debug, Clone)]
pub struct ExtractionBatch {
    pub scope: MemoryScope,
    pub events: Vec<CanonicalMemoryEvent>,
}
#[derive(Debug, Clone)]
pub struct CandidateExtractor {
    max_events: usize,
    max_bytes: usize,
    redaction_version: u32,
}
impl Default for CandidateExtractor {
    fn default() -> Self {
        Self {
            max_events: 128,
            max_bytes: 64 * 1024,
            redaction_version: 1,
        }
    }
}
impl CandidateExtractor {
    pub fn extract(&self, batch: &ExtractionBatch) -> anyhow::Result<ExtractionCompletion> {
        batch.scope.validate()?;
        let mut used = 0;
        let mut candidates = Vec::new();
        for event in batch.events.iter().take(self.max_events) {
            if !matches!(
                event.kind.as_str(),
                "assistant_message" | "tool_result" | "goal_outcome" | "architecture_decision"
            ) {
                continue;
            }
            let mut claim = redact(&event.content);
            if claim.trim().is_empty() {
                continue;
            }
            let remaining = self.max_bytes.saturating_sub(used);
            if remaining == 0 {
                break;
            }
            claim = truncate(claim, remaining);
            used += claim.len();
            let kind = match event.kind.as_str() {
                "goal_outcome" => MemoryKind::GoalOutcome,
                "architecture_decision" => MemoryKind::ArchitectureDecision,
                "tool_result" => MemoryKind::ToolOutcome,
                _ => MemoryKind::Reflection,
            };
            candidates.push(MemoryCandidate::new(
                kind,
                redact(&claim),
                vec![event.event_id.clone()],
                0.6,
                batch.scope.clone(),
                None,
                None,
                self.redaction_version,
            )?)
        }
        if candidates.is_empty() {
            Ok(ExtractionCompletion::SucceededNoOutput)
        } else {
            Ok(ExtractionCompletion::Succeeded { candidates })
        }
    }
}
fn redact(value: &str) -> String {
    let patterns = [
        r"(?i)(api[_-]?key|token|password)\s*[:=]\s*\S+",
        r"-----BEGIN [^-]+ PRIVATE KEY-----[\s\S]*?-----END [^-]+ PRIVATE KEY-----",
    ];
    patterns.iter().fold(value.to_string(), |v, p| {
        Regex::new(p)
            .unwrap()
            .replace_all(&v, "[REDACTED]")
            .into_owned()
    })
}
fn truncate(mut value: String, max: usize) -> String {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1
    }
    value.truncate(end);
    value
}
