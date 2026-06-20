use std::collections::HashMap;

/// Rate limiter for tool invocations — per-turn cap, concurrency cap,
/// and optional per-tool limits.
pub struct ToolRateLimiter {
    max_per_turn: u32,
    max_concurrent: u32,
    per_tool_limits: HashMap<String, u32>,
    turn_count: u32,
    concurrent_count: u32,
    per_tool_turn_counts: HashMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolLimitResult {
    Allow,
    TurnLimitReached,
    ConcurrencyLimitReached,
    ToolLimitReached(String),
}

impl ToolRateLimiter {
    pub fn new(
        max_per_turn: u32,
        max_concurrent: u32,
        per_tool_limits: HashMap<String, u32>,
    ) -> Self {
        Self {
            max_per_turn,
            max_concurrent,
            per_tool_limits,
            turn_count: 0,
            concurrent_count: 0,
            per_tool_turn_counts: HashMap::new(),
        }
    }

    /// Start a new turn — resets per-turn counters.
    pub fn begin_turn(&mut self) {
        self.turn_count = 0;
        self.per_tool_turn_counts.clear();
    }

    /// Try to start a tool call. Returns `Allow` if within limits.
    pub fn try_acquire(&mut self, tool_name: &str) -> ToolLimitResult {
        // Per-turn overall cap.
        if self.turn_count >= self.max_per_turn {
            return ToolLimitResult::TurnLimitReached;
        }

        // Concurrency cap.
        if self.concurrent_count >= self.max_concurrent {
            return ToolLimitResult::ConcurrencyLimitReached;
        }

        // Per-tool per-turn cap.
        let tool_count = self
            .per_tool_turn_counts
            .get(tool_name)
            .copied()
            .unwrap_or(0);
        if let Some(&limit) = self.per_tool_limits.get(tool_name) {
            if tool_count >= limit {
                return ToolLimitResult::ToolLimitReached(tool_name.to_string());
            }
        }

        // All clear — record.
        self.turn_count += 1;
        self.concurrent_count += 1;
        self.per_tool_turn_counts
            .insert(tool_name.to_string(), tool_count + 1);

        ToolLimitResult::Allow
    }

    /// Signal that a tool call has completed (release concurrency slot).
    pub fn release(&mut self) {
        if self.concurrent_count > 0 {
            self.concurrent_count -= 1;
        }
    }

    pub fn turn_count(&self) -> u32 {
        self.turn_count
    }

    pub fn concurrent_count(&self) -> u32 {
        self.concurrent_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_turn_limit() {
        let mut limiter = ToolRateLimiter::new(5, 10, HashMap::new());
        limiter.begin_turn();
        for i in 0..5 {
            assert_eq!(
                limiter.try_acquire("shell"),
                ToolLimitResult::Allow,
                "call {} should be allowed",
                i
            );
        }
    }

    #[test]
    fn rejects_when_turn_limit_hit() {
        let mut limiter = ToolRateLimiter::new(2, 10, HashMap::new());
        limiter.begin_turn();
        assert_eq!(limiter.try_acquire("a"), ToolLimitResult::Allow);
        assert_eq!(limiter.try_acquire("b"), ToolLimitResult::Allow);
        assert_eq!(limiter.try_acquire("c"), ToolLimitResult::TurnLimitReached);
    }

    #[test]
    fn enforces_concurrency_limit() {
        let mut limiter = ToolRateLimiter::new(10, 2, HashMap::new());
        limiter.begin_turn();
        assert_eq!(limiter.try_acquire("a"), ToolLimitResult::Allow);
        assert_eq!(limiter.try_acquire("b"), ToolLimitResult::Allow);
        assert_eq!(
            limiter.try_acquire("c"),
            ToolLimitResult::ConcurrencyLimitReached
        );
        limiter.release();
        assert_eq!(limiter.try_acquire("c"), ToolLimitResult::Allow);
    }

    #[test]
    fn enforces_per_tool_limit() {
        let mut limits = HashMap::new();
        limits.insert("dangerous_cmd".to_string(), 1);
        let mut limiter = ToolRateLimiter::new(10, 10, limits);
        limiter.begin_turn();
        assert_eq!(limiter.try_acquire("dangerous_cmd"), ToolLimitResult::Allow);
        assert_eq!(
            limiter.try_acquire("dangerous_cmd"),
            ToolLimitResult::ToolLimitReached("dangerous_cmd".to_string())
        );
        // Other tools are unaffected.
        assert_eq!(limiter.try_acquire("safe_cmd"), ToolLimitResult::Allow);
    }
}
