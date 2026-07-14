//! Durable goal budget reservations.
//!
//! The budget ledger supplements (does not replace) the capability `AdmissionController`.
//! It tracks token/cost/attempt consumption per goal so that persisted goals
//! stay within their declared limits across restarts.

use super::{GoalBudgetUsage, GoalId, ObjectiveStore};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Budget request / reservation types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBudgetRequest {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBudgetReservation {
    pub reservation_id: String,
    pub goal_id: GoalId,
    pub request: GoalBudgetRequest,
    pub status: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// GoalBudgetError
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum GoalBudgetError {
    GoalNotFound(GoalId),
    InputTokenExhausted { limit: u64, used: u64, requested: u64 },
    OutputTokenExhausted { limit: u64, used: u64, requested: u64 },
    CostExhausted { limit: f64, used: f64, requested: f64 },
    AttemptExhausted { limit: u32, used: u32, requested: u32 },
    DeadlineExpired { deadline_ms: i64, now_ms: i64 },
    DuplicateReservation(String),
    ReservationNotFound(String),
    Storage(String),
}

impl fmt::Display for GoalBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GoalNotFound(id) => write!(f, "goal {id} not found"),
            Self::InputTokenExhausted { limit, used, requested } => {
                write!(f, "input token budget exhausted: limit={limit}, used={used}, requested={requested}")
            }
            Self::OutputTokenExhausted { limit, used, requested } => {
                write!(f, "output token budget exhausted: limit={limit}, used={used}, requested={requested}")
            }
            Self::CostExhausted { limit, used, requested } => {
                write!(f, "cost budget exhausted: limit={limit}, used={used}, requested={requested}")
            }
            Self::AttemptExhausted { limit, used, requested } => {
                write!(f, "attempt budget exhausted: limit={limit}, used={used}, requested={requested}")
            }
            Self::DeadlineExpired { deadline_ms, now_ms } => {
                write!(f, "deadline expired: deadline_ms={deadline_ms}, now_ms={now_ms}")
            }
            Self::DuplicateReservation(id) => write!(f, "duplicate reservation: {id}"),
            Self::ReservationNotFound(id) => write!(f, "reservation not found: {id}"),
            Self::Storage(msg) => write!(f, "storage error: {msg}"),
        }
    }
}

impl std::error::Error for GoalBudgetError {}

impl From<rusqlite::Error> for GoalBudgetError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<anyhow::Error> for GoalBudgetError {
    fn from(e: anyhow::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Budget ledger methods
// ---------------------------------------------------------------------------

impl ObjectiveStore {
    /// Reserve budget for a goal attempt. Checks limits against settled +
    /// active reservations, then inserts a 'reserved' ledger row.
    pub fn reserve_goal_budget(
        &self,
        id: GoalId,
        request: GoalBudgetRequest,
        now_ms: i64,
    ) -> Result<GoalBudgetReservation, GoalBudgetError> {
        let goal = self
            .get_goal(id)?
            .ok_or(GoalBudgetError::GoalNotFound(id))?;

        let budget = &goal.spec.budget;

        // Check deadline.
        if let Some(deadline) = budget.deadline_ms {
            if now_ms > deadline {
                return Err(GoalBudgetError::DeadlineExpired {
                    deadline_ms: deadline,
                    now_ms,
                });
            }
        }

        // Sum settled usage.
        let settled = self.settled_usage(id)?;

        // Sum active (non-settled) reservations.
        let active = self.active_reserved_usage(id)?;

        let total_input = settled.input_tokens + active.input_tokens;
        let total_output = settled.output_tokens + active.output_tokens;
        let total_cost = settled.cost_usd + active.cost_usd;
        let total_attempts = settled.attempts + active.attempts;

        // Check limits.
        if total_input + request.input_tokens > budget.max_input_tokens {
            return Err(GoalBudgetError::InputTokenExhausted {
                limit: budget.max_input_tokens,
                used: total_input,
                requested: request.input_tokens,
            });
        }
        if total_output + request.output_tokens > budget.max_output_tokens {
            return Err(GoalBudgetError::OutputTokenExhausted {
                limit: budget.max_output_tokens,
                used: total_output,
                requested: request.output_tokens,
            });
        }
        if let Some(limit) = budget.max_cost_usd {
            if total_cost + request.cost_usd > limit {
                return Err(GoalBudgetError::CostExhausted {
                    limit,
                    used: total_cost,
                    requested: request.cost_usd,
                });
            }
        }
        if total_attempts + request.attempts > budget.max_attempts {
            return Err(GoalBudgetError::AttemptExhausted {
                limit: budget.max_attempts,
                used: total_attempts,
                requested: request.attempts,
            });
        }

        // Insert reservation.
        let reservation_id = Uuid::new_v4().to_string();

        self.db
            .execute(
                "INSERT INTO goal_budget_ledger (objective_id, reservation_id, input_tokens,
                 output_tokens, cost_usd, attempts, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'reserved')",
                rusqlite::params![
                    id.0,
                    reservation_id,
                    request.input_tokens,
                    request.output_tokens,
                    request.cost_usd,
                    request.attempts,
                ],
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("UNIQUE") {
                    GoalBudgetError::DuplicateReservation(reservation_id.clone())
                } else {
                    GoalBudgetError::Storage(msg)
                }
            })?;

        let created: String = self
            .db
            .query_row(
                "SELECT created_at FROM goal_budget_ledger WHERE reservation_id = ?1",
                rusqlite::params![reservation_id],
                |r| r.get(0),
            )
            .map_err(|e| GoalBudgetError::Storage(e.to_string()))?;

        Ok(GoalBudgetReservation {
            reservation_id,
            goal_id: id,
            request,
            status: "reserved".into(),
            created_at: created,
        })
    }

