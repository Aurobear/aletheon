/// Configuration for the cognitive engine.
pub struct EngineConfig {
    pub max_iterations: usize,
    pub compact_threshold_tokens: usize,
    pub system_prompt: String,
    pub session_id: String,
    pub compaction_enabled: bool,
    pub compaction_keep_recent: usize,
    pub compaction_threshold: usize,
    /// Token budget for tail protection (soft ceiling = tail_token_budget * soft_multiplier)
    pub tail_token_budget: usize,
    /// Target summary length in characters after compaction
    pub target_summary_chars: usize,
    /// Context window size in tokens (used for 80% auto-compaction threshold).
    pub context_window_tokens: usize,
    /// Enable the learning module for outcome recording and pattern matching.
    pub learning_enabled: bool,
    /// Minimum occurrences before a pattern is considered significant.
    pub learning_min_occurrences: usize,
    /// Success threshold (0.0-1.0) below which a warning rule is created.
    pub learning_success_threshold: f64,
    /// Maximum number of learned rules to keep in memory.
    pub learning_max_rules: usize,
    /// Optional CommunicationBus for inter-module communication.
    /// Replaces the old event_bus; provides request-response, pub-sub, and module mailbox APIs.
    pub bus: Option<std::sync::Arc<fabric::CommunicationBus>>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            compact_threshold_tokens: 100_000,
            system_prompt:
                "You are a helpful system assistant. You can execute commands and manage files."
                    .to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            compaction_enabled: true,
            compaction_keep_recent: 10,
            compaction_threshold: 30,
            tail_token_budget: 4000,
            target_summary_chars: 2000,
            context_window_tokens: 128_000,
            learning_enabled: false,
            learning_min_occurrences: 3,
            learning_success_threshold: 0.5,
            learning_max_rules: 100,
            bus: None,
        }
    }
}
