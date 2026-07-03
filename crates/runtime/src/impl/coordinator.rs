//! Coordinator — arbitration layer between BrainCore plans and SelfField verdicts.
//!
//! The Coordinator is a **temporary arbitrator**, not a supreme authority.
//! It synthesizes the SelfField verdict, plan risk, execution history, and
//! memory context into a single `ArbitrationResult` that tells the Engine
//! what to do next. The Coordinator owns no state — it reads, decides, and
//! returns. It can be replaced or bypassed without data loss.
//!
//! Relationship to Engine:
//!   Engine -> Coordinator::arbitrate(verdict, plan, history) -> ArbitrationResult
//!   Engine acts on the result (execute, deny, delay, etc.)
//!   Engine does NOT delegate control flow to Coordinator permanently.

use anyhow::Result;
use base::brain::Plan;
use base::self_field::{RiskLevel, Verdict};

/// The outcome of arbitration — what the Engine should do next.
#[derive(Debug, Clone)]
pub enum ArbitrationResult {
    /// Proceed with execution as-is.
    Execute,
    /// Refuse to execute. Contains the reason.
    Deny { reason: String },
    /// Postpone execution until the named condition is met.
    Delay { reason: String, until: String },
    /// Run in sandbox first; only promote to real execution if sandbox passes.
    SandboxFirst { reason: String },
    /// Ask the user for explicit confirmation before proceeding.
    AskConfirmation { reason: String },
    /// The Coordinator wants to reflect (re-think) before deciding.
    /// The Engine should feed this back to BrainCore for another think cycle.
    Reflect { reason: String },
    /// The plan should be mutated (rewritten) before execution.
    Mutate {
        reason: String,
        suggested_modification: Option<String>,
    },
}

/// Historical context from memory that informs arbitration decisions.
#[derive(Debug, Clone, Default)]
pub struct MemoryContext {
    /// Number of past failures for similar actions (same action string prefix).
    pub similar_action_failures: usize,
    /// Whether the user has previously overridden denials for this action type.
    pub user_has_overridden: bool,
    /// Whether this action type has been sandboxed successfully before.
    pub prior_sandbox_success: bool,
    /// Timestamp of the most recent failure (epoch millis).
    pub last_failure_at: Option<i64>,
}

/// The Coordinator — a stateless arbitration function.
///
/// It is NOT a god object. It does not persist, schedule, or execute.
/// It takes inputs, applies a decision tree, and returns an `ArbitrationResult`.
pub struct Coordinator;

