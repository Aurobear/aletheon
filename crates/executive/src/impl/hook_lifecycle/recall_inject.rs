//! Recall Injector — on UserPromptSubmit, recalls relevant facts, entities,
//! knowledge, and skill suggestions to inject as additional context.

use anyhow::Result;
use std::path::Path;
use std::sync::Mutex;

use super::{Hook, HookEvent, HookResult};
use corpus::skill::router::SkillRouter;
use mnemosyne::FactStore;

const MIN_PROMPT_LEN: usize = 8;
const MAX_FACTS: usize = 4;
const MAX_KNOWLEDGE: usize = 2;
const MAX_ENTITIES: usize = 3;
const _TRUST_BUMP: f64 = 0.005;
const MIN_TRUST: f64 = 0.15;
const SKILL_MIN_PROMPT_LEN: usize = 80;

/// Debug keywords that bypass the short-prompt gate.
const DEBUG_KEYWORDS: &[&str] = &["debug", "error", "bug", "fix", "crash", "fail"];

pub struct RecallInjector {
    fact_store: Mutex<FactStore>,
    skill_router: SkillRouter,
}

impl RecallInjector {
    pub fn new(fact_store: FactStore, skill_router: SkillRouter) -> Self {
        Self {
            fact_store: Mutex::new(fact_store),
            skill_router,
        }
    }

    /// Convenience constructor: open FactStore at path, empty SkillRouter.
    pub fn open(db_path: &Path) -> Result<Self> {
        let fact_store = FactStore::open(db_path)?;
        Ok(Self {
            fact_store: Mutex::new(fact_store),
            skill_router: SkillRouter::new(),
        })
    }

    /// Set the skill router.
    pub fn with_skill_router(mut self, router: SkillRouter) -> Self {
        self.skill_router = router;
        self
    }

    /// Get a reference to the fact store (for testing).
    #[cfg(test)]
    fn fact_store(&self) -> &Mutex<FactStore> {
        &self.fact_store
    }
}

impl Hook for RecallInjector {
    fn name(&self) -> &str {
        "recall-inject"
    }

    fn handle(&self, event: &HookEvent) -> Result<HookResult> {
        let prompt = match event {
            HookEvent::UserPromptSubmit { prompt } => prompt,
            _ => return Ok(HookResult::Noop),
        };

        // ── Trivial prompt gate ──────────────────────────────────────────
        if prompt.len() < MIN_PROMPT_LEN {
            // Allow short prompts that contain debug keywords
            let lower = prompt.to_lowercase();
            let has_debug = DEBUG_KEYWORDS.iter().any(|kw| lower.contains(kw));
            if !has_debug {
                return Ok(HookResult::Noop);
            }
        }

        // Skip slash commands
        if prompt.trim_start().starts_with('/') {
            return Ok(HookResult::Noop);
        }

        // ── Extract keywords for FTS query ───────────────────────────────
        let keywords = extract_keywords(prompt);
        if keywords.is_empty() {
            return Ok(HookResult::Noop);
        }
        let query = keywords.join(" OR ");

        let mut injected_parts: Vec<String> = Vec::new();
        let store = self.fact_store.lock().unwrap_or_else(|e| e.into_inner());

        // ── FTS5 recall ──────────────────────────────────────────────────
        let facts = store
            .search_facts(&query, None, MIN_TRUST, MAX_FACTS)
            .unwrap_or_default();

        if !facts.is_empty() {
            let mut lines = Vec::new();
            for fact in &facts {
                lines.push(format!(
                    "- [trust={:.2}] {}",
                    fact.trust_score, fact.content
                ));
                // Trust recall boost
                let _ = store.record_feedback(fact.fact_id, true);
            }
            injected_parts.push(format!("Recalled facts:\n{}", lines.join("\n")));
        }

        // ── Entity graph boost ───────────────────────────────────────────
        let entities = extract_entities_from_prompt(prompt);
        let mut entity_facts_count = 0usize;
        for entity in entities.iter().take(MAX_ENTITIES) {
            if let Ok(eid) = store.resolve_entity(entity) {
                if let Ok(efacts) = store.get_entity_facts(eid) {
                    for ef in efacts.iter().take(1) {
                        // Skip facts we already recalled
                        if facts.iter().any(|f| f.fact_id == ef.fact_id) {
                            continue;
                        }
                        injected_parts.push(format!("- [entity:{}] {}", entity, ef.content));
                        entity_facts_count += 1;
                        if entity_facts_count >= MAX_ENTITIES {
                            break;
                        }
                    }
                }
            }
            if entity_facts_count >= MAX_ENTITIES {
                break;
            }
        }

        // ── Knowledge recall ─────────────────────────────────────────────
        let knowledge = store
            .search_knowledge(&query, MAX_KNOWLEDGE)
            .unwrap_or_default();

        if !knowledge.is_empty() {
            let lines: Vec<String> = knowledge
                .iter()
                .map(|k| format!("- [knowledge] {}: {}", k.topic, k.content))
                .collect();
            injected_parts.push(format!("Recalled knowledge:\n{}", lines.join("\n")));
        }

        drop(store);

        // ── Skill routing (for longer prompts) ───────────────────────────
        if prompt.len() >= SKILL_MIN_PROMPT_LEN {
            let suggestions = self.skill_router.suggest(prompt, 0.3, 2);
            if !suggestions.is_empty() {
                let lines: Vec<String> = suggestions
                    .iter()
                    .map(|s| {
                        format!(
                            "- [skill:{}] {} (confidence: {:.0}%)",
                            s.name,
                            s.description,
                            s.confidence * 100.0
                        )
                    })
                    .collect();
                injected_parts.push(format!("Suggested skills:\n{}", lines.join("\n")));
            }
        }

        if injected_parts.is_empty() {
            return Ok(HookResult::Noop);
        }

        Ok(HookResult::Inject {
            context: injected_parts.join("\n\n"),
        })
    }
}

