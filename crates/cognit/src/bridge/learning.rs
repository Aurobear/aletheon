use crate::r#impl::learning::{
    LearnRule, OutcomeContext, OutcomeRecord, OutcomeRecorder, PatternExtractor, RuleStore,
};
use anyhow::Result;
use fabric::body::{Action, ActionResult};
use fabric::cognit::{Experience, LearnedRule};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Bridges learning pipeline into CognitCore
pub struct LearningBridge {
    outcome_recorder: OutcomeRecorder,
    pattern_extractor: PatternExtractor,
    rule_store: Arc<Mutex<RuleStore>>,
}

impl LearningBridge {
    pub fn new(db_path: PathBuf, max_rules: usize) -> Self {
        Self {
            outcome_recorder: OutcomeRecorder::new(db_path),
            pattern_extractor: PatternExtractor::new(3, 0.7),
            rule_store: Arc::new(Mutex::new(RuleStore::new(max_rules))),
        }
    }

    /// Record a tool execution outcome
    pub fn record_outcome(
        &self,
        action: &Action,
        result: &ActionResult,
        session_id: &str,
    ) -> Result<()> {
        let outcome = OutcomeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            turn_id: String::new(),
            tool_name: action.name.clone(),
            args: action.parameters.clone(),
            result_summary: result.output.chars().take(200).collect(),
            is_error: !result.success,
            user_feedback: None,
            timestamp: chrono::Utc::now(),
            context: OutcomeContext::default(),
        };
        self.outcome_recorder.record(&outcome)
    }

    /// Extract patterns from recent outcomes and update rule store
    pub fn extract_and_update(&self) -> Result<Vec<LearnRule>> {
        let outcomes = self.outcome_recorder.get_recent(100)?;
        let new_rules = self.pattern_extractor.extract(&outcomes);
        let mut store = self.rule_store.lock().unwrap();
        for rule in &new_rules {
            store.add(rule.clone());
        }
        Ok(new_rules)
    }

    /// Get learned rules formatted for LLM context injection
    pub fn rules_for_context(&self) -> String {
        let store = self.rule_store.lock().unwrap();
        store.format_for_context()
    }

    /// Get rules relevant to a specific tool
    pub fn rules_for_tool(&self, tool_name: &str) -> Vec<LearnRule> {
        let store = self.rule_store.lock().unwrap();
        store.get_for_tool(tool_name).into_iter().cloned().collect()
    }

    /// Convert aletheon Experience to OutcomeRecord for learning
    pub fn experience_to_outcome(experience: &Experience, session_id: &str) -> OutcomeRecord {
        OutcomeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            turn_id: String::new(),
            tool_name: experience.action.name.clone(),
            args: experience.action.parameters.clone(),
            result_summary: experience.result.output.chars().take(200).collect(),
            is_error: !experience.result.success,
            user_feedback: None,
            timestamp: chrono::Utc::now(),
            context: OutcomeContext::default(),
        }
    }

    /// Convert LearnRule to LearnedRule
    pub fn to_learned_rule(rule: &LearnRule) -> LearnedRule {
        LearnedRule {
            id: rule.id.clone(),
            pattern: rule.condition.clone(),
            action: rule.action.clone(),
            confidence: rule.confidence,
            examples: rule.examples.clone(),
        }
    }
}
