use std::sync::atomic::{AtomicUsize, Ordering};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetStatus {
    Ok,
    Warning,   // >= 80% used
    Exhausted,
}

/// Per-agent iteration budget — independent from parent budget.
pub struct IterationBudget {
    max_total: usize,
    used: AtomicUsize,
}

impl IterationBudget {
    pub fn new(max_total: usize) -> Self {
        Self {
            max_total,
            used: AtomicUsize::new(0),
        }
    }

    /// Consume one iteration. Returns the new status.
    pub fn consume(&self) -> BudgetStatus {
        let prev = self.used.fetch_add(1, Ordering::SeqCst);
        let now = prev + 1;
        if now >= self.max_total {
            warn!(used = now, max = self.max_total, "Iteration budget exhausted");
            BudgetStatus::Exhausted
        } else if now as f64 / self.max_total as f64 >= 0.8 {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Ok
        }
    }

    /// Refund iterations (e.g., on API error or timeout).
    pub fn refund(&self, count: usize) {
        self.used.fetch_sub(count, Ordering::SeqCst);
    }

    /// Check current status without consuming.
    pub fn status(&self) -> BudgetStatus {
        let used = self.used.load(Ordering::SeqCst);
        if used >= self.max_total {
            BudgetStatus::Exhausted
        } else if used as f64 / self.max_total as f64 >= 0.8 {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Ok
        }
    }

    pub fn remaining(&self) -> usize {
        self.max_total.saturating_sub(self.used.load(Ordering::SeqCst))
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }

    pub fn max(&self) -> usize {
        self.max_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_consume() {
        let budget = IterationBudget::new(10);
        assert_eq!(budget.status(), BudgetStatus::Ok);
        assert_eq!(budget.remaining(), 10);

        for _ in 0..8 {
            budget.consume();
        }
        assert_eq!(budget.status(), BudgetStatus::Warning);

        budget.consume();
        budget.consume();
        assert_eq!(budget.status(), BudgetStatus::Exhausted);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_refund() {
        let budget = IterationBudget::new(10);
        for _ in 0..5 {
            budget.consume();
        }
        assert_eq!(budget.used(), 5);
        budget.refund(2);
        assert_eq!(budget.used(), 3);
        assert_eq!(budget.remaining(), 7);
    }
}
