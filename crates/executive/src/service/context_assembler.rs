//! Deterministic, bounded turn context assembly.

use crate::service::daemon_turn::helpers::{bounded_text_history, build_request_messages};
use async_trait::async_trait;
use fabric::{Message, TurnRequest};
use std::sync::Arc;
use thiserror::Error;

const MAX_FRAGMENT_CHARS: usize = 16 * 1024;
const MAX_INJECTED_CHARS: usize = 48 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextFragments {
    pub system_prefix: String,
    pub recall: String,
    pub core_memory: String,
    pub facts: String,
    pub skills: String,
    pub dasein: String,
    pub agora: String,
}

#[derive(Clone, Debug)]
pub struct AssembledContext {
    pub messages: Vec<Message>,
    pub effective_user_message: String,
}

#[derive(Debug, Error)]
pub enum ContextAssemblyError {
    #[error("context source failed: {0}")]
    Source(String),
}

#[async_trait]
pub trait ContextSource: Send + Sync {
    async fn load(&self, request: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError>;
}

pub struct ContextAssembler {
    source: Arc<dyn ContextSource>,
}

impl ContextAssembler {
    pub fn new(source: Arc<dyn ContextSource>) -> Self {
        Self { source }
    }

    pub async fn assemble(
        &self,
        request: &TurnRequest,
        canonical_history: &[Message],
    ) -> Result<AssembledContext, ContextAssemblyError> {
        let fragments = self.source.load(request).await?;
        let mut effective = String::new();
        let mut remaining = MAX_INJECTED_CHARS;
        for (label, value) in [
            ("recall", fragments.recall.as_str()),
            ("core-memory", fragments.core_memory.as_str()),
            ("facts", fragments.facts.as_str()),
            ("skills", fragments.skills.as_str()),
            ("dasein", fragments.dasein.as_str()),
            ("agora", fragments.agora.as_str()),
        ] {
            append_fragment(&mut effective, label, value, &mut remaining);
        }
        if !effective.is_empty() {
            effective.push('\n');
        }
        effective.push_str(&request.input);
        let history = bounded_text_history(canonical_history);
        let messages = build_request_messages(
            truncate(&fragments.system_prefix, MAX_FRAGMENT_CHARS),
            &history,
            effective.clone(),
        );
        Ok(AssembledContext {
            messages,
            effective_user_message: effective,
        })
    }
}

fn append_fragment(output: &mut String, label: &str, value: &str, remaining: &mut usize) {
    if value.trim().is_empty() || *remaining == 0 {
        return;
    }
    let bounded = truncate(value, MAX_FRAGMENT_CHARS.min(*remaining));
    *remaining = remaining.saturating_sub(bounded.chars().count());
    output.push_str(&format!("<{label}>\n{bounded}\n</{label}>\n"));
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
