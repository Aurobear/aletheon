//! In-memory budget controller — tracks token/cost budgets per principal.
//!
//! Each principal is initialized with a token budget and cost budget.
//! `admit()` reserves a portion; `settle()` adjusts to actual usage;
//! `revoke()` returns the reserved budget to the pool.
//!
//! # Concurrency
//!
//! All operations are behind `Mutex<HashMap<...>>`. The budget controller is
//! expected to be called sequentially within the admission pipeline (admit →
//! settle/revoke), so lock contention is minimal.

use fabric::{AdmissionError, BudgetRequest, BudgetReservationId, UsageReport};
use std::collections::HashMap;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Budget controller
// ---------------------------------------------------------------------------

/// Per-principal budget state.
#[derive(Debug, Clone)]
struct BudgetAccount {
    tokens_remaining: Option<u64>,     // None = unlimited
    cost_remaining_micro: Option<u64>, // None = unlimited
    active_reservations: HashMap<BudgetReservationId, BudgetHold>,
}

#[derive(Debug, Clone)]
struct BudgetHold {
    tokens: Option<u64>,
    cost_micro: Option<u64>,
}

impl BudgetAccount {
    fn new(max_tokens: Option<u64>, max_cost_micro: Option<u64>) -> Self {
        Self {
            tokens_remaining: max_tokens,
            cost_remaining_micro: max_cost_micro,
            active_reservations: HashMap::new(),
        }
    }

    /// Check whether a request fits within remaining budget.
    fn can_afford(&self, request: &BudgetRequest) -> bool {
        if let Some(max_tokens) = request.max_tokens {
            if let Some(remaining) = self.tokens_remaining {
                if max_tokens > remaining {
                    return false;
                }
            }
        }
        if let Some(max_cost) = request.max_cost_micro {
            if let Some(remaining) = self.cost_remaining_micro {
                if max_cost > remaining {
                    return false;
                }
            }
        }
        true
    }

    /// Reserve budget from the account. Caller must have checked `can_afford` first.
    fn reserve(&mut self, request: &BudgetRequest) -> BudgetHold {
        let hold = BudgetHold {
            tokens: request.max_tokens,
            cost_micro: request.max_cost_micro,
        };
        if let Some(t) = request.max_tokens {
            if let Some(ref mut rem) = self.tokens_remaining {
                *rem = rem.saturating_sub(t);
            }
        }
        if let Some(c) = request.max_cost_micro {
            if let Some(ref mut rem) = self.cost_remaining_micro {
                *rem = rem.saturating_sub(c);
            }
        }
        hold
    }

    /// Settle: adjust reservation to actual usage.
    fn settle(&mut self, hold: BudgetHold, usage: &UsageReport) {
        // Return unused reserved tokens/cost, then deduct actual usage.
        if let Some(reserved) = hold.tokens {
            if let Some(ref mut rem) = self.tokens_remaining {
                let unused = reserved.saturating_sub(usage.tokens_used);
                *rem = rem.saturating_add(unused);
            }
        }
        if let Some(reserved) = hold.cost_micro {
            if let Some(ref mut rem) = self.cost_remaining_micro {
                let unused = reserved.saturating_sub(usage.cost_micro);
                *rem = rem.saturating_add(unused);
            }
        }
    }

