// crates/runtime/src/core/react_loop/tool_budget.rs
use fabric::MonoTime;
use tracing::warn;

/// Record of a single tool call for budget tracking.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub timestamp: MonoTime,
    pub success: bool,
}

/// Manages tool call budget per turn to prevent infinite loops.
#[derive(Debug)]
pub struct ToolBudget {
    max_calls: usize,
    used_calls: usize,
    call_history: Vec<ToolCallRecord>,
}

impl ToolBudget {
    /// Create a new budget with the given maximum calls per turn.
    pub fn new(max_calls: usize) -> Self {
        Self {
            max_calls,
            used_calls: 0,
            call_history: Vec::new(),
        }
    }

    /// Check if we can still make tool calls.
    /// max_calls == 0 means unlimited.
    pub fn can_call(&self) -> bool {
        self.max_calls == 0 || self.used_calls < self.max_calls
    }

    /// Record a tool call and check budget.
    /// Returns true if the call was within budget, false if budget exceeded.
    pub fn record_call(&mut self, record: ToolCallRecord) -> bool {
        if !self.can_call() {
            warn!(
                tool = %record.tool_name,
                used = self.used_calls,
                max = self.max_calls,
                "Tool budget exceeded!"
            );
            return false;
        }

        self.used_calls += 1;
        self.call_history.push(record);
        true
    }

    /// Get remaining calls in budget.
    /// max_calls == 0 means unlimited (returns usize::MAX).
    pub fn remaining(&self) -> usize {
        if self.max_calls == 0 {
            usize::MAX
        } else {
            self.max_calls.saturating_sub(self.used_calls)
        }
    }

    /// Check if budget is exhausted.
    /// max_calls == 0 means unlimited (never exhausted).
    pub fn is_exhausted(&self) -> bool {
        self.max_calls > 0 && self.used_calls >= self.max_calls
    }

    /// Get total calls made.
    pub fn total_calls(&self) -> usize {
        self.used_calls
    }

    /// Get call history.
    pub fn history(&self) -> &[ToolCallRecord] {
        &self.call_history
    }

    /// Reset budget for a new turn.
    pub fn reset(&mut self) {
        self.used_calls = 0;
        self.call_history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_enforcement() {
        let mut budget = ToolBudget::new(3);

        assert!(budget.can_call());
        assert_eq!(budget.remaining(), 3);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: MonoTime(0),
            success: true,
        });
        assert_eq!(budget.remaining(), 2);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: MonoTime(0),
            success: true,
        });
        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: MonoTime(0),
            success: true,
        });

        assert!(budget.is_exhausted());
        assert!(!budget.can_call());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_reset() {
        let mut budget = ToolBudget::new(2);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: MonoTime(0),
            success: true,
        });
        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: MonoTime(0),
            success: true,
        });

        assert!(budget.is_exhausted());

        budget.reset();
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining(), 2);
    }
}
