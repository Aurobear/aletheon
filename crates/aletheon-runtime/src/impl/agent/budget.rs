//! Token budget management for AgentProcess.
//!
//! Each agent has a per-pulse energy budget. When the pulse arrives,
//! the agent claims tokens from the pulse and consumes them during ReAct execution.

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Manages token budget for an agent process.
pub struct TokenBudget {
    /// Maximum tokens per pulse.
    max_per_pulse: u32,
    /// Remaining tokens in current pulse.
    remaining: AtomicU32,
    /// Total tokens consumed across all pulses.
    total_consumed: AtomicU64,
}

impl TokenBudget {
    pub fn new(max_per_pulse: u32) -> Self {
        Self {
            max_per_pulse,
            remaining: AtomicU32::new(0),
            total_consumed: AtomicU64::new(0),
        }
    }

    /// Claim tokens from a pulse. Returns the amount claimed.
    pub fn claim(&self, pulse_available: u32) -> u32 {
        let claim = pulse_available.min(self.max_per_pulse);
        self.remaining.store(claim, Ordering::SeqCst);
        claim
    }

    /// Consume tokens. Returns remaining budget.
    pub fn consume(&self, tokens: u32) -> u32 {
        let current = self.remaining.load(Ordering::SeqCst);
        let actual = tokens.min(current);
        self.remaining.fetch_sub(actual, Ordering::SeqCst);
        self.total_consumed.fetch_add(actual as u64, Ordering::Relaxed);
        self.remaining.load(Ordering::SeqCst)
    }

    /// Check if budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.remaining.load(Ordering::SeqCst) == 0
    }

    /// Get remaining tokens.
    pub fn remaining(&self) -> u32 {
        self.remaining.load(Ordering::SeqCst)
    }

    /// Get total consumed.
    pub fn total_consumed(&self) -> u64 {
        self.total_consumed.load(Ordering::Relaxed)
    }

    /// Reset for new pulse.
    pub fn reset(&self, pulse_available: u32) {
        let claim = pulse_available.min(self.max_per_pulse);
        self.remaining.store(claim, Ordering::SeqCst);
    }
}

impl fmt::Debug for TokenBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenBudget")
            .field("max_per_pulse", &self.max_per_pulse)
            .field("remaining", &self.remaining.load(Ordering::Relaxed))
            .field("total_consumed", &self.total_consumed.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_budget() {
        let budget = TokenBudget::new(1000);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.total_consumed(), 0);
        assert!(budget.is_exhausted());
    }

    #[test]
    fn test_claim() {
        let budget = TokenBudget::new(500);
        let claimed = budget.claim(1000);
        assert_eq!(claimed, 500); // capped at max_per_pulse
        assert_eq!(budget.remaining(), 500);
        assert!(!budget.is_exhausted());
    }

    #[test]
    fn test_claim_partial() {
        let budget = TokenBudget::new(1000);
        let claimed = budget.claim(300);
        assert_eq!(claimed, 300);
        assert_eq!(budget.remaining(), 300);
    }

    #[test]
    fn test_consume() {
        let budget = TokenBudget::new(1000);
        budget.claim(500);

        let remaining = budget.consume(200);
        assert_eq!(remaining, 300);
        assert_eq!(budget.total_consumed(), 200);
    }

    #[test]
    fn test_consume_more_than_remaining() {
        let budget = TokenBudget::new(1000);
        budget.claim(100);

        let remaining = budget.consume(200);
        assert_eq!(remaining, 0);
        assert_eq!(budget.total_consumed(), 100); // only actual consumed
        assert!(budget.is_exhausted());
    }

    #[test]
    fn test_reset() {
        let budget = TokenBudget::new(1000);
        budget.claim(500);
        budget.consume(300);

        budget.reset(800);
        assert_eq!(budget.remaining(), 800);
        // total_consumed is preserved across resets
        assert_eq!(budget.total_consumed(), 300);
    }
}
