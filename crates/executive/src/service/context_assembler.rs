//! Deterministic, bounded turn context assembly.

use crate::service::daemon_turn::helpers::{bounded_text_history, build_request_messages};
use async_trait::async_trait;
use fabric::{AgoraOps, Clock, Message, TurnRequest};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

const MAX_FRAGMENT_CHARS: usize = 16 * 1024;
const MAX_INJECTED_CHARS: usize = 48 * 1024;
const MAX_SYSTEM_PREFIX_CHARS: usize = 128 * 1024;

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

pub struct ProductionContextSource {
    pub cached_prefix: Arc<Mutex<String>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub recall_service: Arc<dyn mnemosyne::MemoryService>,
    pub facts: Arc<dyn mnemosyne::FactUseCases>,
    pub core_memory: Arc<Mutex<mnemosyne::CoreMemory>>,
    pub skill_loader: Arc<Mutex<corpus::SkillLoader>>,
    pub skill_router: Arc<Mutex<corpus::SkillRouter>>,
    pub self_field: Arc<Mutex<dasein::SelfField>>,
    pub agora: Arc<dyn AgoraOps>,
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl ContextSource for ProductionContextSource {
    async fn load(&self, request: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        let system_prefix = format!(
            "{}\n\nCurrent working directory: {}\nTreat this as the user's current project. Do not scan unrelated host directories to guess a project.",
            self.cached_prefix.lock().await.clone(), request.working_dir.display()
        );
        let updates = self.memory_queue.lock().await.drain(..).collect::<Vec<_>>();
        let recall =
            if request.input.trim().len() < 8 || request.input.trim_start().starts_with('/') {
                String::new()
            } else {
                let normalized = request.input.to_ascii_lowercase();
                let include_historical = ["historical", "history", "superseded", "历史", "旧决策"]
                    .iter()
                    .any(|marker| normalized.contains(marker));
                crate::r#impl::hook_lifecycle::recall_inject::recall_composite_context(
                    self.recall_service.as_ref(),
                    mnemosyne::RecallRequest {
                        session: request.session_id.clone(),
                        query: request.input.clone(),
                        max_items: 8,
                        max_content_bytes: 16 * 1024,
                        current_at: Some(fabric::wall_to_datetime(self.clock.wall_now())),
                        include_historical,
                    },
                    std::time::Duration::from_millis(250),
                    8,
                    16 * 1024,
                )
                .await
            };
        let facts = self
            .facts
            .search(mnemosyne::SearchFactsRequest {
                query: request.input.clone(),
                scope: None,
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .take(4)
            .map(|fact| format!("- {} (trust: {:.2})", fact.content, fact.trust_score))
            .chain(updates.into_iter().map(|item| format!("- {item}")))
            .collect::<Vec<_>>()
            .join("\n");
        let core_memory = self
            .core_memory
            .lock()
            .await
            .blocks()
            .iter()
            .filter(|(_, block)| !block.read_only && !block.value.is_empty())
            .flat_map(|(label, block)| {
                block
                    .value
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(move |line| format!("[core:{label}] {line}"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let skills = {
            let loader = self.skill_loader.lock().await;
            let keywords = loader
                .plugins()
                .iter()
                .filter(|plugin| !plugin.keywords.is_empty())
                .map(|plugin| corpus::skill::keyword_matcher::SkillKeywords {
                    name: plugin.name.clone(),
                    keywords: plugin.keywords.clone(),
                    body: plugin.system_prompt.clone(),
                })
                .collect::<Vec<_>>();
            corpus::skill::keyword_matcher::match_skills(&request.input, &keywords).join("\n\n")
        };
        let suggestion = self
            .skill_router
            .lock()
            .await
            .suggest(&request.input, 0.6, 1)
            .first()
            .map(|item| {
                format!(
                    "Suggested /{} ({:.2}) — {}",
                    item.name, item.confidence, item.description
                )
            })
            .unwrap_or_default();
        let skills = [skills, suggestion]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let dasein = self
            .self_field
            .lock()
            .await
            .dasein_prompt_injection()
            .unwrap_or_default();
        let agora = self
            .agora
            .snapshot(&request.session_id)
            .await
            .map(|value| value.to_string())
            .unwrap_or_default();
        Ok(ContextFragments {
            system_prefix,
            recall,
            core_memory,
            facts,
            skills,
            dasein,
            agora,
        })
    }
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
            truncate(&fragments.system_prefix, MAX_SYSTEM_PREFIX_CHARS),
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