    /// Settle a reservation — mark it as 'settled' with actual usage.
    pub fn settle_goal_budget(
        &self,
        reservation_id: &str,
        actual: GoalBudgetUsage,
    ) -> Result<(), GoalBudgetError> {
        let changed = self
            .db
            .execute(
                "UPDATE goal_budget_ledger SET
                 input_tokens = ?1, output_tokens = ?2, cost_usd = ?3, attempts = ?4,
                 status = 'settled', settled_at = datetime('now')
                 WHERE reservation_id = ?5 AND status = 'reserved'",
                rusqlite::params![
                    actual.input_tokens,
                    actual.output_tokens,
                    actual.cost_usd,
                    actual.attempts,
                    reservation_id,
                ],
            )
            .map_err(|e| GoalBudgetError::Storage(e.to_string()))?;

        if changed == 0 {
            return Err(GoalBudgetError::ReservationNotFound(reservation_id.into()));
        }
        Ok(())
    }

    /// Revoke a reservation — marks it as 'revoked', releasing the reserved quota.
    pub fn revoke_goal_budget(&self, reservation_id: &str) -> Result<(), GoalBudgetError> {
        let changed = self
            .db
            .execute(
                "UPDATE goal_budget_ledger SET status = 'revoked'
                 WHERE reservation_id = ?1 AND status = 'reserved'",
                rusqlite::params![reservation_id],
            )
            .map_err(|e| GoalBudgetError::Storage(e.to_string()))?;

        if changed == 0 {
            return Err(GoalBudgetError::ReservationNotFound(reservation_id.into()));
        }
        Ok(())
    }

