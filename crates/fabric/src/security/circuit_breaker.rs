use super::loop_detector::LoopVerdict;
use std::collections::{HashMap, VecDeque};

const CONSECUTIVE_BLOCK_THRESHOLD: usize = 3;
const WINDOW_SIZE: usize = 50;
const WINDOW_BLOCK_THRESHOLD: usize = 10;

pub struct LoopCircuitBreaker {
    per_turn: HashMap<String, TurnBreakerState>,
}

struct TurnBreakerState {
    consecutive_blocks: usize,
    recent_blocks: VecDeque<bool>,
    interrupted: bool,
}

impl Default for LoopCircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopCircuitBreaker {
    pub fn new() -> Self {
        Self {
            per_turn: HashMap::new(),
        }
    }

    pub fn on_new_turn(&mut self, turn_id: &str) {
        self.per_turn.insert(
            turn_id.to_string(),
            TurnBreakerState {
                consecutive_blocks: 0,
                recent_blocks: VecDeque::new(),
                interrupted: false,
            },
        );
    }

    pub fn record_block(&mut self, turn_id: &str) {
        if let Some(state) = self.per_turn.get_mut(turn_id) {
            state.consecutive_blocks += 1;
            state.recent_blocks.push_back(true);
            while state.recent_blocks.len() > WINDOW_SIZE {
                state.recent_blocks.pop_front();
            }
        }
    }

    pub fn check(&mut self, turn_id: &str) -> Option<LoopVerdict> {
        let state = self.per_turn.get(turn_id)?;
        if state.interrupted {
            return Some(LoopVerdict::InterruptTurn {
                reason: "Turn already interrupted by circuit breaker".into(),
                consecutive_blocks: state.consecutive_blocks,
            });
        }

        if state.consecutive_blocks >= CONSECUTIVE_BLOCK_THRESHOLD {
            return Some(LoopVerdict::InterruptTurn {
                reason: format!(
                    "Consecutive blocks: {} (threshold: {})",
                    state.consecutive_blocks, CONSECUTIVE_BLOCK_THRESHOLD
                ),
                consecutive_blocks: state.consecutive_blocks,
            });
        }

        let window_blocks = state.recent_blocks.iter().filter(|&&b| b).count();
        if window_blocks >= WINDOW_BLOCK_THRESHOLD {
            return Some(LoopVerdict::InterruptTurn {
                reason: format!(
                    "Blocks in window: {window_blocks} (threshold: {WINDOW_BLOCK_THRESHOLD})"
                ),
                consecutive_blocks: state.consecutive_blocks,
            });
        }

        None
    }

    pub fn end_turn(&mut self, turn_id: &str) {
        self.per_turn.remove(turn_id);
    }
}
