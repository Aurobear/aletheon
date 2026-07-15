//! Pre-turn message injection methods on `DaemonTurnOrchestrator` and `TurnPipeline`.
//!
//! These methods enrich the user message with skill activation, fact recall,
//! core memory, and skill suggestions before the model sees it.

use super::helpers::{
    append_bounded_text, MAX_ACTIVATED_SKILLS_TOTAL_CHARS, MAX_ACTIVATED_SKILL_CHARS,
    MAX_RECALLED_FACT_CHARS, MAX_RECALL_TOTAL_CHARS,
};
use super::orchestrator::DaemonTurnOrchestrator;
use crate::service::turn_pipeline::TurnPipeline;
use mnemosyne::FactStore;
use tracing::info;

#[allow(dead_code)]
impl DaemonTurnOrchestrator {
    pub(crate) async fn inject_keyword_skills(
        &self,
        message: &str,
        effective_message: &mut String,
    ) {
        let loader = self.subsystems.corpus.skill_loader.lock().await;
        let skill_keywords: Vec<corpus::skill::keyword_matcher::SkillKeywords> = loader
            .plugins()
            .iter()
            .filter(|p| !p.keywords.is_empty())
            .map(|p| corpus::skill::keyword_matcher::SkillKeywords {
                name: p.name.clone(),
                keywords: p.keywords.clone(),
                body: p.system_prompt.clone(),
            })
            .collect();
        drop(loader);
        let matched = corpus::skill::keyword_matcher::match_skills(message, &skill_keywords);
        let mut remaining = MAX_ACTIVATED_SKILLS_TOTAL_CHARS;
        for body in matched {
            if remaining == 0 {
                break;
            }
            effective_message.push_str("\n<activated-skill>\n");
            append_bounded_text(
                effective_message,
                &body,
                MAX_ACTIVATED_SKILL_CHARS,
                &mut remaining,
            );
            effective_message.push_str("\n</activated-skill>\n");
        }
    }

    pub(crate) async fn inject_fact_recall(&self, message: &str, effective_message: &mut String) {
        let fs = self.subsystems.memory.fact_store.lock().await;
        let keywords: Vec<String> = message
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_lowercase())
            .collect();
        let query = keywords.join(" ");
        if query.len() < 8 {
            return;
        }
        if let Ok(facts) = fs.search_facts_governed(&query, None, false, 0.15, 4) {
            if !facts.is_empty() {
                let mut recall_block = String::from("\n[Recalled memories]\n");
                let mut remaining = MAX_RECALL_TOTAL_CHARS;
                for fact in &facts {
                    if remaining == 0 {
                        break;
                    }
                    recall_block.push_str("- ");
                    append_bounded_text(
                        &mut recall_block,
                        &fact.content,
                        MAX_RECALLED_FACT_CHARS,
                        &mut remaining,
                    );
                    recall_block.push_str(&format!(" (trust: {:.2})\n", fact.trust_score));
                    let _ = fs.record_feedback(fact.fact_id, true);
                }
                let entities = FactStore::extract_entities(message);
                for entity in entities.iter().take(3) {
                    let _ = fs.resolve_entity(entity).map(|eid| {
                        let _ = fs.get_entity_facts(eid).map(|related| {
                            for rf in related.iter().take(1) {
                                if !facts.iter().any(|f| f.fact_id == rf.fact_id) {
                                    if remaining == 0 {
                                        break;
                                    }
                                    recall_block.push_str("- ");
                                    append_bounded_text(
                                        &mut recall_block,
                                        &rf.content,
                                        MAX_RECALLED_FACT_CHARS,
                                        &mut remaining,
                                    );
                                    recall_block.push_str(&format!(" (entity: {})\n", entity));
                                }
                            }
                        });
                    });
                }
                info!(count = facts.len(), "Fact recall injected");
                effective_message.push_str(&recall_block);
            }
        }
    }

    pub(crate) async fn inject_core_memory(&self, effective_message: &mut String) {
        let cm = self.subsystems.memory.core_memory.lock().await;
        let mut core_lines = Vec::new();
        for (label, block) in cm.blocks() {
            if block.read_only || block.value.is_empty() {
                continue;
            }
            for line in block.value.lines() {
                if !line.trim().is_empty() {
                    core_lines.push(format!("[core:{}] {}", label, line));
                }
            }
        }
        if !core_lines.is_empty() {
            effective_message.push_str("\n[Core Memory — current state]\n");
            for line in &core_lines {
                effective_message.push_str(line);
                effective_message.push('\n');
            }
        }
    }

    pub(crate) async fn inject_skill_suggestion(
        &self,
        message: &str,
        effective_message: &mut String,
    ) {
        let sr = self.subsystems.corpus.skill_router.lock().await;
        let suggestions = sr.suggest(message, 0.6, 1);
        if let Some(suggestion) = suggestions.first() {
            info!(skill = %suggestion.name, confidence = suggestion.confidence, "Skill suggested");
            effective_message.push_str(&format!(
                "\n[Suggested skill] /{} (confidence: {:.2}) — {}\n",
                suggestion.name, suggestion.confidence, suggestion.description
            ));
        }
    }

    pub(crate) async fn decay_stale_facts(&self) {
        let fs = self.subsystems.memory.fact_store.lock().await;
        let _ = fs.decay_stale();
    }
}