impl Coordinator {
    /// Arbitrate between a SelfField verdict and a BrainCore plan.
    ///
    /// Decision tree (evaluated in order — first match wins):
    ///
    /// 1. `Verdict::Deny` → `ArbitrationResult::Deny`
    /// 2. `Verdict::SandboxFirst` → `ArbitrationResult::SandboxFirst`
    /// 3. `Verdict::Delay` → `ArbitrationResult::Delay`
    /// 4. `Verdict::RequireConfirmation` → `ArbitrationResult::AskConfirmation`
    /// 5. `Verdict::AllowWithModification` → `ArbitrationResult::Mutate`
    /// 6. Plan `RiskLevel::Critical` → `ArbitrationResult::SandboxFirst`
    /// 7. Plan `RiskLevel::High` + past failures → `ArbitrationResult::AskConfirmation`
    /// 8. Plan `RiskLevel::High` + no failures → `ArbitrationResult::Execute`
    /// 9. `Verdict::Allow` + `RiskLevel::Medium` → `ArbitrationResult::Execute`
    /// 10. `Verdict::Allow` + `RiskLevel::Low` or `None` → `ArbitrationResult::Execute`
    ///
    /// Edge cases:
    /// - If past failures exist but user has overridden denials before, skip
    ///   AskConfirmation and go to Execute (trust the user).
    /// - If prior sandbox success exists for Critical risk, still SandboxFirst
    ///   (Critical always requires sandbox — safety first).
    pub async fn arbitrate(
        verdict: &Verdict,
        plan: &Plan,
        memory: &MemoryContext,
    ) -> Result<ArbitrationResult> {
        // 1. Hard deny — no override.
        if let Verdict::Deny { reason } = verdict {
            return Ok(ArbitrationResult::Deny {
                reason: format!("SelfField denied: {}", reason),
            });
        }

        // 2. SelfField mandates sandbox.
        if let Verdict::SandboxFirst { reason } = verdict {
            return Ok(ArbitrationResult::SandboxFirst {
                reason: format!("SelfField requires sandbox: {}", reason),
            });
        }

        // 3. SelfField mandates delay.
        if let Verdict::Delay { reason, until } = verdict {
            return Ok(ArbitrationResult::Delay {
                reason: format!("SelfField delays: {}", reason),
                until: until.clone(),
            });
        }

        // 4. SelfField requires user confirmation.
        if let Verdict::RequireConfirmation { reason, risk_level } = verdict {
            return Ok(ArbitrationResult::AskConfirmation {
                reason: format!(
                    "SelfField requires confirmation (risk={:?}): {}",
                    risk_level, reason
                ),
            });
        }

        // 5. SelfField allows with modification — mutate the plan.
        if let Verdict::AllowWithModification { modification } = verdict {
            return Ok(ArbitrationResult::Mutate {
                reason: "SelfField requires plan modification".to_string(),
                suggested_modification: Some(modification.to_string()),
            });
        }

        // From here on, verdict is Allow. Apply risk-based logic on the plan.

        // 6. Critical risk — always sandbox first, regardless of history.
        if plan.risk_level == RiskLevel::Critical {
            return Ok(ArbitrationResult::SandboxFirst {
                reason: format!(
                    "Plan risk is Critical — sandbox first (plan: {})",
                    plan.reasoning
                ),
            });
        }

        // 7. High risk + past failures — ask for confirmation (unless user overrides).
        if plan.risk_level == RiskLevel::High && memory.similar_action_failures > 0 {
            if memory.user_has_overridden {
                return Ok(ArbitrationResult::Execute);
            }
            return Ok(ArbitrationResult::AskConfirmation {
                reason: format!(
                    "High risk with {} past failure(s) for similar actions",
                    memory.similar_action_failures
                ),
            });
        }

        // 8-10. Everything else — allow execution.
        Ok(ArbitrationResult::Execute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::brain::CostEstimate;

    fn make_plan(risk: RiskLevel, reasoning: &str) -> Plan {
        Plan {
            id: uuid::Uuid::new_v4(),
            steps: vec![],
            estimated_cost: CostEstimate::default(),
            risk_level: risk,
            reasoning: reasoning.to_string(),
            alternatives: vec![],
        }
    }

    #[tokio::test]
    async fn deny_verdict_overrides_everything() {
        let verdict = Verdict::Deny {
            reason: "unsafe".into(),
        };
        let plan = make_plan(RiskLevel::None, "low risk plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::Deny { .. }));
    }

    #[tokio::test]
    async fn critical_risk_sandboxes_even_with_allow() {
        let verdict = Verdict::Allow;
        let plan = make_plan(RiskLevel::Critical, "dangerous plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::SandboxFirst { .. }));
    }

    #[tokio::test]
    async fn high_risk_with_failures_asks_confirmation() {
        let verdict = Verdict::Allow;
        let plan = make_plan(RiskLevel::High, "risky plan");
        let mem = MemoryContext {
            similar_action_failures: 2,
            ..Default::default()
        };

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::AskConfirmation { .. }));
    }

    #[tokio::test]
    async fn high_risk_user_override_skips_confirmation() {
        let verdict = Verdict::Allow;
        let plan = make_plan(RiskLevel::High, "risky plan");
        let mem = MemoryContext {
            similar_action_failures: 3,
            user_has_overridden: true,
            ..Default::default()
        };

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::Execute));
    }

    #[tokio::test]
    async fn allow_with_modification_returns_mutate() {
        let verdict = Verdict::AllowWithModification {
            modification: serde_json::json!({"step_0_timeout": 5000}),
        };
        let plan = make_plan(RiskLevel::Medium, "plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::Mutate { .. }));
    }

    #[tokio::test]
    async fn allow_low_risk_executes() {
        let verdict = Verdict::Allow;
        let plan = make_plan(RiskLevel::Low, "safe plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::Execute));
    }

    #[tokio::test]
    async fn selffield_delay_propagates() {
        let verdict = Verdict::Delay {
            reason: "cooldown".into(),
            until: "2026-06-15T00:00:00Z".into(),
        };
        let plan = make_plan(RiskLevel::None, "plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::Delay { .. }));
    }

    #[tokio::test]
    async fn selffield_require_confirmation_propagates() {
        let verdict = Verdict::RequireConfirmation {
            reason: "external API call".into(),
            risk_level: RiskLevel::Medium,
        };
        let plan = make_plan(RiskLevel::Medium, "plan");
        let mem = MemoryContext::default();

        let result = Coordinator::arbitrate(&verdict, &plan, &mem).await.unwrap();
        assert!(matches!(result, ArbitrationResult::AskConfirmation { .. }));
    }
}
