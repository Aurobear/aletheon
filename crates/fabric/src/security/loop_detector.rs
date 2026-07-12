use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

use super::circuit_breaker::LoopCircuitBreaker;
use super::risk_classifier::RiskClassifier;
use crate::types::tool::ToolResult;

#[derive(Debug, Clone)]
pub struct LoopDetectorConfig {
    pub window_size: usize,
    pub stagnation_token_delta: usize,
    pub stagnation_window: usize,
}

impl Default for LoopDetectorConfig {
    fn default() -> Self {
        Self {
            window_size: 50,
            stagnation_token_delta: 100,
            stagnation_window: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopVerdict {
    Allow,
    Warn {
        reason: String,
    },
    Block {
        reason: String,
        suggestion: String,
    },
    Escalate {
        reason: String,
    },
    InterruptTurn {
        reason: String,
        consecutive_blocks: usize,
    },
}

#[derive(Debug, Clone)]
struct ToolCallRecord {
    tool_name: String,
    args_hash: u64,
    is_error: bool,
    #[allow(dead_code)]
    token_cost: usize,
    #[allow(dead_code)]
    turn_id: String,
}

#[derive(Debug, Default)]
struct TurnHistory {
    calls: VecDeque<ToolCallRecord>,
}

pub struct LoopDetector {
    config: LoopDetectorConfig,
    risk_classifier: RiskClassifier,
    per_turn: HashMap<String, TurnHistory>,
    circuit_breaker: LoopCircuitBreaker,
    pub metrics: LoopDetectorMetrics,
}

#[derive(Debug, Default)]
pub struct LoopDetectorMetrics {
    pub total_checks: u64,
    pub allows: u64,
    pub warnings: u64,
    pub blocks: u64,
    pub escalations: u64,
    pub circuit_breaker_trips: u64,
}

impl LoopDetector {
    pub fn new(config: LoopDetectorConfig) -> Self {
        Self {
            config,
            risk_classifier: RiskClassifier::with_defaults(),
            per_turn: HashMap::new(),
            circuit_breaker: LoopCircuitBreaker::new(),
            metrics: LoopDetectorMetrics::default(),
        }
    }

    pub fn on_new_turn(&mut self, turn_id: &str) {
        self.per_turn
            .insert(turn_id.to_string(), TurnHistory::default());
        self.circuit_breaker.on_new_turn(turn_id);
    }

    pub fn pre_check(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
        turn_id: &str,
    ) -> LoopVerdict {
        self.metrics.total_checks += 1;

        let category = self.risk_classifier.classify(tool_name);
        let thresholds = category.thresholds();
        let args_hash = hash_args(args);

        let history = self.per_turn.entry(turn_id.to_string()).or_default();

        // 1. Same-call detection
        let same_count = history
            .calls
            .iter()
            .rev()
            .take_while(|r| r.tool_name == tool_name && r.args_hash == args_hash)
            .count();

        if same_count >= thresholds.same_call_threshold {
            self.metrics.blocks += 1;
            let verdict = LoopVerdict::Block {
                reason: format!(
                    "Same tool+args repeated {} times (threshold: {})",
                    same_count + 1,
                    thresholds.same_call_threshold
                ),
                suggestion: "Try a different approach or ask for help".into(),
            };
            self.circuit_breaker.record_block(turn_id);
            return verdict;
        }

        // 2. Fail-streak detection
        let fail_streak = history
            .calls
            .iter()
            .rev()
            .take_while(|r| r.is_error)
            .count();

        if fail_streak >= thresholds.fail_streak_threshold {
            self.metrics.escalations += 1;
            return LoopVerdict::Escalate {
                reason: format!(
                    "Consecutive failures: {} (threshold: {})",
                    fail_streak + 1,
                    thresholds.fail_streak_threshold
                ),
            };
        }

        // 3. Stagnation detection
        if history.calls.len() >= self.config.stagnation_window {
            let recent: Vec<_> = history
                .calls
                .iter()
                .rev()
                .take(self.config.stagnation_window)
                .collect();
            let any_success = recent.iter().any(|r| !r.is_error);
            if !any_success {
                self.metrics.warnings += 1;
                return LoopVerdict::Warn {
                    reason: format!(
                        "No successful calls in last {} attempts",
                        self.config.stagnation_window
                    ),
                };
            }
        }

        // 4. Circuit breaker check
        if let Some(verdict) = self.circuit_breaker.check(turn_id) {
            self.metrics.circuit_breaker_trips += 1;
            return verdict;
        }

        self.metrics.allows += 1;
        LoopVerdict::Allow
    }

    pub fn post_check(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
        result: &ToolResult,
        turn_id: &str,
    ) {
        let history = self.per_turn.entry(turn_id.to_string()).or_default();
        history.calls.push_back(ToolCallRecord {
            tool_name: tool_name.to_string(),
            args_hash: hash_args(args),
            is_error: result.is_error,
            token_cost: result.content.len(),
            turn_id: turn_id.to_string(),
        });

        // Trim window
        while history.calls.len() > self.config.window_size {
            history.calls.pop_front();
        }
    }

    pub fn end_turn(&mut self, turn_id: &str) {
        self.per_turn.remove(turn_id);
        self.circuit_breaker.end_turn(turn_id);
    }
}

fn hash_args(args: &serde_json::Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    args.to_string().hash(&mut hasher);
    hasher.finish()
}