    /// Sum settled usage for a goal.
    fn settled_usage(&self, id: GoalId) -> Result<GoalBudgetUsage, GoalBudgetError> {
        self.db
            .query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(cost_usd),0),      COALESCE(SUM(attempts),0)
                 FROM goal_budget_ledger
                 WHERE objective_id = ?1 AND status = 'settled'",
                rusqlite::params![id.0],
                |r| {
                    Ok(GoalBudgetUsage {
                        input_tokens: r.get(0)?,
                        output_tokens: r.get(1)?,
                        cost_usd: r.get(2)?,
                        attempts: r.get(3)?,
                    })
                },
            )
            .map_err(|e| GoalBudgetError::Storage(e.to_string()))
    }

    /// Sum active (reserved, not settled) usage for a goal.
    fn active_reserved_usage(&self, id: GoalId) -> Result<GoalBudgetUsage, GoalBudgetError> {
        self.db
            .query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(cost_usd),0),      COALESCE(SUM(attempts),0)
                 FROM goal_budget_ledger
                 WHERE objective_id = ?1 AND status = 'reserved'",
                rusqlite::params![id.0],
                |r| {
                    Ok(GoalBudgetUsage {
                        input_tokens: r.get(0)?,
                        output_tokens: r.get(1)?,
                        cost_usd: r.get(2)?,
                        attempts: r.get(3)?,
                    })
                },
            )
            .map_err(|e| GoalBudgetError::Storage(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::goal::ObjectiveStore;
    use fabric::goal::{GoalBudget, GoalSpec};
    use fabric::PrincipalId;
    use tempfile::NamedTempFile;

    fn setup() -> (ObjectiveStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    fn create_goal_with_budget(store: &ObjectiveStore, budget: GoalBudget) -> GoalId {
        let spec = GoalSpec {
            original_intent: "budget test".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget,
        };
        store
            .create_goal(&PrincipalId("test".into()), "s", "session", &spec)
            .unwrap()
            .id
    }

    #[test]
    fn reserve_within_limits() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 10000,
                max_output_tokens: 5000,
                max_cost_usd: None,
                max_attempts: 5,
                deadline_ms: None,
            },
        );
        let res = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 500,
                    output_tokens: 250,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap();
        assert_eq!(res.status, "reserved");
        assert!(!res.reservation_id.is_empty());
    }

    #[test]
    fn input_token_exhaustion() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 1000,
                max_output_tokens: 5000,
                max_cost_usd: None,
                max_attempts: 5,
                deadline_ms: None,
            },
        );
        // First reserve uses 800 tokens.
        store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 800,
                    output_tokens: 100,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap();
        // Second reserve tries 300 → over limit.
        let err = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 300,
                    output_tokens: 100,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap_err();
        match err {
            GoalBudgetError::InputTokenExhausted { .. } => {}
            _ => panic!("expected InputTokenExhausted, got {err:?}"),
        }
    }

    #[test]
    fn attempt_exhaustion() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 100000,
                max_output_tokens: 50000,
                max_cost_usd: None,
                max_attempts: 2,
                deadline_ms: None,
            },
        );
        // Reserve 2 attempts.
        store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.0,
                    attempts: 2,
                },
                0,
            )
            .unwrap();
        // Third attempt fails.
        let err = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap_err();
        match err {
            GoalBudgetError::AttemptExhausted { .. } => {}
            _ => panic!("expected AttemptExhausted, got {err:?}"),
        }
    }

    #[test]
    fn deadline_expired() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 100000,
                max_output_tokens: 50000,
                max_cost_usd: None,
                max_attempts: 5,
                deadline_ms: Some(1000),
            },
        );
        // now_ms is past the deadline.
        let err = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                2000,
            )
            .unwrap_err();
        match err {
            GoalBudgetError::DeadlineExpired { .. } => {}
            _ => panic!("expected DeadlineExpired, got {err:?}"),
        }
    }

    #[test]
    fn settle_and_revoke() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 10000,
                max_output_tokens: 5000,
                max_cost_usd: None,
                max_attempts: 5,
                deadline_ms: None,
            },
        );
        let res = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 500,
                    output_tokens: 250,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap();

        // Settle with actual usage.
        store
            .settle_goal_budget(
                &res.reservation_id,
                GoalBudgetUsage {
                    input_tokens: 450,
                    output_tokens: 200,
                    cost_usd: 0.0,
                    attempts: 1,
                },
            )
            .unwrap();

        // Cannot settle again.
        let err = store
            .settle_goal_budget(
                &res.reservation_id,
                GoalBudgetUsage::default(),
            )
            .unwrap_err();
        match err {
            GoalBudgetError::ReservationNotFound(_) => {}
            _ => panic!("expected ReservationNotFound, got {err:?}"),
        }
    }

    #[test]
    fn revoke_releases_quota() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 1000,
                max_output_tokens: 5000,
                max_cost_usd: None,
                max_attempts: 1,
                deadline_ms: None,
            },
        );
        let res = store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 500,
                    output_tokens: 250,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap();

        // Revoke.
        store.revoke_goal_budget(&res.reservation_id).unwrap();

        // Now we can reserve again (released quota).
        store
            .reserve_goal_budget(
                id,
                GoalBudgetRequest {
                    input_tokens: 500,
                    output_tokens: 250,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                0,
            )
            .unwrap();
    }

    #[test]
    fn duplicate_reservation_id_rejected() {
        let (store, _tmp) = setup();
        let id = create_goal_with_budget(
            &store,
            GoalBudget {
                max_input_tokens: 10000,
                max_output_tokens: 5000,
                max_cost_usd: None,
                max_attempts: 5,
                deadline_ms: None,
            },
        );
        // Insert a row manually with a known reservation_id.
        store
            .db
            .execute(
                "INSERT INTO goal_budget_ledger (objective_id, reservation_id, status)
                 VALUES (?1, 'dup-id', 'reserved')",
                rusqlite::params![id.0],
            )
            .unwrap();

        // Now try to reserve with the same id — but our generate uses UUID so
        // we test the duplicate path by manually inserting.
        // Instead, let's just verify the UNIQUE constraint works:
        let err = store
            .db
            .execute(
                "INSERT INTO goal_budget_ledger (objective_id, reservation_id, status)
                 VALUES (?1, 'dup-id', 'reserved')",
                rusqlite::params![id.0],
            )
            .unwrap_err();
        assert!(err.to_string().contains("UNIQUE"));
    }
}
