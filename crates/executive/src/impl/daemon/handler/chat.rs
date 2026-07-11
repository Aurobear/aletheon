// Handler module migrated to CommunicationBus — event_bus field is now Arc<CommunicationBus>.
//
// NOTE: The old helper methods (inject_*, record_turn_reflection, etc.) are
// deprecated — the live orchestration now lives in `DaemonTurnOrchestrator`.
// These remain for backward compatibility during migration and will be removed
// in a follow-up cleanup PR.

#![allow(dead_code)]

use super::RequestHandler;

use tracing::{info, warn};

use fabric::hook::{HookContext, HookPoint};
use fabric::{ContentBlock, Message, ReflectionTrigger, Role};
use std::collections::HashMap;

use cognit::harness::linear::TurnMetrics;
use mnemosyne::FactStore;

const MAX_ACTIVATED_SKILL_CHARS: usize = 12 * 1024;
const MAX_ACTIVATED_SKILLS_TOTAL_CHARS: usize = 24 * 1024;
const MAX_RECALLED_FACT_CHARS: usize = 2 * 1024;
const MAX_RECALL_TOTAL_CHARS: usize = 8 * 1024;
const MAX_HISTORY_MESSAGE_CHARS: usize = 16 * 1024;
const MAX_HISTORY_TOTAL_CHARS: usize = 64 * 1024;
const MAX_HISTORY_MESSAGES: usize = 6;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let truncated: String = value.chars().take(max_chars - 1).collect();
    format!("{truncated}…")
}

fn append_bounded_text(target: &mut String, value: &str, per_item: usize, remaining: &mut usize) {
    if *remaining == 0 {
        return;
    }
    let bounded = truncate_chars(value, per_item.min(*remaining));
    *remaining = (*remaining).saturating_sub(bounded.chars().count());
    target.push_str(&bounded);
}

/// Return only bounded conversational text. Tool blocks and historical system
/// prompts are deliberately excluded so a restored session cannot replay
/// transient prompt decorations or malformed tool-call sequences.
fn bounded_text_history(history: &[Message]) -> Vec<Message> {
    let mut remaining = MAX_HISTORY_TOTAL_CHARS;
    let mut selected = Vec::new();

    for message in history.iter().rev() {
        if selected.len() >= MAX_HISTORY_MESSAGES || remaining == 0 {
            break;
        }
        if !matches!(message.role, Role::User | Role::Assistant) {
            continue;
        }
        let text = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }
        let bounded = truncate_chars(&text, MAX_HISTORY_MESSAGE_CHARS.min(remaining));
        remaining = remaining.saturating_sub(bounded.chars().count());
        selected.push(match message.role {
            Role::User => Message::user(bounded),
            Role::Assistant => Message::assistant(bounded),
            Role::System => unreachable!(),
        });
    }

    selected.reverse();
    selected
}

fn build_request_messages(
    system_prompt: String,
    history: &[Message],
    effective_message: String,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(MAX_HISTORY_MESSAGES + 2);
    messages.push(Message::system(system_prompt));
    messages.extend(bounded_text_history(history));
    messages.push(Message::user(effective_message));
    messages
}

impl RequestHandler {
    // -- Pre-turn injection phases (RFC-018 D5 / issue #4) -----------------
    // Each appends to the effective user message; none has early-return
    // control flow, so they extract cleanly out of `handle_chat`.