    /// Revoke: return the full reserved amount to the pool.
    fn revoke(&mut self, hold: BudgetHold) {
        if let Some(t) = hold.tokens {
            if let Some(ref mut rem) = self.tokens_remaining {
                *rem = rem.saturating_add(t);
            }
        }
        if let Some(c) = hold.cost_micro {
            if let Some(ref mut rem) = self.cost_remaining_micro {
                *rem = rem.saturating_add(c);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InMemoryBudgetController
// ---------------------------------------------------------------------------

/// In-memory budget controller keyed by principal id.
///
/// Unlimited accounts (no max set) always approve reservations.
pub struct InMemoryBudgetController {
    accounts: Mutex<HashMap<String, BudgetAccount>>,
}

impl std::fmt::Debug for InMemoryBudgetController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryBudgetController")
            .finish_non_exhaustive()
    }
}

impl InMemoryBudgetController {
    /// Create an empty budget controller. Use [`set_budget`](Self::set_budget)
    /// to configure per-principal limits.
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Set the budget for a principal. `None` values mean unlimited.
    pub async fn set_budget(
        &self,
        principal: &str,
        max_tokens: Option<u64>,
        max_cost_micro: Option<u64>,
    ) {
        let mut accounts = self.accounts.lock().await;
        accounts.insert(
            principal.to_string(),
            BudgetAccount::new(max_tokens, max_cost_micro),
        );
    }

    /// Check whether the given request fits within the principal's budget and
    /// reserve the budget if so. Returns a reservation id on success.
    pub async fn reserve(
        &self,
        principal: &str,
        request: &BudgetRequest,
    ) -> Result<BudgetReservationId, AdmissionError> {
        let mut accounts = self.accounts.lock().await;
        let account = accounts
            .entry(principal.to_string())
            .or_insert_with(|| BudgetAccount::new(None, None));

        // Check limit before passing budget request
        if !account.can_afford(request) {
            return Err(AdmissionError::BudgetExceeded);
        }

        let hold = account.reserve(request);
        let id = BudgetReservationId::new();
        account.active_reservations.insert(id, hold);
        Ok(id)
    }

    /// Settle a reservation with actual usage data.
    pub async fn settle(
        &self,
        principal: &str,
        reservation: BudgetReservationId,
        usage: &UsageReport,
    ) {
        let mut accounts = self.accounts.lock().await;
        if let Some(account) = accounts.get_mut(principal) {
            if let Some(hold) = account.active_reservations.remove(&reservation) {
                account.settle(hold, usage);
            }
        }
    }

    /// Revoke a reservation, returning the budget to the pool.
    pub async fn revoke(&self, principal: &str, reservation: BudgetReservationId) {
        let mut accounts = self.accounts.lock().await;
        if let Some(account) = accounts.get_mut(principal) {
            if let Some(hold) = account.active_reservations.remove(&reservation) {
                account.revoke(hold);
            }
        }
    }

    /// Return the remaining budget for a principal.
    pub async fn remaining_tokens(&self, principal: &str) -> Option<Option<u64>> {
        let accounts = self.accounts.lock().await;
        accounts.get(principal).map(|a| a.tokens_remaining)
    }

    /// Return the remaining cost budget (in micro-dollars) for a principal.
    pub async fn remaining_cost(&self, principal: &str) -> Option<Option<u64>> {
        let accounts = self.accounts.lock().await;
        accounts.get(principal).map(|a| a.cost_remaining_micro)
    }
}

impl Default for InMemoryBudgetController {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_account_always_approves() {
        let ctrl = InMemoryBudgetController::new();
        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: Some(500),
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        // settle
        ctrl.settle(
            "agent-1",
            id,
            &UsageReport {
                tokens_used: 8_000,
                cost_micro: 400,
                ..Default::default()
            },
        )
        .await;
    }

    #[tokio::test]
    async fn limited_account_approves_within_budget() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(100_000), Some(1_000_000))
            .await;

        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: Some(5_000),
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        assert!(id.0 != uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn limited_account_denies_over_budget() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(1_000), Some(10_000)).await;

        // Request more tokens than the budget allows.
        let req = BudgetRequest {
            max_tokens: Some(5_000),
            max_cost_micro: None,
        };
        let err = ctrl.reserve("agent-1", &req).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }

    #[tokio::test]
    async fn settle_adjusts_to_actual_usage() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(100_000), None).await;

        // Reserve 10k tokens, only use 3k — remaining should reflect actual.
        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: None,
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        ctrl.settle(
            "agent-1",
            id,
            &UsageReport {
                tokens_used: 3_000,
                ..Default::default()
            },
        )
        .await;

        // 100_000 - 3_000 = 97_000 remaining
        // After settle, we should be able to request up to 97_000.
        let req2 = BudgetRequest {
            max_tokens: Some(97_000),
            max_cost_micro: None,
        };
        let id2 = ctrl.reserve("agent-1", &req2).await.unwrap();
        ctrl.settle(
            "agent-1",
            id2,
            &UsageReport {
                tokens_used: 97_000,
                ..Default::default()
            },
        )
        .await;

        // Now budget should be exhausted.
        let req3 = BudgetRequest {
            max_tokens: Some(1),
            max_cost_micro: None,
        };
        let err = ctrl.reserve("agent-1", &req3).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }

    #[tokio::test]
    async fn revoke_returns_budget_to_pool() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(10_000), None).await;

        // Reserve 5k, then revoke.
        let req = BudgetRequest {
            max_tokens: Some(5_000),
            max_cost_micro: None,
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        ctrl.revoke("agent-1", id).await;

        // Should be able to reserve 10k again (full budget returned).
        let req2 = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: None,
        };
        let id2 = ctrl.reserve("agent-1", &req2).await.unwrap();
        assert!(id2.0 != uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn cost_budget_limits_enforced() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", None, Some(1_000)).await;

        let req = BudgetRequest {
            max_tokens: None,
            max_cost_micro: Some(2_000),
        };
        let err = ctrl.reserve("agent-1", &req).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }
}