/// Extract significant keywords from a prompt for FTS search.
fn extract_keywords(prompt: &str) -> Vec<String> {
    let stop_words: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "need", "dare", "ought", "used", "to", "of", "in", "for", "on", "with", "at", "by", "from",
        "as", "into", "through", "during", "before", "after", "above", "below", "between", "out",
        "off", "over", "under", "again", "further", "then", "once", "here", "there", "when",
        "where", "why", "how", "all", "both", "each", "few", "more", "most", "other", "some",
        "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too", "very", "just",
        "don", "now", "and", "but", "or", "if", "this", "that", "these", "those", "i", "me", "my",
        "we", "our", "you", "your", "he", "him", "his", "she", "her", "it", "its", "they", "them",
        "their", "what", "which", "who", "whom",
    ];

    prompt
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| w.len() >= 2 && !stop_words.contains(&w.as_str()))
        .collect()
}

/// Extract capitalized words from prompt as potential entity names.
fn extract_entities_from_prompt(prompt: &str) -> Vec<String> {
    let mut entities = Vec::new();
    for word in prompt.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if clean.len() >= 2
            && clean.chars().next().is_some_and(|c| c.is_uppercase())
            && !clean.chars().all(|c| c.is_uppercase())
        {
            entities.push(clean);
        }
    }
    entities.sort();
    entities.dedup();
    entities
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup() -> (RecallInjector, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let injector = RecallInjector::open(tmp.path()).unwrap();
        (injector, tmp)
    }

    #[test]
    fn test_trivial_gate_short() {
        let (injector, _tmp) = setup();
        let event = HookEvent::UserPromptSubmit {
            prompt: "hi".to_string(),
        };
        let result = injector.handle(&event).unwrap();
        assert!(matches!(result, HookResult::Noop));
    }

    #[test]
    fn test_trivial_gate_slash_command() {
        let (injector, _tmp) = setup();
        let event = HookEvent::UserPromptSubmit {
            prompt: "/workflow bugfix".to_string(),
        };
        let result = injector.handle(&event).unwrap();
        assert!(matches!(result, HookResult::Noop));
    }

    #[test]
    fn test_recall_facts() {
        let (injector, _tmp) = setup();
        {
            let store = injector.fact_store().lock().unwrap();
            store
                .add_fact(
                    "Kuavo robot uses EtherCAT bus",
                    "hardware",
                    "robot",
                    "",
                    0.8,
                    "semantic",
                    0,
                )
                .unwrap();
            store
                .add_fact(
                    "ROS Noetic is the target runtime",
                    "software",
                    "ros",
                    "",
                    0.7,
                    "semantic",
                    0,
                )
                .unwrap();
        }

        let event = HookEvent::UserPromptSubmit {
            prompt: "How does the Kuavo robot communicate?".to_string(),
        };
        let result = injector.handle(&event).unwrap();
        match result {
            HookResult::Inject { context } => {
                assert!(context.contains("Recalled facts"));
                assert!(context.contains("EtherCAT"));
            }
            _ => panic!("Expected Inject, got {:?}", result),
        }
    }

    #[test]
    fn test_recall_entity_boost() {
        let (injector, _tmp) = setup();
        {
            let store = injector.fact_store().lock().unwrap();
            store
                .add_fact(
                    "Alice works at Acme Corp on robotics",
                    "people",
                    "",
                    "",
                    0.8,
                    "semantic",
                    0,
                )
                .unwrap();
        }

        let event = HookEvent::UserPromptSubmit {
            prompt: "What has Alice been working on recently?".to_string(),
        };
        let result = injector.handle(&event).unwrap();
        match result {
            HookResult::Inject { context } => {
                // Should contain entity-based recall
                assert!(context.contains("Alice") || context.contains("Acme"));
            }
            _ => panic!("Expected Inject, got {:?}", result),
        }
    }

    #[test]
    fn test_recall_trust_bump() {
        let (injector, _tmp) = setup();
        let fid = {
            let store = injector.fact_store().lock().unwrap();
            store
                .add_fact(
                    "Trust bump test fact about robots",
                    "test",
                    "",
                    "",
                    0.5,
                    "semantic",
                    0,
                )
                .unwrap()
        };

        let event = HookEvent::UserPromptSubmit {
            prompt: "Tell me about the trust bump test fact".to_string(),
        };
        let _ = injector.handle(&event);

        let store = injector
            .fact_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let fact = store.get_fact(fid).unwrap().unwrap();
        assert!(
            fact.trust_score > 0.5,
            "trust should have been bumped, got {}",
            fact.trust_score
        );
    }

    #[test]
    fn test_recall_skill_suggestion() {
        let (injector, _tmp) = setup();
        let mut router = SkillRouter::new();
        router.load_from_skill(corpus::skill::router::SkillEntry {
            name: "git".to_string(),
            description: "Git workflow automation".to_string(),
            triggers: vec!["commit".to_string(), "push".to_string()],
            tags: vec!["vcs".to_string()],
            path: std::path::PathBuf::from("/fake"),
        });

        let injector = injector.with_skill_router(router);

        let event = HookEvent::UserPromptSubmit {
            prompt: "I need help to commit my changes to the repository and push them to the remote branch for review".to_string(),
        };
        let result = injector.handle(&event).unwrap();
        match result {
            HookResult::Inject { context } => {
                assert!(context.contains("Suggested skills"));
                assert!(context.contains("git"));
            }
            _ => panic!("Expected Inject, got {:?}", result),
        }
    }
}