impl TurnPipeline {
    pub(crate) async fn inject_keyword_skills(
        &self,
        message: &str,
        effective_message: &mut String,
    ) {
        let loader = self.subsystems.corpus.skill_loader.lock().await;
        let skill_keywords: Vec<corpus::skill::keyword_matcher::SkillKeywords> = loader
            .plugins()
            .iter()
            .filter(|p| !p.keywords.is_empty())
            .map(|p| corpus::skill::keyword_matcher::SkillKeywords {
                name: p.name.clone(),
                keywords: p.keywords.clone(),
                body: p.system_prompt.clone(),
            })
            .collect();
        drop(loader);
        let matched = corpus::skill::keyword_matcher::match_skills(message, &skill_keywords);
        let mut remaining = MAX_ACTIVATED_SKILLS_TOTAL_CHARS;
        for body in matched {
            if remaining == 0 {
                break;
            }
            effective_message.push_str("\n<activated-skill>\n");
            append_bounded_text(
                effective_message,
                &body,
                MAX_ACTIVATED_SKILL_CHARS,
                &mut remaining,
            );
            effective_message.push_str("\n</activated-skill>\n");
        }
    }

    pub(crate) async fn inject_composite_recall(
        &self,
        message: &str,
        session: &str,
        effective_message: &mut String,
    ) {
        const MAX_ITEMS: usize = 8;
        const MAX_BYTES: usize = 16 * 1024;
        const RECALL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

        if message.trim().len() < 8 || message.trim_start().starts_with('/') {
            return;
        }
        let normalized = message.to_ascii_lowercase();
        let include_historical = ["historical", "history", "superseded", "历史", "旧决策"]
            .iter()
            .any(|marker| normalized.contains(marker));
        let request = mnemosyne::RecallRequest {
            session: session.to_owned(),
            query: message.to_owned(),
            max_items: MAX_ITEMS,
            max_content_bytes: MAX_BYTES,
            current_at: Some(fabric::wall_to_datetime(self.clock.wall_now())),
            include_historical,
        };
        let context = crate::r#impl::hook_lifecycle::recall_inject::recall_composite_context(
            self.subsystems.memory.memory_service.as_ref(),
            request,
            RECALL_TIMEOUT,
            MAX_ITEMS,
            MAX_BYTES,
        )
        .await;
        if !context.is_empty() {
            effective_message.push('\n');
            effective_message.push_str(&context);
            effective_message.push('\n');
        }
    }

    pub(crate) async fn inject_core_memory(&self, effective_message: &mut String) {
        let cm = self.subsystems.memory.core_memory.lock().await;
        let mut core_lines = Vec::new();
        for (label, block) in cm.blocks() {
            if block.read_only || block.value.is_empty() {
                continue;
            }
            for line in block.value.lines() {
                if !line.trim().is_empty() {
                    core_lines.push(format!("[core:{}] {}", label, line));
                }
            }
        }
        if !core_lines.is_empty() {
            effective_message.push_str("\n[Core Memory — current state]\n");
            for line in &core_lines {
                effective_message.push_str(line);
                effective_message.push('\n');
            }
        }
    }

    pub(crate) async fn inject_skill_suggestion(
        &self,
        message: &str,
        effective_message: &mut String,
    ) {
        let sr = self.subsystems.corpus.skill_router.lock().await;
        let suggestions = sr.suggest(message, 0.6, 1);
        if let Some(suggestion) = suggestions.first() {
            info!(skill = %suggestion.name, confidence = suggestion.confidence, "Skill suggested");
            effective_message.push_str(&format!(
                "\n[Suggested skill] /{} (confidence: {:.2}) — {}\n",
                suggestion.name, suggestion.confidence, suggestion.description
            ));
        }
    }

    pub(crate) async fn decay_stale_facts(&self) {
        let fs = self.subsystems.memory.fact_store.lock().await;
        let _ = fs.decay_stale();
    }
}