    /// Keyword-triggered skill injection: match loaded skills against the user
    /// message and append their bodies (bounded).
    async fn inject_keyword_skills(&self, message: &str, effective_message: &mut String) {
        let loader = self.subsystems.skill_loader.lock().await;
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

    /// Recall relevant facts from the FactStore (keyword search + entity-graph
    /// boost) and append them to the user turn.
    async fn inject_fact_recall(&self, message: &str, effective_message: &mut String) {
        let fs = self.subsystems.fact_store.lock().await;
        let keywords: Vec<String> = message
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_lowercase())
            .collect();
        let query = keywords.join(" ");
        if query.len() >= 8 {
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
                    // Entity graph boost
                    let entities = FactStore::extract_entities(message);
                    for entity in entities.iter().take(3) {
                        if let Ok(eid) = fs.resolve_entity(entity) {
                            if let Ok(related) = fs.get_entity_facts(eid) {
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
                            }
                        }
                    }
                    info!(count = facts.len(), "Fact recall injected");
                    effective_message.push_str(&recall_block);
                }
            }
        }
    }

    /// Inject the current (post-boot) CoreMemory state so the model sees
    /// up-to-date writable blocks (CoreMemory is baked into the boot prefix,
    /// but core_memory_append / AutoMemory mutate it in-memory afterward).
    async fn inject_core_memory(&self, effective_message: &mut String) {
        let cm = self.subsystems.core_memory.lock().await;
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

    /// Suggest a skill via the SkillRouter (advisory hint, not auto-activated).
    async fn inject_skill_suggestion(&self, message: &str, effective_message: &mut String) {
        let sr = self.subsystems.skill_router.lock().await;
        let suggestions = sr.suggest(message, 0.6, 1);
        if let Some(suggestion) = suggestions.first() {
            info!(skill = %suggestion.name, confidence = suggestion.confidence, "Skill suggested");
            effective_message.push_str(&format!(
                "\n[Suggested skill] /{} (confidence: {:.2}) — {}\n",
                suggestion.name, suggestion.confidence, suggestion.description
            ));
        }
    }

    /// Periodic stale-fact decay (fire-and-forget maintenance).
    async fn decay_stale_facts(&self) {
        let fs = self.subsystems.fact_store.lock().await;
        let _ = fs.decay_stale();
    }

    // -- Post-turn phases (RFC-018 D5 / issue #4) --------------------------
    // Cohesive tail-of-turn steps consuming loop outputs (text, metrics).
    // None has early-return control flow, so each extracts cleanly.

    /// Fire the PostTurn lifecycle hooks.
    async fn run_post_turn_hooks(&self) {
        // Gather session info before locking hook_registry
        let (session_id, turn_count) = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let sm = sm_arc.lock().await;
            (sm.session_id.clone(), sm.turn_count())
        };
        let hr = self.subsystems.hook_registry.lock().await;
        let ctx = HookContext {
            point: HookPoint::PostTurn,
            session_id,
            turn_count,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        hr.execute(&ctx).await;
    }

    /// Auto-memory extraction: cheap-LLM fact extraction from the turn.
    async fn extract_auto_memory(&self, message: &str, text: &str) {
        let mut am = self.subsystems.auto_memory.lock().await;
        if let Ok(facts) = am.analyze_and_store(message, text).await {
            if !facts.is_empty() {
                info!(count = facts.len(), "Auto-memory: stored facts");
            }
        }
    }

    /// Enhanced reflection: score question/response quality, store the
    /// reflection, and run the periodic ExperienceSummarizer trigger.
    async fn record_turn_reflection(&self, task_summary: &str, text: &str, turn: usize) {
        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();

        // Response length as a quality indicator
        let resp_len = text.len();
        if resp_len > 500 {
            what_worked.push(format!("Detailed response ({} chars)", resp_len));
        } else if resp_len > 100 {
            what_worked.push(format!("Concise response ({} chars)", resp_len));
        } else {
            what_worked.push(format!("Brief response ({} chars)", resp_len));
        }

        // Detect error indicators in response
        let text_lower = text.to_lowercase();
        let error_indicators = [
            "error",
            "failed",
            "unable",
            "cannot",
            "couldn't",
            "sorry, i",
            "i don't know",
        ];
        for indicator in &error_indicators {
            if text_lower.contains(indicator) {
                what_failed.push(format!("Response contains '{}'", indicator));
            }
        }

        // Detect learning/self-correction indicators
        let learning_indicators = [
            "i learned",
            "i now understand",
            "i realize",
            "correction:",
            "actually,",
        ];
        for indicator in &learning_indicators {
            if text_lower.contains(indicator) {
                learned.push(format!("Self-correction detected: '{}'", indicator));
            }
        }

        // Track turn context
        what_worked.push(format!("Conversation turn #{}", turn));

        let has_failures = !what_failed.is_empty();
        let entry = self.subsystems.reflector.reflect_conversation(
            task_summary,
            ReflectionTrigger::TaskComplete,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        // Store reflection — drop lock guard before re-locking for evolution check
        let store_result = {
            let mem = self.subsystems.episodic_memory.lock().await;
            mem.store_reflection(&entry)
        };
        if let Err(e) = store_result {
            warn!(error = %e, "Failed to store chat reflection");
        } else {
            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");

            // Periodic evolution trigger: every 10 reflections, run ExperienceSummarizer
            let mem = self.subsystems.episodic_memory.lock().await;
            if let Ok(count) = mem.reflection_count() {
                if count > 0 && count % 10 == 0 {
                    info!(
                        count = count,
                        "Running ExperienceSummarizer (periodic trigger)"
                    );
                    if let Ok(recent) = mem.recall_reflections(20) {
                        if let Some(evo_entry) =
                            cognit::core::ExperienceSummarizer::summarize(&recent)
                        {
                            if let Err(e) = mem.store_evolution_log(&evo_entry) {
                                warn!(error = %e, "Failed to store evolution log");
                            } else {
                                info!(id = %evo_entry.id, patterns = evo_entry.patterns_detected.len(), "Evolution log stored");
                            }
                        }
                    }
                }
            }
        }
    }

    /// EvolutionCoordinator: post-turn evolution (accumulates reflections,
    /// triggers every N turns).
    async fn run_post_evolution(&self, task_summary: &str, text: &str, metrics: &TurnMetrics) {
        let success = metrics.completed_normally && !text.starts_with("error:");
        if let Err(e) = self
            .subsystems
            .runtime
            .lock()
            .await
            .post_evolution(
                task_summary,
                text,
                success,
                metrics.tool_calls_made,
                metrics.tool_errors,
                metrics.elapsed_ms,
                metrics.iterations,
                &*self.subsystems.pipeline,
            )
            .await
        {
            warn!(error = %e, "post_evolution failed");
        }
    }

    pub(super) async fn handle_chat(
        &self,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let message = request["params"]["message"].as_str().unwrap_or("");
        info!(message = %message, "Chat request received");
        self.turn_orchestrator.execute_turn(id, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(message: &Message) -> &str {
        match &message.content[0] {
            ContentBlock::Text { text } => text,
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn bounded_history_excludes_system_and_tool_blocks() {
        let history = vec![
            Message::system("large transient prefix"),
            Message::user("raw user"),
            Message::tool_result("call-1", "tool output", false),
            Message::assistant("raw assistant"),
        ];

        let bounded = bounded_text_history(&history);

        assert_eq!(bounded.len(), 2);
        assert_eq!(text_of(&bounded[0]), "raw user");
        assert_eq!(text_of(&bounded[1]), "raw assistant");
    }

    #[test]
    fn bounded_history_caps_restored_injected_payloads() {
        let huge = format!("<activated-skill>{}</activated-skill>", "x".repeat(200_000));
        let history = vec![Message::user(huge)];

        let bounded = bounded_text_history(&history);

        assert_eq!(bounded.len(), 1);
        assert!(text_of(&bounded[0]).chars().count() <= MAX_HISTORY_MESSAGE_CHARS);
    }

    #[test]
    fn bounded_text_is_utf8_safe_and_respects_budget() {
        let mut output = String::new();
        let mut remaining = 8;

        append_bounded_text(&mut output, "机器人上下文非常长", 6, &mut remaining);

        assert!(output.is_char_boundary(output.len()));
        assert!(output.chars().count() <= 6);
        assert!(remaining <= 2);
    }

    #[test]
    fn request_contains_one_system_prefix_and_one_ephemeral_user_message() {
        let history = vec![
            Message::system("old prefix that must not be replayed"),
            Message::user("raw prior user"),
            Message::assistant("raw prior assistant"),
        ];

        let messages = build_request_messages(
            "current prefix".into(),
            &history,
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user".into(),
        );

        assert_eq!(
            messages.iter().filter(|m| m.role == Role::System).count(),
            1
        );
        assert_eq!(text_of(&messages[0]), "current prefix");
        assert_eq!(
            text_of(messages.last().unwrap()),
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user"
        );
        assert!(!messages.iter().any(|m| text_of(m).contains("old prefix")));
    }
}
